# Invariant details

<!-- Moved verbatim from AGENTS.md, which keeps a summary and says when
     to read this. -->

The numbered invariants in [AGENTS.md](../../AGENTS.md#invariants) state
each rule; these are the parts of them that matter only when working in
that area.

## 1. How each platform measures

From invariant 1, *sizes come from `st_blocks * 512` unless apparent size
was asked for*:

On macOS the directory listing reports every fork's allocation and the data
fork's length in bulk; see `macos.rs`. On Windows it is the allocation the
directory listing reports; see `windows.rs`. An elevated scan of a whole NTFS
drive reads it from the master file table instead, keeping the walk's rules
for hidden entries, links, cloud folders and depth; see `mft.rs`. That path
counts every stream's allocation, alternate data streams included since they
go when the file goes, so for a file with alternate streams it can exceed the
walk's number.

## 12. Cache cleanup

From invariant 12, *the user confirms removal*:

Cache cleanup is for caches PwrDisk positively recognizes *and* can
clean better than by deleting them: an owner with a command here.
Nothing is held back for only looking like a cache (a name, a
`CACHEDIR.TAG`, a hatch, an owner without a command); that is an
ordinary removal. `cleanup/recognize.rs` holds the rule.
A cache cleanup is decided per marked path (Skip by default, the
owner's exact command, or Delete files); owner approval never approves
deleting files, which has its own consent, and a failed owner command
never falls back to it. See `docs/owner-cleanup.md`.

## 14. Serving skills through the MCP Skills extension

From invariant 14, *the skills are the agent's safety brief*:

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

15. **A tree owns a handful of buffers, by index.** Nodes live in one
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
