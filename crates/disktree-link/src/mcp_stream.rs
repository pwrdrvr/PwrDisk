//! A separate, full-duplex MCP stream. The typed UI command channel stays
//! private to the desktop's backend; the relay never interprets MCP.

use std::io;
use std::path::{Path, PathBuf};

use tokio::io::{AsyncRead, AsyncWrite};

pub trait StreamIo: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> StreamIo for T {}
pub type Stream = Box<dyn StreamIo>;

/// Keep the endpoints distinct so old typed clients fail without being
/// mistaken for MCP clients. Respect `PWRDISK_LINK` for isolated checkouts.
pub fn endpoint(command_path: &Path) -> PathBuf {
    let mut path = command_path.as_os_str().to_owned();
    path.push(".mcp");
    path.into()
}

#[derive(Debug)]
pub struct Listener {
    #[cfg(unix)]
    listener: tokio::net::UnixListener,
    #[cfg(unix)]
    path: PathBuf,
    #[cfg(windows)]
    listener: crate::windows::McpListener,
}

impl Listener {
    pub fn bind(path: &Path) -> io::Result<Self> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let listener = match tokio::net::UnixListener::bind(path) {
                Ok(listener) => listener,
                Err(error) if error.kind() == io::ErrorKind::AddrInUse => {
                    // A live desktop owns the name, including while busy.
                    // Only a stale socket may be replaced.
                    match std::os::unix::net::UnixStream::connect(path) {
                        Err(error)
                            if error.kind()
                                == io::ErrorKind::ConnectionRefused => {}
                        _ => return Err(error),
                    }
                    std::fs::remove_file(path)?;
                    tokio::net::UnixListener::bind(path)?
                }
                Err(error) => return Err(error),
            };
            std::fs::set_permissions(
                path,
                std::fs::Permissions::from_mode(0o600),
            )?;
            Ok(Self {
                listener,
                path: path.into(),
            })
        }
        #[cfg(windows)]
        {
            Ok(Self {
                listener: crate::windows::McpListener::bind(path)?,
            })
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = path;
            Err(unsupported())
        }
    }

    pub async fn accept(&mut self) -> io::Result<Stream> {
        #[cfg(unix)]
        {
            let (stream, _) = self.listener.accept().await?;
            check_peer(&stream)?;
            Ok(Box::new(stream))
        }
        #[cfg(windows)]
        {
            self.listener.accept().await
        }
        #[cfg(not(any(unix, windows)))]
        {
            Err(unsupported())
        }
    }
}

#[cfg(unix)]
impl Drop for Listener {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

pub async fn connect(path: &Path) -> io::Result<Stream> {
    #[cfg(unix)]
    {
        let stream = tokio::net::UnixStream::connect(path).await?;
        check_peer(&stream)?;
        Ok(Box::new(stream))
    }
    #[cfg(windows)]
    {
        crate::windows::mcp_connect(path).await
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        Err(unsupported())
    }
}

#[cfg(unix)]
fn check_peer(stream: &tokio::net::UnixStream) -> io::Result<()> {
    if stream.peer_cred()?.uid() == rustix::process::geteuid().as_raw() {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "the PwrDisk MCP peer belongs to another user",
        ))
    }
}

#[cfg(not(any(unix, windows)))]
fn unsupported() -> io::Error {
    io::Error::new(io::ErrorKind::Unsupported, "no local MCP transport")
}

#[cfg(all(test, any(unix, windows)))]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    #[tokio::test]
    async fn protected_stream_is_duplex_and_a_live_endpoint_is_not_stolen() {
        let temp = tempfile::tempdir().expect("fixture");
        #[cfg(unix)]
        let command_path = temp.path().join("link.sock");
        #[cfg(windows)]
        let command_path = PathBuf::from(format!(
            r"\\.\pipe\pwrdisk-stream-{}",
            temp.path()
                .file_name()
                .expect("fixture name")
                .to_string_lossy()
        ));
        let path = endpoint(&command_path);
        let mut listener = Listener::bind(&path).expect("listen");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        let other_path = path.clone();
        let peer =
            tokio::spawn(async move { connect(&path).await.expect("client") });
        let mut server = listener.accept().await.expect("accept");
        let mut client = peer.await.expect("connect");
        // Probe after the real client: macOS refuses peer-credential lookup
        // on an already closed probe, as it should.
        assert!(Listener::bind(&other_path).is_err(), "live ownership");
        client.write_all(b"request\n").await.expect("write");
        let mut request = [0; 8];
        server.read_exact(&mut request).await.expect("read");
        assert_eq!(&request, b"request\n");
        server.write_all(b"notification\n").await.expect("write");
        let mut notice = [0; 13];
        client.read_exact(&mut notice).await.expect("read");
        assert_eq!(&notice, b"notification\n");
        drop(server);
        drop(client);
        drop(listener);
        let _replacement =
            Listener::bind(&endpoint(&command_path)).expect("rebind");
    }
}
