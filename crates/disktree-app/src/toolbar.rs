//! How much of the explore bar fits, and the ⋮ menu that holds the rest.
//!
//! The bar folds by what is changed least: the scan options first (they
//! apply on the next scan), then history and depth (which have keys), then
//! Size | Files | Age. The Treemap | List switch never folds.
//!
//! Also the Sizes menu, which says what a size counts: on disk, private or
//! apparent. It sits with the scan options, since one of its three is one.
//! On disk or Private is remembered between launches; Apparent is not
//! (see [`save_sizes`]).

use std::io;
use std::path::{Path, PathBuf};

use gpui_kit::Context;
use gpui_omarchy::Status;

use crate::state::Disktree;

/// Widest first. Measured in rems, not pixels, so interface zoom walks
/// through the tiers exactly as a narrower window does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum BarFit {
    /// Every control in the bar; no ⋮.
    #[default]
    Full,
    /// Hidden files, Sizes and the scan workers go in ⋮.
    Folded,
    /// History and Depth join them.
    Narrow,
    /// Size | Files | Age too: only the view switch and ⋮ are left.
    Tight,
}

/// At and above this many rems across, the whole bar fits beside a
/// readable trail: the settings take about 51 rem, the stoplights, mark
/// and wordmark about 12, and the trail keeps at least 13.
const FULL_REMS: f32 = 76.0;
/// Below this the side panel gives way too (`PANEL_SHOWN_REMS`); it is
/// where the settings used to wrap onto a second row.
const NARROW_REMS: f32 = 52.0;
/// Below this Size | Files | Age (11 rem) would leave the trail no room.
const TIGHT_REMS: f32 = 40.0;
/// The switch keeps its words down to here; under it, glyphs alone.
const LABELLED_SWITCH_REMS: f32 = 64.0;

impl BarFit {
    pub fn for_width(rems: f32) -> Self {
        if rems >= FULL_REMS {
            Self::Full
        } else if rems >= NARROW_REMS {
            Self::Folded
        } else if rems >= TIGHT_REMS {
            Self::Narrow
        } else {
            Self::Tight
        }
    }

    pub fn labelled_switch(rems: f32) -> bool {
        rems >= LABELLED_SWITCH_REMS
    }

    pub fn shows_scan_options(self) -> bool {
        self == Self::Full
    }

    pub fn shows_history_and_depth(self) -> bool {
        self <= Self::Folded
    }

    pub fn shows_mode(self) -> bool {
        self <= Self::Narrow
    }

    /// The ⋮ menu's rows, top to bottom. The most-used come first, as they
    /// would read left to right in a wider bar.
    pub fn more_items(self) -> Vec<MoreItem> {
        let mut items = Vec::new();
        if !self.shows_mode() {
            items.extend([MoreItem::Mode(0), MoreItem::Mode(1)]);
            items.push(MoreItem::Mode(2));
        }
        if !self.shows_history_and_depth() {
            items.extend([MoreItem::Back, MoreItem::Forward, MoreItem::Depth]);
        }
        if !self.shows_scan_options() {
            items.push(MoreItem::Hidden);
            items.extend(Sizes::ALL.map(MoreItem::Sizes));
            items.push(MoreItem::Workers);
        }
        items
    }
}

/// What a size counts, chosen from the Sizes menu or its rows in ⋮.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sizes {
    /// Allocated blocks: every file's whole allocation, clone blocks it
    /// may share with other clones included.
    #[default]
    OnDisk,
    /// What each holds alone ([`disktree_core::tree::Node::private_bytes`]).
    /// The same scan as On disk, weighted differently: no rescan.
    Private,
    /// File lengths. A scan of its own, with no clone facts.
    Apparent,
}

impl Sizes {
    pub const ALL: [Self; 3] = [Self::OnDisk, Self::Private, Self::Apparent];

    pub const fn name(self) -> &'static str {
        match self {
            Self::OnDisk => "On disk",
            Self::Private => "Private",
            Self::Apparent => "Apparent",
        }
    }

    /// What the choice does, under its name in the menu.
    pub const fn detail(self) -> &'static str {
        match self {
            Self::OnDisk => "Allocated blocks, clones counted in full",
            Self::Private => "Only what each holds alone. No rescan.",
            Self::Apparent => "File lengths. Rescans, without clone facts.",
        }
    }

    pub const fn key(self) -> &'static str {
        match self {
            Self::OnDisk => "sizes-on-disk",
            Self::Private => "sizes-private",
            Self::Apparent => "sizes-apparent",
        }
    }

    const fn index(self) -> usize {
        match self {
            Self::OnDisk => 0,
            Self::Private => 1,
            Self::Apparent => 2,
        }
    }

    /// The choice as `--sizes` and the saved setting spell it.
    pub const fn flag(self) -> &'static str {
        match self {
            Self::OnDisk => "on-disk",
            Self::Private => "private",
            Self::Apparent => "apparent",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|sizes| sizes.flag() == value)
    }
}

/// Where the Sizes choice is kept, beside the Power Efficiency preset.
pub fn sizes_settings_path() -> Option<PathBuf> {
    crate::power::config_base().map(|base| base.join("pwrdisk/sizes"))
}

/// The remembered Sizes choice: On disk when none was saved.
pub fn load_sizes(path: &Path) -> io::Result<Sizes> {
    match std::fs::read_to_string(path) {
        Ok(value) => match Sizes::parse(value.trim()) {
            Some(Sizes::Apparent) | None => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "unknown Sizes choice",
            )),
            Some(sizes) => Ok(sizes),
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            Ok(Sizes::OnDisk)
        }
        Err(error) => Err(error),
    }
}

/// Remember `sizes` for the next launch. Only On disk and Private are
/// kept: they weigh one block scan two ways, while Apparent measures
/// something else, file lengths without clone facts, and that is asked
/// for each time (`--apparent-size`, or the menu) rather than carried
/// into every later launch unseen. Choosing it keeps the saved weighing,
/// which a launch without it comes back to.
pub fn save_sizes(path: &Path, sizes: Sizes) -> io::Result<()> {
    if sizes == Sizes::Apparent {
        return Ok(());
    }
    crate::power::write_setting(path, sizes.flag())
}

/// One row of the ⋮ menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoreItem {
    /// Size, Files or Age, by its index in the segmented control.
    Mode(usize),
    Back,
    Forward,
    /// Depth, adjusted in place with ← → or the row's − and +.
    Depth,
    Hidden,
    /// One of the Sizes menu's choices.
    Sizes(Sizes),
    /// Opens the Power Efficiency menu in its place.
    Workers,
}

impl MoreItem {
    /// The section a row belongs to; a caption opens each.
    pub const fn section(self) -> &'static str {
        match self {
            Self::Mode(_) => "Measure by",
            Self::Back | Self::Forward | Self::Depth => "View",
            Self::Hidden | Self::Workers => "Scan",
            Self::Sizes(_) => "Sizes",
        }
    }
}

impl Disktree {
    /// Treemap or file list. The list takes the keyboard when it appears, so
    /// a screen reader lands in it; the treemap gives it back to the root.
    pub fn show_file_list(&mut self, on: bool, cx: &mut Context<'_, Self>) {
        self.more_menu = None;
        if self.file_list != on {
            self.file_list = on;
            self.pointer_active = false;
        }
        cx.notify();
    }

    /// Open ⋮ on its first row, or close it.
    pub fn toggle_more_menu(&mut self, cx: &mut Context<'_, Self>) {
        self.crumb_menu = None;
        // The worker presets open in ⋮'s place; ⋮ then closes them.
        let was_open = self.more_menu.is_some() || self.power_menu.is_some();
        self.power_menu = None;
        self.sizes_menu = None;
        self.more_menu = if was_open {
            None
        } else {
            self.next_enabled_more(&self.bar_fit.more_items(), None, true)
        };
        cx.notify();
    }

    /// The next row after `from` (or the first, from `None`) that can act,
    /// so the highlight never rests on a disabled Back or Forward.
    fn next_enabled_more(
        &self,
        items: &[MoreItem],
        from: Option<usize>,
        forward: bool,
    ) -> Option<usize> {
        let enabled = |&index: &usize| self.more_item_enabled(items[index]);
        match (from, forward) {
            (None, _) => (0..items.len()).find(enabled),
            (Some(at), true) => (at + 1..items.len()).find(enabled),
            (Some(at), false) => (0..at).rev().find(enabled),
        }
    }

    /// Whether a ⋮ row can act now. History rows wait for somewhere to go.
    pub fn more_item_enabled(&self, item: MoreItem) -> bool {
        match item {
            MoreItem::Back => self.can_go_back(),
            MoreItem::Forward => self.can_go_forward(),
            MoreItem::Sizes(sizes) => self.sizes_offered(sizes),
            _ => true,
        }
    }

    /// Do what a ⋮ row says. Every row but Depth closes the menu.
    pub fn activate_more(
        &mut self,
        item: MoreItem,
        cx: &mut Context<'_, Self>,
    ) {
        if !self.more_item_enabled(item) {
            return;
        }
        if item != MoreItem::Depth {
            self.more_menu = None;
        }
        match item {
            MoreItem::Mode(index) => self.set_mode(index, cx),
            MoreItem::Back => self.go_back(cx),
            MoreItem::Forward => self.go_forward(cx),
            MoreItem::Depth => {}
            MoreItem::Hidden => self.toggle_hidden(cx),
            MoreItem::Sizes(sizes) => self.set_sizes(sizes, cx),
            MoreItem::Workers => self.toggle_power_menu(cx),
        }
        cx.notify();
    }

    /// What a size counts now. A Private choice made earlier reads as On
    /// disk where Private is not offered (an SSH host, no clone facts),
    /// and comes back with a scan that offers it.
    pub fn sizes(&self) -> Sizes {
        if self.options.apparent_size {
            Sizes::Apparent
        } else if self.layout_options.private
            && self.sizes_offered(Sizes::Private)
        {
            Sizes::Private
        } else {
            Sizes::OnDisk
        }
    }

    /// Whether sizes read as private now: Private is chosen and offered,
    /// and sizes are bytes, the only measure with a private figure.
    /// Everything ranked or shown by size follows this, as the tiles do.
    pub fn private_weights(&self) -> bool {
        self.options.metric == disktree_core::tree::Metric::Bytes
            && self.sizes() == Sizes::Private
    }

    /// Whether `sizes` can be chosen here. Private needs clone facts,
    /// which only a local scan on macOS with clone metadata gathers;
    /// anywhere else every private size equals its on-disk one.
    pub fn sizes_offered(&self, sizes: Sizes) -> bool {
        sizes != Sizes::Private
            || (cfg!(target_os = "macos")
                && self.options.apfs_clone_metadata
                && self.remote.is_none())
    }

    /// Count sizes as `sizes`. Between On disk and Private only the weights
    /// change, so the tree on screen is laid out again; Apparent, or
    /// leaving it, measures differently and rescans.
    pub fn set_sizes(&mut self, sizes: Sizes, cx: &mut Context<'_, Self>) {
        self.sizes_menu = None;
        if !self.sizes_offered(sizes) {
            cx.notify();
            return;
        }
        self.remember_sizes(sizes, cx);
        let apparent = sizes == Sizes::Apparent;
        // The layout is cached by its options, these weights among them,
        // so the next frame lays the tree out again.
        self.layout_options.private = sizes == Sizes::Private;
        if apparent == self.options.apparent_size {
            cx.notify();
            return;
        }
        self.options.apparent_size = apparent;
        self.start_scan(cx);
    }

    /// Save a Sizes choice for the next launch, as the Power Efficiency
    /// preset is. Said only when it could not be: the choice itself shows
    /// in the bar. No settings directory is a session-only setting.
    fn remember_sizes(&mut self, sizes: Sizes, cx: &mut Context<'_, Self>) {
        let Some(path) = self.sizes_settings_path.as_deref() else {
            return;
        };
        if let Err(error) = save_sizes(path, sizes) {
            self.say_until_closed(
                format!(
                    "{} for now, but it could not be saved: {error}",
                    sizes.name()
                ),
                Status::Warning,
                cx,
            );
        }
    }

    /// `d`: the next choice of the Sizes menu this scan can offer.
    pub fn cycle_sizes(&mut self, cx: &mut Context<'_, Self>) {
        let at = self.sizes().index();
        let next = (1..=Sizes::ALL.len())
            .map(|step| Sizes::ALL[(at + step) % Sizes::ALL.len()])
            .find(|&sizes| self.sizes_offered(sizes))
            .unwrap_or_default();
        self.set_sizes(next, cx);
    }

    /// Open the Sizes menu on the current choice, or close it.
    pub fn toggle_sizes_menu(&mut self, cx: &mut Context<'_, Self>) {
        self.crumb_menu = None;
        self.power_menu = None;
        self.more_menu = None;
        self.sizes_menu = if self.sizes_menu.is_some() {
            None
        } else {
            Some(self.sizes().index())
        };
        cx.notify();
    }

    /// Keys while the Sizes menu is open, as the power menu takes them.
    /// The arrows step over a choice this scan cannot offer.
    pub fn on_sizes_key(
        &mut self,
        key: &str,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        let Some(highlighted) = self.sizes_menu else {
            return false;
        };
        let offered: Vec<usize> = (0..Sizes::ALL.len())
            .filter(|&index| self.sizes_offered(Sizes::ALL[index]))
            .collect();
        let step = |forward: bool| {
            let next = if forward {
                offered.iter().find(|&&index| index > highlighted)
            } else {
                offered.iter().rev().find(|&&index| index < highlighted)
            };
            next.copied().unwrap_or(highlighted)
        };
        match key {
            "down" | "j" => self.sizes_menu = Some(step(true)),
            "up" | "k" => self.sizes_menu = Some(step(false)),
            "home" => self.sizes_menu = offered.first().copied(),
            "end" => self.sizes_menu = offered.last().copied(),
            "enter" | "space" => {
                self.set_sizes(Sizes::ALL[highlighted], cx);
            }
            "escape" => self.sizes_menu = None,
            _ => {
                self.sizes_menu = None;
                cx.notify();
                return false;
            }
        }
        cx.notify();
        true
    }

    /// Keys while ⋮ is open, as the sibling and power menus take them. Any
    /// other key closes it and goes on to the screen.
    pub fn on_more_key(
        &mut self,
        key: &str,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        let Some(highlighted) = self.more_menu else {
            return false;
        };
        let items = self.bar_fit.more_items();
        let Some(&item) = items.get(highlighted) else {
            self.more_menu = None;
            return false;
        };
        let step = |forward: bool| {
            self.next_enabled_more(&items, Some(highlighted), forward)
                .unwrap_or(highlighted)
        };
        match key {
            "down" | "j" => self.more_menu = Some(step(true)),
            "up" | "k" => self.more_menu = Some(step(false)),
            "home" => {
                self.more_menu = self.next_enabled_more(&items, None, true);
            }
            "end" => {
                self.more_menu = (0..items.len())
                    .rev()
                    .find(|&index| self.more_item_enabled(items[index]));
            }
            "left" | "h" | "-" if item == MoreItem::Depth => {
                self.adjust_depth(-1, cx);
            }
            "right" | "l" | "+" | "=" if item == MoreItem::Depth => {
                self.adjust_depth(1, cx);
            }
            "enter" | "space" => self.activate_more(item, cx),
            "escape" => self.more_menu = None,
            _ => {
                self.more_menu = None;
                cx.notify();
                return false;
            }
        }
        cx.notify();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bar_folds_least_used_first() {
        assert_eq!(BarFit::for_width(90.0), BarFit::Full);
        assert_eq!(BarFit::for_width(76.0), BarFit::Full);
        // 1100 px and the 900 px minimum window, at 100%.
        assert_eq!(BarFit::for_width(68.75), BarFit::Folded);
        assert_eq!(BarFit::for_width(56.25), BarFit::Folded);
        assert_eq!(BarFit::for_width(45.0), BarFit::Narrow);
        // The minimum window at 175%.
        assert_eq!(BarFit::for_width(32.0), BarFit::Tight);

        assert!(BarFit::Full.more_items().is_empty());
        assert_eq!(
            BarFit::Folded.more_items(),
            [
                MoreItem::Hidden,
                MoreItem::Sizes(Sizes::OnDisk),
                MoreItem::Sizes(Sizes::Private),
                MoreItem::Sizes(Sizes::Apparent),
                MoreItem::Workers
            ]
        );
        assert_eq!(BarFit::Narrow.more_items().len(), 8);
        assert_eq!(BarFit::Tight.more_items()[0], MoreItem::Mode(0));
        assert_eq!(BarFit::Tight.more_items().len(), 11);
    }

    /// On disk and Private come back as saved; Apparent is never written,
    /// so choosing it keeps the weighing saved before, and a file that
    /// names it, or anything else, is reported rather than read.
    #[test]
    fn the_saved_sizes_are_a_weighing_never_apparent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config/sizes");
        assert_eq!(load_sizes(&path).expect("missing"), Sizes::OnDisk);
        save_sizes(&path, Sizes::Apparent).expect("nothing to write");
        assert!(!path.exists(), "Apparent is not remembered");
        for sizes in [Sizes::Private, Sizes::OnDisk, Sizes::Private] {
            save_sizes(&path, sizes).expect("save");
            assert_eq!(load_sizes(&path).expect("load"), sizes);
        }
        save_sizes(&path, Sizes::Apparent).expect("nothing to write");
        assert_eq!(load_sizes(&path).expect("kept"), Sizes::Private);
        for corrupt in ["apparent", "sizes-private", ""] {
            std::fs::write(&path, corrupt).expect("corrupt");
            assert_eq!(
                load_sizes(&path).expect_err(corrupt).kind(),
                io::ErrorKind::InvalidData
            );
        }
        assert!(save_sizes(dir.path(), Sizes::Private).is_err());
        for sizes in Sizes::ALL {
            assert_eq!(Sizes::parse(sizes.flag()), Some(sizes));
        }
    }

    #[test]
    fn the_switch_keeps_its_words_down_to_64_rem() {
        assert!(BarFit::labelled_switch(68.75));
        assert!(!BarFit::labelled_switch(56.25));
    }
}
