//! A device number vacated by our image immediately belongs to another
//! image. Effects are recorded, never sent to the operating system.

use super::*;
use proptest::prelude::*;
use std::collections::VecDeque;
use std::os::unix::process::ExitStatusExt as _;
use std::path::PathBuf;
use std::process::ExitStatus;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Attached(u8),
    Gone,
    Unknown,
}

fn state(value: u8) -> State {
    match value {
        0..=2 => State::Attached(value),
        3 => State::Gone,
        _ => State::Unknown,
    }
}

fn device(number: u8, volume: bool) -> PathBuf {
    // Intentionally outside the native range, even though this model never
    // calls a real command. Retired numbers represent unrelated disks.
    let container = 900_001 + u32::from(number) * 2;
    PathBuf::from(if volume {
        format!("/dev/disk{container}s1")
    } else {
        format!("/dev/disk{container}")
    })
}

struct Model {
    backing: PathBuf,
    root: PathBuf,
    current: State,
    next: VecDeque<State>,
    errors: VecDeque<bool>,
    observed: bool,
    effects: Vec<(String, PathBuf)>,
    violations: Vec<String>,
    reads: usize,
}

impl Operations for Model {
    fn owned(
        &mut self,
        backing: &Path,
    ) -> Result<Option<Vec<serde_json::Value>>, String> {
        assert_eq!(backing, self.backing, "query the exact backing image");
        self.observed = true;
        let State::Attached(number) = self.current else {
            return match self.current {
                State::Gone => Ok(None),
                _ => Err("ownership cannot be established".into()),
            };
        };
        let container = device(number, false);
        let physical = format!("/dev/disk{}", 900_000 + u32::from(number) * 2);
        Ok(Some(vec![
            serde_json::json!({
                "dev-entry": physical,
                "content-hint": "GUID_partition_scheme",
            }),
            serde_json::json!({
                "dev-entry": container,
                "content-hint": super::super::APFS_CONTAINER,
            }),
            serde_json::json!({
                "dev-entry": device(number, true),
                "mount-point": self.root,
            }),
        ]))
    }

    fn run(&mut self, command: &mut Command) -> Result<Output, String> {
        let args: Vec<_> = command.get_args().collect();
        let verb = args[0].to_str().expect("verb");
        if command.get_program() == "df" {
            self.reads += 1;
            assert_eq!(args.last().copied(), Some(self.root.as_os_str()));
        } else {
            let target = PathBuf::from(args.last().expect("device"));
            let volume = matches!(verb, "mount" | "unmount");
            let owned = match self.current {
                State::Attached(number) => target == device(number, volume),
                _ => false,
            };
            if !self.observed || !owned {
                self.violations.push(format!(
                    "{command:?} under {:?}; fresh query: {}",
                    self.current, self.observed,
                ));
            }
            self.effects.push((verb.to_owned(), target));
            self.observed = false;
            self.current = self.next.pop_front().expect("next state");
            if self.errors.pop_front().expect("command status") {
                return Err(
                    "command reported failure after state changed".into()
                );
            }
        }
        Ok(Output {
            status: ExitStatus::from_raw(0),
            stdout: b"Filesystem 1024-blocks Used Available Capacity Mounted on\nimage 100 25 75 25% mount\n".to_vec(),
            stderr: Vec::new(),
        })
    }
}

fn check(
    states: [State; 3],
    errors: [bool; 2],
    checkpointing: bool,
    force: bool,
) -> Result<(), TestCaseError> {
    let temp = tempfile::tempdir().expect("model backing directory");
    let backing = temp.path().join("oracle.sparseimage");
    fs::write(&backing, b"unattached stand-in").expect("backing");
    let root = temp.path().join("mount");
    let mut model = Model {
        backing,
        root,
        current: states[0],
        next: states[1..].iter().copied().collect(),
        errors: errors.into_iter().collect(),
        observed: false,
        effects: Vec::new(),
        violations: Vec::new(),
        reads: 0,
    };
    let backing = model.backing.clone();
    let root = model.root.clone();
    let result = if checkpointing {
        // Setup remembered volume 0, whose number can since have been reused.
        super::checkpoint(&root, &device(0, true), &mut model)
    } else {
        // release_backing resolves this once before the forced pair today.
        let outcome = match model.owned(&backing) {
            Ok(Some(entries)) => {
                let target = super::super::cleanup_device(&entries)
                    .expect("owned cleanup disk");
                super::eject(&target, force, &mut model)
            }
            Ok(None) => Ok(()),
            Err(error) => Err(error),
        };
        outcome.map(|()| Output {
            status: ExitStatus::from_raw(0),
            stdout: Vec::new(),
            stderr: Vec::new(),
        })
    };

    // Independent state-machine oracle: only the currently attached image
    // permits a step. Unmount failure stops a checkpoint, but forced cleanup
    // must still try detachment after another successful ownership query.
    let mut expected = Vec::new();
    if let State::Attached(number) = states[0] {
        expected.push((
            if checkpointing {
                "unmount"
            } else if force {
                "unmountDisk"
            } else {
                "eject"
            }
            .into(),
            device(number, checkpointing),
        ));
        if (checkpointing && !errors[0]) || (!checkpointing && force) {
            if let State::Attached(next) = states[1] {
                expected.push((
                    if checkpointing { "mount" } else { "detach" }.into(),
                    device(next, checkpointing),
                ));
            }
        }
    }
    prop_assert!(model.violations.is_empty(), "{:?}", model.violations);
    prop_assert_eq!(&model.effects, &expected);
    if checkpointing {
        let read = expected.len() == 2 && !errors[1] && states[1] == states[2];
        prop_assert_eq!(model.reads, usize::from(read));
        prop_assert_eq!(result.is_ok(), read);
    }
    Ok(())
}

#[test]
fn checkpoint_refuses_a_reused_cached_volume() {
    check(
        [State::Attached(1), State::Gone, State::Gone],
        [false; 2],
        true,
        false,
    )
    .expect("only the current image may be unmounted");
}

#[test]
fn forced_eject_rechecks_after_unmount_removes_the_attachment() {
    check(
        [State::Attached(0), State::Gone, State::Gone],
        [false; 2],
        false,
        true,
    )
    .expect("the former device now belongs to an unrelated image");
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 256, failure_persistence: None, ..ProptestConfig::default()
    })]

    #[test]
    fn native_substeps_follow_current_image_ownership(
        states in prop::array::uniform3(0_u8..5),
        errors in prop::array::uniform2(any::<bool>()),
        checkpointing in any::<bool>(),
        force in any::<bool>(),
    ) {
        check(states.map(state), errors, checkpointing, force)?;
    }
}
