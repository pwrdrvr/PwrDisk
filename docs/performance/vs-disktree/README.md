# PwrDisk and disktree, measured side by side

What the README's comparison table rests on. Measured 2026-11-04 on an
Apple M5 Max (18 cores, 64 GiB, APFS), PwrDisk `fce13fb` (v0.17.0) against
disktree `9b18b27`, the commit PwrDisk forked from and still disktree's
`main`.

## How

One small program, compiled against each tree's `disktree-core`, scans a
path with `ScanOptions::default()`, then times dropping the tree. It is the
same source in both trees except for reading the root (`&tree` in disktree,
`tree.root()` in PwrDisk). `/usr/bin/time -l` gives the process's peak
memory footprint. Runs alternate between the two builds, each in a fresh
process.

```rust
use std::time::Instant;
fn main() -> std::io::Result<()> {
    let root = std::path::PathBuf::from(std::env::args_os().nth(1).unwrap());
    let mut options = disktree_core::scan::ScanOptions::default();
    // PwrDisk only: BENCH_NOCLONE=1 turns off APFS clone accounting.
    if std::env::var_os("BENCH_NOCLONE").is_some() {
        options.apfs_clone_metadata = false;
    }
    let t = Instant::now();
    let tree = disktree_core::scan::scan(&root, options)?;
    let scan = t.elapsed().as_secs_f64();
    let t = Instant::now();
    drop(tree);
    println!("scan_s={scan:.2} free_s={:.3}", t.elapsed().as_secs_f64());
    Ok(())
}
```

The machine was busy throughout (load average 18 to 42, other scans
running), so scan times are noisy. Peak memory and the free time hardly move
with load. Both builds counted the same files, directories and bytes on
every run.

## A large tree: 18.2 million files

A directory of agent git worktrees: 18,161,059 files, 2,814,523
directories, 755 GiB. Most files are APFS clones.

| build | scan | peak memory | free the tree |
| --- | --- | --- | --- |
| disktree | 60.4 / 64.1 / 56.2 s | 2.58 / 2.57 / 2.58 GiB | 0.569 / 0.597 / 0.580 s |
| PwrDisk, clone accounting off | 63.4 / 62.4 / 89.2 s | **1.84 / 1.84 / 1.85 GiB** | **0.000 s** |
| PwrDisk, default (clone accounting on) | 110.1 / 102.6 / 98.9 s | 2.82 / 2.91 / 3.01 GiB | **0.000 s** |

- **The tree itself costs 29% less.** PwrDisk holds a scan in a handful of
  index-addressed buffers instead of one heap allocation per node
  ([#68](https://github.com/pwrdrvr/PwrDisk/pull/68),
  [#73](https://github.com/pwrdrvr/PwrDisk/pull/73)).
- **APFS clone accounting costs extra on macOS.** PwrDisk reads which bytes
  a clone shares and which are its own, so a marked clone is never credited
  with space that removing it would not return. disktree does not read
  clone sharing
  ([tobi/disktree#117](https://github.com/tobi/disktree/pull/117) proposes
  it), so it counts a clone's full size as reclaimable. On this tree, where most
  files are clones, the accounting adds about 1 GiB and 40 seconds. It is
  on by default on macOS; Linux and Windows do not have it.
- **Freeing is instant.** Dropping 18 million nodes takes disktree about
  0.6 s in an idle process, and disktree does it on the main thread when
  the app quits; under memory pressure that has stretched to 15 s with
  "Application Not Responding"
  ([#57](https://github.com/pwrdrvr/PwrDisk/pull/57)). PwrDisk's tree is
  eight buffers, freed in under a millisecond, and Quit leaves it to the
  operating system.

## A medium tree: 372 thousand files

A folder of git checkouts: 371,891 files, 32,973 directories, 61 GiB.

| build | scan | peak memory |
| --- | --- | --- |
| disktree | 1.9 / 0.8 / 0.9 s | 72 / 73 / 72 MiB |
| PwrDisk, clone accounting off | 1.8 / 0.9 / 0.9 s | 91 / 91 / 89 MiB |
| PwrDisk, default | 1.4 / 1.0 / 1.1 s | 116 / 116 / 116 MiB |

At this size both scan in about a second and PwrDisk uses more memory, by
about 20 MiB even without clone accounting. Where that fixed cost comes
from has not been profiled; the savings show somewhere between this tree
and the large one.
