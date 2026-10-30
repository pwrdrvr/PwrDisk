//! Help › Logs: `main.log`, live, in its own window. The family's viewer
//! cut to what PwrDisk's log needs: a filter, level toggles with the Error
//! and Warning counts, Live / Paused, Copy (the lines shown), and the
//! file's Copy path and Reveal. Design: "Menus and Help Windows" in the
//! PwrDisk Claude Design project, 4c. PwrRoute's viewer (search modes,
//! context, Wrap, Mark) is the model for more.
//!
//! The file is read again every [`POLL_EVERY`] while Live, from where the
//! last read ended; a rotation (the file shrank) starts it over. Nothing
//! here writes anywhere but the clipboard.

use std::fs;
use std::io::{Read as _, Seek as _, SeekFrom};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::base::input::{InputEvent, InputState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, ClipboardItem, Context, Entity, FontWeight, Hsla,
    InteractiveElement as _, IntoElement, ParentElement as _, Render,
    SharedString, StatefulInteractiveElement as _, Styled as _,
    UniformListScrollHandle, Window, WindowHandle, div, uniform_list,
};
use gpui_omarchy::{ActiveTheme as _, ButtonVariant, Theme, button};

use crate::aux_window::{self, Spec};
use crate::chrome::Drags;
use crate::ui::{size, space, text};

/// How often the file is read for new lines.
const POLL_EVERY: Duration = Duration::from_millis(500);

/// Lines kept: the file rotates at a megabyte, so this holds all of it.
const KEEP: usize = 20_000;

pub const FILTER_ID: &str = "logs-filter";
pub const LIVE_ID: &str = "logs-live";
pub const COPY_ID: &str = "logs-copy";
pub const COPY_PATH_ID: &str = "logs-copy-path";
pub const REVEAL_ID: &str = "logs-reveal";

const SPEC: Spec = Spec {
    title: "PwrDisk Logs",
    size: (1120.0, 720.0),
    min: (720.0, 420.0),
};

/// A record's level, as `diag_log` writes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Level {
    Error,
    Warn,
    Info,
    Debug,
}

impl Level {
    pub const ALL: [Self; 4] =
        [Self::Error, Self::Warn, Self::Info, Self::Debug];

    const fn label(self) -> &'static str {
        match self {
            Self::Error => "Error",
            Self::Warn => "Warn",
            Self::Info => "Info",
            Self::Debug => "Debug",
        }
    }

    const fn id(self) -> &'static str {
        match self {
            Self::Error => "logs-level-error",
            Self::Warn => "logs-level-warn",
            Self::Info => "logs-level-info",
            Self::Debug => "logs-level-debug",
        }
    }

    const fn index(self) -> usize {
        self as usize
    }

    const fn color(self, theme: &Theme) -> Hsla {
        match self {
            Self::Error => theme.danger,
            Self::Warn => theme.warning,
            Self::Info => theme.accent,
            Self::Debug => theme.secondary,
        }
    }
}

/// One line of the log, split where `diag_log::format_line` joined it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    /// Its number in the file, from 1.
    pub number: usize,
    /// "23:51:02.118": the time of day; the date is in the raw line.
    pub time: SharedString,
    pub level: Level,
    pub target: SharedString,
    pub message: SharedString,
    pub raw: SharedString,
}

/// Parses `2026-10-24T07:31:02.123-04:00 WARN  pwrdisk::scan: message`.
/// A line in another shape (a panic's own output, an old format) is kept
/// whole as an Info message.
pub fn parse(number: usize, raw: &str) -> Line {
    let fallback = || Line {
        number,
        time: SharedString::default(),
        level: Level::Info,
        target: SharedString::default(),
        message: raw.to_owned().into(),
        raw: raw.to_owned().into(),
    };
    let Some((stamp, rest)) = raw.split_once(' ') else {
        return fallback();
    };
    let rest = rest.trim_start();
    let Some((level, rest)) = rest.split_once(' ') else {
        return fallback();
    };
    let level = match level {
        "ERROR" => Level::Error,
        "WARN" => Level::Warn,
        "INFO" => Level::Info,
        "DEBUG" | "TRACE" => Level::Debug,
        _ => return fallback(),
    };
    let rest = rest.trim_start();
    let (target, message) = rest.split_once(": ").unwrap_or(("", rest));
    let time = stamp
        .split_once('T')
        .map_or(stamp, |(_, time)| time)
        .get(..12)
        .unwrap_or(stamp);
    Line {
        number,
        time: time.to_owned().into(),
        level,
        target: target.to_owned().into(),
        message: message.to_owned().into(),
        raw: raw.to_owned().into(),
    }
}

/// Which lines show: levels on, and the filter's words all in the line
/// (ignoring case).
pub fn shown(lines: &[Line], levels: [bool; 4], filter: &str) -> Vec<usize> {
    let words: Vec<String> =
        filter.split_whitespace().map(str::to_lowercase).collect();
    lines
        .iter()
        .enumerate()
        .filter(|(_, line)| levels[line.level.index()])
        .filter(|(_, line)| {
            words.is_empty() || {
                let raw = line.raw.to_lowercase();
                words.iter().all(|word| raw.contains(word.as_str()))
            }
        })
        .map(|(index, _)| index)
        .collect()
}

/// Opens Logs, or brings it forward.
pub fn open(cx: &mut App) {
    let existing = aux_window::find::<LogsWindow>(cx, |_| true);
    if aux_window::activate(existing, cx) {
        return;
    }
    open_window(crate::diag_log::path().map(Path::to_path_buf), cx);
}

/// A new Logs window on `path`, for `open` and the tests.
pub fn open_window(
    path: Option<PathBuf>,
    cx: &mut App,
) -> Option<WindowHandle<LogsWindow>> {
    aux_window::open(&SPEC, cx, move |window, cx| {
        LogsWindow::new(path, window, cx)
    })
}

/// The window's view.
pub struct LogsWindow {
    pub path: Option<PathBuf>,
    pub lines: Vec<Line>,
    /// Indexes into `lines` that pass the levels and the filter.
    pub shown: Rc<Vec<usize>>,
    pub levels: [bool; 4],
    pub live: bool,
    filter: Entity<InputState>,
    /// Where the last read ended.
    offset: u64,
    /// A line the last read cut in half, finished by the next.
    partial: String,
    scroll: UniformListScrollHandle,
    note: Option<&'static str>,
    strip_drag: bool,
}

impl Drags for LogsWindow {
    fn strip_drag(&mut self) -> &mut bool {
        &mut self.strip_drag
    }
}

impl LogsWindow {
    pub fn new(
        path: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        let filter = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Filter: words, all of them")
        });
        cx.subscribe(&filter, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.refilter(cx);
            }
        })
        .detach();
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(POLL_EVERY).await;
                let alive = this
                    .update(cx, |this, cx| {
                        if this.live {
                            this.read(cx);
                        }
                    })
                    .is_ok();
                if !alive {
                    break;
                }
            }
        })
        .detach();
        let mut view = Self {
            path,
            lines: Vec::new(),
            shown: Rc::new(Vec::new()),
            // Debug is off until asked for, as the family's viewers start.
            levels: [true, true, true, false],
            live: true,
            filter,
            offset: 0,
            partial: String::new(),
            scroll: UniformListScrollHandle::new(),
            note: None,
            strip_drag: false,
        };
        view.read(cx);
        view
    }

    /// Reads what the file gained since the last read.
    pub fn read(&mut self, cx: &mut Context<'_, Self>) {
        let Some(path) = &self.path else {
            return;
        };
        let Ok(mut file) = fs::File::open(path) else {
            return;
        };
        let length = file.metadata().map_or(0, |meta| meta.len());
        if length < self.offset {
            // Rotated: main.log is a new file.
            self.lines.clear();
            self.offset = 0;
            self.partial.clear();
        }
        if length == self.offset
            || file.seek(SeekFrom::Start(self.offset)).is_err()
        {
            return;
        }
        let mut bytes = Vec::new();
        if file.read_to_end(&mut bytes).is_err() {
            return;
        }
        self.offset += bytes.len() as u64;
        let text = std::mem::take(&mut self.partial)
            + &String::from_utf8_lossy(&bytes);
        let mut pieces: Vec<&str> = text.split('\n').collect();
        // The last piece is a line still being written, or "".
        pieces
            .pop()
            .unwrap_or_default()
            .clone_into(&mut self.partial);
        let first = self.lines.last().map_or(1, |line| line.number + 1);
        self.lines.extend(
            pieces
                .into_iter()
                .enumerate()
                .map(|(index, raw)| parse(first + index, raw.trim_end())),
        );
        if self.lines.len() > KEEP {
            self.lines.drain(..self.lines.len() - KEEP);
        }
        self.refilter(cx);
        if self.live {
            self.scroll.scroll_to_item(
                self.shown.len().saturating_sub(1),
                gpui_kit::ScrollStrategy::Top,
            );
        }
    }

    fn refilter(&mut self, cx: &mut Context<'_, Self>) {
        let filter = self.filter.read(cx).value().to_string();
        self.shown = Rc::new(shown(&self.lines, self.levels, &filter));
        cx.notify();
    }

    pub fn toggle(&mut self, level: Level, cx: &mut Context<'_, Self>) {
        self.levels[level.index()] = !self.levels[level.index()];
        self.refilter(cx);
    }

    /// The shown lines, as written in the file.
    pub fn shown_text(&self) -> String {
        let mut out = String::new();
        for &index in self.shown.iter() {
            out.push_str(&self.lines[index].raw);
            out.push('\n');
        }
        out
    }

    fn count(&self, level: Level) -> usize {
        self.lines.iter().filter(|line| line.level == level).count()
    }
}

impl Render for LogsWindow {
    fn render(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> impl IntoElement {
        let theme = cx.omarchy().clone();
        let path = self.path.clone();
        let reveal_path = self.path.clone();
        let actions = div()
            .flex()
            .flex_row()
            .gap(space::SM)
            .child(
                button(COPY_PATH_ID, "Copy path", ButtonVariant::Secondary, cx)
                    .debug_selector(|| COPY_PATH_ID.into())
                    .disabled(path.is_none())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(path) = &path {
                            cx.write_to_clipboard(ClipboardItem::new_string(
                                path.display().to_string(),
                            ));
                            this.note = Some("Copied the path");
                            cx.notify();
                        }
                    })),
            )
            .child(
                button(REVEAL_ID, "Reveal", ButtonVariant::Secondary, cx)
                    .debug_selector(|| REVEAL_ID.into())
                    .disabled(reveal_path.is_none())
                    .on_click(move |_, _, cx| {
                        if let Some(path) = &reveal_path {
                            cx.reveal_path(path);
                        }
                    }),
            );

        let mut levels = div()
            .flex()
            .flex_row()
            .border_1()
            .border_color(theme.border);
        for level in Level::ALL {
            let on = self.levels[level.index()];
            let count = match level {
                Level::Error | Level::Warn => format!(" {}", self.count(level)),
                Level::Info | Level::Debug => String::new(),
            };
            levels = levels.child(
                div()
                    .id(level.id())
                    .debug_selector(move || level.id().into())
                    .flex()
                    .flex_row()
                    .px(space::SM)
                    .py(space::XXS)
                    .border_r_1()
                    .border_color(theme.divider())
                    .cursor_pointer()
                    .when(on, |item| item.bg(theme.accent.opacity(0.18)))
                    .hover(|item| item.bg(theme.hover_fill()))
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(if on {
                                level.color(&theme)
                            } else {
                                theme.secondary
                            })
                            .child(level.label()),
                    )
                    .child(count)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.toggle(level, cx);
                    })),
            );
        }
        let live_label = if self.live { "Live" } else { "Paused" };
        let toolbar = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(space::SM)
            .px(space::LG)
            .py(space::SM)
            .border_b_1()
            .border_color(theme.divider())
            .child(
                div().w(size::LOG_FILTER).child(
                    gpui_omarchy::input(FILTER_ID, &self.filter, window, cx)
                        .debug_selector(|| FILTER_ID.into()),
                ),
            )
            .child(levels)
            .child(div().flex_1())
            .child(
                div()
                    .debug_selector(|| "logs-count".into())
                    .font_family(theme.mono_font.clone())
                    .text_size(text::CAPTION)
                    .text_color(theme.secondary)
                    .child(format!(
                        "{} of {} lines",
                        self.shown.len(),
                        self.lines.len()
                    )),
            )
            .child(
                button(LIVE_ID, live_label, ButtonVariant::Secondary, cx)
                    .debug_selector(|| LIVE_ID.into())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.live = !this.live;
                        if this.live {
                            this.read(cx);
                        }
                        cx.notify();
                    })),
            )
            .child(
                button(COPY_ID, "Copy", ButtonVariant::Secondary, cx)
                    .debug_selector(|| COPY_ID.into())
                    .disabled(self.shown.is_empty())
                    .on_click(cx.listener(|this, _, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(
                            this.shown_text(),
                        ));
                        this.note = Some("Copied the lines shown");
                        cx.notify();
                    })),
            );

        let rows = self.shown.clone();
        let lines: Rc<Vec<Line>> = Rc::new(
            rows.iter()
                .map(|&index| self.lines[index].clone())
                .collect(),
        );
        let list_theme = theme.clone();
        let body = if self.path.is_none() {
            div()
                .flex_1()
                .p(space::XL)
                .text_color(theme.secondary)
                .child(
                    "No log: PwrDisk could not create its log folder, so \
                     this session writes only to the terminal.",
                )
                .into_any_element()
        } else {
            uniform_list("logs-lines", lines.len(), move |range, _, _| {
                range.map(|index| row(&lines[index], &list_theme)).collect()
            })
            .track_scroll(&self.scroll)
            .flex_1()
            .py(space::XS)
            .into_any_element()
        };
        let place = self.path.as_deref().map_or_else(String::new, |path| {
            crate::marks::display_path(path, std::env::home_dir().as_deref())
        });
        let footer = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(space::MD)
            .px(space::LG)
            .py(space::XS)
            .border_t_1()
            .border_color(theme.divider())
            .text_size(text::CAPTION)
            .text_color(theme.secondary)
            .child(
                div()
                    .text_color(if self.live {
                        theme.success
                    } else {
                        theme.secondary
                    })
                    .child(live_label),
            )
            .child(format!("{place} \u{b7} rotates at 1 MB to main.old.log"))
            .child(div().flex_1())
            .children(self.note.map(|note| div().child(note)));
        let content = div()
            .flex()
            .flex_col()
            .size_full()
            .child(toolbar)
            .child(div().flex().flex_col().flex_1().min_h_0().child(body))
            .child(footer);
        aux_window::frame(&["Help", "Logs"], actions, content, window, cx)
    }
}

/// One line: its number, the time, the level, the target and the message.
fn row(line: &Line, theme: &Theme) -> gpui_kit::Stateful<gpui_kit::Div> {
    div()
        .id(line.number)
        .flex()
        .flex_row()
        .gap(space::SM)
        .px(space::LG)
        .whitespace_nowrap()
        .font_family(theme.mono_font.clone())
        .text_size(text::BODY)
        .child(
            div()
                .w(size::LOG_NUMBER)
                .flex_shrink_0()
                .text_color(theme.secondary)
                .child(line.number.to_string()),
        )
        .child(
            div()
                .flex_shrink_0()
                .text_color(theme.secondary)
                .child(line.time.clone()),
        )
        .child(
            div()
                .w(size::LOG_LEVEL)
                .flex_shrink_0()
                .text_color(line.level.color(theme))
                .child(line.level.label().to_uppercase()),
        )
        .child(
            div()
                .flex_shrink_0()
                .text_color(theme.secondary)
                .child(line.target.clone()),
        )
        .child(div().text_color(theme.bright).child(line.message.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_splits_into_time_level_target_and_message() {
        let line = parse(
            3,
            "2026-10-24T07:31:02.123-04:00 WARN  pwrdisk::updater: updates: no",
        );
        assert_eq!(line.time.as_ref(), "07:31:02.123");
        assert_eq!(line.level, Level::Warn);
        assert_eq!(line.target.as_ref(), "pwrdisk::updater");
        assert_eq!(line.message.as_ref(), "updates: no");
        let odd = parse(4, "thread 'main' panicked");
        assert_eq!(odd.level, Level::Info);
        assert_eq!(odd.message.as_ref(), "thread 'main' panicked");
    }

    #[test]
    fn the_filter_needs_every_word_and_the_levels_hold() {
        let lines: Vec<Line> = [
            "2026-10-24T07:31:02.123-04:00 INFO  pwrdisk: started PwrDisk",
            "2026-10-24T07:31:03.123-04:00 WARN  pwrdisk::updater: GitHub said 403",
            "2026-10-24T07:31:04.123-04:00 DEBUG pwrdisk::scan: 8 workers",
        ]
        .iter()
        .enumerate()
        .map(|(index, raw)| parse(index + 1, raw))
        .collect();
        assert_eq!(shown(&lines, [true, true, true, false], ""), [0, 1]);
        assert_eq!(shown(&lines, [true; 4], "PWRDISK"), [0, 1, 2]);
        assert_eq!(shown(&lines, [true; 4], "github 403"), [1]);
        assert_eq!(shown(&lines, [true; 4], "github 404"), Vec::<usize>::new());
        assert_eq!(shown(&lines, [false, false, true, true], ""), [0, 2]);
    }
}
