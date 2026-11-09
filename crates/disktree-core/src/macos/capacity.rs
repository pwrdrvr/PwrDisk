//! Native presentation counters, never inputs to removal measurements.

use std::ffi::{CString, c_char, c_int, c_void};
use std::os::unix::ffi::OsStrExt as _;
use std::path::{Path, PathBuf};

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

fn important_available(path: &Path) -> Option<u64> {
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
    let length = stat.f_mntonname.iter().position(|byte| *byte == 0)?;
    let bytes: Vec<_> = stat.f_mntonname[..length]
        .iter()
        .map(|byte| *byte as u8)
        .collect();
    Some(PathBuf::from(std::ffi::OsStr::from_bytes(&bytes)))
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
    let available = important_available(&root)
        .filter(|available| *available >= raw_available && *available <= total);
    (used.is_some() || available.is_some())
        .then_some(NativeSpace { used, available })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

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
            let display = presentation(&root, raw.total, raw.available)
                .expect("native counters");
            assert_eq!(display.used, Some(after));
            if let Some(available) = display.available {
                assert!((raw.available..=raw.total).contains(&available));
            }
        }
        drop(image);
    }
}
