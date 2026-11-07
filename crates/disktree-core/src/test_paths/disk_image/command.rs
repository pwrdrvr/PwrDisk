//! Native oracle commands must never hold the image-test lock indefinitely.

use std::io::{Read as _, Write as _};
use std::os::unix::process::CommandExt as _;
use std::process::{Child, Command, Output, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use rustix::process::{Pid, Signal, WaitId, WaitIdOptions};

const POLL: Duration = Duration::from_millis(5);
// Reaping is bounded too: SIGKILL cannot promptly retire a kernel-blocked
// process. Its image must remain intact and further native work must stop.
const REAP_GRACE: Duration = Duration::from_millis(200);
// Keep verbose native diagnostics from growing without bound.
const OUTPUT_LIMIT: u64 = 1024 * 1024;

pub(super) struct Commands {
    timeout: Duration,
    failure: OnceLock<String>,
}

impl Commands {
    pub(super) const fn new(timeout: Duration) -> Self {
        Self {
            timeout,
            failure: OnceLock::new(),
        }
    }

    pub(super) fn check(&self) -> Result<(), String> {
        self.failure.get().map_or(Ok(()), |failure| {
            Err(format!("native APFS work stopped after {failure}"))
        })
    }

    pub(super) fn run(&self, command: &mut Command) -> Result<Output, String> {
        self.check()?;
        // Direct stderr bypasses libtest's per-test capture: cancellation
        // must leave the last native command visible in the CI log.
        let _ = writeln!(std::io::stderr(), "APFS oracle start: {command:?}");
        let started = Instant::now();
        let result = output_within(command, self.timeout);
        let _ = writeln!(
            std::io::stderr(),
            "APFS oracle end ({:?}): {command:?}",
            started.elapsed()
        );
        let output = result.map_err(|error| {
            let message = format!("{command:?}: {}", error.detail);
            if error.timed_out {
                let _ = self.failure.set(message.clone());
            }
            message
        })?;
        if output.status.success() {
            Ok(output)
        } else {
            Err(format!(
                "{command:?}: {}\n{}\n{}",
                output.status,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ))
        }
    }
}

#[derive(Debug)]
struct Failure {
    detail: String,
    timed_out: bool,
    reaped: bool,
}

fn failed(error: impl std::fmt::Display) -> Failure {
    Failure {
        detail: error.to_string(),
        timed_out: false,
        reaped: false,
    }
}

fn capture(file: &tempfile::NamedTempFile) -> Result<Vec<u8>, Failure> {
    let mut bytes = Vec::new();
    file.reopen()
        .map_err(failed)?
        .take(OUTPUT_LIMIT)
        .read_to_end(&mut bytes)
        .map_err(failed)?;
    Ok(bytes)
}

fn partial(file: &tempfile::NamedTempFile) -> String {
    capture(file).map_or_else(
        |error| error.detail,
        |bytes| String::from_utf8_lossy(&bytes).into_owned(),
    )
}

fn stop(child: &mut Child) -> bool {
    // The leader has not been reaped, so its group ID cannot be reused by
    // another process between observing the deadline and signalling it.
    let _ = rustix::process::kill_process_group(
        Pid::from_child(child),
        Signal::KILL,
    );
    let _ = child.kill();
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return true,
            Ok(None) if started.elapsed() < REAP_GRACE => {
                std::thread::sleep(POLL);
            }
            _ => return false,
        }
    }
}

fn output_within(
    command: &mut Command,
    timeout: Duration,
) -> Result<Output, Failure> {
    // A helper may inherit stdout after its launcher exits. Regular capture
    // files avoid both pipe-capacity deadlocks and waiting forever for EOF.
    let stdout = tempfile::NamedTempFile::new().map_err(failed)?;
    let stderr = tempfile::NamedTempFile::new().map_err(failed)?;
    let mut child = command
        .stdin(Stdio::null())
        .stdout(stdout.reopen().map_err(failed)?)
        .stderr(stderr.reopen().map_err(failed)?)
        .process_group(0)
        .spawn()
        .map_err(failed)?;
    let started = Instant::now();
    let status = loop {
        if started.elapsed() >= timeout {
            let reaped = stop(&mut child);
            return Err(Failure {
                detail: format!(
                    "timed out after {timeout:?}; pid {}; reaped: {reaped}\n{}\n{}",
                    child.id(),
                    partial(&stdout),
                    partial(&stderr),
                ),
                timed_out: true,
                reaped,
            });
        }
        let options = WaitIdOptions::EXITED
            | WaitIdOptions::NOHANG
            | WaitIdOptions::NOWAIT;
        match rustix::process::waitid(
            WaitId::Pid(Pid::from_child(&child)),
            options,
        ) {
            Ok(Some(_)) => {
                break child
                    .try_wait()
                    .map_err(failed)?
                    .ok_or_else(|| failed("exited child was not reapable"))?;
            }
            Ok(None) => std::thread::sleep(POLL),
            Err(error) => {
                stop(&mut child);
                return Err(failed(error));
            }
        }
    };
    Ok(Output {
        status,
        stdout: capture(&stdout)?,
        stderr: capture(&stderr)?,
    })
}

fn check_timeout(
    descendant: bool,
) -> Result<(), proptest::test_runner::TestCaseError> {
    let script = if descendant {
        "sleep 2 & wait"
    } else {
        "exec sleep 2"
    };
    let started = Instant::now();
    let result = output_within(
        Command::new("/bin/sh").args(["-c", script]),
        Duration::from_millis(40),
    );
    proptest::prop_assert!(
        started.elapsed() < Duration::from_secs(1),
        "the command deadline must bound waiting for native work"
    );
    let error = result.expect_err("sleep must exceed the deadline");
    proptest::prop_assert!(error.timed_out, "{}", error.detail);
    proptest::prop_assert!(error.reaped, "{}", error.detail);
    Ok(())
}

#[test]
fn a_hung_command_has_a_deadline() {
    check_timeout(false).expect("bounded native command");
}

#[test]
fn an_exited_launcher_does_not_wait_for_inherited_output() {
    let started = Instant::now();
    let output = output_within(
        Command::new("/bin/sh")
            .args(["-c", "sleep 2 & printf 'done'; printf 'diagnostic' >&2"]),
        Duration::from_millis(100),
    )
    .expect("the launcher exits immediately");
    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(output.status.success());
    assert_eq!(output.stdout, b"done");
    assert_eq!(output.stderr, b"diagnostic");
}

#[test]
fn a_timeout_stops_followup_commands() {
    let commands = Commands::new(Duration::from_millis(40));
    let first = commands
        .run(Command::new("sleep").arg("2"))
        .expect_err("native timeout");
    let temp = tempfile::tempdir().expect("spawn marker");
    let marker = temp.path().join("must-not-run");
    let second = commands
        .run(Command::new("touch").arg(&marker))
        .expect_err("no further native commands");
    assert!(second.contains(&first));
    assert!(!marker.exists());
}

proptest::proptest! {
    #![proptest_config(proptest::test_runner::Config {
        cases: 16,
        max_shrink_iters: 0,
        failure_persistence: None,
        ..proptest::test_runner::Config::default()
    })]

    #[test]
    fn command_deadlines_cover_descendants(descendant in proptest::bool::ANY) {
        check_timeout(descendant)?;
    }
}
