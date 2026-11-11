//! Usage History: the saved usage reports for this computer, two of them
//! picked as Before and After, and what changed between them.
//!
//! It reads saved reports only, never the tree on screen, so exploring and
//! reviewing are untouched. The figures are [`disktree_core::usage_report`]'s:
//! each folder's own change, which adds up to the total without counting a
//! byte under every ancestor, beside what free space did. A folder only one
//! report kept is said to be under that report's threshold or absent, never
//! new or deleted.

use std::path::{Path, PathBuf};

use disktree_core::size::{human_bytes, human_count};
use disktree_core::usage_report::diff::{self, Bound, Changed, Diff, OneSided};
use disktree_core::usage_report::library::{self, Saved, Unreadable};
use disktree_core::usage_report::{Header, SizeMode};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, Context, Div, ElementId, FocusHandle, FontWeight, Hsla,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, Role,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window,
    WindowHandle, div, px, relative,
};
use gpui_omarchy::{ActiveTheme as _, ButtonVariant, Theme, button};

use crate::aux_window::{self, Spec};
use crate::chrome::Drags;
use crate::ui::{size, space, text};
use crate::widgets;

pub use history_actions::*;

#[allow(
    clippy::derive_partial_eq_without_eq,
    reason = "GPUI's actions! macro writes the derives, not us"
)]
mod history_actions {
    use gpui_kit::actions;

    actions!(
        disktree,
        [
            /// The saved usage reports in their own window.
            OpenUsageHistory,
        ]
    );
}

/// The changes a comparison lists before saying how many more there are.
/// A whole disk has a few thousand folders; past this they are small.
pub const CHANGES_SHOWN: usize = 200;
/// Folders kept by one report only, listed per side.
pub const ONE_SIDED_SHOWN: usize = 50;

pub const COMPARE_ID: &str = "history-compare";
pub const BACK_ID: &str = "history-back";
pub const OPEN_ID: &str = "history-open";

const SPEC: Spec = Spec {
    title: "PwrDisk Usage History",
    size: (960.0, 760.0),
    min: (640.0, 480.0),
};

/// Where `pwrdisk report --save` writes and this window reads: beside
/// PwrDisk's settings, which is where a person's own PwrDisk data lives.
pub fn reports_dir() -> Option<PathBuf> {
    crate::power::config_base().map(|base| base.join("pwrdisk/usage-reports"))
}

pub fn install(cx: &mut App) {
    // Deferred, as Help's windows are: from the window it would bring
    // forward, GPUI still holds it.
    cx.on_action(|_: &OpenUsageHistory, cx| cx.defer(open));
}

/// Opens the window, or brings it forward.
pub fn open(cx: &mut App) {
    let existing = aux_window::find::<UsageHistory>(cx, |_| true);
    if aux_window::activate(existing, cx) {
        return;
    }
    open_window(reports_dir(), cx);
}

/// A new window over the reports in `dir`.
pub fn open_window(
    dir: Option<PathBuf>,
    cx: &mut App,
) -> Option<WindowHandle<UsageHistory>> {
    aux_window::open(&SPEC, cx, |window, cx| {
        let view = UsageHistory::new(dir, cx);
        window.focus(&view.focus, cx);
        view
    })
}

/// The window's view.
pub struct UsageHistory {
    dir: Option<PathBuf>,
    /// Newest first: the folder's, and any opened from elsewhere.
    reports: Vec<Saved>,
    opened: Vec<Saved>,
    unreadable: Vec<Unreadable>,
    before: Option<PathBuf>,
    after: Option<PathBuf>,
    comparison: Option<Comparison>,
    /// What went wrong last: a report that would not read, two that do
    /// not overlap.
    problem: Option<String>,
    strip_drag: bool,
    pub focus: FocusHandle,
}

/// Two reports compared.
pub struct Comparison {
    pub before: Header,
    pub after: Header,
    pub diff: Diff,
}

impl Drags for UsageHistory {
    fn strip_drag(&mut self) -> &mut bool {
        &mut self.strip_drag
    }
}

impl UsageHistory {
    pub fn new(dir: Option<PathBuf>, cx: &Context<'_, Self>) -> Self {
        let mut view = Self {
            dir,
            reports: Vec::new(),
            opened: Vec::new(),
            unreadable: Vec::new(),
            before: None,
            after: None,
            comparison: None,
            problem: None,
            strip_drag: false,
            focus: cx.focus_handle().tab_stop(true),
        };
        view.reload();
        view
    }

    /// Reads the folder again; picks stay where their files still are.
    pub fn reload(&mut self) {
        let (mut reports, unreadable) =
            self.dir.as_deref().map(library::list).unwrap_or_default();
        for opened in &self.opened {
            if !reports.iter().any(|report| report.path == opened.path) {
                reports.push(opened.clone());
            }
        }
        reports.sort_by(|left, right| {
            right.header.captured_at.cmp(&left.header.captured_at)
        });
        let known = |pick: Option<PathBuf>| {
            pick.filter(|path| reports.iter().any(|it| &it.path == path))
        };
        self.before = known(self.before.take());
        self.after = known(self.after.take());
        self.reports = reports;
        self.unreadable = unreadable;
    }

    #[cfg(test)]
    pub fn reports(&self) -> &[Saved] {
        &self.reports
    }

    #[cfg(test)]
    pub const fn comparison(&self) -> Option<&Comparison> {
        self.comparison.as_ref()
    }

    fn report(&self, path: Option<&PathBuf>) -> Option<&Saved> {
        let path = path?;
        self.reports.iter().find(|report| &report.path == path)
    }

    /// Why `report` cannot be compared with the Before pick, if it cannot.
    fn refusal(&self, report: &Saved) -> Option<String> {
        let before = self.report(self.before.as_ref())?;
        if before.path == report.path {
            return None;
        }
        if before.header.root != report.header.root {
            return Some("another root".to_owned());
        }
        diff::incompatible(&before.header, &report.header)
            .into_iter()
            .next()
    }

    pub fn pick_before(&mut self, index: usize, cx: &mut Context<'_, Self>) {
        let Some(path) = self.reports.get(index).map(|it| it.path.clone())
        else {
            return;
        };
        if self.after.as_ref() == Some(&path) {
            self.after = None;
        }
        self.before = Some(path);
        // An After that no longer compares is let go rather than kept
        // dimmed under the button that would use it.
        if let Some(after) = self.report(self.after.as_ref())
            && self.refusal(after).is_some()
        {
            self.after = None;
        }
        self.problem = None;
        cx.notify();
    }

    pub fn pick_after(&mut self, index: usize, cx: &mut Context<'_, Self>) {
        let Some(report) = self.reports.get(index) else {
            return;
        };
        if self.refusal(report).is_some() {
            return;
        }
        let path = report.path.clone();
        if self.before.as_ref() == Some(&path) {
            self.before = None;
        }
        self.after = Some(path);
        self.problem = None;
        cx.notify();
    }

    /// Reads both picks whole and compares them, older first whichever way
    /// round they were picked: Before is the earlier capture.
    pub fn compare(&mut self, cx: &mut Context<'_, Self>) {
        let (Some(before), Some(after)) = (
            self.report(self.before.as_ref()),
            self.report(self.after.as_ref()),
        ) else {
            return;
        };
        let (first, second) =
            if after.header.captured_at < before.header.captured_at {
                (after.path.clone(), before.path.clone())
            } else {
                (before.path.clone(), after.path.clone())
            };
        let outcome = library::read(&first)
            .and_then(|a| library::read(&second).map(|b| (a, b)))
            .map_err(|error| error.to_string())
            .and_then(|(a, b)| {
                diff::compare(&a, &b).map(|diff| Comparison {
                    before: a.header,
                    after: b.header,
                    diff,
                })
            });
        match outcome {
            Ok(comparison) => {
                self.comparison = Some(comparison);
                self.problem = None;
            }
            Err(problem) => self.problem = Some(problem),
        }
        cx.notify();
    }

    pub fn back(&mut self, cx: &mut Context<'_, Self>) {
        self.comparison = None;
        self.reload();
        cx.notify();
    }

    /// Adds `path` to the list, as if it were in the folder.
    pub fn add_report(&mut self, path: &Path, cx: &mut Context<'_, Self>) {
        match library::read_header(path) {
            Ok(header) => {
                self.opened.push(Saved {
                    path: path.to_path_buf(),
                    header,
                });
                self.problem = None;
                self.reload();
            }
            Err(error) => {
                self.problem = Some(format!(
                    "{} is not a report: {error}",
                    path.display()
                ));
            }
        }
        cx.notify();
    }

    fn browse(cx: &Context<'_, Self>) {
        let chosen = cx.prompt_for_paths(gpui_kit::PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Open".into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = chosen.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let _ = this.update(cx, |this, cx| this.add_report(&path, cx));
        })
        .detach();
    }
}

impl Render for UsageHistory {
    fn render(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> impl IntoElement {
        crate::ui::sync_scale(window, cx);
        let theme = cx.omarchy().clone();
        if let Some(comparison) = &self.comparison {
            let back = button(BACK_ID, "Back", ButtonVariant::Secondary, cx)
                .accessibility_label("Back to the list of reports")
                .debug_selector(|| BACK_ID.into())
                .on_click(cx.listener(|this, _, _, cx| this.back(cx)));
            let body = compare_body(comparison, &self.focus, &theme, cx);
            return aux_window::frame(
                &["Usage History", "Compare"],
                back,
                body,
                window,
                cx,
            );
        }
        let open = button(
            OPEN_ID,
            "Open a Report\u{2026}",
            ButtonVariant::Secondary,
            cx,
        )
        .debug_selector(|| OPEN_ID.into())
        .on_click(cx.listener(|_, _, _, cx| Self::browse(cx)));
        let body = self.pick_body(&theme, cx);
        aux_window::frame(&["Usage History"], open, body, window, cx)
    }
}

impl UsageHistory {
    fn pick_body(&self, theme: &Theme, cx: &Context<'_, Self>) -> AnyElement {
        let mut list = div()
            .id("history-list")
            .role(Role::List)
            .aria_label("Saved usage reports, newest first")
            .flex()
            .flex_col()
            .border_1()
            .border_color(theme.border)
            .bg(theme.inset);
        // Grouped by root, in the order the newest of each was captured.
        let mut roots: Vec<&str> = Vec::new();
        for report in &self.reports {
            if !roots.contains(&report.header.root.as_str()) {
                roots.push(&report.header.root);
            }
        }
        for root in roots {
            let count = self
                .reports
                .iter()
                .filter(|report| report.header.root == root)
                .count();
            list = list.child(
                div().px(space::MD).pt(space::SM).pb(space::XS).child(
                    widgets::eyebrow(
                        format!(
                            "{root} \u{00b7} {count} report{}",
                            if count == 1 { "" } else { "s" }
                        ),
                        cx,
                    ),
                ),
            );
            for (index, report) in self.reports.iter().enumerate() {
                if report.header.root == root {
                    list = list.child(self.pick_row(index, report, theme, cx));
                }
            }
        }
        if self.reports.is_empty() {
            list = list.child(
                div()
                    .p(space::LG)
                    .text_color(theme.secondary)
                    .child(empty_note(self.dir.as_deref())),
            );
        }
        let ready = self.report(self.before.as_ref()).is_some()
            && self.report(self.after.as_ref()).is_some();
        let compare = button(COMPARE_ID, "Compare", ButtonVariant::Primary, cx)
            .debug_selector(|| COMPARE_ID.into())
            .disabled(!ready)
            .on_click(cx.listener(|this, _, _, cx| this.compare(cx)));
        let footer = format!(
            "{} report{} \u{00b7} {} on disk{}",
            self.reports.len(),
            if self.reports.len() == 1 { "" } else { "s" },
            human_bytes(library::bytes(&self.reports)),
            self.dir.as_deref().map_or_else(String::new, |dir| format!(
                " \u{00b7} {}",
                home_relative(dir)
            )),
        );
        div()
            .id("history-pick")
            .track_focus(&self.focus)
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap(space::MD)
            .p(space::LG)
            .child(div().text_color(theme.secondary).child(
                "Pick B on the earlier report and A on the later one. \
                         A report measured differently from Before cannot be \
                         compared with it.",
            ))
            .children(self.problem.clone().map(|problem| {
                div()
                    .debug_selector(|| "history-problem".into())
                    .text_color(theme.warning)
                    .child(problem)
            }))
            .child(
                div()
                    .id("history-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(list)
                    .children(self.unreadable.iter().map(|file| {
                        div()
                            .pt(space::SM)
                            .text_size(text::CAPTION)
                            .text_color(theme.secondary)
                            .child(format!(
                                "Not read: {} ({})",
                                file.path.display(),
                                file.reason
                            ))
                    })),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(space::MD)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(text::CAPTION)
                            .text_color(theme.secondary)
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(footer),
                    )
                    .child(compare),
            )
            .into_any_element()
    }

    fn pick_row(
        &self,
        index: usize,
        report: &Saved,
        theme: &Theme,
        cx: &Context<'_, Self>,
    ) -> gpui_kit::Stateful<Div> {
        let header = &report.header;
        let is_before = self.before.as_ref() == Some(&report.path);
        let is_after = self.after.as_ref() == Some(&report.path);
        let refusal = self.refusal(report);
        let free = header.volume.space_after.map(|space| space.available);
        let detail = format!(
            "{} folders from {} \u{00b7} {} sizes{}",
            human_count(header.totals.records),
            human_bytes(header.threshold_bytes),
            match header.size_mode {
                SizeMode::Allocated => "allocated",
                SizeMode::Apparent => "apparent",
            },
            if header.completeness.unreadable > 0 {
                format!(
                    " \u{00b7} {} unreadable",
                    human_count(header.completeness.unreadable)
                )
            } else {
                String::new()
            }
        );
        let when = local_time(header.captured_at);
        let slot = |id: String, label: &'static str, on: bool, color: Hsla| {
            div()
                .id(ElementId::Name(id.clone().into()))
                .debug_selector(move || id)
                .role(Role::Button)
                .flex_shrink_0()
                .size(size::POINTER_TARGET)
                .flex()
                .items_center()
                .justify_center()
                .border_1()
                .border_color(if on { color } else { theme.control_border() })
                .when(on, |slot| slot.bg(color).text_color(theme.background))
                .when(!on, |slot| {
                    slot.text_color(theme.secondary)
                        .hover(|style| style.bg(theme.hover_fill()))
                })
                .text_size(text::CAPTION)
                .font_weight(FontWeight::BOLD)
                .child(label)
        };
        div()
            .id(("history-row", index))
            .debug_selector(move || format!("history-row-{index}"))
            .role(Role::ListItem)
            .aria_label(format!(
                "Report captured {when}; {detail}; {} listed{}{}{}",
                human_bytes(header.totals.bytes),
                free.map_or_else(String::new, |free| format!(
                    "; {} free",
                    human_bytes(free)
                )),
                if is_before { "; Before" } else { "" },
                if is_after { "; After" } else { "" },
            ))
            .flex()
            .items_center()
            .gap(space::MD)
            .px(space::MD)
            .py(space::SM)
            .border_t_1()
            .border_color(theme.divider())
            .when(is_before || is_after, |row| {
                row.bg(theme.selection.opacity(0.35))
            })
            .when(refusal.is_some(), |row| row.opacity(0.55))
            .child(
                slot(
                    format!("history-before-{index}"),
                    "B",
                    is_before,
                    theme.accent,
                )
                .aria_label(format!("Use the report of {when} as Before"))
                .on_click(cx.listener(
                    move |this, _, _, cx| {
                        this.pick_before(index, cx);
                    },
                )),
            )
            .child(
                slot(
                    format!("history-after-{index}"),
                    "A",
                    is_after,
                    growth(theme),
                )
                .aria_label(format!("Use the report of {when} as After"))
                .on_click(cx.listener(
                    move |this, _, _, cx| {
                        this.pick_after(index, cx);
                    },
                )),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(div().text_color(theme.bright).child(when))
                    .child(
                        div()
                            .text_size(text::CAPTION)
                            .text_color(theme.secondary)
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(detail),
                    ),
            )
            .children(refusal.map(|why| {
                widgets::chip(
                    format!("measured differently: {why}"),
                    theme.warning,
                    cx,
                )
                .flex_shrink_0()
            }))
            .child(lane(
                format!("{} listed", human_bytes(header.totals.bytes)),
                theme.bright,
            ))
            .child(lane(
                free.map_or_else(
                    || "free unknown".to_owned(),
                    |free| format!("{} free", human_bytes(free)),
                ),
                theme.secondary,
            ))
    }
}

/// What an empty list says, and how to fill it.
fn empty_note(dir: Option<&Path>) -> String {
    let save = "pwrdisk report -D --save";
    dir.map_or_else(
        || {
            format!(
                "No reports folder on this system. Open a report, or write \
                 one with {save}."
            )
        },
        |dir| {
            format!(
                "No reports yet in {}. Write one with {save}, then come \
                 back; a comparison needs two.",
                dir.display()
            )
        },
    )
}

/// `dir` with the home folder written `~`, as the window's paths are.
fn home_relative(dir: &Path) -> String {
    std::env::home_dir()
        .and_then(|home| dir.strip_prefix(home).ok().map(Path::to_path_buf))
        .map_or_else(
            || dir.display().to_string(),
            |below| {
                format!("~{}{}", std::path::MAIN_SEPARATOR, below.display())
            },
        )
}

/// A right-aligned figure in a fixed lane, so the rows line up.
fn lane(value: String, color: Hsla) -> Div {
    div()
        .w(size::SHARE_LANE)
        .flex_shrink_0()
        .flex()
        .justify_end()
        .whitespace_nowrap()
        .text_color(color)
        .child(value)
}

fn local_time(seconds: i64) -> String {
    chrono::DateTime::from_timestamp(seconds, 0).map_or_else(
        || "an unknown time".to_owned(),
        |time| {
            time.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        },
    )
}

/// Growth, in orange: lighter than the accent's blue in a dark theme and
/// darker in a light one, so the two are told apart by lightness too.
pub const fn growth(theme: &Theme) -> Hsla {
    let dark =
        matches!(theme.appearance, gpui_kit::base::ThemeAppearance::Dark);
    Hsla {
        h: 0.065,
        s: 0.9,
        l: if dark { 0.68 } else { 0.42 },
        a: 1.0,
    }
}

/// `+1.2 GB`, `-300 MB` or `0 B`.
pub fn signed(delta: i128) -> String {
    let size =
        human_bytes(u64::try_from(delta.unsigned_abs()).unwrap_or(u64::MAX));
    match delta.signum() {
        1 => format!("+{size}"),
        -1 => format!("\u{2212}{size}"),
        _ => size,
    }
}

/// What free space did beside what the listing did. `unlisted` is space
/// taken that no listed folder accounts for: snapshots, swap, other
/// volumes, unreadable folders.
pub struct Summary {
    pub free_before: Option<u64>,
    pub free_after: Option<u64>,
    pub listed: i128,
    pub unlisted: Option<u64>,
}

impl Summary {
    pub fn of(diff: &Diff) -> Self {
        let listed =
            i128::from(diff.after.bytes) - i128::from(diff.before.bytes);
        let taken = diff
            .before
            .available
            .zip(diff.after.available)
            .map(|(before, after)| i128::from(before) - i128::from(after));
        Self {
            free_before: diff.before.available,
            free_after: diff.after.available,
            listed,
            unlisted: taken
                .map(|taken| taken - listed)
                .filter(|&rest| rest > 0)
                .and_then(|rest| u64::try_from(rest).ok()),
        }
    }
}

fn compare_body(
    comparison: &Comparison,
    focus: &FocusHandle,
    theme: &Theme,
    cx: &App,
) -> AnyElement {
    let diff = &comparison.diff;
    let summary = Summary::of(diff);
    let scale = diff
        .changed
        .iter()
        .map(|entry| entry.own_delta().unsigned_abs())
        .max()
        .unwrap_or(0)
        .max(1);
    let shown: Vec<&Changed> = diff
        .changed
        .iter()
        .filter(|entry| entry.own_delta() != 0)
        .take(CHANGES_SHOWN)
        .collect();
    let rest: Vec<&Changed> = diff
        .changed
        .iter()
        .filter(|entry| entry.own_delta() != 0)
        .skip(CHANGES_SHOWN)
        .collect();
    let mut body = div()
        .id("history-compare-scroll")
        .debug_selector(|| "history-compare-scroll".into())
        .track_focus(focus)
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .gap(space::LG)
        .p(space::LG)
        .child(
            div()
                .flex()
                .flex_col()
                .gap(space::XXS)
                .child(
                    div()
                        .text_size(text::HEADING)
                        .text_color(theme.bright)
                        .child(format!(
                            "{} \u{2192} {}",
                            local_time(comparison.before.captured_at),
                            local_time(comparison.after.captured_at)
                        )),
                )
                .child(
                    div()
                        .text_size(text::CAPTION)
                        .text_color(theme.secondary)
                        .child(format!(
                            "{} \u{00b7} folders from {}",
                            diff.scope,
                            human_bytes(diff.threshold_bytes)
                        )),
                ),
        );
    if !diff.incompatible.is_empty() {
        body = body.child(
            div()
                .debug_selector(|| "history-incompatible".into())
                .text_color(theme.warning)
                .child(format!(
                    "These reports measure different things, so their \
                     figures do not compare: {}.",
                    diff.incompatible.join("; ")
                )),
        );
    }
    body = body.child(summary_figures(&summary, theme, cx));
    if let Some(bar) = gap_bar(&summary, theme) {
        body = body.child(bar);
    }
    body = body.child(
        div()
            .text_size(text::CAPTION)
            .text_color(theme.secondary)
            .child(
                "Listed allocation is not what the volume holds: clones \
                 share blocks, snapshots keep deleted ones, and other \
                 volumes can share its free space. Each change is the net \
                 of what was written and removed.",
            ),
    );
    if !diff.warnings.is_empty() {
        body = body.child(div().flex().flex_col().gap(space::XXS).children(
            diff.warnings.iter().map(|warning| {
                div()
                    .text_size(text::CAPTION)
                    .text_color(theme.warning)
                    .child(warning.clone())
            }),
        ));
    }
    let mut list = div()
        .id("history-changes")
        .role(Role::List)
        .aria_label("Where it changed, largest first")
        .flex()
        .flex_col()
        .border_1()
        .border_color(theme.border)
        .bg(theme.inset);
    for (index, entry) in shown.iter().enumerate() {
        list =
            list.child(change_row(index, entry, scale, &diff.scope, theme, cx));
    }
    if shown.is_empty() {
        list = list.child(
            div()
                .p(space::MD)
                .text_color(theme.secondary)
                .child("No folder kept in both reports changed."),
        );
    }
    if !rest.is_empty() {
        let total: i128 = rest.iter().map(|entry| entry.own_delta()).sum();
        list = list.child(
            div()
                .px(space::MD)
                .py(space::SM)
                .text_size(text::CAPTION)
                .text_color(theme.secondary)
                .child(format!(
                    "and {} folders with smaller changes, {} in all",
                    human_count(rest.len() as u64),
                    signed(total)
                )),
        );
    }
    body = body
        .child(widgets::section(
            "Where it changed: each folder's own part, without subfolders \
             both reports kept. These add up to the listed change.",
            cx,
        ))
        .child(list);
    for (title, entries, side) in [
        (
            "Kept only after: new, or grown past the threshold",
            &diff.appeared,
            "before",
        ),
        (
            "Kept only before: shrunk below the threshold or gone; not \
             known to be deleted",
            &diff.disappeared,
            "after",
        ),
    ] {
        let tops: Vec<&OneSided> =
            entries.iter().filter(|entry| entry.topmost).collect();
        if tops.is_empty() {
            continue;
        }
        let mut rows = div()
            .id(ElementId::Name(format!("history-{side}-only").into()))
            .role(Role::List)
            .aria_label(title)
            .flex()
            .flex_col()
            .border_1()
            .border_color(theme.border)
            .bg(theme.inset);
        for (index, entry) in tops.iter().take(ONE_SIDED_SHOWN).enumerate() {
            rows = rows.child(one_sided_row(
                index,
                entry,
                side,
                &diff.scope,
                theme,
                cx,
            ));
        }
        body = body.child(widgets::section(title, cx)).child(rows);
    }
    body.into_any_element()
}

fn summary_figures(summary: &Summary, theme: &Theme, cx: &App) -> Div {
    let free = match (summary.free_before, summary.free_after) {
        (Some(before), Some(after)) => Some((before, after)),
        _ => None,
    };
    div()
        .flex()
        .flex_wrap()
        .gap(space::XL)
        .children(free.map(|(before, after)| {
            widgets::figure(
                "Free space after each scan",
                format!(
                    "{} \u{2192} {}",
                    human_bytes(before),
                    human_bytes(after)
                ),
                theme.bright,
                cx,
            )
        }))
        .children(free.map(|(before, after)| {
            widgets::figure(
                "Free space change",
                signed(i128::from(after) - i128::from(before)),
                theme.bright,
                cx,
            )
        }))
        .child(widgets::figure(
            "Listed total change",
            signed(summary.listed),
            if summary.listed > 0 {
                growth(theme)
            } else {
                theme.accent
            },
            cx,
        ))
        .children(summary.unlisted.map(|unlisted| {
            widgets::figure(
                "Not in any listing",
                human_bytes(unlisted),
                theme.warning,
                cx,
            )
        }))
}

/// Of the free space lost, how much listed growth explains, and the rest.
fn gap_bar(
    summary: &Summary,
    theme: &Theme,
) -> Option<gpui_kit::Stateful<Div>> {
    let unlisted = summary.unlisted?;
    let listed = u64::try_from(summary.listed.max(0)).unwrap_or(0);
    let total = (listed + unlisted).max(1) as f32;
    Some(
        div()
            .id("history-gap")
            .debug_selector(|| "history-gap".into())
            .role(Role::Image)
            .aria_label(format!(
                "Of the free space lost, {} is listed growth and {} is in no \
                 listing",
                human_bytes(listed),
                human_bytes(unlisted)
            ))
            .flex()
            // Empty, so a scrolling column would shrink it to nothing.
            .flex_shrink_0()
            .h(space::MD)
            .border_1()
            .border_color(theme.border)
            .child(
                div()
                    .h_full()
                    .w(relative(listed as f32 / total))
                    .bg(growth(theme)),
            )
            .child(
                div()
                    .h_full()
                    .w(relative(unlisted as f32 / total))
                    // The figure's colour, dimmed: told apart from growth by
                    // lightness, not hue alone.
                    .bg(theme.warning.opacity(0.45)),
            ),
    )
}

/// The path below the scope, which heads the window, or the scope itself.
fn below(path: &str, scope: &str) -> String {
    match path.strip_prefix(scope) {
        Some("") => scope.to_owned(),
        Some(rest) => rest.trim_start_matches(['/', '\\']).to_owned(),
        None => path.to_owned(),
    }
}

/// The name and the folder it is in below the scope, as Review shows a
/// row; the scope itself is named for what its own part holds.
fn name_and_folder(path: &str, scope: &str) -> (String, String) {
    if path == scope {
        let name = scope
            .trim_end_matches(['/', '\\'])
            .rsplit(['/', '\\'])
            .next()
            .filter(|name| !name.is_empty())
            .unwrap_or(scope);
        return (
            name.to_owned(),
            "its own files, and folders under the threshold".to_owned(),
        );
    }
    let relative = below(path, scope);
    match relative.rfind(['/', '\\']) {
        Some(at) => (relative[at + 1..].to_owned(), relative[..at].to_owned()),
        None => (relative, "top level".to_owned()),
    }
}

fn change_row(
    index: usize,
    entry: &Changed,
    scale: u128,
    scope: &str,
    theme: &Theme,
    cx: &App,
) -> gpui_kit::Stateful<Div> {
    let delta = entry.own_delta();
    let color = if delta > 0 {
        growth(theme)
    } else {
        theme.accent
    };
    let fraction = (delta.unsigned_abs() as f32 / scale as f32).clamp(0.0, 1.0);
    let (name, folder) = name_and_folder(&entry.path, scope);
    let half = |filled: bool, end: bool| {
        div()
            .flex_1()
            .h_full()
            .flex()
            .when(end, gpui_kit::Styled::justify_end)
            .when(filled, |side| {
                side.child(div().h_full().w(relative(fraction)).bg(color))
            })
    };
    let row = div()
        .id(("history-change", index))
        .debug_selector(move || format!("history-change-{index}"))
        .role(Role::ListItem)
        .aria_label(format!(
            "{}: {} of its own; folder {} to {}{}",
            entry.path,
            signed(delta),
            human_bytes(entry.before),
            human_bytes(entry.after),
            if entry.uncertain {
                "; partly unreadable"
            } else {
                ""
            }
        ))
        .flex()
        .items_center()
        .gap(space::MD)
        .px(space::MD)
        .py(space::SM)
        .border_b_1()
        .border_color(theme.divider())
        .child(
            div()
                .w(size::SIZE_LANE)
                .flex_shrink_0()
                .flex()
                .justify_end()
                .whitespace_nowrap()
                .text_color(color)
                .child(signed(delta)),
        )
        .child(
            div()
                .w(size::SHARE_LANE)
                .h(px(10.))
                .flex_shrink_0()
                .flex()
                .child(half(delta < 0, true))
                .child(div().w(px(1.)).h_full().bg(theme.control_border()))
                .child(half(delta > 0, false)),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .text_color(theme.bright)
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .text_ellipsis()
                        .child(name),
                )
                .when(!folder.is_empty(), |text_block| {
                    text_block.child(
                        div()
                            .text_size(text::CAPTION)
                            .text_color(theme.secondary)
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(folder),
                    )
                }),
        )
        .children(entry.uncertain.then(|| {
            widgets::chip("partly unreadable", theme.warning, cx)
                .flex_shrink_0()
        }))
        .child(
            div()
                .flex_shrink_0()
                .whitespace_nowrap()
                .text_size(text::CAPTION)
                .text_color(theme.secondary)
                .child(format!(
                    "{} \u{2192} {}",
                    human_bytes(entry.before),
                    human_bytes(entry.after)
                )),
        );
    widgets::with_tooltip(row, SharedString::from(entry.path.clone()))
}

fn one_sided_row(
    index: usize,
    entry: &OneSided,
    side: &'static str,
    scope: &str,
    theme: &Theme,
    cx: &App,
) -> gpui_kit::Stateful<Div> {
    let (name, folder) = name_and_folder(&entry.path, scope);
    let (bound, color) = match &entry.other {
        Bound::Under(bytes) => (
            format!("{side}: under {} or absent", human_bytes(*bytes)),
            theme.secondary,
        ),
        Bound::Unknown(why) => {
            (format!("{side}: size unknown, {why}"), theme.warning)
        }
    };
    let row = div()
        .id(ElementId::Name(
            format!("history-{side}-only-{index}").into(),
        ))
        .debug_selector(move || format!("history-{side}-only-{index}"))
        .role(Role::ListItem)
        .aria_label(format!(
            "{}: {}; {bound}",
            entry.path,
            human_bytes(entry.bytes)
        ))
        .flex()
        .items_center()
        .gap(space::MD)
        .px(space::MD)
        .py(space::SM)
        .border_b_1()
        .border_color(theme.divider())
        .child(
            div()
                .w(size::SIZE_LANE)
                .flex_shrink_0()
                .flex()
                .justify_end()
                .text_color(theme.bright)
                .child(human_bytes(entry.bytes)),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(div().text_color(theme.bright).child(name))
                .child(
                    div()
                        .text_size(text::CAPTION)
                        .text_color(theme.secondary)
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .text_ellipsis()
                        .child(folder),
                ),
        )
        .child(widgets::chip(bound, color, cx).flex_shrink_0());
    widgets::with_tooltip(row, SharedString::from(entry.path.clone()))
}
