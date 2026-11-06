//! Read-only removal preflight using the remover's directory boundaries.

use std::os::fd::{AsFd as _, BorrowedFd};
use std::os::unix::ffi::OsStrExt as _;
use std::path::Path;

use rustix::fs::{AtFlags, CWD, Dir, FileType, Stat, statat};

use super::{Volume, open_dir};

/// Inspect every entry without giving descendant paths to the kernel.
///
/// Full paths can exceed `PATH_MAX` even though each component is legal.
/// Directory streams retain only open ancestors, so neither a wide directory
/// nor a deep tree requires a path queue or recursive calls. The caller can
/// refuse any entry, including to enforce a shared selection budget.
pub fn inspect(
    path: &Path,
    mut visit: impl FnMut(
        &Path,
        &Stat,
        Option<BorrowedFd<'_>>,
    ) -> Result<(), String>,
) -> Result<(), String> {
    let failure = |error| format!("Cannot inspect {}: {error}", path.display());
    let stat = match statat(CWD, path, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(stat) => stat,
        Err(rustix::io::Errno::NOENT) => return Ok(()),
        Err(error) => return Err(failure(error)),
    };
    if FileType::from_raw_mode(stat.st_mode) != FileType::Directory {
        return visit(path, &stat, None);
    }
    let dir = open_dir(CWD, path).map_err(failure)?;
    let top = Volume::of(&dir).map_err(|error| {
        format!("Cannot inspect {}: {error}", path.display())
    })?;
    visit(path, &top.device, Some(dir.as_fd()))?;
    let mut frames = vec![Dir::new(dir).map_err(failure)?];
    let mut current = path.to_owned();
    while let Some(entries) = frames.last_mut() {
        let failure =
            |error| format!("Cannot inspect {}: {error}", current.display());
        let Some(entry) = entries.next() else {
            frames.pop();
            current.pop();
            continue;
        };
        let entry = entry.map_err(failure)?;
        let name = entry.file_name();
        if name == c"." || name == c".." {
            continue;
        }
        let parent = entries.fd().map_err(failure)?;
        let stat =
            statat(parent, name, AtFlags::SYMLINK_NOFOLLOW).map_err(failure)?;
        current.push(std::ffi::OsStr::from_bytes(name.to_bytes()));
        if FileType::from_raw_mode(stat.st_mode) == FileType::Directory {
            let dir = open_dir(parent, name).map_err(|error| {
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
            visit(&current, &volume.device, Some(dir.as_fd()))?;
            frames.push(Dir::new(dir).map_err(|error| {
                format!("Cannot list {}: {error}", current.display())
            })?);
        } else {
            visit(&current, &stat, None)?;
            current.pop();
        }
    }
    Ok(())
}
