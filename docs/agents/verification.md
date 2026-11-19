# Building and testing PwrDisk

## Commands

Before using a command that opens a window, read the
[desktop test restrictions](../../AGENTS.md#testing-on-a-desktop).

```sh
make build                      # release build
make run                        # opens a window: see Testing on a desktop
make install                    # ~/.local: binary, desktop entry, icon
make install PREFIX=/usr/local  # system-wide (needs root)
make uninstall
make bundle                     # macOS: target/bundle/PwrDisk.app, zip and DMG
make dock-icons                 # macOS 26: redraw the Dock icon of an unbundled run
make lint                       # AGENTS.md size, rustfmt --check, clippy -D warnings
make test                       # core and window-harness tests
make ci                         # lint, then test
make fmt                        # format in place
cargo xtask licenses            # rewrite THIRD_PARTY_LICENSES after a dependency change
```

`cargo xtask lint` is the gate. It must be green before anything is called
done, and it must not fix anything: a red local run is the same signal CI gives.
On Windows, where there is no make, run `cargo xtask lint`, `cargo xtask test`
and `cargo build --release` directly; CI runs the gate on both systems.

Run headless checks on the matching platform and in CI. For Linux builds
and tests, use the existing commands in `.github/workflows/ci.yml`. Run
SSH integration with `bun scripts/test-ssh.ts` and mount-namespace checks
with `scripts/test-linux-mounts.sh`. Linux-only checks require a Linux
host. If that platform is unavailable, report the unrun checks and rely
on the relevant CI results. macOS and Windows changes use the headless
gate (`cargo xtask lint`, `cargo xtask test`) on those systems and CI;
anything needing a window follows
[Testing on a desktop](../../AGENTS.md#testing-on-a-desktop).

The [Sandboxer verification process](../sandboxer.md) and its helper
scripts are retained as reference. That workflow is dormant while another
provider is evaluated; do not run its provider operations.

## Desktop checks

* **Offscreen macOS PR pictures** may run on the host with the opt-in
  `screenshots` feature and a contrived Fixtures image. This uses GPUI's
  headless Metal renderer and macOS text; no on-screen window or OS input.
  Read [docs/pr-screenshots.md](../pr-screenshots.md) for the commands.
* **An on-screen macOS window** (screenshots, VoiceOver, native AX checks) runs
  in the PwrSuiteLab headed E2E Tart VM through the PwrSuiteLab Control
  MCP. Read [docs/headed-e2e-pwrsuitelab.md](../headed-e2e-pwrsuitelab.md)
  before the first job. Without that MCP, stop and ask the operator: there
  is no local fallback and no script fallback.
* **Any other window**, such as a Linux or Windows GUI the lab cannot show:
  ask the operator. Never the local desktop.
* **Files, not the clipboard.** Lab VMs start with clipboard sharing off.
  Inputs travel in the committed source a job stages and outputs come back
  as collected artifacts; never ask the operator to paste into or copy
  from the guest.
* **Keep the lab out of this repository.** PwrDisk is public: no lab
  detail goes in code, docs, commits or PRs. The last section of
  [docs/headed-e2e-pwrsuitelab.md](../headed-e2e-pwrsuitelab.md) lists
  what counts. Targets come from `lab_status` at run time.
* **A launch or demo video** records its app footage in that VM: follow
  [.agents/skills/launch-reel/SKILL.md](../../.agents/skills/launch-reel/SKILL.md).

## Test coverage

* Size accounting, hardlinks, symlinks, hidden entries, depth limits, the
  removal guards and squarified layout are covered by `disktree-core` tests
  against real temporary trees.
* The screens are covered by window-harness tests that draw frames and press
  keys, including one that marks a directory, confirms the removal and checks
  the files are gone while unmarked neighbours are untouched, and one that
  drives the window over the real socket or pipe as `pwrdisk mcp` does.
  Initialize them through `tests::init`, which applies a fixed theme without
  native filesystem watching. Keep its control bindings aligned with
  gpui-omarchy when updating that dependency; its normal `init` starts a
  watcher that cannot wake GPUI's deterministic test executor safely.
* `disktree-mcp` tests every tool as a function against a temporary tree,
  with a stand-in window on the socket or pipe for the linked cases;
  `disktree-link` tests the transport itself.
* Rendering is verified by those tests and, where an on-screen macOS window
  is needed, in the PwrSuiteLab VM against a contrived tree (see
  [Testing on a desktop](../../AGENTS.md#testing-on-a-desktop)); it has not been eyeballed
  in every theme and font size.
