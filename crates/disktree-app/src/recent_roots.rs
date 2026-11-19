//! The roots this computer scanned most recently, listed first in Choose
//! what to scan.
//!
//! A root counts when the person chose it: a row or the Directory row in
//! the chooser, or File › Open Folder. A rescan through the chooser moves
//! it to the top. Going up a level, and a scan an agent asks for, leave the
//! list alone. Only paths and times are kept, on this computer, and SSH
//! hosts keep none.

use std::io;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Local, NaiveDate};
use disktree_core::space::Volume;
use serde::{Deserialize, Serialize};

/// How many recent roots the chooser lists above its rule.
pub const LIMIT: usize = 5;

/// One root and when it was last chosen for a scan.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recent {
    pub path: PathBuf,
    /// Unix seconds.
    pub at: i64,
}

/// Where the list is kept, beside the other remembered choices.
pub fn settings_path() -> Option<PathBuf> {
    crate::power::config_base().map(|base| base.join("pwrdisk/recent-roots"))
}

/// The remembered list, newest first; empty when none was saved or the
/// file cannot be read, since the chooser still works without it.
pub fn load(path: &Path) -> Vec<Recent> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut list: Vec<Recent> = serde_json::from_str(&text).unwrap_or_default();
    list.truncate(LIMIT);
    list
}

pub fn save(path: &Path, list: &[Recent]) -> io::Result<()> {
    crate::power::write_setting(path, &serde_json::to_string(list)?)
}

/// Put `path` at the top: once, newest first, at most [`LIMIT`].
pub fn record(list: &mut Vec<Recent>, path: PathBuf, at: i64) {
    list.retain(|recent| recent.path != path);
    list.insert(0, Recent { path, at });
    list.truncate(LIMIT);
}

/// "Scanned today", "Scanned yesterday" or "Scanned N days ago", by the
/// local calendar.
pub fn label(scanned: NaiveDate, today: NaiveDate) -> String {
    match (today - scanned).num_days() {
        ..=0 => "Scanned today".into(),
        1 => "Scanned yesterday".into(),
        days => format!("Scanned {days} days ago"),
    }
}

fn local_date(at: i64) -> Option<NaiveDate> {
    DateTime::from_timestamp(at, 0)
        .map(|time| time.with_timezone(&Local).date_naive())
}

/// The chooser's rows: the recent roots that are there now, newest first,
/// then the listed roots not already among them, in their own order.
/// Returns the rows and how many of them are recent, which is where the
/// rule goes.
///
/// `present` says whether a recent root is there now; an unplugged disk's
/// root is left out until it is back, and keeps its place in the list.
pub fn arrange(
    recents: &[Recent],
    listed: Vec<Volume>,
    present: impl Fn(&Path) -> bool,
    today: NaiveDate,
) -> (Vec<Volume>, usize) {
    let mut rows: Vec<Volume> = recents
        .iter()
        .filter(|recent| present(&recent.path))
        .map(|recent| Volume {
            point: recent.path.clone(),
            device: Some(local_date(recent.at).map_or_else(
                || "Scanned before".into(),
                |day| label(day, today),
            )),
            space: None,
        })
        .collect();
    let recent = rows.len();
    rows.extend(listed.into_iter().filter(|volume| {
        !recents
            .iter()
            .any(|recent| recent.path == volume.point && present(&recent.path))
    }));
    (rows, recent)
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn volume(point: &str) -> Volume {
        Volume {
            point: PathBuf::from(point),
            device: Some("Mounted volume".into()),
            space: None,
        }
    }

    #[test]
    fn labels_count_calendar_days() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 19).unwrap();
        let day = |d| NaiveDate::from_ymd_opt(2026, 11, d).unwrap();
        assert_eq!(label(day(19), today), "Scanned today");
        assert_eq!(label(day(18), today), "Scanned yesterday");
        assert_eq!(label(day(7), today), "Scanned 12 days ago");
        // A clock set back since: still today, never "-1 days ago".
        assert_eq!(label(day(20), today), "Scanned today");
    }

    #[test]
    fn a_saved_list_reads_back_and_a_bad_file_reads_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pwrdisk/recent-roots");
        assert!(load(&path).is_empty());
        let list = vec![
            Recent {
                path: "/Volumes/Projects".into(),
                at: 1_795_000_000,
            },
            Recent {
                path: "/".into(),
                at: 1_794_000_000,
            },
        ];
        save(&path, &list).unwrap();
        assert_eq!(load(&path), list);
        std::fs::write(&path, "not json").unwrap();
        assert!(load(&path).is_empty());
    }

    #[test]
    fn recent_rows_come_first_once_with_the_rule_after_them() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 19).unwrap();
        let recents = vec![
            Recent {
                path: "/Volumes/Projects".into(),
                at: 0,
            },
            Recent {
                path: "/Users/me/Library".into(),
                at: 0,
            },
            Recent {
                path: "/Volumes/Gone".into(),
                at: 0,
            },
        ];
        let listed = vec![
            volume("/Users/me"),
            volume("/"),
            volume("/Volumes/Projects"),
        ];
        let (rows, recent) = arrange(
            &recents,
            listed,
            |path| path != Path::new("/Volumes/Gone"),
            today,
        );
        let points: Vec<_> =
            rows.iter().map(|row| row.point.to_str().unwrap()).collect();
        assert_eq!(
            points,
            ["/Volumes/Projects", "/Users/me/Library", "/Users/me", "/"]
        );
        assert_eq!(recent, 2);
    }

    #[derive(Clone, Debug)]
    enum Step {
        /// Scan the root with this index.
        Scan(usize),
        /// Plug in or unplug the root with this index.
        Toggle(usize),
    }

    const ROOTS: usize = 8;

    fn root(index: usize) -> PathBuf {
        PathBuf::from(format!("/Volumes/Disk {index}"))
    }

    proptest! {
        /// After any run of scans and unplugged disks the chooser lists
        /// the present recent roots newest first, at most five, then
        /// every other listed root once, in listing order. The model is a
        /// plain log of scans.
        #[test]
        fn chooser_rows_follow_a_log_of_scans(
            steps in prop::collection::vec(
                prop_oneof![
                    (0..ROOTS).prop_map(Step::Scan),
                    (0..ROOTS).prop_map(Step::Toggle),
                ],
                0..40,
            ),
        ) {
            let today = NaiveDate::from_ymd_opt(2026, 11, 19).unwrap();
            let mut list = Vec::new();
            let mut log: Vec<usize> = Vec::new();
            let mut present = [true; ROOTS];
            for (at, step) in steps.into_iter().enumerate() {
                match step {
                    Step::Scan(index) => {
                        record(&mut list, root(index), i64::try_from(at).unwrap());
                        log.push(index);
                    }
                    Step::Toggle(index) => present[index] = !present[index],
                }
                // Model: the last five distinct roots scanned, newest
                // first, whether or not they are there now.
                let mut newest: Vec<usize> = Vec::new();
                for &index in log.iter().rev() {
                    if !newest.contains(&index) {
                        newest.push(index);
                    }
                }
                newest.truncate(LIMIT);
                let kept: Vec<PathBuf> =
                    list.iter().map(|recent| recent.path.clone()).collect();
                prop_assert_eq!(
                    &kept,
                    &newest.iter().map(|&i| root(i)).collect::<Vec<_>>()
                );

                let listed: Vec<Volume> = (0..ROOTS)
                    .filter(|&i| present[i])
                    .map(|i| volume(root(i).to_str().unwrap()))
                    .collect();
                let (rows, recent) = arrange(
                    &list,
                    listed,
                    |path| (0..ROOTS).any(|i| present[i] && root(i) == path),
                    today,
                );
                let mut expected: Vec<PathBuf> = newest
                    .iter()
                    .filter(|&&i| present[i])
                    .map(|&i| root(i))
                    .collect();
                prop_assert_eq!(recent, expected.len());
                for i in (0..ROOTS).filter(|&i| present[i]) {
                    if !expected.contains(&root(i)) {
                        expected.push(root(i));
                    }
                }
                let points: Vec<PathBuf> =
                    rows.into_iter().map(|row| row.point).collect();
                prop_assert_eq!(points, expected);
            }
        }
    }
}
