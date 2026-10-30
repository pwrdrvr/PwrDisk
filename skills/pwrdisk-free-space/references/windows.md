# PwrDisk on Windows

What Windows keeps outside a file scan, what PwrDisk refuses, and what to
leave alone. Read with `skill://pwrdisk-free-space/SKILL.md`.

## No window link yet

On Windows `pwrdisk mcp` scans, lists, finds and marks on its own, but it
cannot reach a PwrDisk window: `pwrdisk_show` and `pwrdisk_review` say so
and do nothing. The marks stay in the agent's session. To hand over, give
the user the agreed list of paths with sizes. They open PwrDisk, scan the
same root, mark those paths and review them there. Do not remove them
yourself, and do not keep calling `pwrdisk_review`.

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
| app caches | `%LOCALAPPDATA%\<app>\Cache` | close the app; they regrow |
| browser caches | the browser's profile `Cache` folders | the browser's clear-cache setting |
| language caches | `%LOCALAPPDATA%\npm-cache`, `%USERPROFILE%\.cargo\registry`, `%LOCALAPPDATA%\pip\Cache` | the tool's clean command |
| build output | `target\`, `node_modules\`, `bin\`, `obj\` | check the checkout first (see the skill) |
| game libraries | `steamapps`, Epic and Xbox folders | uninstall in the launcher |

## Recycle Bin

Removal moves marks to the Recycle Bin of their drive. The space returns
when the user empties it. A network share has no Recycle Bin, so a removal
there may not be recoverable; tell the user before they mark one.
