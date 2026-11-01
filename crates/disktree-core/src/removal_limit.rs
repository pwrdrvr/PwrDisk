//! A per-operation capacity limit, independent of projected savings.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::removal::Target;
use crate::size::human_bytes;

/// The default applies to both Trash and permanent removal.
pub const DEFAULT_PERCENT: u8 = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limit(u8);

impl Default for Limit {
    fn default() -> Self {
        Self(DEFAULT_PERCENT)
    }
}

/// One volume's contribution to a proposed operation, before clone savings.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Budget {
    pub volume: String,
    pub bytes: u64,
    pub capacity: u64,
    pub limit: u64,
}

impl Limit {
    pub fn new(percent: u8) -> Result<Self, String> {
        if (1..=100).contains(&percent) {
            Ok(Self(percent))
        } else {
            Err("Removal limit must be between 1% and 100%".into())
        }
    }

    pub const fn percent(self) -> u8 {
        self.0
    }

    /// Count every target against its own volume; unknown capacity fails closed.
    pub fn check(self, targets: &[Target]) -> Result<Vec<Budget>, String> {
        self.check_with(targets, |path| {
            let meta = std::fs::symlink_metadata(path).map_err(|error| {
                format!("Cannot inspect removal target: {error}")
            })?;
            // Removal unlinks the entry itself, even when its destination
            // is missing or lies on another volume. Windows also requires
            // a directory for GetDiskFreeSpaceExW, including regular files.
            let path = if meta.is_symlink() || (cfg!(windows) && meta.is_file())
            {
                path.parent()
                    .ok_or("A removal entry must have a parent directory")?
            } else {
                path
            };
            let volume = crate::space::device_for(path)
                .ok_or("Cannot identify the removal target's volume")?;
            let space = crate::space::space_info(path).map_err(|error| {
                format!("Cannot read volume capacity: {error}")
            })?;
            Ok((volume, space.total))
        })
    }

    fn check_with(
        self,
        targets: &[Target],
        mut volume: impl FnMut(&Path) -> Result<(String, u64), String>,
    ) -> Result<Vec<Budget>, String> {
        let mut totals = BTreeMap::<String, (u64, u64)>::new();
        for target in targets {
            let (key, capacity) = volume(&target.path)?;
            if capacity == 0 {
                return Err("Cannot establish a nonzero volume capacity".into());
            }
            let total = totals.entry(key).or_insert((0, capacity));
            total.0 = total
                .0
                .checked_add(target.bytes)
                .ok_or("Removal size overflow")?;
            total.1 = total.1.min(capacity);
        }
        totals.into_iter().map(|(volume, (bytes, capacity))| {
            // u128 keeps the inclusive boundary exact even on a huge volume.
            let limit = u64::try_from(u128::from(capacity) * u128::from(self.0) / 100)
                .map_err(|_| "Removal capacity overflow")?;
            if bytes > limit {
                return Err(format!(
                    "Removal blocked: {} on {volume} exceeds the {}% per-operation limit ({} of {} capacity). Reduce the selection or change the limit in Settings > Safety; do not split an unapproved removal into batches.",
                    human_bytes(bytes), self.0, human_bytes(limit), human_bytes(capacity),
                ));
            }
            Ok(Budget { volume, bytes, capacity, limit })
        }).collect()
    }
}

/// Re-measure on the worker before any target is touched. A scan may have
/// hidden entries skipped, a depth limit, or sizes that have since grown.
pub(crate) fn measured(targets: &[Target]) -> Result<Vec<Target>, String> {
    targets.iter().map(|target| {
        let meta = std::fs::symlink_metadata(&target.path)
            .map_err(|error| format!("Cannot re-measure {}: {error}", target.path.display()))?;
        if meta.is_dir() != target.is_dir {
            return Err(format!("Target type changed: {}", target.path.display()));
        }
        let bytes = if meta.is_dir() {
            let tree = crate::scan::scan(&target.path, crate::scan::ScanOptions {
                dedup_hardlinks: false,
                apfs_clone_metadata: false,
                ..crate::scan::ScanOptions::default()
            }).map_err(|error| format!("Cannot re-measure removal: {error}"))?;
            let mut nodes = vec![&tree];
            while let Some(node) = nodes.pop() {
                if node.read_error {
                    return Err("Cannot enforce removal limit: part of the selection is unreadable".into());
                }
                nodes.extend(&node.children);
            }
            tree.bytes
        } else {
            #[cfg(windows)]
            let bytes = crate::windows::allocated_size(&target.path)
                .map_err(|error| format!("Cannot re-measure {}: {error}", target.path.display()))?;
            #[cfg(not(windows))]
            let bytes = crate::scan::measure(&meta, false);
            bytes
        };
        Ok(Target { bytes: bytes.max(target.bytes), ..target.clone() })
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(path: &str, bytes: u64) -> Target {
        Target {
            path: path.into(),
            bytes,
            is_dir: true,
            hidden: false,
        }
    }

    #[test]
    fn boundary_is_inclusive_and_sums_targets_per_volume() {
        let limit = Limit::default();
        let targets = [target("a", 20), target("b", 30)];
        assert_eq!(
            limit
                .check_with(&targets, |_| Ok(("disk".into(), 1000)))
                .expect("5% boundary")[0]
                .bytes,
            50
        );
        assert!(
            limit
                .check_with(&[target("a", 51)], |_| Ok(("disk".into(), 1000)))
                .is_err()
        );
        assert!(
            limit
                .check_with(&targets, |path| Ok((
                    path.display().to_string(),
                    600
                )))
                .is_ok()
        );
        assert!(
            limit
                .check_with(&targets, |_| Err("unknown volume".into()))
                .is_err()
        );
        assert!(
            limit
                .check_with(&targets, |_| Ok(("disk".into(), 0)))
                .is_err()
        );
        assert!(
            limit
                .check_with(&[target("a", u64::MAX), target("b", 1)], |_| Ok((
                    "disk".into(),
                    u64::MAX
                )))
                .is_err()
        );
        assert!(Limit::new(0).is_err());
        assert!(Limit::new(101).is_err());
    }

    #[test]
    fn preflight_includes_hidden_entries_and_growth_since_marking() {
        let temp = tempfile::tempdir().expect("fixture");
        let dir = temp.path().join("selected");
        std::fs::create_dir(&dir).expect("mkdir");
        std::fs::write(dir.join(".hidden"), vec![b'x'; 16_384]).expect("write");
        let measured = measured(&[Target {
            path: dir,
            bytes: 0,
            is_dir: true,
            hidden: false,
        }])
        .expect("measure");
        assert!(measured[0].bytes >= 16_384);
        assert!(
            Limit::default()
                .check_with(&measured, |_| Ok(("fixture".into(), 100_000)))
                .is_err()
        );
        assert!(temp.path().join("selected/.hidden").exists());
    }

    #[test]
    fn regular_file_capacity_uses_its_containing_volume() {
        let temp = tempfile::tempdir().expect("fixture");
        let path = temp.path().join("selected.txt");
        let content = vec![b'x'; 1_000];
        std::fs::write(&path, &content).expect("fixture file");
        let targets = [Target {
            path: path.clone(),
            bytes: 1_000,
            is_dir: false,
            hidden: false,
        }];
        let budgets = Limit::default()
            .check(&targets)
            .expect("file capacity preflight");
        assert_eq!(budgets.len(), 1);
        assert_eq!(
            budgets[0].volume,
            crate::space::device_for(temp.path()).expect("parent volume")
        );
        assert_eq!(
            budgets[0].capacity,
            crate::space::space_info(temp.path())
                .expect("parent capacity")
                .total
        );
        assert_eq!(budgets[0].bytes, 1_000);
        assert_eq!(std::fs::read(path).expect("file untouched"), content);
    }

    #[cfg(unix)]
    #[test]
    fn dangling_links_use_the_containing_volume() {
        let temp = tempfile::tempdir().expect("fixture");
        let link = temp.path().join("dangling");
        std::os::unix::fs::symlink("missing/destination", &link)
            .expect("fixture link");
        let targets = [Target {
            path: link.clone(),
            bytes: 0,
            is_dir: false,
            hidden: false,
        }];
        let limit = Limit::default();
        let budget = limit.check(&targets).expect("link preview");
        assert_eq!(
            budget[0].volume,
            crate::space::device_for(temp.path()).expect("parent volume")
        );
        assert_eq!(
            budget[0].capacity,
            crate::space::space_info(temp.path())
                .expect("capacity")
                .total
        );
        let fresh = measured(&targets).expect("link measurement");
        limit.check(&fresh).expect("link worker preflight");
        assert_eq!(
            std::fs::read_link(link).expect("link untouched"),
            Path::new("missing/destination")
        );
    }

    #[cfg(windows)]
    #[test]
    fn sparse_file_preflight_uses_allocation() {
        let temp = tempfile::tempdir().expect("fixture");
        let path = temp.path().join("sparse.bin");
        let file = std::fs::File::create(&path).expect("fixture file");
        let output = std::process::Command::new("fsutil")
            .args(["sparse", "setflag"])
            .arg(&path)
            .output()
            .expect("set fixture sparse flag");
        assert!(output.status.success(), "{output:?}");
        file.set_len(64 * 1024 * 1024).expect("sparse length");
        drop(file);
        assert_file_preflight_uses_allocation(&path);
    }

    #[cfg(windows)]
    #[test]
    fn compressed_file_preflight_uses_allocation() {
        let temp = tempfile::tempdir().expect("fixture");
        let path = temp.path().join("compressed.bin");
        std::fs::write(&path, vec![b'x'; 4 * 1024 * 1024])
            .expect("fixture file");
        let output = std::process::Command::new("compact")
            .args(["/C", "/I", "/Q"])
            .arg(&path)
            .output()
            .expect("compress fixture");
        assert!(output.status.success(), "{output:?}");
        assert_file_preflight_uses_allocation(&path);
    }

    #[cfg(windows)]
    fn assert_file_preflight_uses_allocation(path: &Path) {
        let parent = path.parent().expect("fixture parent");
        let tree =
            crate::scan::scan(parent, crate::scan::ScanOptions::default())
                .expect("directory measurement");
        let length = std::fs::metadata(path).expect("metadata").len();
        let targets = [Target {
            path: path.into(),
            bytes: tree.bytes,
            is_dir: false,
            hidden: false,
        }];
        let fresh = measured(&targets).expect("individual file preflight");
        assert_eq!(fresh[0].bytes, tree.bytes);
        assert!(
            fresh[0].bytes < length,
            "fixture allocation {} must be below logical length {length}",
            fresh[0].bytes
        );
        // Compression ratios depend on cluster and compression-unit sizes.
        // Place the budget just below logical length so allocation fits
        // without requiring a particular ratio from the fixture volume.
        let capacity = (length - 1) * 100 / u64::from(DEFAULT_PERCENT);
        let limit = Limit::default();
        limit
            .check_with(&fresh, |_| Ok(("fixture".into(), capacity)))
            .expect("allocated size fits the budget");
        assert!(
            limit
                .check_with(
                    &[Target {
                        bytes: length,
                        ..targets[0].clone()
                    }],
                    |_| Ok(("fixture".into(), capacity))
                )
                .is_err(),
            "logical length would incorrectly refuse the file"
        );
        assert!(path.exists(), "measurement leaves the fixture intact");
    }
}
