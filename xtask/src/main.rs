//! Check and packaging entry points for PwrDisk.
//!
//! Each gate is reachable on its own (`cargo xtask fmt`) and through the
//! aggregate CI runs (`cargo xtask lint`), mirroring how omatrack exposes every
//! linter both as a build target and as a labelled test. `lint` never fixes
//! anything: it reports the diff and fails, so a red local run is the same
//! signal CI gives.

mod bundle;
mod portable;
mod release;

use std::process::{Command, ExitCode, Stdio};

const USAGE: &str = "\
usage: cargo xtask <task>

tasks:
  lint      fmt --check, then clippy over the workspace with -D warnings
  fmt       rustfmt --check over the workspace
  fmt-fix   rustfmt in place
  clippy    clippy --workspace --all-targets -- -D warnings
  test      cargo test --workspace, then (Unix) the release scripts' tests
  ci        lint, then test
  portable  Linux: desktop executable with static x86_64/ARM64 scanners
  bundle    macOS: target/bundle/PwrDisk.app, its zip and a DMG
              --sign IDENTITY   sign with a Developer ID (default: ad hoc)
              --notarize        notarize and staple; needs NOTARY_PROFILE, or
                                NOTARY_KEY, NOTARY_KEY_ID and NOTARY_ISSUER
              --universal       both architectures, joined with lipo
              --app-only        stop at the signed app: no zip, no DMG
  release-check --tag vX.Y.Z [--notes-file PATH]
            the tag, Cargo.toml version and CHANGELOG.md agree; optionally
            write that version's notes
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let task = args.first().map_or("lint", String::as_str);

    let outcome = match task {
        "lint" => fmt(false).and_then(|()| clippy()),
        "fmt" => fmt(false),
        "fmt-fix" => fmt(true),
        "clippy" => clippy(),
        "test" => test(),
        "ci" => fmt(false).and_then(|()| clippy()).and_then(|()| test()),
        "bundle" => bundle::bundle(&args[1..]),
        "release-check" => release::check(&args[1..]),
        "portable" => portable::build(),
        "help" | "-h" | "--help" => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        other => {
            eprintln!("unknown task: {other}\n\n{USAGE}");
            return ExitCode::FAILURE;
        }
    };

    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("xtask {task}: {message}");
            ExitCode::FAILURE
        }
    }
}

fn fmt(fix: bool) -> Result<(), String> {
    let mut args = vec!["fmt", "--all"];
    args.push(if fix { "" } else { "--check" });
    let args: Vec<&str> = args.into_iter().filter(|a| !a.is_empty()).collect();
    run(&args)
}

fn clippy() -> Result<(), String> {
    run(&[
        "clippy",
        "--workspace",
        "--all-targets",
        "--all-features",
        "--",
        "-D",
        "warnings",
    ])
}

fn test() -> Result<(), String> {
    run(&["test", "--workspace"])?;
    // The DMG layout and the signing keychain's cleanup, with `security`
    // and `xcrun` mocked: no credentials, so they run anywhere with a
    // Python 3. Windows has neither the tools nor the scripts' shell.
    if cfg!(unix) {
        let status = Command::new("python3")
            .args(["-B", "-m", "unittest", "discover", "-s"])
            .arg(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../scripts/release"),
            )
            .args(["-p", "*_test.py"])
            .stdin(Stdio::null())
            .status()
            .map_err(|err| format!("could not start python3: {err}"))?;
        if !status.success() {
            return Err("the release scripts' tests failed".into());
        }
    }
    Ok(())
}

fn run(args: &[&str]) -> Result<(), String> {
    let cargo = option_env!("CARGO").unwrap_or("cargo");
    let display = format!("{cargo} {}", args.join(" "));
    let status = Command::new(cargo)
        .args(args)
        .stdin(Stdio::null())
        .status()
        .map_err(|err| format!("could not start `{display}`: {err}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("`{display}` failed"))
    }
}
