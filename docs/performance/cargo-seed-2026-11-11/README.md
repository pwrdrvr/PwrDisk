# Cargo target seeding on PwrDisk — November 11, 2026

**Verdict: useful as opt-in acceleration for a fresh worktree on this tested
Cargo/APFS setup.** Both PwrGit seed modes completed the unchanged full
PwrDisk workflow. Bounded mode reduced its measured wall time by 47.4%;
strict mode by 23.6%, before their roughly four-second checkout/seed cost.
Neither mode provides ongoing sharing of builds made after seeding.

This answers the full-app question left open by the
[November 9 Blaze study](../blaze-2026-11-09/README.md). Blaze never completed
that GUI workflow within the memory allowance. Its interrupted times are
not a completed-build comparison. PwrGit seeding reuses Cargo artifacts
while retaining the project's compiler; it does not substitute Blaze's
compiler, backend or daemon. No integration or global default was added.

## Controlled source and environment

All three fresh targets used PwrDisk commit
`6a7bad7e5fc53fd54bdbb271fc5b68f789b2d597` (0.22.0), the same source as the
14m38s November 9 control and still origin/main when this study ran.
All checkouts stayed clean and their Cargo.lock hashes matched.
The new scratch-donor run is the contemporaneous plain control; the
historical 878.231-second run is context, not a substitute measurement.

Host: macOS 27.2 (26B123), arm64, APFS, 32 GiB RAM, 12 logical CPUs.
The operator host was shared: phase-end one-minute load averages ranged
from 4.86 to 7.62. There was one serial observation per arm, in the order
plain, bounded, strict. OS file caches were not cleared. These are measured
wall times, not an idle-host estimate or a statistical speedup guarantee.

Project pin `1.97` resolved to rustc 1.97.1 (8bab26f4f; LLVM 22.1.6) and
exactly `cargo 1.97.1 (c980f4866 2026-06-30)`. That is the tool's allowed
Cargo string. A rust-toolchain.toml pin is not itself a rejection: the exact
tool source probes the compiler selected inside each checkout. No pin or
gate was patched. Successful seed JSON omitted `reason`; neither invocation
returned `fingerprint-unproven`.

The unchanged utility came from PwrGit PR #529 commit
`d2e3a806aa4ba384d6e1c30756853a529b6f7036`, `tools/pwrgit-share`.
Its own installed Rust 1.94.1 pin built `cargo build --locked --release`
in 6.63 seconds wall time. This utility compiler is separate from the
unchanged compiler used for every PwrDisk arm. The installed CLI Git runtime
was `git version 2.56.0-dev-pwrgit.4`; no desktop app was launched.
([tool source](https://github.com/pwrdrvr/PwrGit/tree/d2e3a806aa4ba384d6e1c30756853a529b6f7036/tools/pwrgit-share),
[existing PwrGit research](https://github.com/pwrdrvr/PwrGit/blob/d2e3a806aa4ba384d6e1c30756853a529b6f7036/tools/pwrgit-share/BENCHMARK.md))

## Complete serial workflow

Every phase used four Cargo jobs, the same private CARGO_HOME and offline
source access. Only registry/ and git/ sources were cloned into that home;
user Cargo configuration was neither copied nor changed. Dev/release profiles, features, compiler flags, target and
test selection were unchanged. In particular, release retained thin LTO,
one codegen unit and overflow checks. Dependency source downloads were
zero; "cold" means an empty target, not an empty source registry or OS cache.

```sh
cargo build --workspace --offline --locked
cargo xtask lint
cargo xtask test
cargo build --release --offline --locked
```

Lint includes rustfmt checking and Clippy over the workspace's all-targets
configuration, then all features. Tests include the whole workspace, the
app's screenshots-feature selection, and both Python script suites.
These are headless checks. Release here means the normal release-profile
build; no signing, packaging, publication or launch of a GUI occurred.


| Phase | Plain cold | Bounded seed | Strict seed |
| --- | ---: | ---: | ---: |

| Workspace development build | 263.124 s | 42.087 s | 133.264 s |

| Full lint gate | 92.369 s | 29.805 s | 59.798 s |

| Full test gate | 378.830 s | 320.212 s | 378.782 s |

| Normal release build | 175.409 s | 86.406 s | 123.111 s |

| **Workflow total** | **909.732 s** | **478.510 s** | **694.955 s** |

| Checkout + seed | Not separately timed | 0.186 + 4.048 s | 0.148 + 4.209 s |


Plain finished in **15m09.7s**; bounded in **7m58.5s**, or **8m02.7s**
including checkout and seed; strict in **11m35.0s**, or **11m39.3s** with
setup. Compared with the historical 14m38s control, the corresponding
workflow-only times are 45.5% and 20.9% lower, but the fresh control is the
fairer comparison because it shares this experiment's environment.

The donor was a scratch clone built through all four phases. An already
built, compatible donor is a prerequisite for these recipient savings.
Creating a donor solely for one recipient adds its entire 909.732-second
workflow; this does not make a first-ever checkout cheaper. The one-time
utility build and dependency-home preparation are also outside recipient
phase totals and included in storage accounting. Cache/source preparation
and plain clone creation were not separately timed.

## Compilation reuse and seed JSON

Counts below are Cargo `Compiling` log records (packages per build pass),
not rustc process counts or seed units. Some packages occur in multiple
passes/profiles. The development rows also equal distinct package counts.


| Phase | Plain | Bounded | Strict |
| --- | ---: | ---: | ---: |

| dev | 447 | 12 | 85 |

| lint | 65 | 6 | 32 |

| test | 42 | 13 | 25 |

| release | 447 | 12 | 85 |

| **Sum of Compiling records** | 1001 | 43 | 227 |


Clippy additionally logged **374 / 9 / 59 Checking records** for
plain / bounded / strict. Both Clippy passes still ran. Workspace crates
always rebuilt; no tests were removed to obtain the reduction.

Full [bounded JSON](bounded-seed.json) and [strict JSON](strict-seed.json)
are retained. `reason` is absent on both successful seed reports:


| Field | Bounded | Strict |
| --- | ---: | ---: |

| `unitsSeeded` | 1659 | 1307 |

| `packagesSeeded` | 455 | 380 |

| `cloneCount` | 10976 | 8314 |

| `copyCount` | 0 | 0 |

| `rewrittenCount` | 1541 | 1233 |

| `unitsSeededWithDonorOutDirPaths` | 32 | 0 |

| `unitsSeededWithDonorDebugMaps` | 9 | 0 |


Bounded rejected one donor-path package and five dependents, plus six
workspace and 75 unbuilt packages. Strict rejected 16 donor-path packages
and 65 dependents, plus the same workspace/unbuilt categories. Counts are
not a claim that a single package has only one compiled unit.

## Correctness and where the time goes

All three arms passed the complete lint/test gates. The multiset of every
Rust test name and outcome matched the plain arm, as did every suite
summary and both Python suites (five and six tests). There were 1,073
successful Rust result records, including two isolated child-process
results, and the same nine normal ignored tests. No ignore was added.
The new plain arm also matched the historical control's test outcomes.

The principal suites passed: app 450, core 493, link 18, MCP 83, helper
integration four, xtask 14, doc test one, and screenshots selection eight.
Dev and release `pwrdisk --help` and `disktree-scan --help` exited zero in
all three arms, with identical help output: 12 checks. A private HOME
contained startup logs. Binary hashes were recorded, not required to
match across different absolute source paths.

Cold workspace test compilation reported 1m53s; bounded 56.28s; strict
1m44s. Additional screenshots compilation took 23.13 / 22.89 / 23.75s.
Core execution itself took 222.68 / 221.25 / 230.09s, including real APFS
image fixtures. These log durations are explanatory, not an exhaustive
wall-time partition. Fewer compiled packages did not remove fixture I/O,
workspace compilation or final linking. Strict's test gate therefore took
essentially the same wall time as cold despite its reuse.

**The donor remained present during these measured builds/tests.**
Bounded mode permits the reported OUT_DIR and debug-map references. The
upstream tool fixture demonstrates that generated code which opens a
compiled-in donor OUT_DIR can fail after that donor disappears. This study
does not establish donor-independent bounded builds or runtime behavior.
Strict rejects those exceptions. Passing these tests is evidence for this
commit/workflow, not a proof for arbitrary dependencies or future edits.
([mode rules and donor-path fixture](https://github.com/pwrdrvr/PwrGit/blob/d2e3a806aa4ba384d6e1c30756853a529b6f7036/tools/pwrgit-share/README.md))

## APFS storage, including overhead

The cold row is measured **before seeding**. Later clones lower the donor's
own private-byte count, so comparing the final donor with a new recipient
would obscure the original baseline. Decimal GB is used below; exact byte
counts and intermediate snapshots are in [results.json](results.json).


| Completed target | Logical GB | Reported allocated GB | APFS private GB |
| --- | ---: | ---: | ---: |

| plain | 8.019879 | 8.073679 | 7.468605 |

| bounded | 8.019882 | 8.070881 | 2.641060 |

| strict | 8.019879 | 8.081981 | 4.505448 |


The bounded recipient has **64.6% fewer reported private data bytes** than
the pre-seed cold target; strict has **39.7% fewer**. The logical output
footprint remains about 8.02 GB in every arm. This is real copy-on-write
artifact sharing, with zero ordinary copies reported by the seed tool;
it does not eliminate worktree-private workspace outputs or future edits.

The temporary dependency home accounts for approximately 2.074 GB logical
and 2.285 GB reported allocation. It was itself APFS-cloned from existing
registry/git sources; only about 0.2 MB was initially private. The tool's
build target adds about 48 MB reported allocation. Scratch source checkouts,
Git metadata, the donor's full target, both recipients, setup home, tool
source/build and evidence are included in the aggregate, not hidden in an
uncounted shared service or cache.


Before disposal, all owned trees totaled **24.408 GiB logical**, **24.759 GiB reported allocated**, and **9.177 GiB per-file APFS private data**.


Measurement walks regular files, deduplicates hardlinks by device/inode,
and sums length, `st_blocks * 512`, `ATTR_FILE_DATAALLOCSIZE` and
`ATTR_CMNEXT_PRIVATESIZE`. The APFS fields were available for every measured
regular file. Symlinks are not followed. Directory/filesystem metadata and
transient native test fixtures are outside these file totals; the latter
clean themselves up and were covered by the volume free-space guard.

**Private bytes are not a directory's true unique physical footprint.**
Cargo itself clones some incremental outputs into deps, so files created
locally can also have zero private bytes. Reported allocation counts shared
extents repeatedly; summed private bytes miss shared extents entirely.
Neither yields the union of physical extents across these trees. The
measurements establish lower recipient private allocation and actual CoW
sharing, but do not establish an exact total physical-disk saving or a
10-worktree extrapolation. Free-space changes on this shared host are not
attributed entirely to the experiment.

## Scope, budgets and disposal

The bounded harness allowed 40 GiB of summed reported file allocation,
kept 48 GiB available, and sampled aggregate process-tree physical footprint
against 20 GiB every two seconds. Phase limits were 1,200 seconds, release
1,800 seconds, and each arm 3,600 seconds. All completed without a guard
stop. Peak sampled process-tree footprint was 3.72 / 1.85 / 3.57 GiB for
plain / bounded / strict; this is a conservative per-process sum, not
unique system-wide RAM. Seed commands had a 300-second timeout; checkout had 120 seconds.
The four-second seed runs were not separately memory-profiled.

Only owned d/, b/, s/ scratch checkouts and this experiment's tool clone,
tool build target, temporary Cargo home, CLI home and probe binary were
removed. All source checkouts were clean first. Existing checkouts, caches,
user files and earlier Blaze evidence were preserved. About 1.9 MiB of new
raw logs, JSON and harness source remain in
`target/cargo-seed-2026-11-11/`; compiled apps and caches do not remain.
Available volume space returned to approximately 104 GiB. This is an
observed volume reading, not an exact attribution of bytes freed.

Setup exclusions: initial nested utility workspace discovery failed before
compilation. An owned virtual workspace containing only the unchanged
utility, with a verbatim copy of its lockfile, resolved that boundary.
The first control monitor was sandbox-blocked on process inspection; its
owned partial target was removed, then the measured control restarted with
working monitoring. Neither setup attempt is included in the phase table.

Linux and Windows builds were not measured. The tool source uses macOS
clonefile or Linux FICLONE; a compatible filesystem and same-volume cloning
are prerequisites, and other platforms have no clone primitive in this
revision. Different Cargo versions must fail closed until proven by the
tool's upstream gate. No unproven-version bypass was used.


## Reproduction

Use a disposable owned directory on APFS. Keep the source worktree paths
short enough for macOS Unix sockets. In this run the assigned workspace
held d/ (donor/plain), b/ (bounded) and s/ (strict). Native TMPDIR must remain
unchanged: moving it invalidates the protected-temp oracle. Source paths,
compiler, Cargo home and config must match between donor and recipients.

```sh
# Run inside the owned PwrDisk workspace. PWRGIT_SOURCE is read only.
SOURCE=6a7bad7e5fc53fd54bdbb271fc5b68f789b2d597
TOOL=d2e3a806aa4ba384d6e1c30756853a529b6f7036
E="$PWD/target/cargo-seed-2026-11-11"
GIT=/Applications/PwrGit.app/Contents/Resources/git/bin/git
mkdir -p "$E/cargo-home" "$E/logs"
cp -cR "$HOME/.cargo/registry" "$E/cargo-home/registry"
cp -cR "$HOME/.cargo/git" "$E/cargo-home/git"
git clone --shared --no-checkout "$PWRGIT_SOURCE" "$E/tool-git"
git -C "$E/tool-git" sparse-checkout set tools/pwrgit-share
git -C "$E/tool-git" checkout --detach "$TOOL"
# Nested standalone crate boundary, no tracked upstream file changes:
cat > "$E/Cargo.toml" <<'TOML'
[workspace]
members = ["tool-git/tools/pwrgit-share"]
resolver = "3"
TOML
cp "$E/tool-git/tools/pwrgit-share/Cargo.lock" "$E/Cargo.lock"
(cd "$E/tool-git/tools/pwrgit-share" &&
 CARGO_HOME="$E/cargo-home" CARGO_TARGET_DIR="$E/tool-target" \
 CARGO_BUILD_JOBS=4 CARGO_NET_OFFLINE=true \
 cargo build --locked --release)
"$GIT" clone --shared --no-checkout "$PWD" "$PWD/d"
"$GIT" -C d checkout --detach "$SOURCE"
# Copy the retained harness snapshots into the owned experiment directory:
REPORT="$PWD/docs/performance/cargo-seed-2026-11-11"
for name in flow.py next.py inventory.py apfs.c analyze.py \
 check-outputs.py cleanup.py
do
 cp "$REPORT/$name.txt" "$E/$name"
done
printf '%s\n' \
 'Owned PwrDisk pwrgit-share evaluation 2026-11-11 mw8odhs6' > "$E/OWNER.txt"
cc -shared -O2 "$E/apfs.c" -o "$E/apfs.dylib"
# Guarded plain/donor workflow, then bounded and strict in that order:
python3 "$E/flow.py" plain
python3 "$E/next.py"
python3 "$E/analyze.py"
python3 "$E/check-outputs.py"
python3 "$E/inventory.py" before-cleanup-inventory.json
# After collecting results, remove only this run's owned disposable data:
python3 "$E/cleanup.py"
python3 "$E/inventory.py" after-cleanup-inventory.json
rm "$E/apfs.dylib"
```

No recipient target may exist before seeding. Inspect JSON `unitsSeeded`
and `reason`; exit zero alone can mean an intentional skip. The tool takes
nonblocking donor locks and skips a busy donor. No concurrent build or
lock-contention stress test was performed here.

The exact local harness snapshots are [workflow](flow.py.txt),
[checkout/seed sequencing](next.py.txt), [file inventory](inventory.py.txt),
[APFS probe](apfs.c.txt), [result comparison](analyze.py.txt),
[CLI checks](check-outputs.py.txt) and [owned cleanup](cleanup.py.txt).
The commands above copy them without `.txt` into E and set the exact
marker expected by the snapshots. The sequencing script creates each
recipient with `git -c checkout.reflink=true worktree add`, runs
`pwrgit-share cargo seed --from d/target --to b/target --json` (or s/
with `--strict`), snapshots its bytes, and only then starts its workflow.
Snapshots deliberately retain their ownership checks and this host's
installed Git path; review those before reuse. Do not run cleanup against
any directory you did not create for the experiment.

## Ten drifting worktrees and builds made later

**This seed tool does not share later builds.** Every recipient owns its
new output; each future worktree can seed from a compatible, quiescent
snapshot, but existing recipients do not receive subsequent donor units.
Drifting dependency versions reduce overlap and preserve more variants.
No ten-worktree, dependency-drift or concurrent-build matrix was run here.

A local **sccache** result cache can serve outputs produced after worktree
creation without retaining a Blaze-style compiler IR daemon. It still runs
compilers on misses, restores separate outputs and adds cache storage.
Rust support excludes crates invoking the system linker; incremental crates
are not cached. `SCCACHE_ALLOW_INCREMENTAL=1` can leave those uncached while
preserving project profiles. Proc-macro filesystem inputs and cross-directory
path normalization need correctness checks: reused compiled-in runtime paths
can still name another checkout. This is a candidate for measurement, not a
PwrDisk speed/disk result. The documented local cache defaults to 10 GB,
with a configurable cap and one server per local cache.
([Rust caveats](https://github.com/mozilla/sccache/blob/main/docs/Rust.md),
[path normalization](https://github.com/mozilla/sccache#normalizing-paths-with-sccache_basedirs),
[local storage](https://github.com/mozilla/sccache/blob/main/docs/Local.md))

A **shared CARGO_TARGET_DIR** or shared intermediate build directory can
reuse compatible later dependency artifacts without a persistent compiler
daemon. Cargo build locks can serialize users of the same build directory;
workspace changes can cause invalidation/rebuilds. A shared final target
also lets another checkout replace the named executable. This was not
benchmarked here and is not a proposed repository default.
([locking work](https://github.com/rust-lang/cargo/issues/4282),
[Cargo build cache](https://doc.rust-lang.org/cargo/reference/build-cache.html))

Cargo's **build-dir separation** can keep final outputs local while moving
intermediate artifacts elsewhere. A `{workspace-path-hash}` template still
makes separate per-worktree caches; centralizing their location does not
share compiled units. The finer-grained layout and per-user compiled-artifact
cache remain distinct upstream work, not a measured shared CAS on our pin.
([build-dir config](https://doc.rust-lang.org/cargo/reference/config.html#buildbuild-dir),
[layout tracking](https://github.com/rust-lang/cargo/issues/15010),
[per-user artifact cache](https://github.com/rust-lang/cargo/issues/5931))

No option guarantees that ten simultaneous four-job builds fit in this
host's RAM. Sharing files does not cap active compilers or linkers. Aggregate
concurrency needs a budget across worktrees. The measured seed workflows
were serial, with four jobs total, and needed far less process memory than
the earlier interrupted Blaze GUI attempt. No remote artifacts were uploaded,
service provisioned, global configuration changed or duplicate seed tool
implemented.
