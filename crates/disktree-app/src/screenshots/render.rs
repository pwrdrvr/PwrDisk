//! Real Metal pixels and macOS text in GPUI `TestWindows`, on the main thread.

use std::{path::Path, sync::Arc, time::Duration};

use anyhow::{Context as _, Result, ensure};
use disktree_core::{classify::Category, scan::ScanOptions};
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Entity, HeadlessAppContext, MouseButton,
    MouseDownEvent, MouseUpEvent, PlatformInput, point, px, size,
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
    let window = handle.into();
    draw(&mut cx, window)?;
    let mut pointer = None;
    for step in &scenario.steps {
        match step {
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
                State::Legend { kind } => {
                    let index = Category::LEGEND
                        .iter()
                        .position(|k| k.name() == kind)
                        .context("unknown legend kind")?;
                    app.cycle_legend(ChipId::Kind(index), cx);
                }
                State::Select { path } | State::Mark { path } => {
                    let crumbs = app
                        .crumbs_for_path(&root.join(path))
                        .with_context(|| format!("no fixture path: {path}"))?;
                    if matches!(state, State::Mark { .. }) {
                        app.toggle_mark(&crumbs, cx);
                        ensure!(
                            app.marks
                                .contains(&root.join(path), app.path_style()),
                            "fixture mark was refused: {path}"
                        );
                    } else {
                        app.select(Some(crumbs), cx);
                    }
                }
                State::Review => app.open_review(cx),
                State::CleanupReview => {
                    // Proposals only: an empty search path prevents any
                    // installed CLI, proxy or config from being probed.
                    app.cleanup_path = Some(String::new());
                    app.open_review(cx);
                    app.removal_mode =
                        disktree_core::removal::RemovalMode::OwnerCleanup;
                    app.prepare_cleanup(cx);
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
