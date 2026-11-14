# PR screenshots

`AGENTS.md` asks for before/after screenshots on any PR that changes what the
window shows. This file covers making them without leaking a real disk, and
attaching them with `gh` so the links render. The procedure is the Pwr
family's; PwrGit's `docs/pr-screenshots.md` and PwrAgent's `AGENTS.md` carry
the same rules.

## Making them

A disk scan is private by nature: paths carry user names, project and client
names; the status bar names the volume and its free space; the source chooser
lists mounts and the SSH hosts from your config. So point the app at a tree
built for the picture, never at `~` or a real volume.

## Offscreen macOS pictures

The operator approved this host workflow on 2026-11-04. It renders the real
PwrDisk view through GPUI's headless Metal renderer and native macOS text.
It creates only TestWindows, opens no on-screen window, sends no OS mouse
or keyboard input, and never connects to an existing PwrDisk window. Run
it as the binary on the main thread: libtest cannot initialize MacPlatform
on its worker threads, even with `--test-threads=1`.

This is opt-in development tooling. `screenshots` enables GPUI test support
and PNG encoding; a release build with that feature is a compile error.
Normal builds reject the screenshot command before normal startup. The
mode skips startup hooks, logging, updates and the agent link. It applies
a fixed theme without the native theme watcher. `cargo xtask lint` checks
both feature configurations, and `cargo xtask test` covers the scenario
parser and script adapters on CI.

From the repository root, on a Mac with Rust, Python 3.9+ and the usual
build tools:

```sh
scripts/screenshots/fixtures.sh scripts/screenshots/render.sh \
  scripts/screenshots/explore.json target/pr-shots/demo
```

`fixtures.sh` creates a temporary 256 MiB APFS image named Fixtures, mounts
it with `-nobrowse` at `/Volumes/Fixtures`, and fills `Demo` with invented
Projects, Cache, Media and Documents data. The files contain real allocated
bytes; sparse files do not have the desired tile weights. Fixed fixture
timestamps keep age labels unchanged across rebuilds. It refuses an
existing `/Volumes/Fixtures`, exports `PWRDISK_FIXTURE_ROOT` to the command,
and detaches and removes its image on success or failure. A failed detach
is an error and prints the retained image path: resolve it before finishing.
Do not use real paths or disks as fixture input.

The wrapper builds a debug binary and calls only:

```sh
target/debug/pwrdisk offscreen-screenshots \
  scripts/screenshots/explore.json /Volumes/Fixtures/Demo target/pr-shots/demo
```

Run that direct command only while the fixture wrapper owns the mounted
image. Never substitute a normal app launch or `cargo run`.

For before and after with one fixture mount and the same scenario:

```sh
scripts/screenshots/pair.sh scripts/screenshots/explore.json target/pr-shots
# Optional third argument: a different before ref (default origin/main).
```

The pair command fetches `origin/main`, makes its own detached worktree in
`target/`, and builds both revisions with this workspace's `target/` cache.
Each build cleans only PwrDisk's path crates, preserving expensive GPUI
dependencies. Cargo can otherwise reuse another worktree's same-version
binary. The scripts set `PWRDISK_SCREENSHOT_CHECKOUT`; the renderer verifies
its compiled source directory against it and prints the compiled commit.
The adapter transplants only the screenshot harness, entry point, feature,
dependency edges and constructor gates into the temporary baseline. It
preserves the baseline UI and locked package versions; it fails if its
anchors or GPUI version differ, requiring a deliberate adapter update.
It removes its baseline worktree even if rendering fails. Outputs go to
`before/` and `after/`; the final shared binary is rebuilt from this
workspace. Do not run another Cargo command against that target directory
concurrently. The scripts honor an absolute `CARGO_TARGET_DIR` override.

### Scenarios

A scenario is strict JSON: unknown fields and steps fail before rendering.
The scan root and output directory are explicit command arguments; there
is no implicit home scan. `size` is in logical pixels (320..2560 per side),
and `theme` is `tokyo_night` or `flexoki_light`. PNG dimensions depend on the
renderer scale; the command prints both sizes. Keep the same Mac and fonts
for before and after.

```json
{
  "size": [1180, 620],
  "theme": "tokyo_night",
  "steps": [
    {"op": "capture", "name": "explore.png"},
    {"op": "setup", "state": {"type": "legend", "kind": "code"}},
    {"op": "hover", "at": [450, 400]},
    {"op": "wait", "ms": 900},
    {"op": "capture", "name": "hover.png", "pointer": true},
    {"op": "click", "at": [450, 400]},
    {"op": "capture", "name": "clicked.png", "pointer": true}
  ]
}
```

Steps run in order, settling redraws between them:

Screenshot runs explicitly enable reduced motion. Navigation lands on its
destination layout immediately; it never interpolates with the wall clock.
The simulated clock still advances tooltip delays and other timed tasks.

| Step | Behavior |
| --- | --- |
| `hover`, `at: [x,y]` | Move GPUI's simulated pointer in logical pixels. |
| `click`, `at: [x,y]` | Move, dispatch left down and up, with real hit testing. |
| `wait`, `ms` | Advance the deterministic clock, up to 60000 ms per step; useful for tooltips. |
| `capture`, `name`, `pointer` | Write a PNG. `pointer` defaults false; true draws an arrow at the last hover/click. |
| `setup`, `state` | A named app state hook, described below. |
| `usage_history`, `compare` | Switch to the Usage History window over two invented reports of the fixture root, written to `/Volumes/Fixtures/usage-reports`, with Before and After picked; `compare: true` also runs Compare. `scripts/screenshots/usage-history.json` captures both. |

Setup hooks are `legend` with a `kind` (the core category name, such as
`code`, `cache`, `media` or `documents`), `select` or `mark` with a relative
POSIX fixture `path`, `crumb_menu` with the `path` of a directory to go into
and open its crumb's sibling menu, and `review`, `file_list` or `help`
without arguments.
`legend` cycles that category as a click does. Marks obey the normal guards
and capacity limit; a refused mark fails the scenario. Setup does not
confirm removal. Never use clicks to approve removal, open the source
chooser, launch external apps or run owner commands. Scenarios are trusted
developer input, not a sandbox for arbitrary app interactions.

`permanent_review` opens Review with Permanent selected without confirming
removal, for pictures of that mode independent of the host's Trash backend.

`cleanup_review` opens the marked fixture paths' cache cleanup proposals
with an empty CLI search path. It never probes installed owners or chooses
an action; every path stays on Skip. Environment hints for a screenshot
must point only into the contrived fixture image.

`removed` with a relative fixture `path` simulates a successful removal
report in memory. It deletes no files and approves no removal. Consecutive
reports can show removal accounting before a rescan.

`scripts/screenshots/owner-cleanup.sh target/pr-shots/owner-cleanup` builds
before/after pictures for pnpm's separate metadata/dlx cache. It adds only
invented data to the owned Fixtures image and sets its XDG cache hint there.

`bash scripts/screenshots/temporary-labels.sh target/pr-shots/temporary`
compares Review's inherited temporary and agent scratch labels. Its invented
`private/tmp/claude-501` and sibling folders contain Git history so Review
uses the category after the reclaim hint is cleared. It never confirms
removal or uses a real temporary directory's contents.

`bash scripts/screenshots/classification-context.sh target/pr-shots/context`
compares diagnostic logs, Trash containing Git history, temporary data below
a cache, and a scan starting directly at the invented `private/tmp` folder.
An optional second argument selects the before revision. Both scans stay
inside the same owned Fixtures image and stop at Review without removal.

`bash scripts/screenshots/classification-precedence.sh target/pr-shots/precedence`
compares narrow scans below a Python cache's temporary folder and below a
macOS store lookalike inside Git. An optional second argument selects the
before revision. Both scans use the owned Fixtures image. The temporary
case stops at Review; the protected-store case stays in Explore without
marking a path or bypassing a guard.

`bash scripts/screenshots/protected-folders.sh target/pr-shots/protected`
compares the Applications folder and an individual invented app. Its
`system_folders` setup retains the scanned fixture tree and fixture disk
meter but maps paths to a logical `/`, so the real removal guards apply.
It never scans the host's root or modifies its system folders. This hook
requires the owned `/Volumes/Fixtures/Demo` image and is only for pictures.

`disk_capacity` supplies invented raw and native capacity counters (100 GiB
capacity, 5 GiB raw free, 15 GiB available and 80 GiB System/Data usage).
It requires the owned Fixtures image and never changes its actual capacity.
Use it with `system_folders` to compare startup-disk presentation across
revisions; old versions ignore the optional native counters.

`removal_check` and `removal_refused` render the marked fixture paths in
preflight and a simulated whole-selection limit refusal. They require the
owned Fixtures image and never start a removal worker or approve deletion.
The refusal uses a 1% fixture limit and a contrived negative free-space
change so the zero-removal explanation is visible.

Capture names are unique simple filenames: letters, digits, `_` and `-`,
then `.png`. A scenario needs 1..256 steps and at least one capture. Each
capture also writes `<name>.bounds.json`, with logical bounds and paths for
Kit's observed controls. This can be empty: GPUI's `debug_selector` bounds
are only exposed through its test context, not HeadlessAppContext. For
other elements, inspect a first render and divide image coordinates by the
printed scale (usually 2x); do not guess from another revision's layout.
Add a new named setup hook in `screenshots/render.rs` when a change needs
state that these hooks cannot express, and keep it confined to fixture data.

**Inspect every PNG.** Real rendering has caught a see-through pill that
headless state tests missed. Check clipping, tooltip delay and private data.
A drawn arrow is an annotation, not a native pointer; say so in the PR body.
For example: "GPUI offscreen Metal render with native macOS text, contrived
Fixtures image, Tokyo Night, 1180×620 logical pixels; arrow drawn by the
harness." Copy only the PNGs to short temporary attachment paths. Never
commit screenshots or the image. Attach with `gh --attach` below and fetch
every resulting URL; an upload can silently return 404.

`scripts/screenshots/size-units.json` captures the platform default, an
explicit binary override, and Settings › General. Its `size_units` setup
changes only the offscreen process; `settings` opens another TestWindow,
with saving disabled. Neither reads nor writes the saved size preference.

## When this needs the lab or the operator

Native accessibility, VoiceOver, OS dialogs, window chrome behavior and
anything that requires an on-screen macOS window still use the PwrSuiteLab
headed E2E VM. Read [headed-e2e-pwrsuitelab.md](headed-e2e-pwrsuitelab.md)
before the first job. Without that MCP, stop and ask the operator; there is
no local desktop or script fallback. For Linux or Windows windows, ask the
operator. Offscreen macOS pixels do not verify these systems.

In the lab, create the same invented Fixtures image and allocated tree,
launch `pwrdisk` on its explicit root, drive the screen, and capture only
its window (`screencapture -o -x -l <window id>`). Detach the image and quit
the app at the end. List checkout-relative image paths in the job's
`artifacts` and fetch them with `lab_e2e_collect`. Copy images to short local
attachment paths; keep artifact directory paths and all other lab details
out of the PR. For before, run the same job against a clean committed
`origin/main` worktree, then remove that worktree. Use identical fixture
bytes, window size, theme and fonts in both jobs.

Lab VMs have clipboard sharing off. Inputs arrive through committed source,
outputs through collected artifacts; never ask the operator to copy or
paste anything in the guest. The source chooser lists real mounts and SSH
hosts on either system. Do not capture it with this offscreen workflow or
attach a guest chooser capture. Say why in the PR and ask the operator.

For any image that is not 100% contrived, **show it to the operator and
wait for approval before attaching it**. Crop or redact user names, host
names, volume names and anything else that is not invented. Say in the PR
body how each image was made, without lab details. If nothing can show the
change, say why and ask the operator before merging; a missing screenshot
is not optional.

## Attaching them with `gh --attach`

GitHub CLI 2.99+ attaches images and videos with the repeatable `--attach`
flag on `gh pr create`, `gh pr edit` and `gh pr comment`. Reference the local
path in the Markdown body and `gh` uploads the file and rewrites the link to a
`https://github.com/user-attachments/assets/...` URL in place. Unreferenced
attachments are appended at the end. Check `gh --version` first.

Do not host images any other way: not on a side branch, a gist, or committed
to the repository. Those links break when the branch is deleted, and they put
binaries in the fork's history.

```sh
gh pr create --repo pwrdrvr/PwrDisk \
  --title "fix(app): keep tile labels inside their tile" \
  --body-file /tmp/pr-body.md \
  --attach ./before.png \
  --attach ./after.png
```

```markdown
## Before
![labels before](./before.png)

## After
![labels after](./after.png)
```

Alt text can follow the path after `#`, as in
`--attach './after.png#labels after'`; the `#alt` part is not matched.

Three ways this fails silently. `gh` exits 0 in every one of them:

* **The match is a literal string comparison.** `gh` rewrites `](path)` only
  when `path` is byte-identical to what `--attach` was given, so
  `--attach /tmp/shots/after.png` against `![after](./after.png)` does not
  match. The upload still happens, the dead relative link stays where it was,
  and a second copy is appended at the end. Use the same spelling in both
  places, and keep paths short enough to retype exactly.
* **After the upload, edit the live body, never your draft.** The rewrite
  exists only in the uploaded body; your local file keeps the local paths. For
  every later edit, run `gh pr view <n> --json body -q .body > <file>` and edit
  that copy. Re-sending the draft replaces the live `user-attachments` URLs
  with dead local paths. To add an image later, pass `--attach` for the new
  file only: existing `user-attachments` URLs match no `--attach` path, so
  `gh` leaves them alone, and attaching an uploaded image again appends a
  duplicate.
* **Check the result before calling the PR done.** Finding `user-attachments`
  URLs in the body is not enough; after a mismatch they are there, but in the
  wrong place. Every image link in the body must be a `user-attachments` URL,
  and each must fetch with HTTP 200 and the local file's byte count:

  ```sh
  gh pr view <n> --repo pwrdrvr/PwrDisk --json body -q .body \
    | grep -o '!\[[^]]*\]([^)]*)' | sed 's/.*](\(.*\))/\1/' \
    | while read -r url; do
        curl -sL -o /dev/null -w "%{http_code} %{size_download} $url\n" "$url"
      done
  ```

  To recover from a mismatch without uploading again, copy the URLs out of
  the body and `gh pr edit --body-file` with them where you wanted them.
