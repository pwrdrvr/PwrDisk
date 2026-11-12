# Local sccache on PwrDisk — November 12, 2026

**Verdict: conditional for build latency; skip as a disk-saving default.**
A populated local sccache reduced the complete fresh-checkout workflow from
902.300s to 621.080s (31.2%) in this observation. It reused dependencies
built after the recipient checkout existed, with the population checkout's
original path absent. It did not reproduce Blaze's large resident compiler
heap. It added cache storage, and an adversarial proc-macro fixture returned
a stale hit. No project integration or global/default change is justified
by this result. Compiler, profiles, flags, features and test selection stayed
unchanged.

The prior [target-seeding study](https://github.com/pwrdrvr/PwrDisk/blob/4673b23cb507547195aabbc91e26a5ec13484ac5/docs/performance/cargo-seed-2026-11-11/README.md)
and [Blaze study](https://github.com/pwrdrvr/PwrDisk/blob/4673b23cb507547195aabbc91e26a5ec13484ac5/docs/performance/blaze-2026-11-09/README.md)
were read from that commit in this assigned workspace, including their
results and harnesses. Their timings are historical context, not this
study's control. PwrGit PR #529 and its exact
[d2e3a806 tool/research](https://github.com/pwrdrvr/PwrGit/tree/d2e3a806aa4ba384d6e1c30756853a529b6f7036/tools/pwrgit-share)
were consulted. No seeding implementation or target seeding is used here.

## Complete workflow and all cache overhead

| Phase | Plain, empty target | Empty-cache population | Populated-cache recipient |
| --- | ---: | ---: | ---: |
| Development | 257.171s | 156.856s | 102.448s |
| Full lint | 91.443s | 83.937s | 58.783s |
| Full tests | 378.740s | 324.325s | 324.887s |
| Release | 174.946s | 173.281s | 134.962s |
| **Command sum** | **902.300s** | **738.399s** | **621.080s** |
| Instrumented wall-clock envelope | 907.331s | 744.596s | 628.189s |

The command times include wrapper invocation, compiler/key probes, cache
reads/decompression/writes, compilation, linking and actual tests. Server
startup is outside those command sums: 0.053s for population and 0.001s
for the recipient. The envelopes additionally include startup, phase
statistics, final inventories and shutdown; they are derived from original
log-file timestamps, not an additional monotonic benchmark. Their 5–7s
instrumentation overhead is retained rather than presented as compilation.

The measured public-tool/docs download, three source materializations and
private dependency-home setup took **16.317s**. Charging all of that setup
to the recipient envelope gives **644.506s**, versus plain's 907.331s.
It includes setup shared by the control as well, so this is a conservative
cost assignment. Initial research/network queries and the failed setup
attempts below were not timed as routine setup. No PwrDisk dependency
source download occurred during a measured workflow.

Population is not free: population plus one recipient costs 1,359.479s of
commands, or 1,389.102s including their envelopes and the measured setup.
The population dev phase was already faster than plain without result
hits. Host file-cache state and compiler scheduling differ between these
serial observations, so the full 31.2% difference cannot be attributed
solely to result-cache hits. The recipient is 15.9% faster than population.
No statistical confidence interval or repeated expensive matrix is claimed.

### Hits, misses and work that remains

Counts below are actual isolated/reset sccache counters, including Rust
and native C/assembly. Cargo still logs `Compiling`/`Checking` for cache hits;
those log counts are not evidence of a miss. Full reasons and language
breakdowns are in [results.json](results.json).

| Phase | Population hits / misses | Recipient hits / misses |
| --- | ---: | ---: |
| dev | 0 / 458 | 358 / 100 |
| lint | 24 / 466 | 392 / 98 |
| test | 6 / 28 | 22 / 12 |
| release | 0 / 488 | 385 / 103 |

Every primary cache phase had zero cache read/write errors and timeouts.
Non-cacheable reasons include incremental compilation, binaries/proc macros,
compiler queries and unsupported invocation shapes; none were bypassed by
changing the project. Compiler feature-probe failures in sccache statistics
are not failed Cargo phases: every phase exited zero.

Plain/population/recipient workspace-test compilation reported 1m54s /
1m01s / 1m02s; the extra screenshots compilation reported 24.15s / 19.56s /
19.38s. The core suite itself took 212.23s / 215.29s / 214.26s. These
Cargo/test log durations are observations, not an exhaustive wall-time
partition or summed compiler CPU time. Final linking remains in each phase;
no reliable separate linker wall time was collected.

All three primary arms matched every **1,135 successful Rust result
records**, the same nine ignored records, suite summaries and Python suites
(five and six tests). The count includes isolated child-process records;
it is not a count of unique test functions. App 485 and core 505 passed.
All twelve dev/release app/helper `--help` checks exited zero with matching
output. Binary hashes are recorded; cross-path binaries need not hash alike.

The donor's original `b/` source and target were unavailable throughout the
recipient workflow and eight control/recipient help checks. The donor was
retained under a different owned path for accounting, then restored after
those checks. This establishes independence for the covered headless
behavior, not every possible GUI or future dependency path.

## Ongoing reuse, edits and adversarial correctness

Both recipient checkouts existed before cache population. The 1,157 hits
across the fresh recipient's four phases therefore reuse results produced
later, rather than a one-time target seed. Each target stays independent.

| Existing recipient workflow | dev | lint | test | release | Sum |
| --- | ---: | ---: | ---: | ---: | ---: |
| No source change | 4.921s | 10.340s | 256.568s | 52.582s | 324.411s |
| Comment-only core edit | 6.245s | 14.222s | 254.368s | 77.477s | 352.312s |

Both repeat workflows passed the same full gates and matched every outcome.
The edit only appended a comment to the core source and was restored
before cleanup; it checks invalidation and reuse, not changed semantics.
The normal dev/lint/screenshots feature transitions still rebuild some
units on a no-change workflow. A naive line parser missed two warm outcomes
because concurrent APFS stderr split their libtest prefix/status. Their raw
lines and the following standalone `ok` statuses are preserved in results;
the comparison recovers only those unambiguous split records.

Separate small fixtures, outside the PwrDisk timing, found:

* **Runtime paths:** libraries using compiled-in manifest and generated
  OUT_DIR paths ran with the donor absent. Relocation produced two misses,
  not a misleading hit; the new paths worked.
* **Proc-macro filesystem input:** after changing an untracked input from
  `7` to `9` and touching unchanged Rust source to make Cargo invoke rustc,
  sccache returned one hit and the program still printed `7`. A plain clean
  rebuild printed `9`. The proc-macro dylib itself being uncached does not
  prevent stale cached results of its expansion in a library.
* **Dependency drift/concurrency:** both miniature checkouts existed first;
  after populating dependency-context results in one, two independent
  targets built concurrently with two jobs each. A shared dependency hit;
  version 0.1.0 versus 0.2.0 produced the correct `41` and `42`. Combined
  counters were three hits and one miss. This is a narrow version-drift
  oracle, not a full-app dependency-upgrade or ten-worktree stress study.
* **Eviction:** a separate 128 KiB disk cache hit an entry, displaced it
  with four other entries, then missed on it. Cache size stayed within the
  cap and the original/hit/rebuilt rlib/rmeta hashes matched. The main 8 GiB
  cache never filled; long-term production eviction churn remains untested.

These correctness probes use their own tiny release/default-rustc fixtures;
they are not alternative PwrDisk speed measurements. No project incremental
setting was disabled for them.

## Storage, including the cache and all recipients

Decimal GB below. The primary snapshot was taken after all three complete
workflows, before no-change/edit runs. No target output was pruned first.

| Materialized area | Logical GB | Allocated GB | APFS private GB |
| --- | ---: | ---: | ---: |
| Plain target | 8.318370 | 8.369988 | 7.724941 |
| Population target | 8.318369 | 8.369500 | 7.724446 |
| Recipient target | 8.152298 | 8.202047 | 7.556858 |
| Entire cache | 1.590897 | 1.594573 | 1.594573 |

The recipient omits 838 loose third-party objects (166,066,656 logical
bytes) retained by normal rustc misses; the libraries/final programs and
normal test selection are still present. This explains almost all of its
166 MB smaller target. The cache is compressed result storage, not a CoW
link between targets. Its roughly 1.59 GB allocation outweighs that smaller
layout. A second hypothetical plain target is not a measured physical
extent union, and no such union or total physical saving is claimed.

The pre-cache plain target was 8,389,226,496 allocated bytes and
7,727,075,328 APFS private bytes. Later snapshots can change slightly as
APFS allocation accounting settles. Cache restoration did not give the
large recipient private-byte reduction of the earlier CoW seed study.
The completed cache after all repeats/probes was 1,608,372,224 allocated
bytes, below its 8 GiB cap. Independent targets and incremental history
continue consuming disk even when the central cache is bounded.

All introduced roots include the control, both recipients, cache/server
state, binary/archive, cloned dependency home, source/Git metadata,
fixtures and evidence. The private Cargo home alone has about 2.285 GB
reported allocation (initially about 0.14 MB private through APFS cloning);
the unpacked tool adds about 17.1 MB. There is no uncounted tool build target.
At the primary snapshot the roots totaled **28,877,225,984 allocated bytes**.
Before cleanup, including rebuild history and small probes, they totaled
**30,054,066,501 logical**, **30,424,875,008 allocated**, and
**26,199,793,664 APFS per-file private bytes**. Peak sampled allocation was
30,465,355,776 bytes (28.37 GiB); minimum free space was 78,993,788,928 bytes
(73.57 GiB). All budgets held.

Inventories deduplicate regular files by device/inode, do not follow
symlinks, and record length, `st_blocks * 512`, APFS data allocation and
per-file private bytes. The APFS fields were available for every measured
regular file. **Private-byte sums are not physical extent unions.** Shared
extents can be counted repeatedly by allocation and missed entirely by
private-byte sums. Directory/filesystem metadata, transient native test
fixtures and compiler dep-info/linker scratch are outside these regular-file
inventories; the volume free-space guard covers their effect. Native TMPDIR
was preserved. The snapshots do not establish literal accounting of every
transient native scratch byte.


## Source, environment and equivalence

Source: current origin/main `1673f3ea95264a8b8c5077340148852443ff226a`,
PwrDisk 0.24.1. All three scratch checkouts were created before any cache
population, with empty independent targets. They are owned `git clone
--shared --no-checkout` materializations, sharing existing Git objects read
only; this does not benchmark Git worktree registration/checkout speed. Registry/git sources alone
were APFS-cloned into one private CARGO_HOME; no global Cargo configuration
was copied. No source downloads were needed for the PwrDisk workflows.

Host: macOS 27.2 (26B123), arm64, APFS, 32 GiB RAM, 12 logical CPUs.
The project pin remains `1.97`, resolving to rustc 1.97.1
(8bab26f4f, LLVM 22.1.6) and Cargo 1.97.1
(c980f4866 2026-06-30). Development retains limited debug, opt-level 1 and
dependency opt-level 2; release retains thin LTO, one codegen unit and
overflow checks. Native TMPDIR stays unchanged for the protected-temp
oracle. The headless tests use the normal native temporary fixtures,
including 128 MiB sparse APFS disk-image fixtures; these transient fixtures
are covered by the volume free-space guard, outside the file inventories.

Each serial workflow has four Cargo jobs and exactly these phases:

```sh
cargo build --workspace --offline --locked
cargo xtask lint
cargo xtask test
cargo build --release --offline --locked
```

The environment also sets `CARGO_NET_OFFLINE=true`. Lint retains both
Clippy passes, including all features. Test retains all workspace tests,
the screenshot-feature selection and both Python suites. The measured
phases include their compilation and final linking. No GUI is launched.
Packaging, signing and release publication are outside this workflow.

Order is plain, empty-cache population, populated-cache fresh recipient.
There is one observation per arm, on a shared host without clearing OS
file caches. This is not a statistical estimate for an idle machine.

## Tool provenance and isolation

Latest official release checked November 12:
[sccache v0.19.0](https://github.com/mozilla/sccache/releases/tag/v0.19.0),
published October 24, 2026. The official macOS arm64 archive's SHA-256 was
verified against GitHub's release asset digest:
`a35a0b7d622618774bc1bb8102d72562dfb21e619f88b153b370d7f89bc286af`.
The executable reports `sccache 0.19.0`. No source build or extra compiler
installation was needed. sccache is Apache-2.0 and its source MSRV is 1.91.
([manifest](https://github.com/mozilla/sccache/blob/v0.19.0/Cargo.toml),
[license](https://github.com/mozilla/sccache/blob/v0.19.0/LICENSE))

The experiment uses an allowlisted child environment, explicit private
SCCACHE_CONF and SCCACHE_CACHED_CONF paths, one foreground server, an owned
Unix-domain socket, and an 8 GiB disk cap. It never connects to the default
127.0.0.1:4226 server or another owner's cache. The private state directory is mode 0700 and server files use umask 077.
Every server PID is retained,
monitored alongside its clients, and stopped only through that endpoint;
a PID-specific kill is the fail-closed fallback. Cache/server/tool/source
state lives in this workspace. No AWS/remote cache credentials or
multilevel/dist settings are inherited. The explicit config has only a
disk backend. The release includes optional network backends; local disk
is the default, and configuring a remote backend can enable credential
lookup. The `cc` crate also recognizes the wrapper for some native C/assembly
invocations; their hit/miss counters are included. No account, login,
service or source/artifact upload is needed.
This is configuration/source verification, not a packet capture.
([configuration](https://github.com/mozilla/sccache/blob/v0.19.0/docs/Configuration.md),
[local storage](https://github.com/mozilla/sccache/blob/v0.19.0/docs/Local.md),
[S3 auth](https://github.com/mozilla/sccache/blob/v0.19.0/docs/S3.md))

## Incremental compilation and path safety

The release rejects an explicit `CARGO_INCREMENTAL=1` or
`CARGO_BUILD_INCREMENTAL=1`; its client does **not** implement
`SCCACHE_ALLOW_INCREMENTAL`. Current development
[main at 7df8044](https://github.com/mozilla/sccache/blob/7df8044dcabc3f7a0948e3df30678bf6f7b72eae/src/commands.rs)
does implement that escape hatch. The experiment sets it to 1 but does not
set either Cargo incremental override. Cargo's normal workspace/path
incremental profile behavior remains enabled, and those rustc invocations
are non-cacheable. The flag is inert on v0.19.0; it is not a way to force
incremental outputs into the result cache.
([release guard](https://github.com/mozilla/sccache/blob/v0.19.0/src/commands.rs),
[incremental bypass](https://github.com/mozilla/sccache/blob/v0.19.0/src/compiler/rust.rs))

Rust cache keys hash compiler libraries/version, arguments, source content,
extern/static-library content, target JSON, dep-info environment inputs,
additional CARGO variables and the working directory. Output/search paths
are excluded where their actual inputs are separately hashed. `env!` values
are tracked. Rust code does not apply the C/C++ SCCACHE_BASEDIRS
normalization to its working-directory/environment key. Changing flags
or forcing path remapping would be a different experiment. Registry/git
sources in the same private Cargo home have stable source paths; generated
OUT_DIR paths and their downstream artifacts can still cause misses across
target directories. Reuse is selective, not universal relocation.
([Rust key implementation](https://github.com/mozilla/sccache/blob/v0.19.0/src/compiler/rust.rs),
[path normalization](https://github.com/mozilla/sccache/blob/v0.19.0/README.md#normalizing-paths-with-sccache_basedirs))

The tagged implementation also supports metadata-only check results,
despite the tagged Rust documentation still saying `link` must be present.
The lint counters below are direct evidence for this compiler/Clippy pair.
Crates invoking the system linker, including binaries, proc macros and
build-script executables, are not cached. They and final linking still run.
Procedural macros can read filesystem inputs that rustc dep-info does not
list; the official documentation explicitly warns that these may be
cached incorrectly. A hit is not a correctness certificate.
([Rust caveats](https://github.com/mozilla/sccache/blob/v0.19.0/docs/Rust.md))

## Capacity, concurrency and memory

The documented local default is a 10G disk cap; this study explicitly uses
8 GiB. The local cache supports one server per cache directory. Sharing that
server among clients is supported; running independent servers on the same
cache races and can fail builds. Disk operations use a mutex around the
server's LRU. Completed entries are published from temporary files. LRU
eviction removes old entries to fit the configured size; startup rebuilds
order from file mtimes. The cap is cache payload accounting, not an OS
quota for all server state, directory metadata or transient writes.
([local contract](https://github.com/mozilla/sccache/blob/v0.19.0/docs/Local.md),
[disk cache](https://github.com/mozilla/sccache/blob/v0.19.0/src/cache/disk.rs),
[LRU](https://github.com/mozilla/sccache/blob/v0.19.0/src/lru_disk_cache/mod.rs))

sccache retains compiler detection/hash bookkeeping and serves compressed
results, but invokes separate compiler processes on misses rather than
keeping a Blaze-style compiler IR heap alive. Its own jobserver uses the
host's available parallelism for compiler internals; four Cargo jobs bound
scheduled compile units, not every codegen thread or hashing probe. The
concurrent small check uses two jobs per client, four total, with only one
server. This does not validate ten simultaneous full builds.
([jobserver source](https://github.com/mozilla/sccache/blob/v0.19.0/src/jobserver.rs))

Budgets were set before compilation: 40 GiB sum of newly introduced regular
file allocation, 48 GiB available-space floor, 20 GiB sampled aggregate
process-tree physical footprint (including the server), 1,200 seconds per
normal phase, 1,800 seconds release and 90 minutes overall. A touched
128 MiB allocation increased the Darwin physical-footprint probe by
134,316,128 bytes. Monitoring failure terminates the owned process tree.
Sampled per-process sums are resource guards, not unique system-wide RAM.
Peak footprints were 3.12 GiB plain, 2.79 GiB population and 2.45 GiB
recipient. Sampled idle server footprint ranged from 6.47 to 27.92 MiB.
No Blaze-like 20 GiB persistent compiler heap appeared in this study.
Initial RAM capacity/load were recorded; an initial free-page snapshot was
not, and the later vm_stat snapshot cannot reconstruct that initial headroom.

## Platform and scope limits

The official release has macOS x86-64/arm64, Linux and Windows artifacts;
Rust/Cargo wrapper support is documented. Only native macOS arm64 was
measured here. Linux SSH/mount checks and Windows builds were not run.
No universal macOS build, packaging, update installation, GUI rendering,
long-term eviction churn or ten-worktree full-build stress test was run.
Different compilers/profiles/features change cache keys and reuse. The
observations do not establish arbitrary proc-macro filesystem correctness.


## Reproduction, evidence and cleanup

The unchanged gates passed in five complete workflows. No code/default
integration was added: the measured latency benefit comes with extra cache
storage and the demonstrated proc-macro correctness limit. Linux/Windows,
full-app semantic dependency drift, a plain warm/edit counterfactual and
debugger/dSYM quality remain gaps;
these are not implied by successful macOS headless tests.

Use a fresh owned worktree at the recorded source and short direct `a/`,
`b/`, `c/` paths. Read the ownership guards before reusing the snapshots.
All persistent experimental artifacts stay under that worktree; retain
native TMPDIR for tests. A process monitor must work in the execution
context; sandbox-denied process inspection is not permission to run blind.

```sh
E="$PWD/.local/sccache-study"
REPORT="$PWD/docs/performance/sccache-2026-11-12"
mkdir -p "$E/upstream" "$E/logs"
for file in "$REPORT"/harness/*.txt; do
  name=$(basename "$file" .txt)
  cp "$file" "$E/$name"
done
# Freeze the metadata used by setup.py rather than selecting a future release.
curl -fsSL https://api.github.com/repos/mozilla/sccache/releases/tags/v0.19.0 \
  -o "$E/upstream/release.json"
python3 "$E/setup.py"
# setup.py verifies GitHub's asset digest; additionally require the study pin.
python3 - "$E" <<'PY_PIN'
import hashlib, pathlib, sys
p = pathlib.Path(sys.argv[1]) / "upstream"
a = p / "sccache-v0.19.0-aarch64-apple-darwin.tar.gz"
assert hashlib.sha256(a.read_bytes()).hexdigest() == (
    "a35a0b7d622618774bc1bb8102d72562dfb21e619f88b153b370d7f89bc286af"
)
PY_PIN
cc -shared -O2 "$E/apfs.c" -o "$E/apfs.dylib"
python3 "$E/incremental-oracle.py" incremental
python3 "$E/flow-plain-executed.py" plain
python3 "$E/inventory.py" plain-complete-inventory.json
python3 "$E/sequence-executed.py"
# The actual small eviction check ran separately, after sequence completed:
python3 "$E/eviction.py" eviction
python3 "$E/analyze.py"
python3 "$E/inventory.py" before-cleanup-inventory.json
python3 "$E/cleanup.py"
cc -shared -O2 "$E/apfs.c" -o "$E/apfs.dylib"
python3 "$E/inventory.py" after-cleanup-inventory.json --unlink-probe
```

The copy loop removes one `.txt` suffix. The current `sequence.py`
snapshot folds eviction into its final steps; use that version *instead*
if desired, without repeating eviction. The executed sequence snapshot
preserves the actual ordering. Published script snapshots normalize trailing blank-line whitespace.
The plain-flow snapshot preserves its
initial errno handling; subsequent phases tightened physical-probe errors
to fail closed except disappearing processes. No monitor failure was
observed. Tiny probes' two-second command minimums mostly reflect the
inventory watchdog's scan, not two seconds of compiler CPU work.

Setup corrections retained in the evidence: an optional upstream
TESTING.md URL returned 404 before binary installation; the first server
config mistakenly used `[disk]` and was rejected before compilation.
`[cache.disk]` fixed it. Ordinary sandboxed live stats inspection could not
reach the UDS and returned zero defaults; authoritative phase statistics
were read in the same working context as the server, with PID/cache-location
checks. sccache leaves a stale socket after shutdown: cleanup verified
ECONNREFUSED before unlinking only that owned endpoint. The initial cleanup
assertion refused it before removing anything.

[results.json](results.json) contains phase exits, commands, byte inventories,
all reset cache statistics, test comparisons, probe outcomes, startup/idle
samples and cleanup proof. [test-outcomes.json](test-outcomes.json) retains
the complete control multiset; every arm has the same digest.
[phase-output.txt](phase-output.txt), [monitor.jsonl](monitor.jsonl),
[provenance.json](provenance.json) and [harness/](harness/) preserve compact
raw observations, source digests and reproduction logic. Full original
local logs remain in the owned `.local/sccache-study/` evidence archive;
public evidence substitutes workspace/home/temp labels for host paths.
Previous reports were not edited.

The owned disposable artifacts and all started servers were removed after
recording their sizes. The stale owned socket was connection-refused before
unlinking. Full raw logs were checksum-verified in a local gzip archive;
only compact sanitized evidence is committed. The retained local evidence snapshot is 4.72 MiB allocated; the committed report/evidence snapshot adds 0.47 MiB. Inventories precede their own final JSON/report updates, so those last small writes are not an exact self-inclusive total.
Available volume space returned to about 99 GiB. That shared-host reading
is not an exact attribution of physical bytes freed.
