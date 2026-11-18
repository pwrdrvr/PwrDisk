# Where changes belong

<!-- Moved verbatim from AGENTS.md, which keeps a summary and says when
     to read this. -->

Read this before adding code, or when unsure which file owns a behavior.

| change | where |
| --- | --- |
| bounded Unix SSH traversal | `crates/disktree-core/src/remote_walk.rs` |
| SSH protocol, discovery and transport | `crates/disktree-core/src/remote.rs` |
| joining, splitting or comparing a scanned path | `crates/disktree-core/src/path_style.rs` |
| measurement, filtering, parallelism | `crates/disktree-core/src/scan.rs` |
| what a node is, or a derived total | `crates/disktree-core/src/tree.rs` |
| how a tree is held in memory, its buffers and parallel passes | `crates/disktree-core/src/index.rs` |
| tile geometry, nesting, the merged tail | `crates/disktree-core/src/treemap.rs` |
| anything that deletes, or refuses to | `crates/disktree-core/src/removal.rs` |
| free space and projections | `crates/disktree-core/src/space.rs` |
| the folder-level usage report and comparing two | `crates/disktree-core/src/usage_report.rs`, `usage_report/diff.rs`; the CLI in `crates/disktree-app/src/report_cli.rs`; the window's saved report (the chooser's box, File › Save Usage Report, and when a scan may leave one) in `crates/disktree-app/src/scan_report.rs`; the Usage History window in `crates/disktree-app/src/usage_history.rs`; format in `docs/usage-reports.md` |
| what macOS lists and measures in bulk | `crates/disktree-core/src/macos.rs` — `unsafe` for `getattrlistbulk` |
| what Windows lists, measures and compares differently | `crates/disktree-core/src/windows.rs` — `unsafe` for Win32 |
| reading a whole NTFS drive from its file table | `crates/disktree-core/src/mft.rs` |
| APFS clone hints and private-size system calls | `crates/disktree-core/src/macos.rs` — isolated `unsafe` |
| clone sharing totals and conservative savings | `crates/disktree-core/src/sharing.rs` |
| what git is asked about a checkout, when, and how many at once | `crates/disktree-app/src/git.rs` (the probe and its time limit), `crates/disktree-app/src/git_store.rs` (the queue, the bucket, the cache) |
| a key, a screen transition, a mark | `crates/disktree-app/src/state.rs` |
| what may be marked, Mark anyway, Review's over-limit confirmation | `crates/disktree-app/src/mark_guard.rs`, the settings in `safety.rs` |
| cache cleanup: owners, proposals, decisions, per-path results | `crates/disktree-core/src/cleanup.rs` (model, allowlist), `crates/disktree-app/src/cleanup.rs` (window state), `cleanup_view.rs` (Review's proposals) |
| Removing and Done: the run's list, its rows, following and running figures; the row anatomy Review shares | `crates/disktree-app/src/run_view.rs`, the model in `run_list.rs`, Review's `mark_row` in `views.rs` |
| what the explore bar folds into ⋮ at each width, and ⋮'s keys | `crates/disktree-app/src/toolbar.rs` |
| the legend as a filter: its chips and what they keep | `crates/disktree-app/src/legend.rs`, the rule in `crates/disktree-core/src/filter.rs` |
| spacing, type and size | `crates/disktree-app/src/ui.rs` — tokens only, no `px` in layout |
| the mosaic's painting or labels | `crates/disktree-app/src/treemap_view.rs` |
| layout of a screen | `crates/disktree-app/src/views.rs` |
| colours derived from the theme | `crates/disktree-app/src/palette.rs` |
| the title strip, window controls, the client-side frame | `crates/disktree-app/src/chrome.rs` |
| where the main window opens, and remembering it | `crates/disktree-app/src/window_frame.rs` |
| which display a new window opens on | `crates/disktree-app/src/placement.rs` |
| the menu bar, its shortcuts, the strip's menu off macOS | `crates/disktree-app/src/app_menu.rs`, the dropdown in `views.rs` |
| Help's rows, Copy Diagnostics Info | `crates/disktree-app/src/help.rs`, the report in `diagnostics.rs` |
| Settings (General, Updates, Safety, Agents (MCP), Diagnostics, About) | `crates/disktree-app/src/settings.rs` |
| Changelog, License, Third-Party Notices, Logs windows | `doc_window.rs`, `logs_window.rs`, framed by `aux_window.rs` |
| what goes in `main.log`, and where it lives | `crates/disktree-app/src/diag_log.rs` — never a scanned path |
| `THIRD_PARTY_LICENSES` | `cargo xtask licenses` (cargo-about), then commit it; non-crate parts in `licenses/bundled.txt` |
| the macOS icon, in any of its looks | `packaging/macos/` — the SVGs and `AppIcon.icon`, then `make dock-icons` |
| checking, downloading and applying updates | `crates/disktree-app/src/updater.rs` |
| what the window says about updates, and its notices: the toasts, the strip's ready chip | `crates/disktree-app/src/toasts.rs`, the chip in `views.rs` |
| the MCP connect commands and the connected agent's status line | `crates/disktree-app/src/agent.rs` |
| which release a release channel picks | `crates/disktree-app/src/update_channel.rs` |
| release packaging, the DMG, `vpk` | `scripts/release/`, `xtask/src/bundle.rs`, `.github/workflows/release.yml` |
| what an agent can ask of the window, and the answer | `crates/disktree-link/src/lib.rs` (messages), `crates/disktree-app/src/agent.rs` (handling) |
| a tool, its parameters or its wording | `crates/disktree-mcp/src/server.rs` (the MCP surface), `crates/disktree-mcp/src/session.rs` (what it does) |
| desktop MCP hosting and the opaque stdio relay | `crates/disktree-mcp/src/desktop.rs`; `crates/disktree-link/src/mcp_stream.rs` and `windows.rs` own the protected stream |
| how a listing or a plan reads to an agent | `crates/disktree-mcp/src/report.rs` |
| what an agent is told to do, and to avoid | `skills/` (each system's cautions in `skills/pwrdisk-free-space/references/`), listed in `crates/disktree-mcp/src/skills.rs` |
