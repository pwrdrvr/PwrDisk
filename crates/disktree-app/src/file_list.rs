//! Bounded semantic navigation over the complete scan, never its preview.

use std::fmt::Write as _;
use std::path::Path;

use disktree_core::size::human_bytes;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Context, Div, InteractiveElement as _, ParentElement as _, Role,
    SharedString, Stateful, StatefulInteractiveElement as _, Styled, div,
};
use gpui_omarchy::{ActiveTheme as _, ButtonVariant, button};

use crate::state::Disktree;
use crate::ui::{space, text};

/// Explicit pages keep offscreen entries reachable to AT while bounding
/// layout and semantic work independently of the size of the scan.
pub const PAGE_SIZE: usize = 100;

/// Lossless IDs distinguish paths even when their display text is identical.
pub fn path_id(path: &Path) -> SharedString {
    let mut id = String::from("path-");
    for byte in path.as_os_str().as_encoded_bytes() {
        write!(id, "{byte:02x}").expect("writing a string");
    }
    id.into()
}

fn entry_semantics(
    path: &Path,
    label: String,
    selected: bool,
) -> Stateful<Div> {
    div()
        .id(path_id(path))
        .accessibility_id(path_id(path))
        .role(Role::ListBoxOption)
        .aria_label(label)
        .aria_selected(selected)
        .when(
            selected,
            gpui_kit::StatefulInteractiveElement::aria_active_descendant,
        )
}

impl Disktree {
    pub fn file_entries(&self) -> Vec<usize> {
        self.current().map_or_else(Vec::new, |node| {
            (0..node.children.len())
                .filter(|index| {
                    let mut crumbs = self.crumbs.clone();
                    crumbs.push(*index);
                    !self.filter_applied
                        || self
                            .matches
                            .as_ref()
                            .is_none_or(|m| m.keep(&crumbs).is_some())
                })
                .collect()
        })
    }

    pub fn file_step(&mut self, step: isize, cx: &mut Context<'_, Self>) {
        let entries = self.file_entries();
        if entries.is_empty() {
            return;
        }
        let current = self.selected.as_ref().and_then(|crumbs| {
            (crumbs.len() == self.crumbs.len() + 1
                && crumbs.starts_with(&self.crumbs))
            .then(|| entries.iter().position(|i| Some(i) == crumbs.last()))
            .flatten()
        });
        let next = current.map_or(0, |i| {
            i.saturating_add_signed(step).min(entries.len() - 1)
        });
        let mut crumbs = self.crumbs.clone();
        crumbs.push(entries[next]);
        self.pointer_active = false;
        self.file_page = next / PAGE_SIZE;
        self.file_scroll.scroll_to_item(next % PAGE_SIZE);
        self.select(Some(crumbs), cx);
    }
}

pub fn list(app: &Disktree, cx: &Context<'_, Disktree>) -> Stateful<Div> {
    let theme = cx.omarchy();
    let entries = app.file_entries();
    let count = entries.len();
    let page = app.file_page.min(count.saturating_sub(1) / PAGE_SIZE);
    let start = page * PAGE_SIZE;
    let end = (start + PAGE_SIZE).min(count);
    let summary = format!(
        "{} · {} entries · showing {}–{}{}",
        app.current_path().display(),
        count,
        usize::from(count > 0) + start,
        end,
        if app.filter_applied {
            " · filtered"
        } else {
            ""
        }
    );
    let mut rows = div().id("file-rows").role(Role::ListBox)
        .track_focus(&app.file_focus)
        .aria_label(summary.clone())
        .aria_description("Up/Down select; Enter opens a directory; Backspace returns to its parent; Space marks; c reviews. PageUp/PageDown change pages.")
        .flex().flex_col().flex_1().min_h_0().track_scroll(&app.file_scroll).overflow_y_scroll();
    if count == 0 {
        rows = rows.child(
            div()
                .id("empty")
                .role(Role::Label)
                .aria_label("No entries in this directory or filter")
                .p(space::LG)
                .child("No entries in this directory or filter"),
        );
    }
    for (position, &index) in entries.iter().enumerate().take(end).skip(start) {
        let mut crumbs = app.crumbs.clone();
        crumbs.push(index);
        let Some(node) = app.node_at(&crumbs) else {
            continue;
        };
        let Some(path) = app.path_at(&crumbs) else {
            continue;
        };
        let selected = app.selected.as_ref() == Some(&crumbs);
        let marked = app.marks.contains(&path, app.path_style());
        let covered = app.marks.ancestor(&path, app.path_style()).is_some();
        let filtered = app
            .matches
            .as_ref()
            .is_some_and(|m| m.keep(&crumbs).is_none());
        // Clone blocks are not what removing the entry frees; say so
        // where the size is read, and nowhere when there are none.
        let split = crate::views::clone_split(node);
        let clones = split
            .as_ref()
            .map_or_else(String::new, |split| format!(" · {split}"));
        let spoken = split
            .as_ref()
            .map_or_else(String::new, |split| format!("; {split}"));
        let label = format!(
            "{}; {}; {} ({} bytes){}; {}; last written {}; {}{}{}",
            path.display(),
            if node.is_dir() { "directory" } else { "file" },
            human_bytes(node.bytes),
            node.bytes,
            spoken,
            node.category.label(),
            crate::widgets::ago(crate::state::now_seconds(), node.modified),
            if marked {
                "marked"
            } else if covered {
                "inside a marked directory"
            } else {
                "not marked"
            },
            if node.read_error { "; unreadable" } else { "" },
            if filtered {
                "; does not match filter"
            } else {
                ""
            }
        );
        let row = entry_semantics(&path, label, selected)
            .a11y_synthetic_children(move |b| {
                b.parent_node().set_position_in_set(position + 1);
                b.parent_node().set_size_of_set(count);
            })
            .debug_selector(move || format!("file-row-{position}"))
            .p(space::SM)
            .border_b_1()
            .border_color(theme.divider())
            .when(selected, |row| row.bg(theme.hover_fill()).border_l_2())
            .child(format!(
                "{}{} · {}{clones} · {}",
                if node.is_dir() { "▸ " } else { "" },
                node.name,
                human_bytes(node.bytes),
                if marked {
                    "Marked"
                } else {
                    node.category.label()
                }
            ))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.pointer_active = false;
                this.select(Some(crumbs.clone()), cx);
                window.focus(&this.file_focus, cx);
            }));
        rows = rows.child(row);
    }
    div()
        .id("file-list")
        .debug_selector(|| "file-list".into())
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .child(
            div()
                .id("file-summary")
                .role(Role::Label)
                .aria_label(summary.clone())
                .text_size(text::CAPTION)
                .p(space::SM)
                .child(summary),
        )
        .child(rows)
        .child(
            div()
                .flex()
                .gap(space::SM)
                .child(
                    button(
                        "file-open",
                        "Open selected directory",
                        ButtonVariant::Outline,
                        cx,
                    )
                    .disabled(
                        !app.selected
                            .as_ref()
                            .and_then(|c| app.node_at(c))
                            .is_some_and(disktree_core::tree::Node::is_dir),
                    )
                    .on_click(cx.listener(
                        |this, _, window, cx| {
                            this.pointer_active = false;
                            this.descend(cx);
                            window.focus(&this.file_focus, cx);
                        },
                    )),
                )
                .child(
                    button(
                        "file-mark",
                        "Mark or unmark selected path",
                        ButtonVariant::Outline,
                        cx,
                    )
                    .disabled(app.selected.is_none())
                    .on_click(cx.listener(
                        |this, _, _, cx| this.toggle_mark_selected(cx),
                    )),
                ),
        )
        .child(
            div()
                .flex()
                .gap(space::SM)
                .child(
                    button(
                        "file-previous",
                        "Previous 100 entries",
                        ButtonVariant::Outline,
                        cx,
                    )
                    .debug_selector(|| "file-previous".into())
                    .disabled(page == 0)
                    .on_click(cx.listener(
                        move |this, _, _, cx| {
                            this.file_page = page.saturating_sub(1);
                            cx.notify();
                        },
                    )),
                )
                .child(
                    button(
                        "file-next",
                        "Next 100 entries",
                        ButtonVariant::Outline,
                        cx,
                    )
                    .debug_selector(|| "file-next".into())
                    .disabled(end == count)
                    .on_click(cx.listener(
                        move |this, _, _, cx| {
                            this.file_page = page + 1;
                            cx.notify();
                        },
                    )),
                ),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{Element as _, accesskit};

    #[test]
    fn entry_role_selection_and_identity_follow_the_path() {
        let path = Path::new("/Fixtures/Résumé 日本語.txt");
        for selected in [false, true] {
            let element = entry_semantics(
                path,
                "Résumé 日本語.txt; 4096 bytes; marked".into(),
                selected,
            );
            let mut node =
                accesskit::Node::new(element.a11y_role().expect("entry role"));
            element.write_a11y_info(&mut node);
            assert_eq!(node.role(), Role::ListBoxOption);
            assert_eq!(node.is_selected(), Some(selected));
            assert_eq!(
                node.label(),
                Some("Résumé 日本語.txt; 4096 bytes; marked")
            );
            assert_eq!(node.author_id(), Some(path_id(path).as_ref()));
        }
        assert_ne!(path_id(path), path_id(Path::new("/Fixtures/other.txt")));
    }

    #[cfg(unix)]
    #[test]
    fn native_path_ids_do_not_alias_lossy_display_names() {
        use std::os::unix::ffi::OsStrExt as _;
        let a = Path::new(std::ffi::OsStr::from_bytes(b"/Fixtures/\xff"));
        let b = Path::new(std::ffi::OsStr::from_bytes(b"/Fixtures/\xfe"));
        assert_eq!(a.to_string_lossy(), b.to_string_lossy());
        assert_ne!(path_id(a), path_id(b));
    }
}
