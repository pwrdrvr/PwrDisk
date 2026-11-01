//! Toasts in the main window: short notices that answer something the
//! person asked for, or announce the one thing worth acting on. Today that
//! is the updater. Help › Check for Updates… always answers here, and a
//! download that is ready to install is announced once per version.
//! Design: "Settings and Update Toasts" in the PwrDisk Claude Design
//! project, after the Pwr family's update cards (PwrSnap, PwrGit, PwrAgnt).
//!
//! The model is a short list with its own clock. gpui-base's `ToastManager`
//! keeps a toast's remaining time to itself and its `ToastStack` springs
//! the stack about, which a disk tool has no use for; this one only needs
//! to pause while the pointer is over a toast, which is when someone is
//! reading it or reaching for its button.

use std::time::{Duration, Instant, SystemTime};

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, Context, Div, ElementId, FontWeight, Hsla,
    InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, div,
};
use gpui_omarchy::{ActiveTheme as _, ButtonVariant, button};

use crate::semantics::AccessibleButton as _;
use crate::state::Disktree;
use crate::ui::{size, space, text};
use crate::update_channel::Slot;
use crate::updater::{self, Held, Outcome};

/// The element id of the stack, for tests.
pub const STACK_ID: &str = "toast-stack";
/// The one key the updater's toasts share, so "Checking" turns into the
/// answer in place and a second check replaces the first card.
pub const UPDATE: &str = "update";
/// The most toasts on screen; an older one gives way to a newer one.
const MAX: usize = 3;
/// How long an answer stays: the family's 8 s. A failure stays a little
/// longer, as PwrGit's and PwrAgnt's 9 s, because it carries a message.
const ANSWER: Duration = Duration::from_secs(8);
const FAILURE: Duration = Duration::from_secs(9);

/// What colour a toast's title and edge take.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    /// Something to act on, or something under way.
    Accent,
    /// An answer that needs nothing more.
    Neutral,
    /// Something went wrong.
    Danger,
}

/// A toast's buttons. Dismissing is always the last one, so a click aimed
/// at it cannot land on Restart if the card changes under the pointer
/// (PwrAgnt's rule).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// The release's page on GitHub.
    ReleaseNotes(String),
    /// Where to download PwrDisk, for a copy that cannot update itself.
    DownloadPage,
    OpenLogs,
    CheckAgain,
    /// Install the held download and start again; `switch_back` for an
    /// older release.
    Restart {
        switch_back: bool,
    },
    Dismiss,
}

/// One notice.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Toast {
    pub key: &'static str,
    pub tone: Tone,
    pub title: String,
    pub body: String,
    pub actions: Vec<Action>,
    /// How long it stays once shown; `None` stays until dismissed.
    pub timeout: Option<Duration>,
    /// A bar under the body that says work is under way.
    pub busy: bool,
}

#[derive(Clone, Debug)]
struct Entry {
    toast: Toast,
    remaining: Option<Duration>,
}

/// The toasts on screen, oldest first, and the clock their timeouts run on.
#[derive(Clone, Debug, Default)]
pub struct Toasts {
    entries: Vec<Entry>,
    /// The pointer is over the stack: no timeout runs.
    hovered: bool,
    /// When the timeouts were last brought up to date.
    tick: Option<Instant>,
}

impl Toasts {
    /// Shows `toast`, in place of one with the same key.
    pub fn push(&mut self, toast: Toast, now: Instant) {
        self.settle(now);
        self.entries.retain(|entry| entry.toast.key != toast.key);
        let remaining = toast.timeout;
        self.entries.push(Entry { toast, remaining });
        let over = self.entries.len().saturating_sub(MAX);
        self.entries.drain(..over);
    }

    /// Takes away the toast with `key`; whether there was one.
    pub fn dismiss(&mut self, key: &str) -> bool {
        let before = self.entries.len();
        self.entries.retain(|entry| entry.toast.key != key);
        self.forget_hover_if_empty();
        self.entries.len() != before
    }

    /// An empty stack is not drawn, so it never hears the pointer leave:
    /// a click on Dismiss or × takes it away from under the pointer. Left
    /// set, every later toast would wait for a hover that never ends.
    const fn forget_hover_if_empty(&mut self) {
        if self.entries.is_empty() {
            self.hovered = false;
        }
    }

    /// Runs the timeouts up to `now` and drops what has run out; whether
    /// anything went.
    pub fn settle(&mut self, now: Instant) -> bool {
        let elapsed = self
            .tick
            .map_or(Duration::ZERO, |tick| now.saturating_duration_since(tick));
        self.tick = Some(now);
        if self.hovered {
            return false;
        }
        let before = self.entries.len();
        self.entries.retain_mut(|entry| {
            entry.remaining = entry
                .remaining
                .map(|remaining| remaining.saturating_sub(elapsed));
            entry.remaining.is_none_or(|remaining| !remaining.is_zero())
        });
        self.forget_hover_if_empty();
        self.entries.len() != before
    }

    /// Pauses the timeouts while the pointer is over the stack.
    pub fn set_hovered(&mut self, hovered: bool, now: Instant) {
        self.settle(now);
        self.hovered = hovered;
    }

    /// How long until the next toast runs out, if one will.
    pub fn next_deadline(&self) -> Option<Duration> {
        if self.hovered {
            return None;
        }
        self.entries
            .iter()
            .filter_map(|entry| entry.remaining)
            .min()
    }

    #[cfg(test)]
    pub fn get(&self, key: &str) -> Option<&Toast> {
        self.entries
            .iter()
            .find(|entry| entry.toast.key == key)
            .map(|entry| &entry.toast)
    }

    pub const fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Toast> {
        self.entries.iter().map(|entry| &entry.toast)
    }
}

// ── the updater's words ─────────────────────────────────────────────────

/// While a check asked for by hand runs.
pub fn checking(slot: Slot) -> Toast {
    Toast {
        key: UPDATE,
        tone: Tone::Accent,
        title: "Checking for updates".into(),
        body: format!("Asking GitHub for the latest {slot} release\u{2026}"),
        actions: Vec::new(),
        timeout: None,
        busy: true,
    }
}

/// A download that Restart to update installs now.
pub fn ready(held: &Held) -> Toast {
    let (title, verb) = if held.switch_back {
        ("Switch ready", "switch")
    } else {
        ("Update ready", "update")
    };
    Toast {
        key: UPDATE,
        tone: Tone::Accent,
        title: title.into(),
        body: format!("Restart to {verb} to PwrDisk {}.", held.version),
        actions: vec![
            Action::ReleaseNotes(held.version.clone()),
            Action::Restart {
                switch_back: held.switch_back,
            },
            Action::Dismiss,
        ],
        timeout: None,
        busy: false,
    }
}

/// The answer to a check asked for by hand. `ready` is the download Restart
/// would install, which the downloaded outcomes are about; without one they
/// fall back to the log's words.
pub fn answer(
    outcome: &Outcome,
    ready_now: Option<&Held>,
    slot: Slot,
) -> Toast {
    let neutral = |title: &str, body: String, actions: Vec<Action>| Toast {
        key: UPDATE,
        tone: Tone::Neutral,
        title: title.into(),
        body,
        actions,
        timeout: Some(ANSWER),
        busy: false,
    };
    match outcome {
        Outcome::Downloaded(_)
        | Outcome::Pending(_)
        | Outcome::SwitchReady(_)
            if ready_now.is_some() =>
        {
            ready(ready_now.expect("matched on is_some"))
        }
        Outcome::UpToDate(version) => neutral(
            "Up to date",
            format!("PwrDisk {version} is the latest on {slot}."),
            vec![Action::ReleaseNotes(version.clone())],
        ),
        Outcome::NothingPublished(slot) => neutral(
            "Up to date",
            format!("Nothing is published on {slot} yet."),
            Vec::new(),
        ),
        Outcome::NotInstalled(_) => neutral(
            "Updates unavailable",
            "This copy wasn't installed from the DMG, Setup.exe or the \
             MSI, so it doesn't update itself."
                .into(),
            vec![Action::DownloadPage],
        ),
        Outcome::Failed(why) => Toast {
            key: UPDATE,
            tone: Tone::Danger,
            title: "Check failed".into(),
            body: capitalized(why),
            actions: vec![Action::OpenLogs, Action::CheckAgain],
            timeout: Some(FAILURE),
            busy: false,
        },
        other => {
            neutral("Updates", capitalized(&other.to_string()), Vec::new())
        }
    }
}

fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

// ── the main window's side ──────────────────────────────────────────────

/// What the main window remembers about the updater between redraws.
#[derive(Debug, Default)]
pub struct UpdateWatch {
    /// Help › Check for Updates… is waiting for an answer.
    pub asked: bool,
    /// The finished check last looked at, by when it finished.
    pub seen: Option<SystemTime>,
    /// The download last announced: each version once per launch.
    pub announced: Option<String>,
}

impl Disktree {
    /// Help › Check for Updates…: always answers in a toast. A check that
    /// is already running (the startup one, say) gives the answer.
    pub fn check_for_updates(&mut self, cx: &mut Context<'_, Self>) {
        let installed = updater::installed_version();
        let slot = updater::selection().resolve(&installed);
        self.update_watch.asked = true;
        self.show_toast(checking(slot), cx);
        if !updater::state().checking {
            updater::check_in_background(updater::Trigger::Manual);
        }
    }

    /// The updater's state moved: answer a check asked for by hand, or
    /// announce a download once.
    pub fn updates_changed(&mut self, cx: &mut Context<'_, Self>) {
        let installed = updater::installed_version();
        let slot = updater::selection().resolve(&installed);
        self.react_to_updates(&updater::state(), &installed, slot, cx);
    }

    /// [`Self::updates_changed`] for a given state, so tests can hand it one.
    pub fn react_to_updates(
        &mut self,
        state: &updater::State,
        installed: &str,
        slot: Slot,
        cx: &mut Context<'_, Self>,
    ) {
        cx.notify();
        if state.checking {
            return;
        }
        let held = updater::ready(state, installed, slot);
        let finished = state.last.as_ref().map(|(when, _)| *when);
        if finished != self.update_watch.seen {
            self.update_watch.seen = finished;
            if let Some((_, outcome)) = &state.last
                && std::mem::take(&mut self.update_watch.asked)
            {
                let toast = answer(outcome, held.as_ref(), slot);
                if toast
                    .actions
                    .iter()
                    .any(|a| matches!(a, Action::Restart { .. }))
                {
                    self.update_watch.announced =
                        held.as_ref().map(|held| held.version.clone());
                }
                self.show_toast(toast, cx);
                return;
            }
        }
        // A background check, Settings' Check now, or a download held from
        // before: only a ready download is worth a toast, once.
        if let Some(held) = held
            && self.update_watch.announced.as_deref() != Some(&held.version)
        {
            self.update_watch.announced = Some(held.version.clone());
            self.show_toast(ready(&held), cx);
        }
    }

    pub fn show_toast(&mut self, toast: Toast, cx: &mut Context<'_, Self>) {
        let now = cx.background_executor().now();
        self.toasts.push(toast, now);
        self.schedule_toasts(cx);
        cx.notify();
    }

    pub fn dismiss_toast(&mut self, key: &str, cx: &mut Context<'_, Self>) {
        if self.toasts.dismiss(key) {
            self.schedule_toasts(cx);
            cx.notify();
        }
    }

    fn set_toasts_hovered(
        &mut self,
        hovered: bool,
        cx: &mut Context<'_, Self>,
    ) {
        let now = cx.background_executor().now();
        self.toasts.set_hovered(hovered, now);
        self.schedule_toasts(cx);
    }

    /// Wakes when the next toast runs out, rather than ticking.
    fn schedule_toasts(&mut self, cx: &mut Context<'_, Self>) {
        let now = cx.background_executor().now();
        if self.toasts.settle(now) {
            cx.notify();
        }
        self.toast_timer = self.toasts.next_deadline().map(|after| {
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(after).await;
                let _ = this.update(cx, Self::schedule_toasts);
            })
        });
    }

    fn run_toast_action(
        &mut self,
        key: &'static str,
        action: &Action,
        cx: &mut Context<'_, Self>,
    ) {
        match action {
            Action::ReleaseNotes(version) => {
                cx.open_url(&format!(
                    "{}/releases/tag/v{version}",
                    crate::help::REPOSITORY
                ));
            }
            Action::DownloadPage => {
                cx.open_url(&format!("{}/releases", crate::help::REPOSITORY));
            }
            Action::OpenLogs => cx.defer(crate::logs_window::open),
            Action::CheckAgain => self.check_for_updates(cx),
            Action::Restart { .. } => self.restart_to_update(cx),
            Action::Dismiss => self.dismiss_toast(key, cx),
        }
    }
}

/// The element id of a toast's button, for tests.
pub const fn action_id(action: &Action) -> &'static str {
    match action {
        Action::ReleaseNotes(_) => "toast-release-notes",
        Action::DownloadPage => "toast-download",
        Action::OpenLogs => "toast-open-logs",
        Action::CheckAgain => "toast-check-again",
        Action::Restart { .. } => "toast-restart",
        Action::Dismiss => "toast-dismiss",
    }
}

/// The stack, bottom left of the window above the key bar, over the
/// treemap or the review list: the sidebar's free space stays in view; `None` when
/// there is nothing to show.
pub fn stack(app: &Disktree, cx: &Context<'_, Disktree>) -> Option<AnyElement> {
    if app.toasts.is_empty() {
        return None;
    }
    let column = app.toasts.iter().fold(
        div()
            .id(STACK_ID)
            .debug_selector(|| STACK_ID.into())
            .absolute()
            .left(space::LG)
            .bottom(size::TOAST_BOTTOM)
            .w(size::TOAST)
            .flex()
            .flex_col()
            .gap(space::SM)
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                this.set_toasts_hovered(*hovered, cx);
            })),
        |column, toast| column.child(card(app, toast, cx)),
    );
    Some(column.into_any_element())
}

fn card(
    app: &Disktree,
    toast: &Toast,
    cx: &Context<'_, Disktree>,
) -> gpui_kit::Stateful<Div> {
    let theme = cx.omarchy();
    let edge: Hsla = match toast.tone {
        Tone::Accent => theme.accent,
        Tone::Neutral => theme.border,
        Tone::Danger => theme.danger,
    };
    let title_color = match toast.tone {
        Tone::Accent => theme.accent,
        Tone::Neutral => theme.secondary,
        Tone::Danger => theme.danger,
    };
    let key = toast.key;
    let close = gpui_omarchy::icon_button(
        ElementId::Name(format!("toast-close-{key}").into()),
        gpui_omarchy::IconName::Close,
        "Close",
        ButtonVariant::Secondary,
        cx,
    )
    .on_click(cx.listener(move |this, _, _, cx| this.dismiss_toast(key, cx)));
    let mut actions = div().flex().flex_row().items_center().gap(space::XS);
    let mut primary_seen = false;
    for action in &toast.actions {
        // Links lead, then a spacer, then the commands, so Restart and
        // Dismiss sit together at the right.
        let link =
            matches!(action, Action::ReleaseNotes(_) | Action::DownloadPage);
        if !link && !primary_seen {
            primary_seen = true;
            actions = actions.child(div().flex_1());
        }
        actions = actions.child(action_button(app, key, action, cx));
    }
    // A live status, as the window's other notices are, so a screen reader
    // speaks the answer as well as showing it.
    crate::semantics::status(key, format!("{}: {}", toast.title, toast.body))
        .debug_selector(move || format!("toast-{key}"))
        .flex()
        .flex_col()
        .gap(space::XS)
        .p(space::MD)
        .border_1()
        .border_color(edge)
        .bg(theme.surface)
        .shadow_lg()
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(space::SM)
                .child(
                    div()
                        .flex_1()
                        .font_family(theme.mono_font.clone())
                        .text_size(text::CAPTION)
                        .font_weight(FontWeight::BOLD)
                        .text_color(title_color)
                        .child(toast.title.to_uppercase()),
                )
                .child(close),
        )
        .child(
            div()
                .debug_selector(move || format!("toast-{key}-body"))
                .text_color(theme.bright)
                .child(toast.body.clone()),
        )
        .when(toast.busy, |card| {
            card.child(
                div()
                    .h(size::METER)
                    .w_full()
                    .bg(theme.divider())
                    .child(div().h_full().w_1_3().bg(theme.accent)),
            )
        })
        .when(!toast.actions.is_empty(), |card| card.child(actions))
}

fn action_button(
    app: &Disktree,
    key: &'static str,
    action: &Action,
    cx: &Context<'_, Disktree>,
) -> AnyElement {
    let id = action_id(action);
    let (label, variant) = match action {
        Action::ReleaseNotes(_) => ("Release notes", ButtonVariant::Secondary),
        Action::DownloadPage => ("Download page", ButtonVariant::Secondary),
        Action::OpenLogs => ("Open Logs", ButtonVariant::Outline),
        Action::CheckAgain => ("Try again", ButtonVariant::Outline),
        Action::Restart { .. } if !app.can_restart_to_update() => {
            ("Restart after the removal", ButtonVariant::Primary)
        }
        Action::Restart { switch_back: true } => {
            ("Restart to switch", ButtonVariant::Primary)
        }
        Action::Restart { switch_back: false } => {
            ("Restart to update", ButtonVariant::Primary)
        }
        Action::Dismiss => ("Dismiss", ButtonVariant::Outline),
    };
    let action = action.clone();
    button(id, label, variant, cx)
        .debug_selector(move || id.into())
        .semantic_disabled(
            matches!(action, Action::Restart { .. })
                && !app.can_restart_to_update(),
        )
        .on_click(cx.listener(move |this, _, _, cx| {
            this.run_toast_action(key, &action, cx);
        }))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(key: &'static str, timeout: Option<u64>) -> Toast {
        Toast {
            key,
            tone: Tone::Neutral,
            title: key.into(),
            body: String::new(),
            actions: Vec::new(),
            timeout: timeout.map(Duration::from_secs),
            busy: false,
        }
    }

    #[test]
    fn a_toast_runs_out_unless_the_pointer_rests_on_it() {
        let start = Instant::now();
        let mut toasts = Toasts::default();
        toasts.push(note("a", Some(8)), start);
        toasts.push(note("sticky", None), start);
        assert_eq!(toasts.next_deadline(), Some(Duration::from_secs(8)));

        toasts.set_hovered(true, start + Duration::from_secs(3));
        assert_eq!(toasts.next_deadline(), None, "paused while hovered");
        // Ten seconds of reading take nothing off.
        toasts.set_hovered(false, start + Duration::from_secs(13));
        assert_eq!(toasts.next_deadline(), Some(Duration::from_secs(5)));

        assert!(toasts.settle(start + Duration::from_secs(18)));
        assert!(toasts.get("a").is_none());
        assert!(toasts.get("sticky").is_some(), "no timeout, stays");
    }

    #[test]
    fn a_dismissed_stack_forgets_the_pointer_it_hid_from() {
        let now = Instant::now();
        let mut toasts = Toasts::default();
        toasts.push(note(UPDATE, None), now);
        toasts.set_hovered(true, now);
        // Dismiss was clicked: the stack goes, and no hover-out follows.
        assert!(toasts.dismiss(UPDATE));
        toasts.push(note("answer", Some(8)), now);
        assert_eq!(toasts.next_deadline(), Some(Duration::from_secs(8)));
    }

    #[test]
    fn a_key_replaces_its_toast_and_the_oldest_gives_way() {
        let now = Instant::now();
        let mut toasts = Toasts::default();
        toasts.push(note(UPDATE, None), now);
        let mut answer = note(UPDATE, Some(8));
        answer.title = "answer".into();
        toasts.push(answer, now);
        assert_eq!(toasts.iter().count(), 1);
        assert_eq!(
            toasts.get(UPDATE).map(|t| t.title.as_str()),
            Some("answer")
        );

        for key in ["b", "c", "d"] {
            toasts.push(note(key, None), now);
        }
        let keys: Vec<_> = toasts.iter().map(|toast| toast.key).collect();
        assert_eq!(keys, ["b", "c", "d"]);
        assert!(toasts.dismiss("c"));
        assert!(!toasts.dismiss("c"));
    }

    #[test]
    fn every_outcome_answers_and_only_a_download_offers_restart() {
        let slot = Slot::default();
        let held = Held {
            version: "0.15.0".into(),
            file_name: "PwrDisk-0.15.0-full.nupkg".into(),
            slot: slot.key(),
            switch_back: false,
        };
        let up = answer(&Outcome::UpToDate("0.14.0".into()), None, slot);
        assert_eq!(up.title, "Up to date");
        assert!(up.body.contains("0.14.0"), "names the version");
        assert_eq!(up.timeout, Some(ANSWER));

        let got =
            answer(&Outcome::Downloaded("0.15.0".into()), Some(&held), slot);
        assert_eq!(got.title, "Update ready");
        assert_eq!(got.timeout, None, "stays until acted on");
        assert_eq!(
            got.actions.last(),
            Some(&Action::Dismiss),
            "dismiss is last"
        );

        let failed =
            answer(&Outcome::Failed("rate limited".into()), None, slot);
        assert_eq!(failed.tone, Tone::Danger);
        assert_eq!(failed.body, "Rate limited");
        assert!(failed.actions.contains(&Action::CheckAgain));

        let dev =
            answer(&Outcome::NotInstalled("no manifest".into()), None, slot);
        assert_eq!(dev.title, "Updates unavailable");
        assert!(
            !dev.actions
                .iter()
                .any(|a| matches!(a, Action::Restart { .. }))
        );
    }
}
