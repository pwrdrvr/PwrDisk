# Remote Linux verification with Sandboxer

Use this procedure when the thread has Sandboxer MCP tools. Run Linux
builds, lint, tests and integration checks in the remote VM. This keeps
the Rust compile load off the local machine. Use the local commands in
[AGENTS.md](../AGENTS.md#commands) when Sandboxer is not available.

This procedure follows PwrRoute's Sandboxer workflow. It uses committed
source, one VM per thread, a persistent Cargo cache and saved exit codes.
Use only the Sandboxer MCP tools. Do not use the `sbx` CLI.

## Select a sandbox

1. Discover the current Sandboxer tool schemas. In PwrAgent, use
   `search_mcp_tools`, then `call_mcp_tool` to call a discovered tool.
2. Call `list_sandboxes`. Reuse this thread's VM. Use `resume_sandbox` if
   it is stopped. Do not use a VM that belongs to another thread.
3. If necessary, call `create_sandbox` with these values:

   | Field | Value |
   | --- | --- |
   | `name` | `pwrdisk-<branch>`, with `/` replaced by `-` |
   | `noEnv` | `true` |
   | `region` | `us-east` |
   | `type` | `large` |
   | `labels` | `{"repo": "pwrdisk"}` |
   | `ttlSeconds` | `7200` |

   `noEnv` excludes the account's environment variables and secret files.
   If the schema does not have this field, report the blocker. Do not
   create the VM. The TTL stops the VM after two hours, even during a run.
   Select a longer TTL at creation if the checks need more time.
4. Pass `sandboxId` on every later call. The implicit default can refer to
   another task's VM.

The setup script supports Ubuntu with `apt`, `sudo`, Python 3 and Git.
Check `uname -m` and `/etc/os-release` before setup. Record the actual OS
and architecture in the results. Do not assume that all hosts are equal.

## Send the source

Commit the changes first. Verify `HEAD`, including the scripts in this
guide. Do not transfer a working tree with uncommitted changes.

For a commit already pushed to `pwrdrvr/PwrDisk`, use the public HTTPS
remote. No GitHub token is required. In a new VM:

```sh
mkdir -p ~/pwrdisk
git clone https://github.com/pwrdrvr/PwrDisk.git ~/pwrdisk/repo
cd ~/pwrdisk/repo
git fetch origin <branch>
git checkout --detach <full-commit-sha>
```

For an unpublished commit, use the bundle transfer below. Both methods
use `~/pwrdisk/repo`. Keep that path for later runs to reuse `target/`.
Never change the checkout while a check is running.

### Transfer an unpublished commit

The two transfer scripts come from PwrRoute, with PwrDisk paths and a
check that rejects uncommitted changes. They also retrieve logs and
artifacts without passing file contents through the agent's context.

1. Read [transfer.py](../scripts/sandboxer/transfer.py) locally. Use the
   MCP `write_file` tool to put that content at
   `/home/user/pwrdisk/bin/transfer.py` in the VM. This bootstraps a VM
   that does not yet have the repository.
2. Use `exec` with `detached: true` to start the server:

   ```sh
   python3 ~/pwrdisk/bin/transfer.py
   ```

3. Call `host_port` with `port: 8722` and `public: false`. Keep the
   returned URL in a local shell variable named `URL`. It contains an
   access token. Do not put it in a commit, PR, status report or log.
4. On the local machine, run:

   ```sh
   scripts/sandboxer/transfer.sh send "$URL"
   ```

   The script uploads a Git bundle and prints the full commit SHA.
5. In a new VM, replace `<sha>` with that value:

   ```sh
   cd ~/pwrdisk
   git clone files/<sha>.bundle repo
   cd repo
   git remote set-url origin https://github.com/pwrdrvr/PwrDisk.git
   git checkout --detach <sha>
   rm ../files/<sha>.bundle
   ```

   For later runs in the same VM:

   ```sh
   cd ~/pwrdisk/repo
   git fetch ../files/<sha>.bundle HEAD
   git checkout --detach <sha>
   rm ../files/<sha>.bundle
   ```

After a resume, check the transfer server before use. Start it again if
`curl -s -o /dev/null -w '%{http_code}' localhost:8722/probe` does not
return `404`.

## Run the checks

[verify.sh](../scripts/sandboxer/verify.sh) runs in the VM. Use `exec`
with `cwd: "pwrdisk/repo"`, `detached: true` and this command:

```sh
scripts/sandboxer/verify.sh all
```

Save the returned `processId`. Use `command_status` with that ID and the
same `sandboxId` until the process exits. Pass the returned output offsets
to later calls. Long builds can exceed a synchronous `exec` timeout. Do
not start a second run because the first call has returned.

| Step | Command run by `verify.sh` |
| --- | --- |
| `setup` | Install CI's GPUI libraries, build tools, OpenSSH, Bun, the pinned Rust toolchain, and both musl scanner targets; fetch locked dependencies. |
| `lint` | `cargo xtask lint` (rustfmt check and workspace clippy). |
| `test` | `cargo xtask test` (workspace tests and release-script tests). |
| `build` | `cargo xtask portable` (release app and embedded x64/ARM64 SSH helpers). |
| `e2e` | `bun scripts/test-ssh.ts` (isolated localhost SSH scans and error cases). |
| `all` | Setup, lint, test, build and E2E in that order. |

`all` stops if setup fails. Otherwise, it runs every check even if one
fails. Its final exit code is nonzero if any check fails. The script holds
a lock to prevent concurrent runs in the same checkout.

Run `setup` once before individual checks. Repeat it when the toolchain
or system dependencies change. Examples:

```sh
scripts/sandboxer/verify.sh setup
scripts/sandboxer/verify.sh lint
scripts/sandboxer/verify.sh test -p disktree-core
scripts/sandboxer/verify.sh test -p disktree-app
scripts/sandboxer/verify.sh build
scripts/sandboxer/verify.sh e2e
```

Arguments after `test` select `cargo test --locked` instead of the full
xtask. Such a run does not include the Python release-script tests.

The window-harness tests are in `disktree-app`. They draw frames, send
keys, confirm removal of temporary fixtures and drive the real link
socket. They run in `test`; they do not need Xvfb. The `e2e` step uses
temporary SSH keys and a localhost server. It does not use real SSH hosts.
It builds the portable app again, which is incremental after `build`.
PwrDisk has no separate desktop E2E runner. Do not substitute PwrRoute's
`cargo xtask e2e` command.

## Read and report the results

Each invocation replaces `.local/sandboxer/status`. Its first line is the
commit SHA. Each step adds `<step> exit=<code> secs=<seconds>` and writes
`.local/sandboxer/<step>.log`. An `all` run ends with `all exit=<0|1>`.
The directory is ignored by Git.

Read bounded sections of the logs:

```sh
cat .local/sandboxer/status
tail -n 40 .local/sandboxer/lint.log
grep -nE '^test result|FAILED|panicked' .local/sandboxer/test.log | tail -n 20
tail -n 40 .local/sandboxer/e2e.log
```

Report the sandbox ID, OS, architecture, commit, selected checks and exit
codes. Read a failed step's log and state the cause. A missing final
status line is an incomplete run, not a pass. The lint gate must pass.

To retrieve a file, copy it to `~/pwrdisk/files/` in the VM. Then use the
authenticated transfer URL on the local machine:

```sh
scripts/sandboxer/transfer.sh get "$URL" test.log /tmp/pwrdisk-test.log
```

The transfer server accepts single file names. Archive a directory before
download. Use contrived trees for screenshots. Follow the separate
[PR attachment procedure](pr-screenshots.md) before uploading media.

## Scope and lifecycle

- Linux checks do not verify macOS, Windows, another CPU architecture or
  the oldest supported glibc. Keep the relevant platform CI checks.
- `test` includes the default tests. It does not run ignored tests. The
  `e2e` step selects only the ignored real SSH tests. Run mount-namespace
  tests with [test-linux-mounts.sh](../scripts/test-linux-mounts.sh) only
  on a host that permits unprivileged user and mount namespaces. Report
  missing host permissions; do not count a skipped check as a pass.
- Setup uses the commit's `rust-toolchain.toml` and CI's library list. It
  installs Bun only if Bun is absent. Record tool versions from the setup
  log. Keep the package list aligned with `.github/workflows/ci.yml`.
- The wrapper disables dev/test debug info to reduce cache size. Its
  cached artifacts can differ from a local build. It does not change
  release profiles.
- Call `stop_sandbox` after each run. This stops compute billing and keeps
  the disk for this thread's next run. Resume it instead of creating a VM.
- Attach the PR to the thread and call `watch_thread_pull_request` with
  `notifyOn: ["merged", "closed"]`. PwrAgent releases VMs created by the
  thread when its attached PRs merge or close. Check that the PR is
  attached; report a cleanup failure. Without that integration, delete
  this thread's VM with `delete_sandbox` when it is no longer needed.
- A base snapshot is optional. Use `list_snapshots` to find a suitable
  PwrDisk snapshot before passing `from` to `create_sandbox`. Do not assume
  one exists or use a PwrRoute snapshot. Before `save_snapshot`, remove
  transferred bundles, logs and private artifacts. Record the commit in
  the snapshot labels. Do not replace a shared snapshot without a request.
