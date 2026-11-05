//! Cheap recognition of tool caches. A path goes to Cache cleanup only
//! when it is positively one PwrDisk can clean better than by deleting
//! it: a cache whose owner command has an adapter here. A name such as
//! `cache`, a cache tag or a regenerable hatch is not that, and never
//! holds a removal back. The owner's own query must still confirm a root
//! before a command can act on it.

use std::path::{Path, PathBuf};

use super::{Environment, Owner};

/// Where each owner keeps its cache by default or as configured.
fn roots(env: &Environment) -> Vec<(Owner, PathBuf)> {
    let cache = env
        .variables
        .get("XDG_CACHE_HOME")
        .map_or_else(|| env.home.join(".cache"), PathBuf::from);
    let library = env.home.join("Library/Caches");
    let mut roots = vec![
        (Owner::Npm, env.home.join(".npm/_cacache")),
        (Owner::Npx, env.home.join(".npm/_npx")),
        (Owner::Uv, cache.join("uv")),
        (Owner::Pip, cache.join("pip")),
        (Owner::Pip, library.join("pip")),
        (Owner::Cypress, cache.join("Cypress")),
        (Owner::Cypress, library.join("Cypress")),
        (Owner::Pnpm, env.home.join(".local/share/pnpm/store")),
        (Owner::Pnpm, env.home.join("Library/pnpm/store")),
        (Owner::Homebrew, library.join("Homebrew")),
        (Owner::Homebrew, cache.join("Homebrew")),
        (Owner::Playwright, cache.join("ms-playwright")),
        (Owner::Playwright, library.join("ms-playwright")),
        (Owner::Puppeteer, cache.join("puppeteer")),
    ];
    if let Some(local) = env.variables.get("LOCALAPPDATA") {
        let local = PathBuf::from(local);
        roots.extend([
            (Owner::Npm, local.join("npm-cache/_cacache")),
            (Owner::Npx, local.join("npm-cache/_npx")),
            (Owner::Uv, local.join("uv/cache")),
            (Owner::Pip, local.join("pip/Cache")),
            (Owner::Playwright, local.join("ms-playwright")),
        ]);
    }
    for (key, owner) in [
        ("UV_CACHE_DIR", Owner::Uv),
        ("PIP_CACHE_DIR", Owner::Pip),
        ("CYPRESS_CACHE_FOLDER", Owner::Cypress),
        ("HOMEBREW_CACHE", Owner::Homebrew),
        ("PLAYWRIGHT_BROWSERS_PATH", Owner::Playwright),
        ("PUPPETEER_CACHE_DIR", Owner::Puppeteer),
    ] {
        if let Some(value) = env.variables.get(key) {
            let root = PathBuf::from(value);
            if root.is_absolute() {
                roots.push((owner, root));
            }
        }
    }
    for key in ["npm_config_cache", "NPM_CONFIG_CACHE"] {
        if let Some(root) = env.variables.get(key) {
            roots.push((Owner::Npm, PathBuf::from(root).join("_cacache")));
            roots.push((Owner::Npx, PathBuf::from(root).join("_npx")));
        }
    }
    roots
}

/// The owner a cache directory's own name gives away, wherever it is.
fn named_owner(directory: &Path) -> Option<Owner> {
    match directory.file_name()?.to_str()? {
        "_cacache" => Some(Owner::Npm),
        "_npx" => Some(Owner::Npx),
        ".pnpm-store" => Some(Owner::Pnpm),
        "ms-playwright" => Some(Owner::Playwright),
        _ => None,
    }
}

/// The owner whose cache holds `path`, and that cache's root: a default
/// or configured root, or a directory named the way only that owner
/// names it. Every owner, for Cache cleanup's proposals.
pub(super) fn owner(
    path: &Path,
    env: &Environment,
) -> Option<(Owner, PathBuf)> {
    roots(env)
        .into_iter()
        .find(|(_, root)| path.starts_with(root))
        .or_else(|| {
            path.ancestors().find_map(|root| {
                named_owner(root).map(|owner| (owner, root.to_owned()))
            })
        })
}

/// The standard cache directory tag (<https://bford.info/cachedir/>).
const TAG: &str = "CACHEDIR.TAG";

/// One of the versioned buckets uv keeps at its cache root, such as
/// `wheels-v5` or `archive-v0`. Old bucket versions are accepted too: a
/// cache an older uv wrote is still uv's.
fn uv_bucket(name: &str) -> bool {
    const STEMS: [&str; 12] = [
        "archive",
        "binaries",
        "builds",
        "built-wheels",
        "environments",
        "flat-index",
        "git",
        "interpreter",
        "python",
        "sdists",
        "simple",
        "wheels",
    ];
    name.rsplit_once("-v").is_some_and(|(stem, version)| {
        STEMS.contains(&stem)
            && !version.is_empty()
            && version.bytes().all(|byte| byte.is_ascii_digit())
    })
}

/// The owner a directory's own layout gives away, wherever it was moved.
///
/// A cache tag on its own is not an owner. Cargo tags every `target`
/// directory and `~/.cargo/registry`, virtualenv and uv tag each `.venv`,
/// and ruff, pytest and mypy tag their project caches: all of it is the
/// writer's to rebuild, by the tag's own definition, and none of it has
/// an owner command to run. Of the supported owners only uv writes the
/// tag, so a tag counts when uv's buckets sit beside it.
pub(super) fn shape_owner(root: &Path) -> Option<Owner> {
    shapes(root).into_iter().next()
}

/// Every owner whose layout `root` has. A directory can match more than
/// one, and one PwrDisk has no command for must not hide one it has.
fn shapes(root: &Path) -> Vec<Owner> {
    let mut found = Vec::new();
    if root.join("wheels").is_dir()
        && (root.join("http-v2").is_dir() || root.join("http").is_dir())
    {
        found.push(Owner::Pip);
    }
    // Versioned stores and binary caches can be relocated to arbitrary
    // names. Read only a bounded immediate listing for recognition; the
    // installed owner must still verify the exact configured root.
    let tagged = root.join(TAG).is_file();
    let Ok(entries) = std::fs::read_dir(root) else {
        return found;
    };
    for entry in entries.take(32).flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if tagged && uv_bucket(name) && path.is_dir() {
            found.push(Owner::Uv);
        }
        if name.strip_prefix('v').is_some_and(|version| {
            !version.is_empty()
                && version.bytes().all(|byte| byte.is_ascii_digit())
        }) && path.join("files").is_dir()
        {
            found.push(Owner::Pnpm);
        }
        if name.split('.').count() == 3
            && name
                .bytes()
                .all(|byte| byte.is_ascii_digit() || byte == b'.')
            && (path.join("Cypress.app").is_dir()
                || path.join("Cypress/Cypress").is_file())
        {
            found.push(Owner::Cypress);
        }
    }
    found
}

/// The cleanable owner whose cache starts at `directory` by its name or
/// its layout, wherever it was moved.
fn cleanable_here(directory: &Path) -> Option<Owner> {
    named_owner(directory)
        .into_iter()
        .chain(shapes(directory))
        .find(|owner| owner.has_cleanup())
}

/// Why `path` should go through Cache cleanup rather than plain removal.
///
/// It should when it is in, or holds, a cache PwrDisk recognizes and has
/// the owner's command for. Review switches to Cache cleanup for it, and
/// Trash and Permanent leave it to Cache cleanup. Everything else is an
/// ordinary removal, cache-like or not.
pub fn inspection_reason(path: &Path) -> Option<String> {
    let env = Environment::current().ok()?;
    inspection_with(path, &env)
}

pub(super) fn inspection_with(
    path: &Path,
    env: &Environment,
) -> Option<String> {
    // Any cleanable cache above, not only the nearest owner: npm's npx
    // cache can hold a pnpm store, which has no command of its own.
    let inside = roots(env)
        .into_iter()
        .find(|(owner, root)| owner.has_cleanup() && path.starts_with(root))
        .map(|(owner, _)| owner)
        .or_else(|| path.ancestors().find_map(cleanable_here));
    if let Some(owner) = inside {
        return Some(format!(
            "In the {}: Cache cleanup can run `{}` for it instead of deleting its files",
            owner.label(),
            owner.command_label()
        ));
    }
    // A wrapper such as ~/.cache or ~/Library/Caches, marked whole. Only
    // the roots where owners keep their caches are looked for here, so
    // Review stays cheap; the removal worker's walk finds the rest.
    roots(env)
        .into_iter()
        .filter(|(owner, root)| {
            owner.has_cleanup() && root.starts_with(path) && root.is_dir()
        })
        .map(|(owner, root)| {
            format!(
                "Holds the {} at {}: Cache cleanup can run `{}` for it instead of deleting its files",
                owner.label(),
                root.display(),
                owner.command_label()
            )
        })
        .next()
}

/// The cleanable cache rooted exactly at `directory`, for the walk below a
/// marked path: its own root, name or layout, never an ancestor's, which
/// the marked path's own check has already seen.
pub(super) fn cleanable_root(
    directory: &Path,
    env: &Environment,
) -> Option<Owner> {
    roots(env)
        .into_iter()
        .find(|(owner, root)| owner.has_cleanup() && root == directory)
        .map(|(owner, _)| owner)
        .or_else(|| cleanable_here(directory))
}

/// The owner a path most likely belongs to: a known or configured root
/// containing it, or a recognizable layout a few levels up. A hint picks
/// which owner to ask; only that owner's own query confirms a root.
pub(super) fn hinted(
    path: &Path,
    env: &Environment,
) -> Option<(Owner, PathBuf)> {
    owner(path, env).or_else(|| {
        path.ancestors().take(4).find_map(|root| {
            shape_owner(root).map(|owner| (owner, root.to_owned()))
        })
    })
}

// Enough to name the caches inside a wrapper such as ~/.cache without
// walking every file of every cache in it on the preview worker.
const CONTAINED_DEPTH: usize = 3;
const CONTAINED_DIRECTORIES: usize = 20_000;
const CONTAINED_LISTED: usize = 8;

/// Cleanable caches strictly inside `path`, for the disclosure shown
/// before deleting it directly: their owners are bypassed too. Bounded,
/// and fails closed so a disclosure is never silently incomplete.
pub(super) fn contained(
    path: &Path,
    env: &Environment,
) -> Result<Vec<String>, String> {
    let mut found = Vec::new();
    let mut pending = vec![(path.to_owned(), 0)];
    let mut visited = 0;
    while let Some((directory, depth)) = pending.pop() {
        let entries = match std::fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) if directory == path => {
                // A file or a vanished path contains nothing.
                if error.kind() == std::io::ErrorKind::NotFound
                    || error.kind() == std::io::ErrorKind::NotADirectory
                    || path.is_file()
                {
                    return Ok(found);
                }
                return Err(format!("Cannot inspect contents: {error}"));
            }
            Err(error) => {
                return Err(format!(
                    "Cannot inspect {}: {error}",
                    directory.display()
                ));
            }
        };
        for entry in entries {
            let entry = entry.map_err(|error| error.to_string())?;
            let child = entry.path();
            if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                continue;
            }
            visited += 1;
            if visited > CONTAINED_DIRECTORIES {
                return Err("Too many folders to list the caches inside; inspect a smaller selection".into());
            }
            let label = cleanable_root(&child, env)
                .map(|owner| owner.label().to_owned());
            if let Some(label) = label {
                if found.len() < CONTAINED_LISTED {
                    found.push(format!("{label} at {}", child.display()));
                }
                // Its own contents belong to it; no need to look inside.
                continue;
            }
            if depth + 1 < CONTAINED_DEPTH {
                pending.push((child, depth + 1));
            }
        }
    }
    found.sort();
    Ok(found)
}
