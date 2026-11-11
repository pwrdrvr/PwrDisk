# Usage reports

A usage report is a small record of where a volume's space is, kept so
that a later one can say where it went. It is written from a scan with no
window:

```sh
pwrdisk report -D -o ~/reports/$(date -u +%Y%m%dT%H%M%SZ).jsonl
pwrdisk report-diff ~/reports/monday.jsonl ~/reports/today.jsonl
```

`report` takes the same scan options as `pwrdisk` itself (`-a`, `-H`,
`-X`, `--power-efficiency` …), a `PATH` or `-D` for the whole disk,
`--threshold SIZE` (default `100MB`) and `-o FILE` (default standard
output). Sizes are bytes unless given a unit: `kB`, `MB`, `GB`, `TB` are
powers of 1000; `KiB`, `MiB`, `GiB`, `TiB` of 1024.

A report holds real paths. Keep it where you would keep the disk's
contents, and do not attach one to an issue.

## What a report keeps

Every directory, at any depth, whose subtree total reaches the threshold,
plus the scanned root. Nothing else: no file, no smaller folder, no list of
clone groups. A subtree total only grows towards the root, so the parent of
every kept folder is kept as well, and a report is one tree.

A whole startup volume of a few million files keeps a few thousand
folders at the default threshold: under a megabyte, and a small fraction
of that gzipped. Compression is left to the reader.

The figures are the scan's own: the tree's subtree totals (see invariant 2
in `AGENTS.md`), in the size mode the header names. Nothing is measured
again.

## Format

JSON Lines. The first line is the header; each line after it is one folder,
parents before children (depth first). A reader links a folder to its
parent by depth: the parent is the nearest earlier line one level up.

```json
{"type":"header","format":"pwrdisk-usage-report","version":1,
 "generator":"pwrdisk 0.24.0","captured_at":1790000000,
 "scan_started_at":1789999970,"scan_elapsed_ms":30000,
 "host":{"hostname":"…","machine_id":null},"root":"/","path_style":"posix",
 "volume":{"device":"/dev/disk1s1","mount":"/",
           "space_before":{"total":…,"free":…,"available":…,
                           "native":{"used":…,"available":…}},
           "space_after":{…}},
 "units":"bytes","threshold_bytes":100000000,"size_mode":"allocated",
 "options":{"include_hidden":true,"follow_links":false,
            "one_filesystem":true,"dedup_hardlinks":true,"max_depth":null,
            "apfs_clone_metadata":true},
 "completeness":{"complete":false,"cancelled":false,"unreadable":12,
                 "unreadable_detail":["…"],"excluded":["/System/Volumes/VM","…"]},
 "sharing":{"collected":true,"note":"…"},
 "totals":{"bytes":…,"files":…,"dirs":…,"records":2000}}
{"type":"dir","path":"/","depth":0,"bytes":…,"files":…,"dirs":…,
 "direct_bytes":…,"direct_files":…,"exclusive_bytes":…,"modified":…}
```

Shown wrapped here; each record is one line in the file.

| Folder field | Meaning |
| --- | --- |
| `bytes`, `files`, `dirs` | Everything at or below, in `size_mode` |
| `direct_bytes`, `direct_files` | Files, links and other entries directly in the folder |
| `exclusive_bytes` | `bytes` less every kept subfolder's: direct entries and subfolders under the threshold. These never overlap and add up to the root's `bytes` |
| `unreadable` | Something at or below could not be read; `bytes` is a floor |
| `residual_unreadable` | Something *not* kept as its own folder may be unreadable here, so a path missing below it may have been hidden rather than small |
| `modified` | Newest write time at or below, Unix seconds |
| `sharing` | APFS clone figures, only where clone metadata was found below: `bytes` in clone files, `private_bytes`, `duplicate_bytes`, `unknown_files`, the removal estimate `reclaim_lower`/`reclaim_upper` and whether it is `partial` |

Header notes:

* `size_mode` is `allocated` (blocks the volume spends) or `apparent`
  (length). Allocation is listed per file, so on APFS the sum can exceed
  what the volume holds. `sharing.collected` says whether clone metadata
  was read at all; without it a folder with no `sharing` may still share
  blocks.
* `completeness.unreadable` counts every error; `unreadable_detail` holds
  the first 50. `excluded` lists mounts below the root the scan does not
  enter and automount points it never touches. Aliases of folders the
  walk reaches another way (macOS firmlinks, repeated bind mounts) are not
  listed: their files are counted once, where the walk found them.
* `space_before` and `space_after` are `statvfs` (and, on macOS, the
  system's "important" capacity in `native`) around the walk.
* The reader refuses a file whose line count differs from
  `totals.records`, whose lines are out of order, or whose `version` is
  newer than it knows: a missing line would otherwise read as a folder
  that went away.

## What a comparison says

`report-diff BEFORE AFTER` compares two reports of the same tree.

**Where it changed.** Every folder kept in *both* reports gets its subtree
less its subfolders kept in both. Those parts never overlap, and their
changes add up exactly to the change of the whole scope, whatever either
threshold left out. So growth is placed in the deepest folder both reports
can see, and is never counted again under each ancestor. The folder's
subtree change is shown beside it.

**Kept in one report only.** Absent from a report means "under that
report's threshold, or not there", and a comparison says exactly that. It
never says deleted, and never zero. The size in the other report is
*unknown*, not bounded, when the folder that would hold it had something
unreadable outside its kept subfolders, the scan was cancelled, or the
path is below a depth limit. Only the topmost such folder of a branch is
printed; `--json` lists every one.

**Thresholds.** With different thresholds, folders under the larger one
are not listed as appearing or going, since one report would not have
kept them; the own-part totals still add up.

**Roots.** If one root is inside the other, only the inner one is
compared, and it must be kept in the wider report.

**Measured differently.** Different size modes, path styles, hidden-entry,
link, filesystem, hardlink or depth settings make the figures measure
different things, and `report-diff` stops unless given
`--allow-incompatible`. Different machines, devices and thresholds, a
missing clone-metadata pass and unreadable entries are warnings.

**What it is not.** A change in listed allocation is not a change in what
the volume holds: clones share blocks, snapshots keep blocks no listing
shows, and other volumes in an APFS container draw on the same free space.
The free-space line beside the totals is `statvfs` after each scan; the
difference between it and the listed change is exactly what a listing
cannot see. Nor is a change write history: a folder that grew 1 GB may
have had 10 GB written and 9 GB removed.

## Known limits

* The tree merges a folder's own read errors with its children's. An entry
  a folder itself failed to read is caught in `residual_unreadable` unless
  a kept subfolder of it also had errors; then only `unreadable` and the
  header's count say so.
* Reports scan this computer only; SSH hosts are not supported yet.
