//! Native presentation counters, never inputs to removal measurements.

use std::collections::BTreeMap;
use std::ffi::{CString, OsString, c_char, c_int, c_void};
use std::os::unix::ffi::{OsStrExt as _, OsStringExt as _};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::space::{MACOS_DATA_VOLUME, NativeSpace};

use super::AttrList;

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFURLVolumeAvailableCapacityForImportantUsageKey: *const c_void;
    fn CFURLCreateFromFileSystemRepresentation(
        allocator: *const c_void,
        bytes: *const u8,
        length: isize,
        directory: u8,
    ) -> *const c_void;
    fn CFURLCopyResourcePropertyForKey(
        url: *const c_void,
        key: *const c_void,
        value: *mut *const c_void,
        error: *mut *const c_void,
    ) -> u8;
    fn CFGetTypeID(value: *const c_void) -> usize;
    fn CFNumberGetTypeID() -> usize;
    fn CFNumberGetValue(
        number: *const c_void,
        kind: isize,
        value: *mut c_void,
    ) -> u8;
    fn CFRelease(value: *const c_void);
}

unsafe extern "C" {
    fn getattrlist(
        path: *const c_char,
        attributes: *mut AttrList,
        buffer: *mut c_void,
        length: usize,
        options: u64,
    ) -> c_int;
}

struct OwnedCf(*const c_void);

impl Drop for OwnedCf {
    fn drop(&mut self) {
        // SAFETY: constructed only for a non-null Create/Copy result; this
        // owner releases exactly the retained reference returned by CF.
        unsafe { CFRelease(self.0) };
    }
}

trait CapacityProbe {
    fn important_available(&mut self, volume: &Path) -> Option<u64>;
}

struct SystemProbe;

impl CapacityProbe for SystemProbe {
    fn important_available(&mut self, volume: &Path) -> Option<u64> {
        read_important_available(volume)
    }
}

// CoreFoundation's important-usage key goes through CacheDelete on macOS
// 27. A miss can write roughly 120 persisted log lines. Include failed
// lookups in the same budget so an unavailable estimate cannot spin.
const REFRESH_INTERVAL: Duration = Duration::from_mins(1);

#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct MountedVolume {
    root: PathBuf,
    source: OsString,
}

struct Sample {
    at: Instant,
    available: Option<u64>,
}

struct CapacitySampler<P> {
    probe: P,
    samples: BTreeMap<MountedVolume, Sample>,
}

impl<P: CapacityProbe> CapacitySampler<P> {
    const fn new(probe: P) -> Self {
        Self {
            probe,
            samples: BTreeMap::new(),
        }
    }

    fn available(
        &mut self,
        volume: MountedVolume,
        now: Instant,
        refresh: bool,
    ) -> Option<u64> {
        if let Some(sample) = self.samples.get(&volume)
            && now.duration_since(sample.at) < REFRESH_INTERVAL
        {
            return sample.available;
        }
        if !refresh {
            return None;
        }
        // Bound the cache to recently sampled mounts, including removable
        // disks that may never be looked up again.
        self.samples.retain(|_, sample| {
            now.duration_since(sample.at) < REFRESH_INTERVAL
        });
        let available = self.probe.important_available(&volume.root);
        self.samples.insert(volume, Sample { at: now, available });
        available
    }
}

// Serialize lookup and sampling so a scan and the live meter cannot both
// spend the same mounted volume's refresh budget.
static CAPACITY: Mutex<CapacitySampler<SystemProbe>> =
    Mutex::new(CapacitySampler::new(SystemProbe));

fn read_important_available(path: &Path) -> Option<u64> {
    let bytes = path.as_os_str().as_bytes();
    let length = isize::try_from(bytes.len()).ok()?;
    // SAFETY: CF copies the borrowed filesystem bytes. The default
    // allocator and directory flag are valid; null is handled below.
    let url = unsafe {
        CFURLCreateFromFileSystemRepresentation(
            std::ptr::null(),
            bytes.as_ptr(),
            length,
            1,
        )
    };
    if url.is_null() {
        return None;
    }
    let url = OwnedCf(url);
    let mut value = std::ptr::null();
    // SAFETY: url is retained, the key is a system-owned CFString, and
    // value is writable. A null error pointer declines an error object.
    let copied = unsafe {
        CFURLCopyResourcePropertyForKey(
            url.0,
            kCFURLVolumeAvailableCapacityForImportantUsageKey,
            &raw mut value,
            std::ptr::null_mut(),
        )
    };
    if value.is_null() {
        return None;
    }
    let value = OwnedCf(value);
    // SAFETY: value is a retained CF object. Check its runtime type before
    // asking for the signed 64-bit CFNumber representation (type code 4).
    let mut bytes = 0_i64;
    let valid = unsafe {
        copied != 0
            && CFGetTypeID(value.0) == CFNumberGetTypeID()
            && CFNumberGetValue(value.0, 4, (&raw mut bytes).cast()) != 0
    };
    valid.then(|| u64::try_from(bytes).ok()).flatten()
}

fn mounted_volume(path: &Path) -> Option<MountedVolume> {
    let stat = rustix::fs::statfs(path).ok()?;
    let root = mount_field(&stat.f_mntonname);
    (!root.is_empty()).then(|| MountedVolume {
        root: PathBuf::from(root),
        source: mount_field(&stat.f_mntfromname),
    })
}

fn mount_field(field: &[c_char]) -> OsString {
    OsString::from_vec(
        field
            .iter()
            .take_while(|&&byte| byte != 0)
            .map(|&byte| byte as u8)
            .collect(),
    )
}

pub(super) fn volume_used(path: &Path) -> Option<u64> {
    let path = CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut attrs = AttrList {
        bitmapcount: 5,
        reserved: 0,
        commonattr: 0,
        // ATTR_VOL_INFO is required with volume attributes. SPACEUSED is
        // volume allocation, unlike capacity minus the container's free pool.
        volattr: 0x8080_0000,
        dirattr: 0,
        fileattr: 0,
        forkattr: 0,
    };
    let mut buffer = [0_u8; 12];
    // SAFETY: path is NUL-terminated, the attribute list has the Darwin ABI,
    // and the output fits a length word plus one 64-bit volume attribute.
    let result = unsafe {
        getattrlist(
            path.as_ptr(),
            &raw mut attrs,
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            0,
        )
    };
    if result != 0 || u32::from_ne_bytes(buffer[..4].try_into().ok()?) != 12 {
        return None;
    }
    Some(u64::from_ne_bytes(buffer[4..].try_into().ok()?))
}

pub fn presentation(
    path: &Path,
    total: u64,
    raw_available: u64,
    refresh: bool,
) -> Option<NativeSpace> {
    let volume = mounted_volume(path)?;
    let root = &volume.root;
    let used =
        if root == Path::new("/") || root == Path::new(MACOS_DATA_VOLUME) {
            // Finder presents the startup System/Data volume group. Preboot,
            // Recovery and VM also consume shared capacity, but not this figure.
            volume_used(Path::new("/"))
                .zip(volume_used(Path::new(MACOS_DATA_VOLUME)))
                .and_then(|(system, data)| system.checked_add(data))
        } else {
            volume_used(root)
        }
        .filter(|used| *used <= total);
    // This API already includes purgeable space. Zero while raw free is
    // positive (as in a restricted process) is unavailable, not a measurement.
    let available = CAPACITY
        .lock()
        .ok()
        .and_then(|mut sampler| {
            sampler.available(volume, Instant::now(), refresh)
        })
        .filter(|available| *available >= raw_available && *available <= total);
    (used.is_some() || available.is_some())
        .then_some(NativeSpace { used, available })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    // One logging backend call is approximately 120 lines on macOS 27.
    const CALLS_PER_VOLUME_REFRESH: usize = 1;
    const CALLS_PER_CACHED_SAMPLE: usize = 0;

    struct CountingProbe {
        calls: usize,
        result: Option<u64>,
    }

    impl CapacityProbe for CountingProbe {
        fn important_available(&mut self, _: &Path) -> Option<u64> {
            self.calls += 1;
            self.result
        }
    }

    fn fixture(source: &str) -> MountedVolume {
        MountedVolume {
            root: PathBuf::from("/Volumes/Fixture"),
            source: OsString::from(source),
        }
    }

    proptest::proptest! {
        #[test]
        fn refreshes_within_a_minute_share_one_probe(
            offsets in proptest::collection::vec(1..60_000_u64, 1..100),
            result in proptest::option::of(1_u64..100),
        ) {
            let mut sampler = CapacitySampler::new(CountingProbe { calls: 0, result });
            let start = Instant::now();
            proptest::prop_assert_eq!(sampler.available(fixture("one"), start, true), result);
            for offset in offsets {
                let now = start + Duration::from_millis(offset);
                proptest::prop_assert_eq!(sampler.available(fixture("one"), now, true), result);
            }
            proptest::prop_assert_eq!(sampler.probe.calls, CALLS_PER_VOLUME_REFRESH);
        }
    }

    #[test]
    fn unavailable_sample_is_reused_on_the_next_tick() {
        let mut sampler = CapacitySampler::new(CountingProbe {
            calls: 0,
            result: None,
        });
        let start = Instant::now();
        assert_eq!(sampler.available(fixture("one"), start, true), None);
        let next = start + Duration::from_millis(1);
        assert_eq!(sampler.available(fixture("one"), next, true), None);
        assert_eq!(sampler.probe.calls, CALLS_PER_VOLUME_REFRESH);
    }

    #[test]
    fn cached_reads_are_free_and_a_new_volume_has_its_own_budget() {
        let mut sampler = CapacitySampler::new(CountingProbe {
            calls: 0,
            result: Some(10),
        });
        let start = Instant::now();
        assert_eq!(sampler.available(fixture("one"), start, false), None);
        assert_eq!(sampler.probe.calls, CALLS_PER_CACHED_SAMPLE);
        assert_eq!(sampler.available(fixture("one"), start, true), Some(10));
        sampler.probe.result = Some(20);
        assert_eq!(sampler.available(fixture("two"), start, true), Some(20));
        assert_eq!(sampler.available(fixture("one"), start, false), Some(10));
        assert_eq!(sampler.probe.calls, 2 * CALLS_PER_VOLUME_REFRESH);

        let nearly_expired = start + Duration::from_secs(59);
        assert_eq!(
            sampler.available(fixture("one"), nearly_expired, true),
            Some(10)
        );
        let expired = start + Duration::from_mins(1);
        assert_eq!(sampler.available(fixture("one"), expired, false), None);
        assert_eq!(sampler.probe.calls, 2 * CALLS_PER_VOLUME_REFRESH);
        assert_eq!(sampler.available(fixture("one"), expired, true), Some(20));
        assert_eq!(sampler.probe.calls, 3 * CALLS_PER_VOLUME_REFRESH);
    }

    /// diskutil is an independent oracle for volume allocation. Test only
    /// the owned image: the startup disk changes under other processes.
    fn diskutil_used(root: &Path) -> u64 {
        let device = crate::space::device_for(root).expect("image device");
        let device = device.strip_prefix("/dev/").expect("BSD device");
        let list = Command::new("diskutil")
            .args(["apfs", "list", "-plist"])
            .output()
            .expect("diskutil");
        assert!(list.status.success());
        let file = tempfile::NamedTempFile::new().expect("plist");
        std::fs::write(file.path(), list.stdout).expect("write plist");
        let json = Command::new("plutil")
            .args(["-extract", "Containers", "json", "-o", "-"])
            .arg(file.path())
            .output()
            .expect("plutil");
        assert!(json.status.success());
        let containers: Vec<serde_json::Value> =
            serde_json::from_slice(&json.stdout).expect("containers");
        containers
            .iter()
            .filter_map(|container| container["Volumes"].as_array())
            .flatten()
            .find(|volume| volume["DeviceIdentifier"].as_str() == Some(device))
            .and_then(|volume| volume["CapacityInUse"].as_u64())
            .expect("owned volume allocation")
    }

    #[test]
    fn native_volume_usage_matches_diskutil_before_and_after_writes() {
        let image = crate::test_paths::disk_image::Image::new();
        for bytes in [0, 1_048_576, 3_145_728] {
            std::fs::write(image.root().join("sel/data"), vec![7; bytes])
                .expect("owned fixture write");
            // Flush through the fixture's ownership-checked remount, so
            // diskutil and getattrlist observe committed allocation.
            image.free();
            let root = image.root();
            let before = volume_used(&root).expect("native allocation");
            let oracle = diskutil_used(&root);
            let after = volume_used(&root).expect("native allocation");
            assert!(
                (before.min(after)..=before.max(after)).contains(&oracle),
                "diskutil {oracle}, native {before}..{after}"
            );
            let raw = crate::space::space_info(&root).expect("raw counters");
            let display = presentation(&root, raw.total, raw.available, false)
                .expect("native counters");
            assert_eq!(display.used, Some(after));
            if let Some(available) = display.available {
                assert!((raw.available..=raw.total).contains(&available));
            }
        }
        drop(image);
    }
}
