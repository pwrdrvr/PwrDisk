# PwrDisk and disktree, measured side by side

What the README's comparison table rests on. Measured 2026-11-05 on an
Apple M5 Max (18 cores, 64 GiB, APFS): PwrDisk `bf7c6b3` (v0.18.0) and
`fce13fb` (v0.17.0) against disktree `9b18b27`, the commit PwrDisk forked
from and still disktree's `main`.

## How

One small program, compiled against each tree's `disktree-core`, scans a
path with `ScanOptions::default()`, then times dropping the tree. It is the
same source in both trees except for reading the root (`&tree` in disktree,
`tree.root()` in PwrDisk). `/usr/bin/time -l` gives the process's peak
memory footprint and the CPU time spent in the program (user) and in
the kernel on its behalf (system). Every run is a fresh process.

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
rotated each round, on a machine left as quiet as it gets. A run waited for
any other scan to finish, and the script logged compilers and other scans
that overlapped it; those runs are kept and marked. The disk is in use, so
file counts drift by a few thousand between runs; the builds are compared
only within this one session.

## The whole disk: 34 million files

`/`, as either app's "whole disk" scans it: about 33.6 million files, 6.2
million directories, 2.14 TiB.

| build | scan | peak memory | free the tree | CPU, user / system |
| --- | --- | --- | --- | --- |
| disktree | 113.5 / 114.0 / 134.6† s | 4.85 / 4.85 / 4.86 GiB | 1.020 / 1.079 / 1.057 s | 32 / 1,258–1,288 s |
| PwrDisk 0.18, clone accounting off | 118.0 / 116.0 / 137.8† s | **3.61 / 3.62 / 3.62 GiB** | **0.000 s** | 37 / 870–933 s |
| PwrDisk 0.18, default (clone accounting on) | 154.5 / 202.7‡ / 135.0† s | 4.34 / 4.34 / 4.23 GiB | **0.000 s** | 49–52 / 1,013–1,073 s |
| PwrDisk 0.17, default | 192.9 / 200.6 / 200.1† s | 4.63 / 4.48 / 4.46 GiB | 0.000–0.031 s | 60–63 / 1,747–1,915 s |

† the last round, which ran slower for every build: disktree took 18%
longer than in its first two runs.
‡ about 20,000 files were written to the disk while it ran.

## Agent worktrees: 11 million files

A directory of agent git worktrees: 10,842,774 files, 1,638,475
directories, 465 GiB. Most files are APFS clones.

| build | scan | peak memory | free the tree | CPU, user / system |
| --- | --- | --- | --- | --- |
| disktree | 31.5 / 31.5 / 33.9\* s | 1.55 / 1.55 / 1.55 GiB | 0.328 / 0.344 / 0.362 s | 9 / 348–355 s |
| PwrDisk 0.18, clone accounting off | 31.7 / 46.6 / 48.8 s | **1.14 / 1.14 / 1.14 GiB** | **0.000 s** | 10–11 / 241–282 s |
| PwrDisk 0.18, default (clone accounting on) | 37.6 / 38.9 / 44.5 s | 1.51 / 1.50 / 1.53 GiB | **0.000 s** | 15–16 / 282–304 s |
| PwrDisk 0.17, default | 56.5 / 66.4 / 60.0\* s | 1.92 / 1.89 / 1.87 GiB | **0.000 s** | 18–19 / 568–580 s |

\* overlapped another thread's `cargo` build. Nothing was logged against
the two slow clone-off runs.

## A medium tree: 377 thousand files

A folder of git checkouts: about 377,000 files, 33,000 directories, 63 GiB.

| build | scan | peak memory |
| --- | --- | --- |
| disktree | 1.8 / 0.8 / 1.8 s | 76 / 76 / 74 MiB |
| PwrDisk 0.18, clone accounting off | 1.9 / 0.8 / 0.8 s | 98 / 99 / 99 MiB |
| PwrDisk 0.18, default | 1.3 / 2.1 / 1.3 s | 129 / 126 / 121 MiB |
| PwrDisk 0.17, default | 1.6 / 1.0 / 1.0 s | 126 / 126 / 128 MiB |

Scans this short swing by a second either way; neither build is faster.

## What it shows

- **The tree itself costs about 25% less.** With clone accounting off,
  PwrDisk peaks at 3.6 GiB against disktree's 4.85 GiB for the whole disk,
  and 1.14 against 1.55 GiB for the worktrees. PwrDisk holds a scan in a
  handful of index-addressed buffers instead of one heap allocation per
  node ([#68](https://github.com/pwrdrvr/PwrDisk/pull/68),
  [#73](https://github.com/pwrdrvr/PwrDisk/pull/73)). On the medium tree
  PwrDisk uses about 25 MiB more; the savings start somewhere between it
  and the large trees.
- **Freeing is instant.** disktree takes about 1 s to free a whole-disk
  scan in an idle process, and does it on the main thread when the app
  quits. In the app, where a long-open scan has been compressed or paged
  out, that has stretched to 15 s with "Application Not Responding"
  ([#57](https://github.com/pwrdrvr/PwrDisk/pull/57)). PwrDisk's tree is
  eight buffers, freed in a few milliseconds at most, and Quit leaves it
  to the operating system.
- **Without clone accounting, scans take about as long.** 2–4% longer
  than disktree's on the whole disk, in each round. PwrDisk spends a few
  more seconds of its own CPU and about 30% less time in the kernel.
- **APFS clone accounting costs time on macOS, less than it did.**
  PwrDisk reads which bytes a clone shares and which are its own, so a
  marked clone is never credited with space that removing it would not
  return. disktree does not read clone sharing
  ([tobi/disktree#117](https://github.com/tobi/disktree/pull/117) proposes
  it), so it counts a clone's full size as reclaimable. In v0.18 that work
  makes a whole-disk scan up to 1.35× as long as disktree's (one round
  ended level with it) and the worktrees 1.2–1.3×, against 1.65–1.8× in
  v0.17 ([#112](https://github.com/pwrdrvr/PwrDisk/pull/112),
  [#116](https://github.com/pwrdrvr/PwrDisk/pull/116),
  [#118](https://github.com/pwrdrvr/PwrDisk/pull/118)). It now peaks
  11–13% under disktree on the whole disk and level with it on the
  worktrees. It is on by default on macOS; Linux and Windows do not have
  it.
