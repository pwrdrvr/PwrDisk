//! Small semantic surfaces for text that GPUI does not expose implicitly.

use gpui_kit::{
    Div, InteractiveElement as _, Role, Stateful,
    StatefulInteractiveElement as _, div,
};

pub fn text(
    id: &'static str,
    label: impl Into<gpui_kit::SharedString>,
) -> Stateful<Div> {
    div().id(id).role(Role::Label).aria_label(label)
}

pub fn status(id: &'static str, label: String) -> Stateful<Div> {
    // The macOS adapter announces live values, rather than label changes.
    div()
        .id(id)
        .role(Role::Status)
        .aria_label(label.clone())
        .aria_value(label)
        .a11y_synthetic_children(|b| {
            let node = b.parent_node();
            node.set_live(gpui_kit::accesskit::Live::Polite);
            node.set_read_only();
        })
}

impl crate::state::Disktree {
    pub fn accessibility_status(&mut self) -> String {
        use crate::state::Screen;
        use disktree_core::size::human_bytes;
        if self.scan_error.is_some() || !self.is_indexing() {
            self.accessible_progress = None;
        }
        // Review and removal remain the foreground workflow while a previous
        // scan is cancelled or failed, or a completion refresh runs behind it.
        let phase = match self.screen {
            Screen::Explore => {
                if self.awaiting_root {
                    return "Choose a source, then a root. No scan has started."
                        .into();
                }
                if let Some(error) = &self.scan_error {
                    return format!(
                        "Scan failed: {error}. Scan again to retry."
                    );
                }
                if self.is_indexing() {
                    return format!(
                        "{} Escape cancels. Live overview is incomplete and cannot be marked.",
                        self.accessibility_scan_progress()
                    );
                }
                if self.progress.cancelled {
                    "Scan cancelled"
                } else {
                    "Scan complete"
                }
            }
            Screen::Review => "Review marked paths",
            Screen::Running => "Removal running",
            Screen::Done => "Removal finished",
        };
        let background = if self.screen == Screen::Explore {
            String::new()
        } else if let Some(error) = &self.scan_error {
            format!(" Previous scan failed: {error}.")
        } else if self.is_indexing() {
            format!(" Background scan. {}", self.accessibility_scan_progress())
        } else if self.progress.cancelled {
            " Previous scan cancelled.".into()
        } else {
            String::new()
        };
        let volume = self.space.map_or_else(
            || "Volume space unavailable".to_owned(),
            |s| {
                format!(
                    "Volume {} free of {} total",
                    human_bytes(s.available),
                    human_bytes(s.total)
                )
            },
        );
        format!(
            "{phase}. {}. {} marked paths. {volume}.{background} {}{}",
            self.current_path().display(),
            self.marks.len(),
            if self.remote.is_some() {
                "SSH read-only. Space is a scan snapshot. "
            } else {
                ""
            },
            self.notice
                .as_ref()
                .map_or("", |(message, _)| message.as_str())
        )
    }

    fn accessibility_scan_progress(&mut self) -> String {
        let now = std::time::Instant::now();
        if let Some((at, label)) = &self.accessible_progress
            && now.duration_since(*at) < std::time::Duration::from_secs(2)
        {
            return label.clone();
        }
        let label = format!(
            "Scanning {}. Busy; progress is indeterminate. {} files, {} indexed.",
            self.scan_root.display(),
            self.progress.files,
            disktree_core::size::human_bytes(self.progress.bytes)
        );
        self.accessible_progress = Some((now, label.clone()));
        label
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{Element as _, accesskit};

    #[test]
    fn current_status_and_text_have_stable_named_roles() {
        for (element, role, label) in [
            (
                text("source-instructions", "Choose a root"),
                Role::Label,
                "Choose a root",
            ),
            (
                status("current-status", "Scan cancelled".into()),
                Role::Status,
                "Scan cancelled",
            ),
        ] {
            assert_eq!(element.a11y_role(), Some(role));
            let mut node = accesskit::Node::new(role);
            element.write_a11y_info(&mut node);
            assert_eq!(node.label(), Some(label));
        }
    }

    #[test]
    fn status_review_current_updates_supply_an_announced_value() {
        for label in ["Scanning Fixtures", "Scan failed", "Scan complete"] {
            let element = status("current-status", label.into());
            let mut node = accesskit::Node::new(Role::Status);
            element.write_a11y_info(&mut node);
            assert_eq!(node.label(), Some(label));
            assert_eq!(node.value(), Some(label));
        }
    }

    #[test]
    fn status_review_mark_updates_supply_an_announced_value() {
        let label = "1 marked path; 1024 listed bytes";
        let element = status("marked-summary", label.into());
        let mut node = accesskit::Node::new(Role::Status);
        element.write_a11y_info(&mut node);
        assert_eq!(node.label(), Some(label));
        assert_eq!(node.value(), Some(label));
    }
}
