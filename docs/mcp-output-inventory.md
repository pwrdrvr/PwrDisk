# Typed output and temporary-data inventory over MCP

`pwrdisk_find_outputs` searches completed scans for build output, caches,
installed dependencies and descriptive temporary/scratch/log/trash locations.
Omit type filters to discover all supported kinds. Each result includes its
`output_type`, `family`, evidence strength, assessment, exact newest file-write
time, bytes and scan limitations. Identity and cleanup suitability remain
separate; no result authorizes removal.

For example, find older, large candidates across all types:

```json
{
  "path": "/invented/workspaces",
  "min_bytes": 1073741824,
  "older_than_days": 30,
  "order": "oldest",
  "limit": 25
}
```

Use `types: ["cargo"]` for Cargo output, `types: ["scala"]` for Scala output,
or `family: "jvm"` for the JVM family. `types: ["jvm"]` is also an umbrella
selector for that family. Multiple types use OR; family, category, name,
evidence, size and age filters narrow together with AND.

| Family | Types / selectors |
| --- | --- |
| rust | cargo, cargo-registry; rust selects the family |
| jvm | scala, gradle, maven, coursier, other recognized jvm layouts; jvm selects the family |
| javascript | node-modules, frontend-output; javascript selects the family |
| python, go, dotnet | respective packages/cache/intermediates |
| native | compiler-cache, cmake |
| infrastructure | terraform |
| packages | package-cache (the existing cross-language store classification) |
| location | temporary, agent-scratch, cache-location, trash |
| logs | diagnostic-logs |

Scala output is a subtype within JVM: `target/scala-*` is distinct from Gradle
caches, Maven metadata/repositories and Coursier caches. Existing classifiers
identify these layouts; the query does not invent a language from an arbitrary
`target` name. Worktrees, Git history, VM data and protected macOS stores are
not output candidates. Their protected ownership still wins in narrow scans.

## Evidence and confidence

`evidence.strength` distinguishes `name`, `layout` and `metadata`. Request a
minimum with `minimum_evidence`. A descriptive location or distinctive name
can be a lookalike; it does not assert a known owner. Name/layout evidence is
not metadata verification. `metadata_capable_types` currently names `cargo`;
a metadata-only JVM query does not establish that JVM output is absent.

Cargo is one internal detector behind the generic query. It discovers custom
output/build directories and profiles from scanned markers, then checks:

- Parsed compiler-cache metadata or a valid cache-tag signature.
- Cargo fingerprint JSON in a crate/hash directory.
- A matching dependency directory.
- No Git-tracked contents; unavailable tracking checks exclude the candidate.

Two-level cross-target profiles require compiler metadata at their own root.
A generic tag on an enclosing workspace cannot absorb its source files into
Cargo output. A directory name, manifest, layout or tag alone is insufficient.
Cargo retains this metadata requirement even when the minimum evidence is
`name`. Other types report their actual name/layout evidence. The query runs
no Cargo/build/cleanup commands. Git uses a read-only pathspec check with
optional writes and the fsmonitor hook disabled.

Sizes and ages are from the completed scan; identity checks have their own
`evidence_checked_at`. Evidence establishes output presence, not that every
untracked file is generated, that a build is inactive, or that contents are
safe to lose. Supported-owner cleanup routing and removal rules are unchanged.

## Ordering, filters and totals

The default order is oldest first, breaking ties by largest size, then path.
`order: "largest"` prioritizes size instead. Unknown ages sort last and fail
age filters. `last_modified` is the newest scanned **file mtime beneath the
root**, in Unix seconds. It is not directory mtime, last access or proof of
inactivity. Automated builds can write recently without active human work.

Try large older output first, then relax the age cutoff if insufficient for
the space goal. Keep the size floor to avoid many tiny directories distracting
from that goal. `name` is a case-insensitive root-name substring; `category`
is the existing display classification, independently of output type.

Known types remain visible beneath generic locations and other output types.
For example, a cache location can contain Cargo, Scala and Python subtrees.
`covered_by` identifies an enclosing match. `matching_listed_bytes` counts the
union once, before pagination. Per-type summaries can overlap across types:
never add them together or add row sizes blindly. Filtering occurs before
coverage and pagination, so a younger parent cannot hide an older typed child.
Use `next_offset` to fetch all matching rows; rescans and live changes can
alter subsequent pages. Unknown linked-window depth/mount settings remain
explicitly unavailable. Scan context is returned even with zero matches.

Standalone discovery without `path` searches the outermost retained scans
once; an explicit path uses the innermost covering scan. Each row retains
the measurements, timestamp, root and options of its originating snapshot.
A newer nested scan does not relabel an older outer-scan measurement. Scope
the query to use that nested snapshot, or rescan the enclosing root. Cargo
inference stops at the originating scan and any explicit scope; a narrow
scan never probes unscanned ancestors for output identity.

Use `pwrdisk_estimate` with the full path list and `pwrdisk_retention` for each
volume. Listed allocation includes shared/repeated APFS clone allocation and
is not guaranteed recovered space. Independent reclaim estimates are not
additive. Generic `pwrdisk_find` remains available for ordinary file/directory
searches, with its existing largest-first behavior.

## Bounds and the linked window

Discovery returns at most 4,096 output boundaries and 4,096 of each Cargo
marker. Cargo root listing and live inspection each cap at 512 candidates.
Cargo probes inspect at most 16 profiles, 64 crate entries and 16 files per
crate, with a 256 KiB file bound. Git has a two-second check deadline and the
inspection loop has a 30-second budget. At most 200 rows are returned per
page. Unknown metadata formats, empty remnants, separated final artifacts,
scan settings, unreadable paths and bounds can miss output. Truncation and
exclusions are explicit; narrow the query when needed. Only the largest 25
exclusions are shown with the full count, and their sizes can overlap.

A linked window remains the source of truth. Queries use State/Find/Ls through
a separate connection, with no Hello, mark transfer, navigation, window launch
or changes to removal intents. An unavailable, incompatible or changed linked
tree is an error, without falling back to stale local scans. Link protocol 10
prevents older peers from silently ignoring output-boundary discovery. No
on-screen window is needed for standalone stdio use.
