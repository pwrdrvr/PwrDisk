//! Native auxiliary-window checks over an explicitly supplied invented log.
//! No production log, scan, preferences, or removal surface is opened.
#![allow(
    dead_code,
    reason = "the native fixture reuses production view modules"
)]

#[path = "../src/agent.rs"]
mod agent;
#[path = "../src/agent_removal.rs"]
mod agent_removal;
#[path = "../src/app_menu.rs"]
mod app_menu;
#[path = "../src/appearance.rs"]
mod appearance;
#[path = "../src/aux_window.rs"]
mod aux_window;
#[path = "../src/build_info.rs"]
mod build_info;
#[path = "../src/chrome.rs"]
mod chrome;
#[path = "../src/diag_log.rs"]
mod diag_log;
#[path = "../src/diagnostics.rs"]
mod diagnostics;
#[path = "../src/doc_window.rs"]
mod doc_window;
#[cfg(target_os = "macos")]
#[path = "../src/dock_icon.rs"]
mod dock_icon;
#[path = "../src/file_list.rs"]
mod file_list;
#[path = "../src/git.rs"]
mod git;
#[path = "../src/github_releases.rs"]
mod github_releases;
#[path = "../src/help.rs"]
mod help;
#[path = "../src/log_view.rs"]
mod log_view;
#[path = "../src/logs_window.rs"]
mod logs_window;
#[path = "../src/marks.rs"]
mod marks;
#[path = "../src/palette.rs"]
mod palette;
#[path = "../src/power.rs"]
mod power;
#[path = "../src/safety.rs"]
mod safety;
#[path = "../src/semantics.rs"]
mod semantics;
#[path = "../src/settings.rs"]
mod settings;
#[path = "../src/state.rs"]
mod state;
#[path = "../src/toasts.rs"]
mod toasts;
#[path = "../src/treemap_view.rs"]
mod treemap_view;
#[path = "../src/ui.rs"]
mod ui;
#[path = "../src/update_channel.rs"]
mod update_channel;
#[path = "../src/updater.rs"]
mod updater;
#[path = "../src/views.rs"]
mod views;
#[path = "../src/widgets.rs"]
mod widgets;

use std::path::PathBuf;

fn main() {
    let path = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .expect("supply a contrived log file path");
    assert!(path.is_file(), "the contrived log must already exist");
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            gpui_omarchy::init(cx);
            app_menu::install(cx);
            logs_window::open_window(Some(path), None, cx);
            cx.activate(true);
        });
}
