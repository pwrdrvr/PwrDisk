# PwrDisk on macOS

What macOS keeps outside a file scan, what PwrDisk refuses, and what to
leave alone. Read with `skill://pwrdisk-free-space/SKILL.md`.

## Reading the numbers

- **APFS clones share blocks.** Finder duplicates, `cp -c` and some package
  managers make clones: two copies can cost one copy's space, and removing
  one frees little or nothing. PwrDisk counts each file's allocation, so a
  cloned tree can look bigger than what removing it gives back. The free
  space measured after the review is the real figure.
- **One container, several volumes.** The System, Data, Preboot, Recovery
  and VM volumes share one APFS container's space. A whole-disk scan of
  `/` explains the Data volume; the rest shows as space the scan does not
  explain. `diskutil apfs list` shows each volume's use.
- **Local snapshots hold deleted data.** Time Machine and system updates
  keep APFS snapshots; space freed while one exists may not come back until
  it expires. `tmutil listlocalsnapshots /` lists them. Deleting one
  (`tmutil deletelocalsnapshots`) is the user's decision; give the command,
  do not run it.
- **"Purgeable" is not free.** Finder counts purgeable space as available;
  PwrDisk and `df` do not. If the numbers disagree, that is usually why.
- **Swap and the sleep image** live on the VM volume. Leave them alone.

## Privacy and access

- macOS protects Desktop, Documents, Downloads, Mail, Messages, Safari,
  Photos and parts of `~/Library`. A scan started by the agent runs with the
  privacy grants of the app hosting the agent, such as a terminal or an
  editor. It may show these folders as unreadable or make macOS prompt for
  that app.
- A scan the PwrDisk window runs uses PwrDisk's own grants. If a protected
  folder matters, open the window (`pwrdisk_show`) and scan from there.
- Do not ask the user to give the agent's host Full Disk Access just to
  measure. Report what was unreadable and let them decide.

## Refused by PwrDisk

`pwrdisk_mark` keeps these back and says why: `/System`, `/Library`,
`/private`, `/cores`, `/opt/homebrew` and the other system trees, the home
directory and anything holding it, mount points, and the `.fseventsd` and
`.Spotlight-V100` stores (use `mdutil` for Spotlight, never delete either).

## Never propose

- **Inside a library package**: the Photos Library, Music, Apple TV, iMovie,
  Final Cut and Logic libraries. Removing files inside breaks the library;
  the app manages its own media.
- **`~/Library/Mobile Documents`, `~/Library/CloudStorage`**: iCloud Drive
  and File Provider folders (Dropbox, Google Drive, OneDrive). Deleting a
  file there deletes it in the cloud and on every device. An evicted file
  takes no local space; to free space without deleting, the user chooses
  "Remove Download" in Finder or turns on Optimize Mac Storage.
- **App data**: `~/Library/Containers`, `~/Library/Group Containers` and
  `~/Library/Application Support` hold apps' databases and settings. Only
  propose a specific, known cache inside them, with the app quit.
- **Mail and Messages attachments, Keychains, and iPhone and iPad backups**
  (`~/Library/Application Support/MobileSync/Backup`): user data and
  secrets. Old device backups are the user's to delete from Finder.
- **`/Applications`**: uninstall with the app's uninstaller or by dragging to
  the Trash in Finder, which the user does.

## Usually safe, with the owning tool first

| what | where | prefer |
| --- | --- | --- |
| app caches | `~/Library/Caches/<app>` | quit the app first; they regrow |
| Xcode build data | `~/Library/Developer/Xcode/DerivedData` | Xcode rebuilds it |
| old device support | `~/Library/Developer/Xcode/iOS DeviceSupport` | re-created when a device connects |
| simulators | `~/Library/Developer/CoreSimulator` | `xcrun simctl delete unavailable`; runtimes in Xcode › Settings › Components |
| Xcode archives | `~/Library/Developer/Xcode/Archives` | ask: needed to symbolicate shipped builds |
| Homebrew downloads | `~/Library/Caches/Homebrew` | `brew cleanup --dry-run`, then `brew cleanup` |
| Docker Desktop | `~/Library/Containers/com.docker.docker/.../Docker.raw` | never mark the disk image; `docker system df`, then prune |
| language caches | `~/.npm`, `~/.cargo/registry`, `~/Library/Caches/pip`, `~/.gradle/caches` | the tool's clean command |

## Trash

Marks go to the Trash on the volume that holds them. The space returns when
the user empties the Trash; on an external disk, that disk's `.Trashes`.
