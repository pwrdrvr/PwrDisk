//! The Settings window: its own window, one at a time, as in PwrRoute,
//! PwrGit and PwrSnap. A nav on the left and panes of cards: Updates,
//! Diagnostics and About. No General pane until there is something to put
//! in it: Power Efficiency and the view settings stay in the strip, where
//! they are used. Design: "Menus and Help Windows" in the PwrDisk Claude
//! Design project, turn 3.
//!
//! Updates is the panel the main window used to draw, moved here with its
//! rows, its names and its keys (↑↓ and Enter pick a channel, C checks, R
//! restarts). Restart to update goes through the main window, which knows
//! whether a removal is running and refuses to cut one short.

use std::path::PathBuf;

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, ClipboardItem, Context, Div, ElementId, FocusHandle,
    FontWeight, InteractiveElement as _, IntoElement, KeyDownEvent,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _,
    Styled as _, Window, WindowHandle, div,
};
use gpui_omarchy::{ActiveTheme as _, ButtonVariant, Theme, button};

use crate::aux_window::{self, Spec};
use crate::build_info::BuildInfo;
use crate::chrome::Drags;
use crate::diagnostics::{self, Diagnostics};
use crate::state::Disktree;
use crate::ui::{icon, size, space, text};
use crate::update_channel::{Slot, UpdateSettings};
use crate::updater;

pub use settings_actions::OpenSettings;

#[allow(
    clippy::derive_partial_eq_without_eq,
    reason = "GPUI's actions! macro writes the derives, not us"
)]
mod settings_actions {
    use gpui_kit::actions;

    actions!(
        disktree,
        [
            /// Open the Settings window, or bring it forward.
            OpenSettings,
        ]
    );
}

/// The element id of the window's root, for tests.
pub const ROOT_ID: &str = "settings-root";
pub const CHECK_ID: &str = "updates-check";
pub const RESTART_ID: &str = "updates-restart";
pub const COPY_DIAGNOSTICS_ID: &str = "diagnostics-copy";
pub const COPY_BUILD_ID: &str = "about-copy-build";

const SPEC: Spec = Spec {
    title: "PwrDisk Settings",
    size: (1000.0, 680.0),
    // PwrGit's Settings minimum.
    min: (760.0, 520.0),
};

/// The nav's sections, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pane {
    Updates,
    Diagnostics,
    Safety,
    About,
}

impl Pane {
    pub const ALL: [Self; 4] =
        [Self::Updates, Self::Safety, Self::Diagnostics, Self::About];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Updates => "Updates",
            Self::Diagnostics => "Diagnostics",
            Self::Safety => "Safety",
            Self::About => "About",
        }
    }

    /// The nav item's element id.
    pub const fn nav_id(self) -> &'static str {
        match self {
            Self::Updates => "settings-nav-updates",
            Self::Diagnostics => "settings-nav-diagnostics",
            Self::Safety => "settings-nav-safety",
            Self::About => "settings-nav-about",
        }
    }
}

pub fn install(cx: &mut App) {
    cx.on_action(|_: &OpenSettings, cx| {
        // Settings… reopens on the pane last shown; a first open starts on
        // Updates, the one with something to set.
        cx.defer(|cx| {
            let pane = aux_window::find::<SettingsWindow>(cx, |_| true)
                .and_then(|handle| handle.read(cx).ok().map(|view| view.pane))
                .unwrap_or(Pane::Updates);
            open(pane, cx);
        });
    });
}

/// Opens Settings on `pane`, or brings it forward and switches there.
pub fn open(pane: Pane, cx: &mut App) {
    let existing = aux_window::find::<SettingsWindow>(cx, |_| true);
    if let Some(handle) = existing {
        let _ = handle.update(cx, |view, _, cx| view.show(pane, cx));
    }
    if aux_window::activate(existing, cx) {
        return;
    }
    open_window(pane, cx);
}

/// A new Settings window, for `open` and the tests.
pub fn open_window(
    pane: Pane,
    cx: &mut App,
) -> Option<WindowHandle<SettingsWindow>> {
    aux_window::open(&SPEC, cx, |window, cx| {
        let view = SettingsWindow::new(pane, cx);
        window.focus(&view.focus, cx);
        view
    })
}

/// Redraws Settings when the updater's state moves (`updater::install`).
pub fn refresh(cx: &mut App) {
    if let Some(handle) = aux_window::find::<SettingsWindow>(cx, |_| true) {
        let _ = handle.update(cx, |view, _, cx| {
            view.diagnostics = diagnostics::collect(cx);
            cx.notify();
        });
    }
}

/// The window's view.
pub struct SettingsWindow {
    pub pane: Pane,
    /// The channel row the keys are on.
    pub updates_row: usize,
    /// Where the channel pick is saved: the settings folder; tests point
    /// it at a temporary one.
    pub updates_dir: Option<PathBuf>,
    pub diagnostics: Diagnostics,
    /// The copy button that last copied, which says "Copied".
    copied: Option<&'static str>,
    /// Why the last pick could not be saved.
    pub note: Option<String>,
    focus: FocusHandle,
    strip_drag: bool,
}

impl Drags for SettingsWindow {
    fn strip_drag(&mut self) -> &mut bool {
        &mut self.strip_drag
    }
}

impl SettingsWindow {
    pub fn new(pane: Pane, cx: &Context<'_, Self>) -> Self {
        let installed = updater::installed_version();
        Self {
            pane,
            updates_row: updater::selection().resolve(&installed).index(),
            updates_dir: updater::settings_dir(),
            diagnostics: diagnostics::collect(cx),
            copied: None,
            note: None,
            focus: cx.focus_handle(),
            strip_drag: false,
        }
    }

    fn show(&mut self, pane: Pane, cx: &mut Context<'_, Self>) {
        self.pane = pane;
        self.copied = None;
        self.diagnostics = diagnostics::collect(cx);
        cx.notify();
    }

    fn copy(
        &mut self,
        id: &'static str,
        text: String,
        cx: &mut Context<'_, Self>,
    ) {
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.copied = Some(id);
        cx.notify();
    }

    /// Follow `slot` for updates from now on, and save the pick.
    pub fn pick(&mut self, slot: Slot, cx: &mut Context<'_, Self>) {
        self.updates_row = slot.index();
        self.note = updater::set_selection(
            UpdateSettings::picked(slot),
            self.updates_dir.as_deref(),
        )
        .err()
        .map(|error| {
            format!("Updates follow {slot} until PwrDisk quits: {error}")
        });
        self.diagnostics = diagnostics::collect(cx);
        cx.notify();
    }

    /// Whether the main window would let a restart through now.
    fn can_restart(cx: &App) -> bool {
        cx.windows()
            .into_iter()
            .filter_map(|window| window.downcast::<Disktree>())
            .filter_map(|main| main.read(cx).ok())
            .all(Disktree::can_restart_to_update)
    }

    /// Restart into the download, through the main window: it refuses
    /// while a removal runs, and says so in its notice line.
    fn restart(cx: &mut App) {
        let main = cx
            .windows()
            .into_iter()
            .find_map(|window| window.downcast::<Disktree>());
        match main {
            Some(main) => {
                let _ = main.update(cx, |app, window, cx| {
                    window.activate_window();
                    app.restart_to_update(cx);
                });
            }
            None => {
                let _ = updater::restart_to_update();
            }
        }
    }

    fn on_key(&mut self, event: &KeyDownEvent, cx: &mut Context<'_, Self>) {
        let keystroke = &event.keystroke;
        if keystroke.modifiers.platform || keystroke.modifiers.control {
            return;
        }
        let rows = Slot::ALL.len();
        match keystroke.key.as_str() {
            "tab" => {
                let index = Pane::ALL
                    .iter()
                    .position(|pane| *pane == self.pane)
                    .unwrap_or(0);
                let step = if keystroke.modifiers.shift {
                    Pane::ALL.len() - 1
                } else {
                    1
                };
                self.show(Pane::ALL[(index + step) % Pane::ALL.len()], cx);
            }
            _ if self.pane != Pane::Updates => {}
            "down" | "j" => {
                self.updates_row = (self.updates_row + 1) % rows;
                cx.notify();
            }
            "up" | "k" => {
                self.updates_row = (self.updates_row + rows - 1) % rows;
                cx.notify();
            }
            "enter" | "space" => self.pick(Slot::ALL[self.updates_row], cx),
            "c" if !updater::state().checking => {
                updater::check_in_background(updater::Trigger::Manual);
                cx.notify();
            }
            "r" => {
                let installed = updater::installed_version();
                let slot = updater::selection().resolve(&installed);
                if updater::ready(&updater::state(), &installed, slot).is_some()
                {
                    cx.defer(Self::restart);
                }
            }
            _ => {}
        }
    }
}

impl Render for SettingsWindow {
    fn render(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> impl IntoElement {
        let theme = cx.omarchy().clone();
        let nav = Pane::ALL.iter().fold(
            div()
                .flex()
                .flex_col()
                .gap(space::XXS)
                .w(size::SETTINGS_NAV)
                .flex_shrink_0()
                .p(space::SM)
                .border_r_1()
                .border_color(theme.divider())
                .bg(theme.inset),
            |nav, &pane| {
                let on = pane == self.pane;
                nav.child(
                    div()
                        .id(pane.nav_id())
                        .debug_selector(move || pane.nav_id().into())
                        .px(space::MD)
                        .py(space::SM)
                        .border_1()
                        .border_color(if on {
                            theme.border
                        } else {
                            theme.inset
                        })
                        .when(on, |item| {
                            item.bg(theme.accent.opacity(0.18))
                                .text_color(theme.bright)
                        })
                        .hover(|item| item.bg(theme.hover_fill()))
                        .cursor_pointer()
                        .child(pane.label())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.show(pane, cx);
                        })),
                )
            },
        );
        let pane = match self.pane {
            Pane::Updates => self.updates(&theme, cx),
            Pane::Diagnostics => self.diagnostics_pane(&theme, cx),
            Pane::Safety => self.safety_pane(&theme, cx),
            Pane::About => self.about(&theme, cx),
        };
        let body = div()
            .id(ROOT_ID)
            .debug_selector(|| ROOT_ID.into())
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                this.on_key(event, cx);
            }))
            .flex()
            .flex_row()
            .flex_1()
            .min_h_0()
            .child(nav)
            .child(
                div()
                    .id("settings-scroll")
                    .flex_1()
                    .min_w_0()
                    .overflow_y_scroll()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(space::LG)
                            .px(space::XL)
                            .py(space::XL)
                            .max_w(size::SETTINGS_PANE)
                            .child(pane),
                    ),
            );
        aux_window::frame(&["Settings"], div(), body, window, cx)
    }
}

// ── panes ───────────────────────────────────────────────────────────────

impl SettingsWindow {
    fn safety_pane(&self, theme: &Theme, cx: &Context<'_, Self>) -> Div {
        let current = crate::safety::load(self.updates_dir.as_deref());
        let mut rows = div().flex().flex_col().gap(space::SM);
        for percent in [1, 5, 10, 25, 50, 100] {
            let picked = current
                .as_ref()
                .is_ok_and(|limit| limit.percent() == percent);
            rows = rows.child(
                button(
                    ElementId::Name(format!("safety-limit-{percent}").into()),
                    format!(
                        "{percent}%{}",
                        if picked { " (selected)" } else { "" }
                    ),
                    if picked {
                        ButtonVariant::Primary
                    } else {
                        ButtonVariant::Secondary
                    },
                    cx,
                )
                .debug_selector(move || format!("safety-limit-{percent}"))
                .on_click(cx.listener(
                    move |this, _, window, cx| {
                        this.note =
                            disktree_core::removal_limit::Limit::new(percent)
                                .and_then(|limit| {
                                    crate::safety::save(
                                        this.updates_dir.as_deref(),
                                        limit,
                                    )
                                })
                                .err();
                        window.focus(&this.focus, cx);
                        cx.notify();
                    },
                )),
            );
        }
        let mut body = div().flex().flex_col().gap(space::MD)
            .child(div().text_size(text::TITLE).child("Removal limit"))
            .child("Maximum measured data per operation, as a percentage of each volume's capacity. Applies to both Trash and permanent deletion, from the window and MCP. The default is 5%. Each operation is re-measured before anything is removed.")
            .child("Trash may be recoverable until emptied; restoration is not guaranteed. Permanent deletion cannot be undone through PwrDisk. MCP requires confirmation of the exact paths and mode for every operation.")
            .child(rows);
        if let Err(reason) = current {
            body = body.child(div().text_color(theme.warning).child(reason));
        }
        if let Some(note) = &self.note {
            body =
                body.child(div().text_color(theme.warning).child(note.clone()));
        }
        body
    }

    fn updates(&self, theme: &Theme, cx: &Context<'_, Self>) -> Div {
        use crate::updater::Installation;

        let installed = updater::installed_version();
        let selection = updater::selection();
        let slot = selection.resolve(&installed);
        let state = updater::state();
        let ready = updater::ready(&state, &installed, slot);
        let lede = match updater::installation() {
            Installation::Velopack { .. } => {
                "Checks GitHub Releases at startup and every six hours, and \
                 downloads what it finds. A download installs when you \
                 restart or quit."
            }
            Installation::NotInstalled { .. } => {
                "Not installed from the DMG, Setup.exe or the MSI, so this \
                 copy does not update itself. The channel still says which \
                 releases a check reads."
            }
        };

        let mut rows = div().flex().flex_col();
        for (index, choice) in Slot::ALL.into_iter().enumerate() {
            let current = choice == slot;
            let note = match (current, selection.is_inferred()) {
                (true, true) => " \u{b7} follows this build",
                (true, false) => " \u{b7} picked",
                _ => "",
            };
            rows = rows.child(
                div()
                    .id(ElementId::Name(format!("update-slot-{index}").into()))
                    .debug_selector(move || format!("update-slot-{index}"))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(space::MD)
                    .px(space::MD)
                    .py(space::SM)
                    .when(index == self.updates_row, |row| {
                        row.bg(theme.accent.opacity(0.18))
                    })
                    .hover(|row| row.bg(theme.accent.opacity(0.1)))
                    .cursor_pointer()
                    .child(div().w(icon::MD).flex_shrink_0().when(
                        current,
                        |mark| {
                            mark.child(
                                gpui_omarchy::icon(
                                    gpui_omarchy::IconName::Check,
                                )
                                .size(icon::MD)
                                .text_color(theme.accent),
                            )
                        },
                    ))
                    .child(
                        div()
                            .w(size::UPDATE_SLOT)
                            .flex_shrink_0()
                            .text_color(if current {
                                theme.bright
                            } else {
                                theme.foreground
                            })
                            .child(choice.to_string()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(text::CAPTION)
                            .text_color(theme.secondary)
                            .child(format!("{}{note}", slot_purpose(choice))),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.pick(choice, cx);
                    })),
            );
        }
        let chip = ready.as_ref().map(|held| {
            chip(format!("{} ready", held.version), theme.warning, theme)
                .into_any_element()
        });

        let status = if state.checking {
            "Checking\u{2026}".to_owned()
        } else {
            match &state.last {
                Some((when, outcome)) => {
                    let then = when
                        .duration_since(std::time::UNIX_EPOCH)
                        .map_or(0, |since| {
                            i64::try_from(since.as_secs()).unwrap_or(i64::MAX)
                        });
                    let ago =
                        crate::widgets::ago(crate::state::now_seconds(), then);
                    format!("{ago}: {outcome}")
                }
                None => "Not checked yet this session.".to_owned(),
            }
        };
        let mut actions = div().flex().flex_row().gap(space::SM);
        if let Some(held) = &ready {
            let verb = if held.switch_back { "switch" } else { "update" };
            actions = actions.child(
                button(
                    RESTART_ID,
                    format!("Restart to {verb} to {}", held.version),
                    ButtonVariant::Primary,
                    cx,
                )
                .debug_selector(|| RESTART_ID.into())
                // As the panel had it: never while a removal runs, which an
                // exit would stop partway through the marked list.
                .disabled(!Self::can_restart(cx))
                .on_click(|_, _, cx| cx.defer(Self::restart)),
            );
        }
        actions = actions.child(
            button(CHECK_ID, "Check now", ButtonVariant::Secondary, cx)
                .debug_selector(|| CHECK_ID.into())
                .disabled(state.checking)
                .on_click(cx.listener(|_, _, _, cx| {
                    updater::check_in_background(updater::Trigger::Manual);
                    cx.notify();
                })),
        );

        let mut pane = div()
            .flex()
            .flex_col()
            .gap(space::LG)
            .child(head(
                &format!("PwrDisk {installed}"),
                "Updates",
                lede,
                None,
                theme,
            ))
            .child(card("Release channel", chip, theme).child(rows))
            .child(card_plain(theme).child(field(
                "Last check",
                None,
                vec![
                    div()
                        .debug_selector(|| "updates-status".into())
                        .child(status)
                        .into_any_element(),
                    actions.into_any_element(),
                ],
                theme,
            )));
        if let Some(note) = &self.note {
            pane =
                pane.child(div().text_color(theme.warning).child(note.clone()));
        }
        pane.child(hint(
            "\u{2191}\u{2193} and enter pick a channel \u{b7} c checks \u{b7} \
             r restarts into a download \u{b7} tab switches panes",
            theme,
        ))
    }

    fn diagnostics_pane(&self, theme: &Theme, cx: &Context<'_, Self>) -> Div {
        let model = &self.diagnostics;
        let report = model.report();
        let copy = self.copy_button(COPY_DIAGNOSTICS_ID, "Copy", report, cx);
        let mut pane = div().flex().flex_col().gap(space::LG).child(head(
            "Support",
            "Diagnostics",
            "What this copy runs with. Copy carries these lines and \
             nothing else: no scanned path ever appears here. Help \u{203a} \
             Copy Diagnostics Info copies the same.",
            Some(copy.into_any_element()),
            theme,
        ));
        let mut this_copy = card("This copy", None, theme).child(field(
            "Version",
            None,
            vec![model.version_line().into_any_element()],
            theme,
        ));
        // Paths and Session have their own cards below.
        for section in model
            .sections
            .iter()
            .filter(|section| !matches!(section.title, "Paths" | "Session"))
        {
            for fact in &section.facts {
                this_copy = this_copy.child(field(
                    fact.label,
                    None,
                    vec![fact.value.clone().into_any_element()],
                    theme,
                ));
            }
        }
        pane = pane.child(this_copy);

        let home = std::env::home_dir();
        let shown = |path: &Option<PathBuf>| {
            path.as_deref().map_or_else(
                || "None".to_owned(),
                |path| crate::marks::display_path(path, home.as_deref()),
            )
        };
        let settings_dir = model.settings_dir.clone();
        let log = model.log.clone();
        pane = pane.child(
            card("Files", None, theme)
                .child(field(
                    "Settings folder",
                    Some("The release channel and the power preset."),
                    vec![
                        command_box(shown(&settings_dir), theme)
                            .into_any_element(),
                        button(
                            "diagnostics-open-settings",
                            "Open settings folder",
                            ButtonVariant::Secondary,
                            cx,
                        )
                        .debug_selector(|| "diagnostics-open-settings".into())
                        .disabled(
                            !settings_dir
                                .as_deref()
                                .is_some_and(std::path::Path::exists),
                        )
                        .on_click(move |_, _, cx| {
                            if let Some(dir) = &settings_dir {
                                cx.open_with_system(dir);
                            }
                        })
                        .into_any_element(),
                    ],
                    theme,
                ))
                .child(field(
                    "Log",
                    Some(
                        "Scans, removals and updates, never a scanned path. \
                         Help \u{203a} Logs follows it live.",
                    ),
                    vec![
                        command_box(shown(&log), theme).into_any_element(),
                        button(
                            "diagnostics-open-logs",
                            "Open Logs",
                            ButtonVariant::Secondary,
                            cx,
                        )
                        .debug_selector(|| "diagnostics-open-logs".into())
                        .disabled(log.is_none())
                        .on_click(|_, _, cx| {
                            cx.defer(crate::logs_window::open);
                        })
                        .into_any_element(),
                    ],
                    theme,
                )),
        );
        if let Some(session) = model
            .sections
            .iter()
            .find(|section| section.title == "Session")
        {
            pane = pane.child(session.facts.iter().fold(
                card("This session", None, theme),
                |card, fact| {
                    card.child(field(
                        fact.label,
                        None,
                        vec![fact.value.clone().into_any_element()],
                        theme,
                    ))
                },
            ));
        }
        pane
    }

    fn about(&self, theme: &Theme, cx: &Context<'_, Self>) -> Div {
        let build = BuildInfo::current();
        let trailing = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(space::SM)
            .when(!build.is_release(), |row| {
                row.child(chip("Dev build".into(), theme.warning, theme))
            })
            .child(self.copy_button(
                COPY_BUILD_ID,
                "Copy build info",
                build.report(),
                cx,
            ));
        let mut source = Vec::new();
        if let Some(checkout) = &build.checkout {
            source.push(self.copy_row(
                "about-copy-checkout",
                checkout,
                theme,
                cx,
            ));
        }
        match (build.is_release(), &build.tag, &build.branch) {
            (true, Some(tag), _) => {
                source.push(self.copy_row("about-copy-branch", tag, theme, cx));
            }
            (_, _, Some(branch)) => {
                source.push(self.copy_row(
                    "about-copy-branch",
                    branch,
                    theme,
                    cx,
                ));
            }
            _ => source
                .push("Detached HEAD: the commit names it.".into_any_element()),
        }
        match &build.commit {
            Some(commit) => {
                source.push(self.copy_row(
                    "about-copy-commit",
                    commit,
                    theme,
                    cx,
                ));
            }
            None => source.push(
                "No commit recorded: this copy was built outside a git \
                 checkout."
                    .into_any_element(),
            ),
        }
        if let Some(line) = build.commit_line() {
            source.push(
                div()
                    .text_color(theme.secondary)
                    .child(line)
                    .into_any_element(),
            );
        }
        if let Some(line) = build.changes_line() {
            source.push(
                div()
                    .text_color(theme.warning)
                    .child(line)
                    .into_any_element(),
            );
        }
        let docs = div()
            .flex()
            .flex_row()
            .flex_wrap()
            .gap(space::SM)
            .child(
                button(
                    "about-license",
                    "View license",
                    ButtonVariant::Secondary,
                    cx,
                )
                .debug_selector(|| "about-license".into())
                .on_click(|_, window, cx| {
                    window.dispatch_action(
                        Box::new(crate::help::ViewLicense),
                        cx,
                    );
                }),
            )
            .child(
                button(
                    "about-notices",
                    "Third-party notices",
                    ButtonVariant::Secondary,
                    cx,
                )
                .debug_selector(|| "about-notices".into())
                .on_click(|_, window, cx| {
                    window.dispatch_action(
                        Box::new(crate::help::ThirdPartyNotices),
                        cx,
                    );
                }),
            )
            .child(
                button(
                    "about-changelog",
                    "Changelog",
                    ButtonVariant::Secondary,
                    cx,
                )
                .debug_selector(|| "about-changelog".into())
                .on_click(|_, window, cx| {
                    window.dispatch_action(
                        Box::new(crate::help::OpenChangelog),
                        cx,
                    );
                }),
            );
        let repository = crate::help::REPOSITORY
            .trim_start_matches("https://")
            .to_owned();
        div()
            .flex()
            .flex_col()
            .gap(space::LG)
            .child(head(
                "PwrDisk",
                "About",
                "Finds what is eating a volume and removes what you mark, \
                 with the free space live on screen. A fork of disktree, in \
                 the Pwr family with PwrRoute, PwrGit and PwrAgnt.",
                None,
                theme,
            ))
            .child(
                card("This copy", Some(trailing.into_any_element()), theme)
                    .child(field(
                        "Version",
                        None,
                        vec![
                            format!("PwrDisk {}", build.version)
                                .into_any_element(),
                            div()
                                .text_color(theme.secondary)
                                .child(build.build_line())
                                .into_any_element(),
                        ],
                        theme,
                    ))
                    .child(field(
                        "Source",
                        Some("Read by git when this copy was built."),
                        source,
                        theme,
                    ))
                    .child(field(
                        "License",
                        Some("MIT. Includes disktree, by Tobi L\u{fc}tke."),
                        vec![docs.into_any_element()],
                        theme,
                    )),
            )
            .child(
                card("Project", None, theme)
                    .child(field(
                        "Source",
                        None,
                        vec![
                            mono(repository.clone(), theme).into_any_element(),
                        ],
                        theme,
                    ))
                    .child(field(
                        "Issues",
                        None,
                        vec![
                            mono(format!("{repository}/issues"), theme)
                                .into_any_element(),
                        ],
                        theme,
                    )),
            )
    }

    fn copy_button(
        &self,
        id: &'static str,
        label: &'static str,
        text: String,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement {
        let label = if self.copied == Some(id) {
            "Copied"
        } else {
            label
        };
        button(id, label, ButtonVariant::Secondary, cx)
            .debug_selector(move || id.into())
            .on_click(cx.listener(move |this, _, _, cx| {
                this.copy(id, text.clone(), cx);
            }))
    }

    /// A value in a command box with its own Copy.
    fn copy_row(
        &self,
        id: &'static str,
        value: &str,
        theme: &Theme,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        command_box(value.to_owned(), theme)
            .child(self.copy_button(id, "Copy", value.to_owned(), cx))
            .into_any_element()
    }
}

/// What each release channel slot is for, in the family's words.
const fn slot_purpose(slot: Slot) -> &'static str {
    use crate::update_channel::{Track, Train};
    match (slot.train, slot.track) {
        (Train::Stable, Track::Latest) => "Releases promoted after a check",
        (Train::Stable, Track::Prerelease) => "Candidates for the next one",
        (Train::Beta, Track::Latest) => "Beta builds off main",
        (Train::Beta, Track::Prerelease) => "The newest alpha; may not work",
    }
}

// ── the pane's parts ────────────────────────────────────────────────────

/// The pane's head: an eyebrow, the title, what the pane is for, and an
/// action at the end.
fn head(
    eyebrow: &str,
    title: &'static str,
    lede: &'static str,
    action: Option<AnyElement>,
    theme: &Theme,
) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(space::SM)
        .child(
            div()
                .flex()
                .flex_row()
                .items_end()
                .gap(space::MD)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .child(
                            div()
                                .font_family(theme.mono_font.clone())
                                .text_size(text::CAPTION)
                                .text_color(theme.secondary)
                                .child(SharedString::from(
                                    eyebrow.to_uppercase(),
                                )),
                        )
                        .child(
                            div()
                                .text_size(text::HEADING)
                                .font_weight(FontWeight::BOLD)
                                .text_color(theme.bright)
                                .child(title),
                        ),
                )
                .children(action),
        )
        .child(div().text_color(theme.foreground).child(lede))
}

/// A card: a heading row with something at its end, then rows.
fn card(
    title: &'static str,
    trailing: Option<AnyElement>,
    theme: &Theme,
) -> Div {
    card_plain(theme).child(
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(space::MD)
            .px(space::MD)
            .py(space::SM)
            .border_b_1()
            .border_color(theme.divider())
            .child(
                div()
                    .flex_1()
                    .font_weight(FontWeight::BOLD)
                    .text_color(theme.bright)
                    .child(title),
            )
            .children(trailing),
    )
}

fn card_plain(theme: &Theme) -> Div {
    div()
        .flex()
        .flex_col()
        .border_1()
        .border_color(theme.border)
        .bg(theme.inset)
}

/// A labelled row: the label and what it is for on the left, values on the
/// right, one under the other.
fn field(
    label: &'static str,
    sub: Option<&'static str>,
    values: Vec<AnyElement>,
    theme: &Theme,
) -> Div {
    div()
        .flex()
        .flex_row()
        .gap(space::LG)
        .px(space::MD)
        .py(space::SM)
        .border_b_1()
        .border_color(theme.divider())
        .child(
            div()
                .flex()
                .flex_col()
                .gap(space::XXS)
                .w(size::SETTINGS_LABEL)
                .flex_shrink_0()
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.bright)
                        .child(label),
                )
                .children(sub.map(|sub| {
                    div()
                        .text_size(text::CAPTION)
                        .text_color(theme.secondary)
                        .child(sub)
                })),
        )
        .child(
            div()
                .flex()
                .flex_col()
                // Buttons keep their own width; a command box spans it.
                .items_start()
                .gap(space::SM)
                .flex_1()
                .min_w_0()
                .children(values),
        )
}

/// A value to copy, in monospace on the window's ground.
fn command_box(value: String, theme: &Theme) -> Div {
    div()
        .w_full()
        .flex()
        .flex_row()
        .items_center()
        .gap(space::SM)
        .pl(space::SM)
        .pr(space::XS)
        .py(space::XXS)
        .border_1()
        .border_color(theme.divider())
        .bg(theme.background)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .font_family(theme.mono_font.clone())
                .text_color(theme.bright)
                .child(value),
        )
}

fn mono(value: String, theme: &Theme) -> Div {
    div()
        .font_family(theme.mono_font.clone())
        .text_color(theme.foreground)
        .child(value)
}

fn chip(label: String, color: gpui_kit::Hsla, theme: &Theme) -> Div {
    div()
        .px(space::SM)
        .border_1()
        .border_color(color)
        .text_color(color)
        .font_family(theme.mono_font.clone())
        .text_size(text::CAPTION)
        .child(label)
}

fn hint(text_: &'static str, theme: &Theme) -> Div {
    div()
        .text_size(text::CAPTION)
        .text_color(theme.secondary)
        .child(text_)
}
