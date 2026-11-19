# Whole-app Blaze evaluation — November 9, 2026

**Verdict: keep Cargo on this host. A full Blaze workflow time was not
obtained.** The complete fresh-worktree Cargo workflow passed in **14m38s**.
Blaze's full GUI workspace build exceeded the experiment's physical-memory
allowance before producing PwrDisk. This is a resource-limited result,
not proof of compiler incompatibility or a measured whole-workflow slowdown.

The [November 8 study](../blaze-2026-11-08/README.md) measured the SSH
helper. Its timings do not answer how long a new PwrDisk worktree takes to
build and test. This study measures the application, its workspace, the
actual quality gates and the normal release profile. No Blaze integration,
product setting, global configuration or CI default was added.

The [November 11 Cargo-seeding follow-up](../cargo-seed-2026-11-11/README.md)
completed the same full workflow with PwrGit bounded and strict target
seeding, using the pinned compiler. It measures a separate reuse mechanism;
it does not supply the missing Blaze completion time.

## Source, host and commands

Both owned detached Git worktrees were created at origin/main commit
**6a7bad7e5fc53fd54bdbb271fc5b68f789b2d597**, PwrDisk 0.22.0.
The host was macOS 27.2 (26B123), arm64, APFS, 32 GiB RAM and 12 logical
CPUs. Approximately 124 GiB was initially available. Existing Cargo
dependency sources and the installed project compiler were reused;
the primary Cargo build target started empty.

The primary serial workflow was:

~~~sh
cargo build --workspace --offline --locked
cargo xtask lint
cargo xtask test
cargo build --release --offline --locked
~~~

Cargo used four jobs and offline dependency access. The project pin was
Rust 1.97, resolving to rustc 1.97.1 (8bab26f4f); Cargo was 1.97.1.
No profiles or features were changed: dev remains optimization level 1
with limited debug information, dependencies level 2; release retains
thin LTO, one codegen unit and overflow checks.

The test command runs the entire workspace, then the app's screenshots
feature tests, then both Python script suites. Lint runs rustfmt checking
and Clippy over all workspace targets, both with and without all features.
These are headless checks. No app window, existing app link, provider
operation, signing, packaging, release or merge was performed.

## Completed Cargo baseline

| Serial phase | Wall time | Result |
| --- | ---: | --- |
| Development workspace/app build | 252.690 s | Passed |
| Complete lint gate | 89.341 s | Passed |
| Complete test gate | 370.147 s | Passed |
| Normal release build | 166.053 s | Passed |
| **Total** | **878.231 s / 14m38s** | **All passed** |

This total excludes worktree creation and experiment setup. It includes
command startup, compilation, linking and test execution. This is one
bounded trial on a shared operator host, not a statistical benchmark.
“Cold” means an empty build target, not empty system file caches, a newly
installed compiler or a freshly downloaded Cargo registry.

The log reports 1m48s for the workspace test compilation and 24.77s for
the additional screenshots-feature test compilation. Core execution took
217.21s and the app suite 8.80s. These log durations explain much of the
test gate; they are not a precise exhaustive wall-time partition.

The app passed 450 tests (two ignored); core 493 (seven ignored); link 18;
MCP 83; helper integration four; xtask 14. The additional screenshots
selection passed eight tests, and Python passed five screenshot-script
and six release-script tests. These are the repository's normal ignores;
none were added for the benchmark. Development and release app/helper
executables also exited zero on --help, with hashes retained in results.

### Rejected setup runs

Initial setup attempts were excluded from the primary table. Long nested
checkout paths exceeded macOS's Unix-socket path limit in four app tests.
Shortening TMPDIR alone did not fix checkout-local sockets; both owned
worktrees were moved to short a/ and b/ paths. The app crate path was then
72 bytes.

A custom workspace TMPDIR also invalidated a core oracle that deliberately
uses macOS's protected native temp location. Restoring native TMPDIR for
tests made the complete test gate pass. The primary table is a subsequent
fresh empty-target run with that verified setup, not a sum of successful
fragments from failed runs. Failed-attempt logs and disposal sizes remain
in the evidence.

## Actual full-app Blaze attempts

Official releases were checked on November 9. Latest stable remained
0.20261027.0; latest canary remained 0.20261105.4, source commit
e93029602544df9882fffb18bf272d048a1a7136. Current main inspected was
d7c64049d0733c7cf04a85cbcc1cac92c462158f; it was not built.

Stable was used only to provision the private nightly prerequisite.
Canary performed the full-app attempts. The earlier stable writable-static
MIN failure was not retested or claimed fixed.
([stable](https://github.com/indexable-inc/blaze/releases/tag/v0.20261027.0),
[canary](https://github.com/indexable-inc/blaze/releases/tag/v0.20261105.4),
[current source](https://github.com/indexable-inc/blaze/tree/d7c64049d0733c7cf04a85cbcc1cac92c462158f))

| Attempt | Observed result |
| --- | --- |
| Private nightly setup | Passed, 5.652 s |
| First private daemon startup | 3.117 s |
| Standalone source fetch | Rejected in 0.571 s: no such command, despite its documentation |
| Full workspace development build, memory setting 12 | Interrupted after approximately 487.853 s; vmmap showed physical footprint 22.8G, peak 23.7G |
| Memory setting 6 | Build, workspace tests, screenshots tests and release all refused at lowering; predicted need 6.84 GB exceeded the stated 5 GB allowance; diagnostic requested setting 8 |
| Separate pinned-Cargo candidate lint target | Passed, 159.450 s; its compilation and storage are counted |
| Full workspace development build, setting 8, retained cache | Stopped at 205.259 s by the physical-footprint watchdog; sampled aggregate peak 21,625,353,704 bytes (20.14 GiB) |
| **Completed Blaze app/test/release workflow** | **None** |

The setting-8 run reused the setting-12 attempt's partial cache. It is
not a cold-build timing, and neither interrupted time is a time-to-build.
The GUI executable was absent after the final attempt. Both private
daemons exited, including exit zero after the final stop.

The initial RSS watchdog missed compressed memory: a contemporaneous RSS
sample was about 7 GiB while vmmap reported over 22G physical footprint.
Subsequent monitoring used Darwin proc_pid_rusage physical footprint,
including compressed memory, sampled every two seconds. A known touched
128 MiB allocation increased that meter by 134,316,128 bytes. Aggregate
per-process footprint is a conservative resource guard, not a measurement
of unique system-wide RAM.

The remaining setting-6 entry points were actual invocations, but their
memory refusals did not compile or run their Rust tests. Its Python
screenshot suite also failed because the isolated HOME hid the existing
Rustup setup from Cargo manifest inspection. That setup error is not
attributed to Blaze. Both Python suites passed when rerun with the normal
pinned-Cargo environment; the final harness preserves that environment
for Python, as it does for mandatory lint.

### What can and cannot be compared

Blaze's documented compiler ignores the project Rust pin and uses its own
nightly/compiler, here nightly-2026-10-27, reporting Rust 1.101.0-nightly.
Its dev backend selection also differs from pinned rustc/LLVM. Keeping
Cargo.toml unchanged does not make generated code, optimization, debug
information, linking or release quality equivalent.
([toolchain](https://github.com/indexable-inc/blaze/blob/e93029602544df9882fffb18bf272d048a1a7136/docs/toolchain.md),
[backend documentation](https://github.com/indexable-inc/blaze/blob/e93029602544df9882fffb18bf272d048a1a7136/docs/cargo.md#backends))

Blaze used four cores and its documented nice-10 policy; the daemon's nice
value was observed as 10. Cargo used four normal-priority jobs. The memory
setting is a soft accounting request, not a physical-memory ceiling.
Blaze also required a private HOME for its toolchain/cache isolation.
These differences preclude an equivalent-compiler speedup claim.

Blaze downloaded dependencies inside its first build because standalone
fetch was unavailable in the published binary. Its daemon separately
logged a 1.7s standard-library download/materialization. No trustworthy
complete download/compile/link partition or completed-build cache-hit/miss
summary was obtained. Compiling-line counts are not cache-hit counters.

Full-app warm/no-change, edit/rebuild, second-worktree reuse and concurrent
artifact correctness checks were not run: the working GUI executable
prerequisite was not satisfied within the resource allowance. The second
worktree's source and Git metadata are nevertheless counted. Earlier
helper reuse/concurrency observations do not establish those properties
for GUI, MCP, feature combinations, tests or release builds.

## All introduced storage

The final inventory below deduplicates hardlinks by device/inode.
“Allocated” is regular-file st_blocks multiplied by 512, not unique APFS
physical usage. The exact byte inventories are in results.json.

| Area | Logical GiB | Reported allocated GiB |
| --- | ---: | ---: |
| Cargo baseline target | 7.449 | 7.497 |
| Partial Blaze target | 7.189 | 7.194 |
| Required candidate Cargo lint target | 1.503 | 1.519 |
| Private Blaze state/store/sources | 11.342 | 11.461 |
| Private HOME cache/materializations | 1.216 | 1.411 |
| Private nightly prerequisite | 0.615 | 0.624 |
| Stable and canary distributions, unpacked | 0.631 | 0.631 |
| Distribution archives, logs, other files and both checkouts | included below | included below |
| **All owned experiment roots** | **30.165** | **30.558** |

Exact all-root totals were 32,389,529,513 logical bytes and
32,810,999,808 reported allocated bytes over 146,253 regular-file inodes,
plus 106,496 allocated bytes of new shared-repository worktree metadata.
Excluding the retained Cargo baseline target and both source checkouts
leaves approximately **23.045 GiB reported allocation** introduced by
Blaze evaluation, its required separate lint target and supporting evidence.
This is a partial-build footprint, not a completed-Blaze-vs-completed-Cargo
storage ratio.

An early per-pathname Cargo target tally was 7.90 GiB; it counted hardlinked
names repeatedly. The final inode-deduplicated tally is 7.50 GiB.
Small allocation changes were also observed while files remained retained;
they are not cache savings.

APFS clones can share extents across distinct inodes, so even the corrected
sum can count shared blocks repeatedly. Per-file private-byte fields and
clone IDs cannot reconstruct the complete extent-sharing graph. This study
does not measure total shared or unique physical bytes, nor establish a
total disk reduction. The prior helper probe demonstrated clone placement
with this canary; it does not establish net savings for this failed app
build. Free-space deltas are confounded by other host activity.

Source checkouts were ordinary Git worktrees sharing existing repository
objects, not new repository clones. A checkout's roughly 8 MB of source
is separate from compilation reuse. Cargo targets contain profile-specific
objects, final programs, test executables and incremental state. Blaze's
documented cross-worktree function store still materializes per-worktree
programs and build-script output directories; clone placement does not
eliminate linking or prove all final program bytes are identical. Profile,
feature, test and source-path differences can change what is reusable.
Those full-app reuse and eviction claims remain unverified here. Directory/symlink metadata,
shared repository fetch/object deltas and transient test fixtures outside
the roots are not attributed by the regular-file sum. Existing unchanged
tests use native macOS temp and hardcoded /tmp fixtures, including owned
sparse APFS oracle images. Those fixtures were cleaned by the tests.
The whole-volume free-space guard still observed their effect.

## Limits, privacy and platform scope

Watchdogs allowed 64 GiB of summed reported allocation, required 48 GiB
available space, and limited phases to 1,200s (release 1,800s) and each
candidate workflow attempt to 45 minutes. The later physical-footprint
guard was 20 GiB. These were sampled guards, not OS quotas. Disk/time
limits did not terminate the full-app attempts; physical memory did.
The memory-setting retries and failed setup artifacts are retained in
the evidence rather than discarded from the account.

All distributions, toolchains, sources, stores, build directories and
benchmark-owned temporary directories were inside this assigned workspace.
No global Cargo/Rust/PATH/shell edits, source/artifact upload, HTTP daemon
listener, account signup, paid service or external cache was configured.
Public GitHub, Rust distribution and crate downloads were used. Network
traffic was not packet-audited. Local use requires no paid remote service;
optional remote-build offerings are not evaluated.
([upstream README](https://github.com/indexable-inc/blaze/tree/d7c64049d0733c7cf04a85cbcc1cac92c462158f#readme))

Blaze is MIT OR Apache-2.0. Published support is macOS arm64 and Linux
x86-64/arm64 with glibc 2.34+, not Intel macOS, Windows or musl.
No Linux, Windows, universal macOS, release packaging or published-main
build was tested here. Current open issues include Linux filesystem/linker
limitations; they are upstream reports, not observations from this host.
([license](https://github.com/indexable-inc/blaze/blob/d7c64049d0733c7cf04a85cbcc1cac92c462158f/LICENSE-MIT),
[installation/platform requirements](https://github.com/indexable-inc/blaze/blob/e93029602544df9882fffb18bf272d048a1a7136/docs/install.md),
[glibc linking issue](https://github.com/indexable-inc/blaze/issues/1888))

Existing PwrGit worktree-sharing research was consulted. No source-cloning,
Cargo-cache-sharing, eviction or fingerprint tooling was duplicated.
Upstream's store, clone placement, leases and garbage collection are
documented mechanisms; this experiment does not verify full-app eviction
or concurrent-build correctness.
([storage layout](https://github.com/indexable-inc/blaze/blob/e93029602544df9882fffb18bf272d048a1a7136/docs/cargo.md#where-a-build-writes),
[GC and leases](https://github.com/indexable-inc/blaze/blob/e93029602544df9882fffb18bf272d048a1a7136/docs/gc.md))

## Reproduction and retained evidence

The harness files are evidence snapshots, not adopted project tooling.
Use a short isolated checkout and an actual disposable disk/memory budget;
the same operator-host memory allowance is expected to stop these attempts.
Do not reuse another owner's targets or delete their caches.

The setup recipe downloads the two pinned, checksum-verified official
archives into target/blaze-full. It reconstructs the recorded download and
extraction steps; its syntax and checksum pins were checked, but the
installer recipe was not rerun as a whole. The following is a guarded rerun recipe
for a future suitable host, not a recommendation to repeat this on the
operator's current memory budget. Keep native TMPDIR available for tests.

~~~sh
mkdir -p target/blaze-full
cp docs/performance/blaze-2026-11-09/harness/setup.py.txt target/blaze-full/setup.py
python3 target/blaze-full/setup.py
git worktree add --detach a 6a7bad7e5fc53fd54bdbb271fc5b68f789b2d597
git worktree add --detach b 6a7bad7e5fc53fd54bdbb271fc5b68f789b2d597
cp docs/performance/blaze-2026-11-09/harness/flow-footprint.py.txt target/blaze-full/flow.py
python3 target/blaze-full/flow.py cargo-a-cold
python3 target/blaze-full/flow.py bootstrap
BLAZE_BENCH_MEMORY_GB=8 python3 target/blaze-full/flow.py blaze-a-8gb
~~~

The original Cargo/RSS harness is retained separately to preserve the exact
historical instrumentation. Per-phase exit codes are authoritative: the
harness continues ordinary command failures to collect the remaining
evidence, so its own exit zero does not certify a passing workflow. Do not rerun its setting-12 Blaze command
without an independent physical-memory guard. The actual run history also
contains the rejected setting-6 retry; the recorded setting-8 attempt
resumed the retained cache, whereas the guarded recipe above starts a new
cache. These are different initial states, not interchangeable cold times.

Distribution installation and worktree creation were outside the primary
timing. Exact setup, commands, phases, output checks, memory-stop records,
failed-setup disposal sizes and inventories are in
[results.json](results.json), [captured output](captured-output.txt) and
the [harness snapshots](harness/).

Only this experiment's worktrees, targets, distributions, private daemon
state, toolchain, source caches and temp directory were removed after
inventory. Raw evidence and scripts remain under target/blaze-full. The retained
snapshot contains 3,834,312 logical bytes / 4,079,616 reported allocated
bytes (about 3.9 MiB allocated), across 91 regular-file inodes. The
committed report and evidence snapshots add roughly 73 KiB of logical
content. The checkpoint is recorded in results.json. No existing
user worktree, cache, Docker disk, app data or other owner's build was
cleaned.

