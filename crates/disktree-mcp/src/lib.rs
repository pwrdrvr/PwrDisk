//! `pwrdisk mcp`: PwrDisk as a Model Context Protocol server over stdio.
//!
//! The normal command relays MCP bytes to an already running desktop over
//! a same-user Unix socket or Windows named pipe. The desktop owns the
//! complete interface and behavior. Nothing here launches the app.
//! `pwrdisk mcp --standalone` explicitly runs an independent scanner with
//! its own definitions and marks, without desktop access or removal.
//!
//! Destructive removal requires a single-use preview and user confirmation.
//! [`Session`] holds what one connected agent knows; [`server`] is the
//! protocol around it.

pub mod desktop;
pub mod report;
pub mod server;
pub mod session;
pub mod skills;

#[cfg(test)]
mod confirmation_tests;

#[cfg(test)]
mod desktop_tests;

pub use desktop::DesktopHost;
pub use session::Session;

/// Serve MCP on stdin and stdout until the client hangs up.
pub fn serve() -> anyhow::Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let result = runtime.block_on(desktop::proxy());
    // stdin uses a blocking reader; do not wait for it after desktop EOF.
    runtime.shutdown_background();
    result.map_err(Into::into)
}

/// Explicit no-window mode. It never connects to or launches a desktop.
pub fn serve_standalone() -> anyhow::Result<()> {
    use rmcp::ServiceExt as _;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let server = server::Server::new(Session::standalone());
        let running = server.serve(rmcp::transport::stdio()).await?;
        running.waiting().await?;
        Ok(())
    })
}
