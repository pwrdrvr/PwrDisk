# Cargo output inventory over MCP

`pwrdisk_cargo_outputs` is a read-only query over completed scans. It gives
agents Cargo identity evidence and exact timestamps without asking them to
repeat a filesystem walk. It never launches a window, changes the scan root,
moves the view, transfers marks or approves removal. A linked window remains
the source of truth; an unavailable or changing linked tree is an error.

For example, after scanning a contrived workspace:

```json
{
  "path": "/invented/workspaces",
  "min_bytes": 1073741824,
  "older_than_days": 30,
  "order": "oldest",
  "limit": 25,
  "offset": 0
}
```

Try large, older output first. The default order is oldest first, breaking
age ties by largest listed size, then path. `order: "largest"` reverses the
priority of size and age. Both orders put unknown ages last. `next_offset`
paginates the matching set; counts and bytes describe all matches before
pagination. Remove the age filter if older output cannot meet the goal, and
explain the broader age range before proposing recent output. Generic
`pwrdisk_find` remains largest first and retains its existing age filter.

`last_modified` is the newest **file mtime beneath the root in the scan**, in
Unix seconds. It is not the root directory's mtime, access time or proof of
inactivity. Automated builds can write recently without active human work;
reading or running an old executable need not update its mtime. Unknown ages
are null and fail `older_than_days`. `scanned_at` and `evidence_checked_at`
distinguish the completed measurement from the live metadata check.

A directory name, manifest, layout or cache tag alone does not establish
Cargo identity. Discovery finds `.rustc_info.json` and `.fingerprint` in the
scanned tree, including roots with custom names, custom profiles and target
triples. A returned root needs:

- Parsed rustc cache metadata or a valid `CACHEDIR.TAG` signature.
- Cargo fingerprint JSON contents inside a crate/hash directory.
- A matching profile's dependency directory.
- No Git-tracked contents when inside a Git worktree. An unavailable or
  timed-out tracking check excludes the root.

For a two-level cross-target profile, the root must have its own compiler
metadata. A generic tag on an enclosing workspace cannot absorb a nested
Cargo output directory and count its source files as output.

The query never runs Cargo or a cleanup command. Git uses a read-only
pathspec check with optional writes and the fsmonitor hook disabled. Linked
Git, VM, FSEvents and Spotlight ownership, unreadable roots, and roots removed
since the scan are excluded. Current removal/owner-cleanup rules still apply.

Only confirmed parents cover nested roots. An unverified parent cannot hide
a confirmed child. `outputs` contains non-overlapping roots; byte values are
listed allocation, or logical bytes when `apparent_size` is true. To assess a
selection, pass all its paths to `pwrdisk_estimate` and check volume retention.
Never add independent APFS reclaim estimates or promise listed bytes as
immediate recovered space.

Coverage is explicit, including scans with zero matching outputs: scan roots
and times, hidden/depth/mount settings where available, unreadable counts,
discovery and inspection truncation, and excluded roots with reasons. Linked
peers do not currently report depth/mount options; these remain unknown.
Only the largest 25 exclusions are shown, with the full exclusion count.
Exclusion sizes can overlap each other and confirmed roots; do not sum them.

Work is bounded to 4,096 results per marker search, 512 candidate roots,
16 profiles per root, 64 fingerprint crate entries and 16 files per crate,
256 KiB per metadata file, two seconds per Git check and a 30-second
inspection budget. At most 200 confirmed roots are returned per page. Unknown
metadata formats, empty remnants, output with separated final artifacts,
changes during inspection and bounds can miss genuine Cargo output. Narrow
the query when discovery or inspection truncates. Do not describe an
exclusion or an unreadable area as known to contain no Cargo output.

Positive evidence establishes Cargo output presence. It does not prove that
every untracked file is generated, that the directory is inactive, or that
its contents are safe to lose. Identity, prioritization, estimated recovery
and the user's removal authorization remain separate decisions.
