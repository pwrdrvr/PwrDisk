//! Bind a human's approval to the filesystem objects that were previewed.

use std::path::{Path, PathBuf};

use super::{Plan, Target, identity};

#[cfg(windows)]
use crate::windows::entry_identity;

#[cfg(not(windows))]
fn entry_identity(path: &Path) -> Option<(u64, u64)> {
    identity(path, false)
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Entry {
    path: PathBuf,
    parent: PathBuf,
    parent_id: (u64, u64),
    id: (u64, u64),
    is_dir: bool,
    is_link: bool,
}

impl Entry {
    fn read(path: &Path) -> Result<Self, String> {
        let metadata = std::fs::symlink_metadata(path).map_err(|error| {
            format!("Cannot bind approval to {}: {error}", path.display())
        })?;
        Ok(Self {
            path: path.to_owned(),
            parent: path
                .parent()
                .and_then(|parent| parent.canonicalize().ok())
                .ok_or("Cannot resolve the approved path's parent")?,
            parent_id: path
                .parent()
                .and_then(|parent| identity(parent, true))
                .ok_or("Cannot identify the approved parent")?,
            id: entry_identity(path)
                .ok_or("Cannot identify the approved path")?,
            is_dir: metadata.is_dir(),
            is_link: metadata.file_type().is_symlink(),
        })
    }
}

/// A process-local approval binding, never a token the agent can submit.
///
/// Directory contents remain live; this identifies roots, parents and the
/// selected objects, rather than pretending to snapshot a recursive tree.
#[derive(Clone, Debug)]
pub struct ApprovalScope {
    root: PathBuf,
    resolved_root: PathBuf,
    root_id: (u64, u64),
    entries: Vec<Entry>,
}

impl ApprovalScope {
    pub fn capture(plan: &Plan) -> Result<Self, String> {
        let resolved_root = plan
            .root
            .canonicalize()
            .map_err(|error| error.to_string())?;
        Ok(Self {
            root: plan.root.clone(),
            root_id: identity(&resolved_root, true)
                .ok_or("Cannot identify the approved scan root")?,
            resolved_root,
            entries: plan
                .targets
                .iter()
                .map(|target| Entry::read(&target.path))
                .collect::<Result<_, _>>()?,
        })
    }

    pub fn check(&self, plan: &Plan) -> Result<(), String> {
        if self.root != plan.root
            || !self
                .entries
                .iter()
                .map(|entry| &entry.path)
                .eq(plan.targets.iter().map(|target| &target.path))
        {
            return Err("Removal paths changed after approval; obtain a new preview and human approval".into());
        }
        // Validate the shared root once, then walk the matching ordered
        // lists. Searching for each entry stalls large selections.
        if !self.root_unchanged() {
            return Err(changed(&self.root));
        }
        for (entry, target) in self.entries.iter().zip(&plan.targets) {
            if Entry::read(&target.path).as_ref() != Ok(entry) {
                return Err(changed(&target.path));
            }
        }
        Ok(())
    }

    pub(super) fn check_target(
        &self,
        index: usize,
        target: &Target,
    ) -> Result<(), String> {
        // The worker retains the plan's order. Direct indexing avoids a
        // second quadratic pass while still rechecking identity per write.
        let same_target = self.entries.get(index).is_some_and(|entry| {
            entry.path == target.path
                && Entry::read(&target.path).as_ref() == Ok(entry)
        });
        if self.root_unchanged() && same_target {
            Ok(())
        } else {
            Err(changed(&target.path))
        }
    }

    fn root_unchanged(&self) -> bool {
        self.root.canonicalize().ok().as_ref() == Some(&self.resolved_root)
            && identity(&self.resolved_root, true) == Some(self.root_id)
    }
}

fn changed(path: &Path) -> String {
    format!(
        "{} or its scan root changed after preview; obtain a new preview and human approval",
        path.display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn plan_for(root: &Path, paths: &[PathBuf]) -> Plan {
        Plan {
            root: root.to_owned(),
            targets: paths
                .iter()
                .map(|path| Target {
                    path: path.clone(),
                    bytes: 0,
                    is_dir: false,
                    hidden: false,
                })
                .collect(),
            ..Plan::default()
        }
    }

    fn check_dispatch_ordinals(count: usize) -> Result<(), TestCaseError> {
        let temp = tempfile::tempdir().expect("selection");
        let paths: Vec<_> = (0..count)
            .map(|index| temp.path().join(index.to_string()))
            .collect();
        for path in &paths {
            std::fs::write(path, []).expect("selected file");
        }
        let plan = plan_for(temp.path(), &paths);
        let scope = ApprovalScope::capture(&plan).expect("capture");
        prop_assert!(scope.check(&plan).is_ok());
        // Dispatch is bound to its original ordinal, so it can check an
        // entry directly instead of searching the selection again.
        for (slot, target) in plan.targets.iter().enumerate() {
            for index in 0..=count {
                prop_assert_eq!(
                    scope.check_target(index, target).is_ok(),
                    slot == index
                );
            }
        }
        let mut reordered = plan;
        reordered.targets.rotate_left(1);
        prop_assert!(scope.check(&reordered).is_err());
        Ok(())
    }

    #[test]
    fn dispatch_refuses_a_target_at_another_approved_ordinal() {
        check_dispatch_ordinals(2).expect("ordinal binding");
    }

    #[cfg(any(unix, windows))]
    fn directory_link(path: &Path, destination: &Path) {
        #[cfg(unix)]
        std::os::unix::fs::symlink(destination, path).expect("directory link");
        #[cfg(windows)]
        assert!(crate::windows::make_junction(path, destination));
    }

    #[cfg(any(unix, windows))]
    fn check_link_binding(dangling: bool) -> Result<(), TestCaseError> {
        let temp = tempfile::tempdir().expect("link fixture");
        let destination = temp.path().join("destination");
        let path = temp.path().join("selected-junction");
        std::fs::create_dir(&destination).expect("destination");
        directory_link(&path, &destination);
        let followed = identity(&destination, true).expect("destination id");
        // Preserve the following identity used by Windows home guards.
        #[cfg(windows)]
        prop_assert_eq!(identity(&path, false), Some(followed));
        if dangling {
            std::fs::remove_dir(&destination).expect("make dangling");
        }
        let plan = plan_for(temp.path(), std::slice::from_ref(&path));
        let scope =
            ApprovalScope::capture(&plan).map_err(TestCaseError::fail)?;
        prop_assert_ne!(scope.entries[0].id, followed);
        prop_assert!(scope.check(&plan).is_ok());
        // A change at the destination does not replace the selected link.
        if !dangling {
            std::fs::rename(&destination, temp.path().join("old-destination"))
                .expect("retain destination");
            std::fs::create_dir(&destination).expect("new destination");
            prop_assert!(scope.check(&plan).is_ok());
        }
        std::fs::rename(&path, temp.path().join("old-junction"))
            .expect("retain original link identity");
        directory_link(&path, &destination);
        prop_assert!(scope.check(&plan).is_err());
        Ok(())
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn a_dangling_link_can_be_bound_without_approving_its_destination() {
        check_link_binding(true).expect("dangling link binding");
    }

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 16, failure_persistence: None, ..ProptestConfig::default()
        })]

        #[test]
        fn dispatch_matches_the_ordered_selection_model(count in 2..12_usize) {
            check_dispatch_ordinals(count)?;
        }

        #[cfg(any(unix, windows))]
        #[test]
        fn approval_binds_the_link_not_its_destination(dangling in any::<bool>()) {
            check_link_binding(dangling)?;
        }
    }

    #[test]
    fn the_worker_refuses_the_whole_batch_if_a_target_was_replaced() {
        let temp = tempfile::tempdir_in(std::env::current_dir().expect("cwd"))
            .expect("fixture");
        let paths = [temp.path().join("first"), temp.path().join("second")];
        for path in &paths {
            std::fs::write(path, b"owned fixture").expect("file");
        }
        let mut plan = Plan {
            root: temp.path().to_owned(),
            targets: paths
                .iter()
                .map(|path| Target {
                    path: path.clone(),
                    bytes: 4096,
                    is_dir: false,
                    hidden: false,
                })
                .collect(),
            ..Plan::default()
        };
        plan.bind_approval(ApprovalScope::capture(&plan).expect("capture"))
            .expect("bind");
        let handle = super::super::start(
            plan,
            super::super::RemovalMode::Permanent,
            None,
            None,
            |work| {
                std::fs::rename(&paths[1], temp.path().join("retained"))?;
                std::fs::write(&paths[1], b"unapproved replacement")?;
                work();
                Ok(())
            },
        );
        let events: Vec<_> = std::iter::from_fn(|| handle.poll()).collect();
        assert!(events.iter().any(|event| matches!(
            event,
            super::super::RemovalEvent::Done {
                removed: 0,
                failed: 2,
                ..
            }
        )));
        assert_eq!(std::fs::read(&paths[0]).expect("first"), b"owned fixture");
        assert_eq!(
            std::fs::read(&paths[1]).expect("second"),
            b"unapproved replacement"
        );
    }

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 32,
            failure_persistence: None,
            ..ProptestConfig::default()
        })]

        /// Filesystem identity, not size or spelling, is the oracle. Keep
        /// the old entry alive so the filesystem cannot reuse its inode.
        #[test]
        fn replacement_never_inherits_approval(directory in any::<bool>(), bytes in 0..4096_usize) {
            let temp = tempfile::tempdir_in(std::env::current_dir().expect("cwd")).expect("fixture");
            let path = temp.path().join("selected");
            if directory { std::fs::create_dir(&path).expect("dir"); }
            else { std::fs::write(&path, vec![1; bytes]).expect("file"); }
            let plan = Plan {
                root: temp.path().to_owned(),
                targets: vec![Target { path: path.clone(), bytes: bytes as u64, is_dir: directory, hidden: false }],
                ..Plan::default()
            };
            let scope = ApprovalScope::capture(&plan).expect("capture");
            prop_assert!(scope.check(&plan).is_ok());
            std::fs::rename(&path, temp.path().join("retained-original")).expect("retain");
            if directory { std::fs::create_dir(&path).expect("replacement dir"); }
            else { std::fs::write(&path, vec![1; bytes]).expect("replacement file"); }
            prop_assert!(scope.check(&plan).is_err());
            prop_assert!(path.exists());
        }
    }
}
