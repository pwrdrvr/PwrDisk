//! The desktop hosts MCP; the CLI relays the entire stream without
//! deserializing MCP messages. Only this small transport preamble is shared.

use std::io;
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use disktree_link::mcp_stream::{self, Listener, Stream};
use rmcp::ServiceExt as _;
use serde::{Deserialize, Serialize};
use tokio::io::{
    AsyncBufRead, AsyncBufReadExt as _, AsyncRead, AsyncReadExt as _,
    AsyncWrite, AsyncWriteExt as _, BufReader,
};
use tokio::sync::oneshot;

use crate::{Session, server::Server};

/// This is a stream contract, independent of application releases and of
/// the MCP versions negotiated by the desktop and host.
pub(crate) const TRANSPORT_VERSION: u32 = 1;
const HANDSHAKE_WAIT: Duration = Duration::from_secs(5);
const PREAMBLE_LIMIT: u64 = 4096;
pub const RECONNECT: &str = "Start PwrDisk yourself, then restart/reconnect \
this MCP connection in its client. MCP never launches the desktop. \
Do not retry an uncertain removal; obtain a new preview and confirmation.";

#[derive(Debug, Serialize, Deserialize)]
struct Hello {
    transport_version: u32,
    relay_version: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct Ready {
    transport_version: u32,
    desktop_version: String,
}

async fn read_preamble<T: serde::de::DeserializeOwned>(
    reader: &mut (impl AsyncBufRead + Unpin + Send),
) -> io::Result<T> {
    let mut line = String::new();
    let count = reader.take(PREAMBLE_LIMIT).read_line(&mut line).await?;
    if count == 0 || !line.ends_with('\n') {
        return Err(io::Error::other("incomplete MCP transport preamble"));
    }
    serde_json::from_str(&line).map_err(io::Error::other)
}

async fn write_preamble(
    writer: &mut (impl AsyncWrite + Unpin + Send),
    value: &(impl Serialize + Sync),
) -> io::Result<()> {
    let mut bytes = serde_json::to_vec(value).map_err(io::Error::other)?;
    bytes.push(b'\n');
    writer.write_all(&bytes).await?;
    writer.flush().await
}

fn check_transport(version: u32) -> io::Result<()> {
    if version == TRANSPORT_VERSION {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "Incompatible MCP stream protocols: local {TRANSPORT_VERSION}, \
             peer {version}. Update the older installation. {RECONNECT}"
        )))
    }
}

/// Relay an already connected stream. Keeping MCP opaque preserves future
/// tools, schemas, annotations, _meta and bidirectional elicitation intact.
pub async fn relay(
    stream: Stream,
    input: impl AsyncRead + Unpin + Send,
    mut output: impl AsyncWrite + Unpin + Send,
    relay_version: &str,
) -> io::Result<()> {
    let (reader, mut writer) = tokio::io::split(stream);
    let mut reader = BufReader::new(reader);
    let ready: Ready = tokio::time::timeout(HANDSHAKE_WAIT, async {
        write_preamble(
            &mut writer,
            &Hello {
                transport_version: TRANSPORT_VERSION,
                relay_version: relay_version.into(),
            },
        )
        .await?;
        read_preamble(&mut reader).await
    })
    .await??;
    check_transport(ready.transport_version)?;
    let mut input = input;
    let down = async {
        tokio::io::copy(&mut reader, &mut output).await?;
        output.flush().await
    };
    let up = async {
        tokio::io::copy(&mut input, &mut writer).await?;
        writer.shutdown().await
    };
    tokio::pin!(down);
    tokio::select! {
        result = &mut down => result,
        result = up => {
            result?;
            // A pipe cannot half-close. Bound draining after client EOF,
            // but never replay calls or reconnect a live MCP session.
            match tokio::time::timeout(Duration::from_secs(2), down).await {
                Ok(result) => result,
                Err(_) => Ok(()),
            }
        }
    }
}

pub async fn proxy() -> io::Result<()> {
    let path = mcp_stream::endpoint(&disktree_link::socket_path());
    let stream = mcp_stream::connect(&path).await.map_err(|error| {
        io::Error::new(error.kind(), format!(
            "Cannot connect to the running PwrDisk desktop: {error}. {RECONNECT}"
        ))
    })?;
    relay(
        stream,
        tokio::io::stdin(),
        tokio::io::stdout(),
        env!("CARGO_PKG_VERSION"),
    )
    .await
}

/// Serve exactly this desktop's UI backend. Its definitions and skill text
/// are compiled into the desktop, never loaded from the relay executable.
pub async fn serve_connection(
    stream: Stream,
    command_path: PathBuf,
) -> anyhow::Result<()> {
    let (reader, mut writer) = tokio::io::split(stream);
    let mut reader = BufReader::new(reader);
    let hello: Hello =
        tokio::time::timeout(HANDSHAKE_WAIT, read_preamble(&mut reader))
            .await??;
    write_preamble(
        &mut writer,
        &Ready {
            transport_version: TRANSPORT_VERSION,
            desktop_version: env!("CARGO_PKG_VERSION").into(),
        },
    )
    .await?;
    check_transport(hello.transport_version)?;
    let server =
        Server::desktop(Session::desktop(command_path), hello.relay_version);
    let running = server.serve((reader, writer)).await?;
    running.waiting().await?;
    Ok(())
}

/// Kept alive by the window. Its runtime is separate from GPUI, so MCP
/// confirmation can await the client while the UI keeps processing events.
#[derive(Debug)]
pub struct DesktopHost {
    stop: Option<oneshot::Sender<()>>,
    worker: Option<thread::JoinHandle<()>>,
}

impl DesktopHost {
    pub fn start(command_path: PathBuf) -> io::Result<Self> {
        let (stop, mut stopped) = oneshot::channel();
        let (ready, started) = mpsc::sync_channel(1);
        let worker = thread::Builder::new().name("pwrdisk-mcp".into())
            .spawn(move || {
                let setup = (|| {
                    let runtime = tokio::runtime::Builder::new_multi_thread()
                        .worker_threads(2).enable_all().build()?;
                    let listener = {
                        let _entered = runtime.enter();
                        Listener::bind(&mcp_stream::endpoint(&command_path))?
                    };
                    Ok::<_, io::Error>((runtime, listener))
                })();
                match setup {
                    Err(error) => { let _ = ready.send(Err(error)); }
                    Ok((runtime, mut listener)) => {
                        if ready.send(Ok(())).is_ok() {
                            runtime.block_on(async {
                                let mut sessions = tokio::task::JoinSet::new();
                                loop {
                                    tokio::select! {
                                        _ = &mut stopped => break,
                                        Some(_) = sessions.join_next() => {},
                                        stream = listener.accept() => {
                                            let Ok(stream) = stream else {
                                                tokio::time::sleep(Duration::from_millis(50)).await;
                                                continue;
                                            };
                                            // Bound idle clients as well as active requests.
                                            if sessions.len() < 16 {
                                                let path = command_path.clone();
                                                sessions.spawn(async move {
                                                    let _ = serve_connection(stream, path).await;
                                                });
                                            }
                                        }
                                    }
                                }
                            });
                        }
                        // A UI teardown must not wait on a blocking backend
                        // request whose window has just stopped answering.
                        runtime.shutdown_background();
                    }
                }
            })?;
        started.recv().map_err(io::Error::other)??;
        Ok(Self {
            stop: Some(stop),
            worker: Some(worker),
        })
    }
}

impl Drop for DesktopHost {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
