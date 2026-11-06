# Protected paths

PwrDisk permits inspecting system folders. Marking and removal use two
policies in `crates/disktree-core/src/removal.rs`:

- **Protected tree:** the path and everything beneath it are refused.
- **Protected container:** the folder itself is refused; individual items
  inside it are judged separately by every other guard.

These are PwrDisk safety rules, enforced even when filesystem permissions
would allow deletion. They cover marking in the window and MCP, Review's
plan, and a fresh check on the removal worker. They do not establish that
an allowed item is disposable or authorize removing it.

## Unix (macOS and Linux)

The Unix lists are shared, preserving the existing policy across both
systems. A path must match whole components, never a string prefix.

| Policy | Paths |
| --- | --- |
| Protected trees | `/bin`, `/boot`, `/dev`, `/etc`, `/lib`, `/lib64`, `/lib32`, `/libx32`, `/proc`, `/run`, `/sbin`, `/sys`, `/usr`, `/efi` |
| System state | `/var/lib`, `/var/db`, `/var/spool`, `/var/mail`, `/var/run`, `/var/lock` |
| Package stores | `/nix/store`, `/gnu/store`, `/home/linuxbrew/.linuxbrew`, `/opt/homebrew` |
| macOS system trees | `/System`, `/Library`, `/private`, `/cores`, `/Network` |
| Application containers | `/Applications`, `/Applications/Utilities` |
| Temporary containers | `/tmp`, `/var/tmp`, `/private/tmp`, `/private/var/tmp` |
| Shared data containers | `/var`, `/var/cache`, `/var/log`, `/var/backups`, `/var/local`, `/var/opt`, `/opt`, `/srv`, `/nix`, `/gnu` |
| Volume containers | `/Volumes`, `/mnt`, `/media`, `/lost+found` |
| Root account's home | `/root` itself (outside the `/home` profiles container) |

There are two existing tree exceptions: items strictly inside
`/private/tmp` and `/private/var/tmp`, and independently mounted removable
media under Linux's `/run/media`. The temporary folders themselves stay
protected. `/private/var/folders` remains a protected tree.

For example, `/Applications` and `/Applications/Utilities` cannot be
marked, while an individual third-party app bundle remains eligible.
The application may require its own uninstaller. `/System/Applications`
is part of a protected tree. On Linux, `/var/log` cannot be marked as a
whole; an individual log still goes through the other guards.

All removal guards also resolve parent directories, so choosing a scan root
through a symbolic link does not disguise a protected tree, container,
profile or nested mount. A selected
symbolic link itself is unlinked rather than following its destination.
macOS comparisons also fold case and the `/System/Volumes/Data` spelling.

## Windows

Installed system locations come from the Known Folder API (`Windows`,
`System`, `ProgramData`, and the native, x86 and x64 Program Files folders),
absolute process environment values, and conventional locations on the
system drive. These are protected trees; uninstall programs through
Settings or their own uninstaller.

At every drive root, these entries and their contents are protected:
`System Volume Information`, `$Recycle.Bin`, `Recovery`, `Boot`, `EFI`,
`Config.Msi`, `bootmgr`, `pagefile.sys`, `hiberfil.sys`, and `swapfile.sys`.
Case and verbatim drive prefixes do not change the verdict. A similarly
named user folder such as `D:\Documents\Recovery` is not reserved by this
rule. UNC shares can expose arbitrary directories, so names such as
`\\nas\backups\Recovery` and `\\nas\backups\Boot` are not reserved by
this drive-root rule. This also applies to verbatim UNC paths; other
removal guards still apply.

## Other guards

The filesystem root, scan root, home directory and its ancestors, profile
containers and each profile directly inside them, mount points and paths
containing mounts retain their guards. macOS history and index stores and
Windows names that Win32 would misread retain their guards too.

These conventional lists cannot discover every custom system installation
or every app's live data. Keep platform references in the bundled agent
skill aligned with changes to the policy.

Layout references: [Apple's filesystem overview](https://developer.apple.com/library/archive/documentation/FileManagement/Conceptual/FileSystemProgrammingGuide/FileSystemOverview/FileSystemOverview.html),
[Filesystem Hierarchy Standard](https://refspecs.linuxfoundation.org/FHS_3.0/fhs-3.0.html),
and [Windows Known Folder identifiers](https://learn.microsoft.com/en-us/windows/win32/shell/knownfolderid).
