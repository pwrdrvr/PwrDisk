//! A usage report saved from the window's own scan, so Usage History has
//! something to compare without a trip to the command line.
//!
//! The chooser's "Save a usage report" is on unless it was turned off,
//! and the choice is remembered. It covers the scan the chooser starts
//! and nothing else: a rescan, a widening, a scan an agent asked for or
//! one named on the command line saves nothing on its own, so the folder
//! holds one report per scan the person chose rather than one per
//! keypress. File › Save Usage Report saves one of any other finished
//! local scan, once. The report is built from the finished tree off the
//! UI thread and is the same file `pwrdisk report --save` writes.

use std::io;
use std::path::{Path, PathBuf};
use std::time::Instant;

use disktree_core::identity::HostIdentity;
use disktree_core::path_style::PathStyle;
use disktree_core::scan::{ScanOptions, ScanSnapshot};
use disktree_core::space::{self, SpaceInfo};
use disktree_core::usage_report::{
    self, Capture, DEFAULT_THRESHOLD, Volume, library,
};
use gpui_kit::Context;
use gpui_omarchy::Status;

use crate::state::Disktree;

/// The chooser's checkbox.
pub const TICK_ID: &str = "source-save-report";

const ON: &str = "on";
const OFF: &str = "off";

/// Where the choice is kept, beside the Power Efficiency preset.
pub fn settings_path() -> Option<PathBuf> {
    crate::power::config_base()
        .map(|base| base.join("pwrdisk/save-usage-report"))
}

/// The remembered choice: on when none was saved.
pub fn load(path: &Path) -> io::Result<bool> {
    match std::fs::read_to_string(path) {
        Ok(value) => match value.trim() {
            ON => Ok(true),
            OFF => Ok(false),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "unknown usage report choice",
            )),
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(true),
        Err(error) => Err(error),
    }
}

pub fn save(path: &Path, on: bool) -> io::Result<()> {
    crate::power::write_setting(path, if on { ON } else { OFF })
}

/// What a report says of a scan that only its start knows.
#[derive(Clone, Debug)]
pub struct Started {
    pub space_before: Option<SpaceInfo>,
    /// Unix seconds.
    pub at: i64,
    pub clock: Instant,
}

impl Started {
    /// Samples the volume's space; call it before the walk.
    pub fn now(root: &Path) -> Self {
        Self {
            space_before: space::display_space_info(root).ok(),
            at: chrono::Utc::now().timestamp(),
            clock: Instant::now(),
        }
    }
}

/// What a finished local scan of `root` says about itself. Samples the
/// volume's space again and reads the mount table, so not for the UI
/// thread.
pub fn capture(
    root: &Path,
    options: ScanOptions,
    started: &Started,
    progress: ScanSnapshot,
) -> Capture {
    Capture {
        root: root.to_string_lossy().into_owned(),
        style: PathStyle::Native,
        captured_at: chrono::Utc::now().timestamp(),
        scan_started_at: Some(started.at),
        scan_elapsed_ms: u64::try_from(started.clock.elapsed().as_millis())
            .ok(),
        host: HostIdentity::current(),
        volume: Volume {
            device: space::device_for(root),
            mount: space::volume_root_for(root)
                .map(|mount| mount.to_string_lossy().into_owned()),
            space_before: started.space_before,
            space_after: space::display_space_info(root).ok(),
        },
        cancelled: progress.cancelled,
        unreadable: progress.errors,
        unreadable_detail: progress.messages,
        excluded: not_entered(root, options.one_filesystem),
        options,
    }
}

/// Mounts below `root` the walk does not enter, and automount points it
/// never touches. Aliases of folders it reaches by another path (macOS
/// firmlinks, repeated bind mounts) are not listed: their files are
/// counted once, where the walk found them.
fn not_entered(root: &Path, one_filesystem: bool) -> Vec<String> {
    let canonical = root.canonicalize().unwrap_or_else(|_| root.into());
    let data = space::data_volume_below(root, &canonical);
    let mut paths = space::never_scanned(root, &canonical);
    if one_filesystem
        && let Some(mounts) = space::mount_boundaries_for(&canonical)
    {
        // The walk enters the Data volume below `/` on purpose.
        paths.extend(
            mounts
                .into_iter()
                .filter(|path| Some(path) != data.as_ref()),
        );
    }
    paths.sort();
    paths.dedup();
    paths
        .into_iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect()
}

impl Disktree {
    /// Whether the chooser offers the checkbox: a local source, on a
    /// system with a folder to keep reports in. Reports scan this
    /// computer only.
    pub const fn report_offered(&self) -> bool {
        self.reports_dir.is_some() && !self.picker_ssh
    }

    /// The chooser's checkbox, remembered for the next launch. Said only
    /// when it could not be saved: the box itself shows the choice.
    pub fn toggle_save_report(&mut self, cx: &mut Context<'_, Self>) {
        self.save_report = !self.save_report;
        if let Some(path) = self.report_settings_path.as_deref()
            && let Err(error) = save(path, self.save_report)
        {
            self.say_until_closed(
                format!(
                    "Usage reports are {} for now, but the choice could \
                     not be saved: {error}",
                    if self.save_report { "on" } else { "off" }
                ),
                Status::Warning,
                cx,
            );
        }
        cx.notify();
    }

    /// The chooser just started a scan: its tree is to leave a report,
    /// if one was asked for.
    pub(crate) fn arm_report(&mut self) {
        self.report_wanted = (self.save_report
            && self.remote.is_none()
            && self.reports_dir.is_some())
        .then_some(self.scan_epoch);
    }

    /// A local scan landed its tree: write the report the chooser armed
    /// for it, if it did and the scan can still give one.
    pub(crate) fn save_pending_report(&mut self, cx: &Context<'_, Self>) {
        if self.report_wanted.take() == Some(self.scan_epoch)
            && self.report_refusal().is_none()
        {
            self.write_report(cx);
        }
    }

    /// Why the scan on screen cannot leave a report now, in words for a
    /// notice; `None` when it can. A report is one finished local scan as
    /// it was measured, once: half a tree would read in Usage History as
    /// folders that shrank, and the tree is not redrawn after a removal,
    /// so a report of it would list what is already gone.
    pub fn report_refusal(&self) -> Option<&'static str> {
        if self.reports_dir.is_none() {
            Some("This system has no folder to keep usage reports in.")
        } else if self.remote.is_some() {
            Some("Usage reports cover this computer only, not SSH hosts.")
        } else if self.is_indexing() {
            Some("The scan is still running. Save its report when it ends.")
        } else if self.tree.is_none() {
            Some("Nothing has been scanned yet, so there is no report.")
        } else if self.progress.cancelled {
            Some("This scan was stopped early. Rescan, then save its report.")
        } else if !self.removed.is_empty() {
            Some(
                "Files were removed since this scan. Rescan, then save \
                 its report.",
            )
        } else if self.report_written == Some(self.scan_epoch) {
            Some(
                "This scan\u{2019}s report is already saved. File \u{203a} \
                 Usage History lists it.",
            )
        } else {
            None
        }
    }

    /// File \u{203a} Save Usage Report: a report of the scan on screen,
    /// however it was started, or a notice saying why there is none.
    pub fn save_report_now(&mut self, cx: &mut Context<'_, Self>) {
        match self.report_refusal() {
            Some(why) => self.say(why, Status::Warning, cx),
            None => self.write_report(cx),
        }
    }

    /// Builds the report of the tree on screen and writes it, off the UI
    /// thread: a whole disk is tens of millions of nodes to total again.
    /// Callers have asked [`Self::report_refusal`].
    fn write_report(&mut self, cx: &Context<'_, Self>) {
        let (Some(tree), Some(dir), Some(started)) = (
            self.tree.clone(),
            self.reports_dir.clone(),
            self.report_started.clone(),
        ) else {
            return;
        };
        let epoch = self.scan_epoch;
        // Claimed now, so a second ask while this one writes is refused.
        self.report_written = Some(epoch);
        let root = self.root_path.clone();
        let options = self.options.clone();
        let progress = self.progress.clone();
        let written = cx.background_executor().spawn(async move {
            let capture = capture(&root, options, &started, progress);
            let report =
                usage_report::build(&tree, &capture, DEFAULT_THRESHOLD);
            drop(tree);
            library::save(&dir, &report)
        });
        cx.spawn(async move |this, cx| {
            let written = written.await;
            let _ = this.update(cx, |this, cx| match written {
                Ok(_) => {
                    this.say(
                        "Usage report saved. File \u{203a} Usage History \
                         compares it with earlier ones.",
                        Status::Success,
                        cx,
                    );
                    crate::usage_history::reload_open(cx);
                }
                Err(error) => {
                    // Nothing was kept, so the scan may be asked again.
                    if this.report_written == Some(epoch) {
                        this.report_written = None;
                    }
                    this.say_until_closed(
                        format!("The usage report could not be saved: {error}"),
                        Status::Warning,
                        cx,
                    );
                }
            });
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_choice_is_on_until_turned_off_and_survives_a_reload() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("pwrdisk/save-usage-report");
        assert!(load(&path).expect("missing"), "on by default");
        for on in [false, true, false] {
            save(&path, on).expect("save");
            assert_eq!(load(&path).expect("load"), on);
        }
        std::fs::write(&path, "sometimes\n").expect("corrupt");
        assert_eq!(
            load(&path).expect_err("corrupt").kind(),
            io::ErrorKind::InvalidData
        );
    }
}
