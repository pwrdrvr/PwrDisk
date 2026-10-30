# Changelog

Each release's notes are its `## vX.Y.Z` section here: `cargo xtask
release-check` refuses a tag without one, and release.yml publishes the
section as the GitHub release's notes and as the notes Velopack shows beside
an update. Keep new entries under Unreleased until the version is cut.

## Unreleased

## v0.12.0

The first PwrDisk release: disktree 0.11.0, renamed, signed, and installed
and updated through Velopack.

- PwrDisk - The app, binary and bundle are PwrDisk (`pwrdisk`,
  `com.pwrdrvr.pwrdisk`), a maintained fork of tobi/disktree. A Power
  Efficiency preset saved by disktree is still read until PwrDisk saves one.
- macOS - A drag-to-install DMG with a universal app (Apple silicon and
  Intel), signed with a Developer ID, notarized and stapled.
- Windows - `PwrDisk-win-Setup.exe`, a per-user MSI and a portable zip for
  x64, and the same for ARM64 (`PwrDisk-win-arm64-*`), signed through Azure
  Artifact Signing. Setup installs to your profile without administrator
  rights and adds a Start menu shortcut.
- Updates - Copies installed from the DMG, Setup.exe or the MSI check
  GitHub Releases at startup and every six hours, and download a newer
  release in the background. Shift-U (or PwrDisk › Check for Updates… on
  macOS) shows the version, the release channel and what the last check
  found; Restart to update installs a download at once, and otherwise it
  installs when PwrDisk quits. Pick Stable or Beta, Latest or Prerelease
  there; a copy follows the channel its version implies until you do.
- Linux - The tarball and `install.sh` are unchanged. Linux copies do not
  update themselves; download the next release.
