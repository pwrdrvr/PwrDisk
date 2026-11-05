//! Opt-in, offscreen PR pictures; never enter normal application startup.

#[cfg(all(feature = "screenshots", target_os = "macos"))]
mod render;
#[cfg(any(test, target_os = "macos"))]
mod scenario;

#[cfg(feature = "screenshots")]
pub fn run() -> anyhow::Result<()> {
    #[cfg(target_os = "macos")]
    return render::run();
    #[cfg(not(target_os = "macos"))]
    anyhow::bail!("offscreen screenshots currently require macOS");
}
