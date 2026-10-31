# Changelog

Each release's notes are its `## vX.Y.Z` section here: `cargo xtask
release-check` refuses a tag without one, and release.yml publishes the
section as the GitHub release's notes and as the notes Velopack shows beside
an update. Keep new entries under Unreleased until the version is cut.

## Unreleased

## v0.14.0

- Removal safety - Settings > Safety adds a per-operation limit, defaulting
  to 5% of each volume's capacity, for both manual and MCP removal. The
  complete selection is re-measured before removal; oversized, unreadable
  or unknown-capacity selections are refused.
- AI-assisted removal - Preview the marked paths, size and trash/permanent
  mode before confirming removal through the MCP client's interactive
  form. Confirmation is expiring and single-use; changed marks, mode or
  safety settings require a new preview. Clients without form support
  use human Review instead. Removal requires a linked local window, and
  Trash never falls back to permanent deletion.
- Agent browsing - Listing never moves the window, including requests
  from older clients that pass `show`. Navigation uses an explicit
  `pwrdisk_show` request when the user asks to follow or open a path.
- Cleanup candidates - Recognize verified pnpm package stores without
  classifying the executable, global tools or the whole installation as
  disposable. Unknown aggregates retain their own category, and legacy
  Google Drive folders are recognized as synced data.
- Scan context - Linked MCP status reports allocated/apparent size,
  hidden-entry inclusion and unreadable counts. macOS reports explain
  APFS sharing and distinguish listed bytes from potential savings;
  cleanup searches explain removal eligibility without adding marks.
- Compatibility - The window link now uses protocol version 3. Update
  and restart both the PwrDisk window and MCP process together.

## v0.13.0

- Downloads - Add stable filenames for the universal macOS DMG and both
  Linux tarballs so the README's download buttons keep working across
  releases. Versioned downloads remain available.
- Documentation - Put downloads, installation and a quick tour first in
  the README, with detailed usage and build instructions in separate
  guides.
- Third-Party Notices - Include the socket2 attribution required by the
  Windows agent-window link and refresh notices for this release.

## v0.12.0

The first PwrDisk release, continuing from disktree 0.11.0 with local and
SSH exploration, AI-assisted review, and signed installers.

- PwrDisk - The app, binary and bundle are PwrDisk (`pwrdisk`,
  `com.pwrdrvr.pwrdisk`), a maintained fork of tobi/disktree. A Power
  Efficiency preset saved by disktree is still read until PwrDisk saves one.
- macOS - A drag-to-install DMG with a universal app (Apple silicon and
  Intel), signed with a Developer ID, notarized and stapled.
- Windows - `PwrDisk-win-Setup.exe`, a per-user MSI and a portable zip for
  x64, and the same for ARM64 (`PwrDisk-win-arm64-*`), signed through Azure
  Artifact Signing. Setup installs to your profile without administrator
  rights and adds a Start menu shortcut.
- Sources and SSH - Choose a local volume or an SSH host and root before
  scanning. Explore remote disks with live scan progress and treemap
  previews, then export a cleanup plan; remote scans are read-only.
- AI agents - `pwrdisk mcp` lets an agent inspect disk usage and mark an
  agreed cleanup list. Marks appear in the open window on macOS, Linux
  and Windows, and the agent can open Review for you. Only a person can
  confirm removal. Embedded skills explain each system's storage and
  cleanup cautions.
- Scan progress - Whole-drive scans show estimated byte coverage and
  average files per second, followed by an explicit finalizing state.
  Progress scrolls in short windows while the free-space meter stays
  visible. Folder and filtered scans keep counts when no compatible
  total is available.
- APFS space estimates - Show clone sharing separately from treemap
  sizes and estimate what the marked set can reclaim together. Retained
  clones and hardlinks reduce projected savings; the volume meter uses
  the conservative estimate and reports measured free space after removal.
- macOS service stores - Show measured Spotlight-index and FSEvents
  usage when readable, label partial results, and keep those stores
  protected from raw removal. Unreadable service stores do not produce
  repeated Full Disk Access hints.
- Explore and Review - Custom window chrome and faster marking, path
  lookup and review keep large cleanup queues responsive.
- Updates - Copies installed from the DMG, Setup.exe or the MSI check
  GitHub Releases at startup and every six hours, and download a newer
  release in the background. Shift-U (or PwrDisk › Check for Updates… on
  macOS) opens Settings › Updates; Restart to update installs a download
  at once, and otherwise it installs when PwrDisk quits. Pick Stable or
  Beta, Latest or Prerelease there; a copy follows the channel its version
  implies until you do.
- Settings and Help - Shared menus, separate Updates, Diagnostics and
  About panes, and offline Changelog, License and Third-Party Notices
  windows make support information easier to find. Copy Diagnostics Info
  omits scanned paths, SSH hosts and volume names.
- Logs - A live viewer adds wrapped lines, search with Filter and
  Highlight modes, context rows, line selection, and marks for copying
  just one action's output. Following pauses while you read, and display
  preferences persist. Logs record counts and durations, never scanned
  paths.
- Linux - The tarball and `install.sh` are unchanged. Linux copies do not
  update themselves; download the next release.
