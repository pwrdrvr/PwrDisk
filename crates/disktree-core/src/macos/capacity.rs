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
    fn important_available(&mut self, volume: &MountedVolume) -> Option<u64>;
}

struct SystemProbe {
    raw_available: u64,
}

impl CapacityProbe for SystemProbe {
    fn important_available(&mut self, volume: &MountedVolume) -> Option<u64> {
        let available = read_important_available(&volume.root)?;
        // The path can change mounts, or the free blocks can change while
        // CoreFoundation is measuring. Discard either ambiguous estimate.
        let raw = crate::space::space_info(&volume.root).ok()?;
        (raw.available == self.raw_available
            && volume_uuid(&volume.root) == Some(volume.uuid))
        .then_some(available)
    }
}

// CoreFoundation's important-usage key goes through CacheDelete on macOS
// 27. A miss can write roughly 120 persisted log lines. Include failed
// lookups in the same budget so an unavailable estimate cannot spin.
const REFRESH_INTERVAL: Duration = Duration::from_mins(1);

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct MountedVolume {
    root: PathBuf,
    uuid: [u8; 16],
}

struct Sample {
    at: Instant,
    raw_available: u64,
    available: Option<u64>,
}

enum Entry {
    InFlight,
    Ready(Sample),
}

struct CapacitySampler {
    samples: Mutex<BTreeMap<MountedVolume, Entry>>,
}

impl CapacitySampler {
    const fn new() -> Self {
        Self {
            samples: Mutex::new(BTreeMap::new()),
        }
    }

    fn available(
        &self,
        volume: MountedVolume,
        raw_available: u64,
        refresh: bool,
        clock: impl Fn() -> Instant,
        probe: &mut impl CapacityProbe,
    ) -> Option<u64> {
        {
            let mut samples = self.samples.lock().ok()?;
            let now = clock();
            match samples.get(&volume) {
                Some(Entry::InFlight) => return None,
                Some(Entry::Ready(sample))
                    if now.duration_since(sample.at) < REFRESH_INTERVAL =>
                {
                    // The old estimate includes the old free blocks.
                    return sample
                        .available
                        .filter(|_| sample.raw_available == raw_available);
                }
                _ => {}
            }
            if !refresh {
                return None;
            }
            samples.retain(|_, entry| match entry {
                Entry::InFlight => true,
                Entry::Ready(sample) => {
                    now.duration_since(sample.at) < REFRESH_INTERVAL
                }
            });
            // Keep the reservation for the entire call without blocking
            // readers or refreshes of other volumes.
            samples.insert(volume.clone(), Entry::InFlight);
        }
        let available = probe.important_available(&volume);
        self.samples.lock().ok()?.insert(
            volume,
            Entry::Ready(Sample {
                at: clock(),
                raw_available,
                available,
            }),
        );
        available
    }
}

static CAPACITY: CapacitySampler = CapacitySampler::new();

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

fn mount_root(path: &Path) -> Option<PathBuf> {
    let stat = rustix::fs::statfs(path).ok()?;
    let root = mount_field(&stat.f_mntonname);
    (!root.is_empty()).then(|| PathBuf::from(root))
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
    // SPACEUSED is volume allocation, excluding the shared free pool.
    let buffer = volume_attribute::<12>(path, 0x0080_0000)?;
    Some(u64::from_ne_bytes(buffer[4..].try_into().ok()?))
}

fn volume_uuid(path: &Path) -> Option<[u8; 16]> {
    // ATTR_VOL_UUID identifies the filesystem across changes in device
    // numbers. A zero UUID does not identify a volume.
    let buffer = volume_attribute::<20>(path, 0x0004_0000)?;
    let uuid = buffer[4..].try_into().ok()?;
    (uuid != [0; 16]).then_some(uuid)
}

fn volume_attribute<const N: usize>(
    path: &Path,
    attribute: u32,
) -> Option<[u8; N]> {
    let path = CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut attrs = AttrList {
        bitmapcount: 5,
        reserved: 0,
        commonattr: 0,
        // ATTR_VOL_INFO is required when asking for any volume attribute.
        volattr: 0x8000_0000 | attribute,
        dirattr: 0,
        fileattr: 0,
        forkattr: 0,
    };
    let mut buffer = [0_u8; N];
    // SAFETY: path is NUL-terminated, the attribute list has the Darwin ABI,
    // and buffer.len() is the exact size of the writable byte buffer.
    let result = unsafe {
        getattrlist(
            path.as_ptr(),
            &raw mut attrs,
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            0,
        )
    };
    if result != 0
        || u32::from_ne_bytes(buffer.get(..4)?.try_into().ok()?) as usize != N
    {
        return None;
    }
    Some(buffer)
}

pub fn presentation(
    path: &Path,
    total: u64,
    raw_available: u64,
    refresh: bool,
) -> Option<NativeSpace> {
    let root = mount_root(path)?;
    let used =
        if root == Path::new("/") || root == Path::new(MACOS_DATA_VOLUME) {
            // Finder presents the startup System/Data volume group. Preboot,
            // Recovery and VM also consume shared capacity, but not this figure.
            volume_used(Path::new("/"))
                .zip(volume_used(Path::new(MACOS_DATA_VOLUME)))
                .and_then(|(system, data)| system.checked_add(data))
        } else {
            volume_used(&root)
        }
        .filter(|used| *used <= total);
    // This API already includes purgeable space. Zero while raw free is
    // positive (as in a restricted process) is unavailable, not a measurement.
    let available = volume_uuid(&root)
        .and_then(|uuid| {
            CAPACITY.available(
                MountedVolume { root, uuid },
                raw_available,
                refresh,
                Instant::now,
                &mut SystemProbe { raw_available },
            )
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
        fn important_available(&mut self, _: &MountedVolume) -> Option<u64> {
            self.calls += 1;
            self.result
        }
    }

    fn fixture(id: u8) -> MountedVolume {
        MountedVolume {
            root: PathBuf::from("/Volumes/Fixture"),
            uuid: [id; 16],
        }
    }

    proptest::proptest! {
        #![proptest_config(proptest::test_runner::Config {
            failure_persistence: None,
            ..proptest::test_runner::Config::default()
        })]

        #[test]
        fn refreshes_within_a_minute_share_one_probe(
            offsets in proptest::collection::vec(1..60_000_u64, 1..100),
            raw in 1_u64..100,
            changed in 100_u64..200,
            result in proptest::option::of(200_u64..300),
        ) {
            let sampler = CapacitySampler::new();
            let mut probe = CountingProbe { calls: 0, result };
            let start = Instant::now();
            let mut read = |bytes, now| {
                sampler.available(fixture(1), bytes, true, || now, &mut probe)
            };
            proptest::prop_assert_eq!(read(raw, start), result);
            for offset in offsets {
                let now = start + Duration::from_millis(offset);
                proptest::prop_assert_eq!(read(changed, now), None);
                proptest::prop_assert_eq!(read(raw, now), result);
            }
            proptest::prop_assert_eq!(probe.calls, CALLS_PER_VOLUME_REFRESH);
        }
    }

    #[test]
    fn changed_raw_free_space_hides_the_old_estimate() {
        let cache = CapacitySampler::new();
        let mut probe = CountingProbe {
            calls: 0,
            result: Some(110),
        };
        let start = Instant::now();
        let mut read = |raw, refresh| {
            cache.available(fixture(1), raw, refresh, || start, &mut probe)
        };
        assert_eq!(read(100, true), Some(110));
        assert_eq!(read(80, false), None);
        assert_eq!(read(80, true), None);
        assert_eq!(probe.calls, CALLS_PER_VOLUME_REFRESH);
    }

    #[test]
    fn probe_can_take_the_cache_lock() {
        struct LockCheckingProbe<'a> {
            cache: &'a CapacitySampler,
            start: Instant,
        }
        impl CapacityProbe for LockCheckingProbe<'_> {
            fn important_available(
                &mut self,
                _: &MountedVolume,
            ) -> Option<u64> {
                assert!(self.cache.samples.try_lock().is_ok());
                let mut other = CountingProbe {
                    calls: 0,
                    result: Some(120),
                };
                let later = || self.start + Duration::from_mins(2);
                // Even a stalled call keeps its reservation, while the
                // other volume can be read or refreshed independently.
                assert_eq!(
                    self.cache.available(
                        fixture(1),
                        100,
                        true,
                        later,
                        &mut other
                    ),
                    None
                );
                assert_eq!(
                    self.cache.available(
                        fixture(2),
                        100,
                        false,
                        later,
                        &mut other
                    ),
                    None
                );
                assert_eq!(other.calls, CALLS_PER_CACHED_SAMPLE);
                assert_eq!(
                    self.cache.available(
                        fixture(2),
                        100,
                        true,
                        later,
                        &mut other
                    ),
                    Some(120)
                );
                assert_eq!(other.calls, CALLS_PER_VOLUME_REFRESH);
                Some(110)
            }
        }
        let cache = CapacitySampler::new();
        let start = Instant::now();
        let mut probe = LockCheckingProbe {
            cache: &cache,
            start,
        };
        assert_eq!(
            cache.available(fixture(1), 100, true, || start, &mut probe),
            Some(110)
        );
    }

    #[test]
    fn unavailable_sample_is_reused_on_the_next_tick() {
        let sampler = CapacitySampler::new();
        let mut probe = CountingProbe {
            calls: 0,
            result: None,
        };
        let start = Instant::now();
        let mut read =
            |now| sampler.available(fixture(1), 100, true, || now, &mut probe);
        assert_eq!(read(start), None);
        assert_eq!(read(start + Duration::from_millis(1)), None);
        assert_eq!(probe.calls, CALLS_PER_VOLUME_REFRESH);
    }

    #[test]
    fn replacement_at_the_same_mount_has_its_own_budget() {
        let cache = CapacitySampler::new();
        let mut probe = CountingProbe {
            calls: 0,
            result: Some(110),
        };
        let now = Instant::now;
        assert_eq!(
            cache.available(fixture(1), 100, true, now, &mut probe),
            Some(110)
        );
        probe.result = Some(120);
        assert_eq!(
            cache.available(fixture(2), 100, false, now, &mut probe),
            None
        );
        assert_eq!(
            cache.available(fixture(2), 100, true, now, &mut probe),
            Some(120)
        );
        assert_eq!(probe.calls, 2 * CALLS_PER_VOLUME_REFRESH);
    }

    #[test]
    fn cached_reads_are_free_and_a_hit_does_not_extend_expiry() {
        let sampler = CapacitySampler::new();
        let mut probe = CountingProbe {
            calls: 0,
            result: Some(110),
        };
        let start = Instant::now();
        let mut read = |now, refresh| {
            sampler.available(fixture(1), 100, refresh, || now, &mut probe)
        };
        assert_eq!(read(start, false), None);
        assert_eq!(read(start, true), Some(110));
        assert_eq!(read(start + Duration::from_secs(59), true), Some(110));
        let expired = start + Duration::from_mins(1);
        assert_eq!(read(expired, false), None);
        assert_eq!(probe.calls, CALLS_PER_VOLUME_REFRESH);
        probe.result = Some(120);
        assert_eq!(
            sampler.available(fixture(1), 100, true, || expired, &mut probe),
            Some(120)
        );
        assert_eq!(probe.calls, 2 * CALLS_PER_VOLUME_REFRESH);
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
