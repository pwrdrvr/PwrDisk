//! Native effects can be replaced by a device-ownership model, never by
//! running commands against invented device identifiers.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

pub(super) trait Operations {
    fn owned(
        &mut self,
        backing: &Path,
    ) -> Result<Option<Vec<serde_json::Value>>, String>;

    fn run(&mut self, command: &mut Command) -> Result<Output, String>;
}

pub(super) struct Native;

impl Operations for Native {
    fn owned(
        &mut self,
        backing: &Path,
    ) -> Result<Option<Vec<serde_json::Value>>, String> {
        super::owned_entities(backing)
    }

    fn run(&mut self, command: &mut Command) -> Result<Output, String> {
        super::run(command)
    }
}

pub(super) fn eject(
    backing: &Path,
    force: bool,
    ops: &mut impl Operations,
) -> Result<(), String> {
    let Some(root) = cleanup_target(backing, ops)? else {
        return Ok(());
    };
    if force {
        let unmount = ops.run(
            Command::new("diskutil")
                .args(["unmountDisk", "force"])
                .arg(&root),
        );
        // Unmounting can retire the attachment even when it reports failure.
        // Its old number can already name somebody else's disk at this point.
        let root = match cleanup_target(backing, ops) {
            Ok(Some(root)) => root,
            Ok(None) => return Ok(()),
            Err(query) => {
                return Err(match unmount {
                    Ok(_) => query,
                    Err(unmount) => format!("{unmount}\n{query}"),
                });
            }
        };
        return ops
            .run(
                Command::new("hdiutil")
                    .args(["detach", "-force", "-debug"])
                    .arg(root),
            )
            .map(|_| ())
            .map_err(|detach| match unmount {
                Ok(_) => detach,
                Err(unmount) => format!("{unmount}\n{detach}"),
            });
    }
    ops.run(Command::new("diskutil").arg("eject").arg(root))
        .map(|_| ())
}

pub(super) fn checkpoint(
    root: &Path,
    backing: &Path,
    ops: &mut impl Operations,
) -> Result<Output, String> {
    unmount(backing, ops)?;
    fs::create_dir_all(root).map_err(|error| error.to_string())?;
    let entries = required_entries(backing, ops)?;
    let volume = volume_device(&entries)?;
    ops.run(
        Command::new("diskutil")
            .args(["mount", "nobrowse", "-mountPoint"])
            .arg(root)
            .arg(&volume),
    )?;
    // If the image disappeared, df would silently measure the placeholder's
    // host volume. Verify the mounted entry before using it as an oracle.
    let entries = required_entries(backing, ops)?;
    if volume_device(&entries)? != volume {
        return Err("mounted volume identity changed".into());
    }
    let record = entries
        .iter()
        .find(|entry| {
            entry["dev-entry"].as_str().map(Path::new) == Some(&volume)
        })
        .ok_or("mounted volume no longer belongs to the backing image")?;
    let mounted = record["mount-point"]
        .as_str()
        .ok_or("owned volume is not mounted")?;
    let real_root = root.canonicalize().map_err(|error| error.to_string())?;
    if Path::new(mounted)
        .canonicalize()
        .map_err(|error| error.to_string())?
        != real_root
    {
        return Err("owned volume is mounted at a different path".into());
    }
    ops.run(Command::new("df").arg("-kP").arg(root))
}

pub(super) fn unmount(
    backing: &Path,
    ops: &mut impl Operations,
) -> Result<Output, String> {
    let entries = required_entries(backing, ops)?;
    let volume = volume_device(&entries)?;
    ops.run(
        Command::new("diskutil")
            .args(["unmount", "force"])
            .arg(volume),
    )
}

fn required_entries(
    backing: &Path,
    ops: &mut impl Operations,
) -> Result<Vec<serde_json::Value>, String> {
    ops.owned(backing)?
        .ok_or_else(|| "backing image is not attached".into())
}

fn volume_device(entries: &[serde_json::Value]) -> Result<PathBuf, String> {
    // Missing, ambiguous or malformed identities are errors, not permission
    // to reuse a device remembered during setup.
    super::physical_device(entries)?;
    let container = super::container_device(entries)?
        .ok_or("backing image has no APFS container")?;
    super::only_child(entries, &container)
}

fn cleanup_target(
    backing: &Path,
    ops: &mut impl Operations,
) -> Result<Option<PathBuf>, String> {
    ops.owned(backing)?
        .map(|entries| super::cleanup_device(&entries))
        .transpose()
}

mod model;
