//! Real descendants beyond the kernel's full-path lookup limit.

use std::io::Write as _;
use std::os::fd::OwnedFd;
use std::path::PathBuf;

use rustix::fs::{CWD, Mode, OFlags, mkdirat, openat};

#[derive(Debug)]
pub struct DeepTree {
    _temp: tempfile::TempDir,
    pub selected: PathBuf,
    pub deepest: PathBuf,
    pub dir: OwnedFd,
}

impl DeepTree {
    pub fn new() -> Self {
        let temp = tempfile::tempdir_in("/tmp").expect("fixture");
        let root = std::fs::canonicalize(temp.path()).expect("real root");
        let selected = root.join("selected");
        std::fs::create_dir(&selected).expect("selected");
        std::fs::write(root.join("keep"), b"unmarked").expect("sibling");
        let mut dir = openat(
            CWD,
            &selected,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .expect("open selected");
        let mut deepest = selected.clone();
        let name = "s".repeat(200);
        // Exceeds both macOS's 1024-byte and Linux's 4096-byte PATH_MAX,
        // without exceeding a component's limit or needing many handles.
        for _ in 0..32 {
            mkdirat(&dir, name.as_str(), Mode::RWXU).expect("deep directory");
            dir = openat(
                &dir,
                name.as_str(),
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .expect("open deep directory");
            deepest.push(&name);
        }
        let file = openat(
            &dir,
            ".payload",
            OFlags::WRONLY | OFlags::CREATE | OFlags::CLOEXEC,
            Mode::RUSR | Mode::WUSR,
        )
        .expect("deep file");
        std::fs::File::from(file)
            .write_all(&[b'x'; 16_384])
            .expect("payload");
        assert_eq!(
            std::fs::symlink_metadata(&deepest)
                .expect_err("full path cannot be resolved")
                .raw_os_error(),
            Some(rustix::io::Errno::NAMETOOLONG.raw_os_error())
        );
        Self {
            _temp: temp,
            selected,
            deepest,
            dir,
        }
    }
}
