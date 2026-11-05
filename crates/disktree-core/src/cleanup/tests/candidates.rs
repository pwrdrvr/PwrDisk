//! Independent owner locations must remain inspection leads even when
//! settings move them away from a directory named cache or store.

use super::*;

/// The oracle assigns independent directories to data and cache settings.
/// A store query cannot establish ownership of the other prune scopes.
#[test]
fn pnpm_independent_roots_keep_owner_hints_without_forcing_cleanup() {
    for key in [
        "PNPM_HOME",
        "XDG_DATA_HOME",
        "XDG_CACHE_HOME",
        "LOCALAPPDATA",
    ] {
        let mut fixture = Fixture::new();
        let configured = fixture.dir("independent location");
        fixture.env.variables.insert(
            key.into(),
            configured.to_str().expect("configured location").into(),
        );
        let suffixes: &[&str] = match key {
            "PNPM_HOME" => &["store/v10/files"],
            "XDG_DATA_HOME" => &["pnpm/store/v10/files"],
            "XDG_CACHE_HOME" => &["pnpm/metadata-v1", "pnpm/dlx/hash/pkg"],
            "LOCALAPPDATA" => {
                &["pnpm/store/v10/files", "pnpm-cache/dlx/hash/pkg"]
            }
            _ => unreachable!(),
        };
        for suffix in suffixes {
            let scope = configured.join(suffix);
            std::fs::create_dir_all(&scope).expect("owner state");
            let selected = scope.join("entry");
            std::fs::write(&selected, b"fixture state").expect("entry");
            assert_eq!(
                recognize::hinted(&selected, &fixture.env).map(|hint| hint.0),
                Some(Owner::Pnpm),
                "{key}: {selected:?}"
            );
            // pnpm is recognized for proposals, but has no cleanup command:
            // its paths follow ordinary removal rather than forcing cleanup.
            assert!(
                recognize::inspection_with(&selected, &fixture.env).is_none()
            );
            // Nearby user files do not acquire an owner from a common prefix.
            assert!(
                recognize::hinted(
                    &configured.join("ordinary.txt"),
                    &fixture.env
                )
                .is_none()
            );
        }
    }
}

/// Playwright accepts npm's two fallback environment keys. Puppeteer's
/// default stays under home even when another tool uses `XDG_CACHE_HOME`.
#[test]
fn browser_environment_roots_do_not_bypass_raw_removal_inspection() {
    for key in [
        "PLAYWRIGHT_BROWSERS_PATH",
        "npm_config_playwright_browsers_path",
        "npm_package_config_playwright_browsers_path",
    ] {
        let mut fixture = Fixture::new();
        let cache = fixture.dir("browser downloads");
        fixture
            .env
            .variables
            .insert(key.into(), cache.display().to_string());
        for path in [&cache, &cache.join("payload")] {
            assert_eq!(
                recognize::owner(path, &fixture.env).map(|hint| hint.0),
                Some(Owner::Playwright),
                "{key}"
            );
            assert!(recognize::inspection_with(path, &fixture.env).is_some());
        }
    }
    let mut fixture = Fixture::new();
    fixture.env.variables.insert(
        "XDG_CACHE_HOME".into(),
        fixture.root.join("other").display().to_string(),
    );
    let cache = fixture.dir(".cache/puppeteer");
    assert_eq!(
        recognize::owner(&cache, &fixture.env).map(|hint| hint.0),
        Some(Owner::Puppeteer)
    );
}

struct NoProbe;

impl Probe for NoProbe {
    fn executable(
        &mut self,
        _: &str,
        _: &Environment,
    ) -> Result<PathBuf, Issue> {
        panic!(
            "An inspection-only candidate must not resolve or execute a CLI"
        );
    }

    fn output(
        &mut self,
        _: &Command,
        _: &Environment,
    ) -> Result<String, String> {
        panic!(
            "An inspection-only candidate must not run even a version probe"
        );
    }
}

#[test]
fn inspection_only_candidates_never_probe_or_accept_owner_approval() {
    let fixture = Fixture::new();
    for (owner, name) in [
        (Owner::Pnpm, "Library/pnpm/store"),
        (Owner::Homebrew, "Library/Caches/Homebrew"),
        (Owner::Puppeteer, ".cache/puppeteer"),
    ] {
        let path = fixture.dir(name);
        let prepared = prepare_with(
            &[target(&path)],
            &fixture.root,
            fixture.env.clone(),
            &mut NoProbe,
        )
        .expect("inspection");
        assert_eq!(prepared.proposals[0].owner, Some(owner));
        assert_eq!(
            prepared.proposals[0]
                .issue
                .as_ref()
                .expect("no adapter")
                .problem,
            Problem::NoCommand
        );
        assert!(prepared.operations.is_empty());
        assert!(prepared.approve(&[Run], Action::Skip).is_err());
        assert_eq!(
            resolve_owner(owner, &path, &fixture.env, &mut NoProbe)
                .expect_err("candidate resolution is gated before probes")
                .problem,
            Problem::NoCommand
        );
        assert_eq!(
            std::fs::read(path.join("payload")).expect("untouched"),
            vec![0; 8192]
        );
    }
}

proptest::proptest! {
    #![proptest_config(proptest::test_runner::Config {
        cases: 32,
        failure_persistence: None,
        ..proptest::test_runner::Config::default()
    })]

    /// The environment points at an independently generated directory;
    /// all descendants inherit the hint and prefix neighbours do not.
    #[test]
    fn configured_browser_root_membership_matches_components(
        name in "[a-zA-Z0-9 _-]{1,24}",
        key in 0_usize..3,
        depth in 0_usize..8,
    ) {
        let mut fixture = Fixture::new();
        // Keep generated names legal on Windows too (no device basename
        // or trailing space), while exercising embedded whitespace.
        let name = format!("cache-{name}-root");
        let cache = fixture.dir(&name);
        let keys = [
            "PLAYWRIGHT_BROWSERS_PATH",
            "npm_config_playwright_browsers_path",
            "npm_package_config_playwright_browsers_path",
        ];
        fixture.env.variables.insert(keys[key].into(), cache.display().to_string());
        let mut selected = cache.clone();
        for _ in 0..depth {
            selected.push("child");
        }
        std::fs::create_dir_all(&selected).expect("descendant");
        proptest::prop_assert_eq!(
            recognize::owner(&selected, &fixture.env),
            Some((Owner::Playwright, cache))
        );
        let neighbour = fixture.dir(&format!("{name}-neighbour"));
        proptest::prop_assert!(recognize::owner(&neighbour, &fixture.env).is_none());
    }
}

#[cfg(windows)]
#[test]
fn windows_native_and_batch_candidates_are_not_execution_adapters() {
    let mut fixture = Fixture::new();
    let bin = fixture.dir("bin");
    for name in ["uv.exe", "pip.exe", "npm.cmd", "playwright.cmd"] {
        std::fs::write(bin.join(name), b"fixture, never executable")
            .expect("launcher");
    }
    fixture
        .env
        .variables
        .insert("PATH".into(), bin.display().to_string());
    for name in ["uv", "pip", "npm", "playwright"] {
        assert_eq!(
            process::SystemProbe
                .executable(name, &fixture.env)
                .expect_err("no audited Windows adapter")
                .problem,
            Problem::UnsupportedLauncher
        );
    }
    let cache = fixture.dir("Library/pnpm/store");
    let prepared = where_checkable(
        prepare_with(
            &[target(&cache)],
            &fixture.root,
            fixture.env.clone(),
            &mut NoProbe,
        )
        .expect("inspection"),
    );
    assert!(prepared.proposals[0].delete.is_none());
    assert!(prepared.approve(&[DeleteFiles], Action::Permanent).is_err());
}

/// The kernel's image path identifies a native owner even if argv[0]
/// was changed or command arguments cannot identify the owner.
#[cfg(target_os = "linux")]
#[test]
fn native_owner_image_with_unrelated_argv_revokes_cleanup() {
    use std::os::unix::process::CommandExt as _;

    let _execution = EXECUTION
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let fixture = Fixture::new();
    let cache = fixture.dir("owner-cache");
    let executable =
        std::fs::canonicalize("/bin/sleep").expect("native fixture");
    let mut operation = resolve_owner(
        Owner::Uv,
        &cache,
        &fixture.env,
        &mut fixture.probe("uv", "uv 0.11.0", &cache),
    )
    .expect("mock owner")
    .expect("operation");
    // Stand in for a resolved native CLI. No owner cleaner is invoked.
    operation.command.executable = executable.clone();
    let mut child = std::process::Command::new(&executable)
        .arg0("fixture-idle")
        .arg("30")
        .spawn()
        .expect("fixture process");
    std::thread::sleep(Duration::from_millis(100));
    let alive = child.try_wait().expect("child status").is_none();
    let checked = process::check_active(&operation);
    let _ = child.kill();
    child.wait().expect("reap fixture");
    assert!(alive, "fixture must still be running when checked");
    assert!(
        checked.is_err(),
        "The live native owner image must block cleanup: {checked:?}"
    );
    assert!(cache.join("payload").exists());
}
