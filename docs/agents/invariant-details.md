# PwrDisk invariants

Read the relevant sections before changing these behaviors. See
[AGENTS.md](../../AGENTS.md#invariants) for links by area.

## 1. How each platform measures

**Sizes come from `st_blocks * 512` unless apparent size was asked for.**
APFS clones can share those blocks; private-byte estimates are reported
separately without changing tile weights.

On macOS the directory listing reports every fork's allocation and the data
fork's length in bulk; see `macos.rs`. On Windows it is the allocation the
directory listing reports; see `windows.rs`. An elevated scan of a whole NTFS
drive reads it from the master file table instead, keeping the walk's rules
for hidden entries, links, cloud folders and depth; see `mft.rs`. That path
counts every stream's allocation, alternate data streams included since they
go when the file goes, so for a file with alternate streams it can exceed the
walk's number.

## 2. Derived totals

**Totals are derived, never tracked.** A leaf's `bytes` is the only
input: `tree::aggregate` computes every directory's `bytes`, `files` and
`dirs` from its children, and `own_bytes()`/`own_files()` read the direct
children when asked. Hardlink de-duplication zeroes a duplicate leaf's
`bytes` while that pass runs; anything that patches a directory's `bytes`
directly will be overwritten. `Node` is paid for tens of millions of
times, so `a_node_stays_small` in `index.rs` holds its size.

## 3. Directory completion

**A directory is only built when its own scan *and* every subdirectory task
has finished.** That is the `+1` sentinel in `PendingDir::pending`. Building
early silently drops whole subtrees — it has happened once.

## 4. Protected paths

**Only paths under the scanned root may be removed**, and mount points, the
root, the home directory, any directory holding it, and symlink targets are
refused. So are the profiles folder and every directory directly in it:
on Windows the one Windows reports (`FOLDERID_UserProfiles`, usually
`C:\Users`), on macOS `/Users`, on Linux `/home` (and its real path
when it is a link). What is inside a profile stays
removable. The window asks the guards before a mark is made
(`removal::refusal`), so a refused path is never offered for marking.
Shared system containers (including `/Applications`) are refused as a
whole while eligible children remain removable; see
[the protected-path policy](../protected-paths.md) when changing guards.

## 5. Mark identity

**Marks are keyed by absolute path**, not tree position, so they survive a
re-scan; `Marks::refresh` re-reads their sizes and keeps missing paths
marked at zero bytes, so they cannot claim space that is already gone.

## 6. Treemap painting

**The treemap is painted, not composed of elements.** Thousands of
rectangles belong in one canvas callback; labels are shaped there too so they
clip to their own tile.

## 7. Tile crumbs

**Tile crumbs are absolute.** `treemap::layout` takes the drawn node's
crumbs and every tile extends them, so a tile resolves from the scanned
root at any depth. Relative crumbs look right at `~` and silently point
at other directories after descending — including for marks. Any code that
turns a path into crumbs walks from the scanned root, too.

## 8. Zoom

**The view transform is the only thing zoom changes.** Layout runs in
base-space pixels and is cached; `screen = (base - origin) * scale`.

## 9. Volume attribution

**The status bar never claims a saving it cannot measure.** Projections come
from marked bytes *on the volume the meter measures*, which is the scan's own
notion of a volume — the mount *source* — so a mark on a disk mounted inside
the tree, which `-X` and the media exception allow, is reported and never
added to this volume's free space; see `space::attribution`. The final number
comes from `statvfs` before and after.

## 10. Discovery and SSH

**Discovery is not a scan.** No implicit home walk at startup. List
mounts in the selected machine's namespace without probing them; scans
stop at nested mounts, including bind mounts and same-device subvolumes.
Switching machines clears marks and identity. The Unix SSH helper streams
records and never retains the tree. SSH hosts are Unix, so remote paths
are POSIX on every client: join, split and compare them through
`PathStyle`, never `std::path`, which uses `\` on Windows.

## 11. Live overview

**A live overview is not an actionable tree.** Keep provisional,
bounded display summaries separate from `Disktree::tree` and all path,
mark and removal APIs. Only a complete scan may replace the real tree.
Counters and sampled paths belong in fixed, monospace progress lanes.

## 12. Removal and confirmation

**The user confirms removal.** Marking never authorizes deleting.
`review` only opens the review screen; agents must never click its
confirmation or press its approval keys through computer use. MCP removal
is a destructive write: prepare an expiring, single-use preview, show the
exact paths and mode and explain recoverability, then obtain explicit
confirmation for that operation. An expired or changed preview requires
new confirmation. The app validates the intent and rechecks existing
guards and its configured capacity limit for both UI and MCP removals,
including a fresh measurement on the worker. The default is 5% per volume.
Past the limit only the person can go: in the window, through Mark
anyway and then Review's second confirmation (a typed phrase by
default), which names each disk past the limit, with the worker
holding each of those disks to its confirmed share and every other
disk to the limit. An agent's mark past the limit is refused, and
agents keep the default limit when the window's is Off, on the
worker as at the preview: nothing the person confirms in Review
raises an agent's removal. `mark_guard.rs` holds the rule.
Trash never falls back to permanent deletion. No headless or SSH removal.

A linked window's tree and marks remain the source of truth; `ls` and
`find` read it without navigating. Only an explicit `show` request moves
the view, and guidance requires the user to request that behavior.
`scan` explicitly changes the scan root; linking never does so implicitly.

### 12. Cache cleanup

Cache cleanup is for caches PwrDisk positively recognizes *and* can
clean better than by deleting them: an owner with a command here.
Nothing is held back for only looking like a cache (a name, a
`CACHEDIR.TAG`, a hatch, an owner without a command); that is an
ordinary removal. `cleanup/recognize.rs` holds the rule.
A cache cleanup is decided per marked path (Skip by default, the
owner's exact command, or Delete files); owner approval never approves
deleting files, which has its own consent, and a failed owner command
never falls back to it. See [owner cleanup](../owner-cleanup.md).

## 13. Local link transport

**The link is a same-user socket or pipe, nothing more.** `disktree-link`
binds `0600` in a per-user directory on Unix. On Windows it uses a
named pipe keyed by the user's SID, with a protected DACL granting only
that user access and `PIPE_REJECT_REMOTE_CLIENTS`. Before exchanging
messages the client verifies the connected pipe's owner is that SID.
It has no token: the agent already runs as the user. The separate MCP
stream uses the same protections; Unix also verifies peer credentials.
Do not add a port.
A window showing an SSH host answers only `hello` and `state`: an
agent's paths are this computer's.

`pwrdisk mcp` relays stdio to an already running desktop. The desktop owns
the MCP surface and execution. The relay must never parse MCP or launch
the app. `pwrdisk mcp --standalone` owns its scans and marks and never
links to or launches a window.

## 14. Agent skills

**The skills are the agent's safety brief.** `skills/` is compiled
into the desktop and standalone server (`disktree-mcp/src/skills.rs`
lists every file; a URI is looked up, never turned into a path). They tell the agent
to agree the list before marking unless the user gave a standing
instruction, and never to delete by other means. A change to what
the guards refuse, what removal does, or what a system hides from a
scan updates the matching `references/<system>.md` in the same PR.

### 14. Serving skills through the MCP Skills extension

They follow the MCP Skills extension (`io.modelcontextprotocol/skills`,
[spec](https://modelcontextprotocol.io/extensions/skills/overview)):
declare it with `resources` in the capabilities, answer `skills/list`
and `skills/get` with each skill's frontmatter and every file's
`sha256:` digest and byte size, add `resultType`, `ttlMs` and
`cacheScope` to both (rmcp passes custom results through as they
are), and serve the bytes through `resources/read`. Plain resources
and `pwrdisk_read_skill` remain for clients without the extension;
`skills_host_tests.rs` walks the host's sequence.

## 15. A tree's buffers

**A tree owns a handful of buffers, by index.** Nodes live in one
vector, each directory's children a contiguous run of it, names in one
string and clone records in side tables, so dropping a tree frees eight
buffers instead of tens of millions of allocations. An allocation record
keeps only what its node does not say (`index/records.rs`): a clone with
one name reads its size from the node, so a placed leaf's `bytes` and
`inode` change only by hardlink de-duplication. A record is never
changed in place, which is what lets the files of a clone group share
one; a change that writes to a record must stop that. A node is read
through a `Node<'_>` cursor from its tree and written through `NodeMut`
or a whole-tree pass. Build with `Tree::build` (a `Draft` for tests and
small trees); a subtree moves to another tree only as a copy. A
finished tree keeps every subtree one contiguous run, which is what
lets `par_bottom_up` split it across workers without `unsafe`: only
the order *within* a block may change after it is built.
