//! Bind a human's approval to the filesystem objects that were previewed.

use std::path::{Path, PathBuf};

use super::{Plan, Target, identity};

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
            id: identity(path, false)
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
        for target in &plan.targets {
            self.check_target(target)?;
        }
        Ok(())
    }

    pub(super) fn check_target(&self, target: &Target) -> Result<(), String> {
        let same_root = self.root.canonicalize().ok().as_ref()
            == Some(&self.resolved_root)
            && identity(&self.resolved_root, true) == Some(self.root_id);
        let same_target = self
            .entries
            .iter()
            .find(|entry| entry.path == target.path)
            .is_some_and(|entry| {
                Entry::read(&target.path).as_ref() == Ok(entry)
            });
        if same_root && same_target {
            Ok(())
        } else {
            Err(format!(
                "{} or its scan root changed after preview; obtain a new preview and human approval",
                target.path.display()
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

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
