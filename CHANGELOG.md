# Changelog

Each release's notes are its `## vX.Y.Z` section here: `cargo xtask
release-check` refuses a tag without one, and release.yml publishes the
section as the GitHub release's notes and as the notes Velopack shows beside
an update. Keep new entries under Unreleased until the version is cut.

## Unreleased

## v0.22.0

- Size units - Choose and save Platform default, Decimal or Binary in
  Settings > General. Platform default uses decimal units on macOS and
  binary units on Windows and Linux; the choice applies throughout the
  window, exported prompts and MCP reports.
- macOS disk accounting - Include Data-only files in whole-disk scans
  without counting mapped aliases twice. Label readable file allocation
  Scanned, separately from the OS disk meter's Used and Available figures.
  Available includes purgeable space; recovery projections avoid counting
  that space again.
- Messaging approval - Approve concrete MCP removal proposals through
  supported clients' human confirmation controls, including PwrAgent
  messaging. Each proposal names its paths, action, expiry and limits;
  unsupported clients can use native Review. A linked local window and
  explicit human approval remain required.
- Removal previews - Reject MCP removal when a selected object, its parent
  or scan root has been replaced since the preview. Expired, cancelled or
  replayed approvals cannot start a new write; changed selections require
  a fresh proposal and approval.

## v0.21.0

- APFS reclaim estimates - Use measured private allocation for the reclaim
  lower bound projected onto the disk meter. Shared clone data contributes
  only to the possible upper bound, with a partial estimate even when every
  known reference to a clone ID is selected. Modified clones can retain
  shared blocks under a different ID, including outside the scanned root;
  selecting the known group no longer promises those blocks will be freed.
- Agent ranking - Keep measured private allocation available to MCP ranking
  when only shared-extent relationships are uncertain. Missing metadata,
  incomplete scans and inconsistent observations still prevent a known
  private-byte ranking value. Tile weights and hardlink accounting stay
  unchanged.
- macOS preflight - Use bulk directory metadata for cache inspection to
  reduce per-file work on large selections. Preserve deep-path support,
  relocated-cache recognition, mount boundaries, symlink handling and
  inspection limits; capacity remeasurement keeps the scanner's accounting.
- Window restoration - Grow saved windows to the requested minimum size,
  capped by the destination display, before fitting them on screen. Frames
  saved on a scaled laptop or with a tiny size now restore at a usable size
  when the display has room.

## v0.20.0

- Output inventory - Add read-only `pwrdisk_find_outputs` for build output,
  installed dependencies, caches and temporary, scratch, log and Trash
  locations. Filter by type or language family, evidence, size and newest
  file-write age; sort oldest first or largest first. Cargo output requires
  metadata and Git tracking checks rather than a directory name alone.
- Inventory context - Keep nested output types visible, identify enclosing
  matches and count overlapping listed bytes once. Report evidence strength,
  scan limitations and inspection bounds. A result identifies output rather
  than proving it is safe to delete or promising recovered space. Linked
  queries preserve the window's navigation, marks and removal previews.
- Protected paths - Protect shared system containers such as /Applications
  as a whole while eligible individual contents remain removable. Expand
  Unix system-state and Windows installation, boot and volume-metadata
  protections. Marking, Review and the worker share the same guards.
- Root aliases - Apply system, home, profile and mount protections through
  symlinked scan roots and macOS Data-volume aliases. One-filesystem scans
  respect nested mounts under those spellings. Removing a selected symlink
  still unlinks the entry without following its destination.
- Deep paths - Inspect and remeasure deeply nested Unix trees without
  failing solely because their full paths exceed the OS lookup limit.
  Relocated-cache detection, unreadable-directory refusals, mount boundaries
  and non-followed descendant symlinks retain their checks.
- Capacity checks - Share the scanner's accounting with the removal
  recheck, including hardlink de-duplication across the selection and the
  chosen size policy. Check reviewed and fresh totals separately against
  each disk's ceiling, avoiding double charges from changed walk order.
  APFS clone savings remain separate from the capacity budget.
- Preflight status - Show Checking selection before dispatching removal.
  A refused preflight leaves rows waiting and reports that no selected
  paths were removed. Label the signed measurement Free-space change;
  other volume activity can make it negative.
- Nested removals - Preserve all previously removed scanned bytes when a
  later removal absorbs an enclosing directory. Remaining marks, MCP
  listings and searches, and Worth a look totals now agree without double
  discounts; each run still reports only its own removal.
- Review inspection - Inspect marked trees for relocated supported caches
  in the background before enabling raw-removal confirmation. Show the
  refusal and route to Cache cleanup before the worker runs; unrelated
  marks can still be removed after the affected mark is cleared.
- Classification - Distinguish Temporary, Diagnostic logs and Trash from
  caches, including scans started inside those locations. Preserve Git,
  VM and protected-store ownership, and accept all core category filters
  in MCP find.
- Compatibility - The window link now uses protocol version 10. Update
  and restart both the PwrDisk window and MCP process together.

## v0.19.0

- Cleanup routing - Send paths to Cache cleanup only when PwrDisk
  positively recognizes a cache whose owner has a supported cleanup
  command. Cargo build output, virtual environments, generic cache names,
  bare cache tags and caches for pnpm, Homebrew or Puppeteer use ordinary
  Trash or Permanent removal, with the usual guards, limits and consent.
- Cache recognition - Keep supported owners' cache roots and contained
  caches in the cleanup flow, including relocated layouts and wrappers.
  Recognize pnpm's independent store and metadata locations, Playwright's
  npm environment aliases and Puppeteer's home cache for owner proposals,
  without probing unsupported owners or adding cleanup commands for them.
- Active-use checks - Check the running process's executable image as well
  as its arguments before cache cleanup, so a native owner cannot escape
  detection by changing its command-line name. Detection remains best
  effort; stop owning tools before approval.
- Development screenshots - Add opt-in offscreen macOS rendering of the
  real app views with contrived fixtures, scripted scenarios and native
  text. The renderer does not open an on-screen window or send OS input,
  and it is unavailable in release builds.

## v0.18.0

- Cache cleanup - Check owners in Review to prepare a proposal for each
  marked cache. Choose Skip (the default), the owner's exact command, or
  Delete files per path. Ordinary Trash and Permanent removal now refuse
  recognized caches, including caches inside a selected parent.
- Owner commands - On macOS and Linux, support verified versions of npm,
  npx, uv, pip/pip3 and Cypress. Preview the executable, version, arguments,
  configured root, effects and full scope, including unselected siblings.
  Recheck before running; a missing, unsupported, failed or cancelled
  command never falls back to deleting files.
- Playwright - Add owner cleanup for the installed JavaScript CLI versions
  1.64.0 and 1.64.1. The preview names the configured browser cache and
  explains that uninstalling all managed browser versions affects other
  projects sharing it. Owner commands for pnpm, Homebrew and Puppeteer
  remain unavailable; Windows supports cache inspection only.
- Direct cache deletion - Delete files acts only on the selected path and
  requires its own disclosure and consent, separate from owner approval.
  Choose Trash or permanent deletion for the run. This action is available
  on macOS and Linux; unknown caches are never labelled disposable.
- Cleanup safety - Charge each owner's whole scope against the capacity
  limit, reject conflicting overlapping choices, and refuse paths whose
  owners are in use, including versioned pip launchers and uvx. Active-use
  detection is best effort; stop owning tools before approval. Exports
  retain only marked paths and cannot become raw cache-deletion scripts.
- Cleanup results - Report each step's success, failure, cancellation or
  skipped state, keeping scope allocation separate from observed volume
  change. Stop cancels the active owner command; failed steps do not prevent
  later steps from running. Cache cleanup always rescans, including after
  partial failure, without treating a retained cache root as deleted.
- Agent cleanup - MCP previews return per-path proposals before consent.
  Decided previews bind every path's action and deletion mode to a
  single-use confirmation; approving owner commands cannot approve direct
  file deletion. Status reports retain per-path cleanup results.
- APFS scans - Reduce clone-accounting memory and repeated metadata work,
  share identical clone records, and avoid private-extent walks for freshly
  confirmed fully shared clones. Modified clones and incomplete metadata
  keep the conservative queries and estimates.
- Scan performance - Relayout indexed trees in parallel without a second
  node array, reduce shared-store locking, and discover artifact roots in
  parallel. Tune the default macOS worker count to reduce APFS contention
  while preserving explicit worker budgets.
- Hardlink estimates - Keep partial-estimate flags independent of directory
  merge order, including conflicting link counts, clone facts or allocation
  observations. Inconsistent observations remain marked partial.
- Legend and tooltips - Use the only pill, or `o` on a focused legend chip,
  to switch a kind or age filter to that chip alone. Normal clicks still
  combine or cycle chips. Delay tooltips and position them clear of the
  pointer so they do not cover nearby controls.
- Compatibility - The window link now uses protocol version 9. Update
  and restart both the PwrDisk window and MCP process together.

## v0.17.0

- Artifact assessments - Distinguish Detected artifacts from Suggested for
  review cleanup candidates in the inspector and MCP reports, with the
  identification evidence and guidance shown before marking. Recognize
  more developer dependency and build-cache layouts without treating whole
  tool homes or generic build folders as disposable.
- VM and Git data - Identify virtual disks, checkpoints, saved VM memory,
  Git metadata and worktrees without recommending their removal. VM and
  Git contents also prevent enclosing directories from being suggested as
  a whole. Review warns when marked VM data may hold unique work.
- Protected folders - Refuse protected paths before they can be marked.
  Protect the profiles folder and each home directory directly beneath it
  on macOS, Linux and Windows, including Linux's linked `/home` spelling.
  Contents inside a home remain eligible under the other removal guards.
- Removal limits - Require Mark anyway to mark beyond the configured
  capacity limit. Review defaults to a second, typed confirmation naming
  each affected disk and its share. Each disk is held to its own confirmed
  share; growth beyond it is refused. Settings > Safety offers a checkbox
  or no second step instead of typing, and an Off choice for the window.
- Agent limits - Agents cannot use the person's over-limit override. They
  retain the default 5% limit when the window's limit is Off, both at the
  preview and when the removal worker remeasures the selection.
- Removal outcomes - A worker that cannot start now finishes with every
  item marked failed. MCP reconciles a preview's exact native-UI removal
  outcome before issuing another request, including cancellation or errors
  in an outstanding confirmation form. Observed volume changes retain
  their sign rather than reporting a decrease in free space as zero.
- Agent planning - Add read-only `pwrdisk_rank` to compare up to 50 explicit
  candidates, `pwrdisk_retention` for a bounded macOS Time Machine snapshot
  presence check, and `pwrdisk_read_skill` for embedded guidance. Snapshot
  presence does not quantify retained bytes or guarantee recovered space.
- Agents settings - Put connection status, copyable MCP setup commands and
  the first prompt in Settings > Agents (MCP), also reachable from Help >
  Connect an AI Agent (MCP) and the connected-agent indicator.
- Notices - Show transient notices as timed toasts, keep errors until
  dismissed, and offer Undo after Unmark all. Copy buttons confirm in place.
  Toasts, floating menus and list controls no longer activate content
  underneath the pointer. Scan headings no longer clip letter descenders.
- Side panel - Pin section headers while their rows scroll. Short or
  zoomed windows fold secondary selection figures and empty sections,
  keeping assessments, marking actions and the disk meter accessible.
- Legend - Show each filter chip's size across the whole scan and keep the
  legend on one row. Empty, tiny or overflowing kinds move into a +N menu;
  active filters remain visible. Rename Reclaimable to Suggested to match
  the distinction between identification and cleanup assessment.
- Compatibility - The window link now uses protocol version 7. Update
  and restart both the PwrDisk window and MCP process together.

## v0.16.0

- Removal progress - Keep the complete approved list available from the
  start of removal through Done. Each row shows whether it is waiting,
  being removed, in trash, removed, failed or not reached after Stop, with
  failures explained beside the affected item.
- Following removal - Follow the item being processed automatically.
  Scroll it out of view to pause following; use Follow the removal or
  scroll back to resume. Large lists draw only the rows in view.
- Removal totals - Show running progress, listed bytes, private-byte
  estimates, observed free-space change and the disk meter beside the
  list. Keep the final results and the ledger of removals since the scan
  available on Done; moving to trash explains why space is not yet freed.
- Review rows - Show each item's category or reclaim reason and its parent
  folder consistently across Review, Removing and Done. Deep paths shorten
  in the middle to fit one line; tooltips and accessible names retain the
  complete path. Narrow windows move the progress sidebar below the list.
- Git status - Check selected checkouts promptly and hovered checkouts
  only after a pause, with at most two checks at once and a five-second
  limit. Keep cached answers while refreshing, cancel checks on a rescan
  or source change, and stop periodic refreshes while the window is in the
  background. Slow checkouts report too large to check.
- Git inspection - Avoid checkout-defined hooks, filters and network
  access when inspecting repositories. Checkouts with their own filters
  report changes unknown, and macOS does not prompt to install developer
  tools merely because a checkout was selected.
- Responsiveness - Stop rereading checkout metadata and saved Safety
  settings on every frame. Run Git checks on separate threads so they do
  not occupy the app's shared background workers.

## v0.15.0

- Large scans - Use less memory for file names and allocation records,
  classify large trees in parallel, and quit without waiting for every
  file record to be freed on the main thread.
- APFS sharing - Show private and shared bytes per entry in the inspector,
  hover cards, List view and MCP reports. Reclaim estimates account for
  the selected set together, including clones, and distinguish known
  savings from uncertain bounds without changing treemap weights.
- Agent estimates - Add read-only `pwrdisk_estimate` for up to 200 paths,
  with accepted, covered and refused paths, per-volume reclaim bounds,
  and bounded discovery of other clone holders. Estimates do not mark
  paths or authorize removal.
- List view - Switch between Treemap and List from the title strip or
  Cmd/Ctrl+1 and Cmd/Ctrl+2. Browse the complete file list with paging,
  keyboard navigation and accessible names. Less-used toolbar controls
  move into an overflow menu when space is tight.
- Legend filters - Cycle category, age and Reclaimable chips through only,
  hidden and shown. Filters apply to Treemap, List and search, with partial
  folder sizes and an explicit empty result. Marking a partly shown folder
  still marks the whole folder; the inspector explains this.
- Marked totals - Keep the combined marked size, reclaim estimate and
  Review button visible above the Disk meter, with marked paths first in
  the side panel.
- Treemap navigation - A click selects; clicking the selection opens one
  level toward it. Double-clicks no longer skip through multiple levels.
  Improve tile contrast, strike marked names and underline search matches
  without adding prefixes to file names.
- Removal - Keep the scan snapshot by default instead of starting a full
  rescan after every removal. Removed entries stay greyed out and cannot
  be marked again until a rescan. Choose Move & rescan or Delete & rescan
  when a fresh tree is wanted; MCP removal also accepts `rescan`.
- Freed space - Keep a ledger of removals since the scan, separating listed
  bytes, private-byte estimates and the measured free-space change. MCP
  status exposes the ledger, and listings and searches identify entries
  removed since the scan.
- Removal checks - Allow entries inside macOS `/tmp` and `/var/tmp` while
  keeping the temporary-directory roots and other system paths protected.
  Fix Windows volume-capacity checks for individual files. Confirmation,
  capacity limits and the other removal guards still apply.
- Keyboard and accessibility - Improve focus traversal, modal containment,
  source and scan status, Review summaries, document reading and Logs
  selection. Expose text and search values to native accessibility APIs.
  Help, Settings and readers reflow through 200% interface zoom, and
  directory navigation honors Reduce motion.
- Window placement - Remember the main window's position and size,
  keep restored windows on screen, and open macOS windows on the active
  display. Improve Settings organization, About build information and
  platform-specific dialog button placement.
- Updates - Show checking and download progress, keep a downloaded update
  ready across later checks, and offer Restart from a persistent title-strip
  chip. Restart runs once in the background and saves the window position.
- SSH and volumes - Preserve POSIX paths for Unix SSH hosts on Windows,
  omit Linux kernel pseudo-filesystems from source discovery, and handle
  escaped mount sources consistently. Searches retain matching nested
  empty directories.
- Compatibility - The window link now uses protocol version 6. Update
  and restart both the PwrDisk window and MCP process together.

## v0.14.0

- Removal safety - Settings > Safety adds a per-operation limit, defaulting
  to 5% of each volume's capacity, for both manual and MCP removal. The
  complete selection is re-measured before removal; oversized, unreadable
  or unknown-capacity selections are refused.
- AI-assisted removal - Preview the marked paths, size and trash/permanent
  mode before confirming removal through the MCP client's interactive
  form. Confirmation is expiring and single-use; changed marks, mode or
  safety settings require a new preview. Clients without form support
  use human Review instead. Removal requires a linked local window, and
  Trash never falls back to permanent deletion.
- Agent browsing - Listing never moves the window, including requests
  from older clients that pass `show`. Navigation uses an explicit
  `pwrdisk_show` request when the user asks to follow or open a path.
- Cleanup candidates - Recognize verified pnpm package stores without
  classifying the executable, global tools or the whole installation as
  disposable. Unknown aggregates retain their own category, and legacy
  Google Drive folders are recognized as synced data.
- Scan context - Linked MCP status reports allocated/apparent size,
  hidden-entry inclusion and unreadable counts. macOS reports explain
  APFS sharing and distinguish listed bytes from potential savings;
  cleanup searches explain removal eligibility without adding marks.
- Compatibility - The window link now uses protocol version 3. Update
  and restart both the PwrDisk window and MCP process together.

## v0.13.0

- Downloads - Add stable filenames for the universal macOS DMG and both
  Linux tarballs so the README's download buttons keep working across
  releases. Versioned downloads remain available.
- Documentation - Put downloads, installation and a quick tour first in
  the README, with detailed usage and build instructions in separate
  guides.
- Third-Party Notices - Include the socket2 attribution required by the
  Windows agent-window link and refresh notices for this release.

## v0.12.0

The first PwrDisk release, continuing from disktree 0.11.0 with local and
SSH exploration, AI-assisted review, and signed installers.

- PwrDisk - The app, binary and bundle are PwrDisk (`pwrdisk`,
  `com.pwrdrvr.pwrdisk`), a maintained fork of tobi/disktree. A Power
  Efficiency preset saved by disktree is still read until PwrDisk saves one.
- macOS - A drag-to-install DMG with a universal app (Apple silicon and
  Intel), signed with a Developer ID, notarized and stapled.
- Windows - `PwrDisk-win-Setup.exe`, a per-user MSI and a portable zip for
  x64, and the same for ARM64 (`PwrDisk-win-arm64-*`), signed through Azure
  Artifact Signing. Setup installs to your profile without administrator
  rights and adds a Start menu shortcut.
- Sources and SSH - Choose a local volume or an SSH host and root before
  scanning. Explore remote disks with live scan progress and treemap
  previews, then export a cleanup plan; remote scans are read-only.
- AI agents - `pwrdisk mcp` lets an agent inspect disk usage and mark an
  agreed cleanup list. Marks appear in the open window on macOS, Linux
  and Windows, and the agent can open Review for you. Only a person can
  confirm removal. Embedded skills explain each system's storage and
  cleanup cautions.
- Scan progress - Whole-drive scans show estimated byte coverage and
  average files per second, followed by an explicit finalizing state.
  Progress scrolls in short windows while the free-space meter stays
  visible. Folder and filtered scans keep counts when no compatible
  total is available.
- APFS space estimates - Show clone sharing separately from treemap
  sizes and estimate what the marked set can reclaim together. Retained
  clones and hardlinks reduce projected savings; the volume meter uses
  the conservative estimate and reports measured free space after removal.
- macOS service stores - Show measured Spotlight-index and FSEvents
  usage when readable, label partial results, and keep those stores
  protected from raw removal. Unreadable service stores do not produce
  repeated Full Disk Access hints.
- Explore and Review - Custom window chrome and faster marking, path
  lookup and review keep large cleanup queues responsive.
- Updates - Copies installed from the DMG, Setup.exe or the MSI check
  GitHub Releases at startup and every six hours, and download a newer
  release in the background. Shift-U (or PwrDisk › Check for Updates… on
  macOS) opens Settings › Updates; Restart to update installs a download
  at once, and otherwise it installs when PwrDisk quits. Pick Stable or
  Beta, Latest or Prerelease there; a copy follows the channel its version
  implies until you do.
- Settings and Help - Shared menus, separate Updates, Diagnostics and
  About panes, and offline Changelog, License and Third-Party Notices
  windows make support information easier to find. Copy Diagnostics Info
  omits scanned paths, SSH hosts and volume names.
- Logs - A live viewer adds wrapped lines, search with Filter and
  Highlight modes, context rows, line selection, and marks for copying
  just one action's output. Following pauses while you read, and display
  preferences persist. Logs record counts and durations, never scanned
  paths.
- Linux - The tarball and `install.sh` are unchanged. Linux copies do not
  update themselves; download the next release.
