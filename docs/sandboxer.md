# Remote Linux verification with Sandboxer

Use this procedure when the thread has Sandboxer MCP tools. Run Linux
builds, lint, tests and integration checks in the remote VM. This keeps
the Rust compile load off the local machine. Use the local commands in
[AGENTS.md](../AGENTS.md#commands) when Sandboxer is not available.

This procedure follows PwrRoute's Sandboxer workflow. It uses committed
source, one VM per thread, a shared base snapshot and saved exit codes.
Use only the Sandboxer MCP tools. Do not use the `sbx` CLI.

Start new VMs from `pwrdisk-base-us-east` in `us-east`. The snapshot has
the tools, dependencies and Cargo build cache from a verified commit.
This avoids repeated installation and compilation for each thread. Keep
the checkout at `~/pwrdisk/repo` to reuse that cache. Always transfer and
check out the commit under test; the snapshot's commit can be older.

## Select a sandbox

1. Discover the current Sandboxer tool schemas. In PwrAgent, use
   `search_mcp_tools`, then `call_mcp_tool` to call a discovered tool.
2. Call `list_sandboxes`. Reuse this thread's VM. Use `resume_sandbox` if
   it is stopped. Do not use a VM that belongs to another thread.
3. If a new VM is necessary, call `list_snapshots` with
   `name: "pwrdisk-base-us-east"`. Then call `create_sandbox` with these
   values:

   | Field | Value |
   | --- | --- |
   | `name` | `pwrdisk-<branch>`, with `/` replaced by `-` |
   | `from` | `pwrdisk-base-us-east` |
   | `noEnv` | `true` |
   | `region` | `us-east` |
   | `type` | `large` |
   | `labels` | `{"repo": "pwrdisk"}` |
   | `ttlSeconds` | `7200` |

   `noEnv` excludes the account's environment variables and secret files.
   If the schema does not have this field, report the blocker. Do not
   create the VM. The TTL stops the VM after two hours, even during a run.
   Select a longer TTL at creation if the checks need more time.
   Always set `region`; the default can select another region.
   If the snapshot is missing, [build and test it](#build-or-refresh-the-base-snapshot).
   Do not silently create a cold VM for each thread.
4. Pass `sandboxId` on every later call. The implicit default can refer to
   another task's VM.

The setup script supports Ubuntu with `apt`, `sudo`, Python 3 and Git.
Check `uname -m` and `/etc/os-release` before setup. Record the actual OS
and architecture in the results. Do not assume that all hosts are equal.

## Send the source

Commit the changes first. Verify `HEAD`, including the scripts in this
guide. Do not transfer a working tree with uncommitted changes.

For a commit already pushed to `pwrdrvr/PwrDisk`, use the public HTTPS
remote. No GitHub token is required. In a VM started from the snapshot:

```sh
cd ~/pwrdisk/repo
git fetch origin <branch>
git checkout --detach <full-commit-sha>
```

Only a cold VM needs a clone:

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

1. The base snapshot has `~/pwrdisk/bin/transfer.py`. For a cold VM, read
   [transfer.py](../scripts/sandboxer/transfer.py) locally. Use the MCP
   `write_file` tool to put that content at
   `/home/user/pwrdisk/bin/transfer.py` in the VM.
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
5. In a cold VM, replace `<sha>` with that value:

   ```sh
   cd ~/pwrdisk
   git clone files/<sha>.bundle repo
   cd repo
   git remote set-url origin https://github.com/pwrdrvr/PwrDisk.git
   git checkout --detach <sha>
   rm ../files/<sha>.bundle
   ```

   In a VM started from the base snapshot, or for later runs:

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

Run `setup` once before individual checks. In a VM started from the base,
installed tools and downloaded dependencies are reused. Repeat setup when
the toolchain or system dependencies change. Examples:

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

## Build or refresh the base snapshot

Refresh `pwrdisk-base-us-east` when `Cargo.lock`, `rust-toolchain.toml`,
the setup script or CI's system libraries change. Also refresh it when
its commit is far behind `main`. New threads must still select their own
commit; a base snapshot is a cache, not proof that a change passes.

1. Use a builder VM owned by this thread. Create it in `us-east` with
   `type: "large"`, `noEnv: true`, and `name: "pwrdisk-base-builder"`.
   Start from the existing base when possible. Omit `from` only when no
   base exists. A stopped, verified VM from this thread can be the builder.
2. Transfer the commit to cache, normally the latest `main`. Use the PR's
   commit when the snapshot tools are first added. Check it out at
   `~/pwrdisk/repo`. Run `scripts/sandboxer/verify.sh all` with a detached
   `exec`. Wait for exit 0 and record every status line. Resolve failures
   before publishing a base.
3. Stop any transfer server with `kill_command`. Check that no build or
   test is running. Keep a copy of the transfer script outside the checkout:

   ```sh
   mkdir -p ~/pwrdisk/bin ~/pwrdisk/files
   cp ~/pwrdisk/repo/scripts/sandboxer/transfer.py ~/pwrdisk/bin/transfer.py
   ```

4. Remove verification logs, transferred files and temporary test artifacts
   from the builder. Keep `target/`, the Cargo registry and installed tools.
   For the standard paths:

   ```sh
   cd ~/pwrdisk/repo
   rm -rf .local/sandboxer
   find ~/pwrdisk/files -mindepth 1 -maxdepth 1 -exec rm -rf -- {} +
   git status --porcelain
   git rev-parse HEAD
   ```

   Inspect `/tmp` and remove any test fixtures or logs left by this thread.
   The Git status must be empty. Verify that the builder has no account
   tokens or private artifacts. `noEnv` does not remove files inherited
   from an earlier snapshot.
5. Call `save_snapshot` with `sandboxId`, `name: "pwrdisk-base-us-east"`
   and labels `repo: pwrdisk`, `role: base`, `region: us-east`,
   `commit: <full-sha>`. Wait for it to finish. Check `list_snapshots` for
   the name, ready state and labels. Saving an existing unversioned name
   replaces it; coordinate refreshes so builders do not overwrite each other.
6. Test a new VM from the saved snapshot. Pass `from: "pwrdisk-base-us-east"`,
   `region: "us-east"`, `type: "large"` and `noEnv: true` to
   `create_sandbox`. Check the inherited commit, clean Git status,
   `target/`, both musl scanner targets and the transfer script. Run
   `scripts/sandboxer/verify.sh all` there. Require exit 0 for every step.
   A second run in the builder does not test snapshot restoration.
7. Record the builder commit, test VM ID and step times in the PR or task
   results. Stop both VMs after the test. Delete a dedicated builder when
   it is no longer needed; the named snapshot survives its deletion.

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
- Use PwrDisk's base in `us-east`. Do not use a PwrRoute snapshot. Keep the
  named base when deleting a thread's VM; it serves later threads too.
