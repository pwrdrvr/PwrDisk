# PwrDisk agent guide

PwrDisk is a maintained fork of disktree and a GPUI treemap explorer for disk
usage. See [README.md](README.md) for the product. Keep the upstream crate,
module, type and SSH helper names (`disktree-scan`). The user-facing names are
PwrDisk, the `pwrdisk` binary and the `com.pwrdrvr.pwrdisk` bundle ID.

## Testing on a desktop

Never launch PwrDisk, manipulate windows, or send synthetic input on the
operator's desktop. Leave any running PwrDisk window alone, including its MCP
link. Use headless window-harness tests or a stand-in link. Before capturing
screenshots, running native accessibility checks or testing with an on-screen
window, read [the desktop test procedure](docs/agents/verification.md#desktop-checks).
Keep lab details out of this public repo and get operator approval before
attaching screenshots that show real scan data.

## Removal

Never delete anything outside a marked path or bypass the project's removal
guards. Marking and opening Review do not authorize deletion. Agents must
never approve removal through computer use. For an MCP removal, show the
exact paths and mode from an expiring, single-use preview, explain
recoverability and get explicit user confirmation for that preview. A
changed or expired preview requires new confirmation. No headless or SSH
removal. Trash never falls back to permanent deletion. A failed owner cleanup
never falls back to deleting files.
Read [the removal rules](docs/agents/invariant-details.md#12-removal-and-confirmation)
before changing marks, review, removal or cache cleanup.

## Commands

Run `cargo xtask lint` before calling changes done. It is a check and must
leave files untouched. Run matching-platform headless tests with
`cargo xtask test`. See [building and testing](docs/agents/verification.md)
for platform commands, test harnesses and desktop procedures. Sandboxer is
dormant. Do not run its provider operations.

## Working in the repo

- Before adding code or when unsure where a behavior lives, read
  [where changes belong](docs/agents/where-changes-belong.md).
- For a bug fix, add a property, model or oracle test for the broken rule
  and confirm it fails on the pre-fix code. Use the same approach for new
  guard, limit, accounting and path logic, and state that changes over time.
  See [the testing guide](docs/property-model-oracle-testing.md).
- Use stable rustfmt with 80 columns and four-space indents. Record deliberate
  Clippy exceptions with a reason in the lint configuration or on the item.
  Comments explain non-obvious numbers, ordering and boundaries.
- Put scratch files in the gitignored `.local/` directory.
- Leave `CHANGELOG.md` for the release commit. Before preparing a PR, read
  [the PR guide](docs/agents/pull-requests.md) for titles, screenshots,
  upstream ports, push targets and PwrAgent attachment.
- Track deferred work on a [PwrAgent to-do card](docs/agents/pull-requests.md#pwragent-threads).
- Before changing installation, the desktop entry, app bundle or icon,
  release packaging or the release process, read
  [the packaging guide](docs/agents/packaging.md).

## Invariants

Read the sections that cover the behavior you are changing.

- [Measurement, totals and scan completion](docs/agents/invariant-details.md#1-how-each-platform-measures)
- [Protected paths and marks](docs/agents/invariant-details.md#4-protected-paths)
- [Treemap painting, crumbs and zoom](docs/agents/invariant-details.md#6-treemap-painting)
- [Volume projections, discovery, SSH and live scans](docs/agents/invariant-details.md#9-volume-attribution)
- [Removal, confirmation and cache cleanup](docs/agents/invariant-details.md#12-removal-and-confirmation)
- [Local link, MCP and agent skills](docs/agents/invariant-details.md#13-local-link-transport)
- [Tree storage and parallel passes](docs/agents/invariant-details.md#15-a-trees-buffers)

## Agent instruction files

Keep startup guidance in the smallest applicable `AGENTS.md`, with long
procedures under `docs/` and links that say when to read them. Each
`AGENTS.md` has a sibling `CLAUDE.md` symlink. CI checks this file against a
24 KiB ceiling. Keep it well below that by loading task-specific guidance
only when needed.
