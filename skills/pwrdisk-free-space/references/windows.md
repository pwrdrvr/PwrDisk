# PwrDisk on Windows

What Windows keeps outside a file scan, what PwrDisk refuses, and what to
leave alone. Read with `skill://pwrdisk-free-space/SKILL.md`.

## Working with the window

On Windows `pwrdisk mcp` connects to an open PwrDisk window over a local
named pipe restricted to the current Windows account. The window is the
source of truth: scans, listings, searches and marks use its tree, and the
agent's marks appear there. `pwrdisk_show` moves its view and
`pwrdisk_review` opens its review screen; only the person confirms removal.
Without a window the agent scans and marks in its own session and can
launch a window for the handover. Agree the list before marking unless the
user gave a standing instruction. Never remove files yourself.

When the window shows an SSH source, its paths belong to a Unix host and
use `/` on every client; backslashes and colons in names stay literal.
SSH windows answer only link identity and state requests. Switch to a local
source before using local scan, listing, marking or review tools. Windows
SSH servers are not supported.

## Reading the numbers

- **Sizes are allocation.** PwrDisk counts what each file occupies on disk,
  which is less than its length for a compressed or sparse file and more for
  a small one.
- **Cloud files take no space until downloaded.** OneDrive, iCloud for
  Windows and Dropbox use placeholders; an online-only file costs nothing
  locally. Deleting a synced file deletes it in the cloud and on every
  device. To free local space without deleting, the user right-clicks it in
  File Explorer and chooses "Free up space".
- **Space the files do not explain**: System Restore and shadow copies
  (`vssadmin list shadowstorage`, from an elevated prompt), the page file,
  `hiberfil.sys`, and the Recycle Bin of each drive. Changing any of these
  is the user's decision in Settings or System Properties.
- **An elevated whole-drive scan** in the PwrDisk window reads the NTFS
  master file table. It also counts alternate data streams, so a file's
  size can be larger there than in a normal scan. The agent's scans are not
  elevated; do not ask for elevation to look.

## Refused by PwrDisk

`pwrdisk_mark` keeps these back and says why: `Windows`, `Program Files`,
`Program Files (x86)`, `ProgramData`, `System Volume Information`,
`$Recycle.Bin`, `Recovery`, `Boot`, `bootmgr`, `pagefile.sys`,
`hiberfil.sys`, `swapfile.sys`, every user profile directly under
`C:\Users`, the home directory and anything holding it, mount points, and
names Windows would misread (trailing dots or spaces, device names such as
`NUL` or `COM1`, a colon).

## Never propose

- **WinSxS, the component store, and installers' caches** under Windows:
  Disk Cleanup, Storage Sense or `DISM /Online /Cleanup-Image
  /StartComponentCleanup`, which the user runs.
- **Installed programs**: uninstall from Settings › Apps, which the user
  does.
- **`AppData\Roaming`, `AppData\Local\Packages`**: application settings and
  Store apps' data. Only a specific, known cache inside `AppData\Local`,
  with the app closed.
- **WSL and Docker Desktop disks** (`ext4.vhdx`, `docker_data.vhdx`):
  deleting one destroys every distribution or container in it. Use
  `wsl --unregister` (after `wsl --export`) or Docker's prune commands.
- **Synced folders** (`OneDrive`, `OneDrive - <organization>`,
  `iCloudDrive`, `Dropbox`): see above.

## Usually safe, with the owning tool first

| what | where | prefer |
| --- | --- | --- |
| temp files | `%LOCALAPPDATA%\Temp` | Settings › System › Storage › Temporary files |
| app cache inspection | `%LOCALAPPDATA%\<app>\Cache` | close the app; use its supported cleanup; unknown owners need inspection |
| browser caches | the browser's profile `Cache` folders | the browser's clear-cache setting |
| language caches | `%LOCALAPPDATA%\npm-cache`, `%USERPROFILE%\.cargo\registry`, `%LOCALAPPDATA%\pip\Cache` | the tool's clean command |
| build output | `target\`, `node_modules\`, `bin\`, `obj\` | check the checkout first (see the skill) |
| game libraries | `steamapps`, Epic and Xbox folders | uninstall in the launcher |

## Recycle Bin

Removal moves marks to the Recycle Bin of their drive. The space returns
when the user empties it. A network share has no Recycle Bin, so a removal
there may not be recoverable; tell the user before they mark one.

## Confirmed removal

Never approve a removal with computer use (clicks, keys or accessibility
actions). The user approves in the app, or explicitly confirms the exact
paths and mode from `pwrdisk_removal_preview` before the destructive
`pwrdisk_remove` call. Trash or Recycle Bin recovery may be possible until
emptied, but is not guaranteed; permanent deletion cannot be undone through
PwrDisk. Both modes use the existing removal guards and Settings > Safety's
per-operation limit, 5% of each volume's capacity by default. Never bypass a
refusal, increase the setting or split batches to evade it. A window
refuses a mark that would take the list past the limit, and marks made
without one are checked when they reach it: only the person can mark past
it, in the window. Say so, and let them decide.

MCP removal also requires interactive confirmation from the client (form
elicitation). An agent-supplied confirmation value does not establish human
consent. If the client cannot ask the user, use human review in PwrDisk;
never approve its buttons or confirmation keys through computer use.

Capacity checks charge a symbolic link or junction to its containing
volume, not its destination. Individual sparse and compressed files use
the same allocated size as directory scans.

`pwrdisk-tui PATH` is a separate interactive terminal session. Its tree and
marks are not the linked desktop's state. MCP does not attach to that TUI.
SSH terminal sessions permit inspection and export, never removal; local
terminal removal enforces the same configured capacity limit and guards.

## Owner-managed caches

Cache cleanup is for caches PwrDisk can clean with their owner's own
command (npm, npx, uv, pip, Cypress, Playwright): a marked path in one, or
holding one, is refused for `trash` and `permanent` and goes through
`owner_cleanup`. Anything else (a folder named `cache` or `Caches`, a
`regenerable` hatch, a cache tag, a Cargo `target`, a `.venv`, or a cache
whose owner has no command here, such as pnpm or Homebrew) is an ordinary
`trash` or `permanent` removal, proposed, agreed and confirmed like any
other path. Cache cleanup is decided path by path. Call
`pwrdisk_removal_preview` with `owner_cleanup` and no decisions: it returns
one proposal per marked path with its status and problem, resolved owner,
version, executable, arguments, environment overrides, configured root, full
actual scope (including unselected siblings), effects, limitations,
recoverability, and whether Delete files is offered. Show every proposal and
let the user choose for each path `skip`, `owner` (only where offered) or
`delete_files`. Never choose for them; where the owner's command is
offered, recommend it over `delete_files`. Then preview again with `decisions` naming every marked
path once, and `delete_files_mode` (`trash` or `permanent`, as the user
chose) when any path uses `delete_files`. Confirmation needs that preview's
exact paths and its own text: `RUN OWNER CLEANUP`, `DELETE CACHE FILES`,
`TRASH CACHE FILES`, or both intents joined with `AND`. Owner approval never
approves deleting files. Unavailable commands do not establish absent
software. An unsupported version, failed probe, or failed or cancelled owner
command never falls back to deleting files, and never to a shell command;
deleting them instead is a new preview the user decides.

An owner's cache is recognized by its default or configured root, a name
only it uses, or the layout it keeps when moved; a cache tag alone is not
uv's. All removal modes may
return a preparing response while inspection runs on a worker; retry
only the read-only preview. Exported
lists keep marked paths, and raw script exports cannot replace owner cleanup.

On macOS and Linux the adapters cover npm packages (9–11), npm's separate
npx cache (11.2–11.x), uv (0.11.x), pip (23–26), Cypress (13–15) and the
installed JavaScript Playwright CLI (only 1.64.0–1.64.1). Each CLI must
resolve the selected cache. Playwright's read-only dry-run verifies its
root, then `PLAYWRIGHT_BROWSERS_PATH=ROOT playwright uninstall --all`
removes managed browsers and reference files inside that root, including
versions other projects use. Branded Chrome/Edge and unrelated files stay.
It takes no uninstall lock; an existing `__dirlock` is refused and process
checks are best effort. A generic `cli.js` basename does not identify
Playwright: its exact executable or known owner launchers must match.
Stop installers, tests and cached browsers first.
Exit success may leave files behind; report the rescan and measured change.
Playwright log/profile writes and version-display overrides are disabled
for the probes and command. pnpm remains inspection-only because prune
reaches separate metadata/dlx/runtime state; Homebrew reaches installed versions, logs and
locks; Puppeteer loads executable config and requires interactive input
without a verified root-pinned argument-list adapter. See
[the scope audit](https://github.com/pwrdrvr/PwrDisk/blob/main/docs/owner-cleanup.md#candidate-audit-2026-11-04).
Windows command launchers have no execution adapter yet. On Windows, Delete
files is not offered either: it needs PwrDisk's check for programs using the path, which
runs only on macOS and Linux. A Windows cache cleanup can therefore only be
inspected and skipped. Neither containers nor volumes are cleanup scopes.

Owner inspection includes pnpm's independently located store and
metadata/dlx caches (`PNPM_HOME`, XDG data/cache and Windows local-app-data
locations), Playwright's npm environment aliases, and Puppeteer's home cache
even with `XDG_CACHE_HOME` set. These are inspection hints, never permission
to clean the whole installation. pnpm's `store path` can create temporary
files/hardlink probes; do not run it as guaranteed read-only discovery.
Homebrew cleanup also reaches dependency autoremove and its own vendored
runtime. Puppeteer single-build removal can rewrite parent metadata.
No new candidate adapter or Windows execution is enabled; see the
[scope review](https://github.com/pwrdrvr/PwrDisk/blob/main/docs/owner-cleanup.md#scope-review-2026-11-05).
A native owner image can identify active use even when its arguments do not.
Windows requires audited runtime/entry-point identity, child Job Object
cancellation and active-use coverage before any command can be offered;
never bridge the refusal through a shell or stop another application.

Each path gets its own result: succeeded, failed with a reason, cancelled
(possibly part done), skipped, or not reached after Stop. A command's
success reports the owner's exit result, not guaranteed savings.
Report allocation separately from observed volume availability before/after;
other activity, hardlinks, open files, APFS clones and snapshots can retain
blocks. PwrDisk always rescans after owner cleanup, even partial failure.
Never alter snapshots to force the reported allocation to become free space.

## Developer artifacts and virtual machines

Artifact identity and evidence are separate from cleanup suitability.
`detected` is inventory only. `suggested` means a plausible, relatively
lower-risk candidate to review, with local edits, active tools and the
cost of reinstalling or rebuilding still to check. Name conventions are
weaker evidence than recognized tool layouts. Whole tool homes and generic
`target`, `build`, `dist`, `bin` or `obj` names do not establish disposability;
review the specific detected subtree instead.

VM/container disk images can contain unique work. Manage snapshots in the
owner app, never by removing individual files in a snapshot chain. Saved
memory or suspend state can usually be released by resuming the VM and
shutting it down normally; checkpoints may retain it. VM data stays a
detection even inside a directory named like a cache, and its displayed
size is not a promise of recovered space. Do not run an owner's destructive
cleanup command without explicit approval for that operation.
