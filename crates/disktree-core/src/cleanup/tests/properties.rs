//! Rules of cache cleanup checked against many generated cases rather
//! than a few chosen ones. Each test states the promise it holds, builds
//! cases from a seed, and names the seed when a case breaks the rule:
//! `PWRDISK_SEED=<seed> cargo test -p disktree-core <test>` replays it.

use std::collections::BTreeSet;

use super::*;
use crate::test_rng::Rng;

/// This run's seeds, `runs` of them. They start at `PWRDISK_SEED` to
/// replay a failure, at the clock otherwise, so every run tries cases no
/// earlier run tried.
fn seeds(runs: u64) -> impl Iterator<Item = u64> {
    let first = std::env::var("PWRDISK_SEED")
        .ok()
        .and_then(|seed| seed.parse().ok())
        .unwrap_or_else(|| {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default();
            now.as_secs().wrapping_mul(1_000_003)
                ^ u64::from(now.subsec_nanos())
        });
    (0..runs).map(move |run| first.wrapping_add(run))
}

fn pick<'a, T>(rng: &mut Rng, items: &'a [T]) -> &'a T {
    &items[usize::try_from(rng.below(items.len() as u64)).expect("index")]
}

fn count(rng: &mut Rng, low: u64, high: u64) -> usize {
    usize::try_from(low + rng.below(high - low + 1)).expect("count")
}

/// The worker visits exactly the prefix through the first refusal. The
/// fixture's one cleanable cache supplies an oracle for that position.
#[test]
fn raw_worker_visits_only_the_prefix_through_its_first_refusal() {
    for seed in seeds(32) {
        let mut rng = Rng::new(seed);
        let fixture = Fixture::new();
        let marked = count(&mut rng, 2, 8);
        let refused = count(&mut rng, 0, (marked - 1) as u64);
        let targets: Vec<_> = (0..marked)
            .map(|index| {
                let name = format!("project-{index}");
                let path = fixture.dir(&name);
                if index == refused {
                    fixture.dir(&format!("{name}/vendor/_cacache"));
                }
                target(&path)
            })
            .collect();
        let (result, visited) =
            trace_raw_targets(|| check_raw_with(&targets, &fixture.env));
        assert!(result.is_err(), "seed {seed}");
        let expected: Vec<_> = targets[..=refused]
            .iter()
            .map(|target| target.path.clone())
            .collect();
        assert_eq!(visited, expected, "seed {seed}");
    }
}

/// Per-mark results share one selection budget. The filesystem is the
/// oracle: one visit per selected directory and per immediate file. Once
/// their cumulative count exceeds the bound, no later mark is admitted.
#[test]
fn raw_inspection_results_share_the_selection_bound() {
    for seed in seeds(32) {
        let mut rng = Rng::new(seed);
        let fixture = Fixture::new();
        let mut targets = Vec::new();
        let mut visits = Vec::new();
        for index in 0..count(&mut rng, 2, 8) {
            let path = fixture.env.home.join(format!("ordinary-{index}"));
            std::fs::create_dir(&path).expect("directory");
            for file in 0..count(&mut rng, 0, 5) {
                std::fs::write(path.join(format!("file-{file}")), b"data")
                    .expect("file");
            }
            visits.push(1 + std::fs::read_dir(&path).expect("listing").count());
            targets.push(target(&path));
        }
        for maximum in 0..=visits.iter().sum() {
            let results = inspect_raw_with(&targets, &fixture.env, maximum);
            let mut required = 0;
            for (index, (result, visits)) in
                results.iter().zip(&visits).enumerate()
            {
                required += visits;
                assert_eq!(
                    result.is_ok(),
                    required <= maximum,
                    "seed {seed}, bound {maximum}, mark {index}: {result:?}"
                );
            }
        }
    }
}

/// A directory name that can hold the characters a cache root's name may
/// legally have, the ones a careless parser strips or splits on included.
#[cfg(unix)]
fn awkward_name(rng: &mut Rng) -> String {
    const PLAIN: [char; 8] = ['a', 'b', '-', '_', '.', 'é', ' ', '\t'];
    const ENDINGS: [&str; 7] = ["", "", "", " ", "\t", "\r", " \r"];
    loop {
        let mut name: String =
            (0..count(rng, 1, 6)).map(|_| *pick(rng, &PLAIN)).collect();
        if rng.percent(15) {
            name.insert(0, *pick(rng, &['\r', '\n']));
        }
        name.push_str(pick(rng, &ENDINGS));
        let trimmed = name.trim_end();
        if !matches!(name.as_str(), "." | "..")
            && !matches!(trimmed, "" | "." | "..")
        {
            return name;
        }
    }
}

/// An owner that prints exactly `output` for its cache root and the
/// given version, and refuses anything that would change something.
#[cfg(unix)]
struct Printed {
    executable: PathBuf,
    version: String,
    output: String,
}

#[cfg(unix)]
impl Probe for Printed {
    fn executable(
        &mut self,
        _: &str,
        _: &Environment,
    ) -> Result<PathBuf, Issue> {
        Ok(self.executable.clone())
    }

    fn output(
        &mut self,
        command: &Command,
        _: &Environment,
    ) -> Result<String, String> {
        match command
            .arguments
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .as_slice()
        {
            ["--version"] | ["version", "--component", "package"] => {
                Ok(self.version.clone())
            }
            ["config", "get", "cache"] | ["cache", "dir" | "path"] => {
                Ok(self.output.clone())
            }
            _ => panic!("Preview attempted a write: {command:?}"),
        }
    }
}

/// The cache root an owner command acts on is exactly the one the owner
/// printed, never a similar path found by trimming or splitting its
/// output; and a root printed plainly, with one LF, is always accepted.
///
/// The oracle is the generator: it knows the real root, and puts the
/// marked path either inside it or inside a look-alike next to it.
#[cfg(unix)]
#[test]
fn an_owner_command_acts_only_on_the_root_the_owner_printed() {
    const OWNERS: [(Owner, &str); 5] = [
        (Owner::Uv, "uv 0.11.0"),
        (Owner::Pip, "pip 24.0 from /fixture (python 3.12)"),
        (Owner::Cypress, "14.0.0"),
        (Owner::Npm, "10.8.0"),
        (Owner::Npx, "11.6.2"),
    ];
    let scope_of = |owner: Owner, root: &Path| match owner {
        Owner::Npm => root.join("_cacache"),
        Owner::Npx => root.join("_npx"),
        _ => root.to_owned(),
    };
    for seed in seeds(150) {
        let rng = &mut Rng::new(seed);
        let fixture = Fixture::new();
        let (owner, version) = *pick(rng, &OWNERS);
        let mut parent = fixture.env.home.clone();
        if rng.percent(40) {
            parent.push(awkward_name(rng));
        }
        let name = awkward_name(rng);
        let root = parent.join(&name);
        // What a parser that trims or drops a CR would read instead.
        let look_alike = parent.join(name.trim_end());
        let marked_root = if look_alike != root && rng.percent(50) {
            &look_alike
        } else {
            &root
        };
        let mut selected = scope_of(owner, marked_root);
        if rng.percent(50) {
            selected.push("entries");
        }
        for directory in [scope_of(owner, &root), selected.clone()] {
            std::fs::create_dir_all(&directory).expect("cache fixture");
        }
        let printed = root.to_str().expect("UTF-8 root");
        let terminator = *pick(rng, &["\n", "\n", "\n", "\r\n", "", "\n\n"]);
        let mut probe = Printed {
            executable: fixture.executable.clone(),
            version: version.into(),
            output: format!("{printed}{terminator}"),
        };
        let case = format!(
            "seed {seed}: {owner:?} printed {:?}, marked {selected:?}",
            probe.output
        );
        let resolved =
            resolve_owner(owner, &selected, &fixture.env, &mut probe);
        let plain = terminator == "\n"
            && !printed.contains(['\n', '\r'])
            && marked_root == &root;
        match &resolved {
            Ok(Some(operation)) => {
                assert_eq!(operation.cache_root, root, "{case}");
                assert_eq!(operation.scope, scope_of(owner, &root), "{case}");
                let named = operation
                    .command
                    .arguments
                    .iter()
                    .chain(operation.command.environment.values())
                    .any(|argument| argument == printed);
                assert!(named, "{case}: {:?}", operation.command);
            }
            Ok(None) | Err(_) => {
                assert!(!plain, "{case}: refused a plain root: {resolved:?}");
            }
        }
        if let Err(issue) = &resolved {
            assert_eq!(issue.problem, Problem::ConfigCheckFailed, "{case}");
        }
    }
}

/// Signs of a cache put on a directory in the generated tree. Only the
/// layouts of owners PwrDisk has a cleanup command for count; the rest
/// are decoys that look close. Build outputs carry the standard cache tag
/// and are decoys too: Review once sent every Cargo `target` to Cache
/// cleanup because of it. So is pnpm's store, which PwrDisk recognizes
/// but has no command for.
#[derive(Clone, Copy, Debug)]
enum Sign {
    Uv,
    Pip,
    Cypress,
    Pnpm,
    BareTag,
    CargoTarget,
    Venv,
    ToolCache,
    UvBucketOnly,
    TagDirectory,
    WheelsOnly,
    ShortCypressVersion,
}

const SIGNS: [Sign; 12] = [
    Sign::Uv,
    Sign::Pip,
    Sign::Cypress,
    Sign::Pnpm,
    Sign::BareTag,
    Sign::CargoTarget,
    Sign::Venv,
    Sign::ToolCache,
    Sign::UvBucketOnly,
    Sign::TagDirectory,
    Sign::WheelsOnly,
    Sign::ShortCypressVersion,
];

/// Names a generated directory can take. Three are the names only npm
/// and Playwright give their caches; the others merely sound like caches,
/// or belong to an owner PwrDisk has no command for.
const NAMES: [&str; 13] = [
    "_cacache",
    "_npx",
    "ms-playwright",
    "cache",
    ".cache",
    "caches",
    "Caches",
    "codex-runtimes",
    "npm-cache",
    ".npm",
    ".pnpm-store",
    "target",
    "node_modules",
];

const BARE_TAG: &str = "Signature: 8a477f597d28d172789f06886806bc55\n";

/// What Cargo writes into every target directory, word for word.
const CARGO_TAG: &str = concat!(
    "Signature: 8a477f597d28d172789f06886806bc55\n",
    "# This file is a cache directory tag created by cargo.\n",
    "# For information about cache directory tags see ",
    "https://bford.info/cachedir/\n",
);

const UV_BUCKETS: [&str; 6] = [
    "archive-v0",
    "wheels-v5",
    "sdists-v9",
    "simple-v16",
    "interpreter-v4",
    "built-wheels-v3",
];

fn put(sign: Sign, at: &Path, rng: &mut Rng) {
    let dir = |path: PathBuf| std::fs::create_dir_all(path).expect("sign");
    let file = |path: PathBuf, text: &str| {
        std::fs::write(path, text).expect("sign file");
    };
    match sign {
        Sign::Uv => {
            file(at.join("CACHEDIR.TAG"), BARE_TAG);
            for _ in 0..count(rng, 1, 3) {
                dir(at.join(pick(rng, &UV_BUCKETS)));
            }
        }
        Sign::Pip => {
            dir(at.join("wheels"));
            dir(at.join(pick(rng, &["http", "http-v2"])));
        }
        Sign::Cypress => {
            let version = format!("{}.{}.0", 10 + rng.below(6), rng.below(9));
            if rng.percent(50) {
                dir(at.join(version).join("Cypress.app"));
            } else {
                dir(at.join(&version).join("Cypress"));
                file(at.join(version).join("Cypress/Cypress"), "");
            }
        }
        Sign::Pnpm => dir(at.join(format!("v{}/files", rng.below(20)))),
        Sign::BareTag => file(at.join("CACHEDIR.TAG"), BARE_TAG),
        Sign::CargoTarget => {
            file(at.join("CACHEDIR.TAG"), CARGO_TAG);
            file(at.join(".rustc_info.json"), "{}");
            let profile = at.join(pick(rng, &["debug", "release"]));
            for part in ["deps", "incremental", ".fingerprint", "build"] {
                dir(profile.join(part));
            }
            if rng.percent(40) {
                let triple = at.join("x86_64-pc-windows-msvc");
                dir(triple.join("debug/deps"));
                file(triple.join("CACHEDIR.TAG"), CARGO_TAG);
            }
        }
        Sign::Venv => {
            file(at.join("CACHEDIR.TAG"), BARE_TAG);
            file(at.join("pyvenv.cfg"), "home = /usr/bin\n");
            dir(at.join("lib/python3.12/site-packages"));
            dir(at.join("bin"));
        }
        Sign::ToolCache => {
            file(at.join("CACHEDIR.TAG"), BARE_TAG);
            file(at.join(".gitignore"), "*\n");
            dir(at.join(pick(rng, &["0.8.4", "v", "content"])));
        }
        Sign::UvBucketOnly => dir(at.join(pick(rng, &UV_BUCKETS))),
        Sign::TagDirectory => dir(at.join("CACHEDIR.TAG")),
        Sign::WheelsOnly => dir(at.join("wheels")),
        Sign::ShortCypressVersion => dir(at.join("14.0/Cypress.app")),
    }
}

/// The oracle: is this directory itself the root of a cache PwrDisk can
/// clean with its owner's command? Reads every entry, where the code
/// under test reads a bounded few, and spells each owner out on its own.
fn cleanable(directory: &Path) -> bool {
    let name = directory.file_name().and_then(|name| name.to_str());
    if matches!(name, Some("_cacache" | "_npx" | "ms-playwright")) {
        return true;
    }
    let is_dir = |name: &str| directory.join(name).is_dir();
    if is_dir("wheels") && (is_dir("http") || is_dir("http-v2")) {
        return true;
    }
    let tagged = directory.join("CACHEDIR.TAG").is_file();
    std::fs::read_dir(directory)
        .into_iter()
        .flatten()
        .flatten()
        .any(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = entry.path();
            let uv = tagged && path.is_dir() && UV_BUCKETS.contains(&&*name);
            let parts: Vec<_> = name.split('.').collect();
            let cypress = parts.len() == 3
                && parts.iter().all(|part| {
                    !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit())
                })
                && (path.join("Cypress.app").is_dir()
                    || path.join("Cypress/Cypress").is_file());
            uv || cypress
        })
}

fn directories_under(path: &Path) -> Vec<PathBuf> {
    let mut found = vec![];
    let mut pending = vec![path.to_owned()];
    while let Some(directory) = pending.pop() {
        if !directory.is_dir() {
            continue;
        }
        for entry in std::fs::read_dir(&directory).expect("listing") {
            pending.push(entry.expect("entry").path());
        }
        found.push(directory);
    }
    found
}

/// Cache cleanup steps in only for a cache PwrDisk can clean better than
/// by deleting it. A path in such a cache goes to Cache cleanup in Review,
/// and plain removal leaves it to Cache cleanup; so does a folder with
/// one inside. Everything else stays an ordinary removal, whatever its
/// name, tag or hatch says: `cache`, `.cache`, Cargo's `target`, a venv,
/// or pnpm's store, which has no command here.
///
/// The oracle walks every ancestor up to the fixture and every directory
/// below the path, reading each one whole.
#[test]
fn only_a_cleanable_cache_holds_a_removal_back() {
    for seed in seeds(150) {
        let rng = &mut Rng::new(seed);
        let fixture = Fixture::new();
        let mut chain = vec![fixture.env.home.join(format!("n{seed:x}"))];
        for level in 0..count(rng, 1, 5) {
            let name = if rng.percent(35) {
                (*pick(rng, &NAMES)).to_owned()
            } else {
                format!("n{level}")
            };
            let next = chain.last().expect("level").join(name);
            chain.push(next);
        }
        let deepest = chain.last().expect("deepest");
        std::fs::create_dir_all(deepest).expect("chain");
        std::fs::write(deepest.join("payload"), [0; 64]).expect("payload");
        for (level, directory) in chain.iter().enumerate() {
            if rng.percent(30) {
                let name = if rng.percent(50) {
                    (*pick(rng, &NAMES)).to_owned()
                } else {
                    format!("m{level}")
                };
                std::fs::create_dir_all(directory.join(name))
                    .expect("neighbour");
            }
            if rng.percent(25) {
                put(*pick(rng, &SIGNS), directory, rng);
            }
        }
        let selected = if rng.percent(25) {
            deepest.join("payload")
        } else {
            pick(rng, &chain).clone()
        };
        let above = selected
            .ancestors()
            .take_while(|ancestor| ancestor.starts_with(&fixture.root))
            .any(cleanable);
        let inside = directories_under(&selected).iter().any(|d| cleanable(d));
        let case = format!("seed {seed}: {selected:?}");
        let hint = recognize::inspection_with(&selected, &fixture.env);
        assert_eq!(hint.is_some(), above, "{case}: {hint:?}");
        // Cache cleanup asks the owner Review named, not a nearer one
        // without a command.
        if above {
            let owner = recognize::hinted(&selected, &fixture.env);
            assert!(
                owner.as_ref().is_some_and(|(owner, _)| owner.has_cleanup()),
                "{case}: {hint:?} but Cache cleanup asks {owner:?}"
            );
        }
        let raw = check_raw_with(&[target(&selected)], &fixture.env);
        assert_eq!(raw.is_err(), above || inside, "{case}: {raw:?}");
        assert!(selected.exists(), "{case}: checking cannot delete");
    }
}

/// A wrapper that holds an owner's cache where the owner keeps it, such
/// as `~/.cache` with uv's cache in it, goes to Cache cleanup in Review
/// as soon as it is marked, not only when removal is refused later. One
/// that holds only caches PwrDisk has no command for stays ordinary.
#[test]
fn a_wrapper_holding_a_cleanable_cache_is_named_in_review() {
    for seed in seeds(60) {
        let rng = &mut Rng::new(seed);
        let fixture = Fixture::new();
        let home = &fixture.env.home;
        let homes: [(&str, bool); 6] = [
            (".cache/uv", true),
            (".cache/pip", true),
            ("Library/Caches/ms-playwright", true),
            (".npm/_cacache", true),
            ("Library/Caches/Homebrew", false),
            (".cache/some-app", false),
        ];
        let planted: Vec<_> =
            homes.iter().filter(|_| rng.percent(40)).copied().collect();
        // A root that is a link is not deleted with its wrapper, which
        // removes the link only, so it holds nothing back.
        let elsewhere = fixture.root.join("elsewhere");
        let mut linked = Vec::new();
        for (path, _) in &planted {
            let root = home.join(path);
            #[cfg(unix)]
            if rng.percent(25) {
                std::fs::create_dir_all(elsewhere.join(path).join("entry"))
                    .expect("cache elsewhere");
                std::fs::create_dir_all(root.parent().expect("parent"))
                    .expect("parent");
                std::os::unix::fs::symlink(elsewhere.join(path), &root)
                    .expect("link");
                linked.push(true);
                continue;
            }
            std::fs::create_dir_all(root.join("entry")).expect("cache");
            linked.push(false);
        }
        // Only Unix makes links here.
        let _ = &elsewhere;
        let wrapper = home
            .join(pick(rng, &[".cache", "Library", "Library/Caches", ".npm"]));
        std::fs::create_dir_all(&wrapper).expect("wrapper");
        let holds =
            planted
                .iter()
                .zip(&linked)
                .any(|((path, cleanable), link)| {
                    *cleanable && !link && home.join(path).starts_with(&wrapper)
                });
        let case = format!("seed {seed}: {wrapper:?} with {planted:?}");
        let hint = recognize::inspection_with(&wrapper, &fixture.env);
        assert_eq!(hint.is_some(), holds, "{case}: {hint:?}");
        assert_eq!(
            check_raw_with(&[target(&wrapper)], &fixture.env).is_err(),
            holds,
            "{case}"
        );
    }
}

/// How the owner's executable is reached on PATH.
#[cfg(unix)]
#[derive(Clone, Copy, Debug)]
enum Reach {
    /// A file in an ordinary directory.
    Direct,
    /// A link into an installation, as npm's own `npm -> npm-cli.js`.
    Installed,
    /// A file in a `shims` directory on PATH.
    InShims,
    /// A link into some `shims` directory.
    LinkToShims,
    /// A link to Volta's dispatcher.
    VoltaShim,
    /// A file in `~/.volta/bin`.
    VoltaBin,
}

/// What runs the script, read from its first line.
#[cfg(unix)]
#[derive(Clone, Copy, Debug)]
enum Interpreter {
    Shell,
    /// `env node`, with node on PATH reached as given.
    Env(Reach),
    /// `env node --flag`: arguments env would pass on.
    EnvWithArguments,
    /// A shim named directly on the first line.
    ShimPath,
}

#[cfg(unix)]
const fn installs(reach: Reach) -> bool {
    matches!(
        reach,
        Reach::InShims
            | Reach::LinkToShims
            | Reach::VoltaShim
            | Reach::VoltaBin
    )
}

/// Write a script that records that it ran and prints `version`, and put
/// it on a PATH directory as `name`, reached as `reach`. Returns the PATH
/// directory it is reachable from.
#[cfg(unix)]
fn place(
    root: &Path,
    name: &str,
    target_name: &str,
    reach: Reach,
    first_line: &str,
    version: &str,
) -> PathBuf {
    use std::os::unix::fs::PermissionsExt as _;
    let ran = root.join("ran");
    let script = |path: &Path| {
        std::fs::create_dir_all(path.parent().expect("parent")).expect("dir");
        std::fs::write(
            path,
            format!(
                concat!(
                    "{first_line}\n",
                    "printf '%s\\n' \"$0\" >> '{ran}'\n",
                    "printf '%s\\n' '{version}'\n",
                ),
                first_line = first_line,
                ran = ran.display(),
                version = version,
            ),
        )
        .expect("script");
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
            .expect("executable");
    };
    let link = |from: &Path, to: &Path| {
        std::fs::create_dir_all(from.parent().expect("parent")).expect("dir");
        std::os::unix::fs::symlink(to, from).expect("link");
    };
    let path_dir = |dir: &str| root.join(format!("{dir}-{name}"));
    match reach {
        Reach::Direct => {
            let dir = path_dir("bin");
            script(&dir.join(name));
            dir
        }
        Reach::Installed => {
            let dir = path_dir("bin");
            let installed = path_dir("lib").join("dist").join(target_name);
            script(&installed);
            link(&dir.join(name), &installed);
            dir
        }
        Reach::InShims => {
            let dir = path_dir("version-manager").join("shims");
            script(&dir.join(name));
            dir
        }
        Reach::LinkToShims => {
            let dir = path_dir("bin");
            let shim = path_dir("version-manager").join("shims").join(name);
            script(&shim);
            link(&dir.join(name), &shim);
            dir
        }
        Reach::VoltaShim => {
            let dir = path_dir("bin");
            let shim = path_dir("volta").join("volta-shim");
            script(&shim);
            link(&dir.join(name), &shim);
            dir
        }
        Reach::VoltaBin => {
            let dir = path_dir("home").join(".volta").join("bin");
            script(&dir.join(name));
            dir
        }
    }
}

/// Finding an owner's executable never runs one that could install
/// software on demand: a version manager's shim, Volta's dispatcher, an
/// npm that is not npm's own `npm-cli.js`, or a script whose interpreter
/// is any of those. Everything else is found, and its version probed.
///
/// The oracle is the generator's own record of what each layout is, and
/// every generated script writes down that it ran.
#[cfg(unix)]
#[test]
fn probing_never_runs_a_launcher_that_could_install() {
    const REACHES: [Reach; 6] = [
        Reach::Direct,
        Reach::Installed,
        Reach::InShims,
        Reach::LinkToShims,
        Reach::VoltaShim,
        Reach::VoltaBin,
    ];
    const OWNERS: [(Owner, &str, &str); 5] = [
        (Owner::Uv, "uv", "uv 0.11.0"),
        (Owner::Pip, "pip", "pip 24.0 from /fixture (python 3.12)"),
        (Owner::Pip, "pip3", "pip 24.0 from /fixture (python 3.12)"),
        (Owner::Cypress, "cypress", "14.0.0"),
        (Owner::Npm, "npm", "10.8.0"),
    ];
    // Generated owners are real processes named like real owners, which
    // another test's in-use check would see.
    let _execution = EXECUTION
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    for seed in seeds(60) {
        let rng = &mut Rng::new(seed);
        let fixture = Fixture::new();
        let (owner, name, version) = *pick(rng, &OWNERS);
        let reach = *pick(rng, &REACHES);
        let interpreter = match rng.below(4) {
            0 => Interpreter::Env(*pick(rng, &REACHES)),
            1 if rng.percent(50) => Interpreter::EnvWithArguments,
            1 => Interpreter::ShimPath,
            _ => Interpreter::Shell,
        };
        let root = &fixture.root;
        let mut path = vec![];
        let first_line = match interpreter {
            Interpreter::Shell => "#!/bin/sh".to_owned(),
            Interpreter::Env(node) => {
                path.push(place(
                    root,
                    "node",
                    "node",
                    node,
                    "#!/bin/sh",
                    version,
                ));
                "#!/usr/bin/env node".to_owned()
            }
            Interpreter::EnvWithArguments => {
                "#!/usr/bin/env node --flag".into()
            }
            Interpreter::ShimPath => {
                let shims = place(
                    root,
                    "node",
                    "node",
                    Reach::InShims,
                    "#!/bin/sh",
                    version,
                );
                format!("#!{}", shims.join("node").display())
            }
        };
        // npm's distribution names its entry point npm-cli.js; a link
        // to anything else is a proxy such as Corepack.
        let target_name = if owner == Owner::Npm && rng.percent(60) {
            "npm-cli.js"
        } else {
            name
        };
        path.insert(
            0,
            place(root, name, target_name, reach, &first_line, version),
        );
        let reached_npm_cli =
            matches!(reach, Reach::Installed) && target_name == "npm-cli.js";
        let unsafe_launcher = installs(reach)
            || (owner == Owner::Npm && !reached_npm_cli)
            || match interpreter {
                Interpreter::Shell => false,
                Interpreter::Env(node) => installs(node),
                Interpreter::EnvWithArguments | Interpreter::ShimPath => true,
            };
        let mut env = fixture.env.clone();
        env.variables.extend([
            (
                "PATH".into(),
                std::env::join_paths(&path)
                    .expect("PATH")
                    .into_string()
                    .expect("UTF-8"),
            ),
            ("HOME".into(), env.home.to_str().expect("home").into()),
        ]);
        let cache = env.home.join("cache");
        let case = format!(
            "seed {seed}: {name} reached {reach:?}, run by {interpreter:?}, \
             linked to {target_name}"
        );
        let resolved =
            resolve_owner(owner, &cache, &env, &mut process::SystemProbe);
        let ran = std::fs::read_to_string(root.join("ran")).unwrap_or_default();
        let issue = resolved.expect_err(&case);
        if unsafe_launcher {
            assert_eq!(issue.problem, Problem::UnsupportedLauncher, "{case}");
            assert_eq!(ran, "", "{case}: ran before it was refused");
        } else {
            // Our scripts print a version for a cache root, so the root
            // check fails after the probe: proof that the probe ran.
            assert_eq!(issue.problem, Problem::ConfigCheckFailed, "{case}");
            assert_ne!(ran, "", "{case}: an ordinary launcher was not run");
        }
    }
}

/// A running owner is recognized whatever form launched it: the friendly
/// name, a versioned name such as `pip3.12`, the companion runner such as
/// `uvx`, the node entry points npm runs through, or the exact executable
/// the command names. Programs that only share a prefix are not.
///
/// The oracle is the list of each owner's launcher names.
#[test]
fn a_running_owner_is_recognized_in_every_launcher_form() {
    const OTHERS: [&str; 10] = [
        "node",
        "python3",
        "python3.12",
        "bash",
        "unrelated.js",
        "npm-cli.json",
        "pipx",
        "uvicorn",
        "cypress.json",
        "pip3.x",
    ];
    const OWNERS: [Owner; 5] = [
        Owner::Npm,
        Owner::Npx,
        Owner::Uv,
        Owner::Pip,
        Owner::Cypress,
    ];
    const DIRECTORIES: [&str; 4] = [
        "",
        "/usr/local/bin/",
        "/opt/tool/lib/node_modules/npm/bin/",
        "/install/bin/",
    ];
    let launchers = |owner: Owner| -> Vec<String> {
        match owner {
            Owner::Npm | Owner::Npx => {
                ["npm", "npx", "npm-cli.js", "npx-cli.js"]
                    .map(String::from)
                    .to_vec()
            }
            Owner::Uv => vec!["uv".into(), "uvx".into()],
            Owner::Pip => vec!["pip".into(), "pip3".into(), "pip3.12".into()],
            _ => vec![owner.executable().into()],
        }
    };
    for seed in seeds(400) {
        let rng = &mut Rng::new(seed);
        let owner = *pick(rng, &OWNERS);
        let names = launchers(owner);
        let executable = rng
            .percent(50)
            .then(|| PathBuf::from("/install/bin").join(pick(rng, &names)));
        let launcher = rng.percent(60);
        let base = if launcher {
            pick(rng, &names).clone()
        } else {
            (*pick(rng, &OTHERS)).to_owned()
        };
        let argument = format!("{}{base}", pick(rng, &DIRECTORIES));
        let expected = launcher
            || executable
                .as_ref()
                .is_some_and(|executable| executable.ends_with(&base));
        assert_eq!(
            process::owner_named(owner, executable.as_deref(), &argument),
            expected,
            "seed {seed}: {owner:?} with executable {executable:?} \
             launched as {argument:?}"
        );
    }
}

/// A generated set of marked paths, owner commands and decisions, over
/// real directories so the removal guards see a real tree.
struct Cleanup {
    _fixture: Fixture,
    nodes: Vec<PathBuf>,
    prepared: Prepared,
    decisions: Vec<Decision>,
    files: Action,
}

fn generate(rng: &mut Rng) -> Cleanup {
    let fixture = Fixture::new();
    let root = fixture.env.home.join("scan");
    let mut nodes = vec![root.clone()];
    for index in 0..count(rng, 4, 12) {
        let parent = pick(rng, &nodes).clone();
        nodes.push(parent.join(format!("d{index}")));
    }
    for node in &nodes {
        std::fs::create_dir_all(node).expect("tree");
    }
    let candidates = &nodes[1..];
    let mut scopes: Vec<PathBuf> = vec![];
    for _ in 0..count(rng, 0, 3) {
        let scope = pick(rng, candidates).clone();
        if !scopes.contains(&scope) {
            scopes.push(scope);
        }
    }
    let operations = scopes
        .iter()
        .map(|scope| Operation {
            owner: Owner::Uv,
            version: "uv 0.11.0".into(),
            selected: vec![],
            cache_root: scope.clone(),
            scope: scope.clone(),
            command: Command {
                executable: "/fixture/uv".into(),
                arguments: vec!["cache".into(), "clean".into()],
                environment: BTreeMap::new(),
                working_directory: fixture.env.home.clone(),
            },
            effects: String::new(),
            limitations: String::new(),
        })
        .collect::<Vec<_>>();
    let mut selected = candidates.to_vec();
    rng.shuffle(&mut selected);
    selected.truncate(count(rng, 1, 5));
    let proposals: Vec<Proposal> = selected
        .iter()
        .map(|path| Proposal {
            selected: path.clone(),
            bytes: 1,
            is_dir: true,
            hint: None,
            owner: None,
            // An owner's scope contains the marked path, or, for a wrapper
            // such as npm's, lies inside it.
            operations: scopes
                .iter()
                .enumerate()
                .filter(|(_, scope)| {
                    (path.starts_with(scope) || scope.starts_with(path))
                        && rng.percent(70)
                })
                .map(|(index, _)| index)
                .collect(),
            issue: None,
            delete: rng.percent(60).then(|| Disclosure {
                path: path.clone(),
                contains: vec![],
                warning: String::new(),
            }),
            delete_issue: None,
        })
        .collect();
    let decisions = proposals
        .iter()
        .map(|proposal| {
            let offered: Vec<_> =
                [Decision::Skip, Decision::Owner, Decision::DeleteFiles]
                    .into_iter()
                    .filter(|decision| proposal.allows(*decision))
                    .collect();
            *pick(rng, &offered)
        })
        .collect();
    let files = *pick(
        rng,
        &[
            Action::Trash,
            Action::Permanent,
            Action::Trash,
            Action::Skip,
        ],
    );
    let prepared = Prepared {
        proposals,
        scope_bytes: vec![0; operations.len()],
        operations,
        root,
        environment: fixture.env.clone(),
        identities: vec![],
    };
    Cleanup {
        _fixture: fixture,
        nodes,
        prepared,
        decisions,
        files,
    }
}

/// What a decision acts on when it runs.
fn reached(prepared: &Prepared, step: Step) -> &Path {
    match step {
        Step::Owner(index) => &prepared.operations[index].scope,
        Step::Files { proposal, .. } => &prepared.proposals[proposal].selected,
    }
}

/// Approval turns decisions into steps that keep each one:
///
/// * Skip means nothing happens to the path: no step acts inside it or
///   on anything around it.
/// * No step acts inside another, so nothing is acted on twice.
/// * A path chosen for its owner command is reached only by owner
///   commands, and one chosen for Delete files only by Delete files.
/// * The steps are exactly the chosen commands, and the deletions cover
///   exactly the paths chosen for Delete files.
/// * The confirmation names exactly the intents present.
///
/// Decisions that keep all of those are approved. The oracle is the set
/// of directories each step reaches, compared node by node.
#[test]
fn approval_keeps_every_decision_and_skip_means_nothing_happens() {
    for seed in seeds(300) {
        let rng = &mut Rng::new(seed);
        let cleanup = generate(rng);
        let prepared = &cleanup.prepared;
        let decisions = &cleanup.decisions;
        let under = |path: &Path| -> BTreeSet<&PathBuf> {
            cleanup
                .nodes
                .iter()
                .filter(|node| node.starts_with(path))
                .collect()
        };
        let chosen = |wanted: Decision| {
            prepared
                .proposals
                .iter()
                .zip(decisions)
                .filter(move |(_, decision)| **decision == wanted)
                .map(|(proposal, _)| proposal)
        };
        let commands: BTreeSet<usize> = chosen(Decision::Owner)
            .flat_map(|proposal| proposal.operations.iter().copied())
            .collect();
        // Every path some decision acts on, with the decision.
        let actions: Vec<(&Path, Decision)> = commands
            .iter()
            .map(|index| {
                (prepared.operations[*index].scope.as_path(), Decision::Owner)
            })
            .chain(chosen(Decision::DeleteFiles).map(|proposal| {
                (proposal.selected.as_path(), Decision::DeleteFiles)
            }))
            .collect();
        let consistent = prepared.proposals.iter().zip(decisions).all(
            |(proposal, decision)| {
                actions.iter().all(|(path, action)| {
                    action == decision
                        || under(path).is_disjoint(&under(&proposal.selected))
                })
            },
        ) && commands.iter().all(|left| {
            commands.iter().all(|right| {
                left == right
                    || under(&prepared.operations[*left].scope)
                        .is_disjoint(&under(&prepared.operations[*right].scope))
            })
        });
        let deletes = chosen(Decision::DeleteFiles).count() > 0;
        let acceptable = consistent
            && !actions.is_empty()
            && (!deletes
                || matches!(cleanup.files, Action::Trash | Action::Permanent));
        let case = format!(
            "seed {seed}: {:#?} decided {decisions:?} with {:?}",
            prepared.proposals, cleanup.files
        );
        let approved = match prepared.approve(decisions, cleanup.files) {
            Ok(approved) => approved,
            Err(error) => {
                assert!(!acceptable, "{case}: refused: {error}");
                continue;
            }
        };
        assert!(acceptable, "{case}: approved {:?}", approved.steps);
        let reach: Vec<_> = approved
            .steps
            .iter()
            .map(|step| (*step, under(reached(prepared, *step))))
            .collect();
        for (index, (step, nodes)) in reach.iter().enumerate() {
            for (other, other_nodes) in &reach[index + 1..] {
                assert!(
                    nodes.is_disjoint(other_nodes),
                    "{case}: {step:?} and {other:?} overlap"
                );
            }
        }
        for (proposal, decision) in prepared.proposals.iter().zip(decisions) {
            let own = under(&proposal.selected);
            for (step, nodes) in &reach {
                let action = match step {
                    Step::Owner(_) => Decision::Owner,
                    Step::Files { .. } => Decision::DeleteFiles,
                };
                assert!(
                    action == *decision || own.is_disjoint(nodes),
                    "{case}: {step:?} reaches {:?}, decided {decision:?}",
                    proposal.selected
                );
            }
        }
        let owners: BTreeSet<usize> = approved
            .steps
            .iter()
            .filter_map(|step| match step {
                Step::Owner(index) => Some(*index),
                Step::Files { .. } => None,
            })
            .collect();
        assert_eq!(owners, commands, "{case}");
        let deleted: Vec<&PathBuf> = approved
            .steps
            .iter()
            .filter_map(|step| match step {
                Step::Files { proposal, action } => {
                    assert_eq!(*action, cleanup.files, "{case}");
                    Some(&prepared.proposals[*proposal].selected)
                }
                Step::Owner(_) => None,
            })
            .collect();
        for proposal in chosen(Decision::DeleteFiles) {
            assert!(
                deleted
                    .iter()
                    .any(|path| proposal.selected.starts_with(path)),
                "{case}: {:?} is not deleted",
                proposal.selected
            );
        }
        for path in &deleted {
            assert!(
                chosen(Decision::DeleteFiles).any(|p| p.selected == **path),
                "{case}: {path:?} was not chosen for Delete files"
            );
        }
        let skipped: Vec<_> =
            approved.skipped.iter().map(|report| &report.path).collect();
        let skips: Vec<_> = chosen(Decision::Skip)
            .map(|proposal| &proposal.selected)
            .collect();
        assert_eq!(skipped, skips, "{case}");
        let phrase = approved.confirmation();
        assert_eq!(
            (
                phrase.contains("RUN OWNER CLEANUP"),
                phrase.contains("TRASH"),
                phrase.contains("DELETE"),
            ),
            (
                !owners.is_empty(),
                !deleted.is_empty() && cleanup.files == Action::Trash,
                !deleted.is_empty() && cleanup.files == Action::Permanent,
            ),
            "{case}: {phrase}"
        );
    }
}
