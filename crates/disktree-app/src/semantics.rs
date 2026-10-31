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

pub fn status(label: String) -> Stateful<Div> {
    div()
        .id("current-status")
        .role(Role::Status)
        .aria_label(label)
        .a11y_synthetic_children(|b| {
            b.parent_node().set_live(gpui_kit::accesskit::Live::Polite);
        })
}

impl crate::state::Disktree {
    pub fn accessibility_status(&mut self) -> String {
        use disktree_core::size::human_bytes;
        if let Some(error) = &self.scan_error {
            self.accessible_progress = None;
            return format!("Scan failed: {error}. Scan again to retry.");
        }
        if self.is_indexing() {
            let now = std::time::Instant::now();
            if let Some((at, label)) = &self.accessible_progress
                && now.duration_since(*at) < std::time::Duration::from_secs(2)
            {
                return label.clone();
            }
            let label = format!(
                "Scanning {}. Busy; progress is indeterminate. {} files, {} indexed. Escape cancels. Live overview is incomplete and cannot be marked.",
                self.root_path.display(),
                self.progress.files,
                human_bytes(self.progress.bytes)
            );
            self.accessible_progress = Some((now, label.clone()));
            return label;
        }
        self.accessible_progress = None;
        if self.awaiting_root {
            return "Choose a source, then a root. No scan has started.".into();
        }
        let phase = if self.progress.cancelled {
            "Scan cancelled"
        } else {
            match self.screen {
                crate::state::Screen::Explore => "Scan complete",
                crate::state::Screen::Review => "Review marked paths",
                crate::state::Screen::Running => "Removal running",
                crate::state::Screen::Done => "Removal finished",
            }
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
            "{phase}. {}. {} marked paths. {volume}. {}{}",
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
                status("Scan cancelled".into()),
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
}
