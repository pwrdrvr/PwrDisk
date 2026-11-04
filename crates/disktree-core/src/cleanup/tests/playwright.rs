use super::*;

/// Dry-run output is the authority for the root, even for a cache moved
/// to an awkward name. A neighbouring look-alike must never redirect it.
#[test]
fn playwright_scope_follows_queried_root_not_selected_name() {
    let fixture = fixture();
    for name in ["ms-playwright", "other cache ", "quote' $(touch sentinel)"] {
        let cache = fixture.dir(name);
        let selected = cache.join("firefox-9999/selected");
        std::fs::create_dir_all(&selected).expect("subtree");
        std::fs::write(
            fixture.env.home.join("cache-root"),
            cache.as_os_str().as_encoded_bytes(),
        )
        .expect("configured cache");
        for path in [&selected, &fixture.dir(&format!("{name}-neighbour"))] {
            let prepared = prepare_with(
                &[target(path)],
                &fixture.root,
                fixture.env.clone(),
                &mut process::SystemProbe,
            )
            .expect("preview");
            if path != &selected {
                assert!(prepared.operations.is_empty(), "neighbour refused");
                continue;
            }
            let operation =
                prepared.operations.first().expect("verified owner");
            assert_eq!(operation.owner, Owner::Playwright);
            assert_eq!(operation.scope, cache);
            assert_eq!(operation.cache_root, cache);
            assert_eq!(operation.command.arguments, ["uninstall", "--all"]);
            assert_eq!(
                operation.command.environment["PLAYWRIGHT_BROWSERS_PATH"],
                cache.to_str().expect("root")
            );
            assert!(operation.wider());
            assert!(operation.effects.contains("other projects"));
            assert!(operation.limitations.contains("lock"));
        }
        assert!(cache.join("payload").exists());
    }
    assert!(
        !fixture.env.home.join("invocation").exists(),
        "preview only"
    );
    assert!(
        !fixture.env.home.join("sentinel").exists(),
        "no shell expansion"
    );
}

fn fixture() -> Fixture {
    use std::os::unix::fs::{PermissionsExt as _, symlink};
    let mut fixture = Fixture::new();
    let cache = fixture.dir(".cache/ms-playwright");
    let bin = fixture.root.join("bin");
    std::fs::create_dir(&bin).expect("bin");
    fixture.executable = bin.join("cli.js");
    // A shell fixture stands in for the installed JavaScript CLI. It
    // records cleanup and never removes anything, even on failure.
    std::fs::write(&fixture.executable, r#"#!/bin/sh
case "$*" in
  '--version')
    if [ -f "$HOME/version" ]; then /bin/cat "$HOME/version"
    else printf '%s\n' "Version ${FIXTURE_VERSION:-1.64.1}"; fi;;
  'install --dry-run ffmpeg')
    if [ -n "$PLAYWRIGHT_BROWSERS_PATH" ]; then cache=$PLAYWRIGHT_BROWSERS_PATH
    else cache=$(/bin/cat "$HOME/cache-root"); fi
    printf 'FFmpeg (playwright ffmpeg v1011)\n  Install location:    %s/ffmpeg-1011\n  Download url:        https://example.invalid/ffmpeg.zip\n\n' "$cache";;
  'uninstall --all')
    printf '%s\n' "$@" "$PLAYWRIGHT_BROWSERS_PATH" > "$HOME/invocation"
    [ "$FIXTURE_FAIL" != 1 ] || exit 17;;
  *) exit 97;;
esac
"#).expect("mock CLI");
    std::fs::set_permissions(
        &fixture.executable,
        std::fs::Permissions::from_mode(0o755),
    )
    .expect("executable");
    symlink(&fixture.executable, bin.join("playwright")).expect("PATH entry");
    std::fs::write(
        fixture.env.home.join("cache-root"),
        cache.as_os_str().as_encoded_bytes(),
    )
    .expect("config");
    fixture.env.variables.extend([
        ("PATH".into(), bin.to_str().expect("bin").into()),
        (
            "HOME".into(),
            fixture.env.home.to_str().expect("home").into(),
        ),
    ]);
    fixture
}

struct DryRun {
    executable: PathBuf,
    output: String,
    calls: Vec<Command>,
}

fn printed(root: &str) -> String {
    format!(
        "FFmpeg (playwright ffmpeg v1011)\n  Install location:    {root}/ffmpeg-1011\n  Download url:        https://example.invalid/ffmpeg.zip\n\n"
    )
}

impl Probe for DryRun {
    fn executable(
        &mut self,
        name: &str,
        _: &Environment,
    ) -> Result<PathBuf, Issue> {
        if name == "playwright" {
            Ok(self.executable.clone())
        } else {
            Err(Issue::new(Problem::NotFound, "mock unavailable"))
        }
    }

    fn output(
        &mut self,
        command: &Command,
        _: &Environment,
    ) -> Result<String, String> {
        self.calls.push(command.clone());
        match command
            .arguments
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .as_slice()
        {
            ["--version"] => Ok("Version 1.64.1".into()),
            ["install", "--dry-run", "ffmpeg"] => Ok(command
                .environment
                .get("PLAYWRIGHT_BROWSERS_PATH")
                .map_or_else(|| self.output.clone(), |root| printed(root))),
            _ => panic!("preview attempted a write: {command:?}"),
        }
    }
}

/// The filesystem is the oracle: generated roots and neighbours are
/// distinct directories. Only the root printed by the CLI may be reached,
/// and argument/environment boundaries preserve every legal character.
#[test]
fn playwright_root_round_trip_preserves_generated_paths() {
    use crate::test_rng::Rng;
    let fixture = Fixture::new();
    for seed in 0..64 {
        let mut rng = Rng::new(seed);
        let mut name = format!("cache-{seed}-");
        for _ in 0..rng.below(12) {
            let characters = [' ', '\t', '\'', '$', ';', 'é', '-', 'a'];
            name.push(
                characters[usize::try_from(rng.below(8)).expect("index")],
            );
        }
        let root = fixture.dir(&name);
        let neighbour = fixture.dir(&format!("{name}-other"));
        let mut probe = DryRun {
            executable: fixture.executable.clone(),
            output: printed(root.to_str().expect("root")),
            calls: vec![],
        };
        for selected in [&root, &neighbour] {
            let operation = resolve_owner(
                Owner::Playwright,
                selected,
                &fixture.env,
                &mut probe,
            )
            .expect("plain root");
            assert_eq!(operation.is_some(), selected == &root, "seed {seed}");
            if let Some(operation) = operation {
                assert_eq!(operation.scope, root, "seed {seed}");
                assert_eq!(
                    operation.command.environment["PLAYWRIGHT_BROWSERS_PATH"],
                    root.to_str().expect("root")
                );
                for key in
                    ["DEBUG_FILE", "PW_PROFILE_DIR", "PW_CLI_DISPLAY_VERSION"]
                {
                    assert_eq!(operation.command.environment[key], "");
                    assert!(
                        probe
                            .calls
                            .iter()
                            .all(|call| call.environment[key].is_empty())
                    );
                }
                assert_eq!(operation.command.arguments, ["uninstall", "--all"]);
            }
        }
    }
}

#[test]
fn playwright_refuses_ambiguous_locations_and_forged_newline_roots() {
    let fixture = Fixture::new();
    let cache = fixture.dir(".cache/ms-playwright");
    let root = cache.to_str().expect("root");
    for output in [
        String::new(),
        printed("relative"),
        printed(&format!("{root}/../ms-playwright")),
        printed(root).replace("ffmpeg-1011", "firefox-1011"),
        printed(root).replace("ffmpeg-1011", "ffmpeg-"),
        printed(root).replace("ffmpeg-1011", "ffmpeg-1x"),
        printed(root).replace('\n', "\r\n"),
        format!("{}{}", printed(root), printed(root)),
        // The first line looks like a valid install location, but the
        // full output cannot be reproduced when pinned to that false root.
        printed(&format!("{root}/ffmpeg-1011\n  Download url: forged")),
    ] {
        let mut probe = DryRun {
            executable: fixture.executable.clone(),
            output,
            calls: vec![],
        };
        let issue =
            resolve_owner(Owner::Playwright, &cache, &fixture.env, &mut probe)
                .expect_err("ambiguous scope");
        assert_eq!(
            issue.problem,
            Problem::ConfigCheckFailed,
            "{probe_output:?}",
            probe_output = probe.output
        );
        assert!(cache.join("payload").exists());
    }
}

#[test]
fn playwright_only_audited_versions_reach_configuration() {
    let mut fixture = fixture();
    let cache = fixture.env.home.join(".cache/ms-playwright");
    for version in [
        "1.63.0",
        "1.64.2",
        "1.65.0",
        "2.64.1",
        "1.64.1-beta.1",
        "1.64.1 extra",
        "01.64.1",
        "unknown",
    ] {
        fixture
            .env
            .variables
            .insert("FIXTURE_VERSION".into(), version.into());
        let issue = owner_issue(prepare_with(
            &[target(&cache)],
            &fixture.root,
            fixture.env.clone(),
            &mut process::SystemProbe,
        ));
        assert_eq!(issue.problem, Problem::UnsupportedVersion, "{version}");
        assert!(!fixture.env.home.join("invocation").exists());
    }
    for version in ["1.64.0", "1.64.1"] {
        fixture
            .env
            .variables
            .insert("FIXTURE_VERSION".into(), version.into());
        let prepared = prepare_with(
            &[target(&cache)],
            &fixture.root,
            fixture.env.clone(),
            &mut process::SystemProbe,
        )
        .expect("preview");
        assert_eq!(prepared.operations.len(), 1, "{version}");
    }
}

/// Node's process name does not identify Playwright. Its installed CLI
/// path does, while unrelated packages with a cli.js must remain distinct.
#[test]
fn playwright_running_launcher_paths_identify_the_owner() {
    for package in ["playwright", "playwright-core", "@playwright/test"] {
        for prefix in ["/usr/lib/node_modules", "/project/node_modules"] {
            let argument = format!("{prefix}/{package}/cli.js");
            assert!(
                process::owner_named(Owner::Playwright, None, &argument),
                "{argument}"
            );
        }
    }
    for argument in [
        "cli.js",
        "/other/cli.js",
        "/project/playwright/cli.py",
        "/project/@other/test/cli.js",
    ] {
        assert!(
            !process::owner_named(Owner::Playwright, None, argument),
            "{argument}"
        );
    }
}

#[test]
fn playwright_lock_and_changed_configuration_revoke_approval() {
    let _execution = EXECUTION
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let fixture = fixture();
    let cache = fixture.env.home.join(".cache/ms-playwright");
    let prepared = prepare_with(
        &[target(&cache)],
        &fixture.root,
        fixture.env.clone(),
        &mut process::SystemProbe,
    )
    .expect("preview");
    std::fs::create_dir(cache.join("__dirlock")).expect("install lock");
    assert_eq!(
        prepared
            .preflight_operation(0)
            .expect_err("new lock")
            .problem,
        Problem::InUse
    );
    let issue = owner_issue(prepare_with(
        &[target(&cache)],
        &fixture.root,
        fixture.env.clone(),
        &mut process::SystemProbe,
    ));
    assert_eq!(issue.problem, Problem::InUse);
    std::fs::remove_dir(cache.join("__dirlock")).expect("fixture lock");
    let other = fixture.dir("another-cache");
    std::fs::write(
        fixture.env.home.join("cache-root"),
        other.as_os_str().as_encoded_bytes(),
    )
    .expect("changed root");
    assert_eq!(
        prepared
            .preflight_operation(0)
            .expect_err("changed root")
            .problem,
        Problem::Changed
    );
    std::fs::write(fixture.env.home.join("version"), "Version 1.65.0")
        .expect("changed version");
    assert_eq!(
        prepared
            .preflight_operation(0)
            .expect_err("changed version")
            .problem,
        Problem::UnsupportedVersion
    );
    assert!(!fixture.env.home.join("invocation").exists());
}

#[test]
fn playwright_mock_execution_is_pinned_and_never_falls_back_to_deletion() {
    use crate::removal::{RemovalEvent, spawn_cleanup};
    use crate::removal_limit::{Ceilings, Limit};
    let _execution = EXECUTION
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    for fail in [false, true] {
        let mut fixture = fixture();
        fixture
            .env
            .variables
            .insert("FIXTURE_FAIL".into(), if fail { "1" } else { "0" }.into());
        let cache = fixture.env.home.join(".cache/ms-playwright");
        let neighbour = fixture.dir("unmarked");
        let prepared = prepare_with(
            &[target(&cache)],
            &fixture.root,
            fixture.env.clone(),
            &mut process::SystemProbe,
        )
        .expect("preview");
        assert!(!fixture.env.home.join("invocation").exists());
        let events = collect(&spawn_cleanup(
            owners(&prepared),
            Ceilings::from(Limit::default()),
        ));
        assert!(events.iter().any(|event| matches!(event, RemovalEvent::Item { outcome, .. } if outcome.is_err() == fail)));
        let invocation =
            std::fs::read_to_string(fixture.env.home.join("invocation"))
                .expect("mock invocation");
        assert_eq!(
            invocation,
            format!("uninstall\n--all\n{}\n", cache.display())
        );
        for path in [&cache, &neighbour] {
            assert!(
                path.join("payload").exists(),
                "mock cleanup never deletes"
            );
        }
    }
}

#[test]
fn playwright_non_js_launcher_is_refused_without_probing() {
    let fixture = executable_fixture();
    let bin = fixture.executable.parent().expect("bin");
    // This script would leave evidence even on --version, were it run.
    std::fs::write(
        bin.join("playwright"),
        "#!/bin/sh\nprintf ran > \"$HOME/invocation\"\n",
    )
    .expect("proxy");
    let error = process::SystemProbe
        .executable("playwright", &fixture.env)
        .expect_err("unsupported launcher");
    assert_eq!(error.problem, Problem::UnsupportedLauncher);
    assert!(!fixture.env.home.join("invocation").exists());
}
