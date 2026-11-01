# Mac GUI checks in PwrSuiteLab

Anything that needs an on-screen macOS window — a PR screenshot, a
VoiceOver or native accessibility check, a look at a screen the
window-harness tests cannot judge — runs in the dedicated headed E2E Tart
VM of PwrSuiteLab, never on the operator's Mac. A window launched on the
operator's Mac takes focus, and synthetic keys or clicks land in whatever
app the operator is using; that has happened.
[AGENTS.md](../AGENTS.md#testing-on-a-desktop) has the rule.

This file covers macOS only. Linux builds, lint, tests and SSH integration
run in [Sandboxer](sandboxer.md), which has no desktop. A Linux or Windows
window that the lab cannot show is a question for the operator, never a
reason to use the local desktop.

## Through PwrSuiteLab Control MCP

The only route is the PwrSuiteLab Control MCP. If its tools are not
available to this thread, stop and ask the operator. There is no local
fallback and no script fallback.

The served skills are the procedure, and they win over this file when the
two disagree. Discover the live tools and their schemas first, then read
`skill://manage-pwrlab-e2e/SKILL.md` (`skills/list` → `skills/get` →
`resources/read`; older clients `resources/list` → `resources/read`).
`skill://inspect-pwrlab/SKILL.md` explains `lab_status`, and
`skill://operate-pwrlab/SKILL.md` covers lab operations. In PwrAgent, find
the tools with `search_mcp_tools` and call them with `call_mcp_tool`. Keep
the Control MCP connection open. A connection granted Operate may call its
exposed actions; one granted Read only may not, and a `permission_denied`
is reported, not worked around.

1. Call `lab_status` and take the exact target of the dedicated E2E role.
   Never pick a GitHub Actions runner or the physical Mac's desktop. Check
   status again just before submitting: the VM can stop, or another owner
   can take it, while the job is being prepared. `lab_e2e_run` starts the
   VM when needed; no `lab_e2e_acquire` is needed first.
2. Commit the change and use this thread's clean, committed worktree, by
   its absolute path. Never discard changes to make it clean. Only
   committed `HEAD` travels: no ignored files, host environment or
   credentials, and no push is needed.
3. Call `lab_e2e_run` with top-level `target`, `agent_name`,
   `project_name` (`PwrDisk`) and `thread_name` (the current thread's
   title), plus `pr_number` as a string of digits only when a PR is known.
   These are display labels: no paths or credentials in them.
4. The `job` holds `repository` (that absolute path), `setup` (an ordered
   array of argv arrays), `command` (one argv array), `artifacts` (at most
   16 checkout-relative paths; symlinks and `..` are refused) and
   `timeout_seconds` (checkout, setup and command together, at most
   14400). Optional `cwd` is relative to the checkout. These are argv
   arrays, not shell strings: a pipeline, a loop or a shell function needs
   `["bash", "-c", "..."]`.
5. Keep the request ID and the `run_id`. Poll `lab_request_status` until
   the launch request completes; `job_state: running` means the job
   started, not that it passed. Then call `lab_e2e_collect` with the
   target and `run_id`, polling its request if it is still executing. It
   downloads `e2e.log` and the artifacts to the returned
   `artifact_directory`, which is relative to the PwrSuiteLab checkout
   Control uses, not to this worktree; if you cannot resolve it, ask the
   operator rather than search for it. Report the commit, the exit code,
   and which artifacts are missing; tell a setup failure from a check that
   failed. Keep raw logs and images out of Git, and never paste `e2e.log`
   lines, guest paths or the local artifact directory into a PR, issue or
   commit message. Summarize the failure in your own words.

After the job, Control collects its artifacts and stops a VM it started for
you, so a stopped VM is normal: call `lab_e2e_collect` with the saved
`run_id` first, since collected results come back with the VM stopped. Only
if it reports the run was never collected (`e2e_vm_stopped`), start that
exact target with `lab_e2e_start` when capacity permits and collect again.
Never rerun a job only to retrieve its artifacts.

## PwrDisk in the guest

PwrDisk is a Rust workspace, and `rust-toolchain.toml` pins its toolchain,
which rustup provisions on the first `cargo` call. `cargo build --release`
(or `make build`) produces `target/release/pwrdisk`; `make bundle` produces
`target/bundle/PwrDisk.app` when a check needs the bundle. `pwrdisk <path>`
scans that path directly and skips the source chooser.

The guest's command environment loads an existing Cargo setup. If `cargo`
or rustup is missing, install it noninteractively in `setup`. That runs in
the guest only; never install tools on the host as a workaround.

Synthetic input, AX scripting, `osascript` and `screencapture` are fine
inside the guest, from the job's command; never through a viewer on the
host. That is what the guest is for. Capture only the PwrDisk window, and
write into a checkout-relative directory, because artifacts are paths
relative to the guest's checkout. Build fixtures on the `Fixtures` disk
image only, never in the checkout or the guest's home: their paths, volume
and free space describe the lab.

### Example job

A shape to adapt, not a tested recipe. Replace every `<...>`; drop
`pr_number` when no PR is known.

```json
{
  "target": "<exact E2E target from lab_status>",
  "agent_name": "<the calling agent, e.g. Claude Code>",
  "project_name": "PwrDisk",
  "thread_name": "<current thread title>",
  "pr_number": "<digits, only when known>",
  "job": {
    "repository": "<absolute path to this clean, committed worktree>",
    "setup": [
      ["bash", "-c", "command -v cargo >/dev/null || curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain none"],
      ["bash", "-c", "source \"$HOME/.cargo/env\" 2>/dev/null; cargo build --release --locked"]
    ],
    "command": ["bash", "-c", "<the script below, JSON-escaped>"],
    "artifacts": ["target/lab-shots"],
    "timeout_seconds": 3600
  }
}
```

The `command` script builds the contrived tree from
[pr-screenshots.md](pr-screenshots.md) on a disk image named `Fixtures`,
opens PwrDisk on it, waits for its window, captures only that window and
quits:

```sh
set -euo pipefail
shots=target/lab-shots   # relative to the checkout, as artifacts must be
mkdir -p "$shots"
pid=
cleanup() {
  # kill is asynchronous: wait, or the detach finds the volume busy.
  [ -n "$pid" ] && { kill "$pid"; wait "$pid"; } 2>/dev/null || true
  hdiutil detach -force /Volumes/Fixtures 2>/dev/null || true
}
trap cleanup EXIT

# A stale mount would put this image at "/Volumes/Fixtures 1".
[ ! -e /Volumes/Fixtures ] || { echo 'Fixtures already mounted' >&2; exit 1; }
hdiutil create -ov -size 512m -fs APFS -volname Fixtures \
  -attach target/fixtures.dmg
root=/Volumes/Fixtures/Projects
mkdir -p "$root"/{atlas/build,atlas/src,beacon/node_modules,photos/2024}
fill() { dd if=/dev/urandom of="$1" bs=1048576 count="$2" 2>/dev/null; }
fill "$root/atlas/build/app.bin" 120
fill "$root/beacon/node_modules/bundle.js" 40
fill "$root/photos/2024/IMG_0001.raw" 80

target/release/pwrdisk "$root" &
pid=$!

# The CGWindowID of PwrDisk's main window, for screencapture -l.
cat > target/window-id.swift <<'EOF'
import CoreGraphics
let pid = Int(CommandLine.arguments[1])!
let all = CGWindowListCopyWindowInfo(.optionOnScreenOnly, kCGNullWindowID)
for w in all as? [[String: Any]] ?? [] {
    if w[kCGWindowOwnerPID as String] as? Int == pid,
       w[kCGWindowLayer as String] as? Int == 0 {
        print(w[kCGWindowNumber as String]!)
        break
    }
}
EOF
wid=
for _ in $(seq 60); do
  wid=$(swift target/window-id.swift "$pid" 2>/dev/null || true)
  [ -n "$wid" ] && break
  sleep 1
done
[ -n "$wid" ] || { echo "no PwrDisk window" >&2; exit 1; }
sleep 5   # the scan finishes and the treemap draws

# <drive the screen under test: keys or AX actions, in the guest only>
screencapture -o -x -l "$wid" "$shots/<after-name>.png"
```

Check the images after collecting them. A capture that shows only the
desktop, or a shell that cannot send keys or read AX, means the guest is
missing a permission such as Screen Recording or Accessibility. Report that
to the operator as a baseline issue; do not change the VM's settings, its
login or its access to work around it.

For "before" images, run the same job against a clean, committed worktree
of `main` (`git fetch origin main && git worktree add --detach <path>
origin/main`), with the same tree, the same window size and `before`
names. Remove it with `git worktree remove <path>` afterwards.

## Interactive work

A check that needs a person at the guest, such as a VoiceOver walk-through,
is for the operator. To put PwrDisk there, run a job whose command builds
the fixture, launches `target/release/pwrdisk <root>` and keeps it open (for
example `sleep <seconds>` within `timeout_seconds`), then call
`lab_e2e_open` while it runs so the operator can use it. `lab_e2e_acquire`
(same top-level target and attribution, no `job`) and `lab_e2e_release`
reserve the display without a job; a reservation stages no source, so it
puts no PwrDisk build in the guest. Release only this connection's own
reservation, never after a job has consumed it.

The Screen Sharing viewer that `lab_e2e_open` shows runs on the operator's
desktop. It is for the operator to use. Never drive it with computer use,
`osascript`, `cliclick`, AX actions or window moves. Input that the agent
sends goes only through a job's `command`, which runs in the guest.

## Files, not the clipboard

Lab VMs start with clipboard sharing off: Tart's text-only sync echoes back
and strips images and rich text from the operator's clipboard. Inputs travel
in the committed source `lab_e2e_run` stages; outputs come back as
`lab_e2e_collect` artifacts. Never use the shared clipboard, and never ask
the operator to paste into or copy from the guest.

## Errors, ownership and recovery

`Invalid arguments` (`-32602`) is a schema mismatch, not a lost grant.
Refresh the schema, check the three attribution fields, their top-level
place, `pr_number` as a string and the argv shapes, and report it if a
corrected call still fails. After a launch or transport failure, inspect the
returned run before retrying: the job may already be running.

An `interrupted` result is not a pass. A held lock's age is an observation,
not an expiry. Preserve locks and use the guarded `lab_e2e_recover` for an
orphaned workload; never delete a lock or end another owner's session.
Control owns SSH, identity and host trust: no raw SSH, no guessed users or
keys, no new host keys. A missing baseline prerequisite is an
operator-authorized repair, not part of a job.

## Keep the lab out of this repository

PwrDisk is public. Host and guest names, addresses, targets, users, keys,
fingerprints, paths on the lab's or the operator's machines, and capacity
live only in PwrSuiteLab, and stay out of code, docs, commit messages, PR
bodies, issues and fixtures. Target values come from `lab_status` at run
time. What belongs here is that the lab exists, its name, and the public
Control tools and job contract: enough to use it, never enough to map it.
