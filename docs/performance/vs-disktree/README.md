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
memory footprint. Every run is a fresh process.

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

Each tree was scanned three times per build, the order of the builds
rotated each round. Before a run the script waited for the machine to be
at least 85% idle with no other scan running, then watched for compilers
and other scans while it ran; a run that overlapped one was thrown away and
repeated (9 were). One run, marked below, still overlapped a build after
four tries. Both builds counted the same files, directories and bytes.

## The whole disk: 40.6 million files

`/`, as either app's "whole disk" scans it: 40,629,182 files, 7,239,967
directories, 2.41 TiB.

| build | scan | peak memory | free the tree |
| --- | --- | --- | --- |
| disktree | 134.4 / 132.2 / 132.0 s | 5.84 / 5.84 / 5.84 GiB | 1.209 / 1.141 / 1.112 s |
| PwrDisk, clone accounting off | 144.2 / 144.1 / 143.1 s | **4.22 / 4.30 / 4.22 GiB** | **0.030 / 0.000 / 0.032 s** |
| PwrDisk, default (clone accounting on) | 225.6 / 218.5 / 217.6 s | 5.60 / 5.62 / 5.58 GiB | **0.000 s** |

## Agent worktrees: 18.2 million files

A directory of agent git worktrees: 18,161,059 files, 2,814,467
directories, 755 GiB. Most files are APFS clones.

| build | scan | peak memory | free the tree |
| --- | --- | --- | --- |
| disktree | 56.0 / 55.4 / 56.0 s | 2.58 / 2.58 / 2.58 GiB | 0.514 / 0.538 / 0.529 s |
| PwrDisk, clone accounting off | 59.9\* / 60.3 / 59.4 s | **1.84 / 1.84 / 1.85 GiB** | **0.000 s** |
| PwrDisk, default (clone accounting on) | 99.0 / 100.4 / 98.6 s | 2.95 / 2.83 / 2.80 GiB | **0.000 s** |

\* overlapped another thread's `cargo` build on its fourth try.

## A medium tree: 372 thousand files

A folder of git checkouts: 371,941 files, 32,985 directories, 61 GiB.

| build | scan | peak memory |
| --- | --- | --- |
| disktree | 1.7 / 0.7 / 1.8 s | 73 / 73 / 73 MiB |
| PwrDisk, clone accounting off | 1.0 / 1.8 / 0.8 s | 92 / 88 / 88 MiB |
| PwrDisk, default | 1.3 / 2.2 / 2.3 s | 120 / 114 / 114 MiB |

Scans this short swing by a second either way; neither build is faster.

## What it shows

- **The tree itself costs 28–29% less.** With clone accounting off,
  PwrDisk peaks at 4.2 GiB against disktree's 5.8 GiB for the whole disk,
  and 1.84 against 2.58 GiB for the worktrees. PwrDisk holds a scan in a
  handful of index-addressed buffers instead of one heap allocation per
  node ([#68](https://github.com/pwrdrvr/PwrDisk/pull/68),
  [#73](https://github.com/pwrdrvr/PwrDisk/pull/73)). On the medium tree
  PwrDisk uses about 15 MiB more; the savings start somewhere between it
  and the large trees.
- **Freeing is instant.** disktree takes 1.1–1.2 s to free a whole-disk
  scan in an idle process, and does it on the main thread when the app
  quits. In the app, where a long-open scan has been compressed or paged
  out, that has stretched to 15 s with "Application Not Responding"
  ([#57](https://github.com/pwrdrvr/PwrDisk/pull/57)). PwrDisk's tree is
  eight buffers, freed in a few milliseconds at most, and Quit leaves it
  to the operating system.
- **APFS clone accounting costs time on macOS.** PwrDisk reads which bytes
  a clone shares and which are its own, so a marked clone is never
  credited with space that removing it would not return. disktree does not
  read clone sharing
  ([tobi/disktree#117](https://github.com/tobi/disktree/pull/117) proposes
  it), so it counts a clone's full size as reclaimable. That work makes a
  scan 1.65–1.8× as long as disktree's and gives back most of the memory
  saving: the whole disk still peaks 4% under disktree, the clone-heavy
  worktrees 9–14% over. It is on by default on macOS; Linux and Windows do
  not have it.
- **Without it, scans take about as long.** 8% longer on the whole disk,
  7% on the worktrees.
