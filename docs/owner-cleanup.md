# Cache cleanup

A cache name, category or `regenerable` hatch is an inspection lead, not
permission to delete. Review steers marked caches to **Cache cleanup**,
where the person decides each marked path on its own:

* **Skip** (the default): nothing happens to it.
* **Run the owner's command**: the exact allowlisted command shown for it,
  for its owner's whole actual scope. Offered only when that command
  resolved; see the allowlist below.
* **Delete files**: delete exactly that selected path directly, without
  its owner, by Trash or permanently. A separate destructive intent with
  its own disclosure and its own consent.

Ordinary Trash and Permanent removal still refuse recognized cache paths,
cache wrappers and opaque cache tags, including caches inside a selected
parent: deleting a cache's files happens only as a Delete files decision.

## How this reconciles with the earlier guard policy

| Before | Now | What still holds |
| --- | --- | --- |
| One Owner cleanup mode for the whole list; a missing CLI, unsupported version or failed probe refused the entire preview | Check owners returns one proposal per marked path; a failure is that proposal's status | Preview is read-only, off the UI thread, with the same probes, timeouts and shim/proxy refusals |
| Approval meant running every command shown | Each path is decided; Skip is the default and nothing is chosen for the person | An agent cannot decide for the person |
| Raw deletion of a recognized cache was always refused | Allowed only as an explicit Delete files decision for that exact path, never the owner's wider scope | Removal guards, protected folders, the capacity limit and Review's over-limit confirmation, permissions, and an in-use check |
| "No raw deletion fallback" | Unchanged: a missing, unsupported, refused, failed or cancelled owner command deletes nothing else | Deleting those files instead needs a new review and its own decision |
| Unknown caches were refused | Unknown owner, "not known to be disposable"; Delete files is offered with a runtime/installation warning | PwrDisk never labels an unknown cache disposable; agents inspect first and never suggest Delete files |
| Exports: marked paths only; raw scripts refuse caches | Unchanged | Owner scopes never widen an export |

## Allowlist

Commands below are argument lists, executed directly with an absolute
resolved executable. `ROOT` is the owner's queried configured root; it is
never inferred solely from a directory name. PwrDisk does not use `npx`,
`uvx`, a package download, a project script or a shell to find a missing CLI.

| Owner | Accepted stable versions | Query | Cleanup and actual scope |
| --- | --- | --- | --- |
| npm packages | 9–11 | `npm config get cache` | `npm cache clean --force --cache ROOT`; entire `ROOT/_cacache` only |
| npm npx | 11.2–11.x | `npm config get cache` | `npm cache npx rm --force --cache ROOT`; entire `ROOT/_npx` only |
| uv | 0.11.x | `uv cache dir` | `uv cache clean --cache-dir ROOT`; whole cache, using uv's exclusive cache lock, without `--force` |
| pip | 23–26 | `pip cache dir` | `pip --cache-dir ROOT cache purge`; HTTP and wheel entries in that cache; other files can remain |
| Cypress | 13–15 | `cypress cache path` | `CYPRESS_CACHE_FOLDER=ROOT cypress cache clear`; ALL cached binary versions, including ones other projects use |
| Playwright (installed JavaScript CLI) | 1.64.0–1.64.1 only | `playwright install --dry-run ffmpeg`, repeated with the reported root pinned | `PLAYWRIGHT_BROWSERS_PATH=ROOT playwright uninstall --all`; managed browser directories and `.links` reference files inside ROOT; unrelated files and branded Chrome/Edge remain |

The version is queried first. Unknown output, prereleases, unsupported
versions, non-executable launchers and failed configuration queries withhold
the owner command for that path; the proposal says which. A command missing from PwrDisk's inherited PATH means unavailable,
not that the software is uninstalled. Relative PATH entries are ignored. pip resolution also accepts `pip3` when
`pip` is unavailable.
Known version-manager shims (including shimmed `env node` interpreters) are
refused before version probing, because a probe can otherwise install a
missing runtime. Expose the actual installed CLI/runtime directly on PATH.
npm must resolve to its installed `npm-cli.js`; downloading proxy launchers
such as Corepack are unsupported. Playwright must resolve to its installed
JavaScript `cli.js`; Python, shell, .NET and downloading wrappers are not
audited adapters.
Relative PATH entries are removed from the captured execution environment;
non-UTF-8 environments fail closed instead of silently losing configuration.
Only macOS and Linux have execution adapters; Windows cache recognition
remains available, but shell-based Windows launchers are unsupported. Delete
files is not offered on Windows either: it is checked for programs using the
path right before it runs, and that check exists only on macOS and Linux, so
it would always be refused.

Queries run from the user's home with the app's environment, so user config
and environment overrides apply. Project-only configuration must be made
available explicitly to that environment; PwrDisk never searches projects or
evaluates Puppeteer JavaScript config. A stale/default selection that does
not match the queried root is refused, rather than redirected to a different
cache. Selecting a subtree expands to the owner's complete scope. Selecting
the npm wrapper previews its supported sibling caches separately.

| Inspection-only owner | Why automatic cleanup is unavailable |
| --- | --- |
| pnpm | `store prune` can also affect metadata, dlx/runtime caches and global virtual-store state outside a selected store; these scopes are not modeled here |
| Homebrew | `brew cleanup`, including `--prune=all`, also removes old installed formula versions and stale locks outside the selected cache |
| Puppeteer | The installed CLI loads executable configuration, disables `--path`, and `browsers clear` requires a stdin answer without a noninteractive flag; no verified adapter |
| codex-runtimes, gh, Electron, VS Code C++ and unknown caches | No verified owner adapter; inspect the owning installation and its supported lifecycle |

## Candidate audit (2026-11-04)

This audit reads official documentation and tagged source; it never runs
an installed tool, downloads a runtime, or cleans a real cache. The ranges
below describe the evidence checked, not a claim about unreviewed releases.
Mock executables exercise probes, approval revalidation and execution in
temporary trees. Only Playwright gains an adapter.

### Playwright: enabled for 1.64.0 and 1.64.1

The [browser guide](https://playwright.dev/docs/browsers#uninstall-browsers)
documents `uninstall --all` and `PLAYWRIGHT_BROWSERS_PATH`. In both audited
tags, the [registry](https://github.com/microsoft/playwright/blob/v1.64.1/packages/playwright-core/src/server/registry/index.ts)
and [CLI actions](https://github.com/microsoft/playwright/blob/v1.64.1/packages/playwright-core/src/cli/installActions.ts)
are identical to their
[1.64.0 registry](https://github.com/microsoft/playwright/blob/v1.64.0/packages/playwright-core/src/server/registry/index.ts)
and [1.64.0 actions](https://github.com/microsoft/playwright/blob/v1.64.0/packages/playwright-core/src/cli/installActions.ts).
The stable gate accepts only those two patch versions, not future 1.64.x.

* **Scope:** `uninstall --all` unlinks every entry of `ROOT/.links`, then
  removes immediate children of ROOT whose names match Playwright's managed
  browser directory prefixes, including old revisions and helper binaries.
  It leaves the root, `.links` directory and unrelated files. References are
  files inside this cache, not changes to the referenced packages. Branded
  Chrome/Edge installations are excluded. All projects sharing these
  browsers need to install them again. A subtree selection expands to ROOT.
* **Configuration:** Playwright's `getFromENV` reads
  `PLAYWRIGHT_BROWSERS_PATH`, then `npm_config_playwright_browsers_path`,
  then `npm_package_config_playwright_browsers_path`. Otherwise it uses
  `XDG_CACHE_HOME/ms-playwright` or `~/.cache/ms-playwright` on Linux and
  `~/Library/Caches/ms-playwright` on macOS. `0` selects the installed
  package's `.local-browsers`; relative values resolve against `INIT_CWD`
  (with the same npm fallbacks), or the command's working directory.
  PwrDisk queries from home and accepts only the actual absolute root the
  installed CLI prints. It never evaluates a project's test configuration.
* **Read-only query and pin:** `install --dry-run ffmpeg` reports exactly
  one install location without installing browsers, dependencies or links.
  Its `ffmpeg-<revision>` parent is the cache root. PwrDisk repeats that
  query with `PLAYWRIGHT_BROWSERS_PATH` set to the parsed root and requires
  identical complete output, so whitespace is preserved and embedded
  newlines cannot forge a different root. Malformed, multiple, relative or
  traversing locations fail closed. The cleanup keeps that override.
* **Locking/in use:** installation uses `ROOT/__dirlock`; uninstall takes
  no lock. PwrDisk refuses an existing lock, including a stale one, both
  during preview and just before execution. It never clears the lock.
  Existing process checks cover the owner and executables inside the cache.
  Playwright is identified by its exact resolved executable or known owner
  launcher names/paths, never another package's generic `cli.js` basename.
  Checks are best effort and cannot exclude a new installer/browser starting
  afterwards. Stop installers, tests and cached browsers first.
* **Limits:** upstream `removeFolders` catches filesystem removal errors,
  so exit success can leave browsers behind. Report the exit result and
  rescan, never promise every file or byte went. No raw removal fallback.
  [Debug logging](https://github.com/microsoft/playwright/blob/v1.64.1/packages/utils/debugLogger.ts)
  opens `DEBUG_FILE` even for probes; PwrDisk overrides it to empty. The
  [startup profiler](https://github.com/microsoft/playwright/blob/v1.64.1/packages/playwright-core/src/bootstrap.ts)
  can write `PW_PROFILE_DIR` on exit, so that is also overridden to empty.
  `PW_CLI_DISPLAY_VERSION` is also overridden to empty so the CLI reports
  its installed package version. All overrides are shown in the proposal.

### pnpm: inspection-only

The [10.x store guide](https://pnpm.io/10.x/cli/store) describes
`store prune`. The
[10.20.0 dispatcher](https://github.com/pnpm/pnpm/blob/v10.20.0/store/plugin-commands-store/src/store.ts),
[prune orchestration](https://github.com/pnpm/pnpm/blob/v10.20.0/store/plugin-commands-store/src/storePrune.ts)
and [store pruning](https://github.com/pnpm/pnpm/blob/v10.20.0/store/package-store/src/storeController/prune.ts)
show separate scopes: unused content/indexes and temporary store files,
`cacheDir/metadata*`, and expired dlx environments under `cacheDir`.
`--store-dir` alone does not constrain that metadata or dlx scope.
The [current guide](https://pnpm.io/cli/store) also describes versioned
metadata and (since 12.7.0) abandoned private pnpm/runtime copies.
No supported version range or store-only invocation is established here.

[Store/cache settings](https://pnpm.io/10.x/settings) independently choose
`storeDir` (including `PNPM_HOME`/`XDG_DATA_HOME`/platform defaults) and
`cacheDir` (`XDG_CACHE_HOME`/platform defaults); project/user settings can
change them. `store path` reports the versioned store, not every prune
scope. This audit does not establish a cache-wide exclusion lock or safe
coordination with store servers, installers and runtime management.
Redirecting cacheDir and every auxiliary state location into the selected
store was not verified. Keep pnpm inspection-only rather than silently
retarget those other configured caches.

### Homebrew: inspection-only

The [cleanup manual](https://docs.brew.sh/Manpage#cleanup-options-formulacask-)
and [5.0.0 command](https://github.com/Homebrew/brew/blob/5.0.0/Library/Homebrew/cmd/cleanup.rb)
/ [implementation](https://github.com/Homebrew/brew/blob/5.0.0/Library/Homebrew/cleanup.rb)
show that `cleanup`, `--prune=all` and `--scrub` also reach installed kegs,
logs, temporary Cellar entries, prefix directories, caches of installation
state and stale locks. `--prune-prefix` deliberately acts outside the cache.
There is no verified cache-only command or accepted version range.
`HOMEBREW_CACHE` selects downloads (`brew --cache` reports it), with the
macOS Library/Caches and Linux XDG defaults; it does not retarget those
other locations. Cleanup uses per-download locks and skips lock files it
cannot exclusively lock; this is not a cache-wide in-use guarantee and
cannot make the wider scope acceptable.

### Puppeteer: inspection-only

The [configuration guide](https://pptr.dev/guides/configuration) describes
`cacheDirectory` and `PUPPETEER_CACHE_DIR` (default `~/.cache/puppeteer`).
The installed CLI loads configuration before dispatch, including JavaScript
files found up the directory tree. In
[25.16.0's entry point](https://github.com/puppeteer/puppeteer/blob/puppeteer-v25.16.0/packages/puppeteer/src/node/cli.ts),
`allowCachePathOverride` is false: the `--path` option of the standalone
browser CLI is not an option of `puppeteer browsers clear`.
The [browser CLI](https://github.com/puppeteer/puppeteer/blob/puppeteer-v25.16.0/packages/browsers/src/CLI.ts)
asks for `y`/`yes` on stdin; there is no audited noninteractive flag.
[Cache.clear](https://github.com/puppeteer/puppeteer/blob/puppeteer-v25.16.0/packages/browsers/src/Cache.ts)
recursively removes the entire cache root, including metadata and unrelated
files there, without an exclusion lock or running-browser check. The older
24.26.1 CLI has the same confirmation behavior. No version range is
accepted: evaluating arbitrary configuration or scripting stdin consent is
not a read-only root query or an approved argument-list adapter.

### Windows launchers: inspection-only

Windows is an execution platform, not another cache owner: the tool's
version still decides command scope, and the owner's settings still decide
its root (for example npm's `cache`, `UV_CACHE_DIR`, `PIP_CACHE_DIR` and
`PLAYWRIGHT_BROWSERS_PATH`, with their Windows defaults). No Windows
launcher/version combination is accepted by this audit. A Unix adapter is
not evidence that a `.cmd`, PowerShell, Python or native launcher has the
same startup, quoting, in-use and cancellation behavior.

Microsoft's [CreateProcess documentation](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-createprocessw)
requires a command interpreter for batch files; Rust documents
[Windows argument handling](https://doc.rust-lang.org/std/process/struct.Command.html#method.arg)
and implements [batch escaping separately](https://github.com/rust-lang/rust/blob/1.97.0/library/std/src/sys/pal/windows/args.rs).
PwrDisk has neither an audited Windows launch/cancellation adapter nor a
Windows active-use check. Neither shelling through `cmd.exe` nor accepting a
native `.exe` alone establishes those guarantees. Recognition remains
available; execution and Delete files remain unavailable on Windows.

## Scope review (2026-11-05)

This follow-up keeps the accepted versions above unchanged. The ledger
below separates paths the command can mutate from references it reads and
installations that depend on them. A configured store/cache path alone is
not a complete write scope. Unknown locations or unverified coordination
keep a candidate inspection-only; they are never replaced with a common
parent, an environment redirect, a raw remover or an interactive answer.
No installed cleaner was run for this review.

### pnpm 10.20.0

| Location | Role and lifecycle | Protection needed before an adapter |
| --- | --- | --- |
| Versioned `storeDir` (`store path`) | `files` content, `index` package records, legacy JSON indexes and `tmp`; prune unlinks content according to hardlink counts, then removes affected package indexes | Resolve the actual versioned store and measure its entire allocation; hardlinks and clones do not imply reclaimed bytes |
| Independently configured `cacheDir` | Every immediate `metadata*` directory is removed; `dlx` contains executable environments, a `pkg` symlink and orphan installations, also pruned | Query, guard, disclose and measure this scope separately; `--store-dir` does not pin it |
| Store-server connection state | `server.json` under the versioned connection directory can select another process to serve store operations | Reject/deal with a live server using its actual protocol; matching a `pnpm` process name is insufficient |
| Projects and installed environments | Hardlinks into the content store; dlx environments themselves keep links alive | Do not infer project usage from one scan; neither project registrations nor link counts establish a cache-wide exclusion lock |
| pnpm home, runtime and global virtual-store state in later releases | The current guide adds global-store project registrations/links and abandoned private pnpm/runtime copies | Audit those releases separately, including installation state and reference updates; they are not covered by the 10.20.0 source |

The tagged [store path resolver](https://github.com/pnpm/pnpm/blob/v10.20.0/store/store-path/src/index.ts)
can create temporary files and directories and try hardlinks in the working
directory, home and volume-root candidates while choosing a default store.
Thus `store path` is not established as a side-effect-free discovery probe.
The [connection manager](https://github.com/pnpm/pnpm/blob/v10.20.0/store/store-connection-manager/src/index.ts)
can connect to a server or start one when configured. The
[dlx collector](https://github.com/pnpm/pnpm/blob/v10.20.0/store/plugin-commands-store/src/cleanExpiredDlxCache.ts)
removes expired environments and orphans independently of content pruning.
None supplies an audited exclusion mechanism across all those scopes.

The [directory defaults](https://github.com/pnpm/pnpm/blob/v10.20.0/config/config/src/dirs.ts)
independently use `PNPM_HOME`/`XDG_DATA_HOME` for data/store and
`XDG_CACHE_HOME` for metadata/dlx. macOS additionally uses `Library/pnpm`
and `Library/Caches/pnpm`; Windows uses `LOCALAPPDATA/pnpm` and
`LOCALAPPDATA/pnpm-cache`, with `.pnpm`/`.pnpm-cache` fallbacks.
PwrDisk now recognizes these store/cache locations as inspection leads,
including descendants. This does not classify the whole pnpm home or its
installed runtimes as disposable, evaluate project settings, or enable prune.

### Homebrew 5.0.0

| Location | Role and lifecycle | Why a cache-only approval cannot cover it |
| --- | --- | --- |
| `HOMEBREW_CACHE` | Formula/cask downloads, `downloads` targets, nested build caches, API sources, bootsnap, description/linkage databases and `.cleaned` marker | The marker/databases are writes too; printed deletion totals are not a measured volume delta |
| Cellar and dependency installations | Old kegs; default cleanup also calls autoremove for unneeded formula dependencies unless disabled | Installation removal is not download-cache cleanup; disabling autoremove still leaves old-keg cleanup |
| Prefix and Caskroom | Dangling prefix symlinks/empty directories, `.DS_Store` entries and Python site-package links | References outside the cache may be removed; `--prune-prefix` explicitly selects this wider scope |
| Logs and temporary Cellar | Independently configured log trees and incomplete installation state | Neither follows a `HOMEBREW_CACHE` override |
| Locks | Download locks and `HOMEBREW_LOCKS` formula/cask lock files; cleanup skips files it cannot exclusively lock | Individual locks do not exclude another installation across the complete operation |
| Brew's own installation | Old portable Ruby versions and obsolete vendored Bundler trees; cleanup can invoke `git clean` on those trees | The owner itself has a wider lifecycle than the selected cache |

These paths follow directly from tagged
[`clean!`, `cleanup_portable_ruby` and `autoremove`](https://github.com/Homebrew/brew/blob/5.0.0/Library/Homebrew/cleanup.rb).
The [command flags](https://github.com/Homebrew/brew/blob/5.0.0/Library/Homebrew/cmd/cleanup.rb)
do not establish a cache-only invocation: `--prune=all` changes age filtering
and `--scrub` changes download eligibility. Formula/cask loading and Ruby
startup must also be audited before even a discovery/dry-run adapter is
accepted. No Homebrew process or config was evaluated in this investigation.

### Playwright 1.64.0–1.64.1

| Location | Role and lifecycle | Existing adapter boundary |
| --- | --- | --- |
| Queried registry root | Immediate children with managed browser/helper prefixes, including old revisions | Full root allocation is charged even though unrelated entries can remain |
| `ROOT/.links/*` | Text files naming package installations that share the browsers | `uninstall --all` unlinks every reference before garbage collection; the external packages are not removed |
| Referenced packages and branded channels | Package code and Chrome/Edge installations elsewhere | They are not write scopes of this invocation; tests sharing the removed browsers need reinstalling |
| `ROOT/__dirlock` | Installer's exclusive directory lock | Uninstall does not acquire it; PwrDisk refuses any existing lock and never clears one |
| Logs, profiler and displayed version | Inherited startup settings can write elsewhere or mask the version | Existing empty overrides remain applied to every probe and command |

The tagged registry linked above is the scope authority. A root named by
`npm_config_playwright_browsers_path` or
`npm_package_config_playwright_browsers_path` is now also an inspection
hint, so these supported environment aliases cannot bypass raw-removal
inspection. Hints are conservative even when shadowed by a higher-priority
setting; the installed CLI's pinned dry-run still decides which root can
actually be cleaned. `0` and relative roots are not guessed by recognition.
The lock and process checks remain best effort with a race after the last
check; they do not claim atomic exclusion. No new versions are enabled.

### Puppeteer 25.16.0

| Location | Role and lifecycle | Adapter blocker |
| --- | --- | --- |
| `cacheDirectory` / `PUPPETEER_CACHE_DIR` | Defaults to `~/.cache/puppeteer`, independently of `XDG_CACHE_HOME`; the whole root is removed by `clear` | Every entry, including unrelated files, belongs in the capacity and consent scope |
| Browser/platform/build installation directories | Chrome, headless shell and Firefox binaries used by any sharing project | Close browsers and installers; neither clear nor single-build uninstall supplies a cache-wide lock |
| Browser `.metadata` | Alias and executable-path registrations; single-build uninstall can rewrite these before removing the build | A selected build directory alone does not cover metadata in its parent |
| External executable registrations | Metadata can name binaries outside the cache | Clear removes the registration, not the external executable; this does not prove that other cached installations are unused |
| Configuration files and installed CLI/runtime | Configuration discovery walks ancestors and can evaluate JavaScript before dispatch; installed CLI disables `--path` | An environment override does not prevent config execution; the interactive clear prompt has no audited noninteractive flag |

See tagged [configuration resolution](https://github.com/puppeteer/puppeteer/blob/puppeteer-v25.16.0/packages/puppeteer/src/getConfiguration.ts)
and [`Cache.clear`/`Cache.uninstall`](https://github.com/puppeteer/puppeteer/blob/puppeteer-v25.16.0/packages/browsers/src/Cache.ts).
PwrDisk does not substitute the standalone `@puppeteer/browsers` CLI, an API
script, `npx`, or piped `yes`. Recognizing the default under home even with
`XDG_CACHE_HOME` set fixes an inspection gap without evaluating config.

### Windows: structured execution and active-use assessment

A shell is not needed for every possible Windows adapter. A verified native
`uv.exe`, or a verified `node.exe` plus an absolute installed JavaScript
entry point, could be expressed as a fixed executable and argument vector.
That is a possible implementation route, not an enabled adapter. `.cmd`,
`.bat`, PowerShell, Corepack and version-manager dispatch are not translated
or interpreted. Python launcher behavior requires its own version audit.
The current Windows refusal applies to native executables as well as batch
launchers, and fixture tests hold that boundary.

A Windows implementation must establish all of these before offering a run:

* **Invocation identity:** resolve the actual installed runtime and entry
  point without downloading, bind both identities to the preview, and
  revalidate both. Preserve UTF-16 argument boundaries, quotes, empty values,
  environment-key case rules and absolute paths. Unix `/dev/null` logging
  overrides must be replaced with audited Windows equivalents. No caller
  supplies executable names, script bodies or raw command lines.
* **Cancellation before any side effect:** create a process suspended,
  assign it to a Job Object configured to kill members when closed, then
  resume it. Fail assignment before running; never retry outside the job.
  Keep descendants in the job and test nested-job restrictions. Killing
  only `Child`, or requesting a console process group, cannot establish
  descendant termination. Bound output draining when descendants retain
  pipe handles. Microsoft's [Job Objects](https://learn.microsoft.com/en-us/windows/win32/procthread/job-objects),
  [assignment requirements](https://learn.microsoft.com/en-us/windows/win32/api/jobapi2/nf-jobapi2-assignprocesstojobobject)
  and [creation flags](https://learn.microsoft.com/en-us/windows/win32/procthread/process-creation-flags)
  define the relevant primitives.
* **Active use:** [QueryFullProcessImageNameW](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-queryfullprocessimagenamew)
  identifies an executable image, not its script arguments, current
  directory or every open cache file. Access-denied and missing process
  information cannot mean idle. The pinned sysinfo 0.31.4 Windows backend
  has separate image, command-line and working-directory retrieval paths;
  merely adding Windows to the Unix cfg would not establish coverage.
  [Restart Manager registration](https://learn.microsoft.com/en-us/windows/win32/api/restartmanager/nf-restartmanager-rmregisterresources)
  accepts individual full filenames, services or process identities, not
  a recursive directory exclusion lock. It could supplement a bounded
  file inventory; it must never shut down applications for PwrDisk.
* **Scope identity:** Windows case/alias spellings, reparse points and volume
  boundaries need the same guard and measured full-scope accounting as
  Unix. A process starting after inspection remains a race, so owner locks
  and disclosed best-effort checks are separate requirements.

There is no Windows execution or Delete files adapter in this change.
macOS/Linux checks now also match an owner's reported executable image,
independently of its command arguments. An isolated Linux live-process oracle
uses a harmless native sleeper with a changed `argv[0]`; no cleaner runs.
Recognition properties and mock candidates run without installed tools.
The owner-disabled candidates refuse before executable resolution even if
reached without a recognition hint. Existing single-use approvals,
per-path decisions, fresh full-scope limits, cancellation, no fallback,
rescans and measured space reporting are unchanged.

## Proposals

**Check owners** (MCP: `pwrdisk_removal_preview` with `mode:
"owner_cleanup"` and no decisions) resolves each marked path on a worker
without cleaning. Each proposal carries:

* the path, its size, and why it is treated as a cache;
* each resolved operation: owner and version, absolute executable,
  argument list with boundaries, environment overrides, working directory,
  configured cache root and actual scope, whether that scope is wider than
  the selection, effects, limitations and recoverability (none through
  PwrDisk or Trash);
* otherwise the problem, in one vocabulary used everywhere: not found on
  PATH, unsupported launcher, unsupported version (naming the versions that
  run), version check failed, configuration check failed, no cleanup
  command, unknown owner, not the configured cache, refused by guards;
* whether Delete files is offered and what it means: that the owner is not
  involved, any recognized caches inside the path (a bounded look, three
  levels deep), and for an unknown owner that PwrDisk does not know the
  path is disposable.

A path the removal guards refuse offers neither action. A command whose
whole scope fails the guards, crosses a volume, contains an external link,
overlaps another command's scope or cannot be measured is withdrawn from
the proposals that asked for it, with the reason.

## Decisions and confirmation

Decisions that would act on the same files twice are refused: a path left
on Skip, or given another action, may not lie inside a chosen command's
scope or a path being deleted. Choose the same action for both. Paths
chosen for the same owner scope run its command once.

In the window, Review shows each proposal under its mark with Skip, Run
and Delete files; Delete files by Trash or Permanently is chosen once for
the run (Trash by default where it exists). The confirmation lists each
command verbatim, each path deleted directly with its consequence, and
what is skipped. Deleting files has its own tick; the button stays
disabled without it, even when the commands alone could be confirmed.
Changing a decision clears the tick.

Over MCP the agent shows every proposal and asks the person per path, then
previews again with `decisions` (every marked path once: `skip`, `owner` or
`delete_files`) and `delete_files_mode` (`trash` or `permanent`) when any
path deletes files. Without decisions nothing can be confirmed. A decided
preview is single-use, bound to the scan, marks and decisions, expires
within five minutes, and has its own confirmation text naming each
intent: `RUN OWNER CLEANUP`, `DELETE CACHE FILES`, `TRASH CACHE FILES`, or
`RUN OWNER CLEANUP AND DELETE CACHE FILES` / `... AND TRASH CACHE FILES`.
`pwrdisk_remove` needs the preview ID, its exact ordered paths, mode
`owner_cleanup`, that text and the client's interactive confirmation.
Owner approval never confirms deleting files. The agent's decisions run;
the window's own choices never stand in for them. Computer use cannot
approve.

## Execution and results

The usual root/home/system/mount/symlink protections apply to every
step. The configured capacity limit (5% per volume by default) charges each
command's full scope and each deleted path; the worker re-measures first,
and only that ceiling refuses the whole run. A step that can no longer be
measured is refused on its own.

Steps run one at a time. Before each owner command the worker re-reads the
executable's identity and re-probes version and configured root against
the captured environment, rechecks the scope, and checks whether the owner
is running; before each deletion it checks for a process running from,
inside or naming the path, or for a recognized cache any running instance
of its owner. An owner counts as running under any name it launches by:
npm's and npx's node entry points, `pip3` and versioned names such as
`pip3.12`, and uv's `uvx`. Detection is best effort; close owning tools
first. uv's own lock adds coordination; known `UV_LINK_MODE=symlink` is
refused because installed environments can depend on cache files.

A failed step reports why and the run moves on to the next. Probes time
out after ten seconds; commands after ten minutes. Stop kills the current
owner process group; that step reads cancelled, may be partly done, and the
rest read not reached. Nothing is retried automatically. npm logging and
rotation, pip log writes and Playwright log/profile writes are disabled for
these invocations so files
outside the approved scope stay untouched.

Every marked path gets a result: succeeded, failed with its problem and
detail, cancelled, skipped, or not reached. A succeeded command means the
owner exited successfully, not that its root or all its allocation went.
Results keep scope allocation (the scan's size, an upper bound) separate
from the volume's measured availability before and after, which can fall.
Other activity, open files, hardlinks, APFS clones and snapshots can keep
blocks; PwrDisk never alters snapshots. Cache cleanup always rescans,
including after a partial failure, and never marks a retained cache root as
deleted.

## Exports

Exports remain limited to the marked paths, even when a command's scope
is wider or a path is chosen for Delete files. Lists and agent prompts
never inherit a wider scope. Save/copy raw deletion scripts refuse Cache
cleanup mode and recognized cache selections; an export cannot stand in
for an owner command or for a decision made in Review. Tags and owner
layouts on ancestors protect contained subdirectories and files, including
caches relocated through CLI settings.

## Upstream scope evidence

- [npm cache documentation](https://docs.npmjs.com/cli/cache/),
  [npm 11.2 release notes](https://github.com/npm/cli/releases/tag/v11.2.0),
  [npm 11.6.2 cache implementation](https://github.com/npm/cli/blob/v11.6.2/lib/commands/cache.js)
  and [log rotation](https://github.com/npm/cli/blob/v11.6.2/lib/utils/log-file.js).
- [uv cache documentation](https://docs.astral.sh/uv/concepts/cache/) and
  [0.11.0 cleanup lock handling](https://github.com/astral-sh/uv/blob/0.11.0/crates/uv/src/commands/cache_clean.rs).
  Older uv releases do not provide that same cache-wide coordination.
- [pip cache](https://pip.pypa.io/en/stable/cli/pip_cache/) and
  [Cypress cache location and clearing](https://docs.cypress.io/app/references/advanced-installation).
- [pnpm store scope](https://pnpm.io/cli/store),
  [Homebrew cleanup](https://docs.brew.sh/Manpage#cleanup-options-formulacask-),
  [Playwright uninstall](https://playwright.dev/docs/browsers#uninstall-browsers),
  [Puppeteer browser CLI implementation](https://github.com/puppeteer/puppeteer/blob/puppeteer-v24.26.1/packages/browsers/src/CLI.ts).
