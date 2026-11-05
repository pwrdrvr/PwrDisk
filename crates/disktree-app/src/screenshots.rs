//! Opt-in, offscreen PR pictures; never enter normal application startup.

#[cfg(all(feature = "screenshots", target_os = "macos"))]
mod render;
#[cfg(any(test, target_os = "macos"))]
mod scenario;

#[cfg(any(test, target_os = "macos"))]
pub fn init(cx: &mut gpui_kit::App) {
    gpui_kit::base::init(cx);
    // LayoutTransition uses wall time, not the scenario's simulated clock.
    // An explicit override lands navigation before any screenshot frame.
    cx.set_reduce_motion(true);
}

#[cfg(feature = "screenshots")]
pub fn run() -> anyhow::Result<()> {
    #[cfg(target_os = "macos")]
    return render::run();
    #[cfg(not(target_os = "macos"))]
    anyhow::bail!("offscreen screenshots currently require macOS");
}
