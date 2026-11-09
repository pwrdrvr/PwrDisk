//! The installed CLI's dry-run reports its registry without loading test
//! configuration or installing anything. Use a single non-browser target
//! so exactly one location is printed on both supported Unix platforms.

use std::path::Path;

use super::{Command, Environment, Issue, Probe, Problem, strings};

pub(super) fn cache_root(
    command: &mut Command,
    env: &Environment,
    probe: &mut impl Probe,
) -> Result<String, Issue> {
    let config = |detail| Issue::new(Problem::ConfigCheckFailed, detail);
    command.arguments = strings(&["install", "--dry-run", "ffmpeg"]);
    let output = probe.output(command, env).map_err(config)?;
    let root = printed_root(&output).map_err(|detail| config(detail.into()))?;
    let root = root.to_str().ok_or_else(|| {
        config("Playwright returned a non-UTF-8 cache root".into())
    })?;
    command
        .environment
        .insert("PLAYWRIGHT_BROWSERS_PATH".into(), root.into());
    // Comparing the entire round-trip output, not just the parsed line,
    // also rejects roots containing LF that could forge a location line.
    let pinned = probe.output(command, env).map_err(config)?;
    if pinned != output {
        return Err(config(
            "Playwright's pinned dry-run did not reproduce its configured cache exactly".into(),
        ));
    }
    match std::fs::symlink_metadata(Path::new(root).join("__dirlock")) {
        Ok(_) => {
            return Err(Issue::new(
                Problem::InUse,
                "Playwright's __dirlock exists; stop installers and inspect the lock before preparing a new preview. PwrDisk never removes it",
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(config(error.to_string())),
    }
    Ok(root.into())
}

fn printed_root(output: &str) -> Result<&Path, &'static str> {
    let bad = "Playwright returned an ambiguous install location";
    if output.contains('\r') {
        return Err(bad);
    }
    let mut locations = output
        .split('\n')
        .filter_map(|line| line.strip_prefix("  Install location:    "));
    let location = locations.next().ok_or(bad)?;
    if locations.next().is_some() {
        return Err(bad);
    }
    let path = Path::new(location);
    let revision = path
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_prefix("ffmpeg-"))
        .ok_or(bad)?;
    if revision.is_empty() || !revision.bytes().all(|b| b.is_ascii_digit()) {
        return Err(bad);
    }
    let root = path.parent().ok_or(bad)?;
    if !root.is_absolute()
        || root.components().any(|component| {
            matches!(component, std::path::Component::ParentDir)
        })
    {
        return Err(bad);
    }
    Ok(root)
}
