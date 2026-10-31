//! Help's document windows: Changelog, View License and Third-Party
//! Notices, as PwrRoute, PwrGit and PwrAgnt show them. The documents are
//! compiled in, so each window shows exactly what this build ships and
//! works offline on every platform, bundle or not.
//!
//! The changelog is drawn from its Markdown (headings, bullets, `code`,
//! **bold**); the license and the notices are monospace text, wrapped at
//! [`WRAP_COLUMNS`] once when the window opens so the notices' megabyte
//! draws as a list of fixed-height rows.

use std::rc::Rc;

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, ClipboardItem, Context, FocusHandle, FontWeight,
    HighlightStyle, InteractiveElement as _, IntoElement, KeyDownEvent,
    ParentElement as _, Render, Role, ScrollHandle, ScrollStrategy,
    SharedString, StatefulInteractiveElement as _, Styled as _, StyledText,
    UniformListScrollHandle, Window, WindowHandle, div, uniform_list,
};
use gpui_omarchy::{ActiveTheme as _, ButtonVariant, Theme, button};

use crate::aux_window::{self, Spec};
use crate::chrome::Drags;
use crate::ui::{size, space, text};

pub const CHANGELOG: &str = include_str!("../../../CHANGELOG.md");
pub const LICENSE: &str = include_str!("../../../LICENSE");
pub const THIRD_PARTY_NOTICES: &str =
    include_str!("../../../THIRD_PARTY_LICENSES");

/// Where the license and the notices wrap.
pub const WRAP_COLUMNS: usize = 100;

/// The window's Copy button.
pub const COPY_ID: &str = "doc-copy";

/// Which document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Doc {
    Changelog,
    License,
    ThirdPartyNotices,
}

impl Doc {
    /// The crumb after "Help ›".
    pub const fn title(self) -> &'static str {
        match self {
            Self::Changelog => "Changelog",
            Self::License => "License",
            Self::ThirdPartyNotices => "Third-Party Notices",
        }
    }

    const fn spec(self) -> Spec {
        Spec {
            title: match self {
                Self::Changelog => "PwrDisk Changelog",
                Self::License => "PwrDisk License",
                Self::ThirdPartyNotices => "PwrDisk Third-Party Notices",
            },
            size: (920.0, 760.0),
            min: (640.0, 480.0),
        }
    }

    pub const fn source(self) -> &'static str {
        match self {
            Self::Changelog => CHANGELOG,
            Self::License => LICENSE,
            Self::ThirdPartyNotices => THIRD_PARTY_NOTICES,
        }
    }
}

/// Opens `doc`'s window, or brings it forward.
pub fn open(doc: Doc, cx: &mut App) {
    let existing = aux_window::find::<DocWindow>(cx, |view| view.doc == doc);
    if aux_window::activate(existing, cx) {
        return;
    }
    open_window(doc, cx);
}

/// A new window for `doc`, for `open` and the tests.
pub fn open_window(doc: Doc, cx: &mut App) -> Option<WindowHandle<DocWindow>> {
    aux_window::open(&doc.spec(), cx, |window, cx| {
        let view = DocWindow::new(doc, cx);
        window.focus(&view.focus, cx);
        view
    })
}

/// The window's view.
pub struct DocWindow {
    pub doc: Doc,
    body: Body,
    copied: bool,
    strip_drag: bool,
    pub focus: FocusHandle,
    pub position: usize,
    block_scroll: ScrollHandle,
}

enum Body {
    Changelog(Vec<Block>),
    /// Wrapped lines, shared with the list's row builder.
    Lines(Rc<Vec<SharedString>>, UniformListScrollHandle),
}

impl Drags for DocWindow {
    fn strip_drag(&mut self) -> &mut bool {
        &mut self.strip_drag
    }
}

impl DocWindow {
    pub fn new(doc: Doc, cx: &Context<'_, Self>) -> Self {
        let body = match doc {
            Doc::Changelog => {
                Body::Changelog(changelog(CHANGELOG, env!("CARGO_PKG_VERSION")))
            }
            Doc::License | Doc::ThirdPartyNotices => Body::Lines(
                Rc::new(
                    wrap(doc.source(), WRAP_COLUMNS)
                        .into_iter()
                        .map(SharedString::from)
                        .collect(),
                ),
                UniformListScrollHandle::new(),
            ),
        };
        Self {
            doc,
            body,
            copied: false,
            strip_drag: false,
            focus: cx.focus_handle().tab_stop(true),
            position: 0,
            block_scroll: ScrollHandle::new(),
        }
    }

    fn key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let key = &event.keystroke.key;
        if key == "tab" {
            crate::semantics::advance_focus(
                window,
                cx,
                event.keystroke.modifiers.shift,
                None,
            );
        } else if (event.keystroke.modifiers.platform
            || event.keystroke.modifiers.control)
            && key == "c"
        {
            self.copy(cx);
        } else {
            let count = match &self.body {
                Body::Changelog(blocks) => blocks.len(),
                Body::Lines(lines, _) => lines.len(),
            };
            self.position = match key.as_str() {
                "home" => 0,
                "end" => count.saturating_sub(1),
                "up" => self.position.saturating_sub(1),
                "down" => {
                    self.position.saturating_add(1).min(count.saturating_sub(1))
                }
                "pageup" => self.position.saturating_sub(20),
                "pagedown" => self
                    .position
                    .saturating_add(20)
                    .min(count.saturating_sub(1)),
                _ => return,
            };
            match &self.body {
                Body::Changelog(_) => {
                    self.block_scroll.scroll_to_item(self.position);
                }
                Body::Lines(_, scroll) => {
                    scroll.scroll_to_item(self.position, ScrollStrategy::Top);
                }
            }
            cx.notify();
        }
        cx.stop_propagation();
    }

    fn copy(&mut self, cx: &mut Context<'_, Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(
            self.doc.source().to_owned(),
        ));
        self.copied = true;
        cx.notify();
    }
}

impl Render for DocWindow {
    fn render(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> impl IntoElement {
        let theme = cx.omarchy().clone();
        let label = if self.copied { "Copied" } else { "Copy" };
        let copy = button(COPY_ID, label, ButtonVariant::Secondary, cx)
            .accessibility_label(format!(
                "{} {}",
                if self.copied {
                    "Copied"
                } else {
                    "Copy complete"
                },
                self.doc.title()
            ))
            .debug_selector(|| COPY_ID.into())
            .on_click(cx.listener(|this, _, _, cx| this.copy(cx)));
        let position = self.position;
        let body = match &self.body {
            Body::Changelog(blocks) => div()
                .id("doc-scroll")
                .debug_selector(|| "doc-scroll".into())
                .role(Role::Document).aria_label("Changelog; Up/Down reads a block, PageUp/PageDown moves twenty blocks, Home/End reaches either end, Command or Control C copies all")
                .track_focus(&self.focus)
                .on_key_down(cx.listener(Self::key))
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .track_scroll(&self.block_scroll)
                .max_w(size::DOCUMENT)
                .px(space::XXL)
                .py(space::XL)
                .children(blocks.iter().enumerate().map(|(index, block)| {
                    div().id(("doc-block", index))
                        .role(if matches!(block, Block::Version { .. }) {
                            Role::Heading
                        } else {
                            Role::Paragraph
                        })
                        .aria_label(block.label())
                        .when(index == position, gpui_kit::StatefulInteractiveElement::aria_active_descendant)
                        .child(draw_block(block, &theme))
                }))
                .into_any_element(),
            Body::Lines(lines, scroll) => {
                let rows = lines.clone();
                let ink = theme.foreground;
                let mono = theme.mono_font.clone();
                div().id("document-text").role(Role::Document)
                    .aria_label(format!("{}; {} lines. Up/Down reads a line, PageUp/PageDown moves twenty lines, Home/End reaches either end, Command or Control C copies all", self.doc.title(), rows.len()))
                    .track_focus(&self.focus).on_key_down(cx.listener(Self::key))
                    .flex_1()
                    .min_h_0()
                    .p(space::LG)
                    .child(
                        div()
                            .size_full()
                            .border_1()
                            .border_color(theme.divider())
                            .bg(theme.inset)
                            .py(space::SM)
                            .child(
                                uniform_list(
                                    "doc-lines",
                                    rows.len(),
                                    move |range, _, _| {
                                        range
                                            .map(|index| {
                                                crate::semantics::text(("document-line", index), format!("Line {}: {}", index + 1, rows[index]))
                                                    .when(index == position, gpui_kit::StatefulInteractiveElement::aria_active_descendant)
                                                    .debug_selector(move || if index == position { "document-current".into() } else { format!("document-line-{index}") })
                                                    .px(space::LG)
                                                    .font_family(mono.clone())
                                                    .text_size(text::BODY)
                                                    .text_color(ink)
                                                    .whitespace_nowrap()
                                                    .child(rows[index].clone())
                                            })
                                            .collect()
                                    },
                                )
                                .track_scroll(scroll)
                                .size_full(),
                            ),
                    )
                    .into_any_element()
            }
        };
        aux_window::frame(&["Help", self.doc.title()], copy, body, window, cx)
    }
}

// ---- The changelog -------------------------------------------------------

/// A piece of the changelog, top to bottom.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    /// `## v0.2.2 - 2026-10-24`: the version, its date, and whether it is
    /// this copy's.
    Version {
        name: String,
        date: Option<String>,
        current: bool,
    },
    /// `- Area - what changed`.
    Bullet(Vec<Span>),
    /// Any other text.
    Paragraph(Vec<Span>),
}

impl Block {
    fn label(&self) -> String {
        match self {
            Self::Version {
                name,
                date,
                current,
            } => format!(
                "{name} {}{}",
                date.as_deref().unwrap_or(""),
                if *current { "; this copy" } else { "" }
            ),
            Self::Bullet(spans) | Self::Paragraph(spans) => {
                spans.iter().map(|span| span.text.as_str()).collect()
            }
        }
    }
}

/// A run of inline text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub style: Inline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Inline {
    Plain,
    Bold,
    Code,
}

/// The changelog's blocks: the `# Changelog` title is the window's crumb,
/// and a version with nothing under it (an empty Unreleased) is left out.
pub fn changelog(markdown: &str, version: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut paragraph: Vec<&str> = Vec::new();
    let flush = |paragraph: &mut Vec<&str>, blocks: &mut Vec<Block>| {
        if !paragraph.is_empty() {
            blocks.push(Block::Paragraph(spans(&paragraph.join(" "))));
            paragraph.clear();
        }
    };
    for line in markdown.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            flush(&mut paragraph, &mut blocks);
        } else if let Some(heading) = trimmed.strip_prefix("## ") {
            flush(&mut paragraph, &mut blocks);
            let (name, date) = match heading.split_once(" - ") {
                Some((name, date)) => {
                    (name.trim(), Some(date.trim().to_owned()))
                }
                None => (heading.trim(), None),
            };
            let current = name.strip_prefix('v') == Some(version);
            blocks.push(Block::Version {
                name: name.to_owned(),
                date,
                current,
            });
        } else if trimmed.starts_with("# ") {
            flush(&mut paragraph, &mut blocks);
        } else if trimmed.starts_with("###") {
            // A subsection ("### Fixed"): its words in bold, as a line.
            flush(&mut paragraph, &mut blocks);
            blocks.push(Block::Paragraph(vec![Span {
                text: trimmed.trim_start_matches('#').trim().to_owned(),
                style: Inline::Bold,
            }]));
        } else if let Some(item) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            flush(&mut paragraph, &mut blocks);
            blocks.push(Block::Bullet(bullet_spans(item)));
        } else if let Some(Block::Bullet(last)) = blocks.last_mut()
            && paragraph.is_empty()
            && line.starts_with("  ")
        {
            // A bullet's continuation line.
            for span in std::iter::once(Span {
                text: " ".to_owned(),
                style: Inline::Plain,
            })
            .chain(spans(trimmed))
            {
                match last.last_mut() {
                    Some(end) if end.style == span.style => {
                        end.text.push_str(&span.text);
                    }
                    _ => last.push(span),
                }
            }
        } else {
            paragraph.push(trimmed);
        }
    }
    flush(&mut paragraph, &mut blocks);
    // The text above the first version is for whoever writes the notes
    // (how a release's section is used), not for someone reading them.
    let first = blocks
        .iter()
        .position(|block| matches!(block, Block::Version { .. }))
        .unwrap_or(blocks.len());
    blocks.drain(..first);
    // Drop versions with nothing under them.
    let mut kept = Vec::with_capacity(blocks.len());
    for (index, block) in blocks.iter().enumerate() {
        let empty_version = matches!(block, Block::Version { .. })
            && !matches!(
                blocks.get(index + 1),
                Some(Block::Bullet(_) | Block::Paragraph(_))
            );
        if !empty_version {
            kept.push(block.clone());
        }
    }
    kept
}

/// "Area - what changed": the area in bold, as the changelog's convention.
fn bullet_spans(item: &str) -> Vec<Span> {
    match item.split_once(" - ") {
        Some((area, rest))
            if !area.contains('`') && area.split_whitespace().count() <= 5 =>
        {
            let mut out = vec![Span {
                text: format!("{area} \u{2014} "),
                style: Inline::Bold,
            }];
            out.extend(spans(rest));
            out
        }
        _ => spans(item),
    }
}

/// Inline Markdown: `code` and **bold**; everything else as written.
pub fn spans(text: &str) -> Vec<Span> {
    let mut out: Vec<Span> = Vec::new();
    let mut push = |text: &str, style: Inline| {
        if text.is_empty() {
            return;
        }
        match out.last_mut() {
            Some(last) if last.style == style => last.text.push_str(text),
            _ => out.push(Span {
                text: text.to_owned(),
                style,
            }),
        }
    };
    let mut rest = text;
    while !rest.is_empty() {
        let code = rest.find('`');
        let bold = rest.find("**");
        match (code, bold) {
            (Some(start), bold) if bold.is_none_or(|bold| start < bold) => {
                if let Some(len) = rest[start + 1..].find('`') {
                    push(&rest[..start], Inline::Plain);
                    push(&rest[start + 1..start + 1 + len], Inline::Code);
                    rest = &rest[start + 2 + len..];
                } else {
                    push(rest, Inline::Plain);
                    rest = "";
                }
            }
            (_, Some(start)) => {
                if let Some(len) = rest[start + 2..].find("**") {
                    push(&rest[..start], Inline::Plain);
                    push(&rest[start + 2..start + 2 + len], Inline::Bold);
                    rest = &rest[start + 4 + len..];
                } else {
                    push(rest, Inline::Plain);
                    rest = "";
                }
            }
            _ => {
                push(rest, Inline::Plain);
                rest = "";
            }
        }
    }
    out
}

fn draw_block(block: &Block, theme: &Theme) -> AnyElement {
    match block {
        Block::Version {
            name,
            date,
            current,
        } => div()
            .flex()
            .items_center()
            .gap(space::MD)
            .pt(space::XL)
            .pb(space::SM)
            .mb(space::XS)
            .border_b_1()
            .border_color(theme.divider())
            .child(
                div()
                    .text_size(text::HEADING)
                    .font_weight(FontWeight::BOLD)
                    .text_color(theme.bright)
                    .child(name.clone()),
            )
            .when_some(date.clone(), |row, date| {
                row.child(
                    div()
                        .font_family(theme.mono_font.clone())
                        .text_size(text::CAPTION)
                        .text_color(theme.secondary)
                        .child(date),
                )
            })
            .when(*current, |row| {
                row.child(
                    div()
                        .px(space::SM)
                        .border_1()
                        .border_color(theme.success)
                        .text_color(theme.success)
                        .font_family(theme.mono_font.clone())
                        .text_size(text::CAPTION)
                        .child("This copy"),
                )
            })
            .into_any_element(),
        Block::Bullet(spans) => div()
            .flex()
            .gap(space::MD)
            .py(space::XS)
            .child(
                div()
                    .flex_none()
                    .pt(space::SM)
                    .child(div().size(size::BULLET).bg(theme.accent)),
            )
            .child(styled(spans, theme).flex_1().min_w_0())
            .into_any_element(),
        Block::Paragraph(spans) => {
            styled(spans, theme).py(space::XS).into_any_element()
        }
    }
}

/// Spans as one wrapping run: bold in the bright ink, code in the accent
/// on a faint fill (a highlight cannot change font).
fn styled(spans: &[Span], theme: &Theme) -> gpui_kit::Div {
    let mut content = String::new();
    let mut highlights = Vec::new();
    for span in spans {
        let start = content.len();
        content.push_str(&span.text);
        let style = match span.style {
            Inline::Plain => continue,
            Inline::Bold => HighlightStyle {
                color: Some(theme.bright),
                font_weight: Some(FontWeight::BOLD),
                ..Default::default()
            },
            Inline::Code => HighlightStyle {
                color: Some(theme.accent),
                background_color: Some(theme.accent.opacity(0.12)),
                ..Default::default()
            },
        };
        highlights.push((start..content.len(), style));
    }
    div()
        .text_size(text::BODY)
        .line_height(gpui_kit::relative(1.6))
        .text_color(theme.foreground)
        .child(StyledText::new(content).with_highlights(highlights))
}

// ---- Plain text ----------------------------------------------------------

/// `text`'s lines, each broken at spaces to at most `columns` characters
/// (a word longer than that is cut). Tabs become four spaces.
pub fn wrap(text: &str, columns: usize) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.replace('\t', "    ");
        if line.chars().count() <= columns {
            out.push(line);
            continue;
        }
        let indent: String = line.chars().take_while(|c| *c == ' ').collect();
        // A deep indent would leave no room: drop it.
        let indent = if indent.len() * 2 < columns {
            indent
        } else {
            String::new()
        };
        let mut current = indent.clone();
        let mut has_word = false;
        for word in line.split(' ').filter(|word| !word.is_empty()) {
            if has_word
                && current.chars().count() + 1 + word.chars().count() > columns
            {
                out.push(std::mem::replace(&mut current, indent.clone()));
                has_word = false;
            }
            if has_word {
                current.push(' ');
            }
            current.push_str(word);
            has_word = true;
            while current.chars().count() > columns {
                out.push(current.chars().take(columns).collect());
                current = current.chars().skip(columns).collect();
            }
        }
        if !current.is_empty() {
            out.push(current);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_changelog_reads_as_versions_and_bullets() {
        let blocks = changelog(
            "# Changelog\n\nHow to write these notes.\n\n## Unreleased\n\n## v0.2.2 - 2026-10-24\n\n- Browser hand-off - \
             Gives the `front` away.\n  Continued here.\n- Plain bullet with **bold**\n\n\
             Important: a note.\n\n### Fixed\n\n## v0.2.1 - 2026-10-23\n\n- Rules - One.\n",
            "0.2.2",
        );
        assert_eq!(
            blocks,
            vec![
                Block::Version {
                    name: "v0.2.2".into(),
                    date: Some("2026-10-24".into()),
                    current: true,
                },
                Block::Bullet(vec![
                    Span {
                        text: "Browser hand-off \u{2014} ".into(),
                        style: Inline::Bold
                    },
                    Span {
                        text: "Gives the ".into(),
                        style: Inline::Plain
                    },
                    Span {
                        text: "front".into(),
                        style: Inline::Code
                    },
                    Span {
                        text: " away. Continued here.".into(),
                        style: Inline::Plain
                    },
                ]),
                Block::Bullet(vec![
                    Span {
                        text: "Plain bullet with ".into(),
                        style: Inline::Plain
                    },
                    Span {
                        text: "bold".into(),
                        style: Inline::Bold
                    },
                ]),
                Block::Paragraph(vec![Span {
                    text: "Important: a note.".into(),
                    style: Inline::Plain
                }]),
                Block::Paragraph(vec![Span {
                    text: "Fixed".into(),
                    style: Inline::Bold
                }]),
                Block::Version {
                    name: "v0.2.1".into(),
                    date: Some("2026-10-23".into()),
                    current: false,
                },
                Block::Bullet(vec![
                    Span {
                        text: "Rules \u{2014} ".into(),
                        style: Inline::Bold
                    },
                    Span {
                        text: "One.".into(),
                        style: Inline::Plain
                    },
                ]),
            ]
        );
    }

    #[test]
    fn the_shipped_changelog_reads_as_versions() {
        let blocks = changelog(CHANGELOG, env!("CARGO_PKG_VERSION"));
        assert!(
            blocks
                .iter()
                .any(|block| matches!(block, Block::Version { .. })),
            "CHANGELOG.md has no version sections"
        );
    }

    #[test]
    fn unclosed_marks_stay_as_written() {
        assert_eq!(
            spans("a `b and **c"),
            vec![Span {
                text: "a `b and **c".into(),
                style: Inline::Plain
            }]
        );
    }

    #[test]
    fn long_lines_wrap_at_spaces_and_keep_their_indent() {
        assert_eq!(
            wrap("short\n  one two three four", 10),
            ["short", "  one two", "  three", "  four"]
        );
        assert_eq!(wrap("abcdefghijkl", 5), ["abcde", "fghij", "kl"]);
        assert_eq!(wrap("", 5), Vec::<String>::new());
        assert_eq!(wrap("a\n\nb", 5), ["a", "", "b"]);
    }

    #[test]
    fn the_documents_are_compiled_in() {
        assert!(LICENSE.contains("MIT"));
        assert!(THIRD_PARTY_NOTICES.len() > 10_000);
        assert!(
            wrap(THIRD_PARTY_NOTICES, WRAP_COLUMNS)
                .iter()
                .all(|line| line.chars().count() <= WRAP_COLUMNS)
        );
    }
}
