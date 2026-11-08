//! Native effects can be replaced by a device-ownership model, never by
//! running commands against invented device identifiers.

use std::fs;
use std::path::Path;
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
    root: &Path,
    force: bool,
    ops: &mut impl Operations,
) -> Result<(), String> {
    if force {
        let unmount = ops.run(
            Command::new("diskutil")
                .args(["unmountDisk", "force"])
                .arg(root),
        );
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
    volume: &Path,
    ops: &mut impl Operations,
) -> Result<Output, String> {
    ops.run(
        Command::new("diskutil")
            .args(["unmount", "force"])
            .arg(volume),
    )?;
    fs::create_dir_all(root).map_err(|error| error.to_string())?;
    ops.run(
        Command::new("diskutil")
            .args(["mount", "nobrowse", "-mountPoint"])
            .arg(root)
            .arg(volume),
    )?;
    ops.run(Command::new("df").arg("-kP").arg(root))
}

mod model;
