//! `pwrdisk logs-fixture LOG`, in debug builds only: the production Logs
//! window over a log the caller invented, for native accessibility checks
//! and screenshots of a log nobody minds showing.
//!
//! It lives in the binary, rather than an example that re-declares the
//! binary's modules, so a module added to `main.rs` cannot break it. It opens
//! that one window and nothing else: no main window, so no scan and no
//! removal; no agent link and no startup update check; no saved Logs view,
//! read or written; and `main` takes this path before it opens `main.log`,
//! so the real log is never written either. The menu bar is the app's, so
//! its Settings and Check for Updates still do what they do there; a check
//! that wants the fixture untouched leaves them alone. Release builds leave
//! it out, which is why the usage text does not mention it.
//!
//! `cargo run -p disktree-app -- logs-fixture /tmp/invented.log`

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

use anyhow::{Context as _, Result};

const COMMAND: &str = "logs-fixture";

/// Runs the fixture if the command line asks for it, and says how it went;
/// `None` when it asks for something else.
pub fn run_if_asked() -> Option<Result<()>> {
    let own_logs = crate::diag_log::log_dir();
    requested(std::env::args_os().skip(1), own_logs.as_deref())
        .transpose()
        .map(|log| log.map(open))
}

/// The log `logs-fixture LOG` names, program name already skipped. It
/// must be a file outside `own_logs`, the folder PwrDisk writes its own
/// log to, which can hold real paths.
fn requested(
    args: impl IntoIterator<Item = OsString>,
    own_logs: Option<&Path>,
) -> Result<Option<PathBuf>> {
    let mut args = args.into_iter();
    if args.next().is_none_or(|arg| arg != COMMAND) {
        return Ok(None);
    }
    let (Some(log), None) = (args.next(), args.next()) else {
        anyhow::bail!("usage: pwrdisk {COMMAND} LOG");
    };
    let log = dunce::canonicalize(&log).with_context(|| {
        format!("cannot read {}", Path::new(&log).display())
    })?;
    anyhow::ensure!(log.is_file(), "{} is not a file", log.display());
    // Canonical on both sides, so a symlink or `..` cannot lead back in.
    if let Some(own) = own_logs.and_then(|dir| dunce::canonicalize(dir).ok()) {
        anyhow::ensure!(
            !log.starts_with(&own),
            "{} is in PwrDisk's own log folder; invent a log elsewhere",
            log.display()
        );
    }
    Ok(Some(log))
}

/// Logs over `log`, with the menu bar so its shortcuts work as they do in
/// the app. Closing it quits, as closing the last window does there.
fn open(log: PathBuf) {
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            gpui_omarchy::init(cx);
            crate::app_menu::install(cx);
            crate::logs_window::open_window(Some(log), None, cx);
            cx.activate(true);
        });
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    use super::*;

    fn ask(args: &[&OsStr], own: Option<&Path>) -> Result<Option<PathBuf>> {
        requested(args.iter().map(|&arg| arg.to_owned()), own)
    }

    #[test]
    fn other_commands_are_not_the_fixture() {
        for args in [&[][..], &["mcp"], &["."], &["--logs-fixture", "x"]] {
            let args: Vec<&OsStr> = args.iter().map(OsStr::new).collect();
            assert!(ask(&args, None).expect("not ours").is_none());
        }
    }

    #[test]
    fn opens_only_an_existing_file_outside_the_app_logs() {
        let temp = tempfile::tempdir().expect("tempdir");
        let invented = temp.path().join("invented.log");
        std::fs::write(&invented, "INFO invented\n").expect("log");
        let own = temp.path().join("own");
        std::fs::create_dir(&own).expect("own logs");
        let real = own.join("main.log");
        std::fs::write(&real, "INFO real\n").expect("real log");

        let command = OsStr::new(COMMAND);
        let found = ask(&[command, invented.as_os_str()], Some(&own))
            .expect("invented log")
            .expect("the fixture");
        assert_eq!(found, dunce::canonicalize(&invented).expect("canonical"));

        let missing = temp.path().join("missing.log");
        // Through `..`, to the real log all the same.
        let around = temp.path().join("own/../own/main.log");
        // Other case: macOS and Windows canonicalize it to the folder's own
        // case, so it is refused there too; on Linux it does not exist.
        let cased = temp.path().join("OWN/main.log");
        for args in [
            &[command][..],
            &[command, missing.as_os_str()],
            &[command, temp.path().as_os_str()],
            &[command, invented.as_os_str(), OsStr::new("extra")],
            &[command, real.as_os_str()],
            &[command, around.as_os_str()],
            &[command, cased.as_os_str()],
        ] {
            assert!(ask(args, Some(&own)).is_err(), "{args:?} is refused");
        }
    }
}
