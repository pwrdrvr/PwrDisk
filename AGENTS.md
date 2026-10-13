# PwrDisk — agent guide

PwrDisk is a maintained fork of [disktree](https://github.com/tobi/disktree).
The user-facing name is PwrDisk (binary `pwrdisk`, bundle ID
`com.pwrdrvr.pwrdisk`); crate, module and type names and the SSH helper
(`disktree-scan`) keep the upstream names so upstream merges stay clean.

A GPUI + gpui-omarchy treemap explorer for disk usage on Omarchy. Read
`README.md` for the product; this file is the working contract.

Opening a PR? Read [Pull requests](#pull-requests) first: screenshots are
attached with `gh --attach`, never hosted on a branch.

## What this is

Find what is eating a volume, mark paths for removal, review the list, and
remove it — with the volume's free space live on screen the whole time. Two
phases after choosing a local/SSH source and root: explore (treemap,
breadcrumbs, selection and status) and review (the marked list,
the removal mode, the confirmation). Marking is never destructive.

`disktree mcp` is the same thing for an AI agent: an MCP server over stdio
that scans with `disktree-core`, marks, and opens the review screen in a
window over a per-user socket (`disktree-link`). The agent marks; the
person removes. There is no tool that deletes, and the link has no request
that deletes.

## Commands

```sh
make build                      # release build
make run                        # build and open the source/root chooser
make install                    # ~/.local: binary, desktop entry, icon
make install PREFIX=/usr/local  # system-wide (needs root)
make uninstall
make bundle                     # macOS: target/bundle/PwrDisk.app, zip and DMG
make dock-icons                 # macOS 26: redraw the Dock icon of an unbundled run
make lint                       # rustfmt --check, then clippy --all-targets -D warnings
make test                       # core and window-harness tests
make ci                         # lint, then test
make fmt                        # format in place
```

`make install` is the supported way to put this on a machine: it installs the
release binary, `packaging/pwrdisk.desktop.in` (rendered with the real install
prefix and the crate version) and `assets/disktree.svg`. Keep the desktop
entry's `Categories` to a single main category plus additional ones, or
`desktop-file-validate` complains.

On macOS `make install` installs `target/bundle/PwrDisk.app` into
`~/Applications` instead, built by `cargo xtask bundle` from
`packaging/macos/` (the `Info.plist` template and the icon's SVG). The bundle
identifier there keys every macOS permission grant; do not change it lightly.

The macOS icon has two sources in `packaging/macos/`, and each macOS reads
the one made for it. `AppIcon.svg` is the mark on Apple's legacy 824-in-1024
tile; the bundle draws `AppIcon.icns` from it, which macOS 15 and earlier
draw literally. `AppIcon.icon` with `AppGlyph.svg` is the Icon Composer
package: when Xcode 26's actool is at hand `cargo xtask bundle` compiles it
into `Assets.car` and names it with `CFBundleIconName`, and macOS 26 draws
that with its own shape, glass edge and Dark, Clear and Tinted looks. Handed
only the `.icns`, macOS 26 normalizes it instead, and 26.6.2 has drawn such a
padded icns on a light plate, so a bundle without the catalog says so. A run
outside the bundle (`make run`, `cargo run`) has no icon for macOS to draw,
so `dock_icon.rs` paints one: `DockIcon.png`, or on macOS 26
`DockIcon-glass.png`, the shipped icon as that macOS itself draws it, since
Liquid Glass is never applied to an image set at run time. `make dock-icons`
redraws both on a macOS 26 with Xcode 26; run it after either SVG changes.

Releases are cut by `release.yml` from a `vX.Y.Z` tag that matches the
workspace version and a `## vX.Y.Z` section of `CHANGELOG.md` (`cargo xtask
release-check`); `docs/releasing.md` is the procedure and
`.github/workflows/README.md` the design. macOS ships a DMG and Windows a
Velopack Setup.exe, MSI and portable zip, both of which update themselves
from GitHub Releases. The `velopack` crate pin and `VPK_VERSION` in
`scripts/release/install-vpk.sh` move together.

`cargo xtask lint` is the gate. It must be green before anything is called
done, and it must not fix anything: a red local run is the same signal CI gives.
On Windows, where there is no make, run `cargo xtask lint`, `cargo xtask test`
and `cargo build --release` directly; CI runs the gate on both systems.

## House rules

* **Strict lints from omatrack's style.** `clippy::all` and `clippy::pedantic`
  are errors, `clippy::nursery` warns, and `-D warnings` promotes the rest. Every
  deliberate exception is listed with its reason in the workspace `Cargo.toml`
  or as an `#[allow(..., reason = "...")]` on the item — never silently.
* **80 columns**, 4-space indent, by `rustfmt.toml`. Only stable rustfmt
  options, because the gate runs on stable; unstable options would be ignored
  with a warning instead of applying.
* **Comments say why.** The code says what. Any non-obvious number, ordering or
  boundary deserves the reason next to it.
* **Tests live beside the promise they make.** `disktree-core` tests size
  accounting, layout and deletion guards against real temporary trees;
  `disktree-app/src/tests.rs` drives the real window harness — draw a frame,
  press keys — so a screen that panics while painting fails a test.
* **Never delete anything outside a marked path.** See `removal.rs`; the guards
  are load-bearing and are covered by tests.

## Invariants

1. **Sizes come from `st_blocks * 512` unless apparent size was asked for.**
   APFS clones can share those blocks; private-byte estimates are reported
   separately without changing tile weights. On macOS the directory listing
   reports every fork's allocation and the data fork's length in bulk; see
   `macos.rs`. On Windows it is the allocation the directory listing reports;
   see `windows.rs`. An
   elevated scan of a whole NTFS drive reads it from the master file table
   instead, keeping the walk's rules for hidden entries, links, cloud
   folders and depth; see `mft.rs`. That path counts every stream's
   allocation, alternate data streams included since they go when the file
   goes, so for a file with alternate streams it can exceed the walk's
   number.
2. **Totals are derived, never tracked.** A leaf's `bytes` is the only
   input: `tree::aggregate` computes every directory's `bytes`, `files` and
   `dirs` from its children, and `own_bytes()`/`own_files()` read the direct
   children when asked. Hardlink de-duplication zeroes a duplicate leaf's
   `bytes` while that pass runs; anything that patches a directory's `bytes`
   directly will be overwritten. `Node` is paid for tens of millions of
   times, so `a_node_stays_small` holds its size.
3. **A directory is only built when its own scan *and* every subdirectory task
   has finished.** That is the `+1` sentinel in `PendingDir::pending`. Building
   early silently drops whole subtrees — it has happened once.
4. **Only paths under the scanned root may be removed**, and mount points, the
   root, the home directory, any directory holding it, and symlink targets are
   refused. On Windows so is every directory directly in the profiles folder
   Windows reports (`FOLDERID_UserProfiles`, usually `C:\Users`); what is
   inside a profile stays removable.
5. **Marks are keyed by absolute path**, not tree position, so they survive a
   re-scan; `Marks::refresh` re-reads their sizes and drops what is gone.
6. **The treemap is painted, not composed of elements.** Thousands of
   rectangles belong in one canvas callback; labels are shaped there too so they
   clip to their own tile.
7. **Tile crumbs are absolute.** `treemap::layout` takes the drawn node's
   crumbs and every tile extends them, so a tile resolves from the scanned
   root at any depth. Relative crumbs look right at `~` and silently point
   at other directories after descending — including for marks. Any code that
   turns a path into crumbs walks from the scanned root, too.
8. **The view transform is the only thing zoom changes.** Layout runs in
   base-space pixels and is cached; `screen = (base - origin) * scale`.
9. **The status bar never claims a saving it cannot measure.** Projections come
   from marked bytes *on the volume the meter measures*, which is the scan's own
   notion of a volume — the mount *source* — so a mark on a disk mounted inside
   the tree, which `-X` and the media exception allow, is reported and never
   added to this volume's free space; see `space::attribution`. The final number
   comes from `statvfs` before and after.
10. **An agent marks; a person removes.** `disktree mcp` has no tool that
    deletes, and the window link has no request that does. `review` only
    opens the review screen. When a window is linked it is the agent's
    source of truth: `ls`, `find` and `scan` are answered from its tree,
    a mark is measured from it, and its marks are the marks — every link
    answer carries them, and the MCP session replaces its own with what
    the window says. The session's own scans are read only while no
    window is open. The listing and search logic is `core::query`, shared
    by both sides, so they answer alike.
11. **The link is a same-user socket, nothing more.** `disktree-link`
    binds `0600` in a per-user directory and has no token, because the
    agent already runs as the user and `disktree mcp` scans on its own.
    Do not add a port.

10. **Discovery is not a scan.** No implicit home walk at startup. List
    mounts in the selected machine's namespace without probing them; scans
    stop at nested mounts, including bind mounts and same-device subvolumes.
    Switching machines clears marks and identity. The Unix SSH helper streams
    records and never retains the tree.

11. **A live overview is not an actionable tree.** Keep provisional,
    bounded display summaries separate from `Disktree::tree` and all path,
    mark and removal APIs. Only a complete scan may replace the real tree.
    Counters and sampled paths belong in fixed, monospace progress lanes.

## Where changes belong

| change | where |
| --- | --- |
| bounded Unix SSH traversal | `crates/disktree-core/src/remote_walk.rs` |
| SSH protocol, discovery and transport | `crates/disktree-core/src/remote.rs` |
| measurement, filtering, parallelism | `crates/disktree-core/src/scan.rs` |
| what a node is, or a derived total | `crates/disktree-core/src/tree.rs` |
| tile geometry, nesting, the merged tail | `crates/disktree-core/src/treemap.rs` |
| anything that deletes, or refuses to | `crates/disktree-core/src/removal.rs` |
| free space and projections | `crates/disktree-core/src/space.rs` |
| what macOS lists and measures in bulk | `crates/disktree-core/src/macos.rs` — `unsafe` for `getattrlistbulk` |
| what Windows lists, measures and compares differently | `crates/disktree-core/src/windows.rs` — `unsafe` for Win32 |
| reading a whole NTFS drive from its file table | `crates/disktree-core/src/mft.rs` |
| APFS clone hints and private-size system calls | `crates/disktree-core/src/macos.rs` — isolated `unsafe` |
| clone sharing totals and conservative savings | `crates/disktree-core/src/sharing.rs` |
| a key, a screen transition, a mark | `crates/disktree-app/src/state.rs` |
| spacing, type and size | `crates/disktree-app/src/ui.rs` — tokens only, no `px` in layout |
| the mosaic's painting or labels | `crates/disktree-app/src/treemap_view.rs` |
| layout of a screen | `crates/disktree-app/src/views.rs` |
| colours derived from the theme | `crates/disktree-app/src/palette.rs` |
| the title strip, window controls, the client-side frame | `crates/disktree-app/src/chrome.rs` |
| the macOS icon, in any of its looks | `packaging/macos/` — the SVGs and `AppIcon.icon`, then `make dock-icons` |
| checking, downloading and applying updates | `crates/disktree-app/src/updater.rs` |
| which release a release channel picks | `crates/disktree-app/src/update_channel.rs` |
| release packaging, the DMG, `vpk` | `scripts/release/`, `xtask/src/bundle.rs`, `.github/workflows/release.yml` |
| what an agent can ask of the window, and the answer | `crates/disktree-link/src/lib.rs` (messages), `crates/disktree-app/src/agent.rs` (handling) |
| a tool, its parameters or its wording | `crates/disktree-mcp/src/server.rs` (the MCP surface), `crates/disktree-mcp/src/session.rs` (what it does) |
| how a listing or a plan reads to an agent | `crates/disktree-mcp/src/report.rs` |
| the procedure an agent follows | `skills/disktree-free-space/SKILL.md` |

## Verification expectations

* Size accounting, hardlinks, symlinks, hidden entries, depth limits, the
  removal guards and squarified layout are covered by `disktree-core` tests
  against real temporary trees.
* The screens are covered by window-harness tests that draw frames and press
  keys, including one that marks a directory, confirms the removal and checks
  the files are gone while unmarked neighbours are untouched, and one that
  drives the window over the real socket as `disktree mcp` does.
* `disktree-mcp` tests every tool as a function against a temporary tree,
  with a stand-in window on the socket for the linked cases;
  `disktree-link` tests the socket itself.
* Rendering was verified by those tests and by running the app against a real
  home directory; it has not been eyeballed in every theme and font size.

## Pull requests

* **Screenshots and videos go on the PR with `gh --attach`.** GitHub CLI
  2.99+ uploads them: `gh pr create/edit/comment --attach ./after.png`, with
  the same `./after.png` in the Markdown body, rewrites that link to a
  `user-attachments` URL. Never push images to a side branch, a gist or the
  repository to host them. `gh` exits 0 even when the link does not match, so
  check every image link before calling a PR done. The procedure, and the
  three ways it fails silently, are in
  [docs/pr-screenshots.md](docs/pr-screenshots.md); read it before the first
  `--attach`, and before any later body edit.
* **A PR that changes what the window shows has before/after screenshots**,
  of a contrived tree rather than a real disk;
  [docs/pr-screenshots.md](docs/pr-screenshots.md) says how to make one. If
  nothing can show the change, say why in the body and ask the operator.
* **Scans are private data.** A real scan shows user names in paths,
  project and client names, volume names and sizes, and the chooser lists
  mounts and SSH hosts. Show any screenshot that is not 100% contrived to the
  operator and wait for approval before attaching it.
* **Titles are Conventional Commits:** `type(scope): what changed`, in the
  imperative, as the Pwr family's other repositories do. Scopes follow the
  table above: `core`, `scan`, `ssh`, `removal`, `macos`, `windows`, `app`,
  `ci`, `release`, `docs`. A port of an upstream PR says so in the title,
  `(port of tobi/disktree#N)`, and keeps the upstream commits and their
  authorship.
* **Don't edit `CHANGELOG.md` in feature or fix PRs.** Every open PR that
  adds a line conflicts with every other one. Put the user-facing summary in
  the PR body; the release-prep PR (`chore(release): vX.Y.Z`) writes the
  version's section from the PRs merged since the last tag.
* **PRs go to the fork, `pwrdrvr/PwrDisk`.** In a checkout with several
  remotes, push only to the one pointing there; `tobi/disktree` and personal
  forks are read-only from here.

## PwrAgent threads

* Before opening or attaching a PR, check that the thread's workspace holds
  its head commit. Attaching a PR does not move the workspace.
* Attach the thread's PR and watch it with `notifyOn: ["merged", "closed"]`,
  so the thread hears when it lands or is dropped.
* Work worth doing that this change leaves out goes on a PwrAgent to-do card
  (`add_todo`, with a `start_thread` action whose prompt stands alone), not a
  note in the PR body, a commit message or a code comment. The operator does
  not read those notes; a card can be tracked and started.
* Before launching the app, check whether a PwrDisk window from this checkout
  is already open and reuse it. Several checkouts and builds can run at once.

## Agent instruction files

Agent guidance lives in `AGENTS.md`, not in the README or code comments, and
in the smallest scope that needs it. Long procedures go in `docs/` with a
one-line pointer here that says when to read them. Every `AGENTS.md` has a
sibling `CLAUDE.md` symlink (`ln -s AGENTS.md CLAUDE.md`), so Codex and
Claude read one file.
