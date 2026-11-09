//! Read-only removal preflight using the remover's directory boundaries.

use std::os::fd::{AsFd as _, BorrowedFd, OwnedFd};
#[cfg(not(target_os = "macos"))]
use std::os::unix::ffi::OsStrExt as _;
use std::path::Path;

#[cfg(not(target_os = "macos"))]
use rustix::fs::Dir;
use rustix::fs::{AtFlags, CWD, FileType, Stat, statat};

use super::{Volume, open_dir};

#[cfg(test)]
thread_local! {
    static ENTRY_STATS: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
}

fn entry_stat(
    parent: impl std::os::fd::AsFd,
    name: impl rustix::path::Arg,
) -> rustix::io::Result<Stat> {
    #[cfg(test)]
    ENTRY_STATS.set(ENTRY_STATS.get() + 1);
    statat(parent, name, AtFlags::SYMLINK_NOFOLLOW)
}

/// Inspect every entry without giving descendant paths to the kernel.
///
/// Full paths can exceed `PATH_MAX` even though each component is legal.
/// Directory streams retain only open ancestors, so neither a wide directory
/// nor a deep tree requires a path queue or recursive calls. The caller can
/// refuse any entry, including to enforce a shared selection budget.
pub fn inspect(
    path: &Path,
    mut visit: impl FnMut(&Path, Option<BorrowedFd<'_>>) -> Result<(), String>,
) -> Result<(), String> {
    let failure = |error| format!("Cannot inspect {}: {error}", path.display());
    let stat = match entry_stat(CWD, path) {
        Ok(stat) => stat,
        Err(rustix::io::Errno::NOENT) => return Ok(()),
        Err(error) => return Err(failure(error)),
    };
    if FileType::from_raw_mode(stat.st_mode) != FileType::Directory {
        return visit(path, None);
    }
    let dir = open_dir(CWD, path).map_err(failure)?;
    let top = Volume::of(&dir).map_err(|error| {
        format!("Cannot inspect {}: {error}", path.display())
    })?;
    visit(path, Some(dir.as_fd()))?;
    let mut frames =
        vec![listing(dir).map_err(|error| {
            format!("Cannot list {}: {error}", path.display())
        })?];
    let mut current = path.to_owned();
    while let Some(entries) = frames.last_mut() {
        let Some(entry) = next_entry(entries).map_err(|error| {
            format!("Cannot list {}: {error}", current.display())
        })?
        else {
            frames.pop();
            current.pop();
            continue;
        };
        current.push(entry.name());
        if entry.directory {
            let parent = listing_fd(entries).map_err(|error| {
                format!("Cannot inspect {}: {error}", current.display())
            })?;
            let dir = open_dir(parent, entry.name()).map_err(|error| {
                format!("Cannot open {}: {error}", current.display())
            })?;
            let volume = Volume::of(&dir).map_err(|error| {
                format!("Cannot inspect {}: {error}", current.display())
            })?;
            if !top.contains(&volume) {
                return Err(format!(
                    "Cannot inspect {}: another filesystem is mounted there",
                    current.display()
                ));
            }
            visit(&current, Some(dir.as_fd()))?;
            frames.push(listing(dir).map_err(|error| {
                format!("Cannot list {}: {error}", current.display())
            })?);
        } else {
            visit(&current, None)?;
            current.pop();
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
type Listing = crate::macos::ReadDir;
#[cfg(not(target_os = "macos"))]
type Listing = Dir;

struct ListedEntry {
    #[cfg(target_os = "macos")]
    entry: crate::macos::Entry,
    #[cfg(not(target_os = "macos"))]
    entry: rustix::fs::DirEntry,
    directory: bool,
}

impl ListedEntry {
    fn name(&self) -> &std::ffi::OsStr {
        #[cfg(target_os = "macos")]
        return self.entry.file_name();
        #[cfg(not(target_os = "macos"))]
        std::ffi::OsStr::from_bytes(self.entry.file_name().to_bytes())
    }
}

#[cfg(target_os = "macos")]
#[allow(
    clippy::unnecessary_wraps,
    reason = "Match the fallible directory-stream constructor on other Unix platforms"
)]
fn listing(dir: OwnedFd) -> std::io::Result<Listing> {
    let mut entries =
        crate::macos::read_dir_from(dir, crate::macos::Clones::Off);
    // Each directory is reopened without following links and checked against
    // the root volume before descent. A second metadata read in the listing
    // would duplicate that check and could not replace it.
    entries.use_listed_directories();
    Ok(entries)
}

#[cfg(not(target_os = "macos"))]
fn listing(dir: OwnedFd) -> std::io::Result<Listing> {
    Ok(Dir::new(dir)?)
}

#[allow(
    clippy::unnecessary_wraps,
    reason = "The directory-stream handle accessor can fail on other Unix platforms"
)]
fn listing_fd(entries: &Listing) -> std::io::Result<BorrowedFd<'_>> {
    #[cfg(target_os = "macos")]
    return Ok(entries.fd());
    #[cfg(not(target_os = "macos"))]
    Ok(entries.fd()?)
}

#[cfg(target_os = "macos")]
fn next_entry(entries: &mut Listing) -> std::io::Result<Option<ListedEntry>> {
    entries.next().transpose().map(|entry| {
        entry.map(|entry| ListedEntry {
            directory: entry.kind() == crate::macos::Kind::Directory,
            entry,
        })
    })
}

#[cfg(not(target_os = "macos"))]
fn next_entry(entries: &mut Listing) -> std::io::Result<Option<ListedEntry>> {
    loop {
        let Some(entry) = entries.next() else {
            return Ok(None);
        };
        let entry = entry?;
        let name = entry.file_name();
        if name == c"." || name == c".." {
            continue;
        }
        let stat = entry_stat(listing_fd(entries)?, name)?;
        return Ok(Some(ListedEntry {
            entry,
            directory: FileType::from_raw_mode(stat.st_mode)
                == FileType::Directory,
        }));
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn bulk_inspection_refuses_a_real_nested_mount_before_visiting_it() {
        use std::os::unix::fs::MetadataExt as _;

        let temp = tempfile::tempdir_in("/tmp").expect("fixture");
        let root = temp.path().canonicalize().expect("root");
        let mounted = root.join("mounted");
        let _image = crate::test_paths::DiskImage::mount_at(&mounted);
        let foreign = mounted.join("keep");
        std::fs::write(&foreign, b"outside the root volume").expect("file");
        assert_ne!(
            std::fs::metadata(&root).expect("root device").dev(),
            std::fs::metadata(&mounted).expect("mounted device").dev()
        );
        let mut visited = Vec::new();
        let reason = inspect(&root, |path, _| {
            visited.push(path.to_owned());
            Ok(())
        })
        .expect_err("foreign volume");
        assert!(reason.contains("another filesystem"), "{reason}");
        assert!(!visited.iter().any(|path| path.starts_with(&mounted)));
        assert_eq!(
            std::fs::read(&foreign).expect("untouched"),
            b"outside the root volume"
        );
    }

    fn oracle(
        path: &Path,
        relative: &Path,
        out: &mut std::collections::BTreeMap<std::path::PathBuf, bool>,
    ) {
        let directory = std::fs::symlink_metadata(path)
            .expect("oracle metadata")
            .is_dir();
        out.insert(relative.to_owned(), directory);
        if directory {
            for entry in std::fs::read_dir(path).expect("oracle listing") {
                let entry = entry.expect("oracle entry");
                oracle(&entry.path(), &relative.join(entry.file_name()), out);
            }
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 32,
            failure_persistence: None,
            ..ProptestConfig::default()
        })]

        // A flat listing must agree with the filesystem without one metadata
        // syscall per file. This operation bound is independent of disk speed.
        #[test]
        fn bulk_inspection_matches_the_filesystem_without_per_file_stats(
            files in 2_usize..80,
        ) {
            let temp = tempfile::tempdir_in("/tmp").expect("fixture");
            let root = temp.path().canonicalize().expect("root");
            let mut expected = std::collections::BTreeSet::new();
            expected.insert(root.clone());
            for index in 0..files {
                let path = root.join(format!("file-{index}"));
                std::fs::write(&path, [b'x'; 17]).expect("file");
                expected.insert(path);
            }
            let mut found = std::collections::BTreeSet::new();
            ENTRY_STATS.set(0);
            crate::macos::LISTING_STATS.set(0);
            inspect(&root, |path, _| {
                found.insert(path.to_owned());
                Ok(())
            }).expect("inspect");
            prop_assert_eq!(found, expected);
            let calls = ENTRY_STATS.get() + crate::macos::LISTING_STATS.get();
            prop_assert!(calls <= 1,
                "listing {} files made {} individual stats", files, calls);
        }

        // A second traversal, using lstat and no bulk attributes, is the
        // oracle for descendants and link boundaries under both root aliases.
        #[test]
        fn bulk_inspection_matches_a_nofollow_oracle(
            kinds in prop::collection::vec(0_u8..4, 1..40),
        ) {
            let temp = tempfile::tempdir_in("/tmp").expect("fixture");
            let outside = tempfile::tempdir_in("/tmp").expect("outside");
            std::fs::write(outside.path().join("keep"), b"outside")
                .expect("outside file");
            for (index, kind) in kinds.into_iter().enumerate() {
                let parent = temp.path().join(format!("a/{}/b", index % 3));
                std::fs::create_dir_all(&parent).expect("parents");
                let path = parent.join(format!("{index}-é-\\-\n"));
                match kind {
                    0 => std::fs::create_dir(&path).expect("directory"),
                    1 => std::fs::write(&path, b"payload").expect("file"),
                    2 => std::os::unix::fs::symlink(outside.path(), &path)
                        .expect("directory link"),
                    _ => std::os::unix::fs::symlink("missing", &path)
                        .expect("dangling link"),
                }
            }
            let canonical = temp.path().canonicalize().expect("canonical");
            let mut expected = std::collections::BTreeMap::new();
            oracle(&canonical, Path::new(""), &mut expected);
            for root in [temp.path(), canonical.as_path()] {
                let mut found = std::collections::BTreeMap::new();
                inspect(root, |path, directory| {
                    found.insert(path.strip_prefix(root).expect("under root")
                        .to_owned(), directory.is_some());
                    Ok(())
                }).expect("inspect");
                prop_assert_eq!(&found, &expected);
            }
            prop_assert!(outside.path().join("keep").exists());
        }
    }
}
