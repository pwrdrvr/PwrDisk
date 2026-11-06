//! The script is data, with bounded dimensions, times and output names.

use std::collections::HashSet;

use anyhow::{Result, ensure};
use disktree_core::classify::Category;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    pub size: [u16; 2],
    pub theme: Theme,
    pub steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    TokyoNight,
    FlexokiLight,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Step {
    Hover {
        at: [f32; 2],
    },
    Click {
        at: [f32; 2],
    },
    Wait {
        ms: u64,
    },
    Setup {
        state: State,
    },
    Capture {
        name: String,
        #[serde(default)]
        pointer: bool,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum State {
    Legend { kind: String },
    Select { path: String },
    Mark { path: String },
    Review,
    CleanupReview,
    SystemFolders,
    FileList,
    Help,
}

impl Scenario {
    pub fn parse(json: &str) -> Result<Self> {
        let scenario: Self = serde_json::from_str(json)?;
        scenario.validate()?;
        Ok(scenario)
    }

    fn validate(&self) -> Result<()> {
        ensure!(
            self.size.iter().all(|n| (320..=2560).contains(n)),
            "size must be 320..2560 logical pixels per side"
        );
        ensure!(
            !self.steps.is_empty() && self.steps.len() <= 256,
            "a scenario needs 1..256 steps"
        );
        let mut names = HashSet::new();
        let mut moved = false;
        for step in &self.steps {
            match step {
                Step::Hover { at } | Step::Click { at } => {
                    ensure!(
                        at.iter().zip(self.size).all(|(&v, side)| {
                            v.is_finite() && v >= 0.0 && v < f32::from(side)
                        }),
                        "pointer must be inside the logical window"
                    );
                    moved = true;
                }
                Step::Wait { ms } => {
                    ensure!(*ms <= 60_000, "wait must be at most 60000 ms");
                }
                Step::Capture { name, pointer } => {
                    ensure!(
                        output_name(name),
                        "capture name must be a simple .png filename"
                    );
                    ensure!(names.insert(name), "duplicate capture {name}");
                    ensure!(
                        !pointer || moved,
                        "move the pointer before drawing it"
                    );
                }
                Step::Setup { state } => match state {
                    State::Legend { kind } => {
                        ensure!(
                            Category::LEGEND.iter().any(|k| k.name() == kind),
                            "unknown legend kind: {kind}"
                        );
                    }
                    State::Select { path } | State::Mark { path } => {
                        // Scenario paths are portable POSIX fixture paths;
                        // no spelling may escape the explicitly scanned root.
                        ensure!(
                            !path.contains('\\')
                                && path.split('/').all(|part| {
                                    !part.is_empty()
                                        && part != "."
                                        && part != ".."
                                }),
                            "setup path must be relative to the fixture root"
                        );
                    }
                    State::Review
                    | State::CleanupReview
                    | State::SystemFolders
                    | State::FileList
                    | State::Help => {}
                },
            }
        }
        ensure!(!names.is_empty(), "scenario has no captures");
        Ok(())
    }
}

fn output_name(name: &str) -> bool {
    name.strip_suffix(".png").is_some_and(|stem| {
        !stem.is_empty()
            && stem.len() <= 80
            && stem
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
    })
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use serde_json::json;

    use super::*;

    fn script(steps: &serde_json::Value) -> String {
        json!({"size": [1180, 620], "theme": "tokyo_night", "steps": steps})
            .to_string()
    }

    #[test]
    fn parses_all_steps_and_rejects_mistyped_fields() {
        let json = script(&json!([
            {"op": "setup", "state": {"type": "legend", "kind": "code"}},
            {"op": "setup", "state": {"type": "select", "path": "a/b"}},
            {"op": "setup", "state": {"type": "mark", "path": ".cache"}},
            {"op": "setup", "state": {"type": "review"}},
            {"op": "setup", "state": {"type": "file_list"}},
            {"op": "setup", "state": {"type": "help"}},
            {"op": "setup", "state": {"type": "system_folders"}},
            {"op": "hover", "at": [100, 200]},
            {"op": "click", "at": [100, 200]},
            {"op": "wait", "ms": 900},
            {"op": "capture", "name": "hover.png", "pointer": true}
        ]));
        let parsed = Scenario::parse(&json).expect("valid scenario");
        assert!(matches!(parsed.theme, Theme::TokyoNight));
        assert_eq!(parsed.steps.len(), 11);
        assert!(Scenario::parse(&json.replace("\"ms\"", "\"mss\"")).is_err());
        assert!(
            Scenario::parse(&json.replace("tokyo_night", "unknown")).is_err()
        );
    }

    #[test]
    fn refuses_duplicates_and_unknown_or_unbounded_steps() {
        for steps in [
            json!([]),
            json!([{"op": "wait", "ms": 60_001}]),
            json!([{"op": "capture", "name": "x.png", "pointer": true}]),
            json!([
                {"op": "capture", "name": "x.png"},
                {"op": "capture", "name": "x.png"}
            ]),
            json!([{"op": "setup", "state": {"type": "delete"}}]),
            json!([{"op": "setup", "state": {"type": "legend", "kind": "x"}}]),
        ] {
            assert!(Scenario::parse(&script(&steps)).is_err());
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig {
            failure_persistence: None,
            ..ProptestConfig::default()
        })]

        // A capture can only write one PNG immediately inside its output
        // directory. The accepted language has no path separators or dots.
        #[test]
        fn capture_names_are_single_portable_filenames(
            name in prop_oneof!["[a-zA-Z0-9_-]{1,80}\\.png", ".{0,100}"]
        ) {
            let accepted = Scenario::parse(&script(&json!([
                {"op": "capture", "name": name}
            ]))).is_ok();
            let expected = name.rsplit_once('.').is_some_and(|(stem, ext)| {
                ext == "png" &&
                (1..=80).contains(&stem.len()) && stem.chars().all(|c| {
                    c.is_ascii_alphanumeric() || c == '-' || c == '_'
                })
            });
            prop_assert_eq!(accepted, expected);
        }

        #[test]
        fn coordinates_obey_window_bounds(x in any::<f32>(), y in any::<f32>()) {
            let accepted = Scenario::parse(&script(&json!([
                {"op": "hover", "at": [x, y]},
                {"op": "capture", "name": "x.png", "pointer": true}
            ]))).is_ok();
            prop_assert_eq!(accepted,
                x.is_finite() && y.is_finite()
                    && (0.0..1180.0).contains(&x)
                    && (0.0..620.0).contains(&y));
        }

        #[test]
        fn relative_setup_paths_never_escape(
            parts in prop::collection::vec(prop_oneof![
                Just("..".to_owned()), Just(".".to_owned()),
                Just(String::new()), "[a-z]{1,8}"
            ], 0..8)
        ) {
            let path = parts.join("/");
            let accepted = Scenario::parse(&script(&json!([
                {"op": "setup", "state": {"type": "select", "path": path}},
                {"op": "capture", "name": "x.png"}
            ]))).is_ok();
            let expected = !parts.is_empty() && parts.iter().all(|p| {
                !p.is_empty() && p != "." && p != ".."
            });
            prop_assert_eq!(accepted, expected);
        }

        // A pointer capture requires a preceding move; waits and captures
        // do not invent a pointer position. This models sequences of steps.
        #[test]
        fn pointer_capture_tracks_moves(ops in prop::collection::vec(0_u8..3, 0..30)) {
            let mut steps = vec![];
            let mut moved = false;
            let mut valid = true;
            for (i, op) in ops.into_iter().enumerate() {
                steps.push(match op {
                    0 => { moved = true; json!({"op": "hover", "at": [0, 0]}) }
                    1 => json!({"op": "wait", "ms": 100}),
                    _ => {
                        valid &= moved;
                        json!({"op": "capture", "name": format!("s{i}.png"), "pointer": true})
                    }
                });
            }
            steps.push(json!({"op": "capture", "name": "last.png"}));
            prop_assert_eq!(Scenario::parse(&script(&json!(steps))).is_ok(), valid);
        }
    }
}
