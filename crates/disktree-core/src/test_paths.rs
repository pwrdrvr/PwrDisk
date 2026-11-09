//! Real aliases and mounts for the path-identity oracles, never fake tables.

use std::fs;
use std::os::unix::fs::{MetadataExt as _, symlink};
use std::path::{Path, PathBuf};

#[cfg(target_os = "macos")]
pub mod disk_image;

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
    image: Option<disk_image::Image>,
}

#[cfg(target_os = "macos")]
impl DiskImage {
    pub fn mount_at(mount: &Path) -> Self {
        Self {
            image: Some(disk_image::Image::mount_at(mount)),
        }
    }
}

#[cfg(target_os = "macos")]
impl Drop for DiskImage {
    fn drop(&mut self) {
        if let Some(image) = self.image.take() {
            if std::thread::panicking() {
                // Its guard retains backing files if cleanup is uncertain.
                drop(image);
            } else {
                image.close();
            }
        }
    }
}
