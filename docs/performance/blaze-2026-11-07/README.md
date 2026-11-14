# Blaze evaluation — November 7, 2026

**Verdict: skip adoption for PwrDisk today.** The stable Blaze compiler
could not emit our SSH helper. The newest canary reached this evaluation's
disk budget before emitting it. Neither produced an equivalent successful
build, so there is no measured build-time or disk saving to claim. No Cargo,
toolchain, CI, release, application or global configuration was changed.

This evaluates [indexable-inc/blaze](https://github.com/indexable-inc/blaze),
the requested corrected source. It does not evaluate the earlier tweet.
Measurements were taken November 7 in America/New_York (November 8 UTC).
Machine details and paths below describe the measurement environment, not
additional prerequisites for PwrDisk.

## Versions and scope

PwrDisk started clean, refreshed from `origin/main`, at
`3acc2da69f8b304945c0894ee883c52862eaf282`. The branch is
`research/blaze-build-evaluation`. Only its assigned worktree was used.

| Input | Exact version |
| --- | --- |
| Host | macOS 27.2, build 26B123, arm64, APFS |
| PwrDisk compiler | rustc 1.97.1, `8bab26f4f68e0e26f0bb7960be334d5b520ea452` |
| Cargo | 1.97.1, `c980f4866` |
| Latest non-prerelease | Blaze 0.20261027.0; binary reports `49ca152` |
| Latest published canary | Blaze 0.20261105.4; binary reports `e930296` |
| Current source inspected | `5a7dfc46201db1a2962288e9846295ee984dd707` |
| Blaze Rust downloaded | nightly-2026-10-27, separate from PwrDisk's pin |

The [stable release](https://github.com/indexable-inc/blaze/releases/tag/v0.20261027.0)
was published October 28; the
[canary](https://github.com/indexable-inc/blaze/releases/tag/v0.20261105.4)
was published November 5. Stable's tag resolves to
`5d4f78f7577cf7170993b74f735099dd03f5c6ca`, whereas its binary reports
[`49ca152ec9ae52a31d353e7df9d5a304b0ca097c`](https://github.com/indexable-inc/blaze/tree/49ca152ec9ae52a31d353e7df9d5a304b0ca097c).
That distinction matters when reading its docs. Canary's source commit is
[`e93029602544df9882fffb18bf272d048a1a7136`](https://github.com/indexable-inc/blaze/tree/e93029602544df9882fffb18bf272d048a1a7136).
Current `main` describes v2 and explicitly labels its speed tables as v1
measurements; those tables are not PwrDisk results.
([README](https://github.com/indexable-inc/blaze/blob/5a7dfc46201db1a2962288e9846295ee984dd707/README.md))

Blaze is an independent compiler/front end, code generator, linker and
persistent daemon. It is not a cache wrapper around our pinned rustc.
Its MIT/Apache-2.0 licensing permits evaluation; neither it nor any new
dependency was added to PwrDisk.
([licenses and source](https://github.com/indexable-inc/blaze/tree/5a7dfc46201db1a2962288e9846295ee984dd707))

## Observed results

All Cargo measurements used the unchanged workspace profiles, lockfile and
default features, native target, two Cargo jobs and cached dependencies.
The dev profile has workspace `opt-level = 1`, dependency `opt-level = 2`
and limited debug information. There were no dependency downloads in these
Cargo runs. Cold means an empty target directory, not an empty OS file cache
or an empty Cargo source cache. Each number is one sample, not a median.

| Operation | Wall seconds | Result |
| --- | ---: | --- |
| Cargo core, empty target | 33.64 | Emitted `libdisktree_core.rlib` |
| Cargo core, no change, serial sample | 0.151 | Success |
| Cargo core, function edit, serial sample | 2.017 | Success |
| Cargo helper, first link with core/dependencies warm | 1.430 | Emitted arm64 executable |
| Cargo helper, no change | 0.203 | Success |
| Stable Blaze, explicit PwrDisk `RUSTC` | 0.048 | Refused Rust 1.97.1 |
| Stable Blaze toolchain download/install | 8.811 | About 123 MB downloaded; additional disk below |
| Stable Blaze core, first usable socket | 25.794 | Successful **check**, no core `rlib` |
| Stable Blaze helper | 5.641 | Compiler error, no executable |
| Canary helper, offline | 2.827 | Own source store empty despite Cargo cache |
| Canary helper, online | 101.728 | Stopped at free-space floor during compilation |

The edit replaced `scale(bytes).format(1)` in `human_bytes` with a local
`scaled` binding followed by `scaled.format(1)`. It changed a function body
without changing its output. The original file was restored, rebuilt and
verified clean. The final serial Cargo samples ran after the lint/test
jobs and private Blaze daemons had stopped. An earlier helper timing that
waited on this evaluation's lint target lock was discarded. No other
owner's target directory was used.

The cold Cargo wall time comes from `/usr/bin/time -lp`; Cargo reported
33.57 seconds internally. In the restricted execution environment, `time`
also complained about access to `kern.clockrate`, so its exit code and
resource counters were not used as the build verdict. Later serial samples
use Python's monotonic clock and the actual Cargo exit code. No whole-build
CPU or peak daemon-memory comparison was obtained. Background machine load
was not controlled.

Blaze's first attempt hit macOS's Unix socket path-length limit before
compilation. Moving only its owned state from
`target/blaze-evaluation/stable-home` to `target/bh` fixed that. It is
excluded from the core timing above.

Stable then rejected an explicitly supplied Rust 1.97.1. It offers an
unchecked other-toolchain escape hatch that still assumes its nightly's
type layouts; that is not an equivalent pinned-compiler configuration and
was not enabled.
([toolchain source](https://github.com/indexable-inc/blaze/blob/49ca152ec9ae52a31d353e7df9d5a304b0ca097c/crates/ra-jit/src/cargo/toolchain.rs))

Its successful library-only command did not create a core library artifact.
This matches its documented behavior: an ordinary Rust library without a
program is checked rather than emitted for Cargo. Comparing 25.794 seconds
with Cargo's 33.64 would therefore compare different work.
([stable build semantics](https://github.com/indexable-inc/blaze/blob/49ca152ec9ae52a31d353e7df9d5a304b0ca097c/docs/cargo.md#what-a-build-does))

Building the real `disktree-scan` binary reached `std::thread::spawn` and
failed on an unsupported writable static named `MIN`. That is a concrete
stable compiler blocker, not a slow link or a cache miss. The canary's
offline command instead requested a separate source fetch. Its online run
downloaded public dependencies, compiled its standard library and began
project dependencies, but did not finish within the disk allowance. Its
101.728 seconds includes downloads, startup and compilation; no reliable
phase-duration split or final link timing was available.

Cargo's helper passed `--help` without scanning any real filesystem. It was
a Mach-O arm64 executable with SHA-256
`ada4e79f76224be8c1714d19a7fc8fe385c40a2c91e32eb90637d890af3aebd2`.
Blaze produced no helper to compare behavior or hashes against. This is not
a binary-equivalence result.

## Disk accounting and cleanup

There was about 30 GiB free initially (`df -h`). Builds were restricted to
owned directories under `target/`; no existing targets, worktrees or caches
were pruned. Stable had a 4 GiB worker setting and 512 MiB sealed-pack limit;
canary had its v2 4 GiB daemon setting and two-core budget. The stable cache
limit is **not** a cap on all artifacts, and v2 does not implement that v1
setting. Each probe had a 180-second limit. The online canary was stopped
earlier by a 20 GiB available-space floor. Both private daemons were
confirmed stopped before disposal.

This is the end-of-experiment inventory, including lint/test artifacts and
both downloaded tool distributions. Values are bytes, not rounded GB.

| Owned area | Logical bytes | Allocated bytes reported by `st_blocks * 512` |
| --- | ---: | ---: |
| Cargo target, including lint and core test | 2,022,413,875 | 2,040,401,920 |
| Stable build target | 485,527,124 | 486,326,272 |
| Stable store | 213,598,415 | 213,659,648 |
| Stable daemon/toolchain | 660,076,022 | 670,085,120 |
| Stable binary distribution | 188,733,691 | 188,760,064 |
| Stable compressed download | 62,622,556 | 62,623,744 |
| Canary build target | 1,980,581,606 | 1,981,308,928 |
| Canary daemon/store/sources | 4,915,503,278 | 5,042,864,128 |
| Canary binary distribution | 488,476,299 | 488,488,960 |
| Canary compressed download | 154,151,309 | 154,152,960 |
| Canary private user home | 1,398,644 | 1,970,176 |
| All areas, including provenance and logs | 11,183,908,482 | 11,341,750,272 |

Files were enumerated with `lstat`; symlinks were not followed and duplicate
inodes within an area were counted once. These allocated-byte totals can
count APFS clone-shared blocks more than once. Shared and unique physical
bytes were **not measured**, and cannot be derived from `du`, inode identity
or `st_blocks`. No file-level private-allocation API was used. The canary
target and store must not be treated as disjoint physical extents. Nor is
this a successful-build disk comparison: Cargo completed additional gates
while the canary stopped partway through its build.

After recording the inventory, only the explicitly owned binary archives,
tools, toolchain, build targets and stores above were removed. Available
space went from 21,135,360,000 to 30,474,240,000 bytes across disposal.
Other machine activity and APFS accounting can affect that difference; it
is not an exact unique-byte measurement. About **11.1 MB allocated** of
public upstream provenance, local harnesses and logs was retained in
`target/blaze-evaluation/`, marked with `OWNER.txt`. Its `logs/` contains
the raw results, inventory and cleanup record. No shared user data was
removed. The checked-in evidence is [results.json](results.json).

## Reuse, locking and applicability

| Concern | Evidence and implication for PwrDisk |
| --- | --- |
| Worktree source copies | Git shares repository objects; each worktree still has source files. Blaze is not a replacement for that model. No second worktree was needed after the executable blocker. |
| Stable reuse | Docs describe a shared worker and content-keyed function store. Analysis reuse does not eliminate per-worktree targets, final objects or linking. Historical Cargo-first descriptions coexist with newer independent-build behavior; they are not measurements of this binary on PwrDisk. |
| Newer v2 reuse | Current source describes per-workspace build directories and clone placement from a content store. Registry/git sources are separately materialized under its own root, even with a populated Cargo home. This experiment observed that extra source-fetch requirement. |
| Cargo incremental state | Cargo's target/incremental outputs remain separate from Blaze's store. Keeping Cargo for authoritative gates plus Blaze can add storage; relocated bytes still count. |
| Concurrent builds | Stable documents locked append packs and collection, v2 has its own daemon/store and build leases. This evaluation used isolated homes and did not validate multiple writers or concurrent worktree correctness. Sharing a target directory would also serialize Cargo through its build lock. |
| Eviction | Stable limits sealed packs and collects older entries. Current v2 docs describe pressure-based GC and leases. Neither implies fixed total disk use or permission to point an experimental collector at existing data. |

Sources for these distinctions: the
[stable store model](https://github.com/indexable-inc/blaze/blob/49ca152ec9ae52a31d353e7df9d5a304b0ca097c/docs/cache.md),
[stable daemon](https://github.com/indexable-inc/blaze/blob/49ca152ec9ae52a31d353e7df9d5a304b0ca097c/docs/cargo.md#the-daemon),
[current workspace placement](https://github.com/indexable-inc/blaze/blob/5a7dfc46201db1a2962288e9846295ee984dd707/crates/blaze-manifest/src/home.rs),
[v2 store](https://github.com/indexable-inc/blaze/blob/5a7dfc46201db1a2962288e9846295ee984dd707/docs/v2-store.md)
and [GC](https://github.com/indexable-inc/blaze/blob/5a7dfc46201db1a2962288e9846295ee984dd707/docs/gc.md).
The v2 store document still calls out a single-writer locking requirement;
it should not be read as proof that arbitrary daemons can share one store.

PwrDisk has six workspace members: GUI, core, MCP, link, helper and xtask.
Passing one core check cannot validate the rest. GPUI's native libraries,
build scripts, proc macros, test harness and `screenshots` feature were not
successfully built by Blaze here. Replacing `make`'s `CARGO` with `blaze`
does not transparently implement the `xtask` alias, Clippy or rustfmt.
PwrDisk's real lint gate runs Clippy across all targets twice, including all
features. Blaze's checks do not replace that gate.

The release profile uses thin LTO, one codegen unit and overflow checks.
Stable's compatibility table refuses LTO in its daemon model and contains
outdated fallback descriptions; release compatibility is unverified.
Current v2 maps profile levels to different compiler backends, including
stencils for level 1. Matching manifest text alone is not proof of matching
code generation. No release flags were weakened to manufacture a win.
([stable support](https://github.com/indexable-inc/blaze/blob/49ca152ec9ae52a31d353e7df9d5a304b0ca097c/docs/support.md),
[current backend selection](https://github.com/indexable-inc/blaze/blob/5a7dfc46201db1a2962288e9846295ee984dd707/docs/cargo.md#backends))

| Platform | Status relevant to PwrDisk |
| --- | --- |
| macOS arm64 | Both published binaries ran here; stable helper failed, canary incomplete. |
| macOS Intel | Upstream says unsupported; cannot cover PwrDisk's universal release. |
| Linux x86-64 / arm64 | Upstream publishes glibc builds, requiring glibc 2.34+ and a C linker. Not tested here. |
| Linux musl | Upstream does not support an Alpine/musl host. PwrDisk's cross-built static helpers were not tested. |
| Windows | No published installer; v1 reports manual x64 work, v2 has not been run there. No Windows ARM64 validation obtained. |

Those are upstream support claims, except the macOS observations.
([current installation requirements](https://github.com/indexable-inc/blaze/blob/5a7dfc46201db1a2962288e9846295ee984dd707/docs/install.md))
Clone support also constrains disk sharing: current v2 refuses placement
when filesystem cloning is unavailable rather than copying. An open
[ext4 report](https://github.com/indexable-inc/blaze/issues/1781) reproduces
that failure in 0.20261105.1. Another open
[frontend report](https://github.com/indexable-inc/blaze/issues/1826)
shows unsupported-function failures on other ordinary crates. These are
upstream reports, not failures reproduced by this experiment.

## Privacy, network and cost

No remote build/cache service, account, credentials or paid infrastructure
was configured. Remote-cache and AWS variables were removed from probe
environments; stable's remote was explicitly empty. No cache push command
was run. Existing Cargo dependencies were reused read-only as build inputs;
only public toolchain, distribution, metadata and crate downloads were
requested. Canary additionally used a private `HOME` and cache home; its
toolchain path symlink pointed to the evaluation's own downloaded nightly.
Global Rust/Cargo/PATH/shell settings were not edited, and `blaze enable`
was never used.

Local operation does not require a paid service, although first setup is
not offline. Current docs describe background update checks. The upstream
remote-cache design includes authenticated private namespaces, but that
design explicitly does not hide plaintext artifacts from its gateway or
Cloudflare. It is not authorization to upload source, paths, build scripts,
objects or test outputs. No service price was verified and no cost-saving
claim is made. This was a bounded configuration/source review, not a full
network capture or security audit.
([updates](https://github.com/indexable-inc/blaze/blob/5a7dfc46201db1a2962288e9846295ee984dd707/docs/install.md#updates),
[remote design and status](https://github.com/indexable-inc/blaze/blob/5d4f78f7577cf7170993b74f735099dd03f5c6ca/docs/remote-cache.md))

## Verification and limits

`cargo xtask lint` passed, including both default and all-feature Clippy
passes. `cargo test --offline --locked -p disktree-core size::tests` passed
all six selected tests on the original source. The temporary function edit
was restored; Cargo built the restored helper and its help output passed.
No application window, local MCP link or provider workflow was used.

The full workspace test suite, GUI runtime, release packaging, cross-target
matrix, shared-store concurrency and two-worktree reuse were not run.
There was no successful Blaze executable to justify multiplying the build
matrix. The canary result remains inconclusive; it is not a demonstrated
compiler failure. Existing CI remains authoritative for platform gates.

A future adoption experiment first needs a successful unchanged helper,
then matching-profile GUI/MCP/xtask and test output checks. It would need
enough disposable disk to complete v2's cold std/source build, followed by
serial cold, warm and function-edit samples on two real worktrees, a
concurrency correctness check and clone-aware physical accounting. Those
are prerequisites to a new verdict, not completed measurements. Existing
PwrGit worktree-sharing research should be consulted before adding another
sharing implementation.

## Reproduction

Use an isolated clean worktree at the PwrDisk commit above. The binaries
were fetched directly, never through the global installer. Compressed
downloads were checked against each release's `SHA256SUMS`:

| Archive suffix `-aarch64-apple-darwin.tar.gz` | SHA-256 |
| --- | --- |
| `blaze-v0.20261027.0` | `47dae8e838765931ed8e216a94e4d75454469fa6c3950d1c02eba2841bc8dad5` |
| `blaze-v0.20261105.4` | `27da522a507cba1716ecbfbf91fc99546956b6a6f9da21c608539bb70c0da45b` |

For example, with `E="$PWD/target/blaze-evaluation"` and `V` one of the
versions above:

```sh
mkdir -p "$E/download" "$E/$V"
curl -fL "https://github.com/indexable-inc/blaze/releases/download/v$V/blaze-v$V-aarch64-apple-darwin.tar.gz" \
  -o "$E/download/blaze-v$V-aarch64-apple-darwin.tar.gz"
curl -fL "https://github.com/indexable-inc/blaze/releases/download/v$V/SHA256SUMS" \
  -o "$E/download/SHA256SUMS-$V"
# Check the archive's line against the digest above before extraction.
shasum -a 256 "$E/download/blaze-v$V-aarch64-apple-darwin.tar.gz"
tar -xzf "$E/download/blaze-v$V-aarch64-apple-darwin.tar.gz" -C "$E/$V"
```

These were the Cargo workloads, with `CARGO_BUILD_JOBS=2` and
`CARGO_TARGET_DIR="$E/cargo-target"` set only for these commands:

```sh
cargo build --offline --locked -p disktree-core
# Repeat unchanged; make the function edit above; build; restore; build.
cargo build --offline --locked -p disktree-scan
cargo build --offline --locked -p disktree-scan
"$E/cargo-target/debug/disktree-scan" --help
cargo xtask lint
cargo test --offline --locked -p disktree-core size::tests
```

For stable, the extracted `blaze` used this per-process environment:

```sh
BLAZE_HOME="$PWD/target/bh"
BLAZE_CACHE="$E/stable-store"
BLAZE_CACHE_MAX=512M
BLAZE_REMOTE=
BLAZE_IDLE_SECS=60
BLAZE_MEMORY_MAX=4G
BLAZE_SPARE=0
CARGO_TARGET_DIR="$E/blaze-target"
CARGO_BUILD_JOBS=2
# Also remove inherited BLAZE_*, RA_JIT_* and AWS_* variables first.
```

The commands were `--version`, `--help`, then
`build --offline --locked -p disktree-core` with explicit
`RUSTC="$(rustup which rustc)"` to test the pin, `toolchain`, the same core
build without `RUSTC`, `build --offline --locked -p disktree-scan`,
`status` and `stop`. These variables must be exported in the isolated
subprocess, not added to shell configuration.

Canary used the same isolated environment, overriding `BLAZE_HOME` with
`$PWD/target/b2`, `CARGO_TARGET_DIR` with `$E/canary-target`,
`HOME` with `$E/canary-user-home`, `XDG_CACHE_HOME` with `$HOME/cache`,
`BLAZE_DAEMON_MEMORY_GB=4` and `BLAZE_CORES=2`. The private home's
`.blaze/toolchains` was a symlink to `$PWD/target/bh/toolchains`.
Its commands were `build --offline --locked -p disktree-scan`, then
`build --locked -p disktree-scan`, followed by `stop` and `status`.
Do not rely on v1 cache/memory variables to bound v2.

The local Python harness used `subprocess.Popen(...,
start_new_session=True)`, redirected output to an owned log, polled
`time.monotonic()` and `os.statvfs(E).f_bavail * f_frsize` once per second,
and sent SIGTERM to that process group at 180 seconds or below 20 GiB
available. The daemon requires a separate `stop` afterward; the floor
must not prevent stop/status commands. Time each command separately and
serialize comparisons. Inventory all roots using `lstat` before disposal;
retain failed runs rather than presenting them as fast successful builds.
