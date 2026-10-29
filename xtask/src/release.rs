//! `cargo xtask release-check`: the tag, the workspace version and
//! `CHANGELOG.md` agree before anything is built or signed.
//!
//! The changelog section is the release's notes: release.yml writes it with
//! `--notes-file` for the GitHub release and for `vpk pack --releaseNotes`,
//! which Velopack shows beside an update.

use std::fs;
use std::path::{Path, PathBuf};

pub fn check(args: &[String]) -> Result<(), String> {
    let mut tag = std::env::var("RELEASE_TAG").ok();
    let mut notes_file = None;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--tag" => tag = args.next().cloned(),
            "--notes-file" => notes_file = args.next().map(PathBuf::from),
            other => return Err(format!("unknown argument {other}")),
        }
    }
    let tag = tag
        .filter(|tag| !tag.is_empty())
        .ok_or("needs --tag vX.Y.Z (or RELEASE_TAG)")?;
    let root = root();
    let version = workspace_version(&read(&root.join("Cargo.toml"))?)
        .ok_or("no version in Cargo.toml's [workspace.package]")?;
    if tag != format!("v{version}") {
        return Err(format!(
            "tag {tag} does not match the Cargo.toml workspace version \
             {version}"
        ));
    }
    let changelog = read(&root.join("CHANGELOG.md"))?;
    let notes = changelog_section(&changelog, &version).ok_or_else(|| {
        format!("CHANGELOG.md has no `## v{version}` section")
    })?;
    if notes.trim().is_empty() {
        return Err(format!("CHANGELOG.md's `## v{version}` section is empty"));
    }
    if let Some(path) = notes_file {
        fs::write(&path, notes)
            .map_err(|err| format!("cannot write {}: {err}", path.display()))?;
    }
    println!("release metadata agrees for {tag}");
    Ok(())
}

/// The lines under `## vX.Y.Z` (or `## X.Y.Z`), up to the next `## `.
fn changelog_section(changelog: &str, version: &str) -> Option<String> {
    let mut lines = changelog.lines();
    lines.find(|line| {
        line.strip_prefix("## ")
            .map(|heading| heading.trim_start_matches('v'))
            .and_then(|heading| heading.split_whitespace().next())
            == Some(version)
    })?;
    let body: Vec<&str> =
        lines.take_while(|line| !line.starts_with("## ")).collect();
    Some(body.join("\n").trim().to_owned() + "\n")
}

/// `version` under `[workspace.package]`: the one every crate inherits.
fn workspace_version(manifest: &str) -> Option<String> {
    let mut in_package = false;
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_package = line == "[workspace.package]";
        } else if in_package
            && let Some(version) = line.strip_prefix("version = ")
        {
            return Some(version.trim_matches('"').to_owned());
        }
    }
    None
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}

fn read(path: &Path) -> Result<String, String> {
    fs::read_to_string(path)
        .map_err(|err| format!("cannot read {}: {err}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHANGELOG: &str = "# Changelog\n\n## Unreleased\n\n## v0.2.0\n\n\
         - Second.\n\n## v0.1.0 - 2026-10-13\n\n- First.\n";

    #[test]
    fn finds_a_changelog_section() {
        assert_eq!(
            changelog_section(CHANGELOG, "0.1.0").as_deref(),
            Some("- First.\n")
        );
        assert_eq!(
            changelog_section(CHANGELOG, "0.2.0").as_deref(),
            Some("- Second.\n")
        );
        assert_eq!(changelog_section(CHANGELOG, "0.3.0"), None);
    }

    #[test]
    fn a_prefix_is_not_a_match() {
        assert_eq!(changelog_section(CHANGELOG, "0.1"), None);
    }

    #[test]
    fn reads_only_the_workspace_version() {
        let manifest = "[package]\nversion = \"9.9.9\"\n\n\
             [workspace.package]\nversion = \"0.12.0\"\n";
        assert_eq!(workspace_version(manifest).as_deref(), Some("0.12.0"));
        assert_eq!(workspace_version("[package]\nversion = \"1\"\n"), None);
    }

    /// The version this checkout would release has notes to release with.
    #[test]
    fn this_version_has_a_changelog_section() {
        let root = root();
        let version =
            workspace_version(&read(&root.join("Cargo.toml")).unwrap())
                .unwrap();
        let changelog = read(&root.join("CHANGELOG.md")).unwrap();
        assert!(
            changelog_section(&changelog, &version)
                .is_some_and(|notes| !notes.trim().is_empty()),
            "CHANGELOG.md needs a `## v{version}` section"
        );
    }
}
