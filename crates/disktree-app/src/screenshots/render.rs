//! Real Metal pixels and macOS text in GPUI `TestWindows`, on the main thread.

use std::{path::Path, sync::Arc, time::Duration};

use anyhow::{Context as _, Result, ensure};
use disktree_core::{classify::Category, scan::ScanOptions};
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Entity, HeadlessAppContext, MouseButton,
    MouseDownEvent, MouseUpEvent, PlatformInput, ScrollDelta, ScrollWheelEvent,
    TouchPhase, point, px, size,
};

use super::scenario::{Scenario, State, Step, Theme};
use crate::{legend::ChipId, state::Disktree};

pub fn run() -> Result<()> {
    if let Some(expected) = std::env::var_os("PWRDISK_SCREENSHOT_CHECKOUT") {
        let expected =
            dunce::canonicalize(expected)?.join("crates/disktree-app");
        let compiled = dunce::canonicalize(env!("CARGO_MANIFEST_DIR"))?;
        ensure!(
            compiled == expected,
            "stale screenshot binary: built in {}, expected {}",
            compiled.display(),
            expected.display()
        );
    }
    println!(
        "screenshot build: {} ({})",
        env!("PWRDISK_BUILD_COMMIT"),
        env!("CARGO_MANIFEST_DIR")
    );
    let args: Vec<_> = std::env::args_os().skip(2).collect();
    ensure!(
        args.len() == 3,
        "usage: pwrdisk offscreen-screenshots SCENARIO.json ROOT OUTPUT_DIR"
    );
    // Parse everything before creating a platform or touching the scan root.
    let scenario = Scenario::parse(&std::fs::read_to_string(&args[0])?)?;
    disktree_core::size::set_units(
        disktree_core::size::SizeUnits::PlatformDefault,
    );
    let root = dunce::canonicalize(&args[1]).context("fixture scan root")?;
    ensure!(root.is_dir(), "fixture root must be a directory");
    let out = Path::new(&args[2]);
    std::fs::create_dir_all(out)?;
    render(&scenario, &root, out)
}

fn render(scenario: &Scenario, root: &Path, out: &Path) -> Result<()> {
    // Only borrow native text. The app below owns a TestPlatform: even
    // additional GPUI windows and input events stay inside that platform.
    // MacPlatform must be constructed on main, which libtest cannot do.
    let platform = gpui_kit::platform::current_platform(true);
    let mut cx = HeadlessAppContext::with_platform(
        platform.text_system(),
        Arc::new(gpui_kit::assets::Assets),
        gpui_kit::platform::current_headless_renderer,
    );
    cx.update(|cx| {
        // No gpui_omarchy::init: it watches the operator's theme files.
        super::init(cx);
        match scenario.theme {
            Theme::TokyoNight => gpui_omarchy::Theme::tokyo_night(),
            Theme::FlexokiLight => gpui_omarchy::Theme::flexoki_light(),
        }
        .apply(cx);
    });
    let options = ScanOptions::default();
    let tree = disktree_core::scan::scan(root, options.clone())?;
    let [width, height] = scenario.size.map(f32::from);
    let handle = cx.open_window(size(px(width), px(height)), |_, cx| {
        cx.new(|cx| {
            let mut app =
                Disktree::with_tree(root.to_path_buf(), tree, options, 3, cx);
            // Never persist fixture choices in the operator's settings.
            app.removal_settings_dir = None;
            app.power_settings_path = None;
            app
        })
    })?;
    let view = cx.update(|cx| handle.entity(cx))?;
    let mut window = handle.into();
    draw(&mut cx, window)?;
    let mut pointer = None;
    for step in &scenario.steps {
        match step {
            Step::Settings => {
                let settings =
                    cx.open_window(size(px(width), px(height)), |_, cx| {
                        cx.new(|cx| {
                            let mut settings =
                                crate::settings::SettingsWindow::new(
                                    crate::settings::Pane::General,
                                    cx,
                                );
                            settings.updates_dir = None;
                            settings
                        })
                    })?;
                window = settings.into();
            }
            Step::UsageHistory { compare } => {
                let dir = invented_reports(root, out)?;
                let compare = *compare;
                let history =
                    cx.open_window(size(px(width), px(height)), |_, cx| {
                        cx.new(|cx| {
                            let mut history =
                                crate::usage_history::UsageHistory::new(
                                    Some(dir),
                                    cx,
                                );
                            // Newest first: today is After, a week ago
                            // Before.
                            history.pick_before(1, cx);
                            history.pick_after(0, cx);
                            if compare {
                                history.compare_now(cx);
                            }
                            history
                        })
                    })?;
                window = history.into();
            }
            Step::Hover { at } => {
                move_pointer(&mut cx, window, *at)?;
                pointer = Some(*at);
            }
            Step::Click { at } => {
                move_pointer(&mut cx, window, *at)?;
                draw(&mut cx, window)?;
                // Dispatch both edges through real hit testing. No OS input.
                for input in click_events(*at) {
                    cx.update_window(window, |_, window, cx| {
                        window.dispatch_event(input, cx);
                    })?;
                    draw(&mut cx, window)?;
                }
                pointer = Some(*at);
            }
            Step::Scroll { at, pixels, events } => {
                move_pointer(&mut cx, window, *at)?;
                draw(&mut cx, window)?;
                for input in scroll_events(*at, *pixels, *events) {
                    cx.update_window(window, |_, window, cx| {
                        window.dispatch_event(input, cx);
                    })?;
                    draw(&mut cx, window)?;
                }
                pointer = Some(*at);
            }
            Step::Wait { ms } => {
                cx.advance_clock(Duration::from_millis(*ms));
            }
            Step::Setup { state } => {
                setup(&mut cx, &view, state, root)?;
            }
            Step::Capture {
                name,
                pointer: show,
            } => {
                draw(&mut cx, window)?;
                let mut image = cx.capture_screenshot(window)?;
                if *show && let Some(at) = pointer {
                    draw_pointer(&mut image, at, width);
                }
                let path = out.join(name);
                image.save_with_format(&path, image::ImageFormat::Png)?;
                // Kit exposes observed controls, not every debug_selector.
                let bounds = cx.update_window(window, |_, window, _| {
                    gpui_kit::base::test_support::snapshots(window)
                        .into_iter()
                        .map(|element| {
                            let b = element.bounds();
                            serde_json::json!({
                                "path": format!("{:?}", element.path()),
                                "bounds": [f32::from(b.origin.x),
                                    f32::from(b.origin.y),
                                    f32::from(b.size.width),
                                    f32::from(b.size.height)],
                                "visible": element.visible()
                            })
                        })
                        .collect::<Vec<_>>()
                })?;
                std::fs::write(
                    path.with_extension("bounds.json"),
                    serde_json::to_vec_pretty(&bounds)?,
                )?;
                println!(
                    "{}: {}x{} pixels, {}x{} logical; drawn pointer: {show}",
                    path.display(),
                    image.width(),
                    image.height(),
                    scenario.size[0],
                    scenario.size[1]
                );
            }
        }
        draw(&mut cx, window)?;
    }
    Ok(())
}

/// Two usage reports of `root` a week apart, written into `out`. Every
/// folder and size is invented, under the fixture's own root: nothing is
/// read from this computer.
fn invented_reports(root: &Path, out: &Path) -> Result<std::path::PathBuf> {
    use disktree_core::path_style::PathStyle;
    use disktree_core::space::SpaceInfo;
    use disktree_core::tree::{Draft, Metric, NodeKind, Tree, aggregate};
    use disktree_core::usage_report::{self, Capture, Volume, library};

    const GB: u64 = 1_000_000_000;
    const MB: u64 = 1_000_000;
    // Beside the fixture tree, on the fixture image: the window prints its
    // folder, and that must be invented too.
    let dir = root.parent().map_or_else(
        || out.join("usage-reports"),
        |up| up.join("usage-reports"),
    );
    if dir.exists() {
        std::fs::remove_dir_all(&dir)?;
    }
    // (path below the root, bytes) a week ago and today.
    let week_ago: &[(&str, u64)] = &[
        ("Projects/worktrees", 41 * GB),
        ("Projects/agent-shell/node_modules", 2_400 * MB),
        ("Projects/pwr-tools/target", 3 * GB),
        ("Cache/build", 6_200 * MB),
        ("Media/recordings", 2_100 * MB),
        ("Documents", 7_900 * MB),
    ];
    let today: &[(&str, u64)] = &[
        ("Projects/worktrees", 50_600 * MB),
        ("Projects/worktrees/wt-4821", 6_400 * MB),
        ("Projects/agent-shell/node_modules", 4_200 * MB),
        ("Projects/pwr-tools/target", 2_400 * MB),
        ("Cache/build", 9_300 * MB),
        ("Media/recordings", 40 * MB),
        ("Documents", 6_800 * MB),
    ];
    for (captured_at, free, folders) in [
        (1_789_200_000, 48 * GB, week_ago),
        (1_789_804_800, 26 * GB, today),
    ] {
        let mut tree = Draft::directory("Demo");
        for (path, bytes) in folders {
            let mut node = &mut tree;
            for name in path.split('/') {
                let at = node
                    .children
                    .iter()
                    .position(|child| &*child.name == name)
                    .unwrap_or_else(|| {
                        node.children.push(Draft::directory(name));
                        node.children.len() - 1
                    });
                node = &mut node.children[at];
            }
            // A nested folder's bytes are part of its parent's.
            let nested: u64 = folders
                .iter()
                .filter(|(other, _)| {
                    other.len() > path.len()
                        && other.starts_with(path)
                        && other.as_bytes()[path.len()] == b'/'
                })
                .map(|(_, bytes)| bytes)
                .sum();
            node.children.push(Draft::entry(
                "data",
                NodeKind::File,
                bytes - nested,
            ));
        }
        let mut tree = Tree::from(tree);
        aggregate(&mut tree, Metric::Bytes);
        let space = SpaceInfo {
            total: 245 * GB,
            free,
            available: free,
            native: None,
        };
        let capture = Capture {
            root: root.to_string_lossy().into_owned(),
            style: PathStyle::Native,
            options: ScanOptions::default(),
            captured_at,
            scan_started_at: Some(captured_at - 31),
            scan_elapsed_ms: Some(31_000),
            host: None,
            volume: Volume {
                device: Some("/dev/fixture".to_owned()),
                mount: Some(root.to_string_lossy().into_owned()),
                space_before: Some(space),
                space_after: Some(space),
            },
            cancelled: false,
            unreadable: 0,
            unreadable_detail: Vec::new(),
            excluded: Vec::new(),
        };
        let report = usage_report::build(&tree, &capture, 100 * MB);
        library::save(&dir, &report)?;
    }
    Ok(dir)
}

fn draw(cx: &mut HeadlessAppContext, window: AnyWindowHandle) -> Result<()> {
    // Asset loads and layout can invalidate the frame that requested them.
    // Three passes settle the same pipeline the window harness uses.
    for _ in 0..3 {
        cx.run_until_parked();
        cx.update_window(window, |_, window, cx| window.draw(cx).clear(cx))?;
    }
    Ok(())
}

fn move_pointer(
    cx: &mut HeadlessAppContext,
    window: AnyWindowHandle,
    at: [f32; 2],
) -> Result<()> {
    cx.update_window(window, |_, window, cx| {
        window.simulate_mouse_move(point(px(at[0]), px(at[1])), cx);
    })
}

/// A swipe as macOS reports one, with no clock between events: began,
/// moved, ended. Positive `pixels` is a swipe up, which zooms in.
fn scroll_events(
    at: [f32; 2],
    pixels: f32,
    events: u16,
) -> impl Iterator<Item = PlatformInput> {
    let position = point(px(at[0]), px(at[1]));
    (0..events).map(move |index| {
        let touch_phase = match index {
            0 => TouchPhase::Started,
            _ if index + 1 == events => TouchPhase::Ended,
            _ => TouchPhase::Moved,
        };
        PlatformInput::ScrollWheel(ScrollWheelEvent {
            position,
            delta: ScrollDelta::Pixels(point(px(0.), px(pixels))),
            modifiers: gpui_kit::Modifiers::default(),
            touch_phase,
        })
    })
}

fn click_events(at: [f32; 2]) -> [PlatformInput; 2] {
    let position = point(px(at[0]), px(at[1]));
    [
        PlatformInput::MouseDown(MouseDownEvent {
            button: MouseButton::Left,
            position,
            modifiers: gpui_kit::Modifiers::default(),
            click_count: 1,
            first_mouse: false,
        }),
        PlatformInput::MouseUp(MouseUpEvent {
            button: MouseButton::Left,
            position,
            modifiers: gpui_kit::Modifiers::default(),
            click_count: 1,
        }),
    ]
}

fn setup(
    cx: &mut HeadlessAppContext,
    view: &Entity<Disktree>,
    state: &State,
    root: &Path,
) -> Result<()> {
    cx.update(|cx| {
        view.update(cx, |app, cx| {
            match state {
                State::SizeUnits { units } => {
                    let units = disktree_core::size::SizeUnits::parse(units)
                        .context("unknown size units")?;
                    disktree_core::size::set_units(units);
                    cx.refresh_windows();
                }
                State::Legend { kind } => {
                    let index = Category::LEGEND
                        .iter()
                        .position(|k| k.name() == kind)
                        .context("unknown legend kind")?;
                    app.cycle_legend(ChipId::Kind(index), cx);
                }
                State::Select { path }
                | State::Mark { path }
                | State::Removed { path } => {
                    let path_in_tree = app.root_path.join(path);
                    let crumbs = app
                        .crumbs_for_path(&path_in_tree)
                        .with_context(|| format!("no fixture path: {path}"))?;
                    if matches!(state, State::Mark { .. }) {
                        app.toggle_mark(&crumbs, cx);
                        ensure!(
                            app.marks.contains(&path_in_tree, app.path_style()),
                            "fixture mark was refused: {path}"
                        );
                    } else if matches!(state, State::Removed { .. }) {
                        // Simulate the successful report without deleting
                        // fixture files or approving a removal operation.
                        let target =
                            app.target_at(&crumbs).context("target")?;
                        app.note_removed(vec![target]);
                    } else {
                        app.select(Some(crumbs), cx);
                    }
                }
                State::Review => app.open_review(cx),
                State::PermanentReview => {
                    // Reproduce the selected mode without confirming removal.
                    app.removal_mode =
                        disktree_core::removal::RemovalMode::Permanent;
                    app.open_review(cx);
                }
                State::CleanupReview => {
                    // Proposals only: an empty search path prevents any
                    // installed CLI, proxy or config from being probed.
                    app.cleanup_path = Some(String::new());
                    app.open_review(cx);
                    app.removal_mode =
                        disktree_core::removal::RemovalMode::OwnerCleanup;
                    app.prepare_cleanup(cx);
                }
                State::DiskCapacity => {
                    ensure!(
                        root == Path::new("/Volumes/Fixtures/Demo"),
                        "capacity pictures require the contrived Fixtures image"
                    );
                    // Serialized optional counters let the baseline harness
                    // supply identical inputs to old and new presentations.
                    app.space =
                        Some(serde_json::from_value(serde_json::json!({
                            "total": 107_374_182_400_u64,
                            "free": 5_368_709_120_u64,
                            "available": 5_368_709_120_u64,
                            "native": {"used": 85_899_345_920_u64,
                                "available": 16_106_127_360_u64}
                        }))?);
                    app.device = Some("/dev/fixture".into());
                }
                State::SystemFolders => {
                    ensure!(
                        root == Path::new("/Volumes/Fixtures/Demo"),
                        "system folders require the contrived Fixtures image"
                    );
                    // Keep the scanned fixture tree and its own disk meter.
                    // Only its logical paths change, exercising real guards
                    // without scanning or changing any system directory.
                    app.root_path = Path::new("/").to_path_buf();
                }
                State::RemovalCheck | State::RemovalRefused => {
                    ensure!(
                        root == Path::new("/Volumes/Fixtures/Demo"),
                        "removal pictures require the contrived Fixtures image"
                    );
                    let plan = app.plan();
                    ensure!(!plan.is_empty(), "mark fixture paths first");
                    app.removal_mode =
                        disktree_core::removal::RemovalMode::Permanent;
                    app.run_list = crate::run_list::RunList::new(
                        plan.targets
                            .iter()
                            .map(|target| crate::run_list::Item {
                                target: target.clone(),
                                category: Category::Code,
                                reclaim: None,
                                private: Some(target.bytes),
                                status: crate::run_list::Status::Waiting,
                            })
                            .collect(),
                    );
                    app.run_free_before =
                        app.space.map(|space| space.available);
                    app.screen = crate::state::Screen::Running;
                    if matches!(state, State::RemovalRefused) {
                        // Render a refusal, never spawn a removal worker or
                        // approve a confirmation. Every fixture file stays.
                        let reason =
                            disktree_core::removal_limit::Limit::new(1)
                                .expect("fixture limit")
                                .check(&plan.targets)
                                .expect_err("fixture selection exceeds 1%");
                        for target in &plan.targets {
                            app.run_list
                                .settle(&target.path, &Err(reason.clone()));
                        }
                        let before = app.run_free_before;
                        let after =
                            before.map(|n| n.saturating_sub(200 * 1024));
                        if let (Some(space), Some(after)) =
                            (&mut app.space, after)
                        {
                            space.available = after;
                        }
                        app.ledger.begin(app.scans_done);
                        app.ledger.push(crate::ledger::Cohort {
                            mode: app.removal_mode,
                            removed: 0,
                            failed: plan.targets.len(),
                            reported: 0,
                            private: Some(0),
                            before,
                            after,
                            started: std::time::SystemTime::UNIX_EPOCH,
                            finished: std::time::SystemTime::UNIX_EPOCH,
                        });
                        app.screen = crate::state::Screen::Done;
                    }
                }
                State::FileList => app.file_list = true,
                State::Help => app.open_help(cx),
            }
            cx.notify();
            Ok(())
        })
    })
}

fn draw_pointer(image: &mut image::RgbaImage, at: [f32; 2], width: f32) {
    // Captures have no native cursor. This deliberately drawn arrow must be
    // disclosed in the PR body, rather than passed off as an OS screenshot.
    const ARROW: [(f32, f32); 7] = [
        (0.0, 0.0),
        (0.0, 17.0),
        (4.2, 13.0),
        (7.2, 19.5),
        (9.8, 18.4),
        (6.9, 12.0),
        (12.5, 12.0),
    ];
    let scale = image.width() as f32 / width;
    for y in 0..(26.0 * scale) as u32 {
        for x in 0..(18.0 * scale) as u32 {
            let local = [x as f32 / scale - 1.0, y as f32 / scale - 1.0];
            let ix = (at[0] * scale) as u32 + x;
            let iy = (at[1] * scale) as u32 + y;
            if ix >= image.width() || iy >= image.height() {
                continue;
            }
            let color = if inside(&ARROW, local, 1.0) {
                Some([0, 0, 0, 255])
            } else if inside(&ARROW, local, 1.12) {
                Some([255, 255, 255, 255])
            } else {
                None
            };
            if let Some(color) = color {
                image.get_pixel_mut(ix, iy).0 = color;
            }
        }
    }
}

fn inside(polygon: &[(f32, f32)], [x, y]: [f32; 2], grow: f32) -> bool {
    let mut inside = false;
    for (i, &(xi, yi)) in polygon.iter().enumerate() {
        let (xj, yj) = polygon[(i + polygon.len() - 1) % polygon.len()];
        let (xi, yi, xj, yj) = (xi * grow, yi * grow, xj * grow, yj * grow);
        if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
            inside = !inside;
        }
    }
    inside
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn pointer_clips_at_the_image_edge() {
        let mut image = image::RgbaImage::from_pixel(
            100,
            100,
            image::Rgba([128, 128, 128, 255]),
        );
        draw_pointer(&mut image, [95.0, 95.0], 100.0);
        assert_eq!(image.get_pixel(50, 50).0, [128, 128, 128, 255]);
        assert!(image.pixels().any(|p| p.0 == [0, 0, 0, 255]));
    }

    proptest! {
        #![proptest_config(ProptestConfig {
            failure_persistence: None,
            ..ProptestConfig::default()
        })]

        // A rectangle has an independent interval oracle for the pointer's
        // polygon rasterizer, including the scale used for its outline.
        #[test]
        fn polygon_fill_agrees_with_rectangle_intervals(
            x in -5.0_f32..20.0, y in -5.0_f32..20.0, scale in 0.5_f32..1.5
        ) {
            let rectangle = [(0.0, 0.0), (10.0, 0.0),
                             (10.0, 10.0), (0.0, 10.0)];
            prop_assert_eq!(inside(&rectangle, [x, y], scale),
                (0.0..10.0 * scale).contains(&x)
                    && (0.0..10.0 * scale).contains(&y));
        }
    }
}
