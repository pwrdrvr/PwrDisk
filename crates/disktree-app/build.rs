//! Captures where this binary was built from, for Settings › About and
//! Diagnostics (`src/build_info.rs`): the checkout, branch, commit
//! and whether it had uncommitted changes. Read from `git` here, once per
//! build; the app never looks at a checkout while it runs.
//!
//! Every value is an environment variable for `env!`, empty when unknown
//! (no `git`, or a source tree that is not a checkout). On Windows it also
//! embeds the app icon.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
    build_identity();
    #[cfg(windows)]
    windows_icon();
}

/// The checkout, branch, commit and state, for `src/build_info.rs`.
fn build_identity() {
    let manifest = PathBuf::from(
        std::env::var_os("CARGO_MANIFEST_DIR").unwrap_or_default(),
    );
    let on_ci = std::env::var_os("GITHUB_ACTIONS").is_some();

    // One call for HEAD, the checkout and the git dirs `watch` needs.
    let parsed = git(
        &manifest,
        &[
            "rev-parse",
            "HEAD",
            "--show-toplevel",
            "--git-dir",
            "--git-common-dir",
        ],
    )
    .map(|out| out.lines().map(str::to_owned).collect::<Vec<_>>())
    .unwrap_or_default();
    let head = parsed.first().cloned();
    let toplevel = parsed.get(1).cloned();
    // A pull request's CI checkout is GitHub's merge commit, on no branch:
    // the workflow names the PR's head instead (preview-build.yml).
    let commit = std::env::var("PWRDISK_SOURCE_SHA")
        .ok()
        .map(|sha| sha.trim().to_owned())
        .filter(|sha| !sha.is_empty())
        .or(head);
    // The subject and time of that commit; a shallow checkout may not
    // have the PR's head, and then neither is known.
    let (subject, commit_time) = commit
        .as_deref()
        .and_then(|commit| {
            git(&manifest, &["log", "-1", "--format=%ct%n%s", commit])
        })
        .and_then(|out| {
            let (time, subject) = out.split_once('\n')?;
            Some((Some(subject.to_owned()), Some(time.to_owned())))
        })
        .unwrap_or((None, None));
    // `--no-optional-locks`: a build never rewrites the index.
    let dirty = git(
        &manifest,
        &[
            "--no-optional-locks",
            "status",
            "--porcelain",
            "--untracked-files=normal",
        ],
    )
    .is_some_and(|status| !status.is_empty());
    let branch =
        git(&manifest, &["symbolic-ref", "--quiet", "--short", "HEAD"])
            .or_else(|| ci_branch(on_ci));
    // HEAD may carry more than one tag; this version's is the one that
    // makes it a release.
    let release_tag = format!(
        "v{}",
        std::env::var("CARGO_PKG_VERSION").unwrap_or_default()
    );
    let tags =
        git(&manifest, &["tag", "--points-at", "HEAD"]).unwrap_or_default();
    let tag = tags
        .lines()
        .find(|tag| *tag == release_tag)
        .or_else(|| tags.lines().next())
        .map(str::to_owned)
        .or_else(|| ci_tag(on_ci));
    // A runner's path says nothing about the build, so CI leaves it out.
    let checkout = if on_ci { None } else { toplevel };
    // The time only means something when the tree had changes: otherwise
    // the commit says it all, and a time would set two identical builds
    // apart. SOURCE_DATE_EPOCH wins, for reproducible builds.
    let built_at = dirty.then(|| {
        std::env::var("SOURCE_DATE_EPOCH").ok().unwrap_or_else(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|elapsed| elapsed.as_secs().to_string())
                .unwrap_or_default()
        })
    });

    emit("PWRDISK_BUILD_COMMIT", commit.as_deref());
    emit("PWRDISK_BUILD_SUBJECT", subject.as_deref());
    emit("PWRDISK_BUILD_COMMIT_TIME", commit_time.as_deref());
    emit("PWRDISK_BUILD_BRANCH", branch.as_deref());
    emit("PWRDISK_BUILD_TAG", tag.as_deref());
    emit("PWRDISK_BUILD_DIRTY", dirty.then_some("1"));
    emit("PWRDISK_BUILD_CHECKOUT", checkout.as_deref());
    emit("PWRDISK_BUILD_TIME", built_at.as_deref());

    watch(&manifest, parsed.get(2), parsed.get(3));
}

fn emit(name: &str, value: Option<&str>) {
    // One line per value: a newline would end the directive.
    let value = value.unwrap_or_default().replace(['\r', '\n'], " ");
    println!("cargo:rustc-env={name}={value}");
}

/// Trimmed stdout of `git -C dir args`, or `None` when git fails or says
/// nothing.
fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

/// A pull request's checkout is a detached merge commit; the branch it
/// builds is the PR's head.
fn ci_branch(on_ci: bool) -> Option<String> {
    if !on_ci {
        return None;
    }
    let head = std::env::var("GITHUB_HEAD_REF")
        .ok()
        .filter(|v| !v.is_empty());
    head.or_else(|| {
        (std::env::var("GITHUB_REF_TYPE").ok().as_deref() == Some("branch"))
            .then(|| std::env::var("GITHUB_REF_NAME").ok())
            .flatten()
            .filter(|v| !v.is_empty())
    })
}

/// A tag push's shallow checkout may not carry the tag object.
fn ci_tag(on_ci: bool) -> Option<String> {
    if !on_ci || std::env::var("GITHUB_REF_TYPE").ok().as_deref() != Some("tag")
    {
        return None;
    }
    std::env::var("GITHUB_REF_NAME")
        .ok()
        .filter(|v| !v.is_empty())
}

/// Run again when HEAD moves (a checkout, a commit on a detached HEAD), a
/// branch or tag changes (a commit) or anything under `crates/` does (an
/// edit that makes or clears uncommitted changes). Not on the index: tools
/// that only refresh it would rebuild the app for nothing.
fn watch(
    manifest: &Path,
    git_dir: Option<&String>,
    common_dir: Option<&String>,
) {
    for var in [
        "GITHUB_ACTIONS",
        "GITHUB_HEAD_REF",
        "GITHUB_REF_NAME",
        "GITHUB_REF_TYPE",
        "PWRDISK_SOURCE_SHA",
        "SOURCE_DATE_EPOCH",
    ] {
        println!("cargo:rerun-if-env-changed={var}");
    }
    println!("cargo:rerun-if-changed=build.rs");
    if let Some(crates) = manifest.parent() {
        println!("cargo:rerun-if-changed={}", crates.display());
        if let Some(root) = crates.parent() {
            for file in ["Cargo.toml", "Cargo.lock"] {
                println!(
                    "cargo:rerun-if-changed={}",
                    root.join(file).display()
                );
            }
        }
    }
    let absolute = |path: &String| {
        let path = PathBuf::from(path);
        if path.is_absolute() {
            path
        } else {
            manifest.join(path)
        }
    };
    if let Some(git_dir) = git_dir.map(absolute) {
        println!("cargo:rerun-if-changed={}", git_dir.join("HEAD").display());
    }
    if let Some(common) = common_dir.map(absolute) {
        for path in ["refs/heads", "refs/tags", "packed-refs"] {
            println!("cargo:rerun-if-changed={}", common.join(path).display());
        }
    }
}

/// The icon Explorer, the taskbar and Alt-Tab show, as a resource linked
/// into `pwrdisk.exe`.
#[cfg(windows)]
fn windows_icon() {
    use std::{env, fs, path::PathBuf, process::Command};

    let icon = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap())
        .join("../../assets/disktree.ico")
        .canonicalize()
        .expect("Windows icon asset is missing");
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let rc = out.join("disktree.rc");
    let res = out.join("disktree.res");

    // rc.exe treats backslashes inside quoted paths as separators, not escapes.
    let icon_path = icon.to_string_lossy().replace('\\', "/");
    fs::write(&rc, format!("1 ICON \"{icon_path}\"\n"))
        .expect("could not write Windows icon resource script");

    // Cargo can run outside a Visual Studio prompt, where the SDK bin is not
    // on PATH even though the MSVC linker and Windows SDK are installed.
    let rc_exe = env::var_os("DISKTREE_RC").map_or_else(
        || {
            let sdk_bin = env::var_os("WindowsSdkDir")
                .map(PathBuf::from)
                .or_else(|| {
                    env::var_os("ProgramFiles(x86)")
                        .map(|root| PathBuf::from(root).join("Windows Kits/10"))
                })
                .expect("Windows SDK is required to embed the icon")
                .join("bin");
            let host = match env::consts::ARCH {
                "x86_64" => "x64",
                "aarch64" => "arm64",
                "x86" => "x86",
                other => panic!("unsupported Windows build host: {other}"),
            };
            let mut versions: Vec<_> = fs::read_dir(&sdk_bin)
                .expect("Windows SDK bin directory is missing")
                .filter_map(Result::ok)
                .map(|entry| entry.path().join(host).join("rc.exe"))
                .filter(|path| path.is_file())
                .collect();
            versions.sort();
            versions
                .pop()
                .expect("rc.exe is missing from the Windows SDK")
        },
        PathBuf::from,
    );
    let status = Command::new(rc_exe)
        .arg("/nologo")
        .arg(format!("/fo{}", res.display()))
        .arg(&rc)
        .status()
        .expect("could not start rc.exe to embed the Windows icon");
    assert!(
        status.success(),
        "rc.exe failed to compile the Windows icon"
    );

    println!("cargo:rerun-if-changed={}", icon.display());
    println!("cargo:rustc-link-arg-bin=pwrdisk={}", res.display());
}
