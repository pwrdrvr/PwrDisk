//! Real aliases and mounts for the path-identity oracles, never fake tables.

use std::fs;
use std::os::unix::fs::{MetadataExt as _, symlink};
use std::path::{Path, PathBuf};

pub fn id(path: &Path) -> Option<(u64, u64)> {
    fs::symlink_metadata(path)
        .ok()
        .map(|meta| (meta.dev(), meta.ino()))
}

pub fn spellings(path: &Path, link: &Path) -> Vec<PathBuf> {
    let real = path.canonicalize().expect("real path");
    symlink(&real, link).expect("root alias");
    let mut paths = vec![real.clone(), link.to_path_buf(), path.to_path_buf()];
    #[cfg(target_os = "macos")]
    {
        if let Ok(rest) = real.strip_prefix("/private") {
            paths.push(Path::new("/").join(rest));
        }
        paths.push(
            Path::new("/System/Volumes/Data")
                .join(real.strip_prefix("/").expect("absolute path")),
        );
        paths.push(
            Path::new("/Volumes/Macintosh HD")
                .join(real.strip_prefix("/").expect("absolute path")),
        );
    }
    // A volume alias is machine-specific. Only test names stat confirms
    // are the same directory; no assumption about the boot volume's name.
    paths.retain(|path| {
        fs::metadata(path)
            .is_ok_and(|meta| Some((meta.dev(), meta.ino())) == id(&real))
    });
    let trailing: Vec<_> = paths.iter().map(|path| path.join("")).collect();
    paths.extend(trailing);
    paths.sort();
    paths.dedup();
    paths
}

#[cfg(target_os = "macos")]
pub struct DiskImage {
    mount: PathBuf,
    _image: tempfile::TempDir,
}

#[cfg(target_os = "macos")]
fn hdiutil(args: &[&std::ffi::OsStr]) {
    let output = std::process::Command::new("hdiutil")
        .args(args)
        .output()
        .expect("hdiutil");
    assert!(
        output.status.success(),
        "hdiutil {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(target_os = "macos")]
impl DiskImage {
    pub fn mount_at(mount: &Path) -> Self {
        let image = tempfile::tempdir().expect("image directory");
        let file = image.path().join("boundary.dmg");
        hdiutil(&[
            "create".as_ref(),
            "-size".as_ref(),
            "16m".as_ref(),
            "-fs".as_ref(),
            "HFS+".as_ref(),
            "-volname".as_ref(),
            "PwrDiskPathTest".as_ref(),
            file.as_os_str(),
        ]);
        fs::create_dir_all(mount).expect("mount directory");
        hdiutil(&[
            "attach".as_ref(),
            "-nobrowse".as_ref(),
            "-noautoopen".as_ref(),
            "-mountpoint".as_ref(),
            mount.as_os_str(),
            file.as_os_str(),
        ]);
        Self {
            mount: mount.to_path_buf(),
            _image: image,
        }
    }
}

#[cfg(target_os = "macos")]
impl Drop for DiskImage {
    fn drop(&mut self) {
        hdiutil(&[
            "detach".as_ref(),
            "-force".as_ref(),
            self.mount.as_os_str(),
        ]);
    }
}
