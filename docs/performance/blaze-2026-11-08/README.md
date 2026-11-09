# Blaze recheck — November 8, 2026

**Verdict: keep Cargo; do not adopt Blaze for PwrDisk yet.** With enough
disposable disk, the published canary builds our helper and passes basic
runtime checks. It was slower in the measured helper workloads and added
substantial artifact storage. Real APFS sharing was observed, but no net
disk saving was established. No product, Cargo, toolchain, CI, release or
global settings changed.

This resolves the canary's previously inconclusive disk-limited result in
the [November 7 report](https://github.com/pwrdrvr/PwrDisk/blob/bcc4fe7bbb406c621e232e7caa24fd089d202bc4/docs/performance/blaze-2026-11-07/README.md).
It does not attribute the result to an upstream fix: published versions
were unchanged. The earlier stable compiler failure was not rerun.

## Inputs and limits

The thread moved to a fresh isolated worktree on `origin/main`, commit
`bed9b0c6f5a07a0e5586106f26758b04e18ec84d` (PwrDisk 0.22.0). The helper's
source is unchanged from the earlier evaluation, but core now depends on
`tempfile` in production and the lockfile changed. This is not a same-source
speed comparison with November 7.

The host is macOS 27.2 (26B123), arm64, on APFS. About 131 GiB was free before setup,
versus about 30 GiB previously. The helper probe allowed 600 seconds,
32 GiB of summed allocated-byte reports and a 90 GiB available-space floor.
Later individual build calls had 300-second timeouts. Disk checks were
sampled watchdogs, not filesystem quotas; all recorded inventories remained
below the allowance. Blaze used a private daemon, `BLAZE_CORES=2` and
`BLAZE_DAEMON_MEMORY_GB=4`. Cargo used two jobs. No other owner's target,
cache, worktree or daemon was used or cleaned.

Release metadata was fetched at 2026-11-09 03:02 UTC (November 8 locally):

| Input | Pinned value |
| --- | --- |
| PwrDisk Rust | 1.97.1, `8bab26f4f68e0e26f0bb7960be334d5b520ea452` |
| Cargo | 1.97.1, `c980f4866` |
| Latest stable | `0.20261027.0`, used only to fetch the private toolchain |
| Latest canary tested | `0.20261105.4`, commit `e93029602544df9882fffb18bf272d048a1a7136` |
| Blaze toolchain | nightly-2026-10-27, downloaded inside this worktree |
| Current `main` inspected | `0a320c71110b360add318b05a807deb5a0f69c9c`; not built |

Sources: [stable release](https://github.com/indexable-inc/blaze/releases/tag/v0.20261027.0),
[canary release](https://github.com/indexable-inc/blaze/releases/tag/v0.20261105.4),
[current source](https://github.com/indexable-inc/blaze/tree/0a320c71110b360add318b05a807deb5a0f69c9c).

**Compiler equivalence is impossible with this release.** Blaze ignores
the project's Rust pin and implements its own compiler. PwrDisk's unchanged
dev profile uses level 1 for workspace code and level 2 for dependencies;
the canary maps those to stencils and LLVM respectively. Cargo uses its
pinned rustc/LLVM implementation. These are observations of two development
tools, not an equivalent-compiler speedup experiment. No optimization,
features, debug information, overflow checks or test requirements were
weakened to produce a favorable result.
([canary toolchain](https://github.com/indexable-inc/blaze/blob/e93029602544df9882fffb18bf272d048a1a7136/docs/toolchain.md),
[backend selection](https://github.com/indexable-inc/blaze/blob/e93029602544df9882fffb18bf272d048a1a7136/docs/cargo.md#backends))

## Build and correctness results

Wall times use Python's monotonic clock around the command, excluding the
subsequent help-output check. Every successful measured build emitted an
executable which exited zero on `--help`. Timed comparisons ran serially,
except the explicitly concurrent pair. Both source files were restored and
both tools rebuilt the originals afterward. See [results.json](results.json)
for every sample, restoration, hash and snapshot.

| Workload | Cargo | Canary Blaze |
| --- | ---: | ---: |
| First helper build | 41.029 s | 142.914 s |
| Three unchanged builds | 0.203 / 0.481 / 0.313 s | 45.718 / 2.528 / 2.515 s |
| Same function-body edit | 1.845 s | 7.981 s |
| First build in second worktree | 36.240 s | 92.448 s |
| Concurrent worktree 1 / 2 edit builds | 0.783 / 0.681 s | 80.765 / 80.870 s |

The cold row is deliberately **not a speed ratio**. Cargo reused existing
registry sources offline; its build target was empty. Blaze fetched its
own sources and compiled its standard library. Its separate nightly setup
took 6.097 seconds before the 142.914-second build. There is no reliable
download/compile/link duration partition for that cold build. OS file
caches and background machine activity were not controlled.

The first unchanged Blaze request transitioned from the online,
uninstrumented initial command to `--offline` with `BLAZE_STATS=1`; it
recompiled several units. It is shown rather than discarded. The next two
were unchanged in flags, environment and source. Their internal `Finished`
times were 2.35 and 1.88 seconds, versus client walls of 2.528 and 2.515.
The captured timeline exposes script, names, bodies, codegen, store and link
durations, not a trustworthy aggregate hit/miss count. Status reported zero
requests despite the completed builds, so that counter was not interpreted
as cache activity. Stage durations may overlap; do not sum them into a
wall-time partition.

The edit adds ` (benchmark edit)` to the helper's help text in `main`.
The output oracle required exactly that change, then exactly the original
output after restoration. The concurrent check gave each real Git worktree
a different lane marker and used separate targets with the same private
Blaze daemon/store. Both tools returned each lane's own expected output.
This is one successful concurrency check, not exhaustive race coverage.

The canary also completed the real `--stdio-v1` scan path on an owned
one-file fixture: the returned data record said five bytes, exit was zero
and the file still contained `hello`. Only contrived entry records were
retained; headers containing host space/identity were not published. No
application window, local application link or real user scan was used.

## Storage and APFS observations

The second checkout itself adds about 8.4 MB of source content. Git's shared
repository objects are not another source clone. Build targets and caches
are counted separately, including duplicated final objects, intermediate
files, incremental state, package sources, archives and toolchains.

Across all experimental roots, the inventory after both worktrees and the
concurrency/restoration runs was **16,476,532,994 logical bytes** and
**16,636,153,856 allocated bytes reported by `st_blocks * 512`**. This
includes both tools and multiple edit versions, not just two final helpers.
After the lint/test gate, the final inventory was 17,255,019,232 logical /
17,418,600,448 reported allocated bytes, plus 53,248 allocated bytes of
new Git worktree metadata outside the artifact roots. Directory metadata
is not included in the file sums.

From the source-only second-worktree snapshot to its first Blaze build,
the combined roots grew by about 5.23 GB in both logical and reported
allocated bytes. The subsequent Cargo build added about 296 MB logical /
299 MB reported allocated. These are observed deltas for this setup, not
general cache efficiency estimates. Snapshots include small growing logs.

The final original-source helper executables were:

| Output | Logical size | APFS reported private bytes |
| --- | ---: | ---: |
| Cargo, worktree 1 | 1,056,192 | 0 |
| Blaze, worktree 1 | 172,048,432 | 0 |
| Blaze, worktree 2 | 172,114,480 | 0 |

A small native `getattrlist` probe requested file identity, data allocation,
clone ID and `ATTR_CMNEXT_PRIVATESIZE`, using the same packed-field layout
as PwrDisk's macOS implementation. It first passed an oracle on an owned
64 KiB file and an APFS clone: distinct inodes, matching clone IDs,
identical bytes and zero reported private bytes. For each Blaze executable
we then found a same-size content-store file with the same clone ID and
SHA-256. That directly demonstrates sharing with the store. The two
worktree executables had different clone IDs and contents; it does not
prove they share their whole allocation with each other. Cargo's final
helper also reported zero private bytes.

**Zero per-file private bytes does not mean zero total disk cost.** A
clone group's shared extents still occupy storage. `st_blocks` sums can
count them repeatedly, and inode deduplication does not deduplicate clones.
The full extent-sharing graph and total unique physical bytes were not
measured. Free-space changes are also confounded by other host activity.
This experiment demonstrates clone placement, not a net physical-space
reduction. The observed large toolchain/store/source footprint gives no
reason to adopt Blaze for disk savings on this helper workload.

The daemon's final self-report said 12.7 GiB memory; a single OS sample
showed 2,764,624 KiB RSS (about 2.64 GiB). Those are different measures and
neither is a peak of all build processes. The requested 4 GiB setting must
not be presented as a measured hard RSS ceiling. The private daemon was
stopped and status confirmed it was no longer running before disposal.

## Scope, privacy and validation

This establishes basic macOS arm64 helper compatibility for the canary.
It does not establish GUI, MCP, xtask, all-feature tests, release thin LTO,
universal macOS, Linux/musl or Windows compatibility. Those builds were not
multiplied after the measured helper showed no performance benefit.
The earlier platform and remote-cache cautions still apply. No global
Cargo build/profile/target sections or inherited build-control variables
were present in these comparisons.

All tools, caches, two worktrees, fixtures and temporary files belonged to
this workspace. Only public metadata, distribution, toolchain and crate
downloads were used. There was no remote upload/cache configuration,
credential provisioning, paid service, `blaze enable`, shell/PATH change,
provider operation, release or merge. The existing PwrGit sharing threads
were consulted before considering overlap; no sharing implementation or
Blaze integration was added.

The four existing Cargo helper stdio integration tests passed. The required
`cargo xtask lint` passed, including default and all-feature Clippy passes.
After recording the inventory and confirming both sources restored, only
the owned disposable targets, caches, tools and second worktree were
removed. Local evidence retained 1,460,509 logical / 1,683,456 allocated
bytes (about 1.7 MB), with small subsequent log-record growth. Available
space rose from 121,651,200,000 to 133,939,200,000 bytes during disposal;
this is not an exact unique-byte measurement. The clean assigned runtime
worktree and all pre-existing user data remain. No claim is made that the full workspace test suite ran through
Blaze. One local sandbox socket failure was excluded and rerun with the
same private daemon scope; it was not attributed to the compiler.

## Reproduction

Use a fresh worktree at the PwrDisk commit above. The exact local harnesses
are in [harness/](harness/); copy their `.txt` contents to the corresponding
filenames under a new, owned `target/blaze-recheck/` directory before
running them. They intentionally resolve the workspace relative to that
location. They are experiment transcripts, not supported build defaults.

Download and extract these release archives into `stable-bin/` and
`canary-bin/` under that directory, retaining their top-level directories:

| Archive | SHA-256 |
| --- | --- |
| `blaze-v0.20261027.0-aarch64-apple-darwin.tar.gz` | `47dae8e838765931ed8e216a94e4d75454469fa6c3950d1c02eba2841bc8dad5` |
| `blaze-v0.20261105.4-aarch64-apple-darwin.tar.gz` | `27da522a507cba1716ecbfbf91fc99546956b6a6f9da21c608539bb70c0da45b` |

The download URL is
`https://github.com/indexable-inc/blaze/releases/download/v<VERSION>/<ARCHIVE>`.
Check each digest against the release's `SHA256SUMS` before extraction.
Fetch release/source metadata into `upstream/` and write an ownership marker
there as recorded by the setup; never run the global installer.

```sh
python3 target/blaze-recheck/probe.py stable-toolchain stable toolchain
python3 target/blaze-recheck/probe.py canary-helper canary build --locked -p disktree-scan
# Run emitted helper --help and check_protocol.py before proceeding.
python3 target/blaze-recheck/bench.py
git worktree add --detach "$PWD/target/blaze-recheck/worktree2" bed9b0c6f5a07a0e5586106f26758b04e18ec84d
python3 target/blaze-recheck/worktrees.py
# Compile apfs-probe.c with clang -fno-modules; query only owned artifacts.
python3 target/blaze-recheck/probe.py canary-status canary status
python3 target/blaze-recheck/probe.py canary-stop canary stop
python3 target/blaze-recheck/probe.py canary-stopped-status canary status
```

The harnesses capture their scoped environments and every build's exit,
wall time, helper output, size and hash. Cargo's native pin is retained.
Canary uses private `HOME`, cache/state roots and an owned nightly symlink;
no `BLAZE_OTHER_TOOLCHAIN` override is used. Set `TMPDIR`,
`CARGO_TARGET_DIR` and `CARGO_BUILD_JOBS=2` only in the subprocess when
running the Cargo integration tests and lint gate. Remove only the owned
second worktree, then the owned disposable targets, caches and tools after
recording the final inventory; keep evidence and the clean source checkout.
