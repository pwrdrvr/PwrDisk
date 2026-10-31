//! Same-user, local-only Windows named pipes. Overlapped I/O keeps reads,
//! writes and listener shutdown cancellable even when a peer stops reading.

#![allow(
    unsafe_code,
    reason = "Win32 user identity and pipe ACLs have no std wrapper; blocks document safety"
)]

use std::collections::VecDeque;
use std::ffi::c_void;
use std::io;
use std::os::windows::ffi::OsStrExt as _;
use std::os::windows::io::{AsRawHandle, FromRawHandle as _, OwnedHandle};
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Duration;

use tokio::io::{
    AsyncBufReadExt as _, AsyncRead, AsyncReadExt as _, AsyncWriteExt as _,
    BufReader,
};
use tokio::net::windows::named_pipe::{
    ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions,
};
use tokio::runtime::{Builder, Runtime};
use tokio::sync::oneshot;
use tokio::time::{sleep, timeout};
use windows_sys::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_PIPE_BUSY, HANDLE, LocalFree,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW,
    ConvertStringSecurityDescriptorToSecurityDescriptorW, GetSecurityInfo,
    SDDL_REVISION_1, SE_KERNEL_OBJECT,
};
use windows_sys::Win32::Security::{
    GetTokenInformation, OWNER_SECURITY_INFORMATION, SECURITY_ATTRIBUTES,
    TOKEN_QUERY, TOKEN_USER, TokenUser,
};
use windows_sys::Win32::System::IO::CancelIoEx;
use windows_sys::Win32::System::Pipes::DisconnectNamedPipe;
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, OpenProcessToken,
};

use crate::{MAX_LINE, Request, Response, decode, encode, socket_path};

/// Matches the Unix link's deadlines. A stalled window must not leave an
/// MCP tool waiting forever, and a stalled client must not pin a writer.
const READ_WAIT: Duration = Duration::from_secs(20);
const WRITE_WAIT: Duration = Duration::from_secs(5);
const CONNECT_TICK: Duration = Duration::from_millis(10);

/// Something a connection sent, or the news that it is gone.
#[derive(Debug)]
pub enum Incoming {
    Request {
        connection: u64,
        request: Request,
        reply: Reply,
    },
    Closed {
        connection: u64,
    },
}

/// Dropping an unanswered reply closes the connection.
#[derive(Debug)]
pub struct Reply(oneshot::Sender<Response>);

impl Reply {
    pub fn send(self, response: Response) {
        let _ = self.0.send(response);
    }
}

type Queue = Arc<Mutex<VecDeque<Incoming>>>;

/// The window's end. Its I/O runtime lives on a dedicated thread so this
/// synchronous API also works inside the MCP server's Tokio runtime.
#[derive(Debug)]
pub struct Listener {
    path: PathBuf,
    queue: Queue,
    connections: Arc<AtomicUsize>,
    stop: Option<oneshot::Sender<()>>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Listener {
    pub fn bind() -> io::Result<Self> {
        Self::bind_at(&socket_path())
    }

    /// Claim a local pipe name without taking it from another window.
    pub fn bind_at(path: &Path) -> io::Result<Self> {
        validate_path(path)?;
        let path = path.to_path_buf();
        let queue = Arc::new(Mutex::new(VecDeque::new()));
        let connections = Arc::new(AtomicUsize::new(0));
        let (stop, stopped) = oneshot::channel();
        let (ready, started) = mpsc::sync_channel(1);
        let (p, q, c) =
            (path.clone(), Arc::clone(&queue), Arc::clone(&connections));
        let worker = thread::Builder::new()
            .name("disktree-link".into())
            .spawn(move || {
                let setup = (|| {
                    let runtime = runtime()?;
                    let security = Security::current_user()?;
                    let server = {
                        let _entered = runtime.enter();
                        security.create(&p, true)?
                    };
                    Ok::<_, io::Error>((runtime, security, server))
                })();
                match setup {
                    Ok((runtime, security, server)) => {
                        if ready.send(Ok(())).is_ok() {
                            runtime.block_on(accept(
                                p, q, c, security, server, stopped,
                            ));
                        }
                    }
                    Err(error) => {
                        let _ = ready.send(Err(error));
                    }
                }
            })?;
        started.recv().map_err(|_| gone())??;
        Ok(Self {
            path,
            queue,
            connections,
            stop: Some(stop),
            worker: Some(worker),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn connections(&self) -> usize {
        self.connections.load(Ordering::Relaxed)
    }

    pub fn drain(&self) -> Vec<Incoming> {
        self.queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .drain(..)
            .collect()
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        // Joining relinquishes every instance before a replacement binds.
        // Runtime shutdown cancels idle reads and pending replies as well.
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[allow(
    clippy::future_not_send,
    reason = "the ACL allocation stays on this dedicated current-thread runtime"
)]
async fn accept(
    path: PathBuf,
    queue: Queue,
    connections: Arc<AtomicUsize>,
    security: Security,
    mut server: NamedPipeServer,
    mut stop: oneshot::Receiver<()>,
) {
    let mut id = 0;
    loop {
        tokio::select! {
            _ = &mut stop => break,
            result = server.connect() => if result.is_err() { break; },
        }
        // Keep an instance alive while making the next one: there must
        // never be a gap in which another listener can claim this name.
        let Ok(next) = security.create(&path, false) else {
            break;
        };
        let connected = std::mem::replace(&mut server, next);
        id += 1;
        connections.fetch_add(1, Ordering::Relaxed);
        let (q, c) = (Arc::clone(&queue), Arc::clone(&connections));
        tokio::spawn(async move {
            let _ = exchange(connected, &q, id).await;
            c.fetch_sub(1, Ordering::Relaxed);
            push(&q, Incoming::Closed { connection: id });
        });
    }
}

async fn exchange(
    server: NamedPipeServer,
    queue: &Queue,
    id: u64,
) -> io::Result<()> {
    let mut owned = PipeIo {
        pipe: server,
        server: true,
    };
    let mut pipe = BufReader::new(&mut owned.pipe);
    loop {
        let Some(line) = read_line(&mut pipe).await? else {
            return Ok(());
        };
        let response = match decode::<Request>(&line) {
            Ok(request) => {
                let (sender, receiver) = oneshot::channel();
                push(
                    queue,
                    Incoming::Request {
                        connection: id,
                        request,
                        reply: Reply(sender),
                    },
                );
                receiver.await.map_err(|_| gone())?
            }
            Err(error) => Response::error(format!("bad request: {error}")),
        };
        timeout(
            WRITE_WAIT,
            pipe.get_mut().write_all(encode(&response)?.as_bytes()),
        )
        .await??;
    }
}

fn push(queue: &Queue, incoming: Incoming) {
    queue
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push_back(incoming);
}

/// A bounded line on both sides: partial JSON and unterminated oversized
/// input must never be accepted as a complete request or response.
async fn read_line<R: AsyncRead + Unpin>(
    reader: &mut BufReader<R>,
) -> io::Result<Option<String>> {
    let mut line = String::new();
    let count = reader.take(MAX_LINE as u64).read_line(&mut line).await?;
    if count == 0 {
        return Ok(None);
    }
    if !line.ends_with('\n') {
        return Err(io::Error::other("unterminated or oversized link message"));
    }
    Ok(Some(line))
}

#[derive(Debug)]
struct Call {
    request: Request,
    answer: mpsc::SyncSender<io::Result<Response>>,
}

/// The agent's end: a synchronous facade over cancellable pipe I/O.
#[derive(Debug)]
pub struct Link {
    calls: mpsc::Sender<Call>,
}

impl Link {
    pub fn connect() -> io::Result<Self> {
        Self::connect_at(&socket_path())
    }

    pub fn connect_at(path: &Path) -> io::Result<Self> {
        validate_path(path)?;
        let path = path.to_path_buf();
        let (calls, requests) = mpsc::channel::<Call>();
        let (ready, started) = mpsc::sync_channel(1);
        thread::Builder::new()
            .name("disktree-link-client".into())
            .spawn(move || {
                let setup = (|| {
                    let runtime = runtime()?;
                    let pipe = runtime.block_on(connect(&path))?;
                    Ok::<_, io::Error>((
                        runtime,
                        PipeIo {
                            pipe,
                            server: false,
                        },
                    ))
                })();
                match setup {
                    Ok((runtime, mut owned)) => {
                        let mut pipe = BufReader::new(&mut owned.pipe);
                        if ready.send(Ok(())).is_err() {
                            return;
                        }
                        for call in requests {
                            let result = runtime.block_on(async {
                                timeout(
                                    WRITE_WAIT,
                                    pipe.get_mut().write_all(
                                        encode(&call.request)?.as_bytes(),
                                    ),
                                )
                                .await??;
                                let line =
                                    timeout(READ_WAIT, read_line(&mut pipe))
                                        .await??
                                        .ok_or_else(gone)?;
                                decode(&line)
                            });
                            // After a timeout the next line might be the old
                            // reply: never reuse a desynchronized connection.
                            let failed = result.is_err();
                            let _ = call.answer.send(result);
                            if failed {
                                break;
                            }
                        }
                    }
                    Err(error) => {
                        let _ = ready.send(Err(error));
                    }
                }
            })?;
        started.recv().map_err(|_| gone())??;
        Ok(Self { calls })
    }

    pub fn call(&mut self, request: &Request) -> io::Result<Response> {
        let (answer, response) = mpsc::sync_channel(1);
        self.calls
            .send(Call {
                request: request.clone(),
                answer,
            })
            .map_err(|_| gone())?;
        response.recv().map_err(|_| gone())?
    }
}

async fn connect(path: &Path) -> io::Result<NamedPipeClient> {
    connect_for_user(path, &current_sid()?).await
}

async fn connect_for_user(
    path: &Path,
    user: &str,
) -> io::Result<NamedPipeClient> {
    timeout(WRITE_WAIT, async {
        loop {
            // Tokio uses SECURITY_IDENTIFICATION: a pipe server cannot
            // impersonate an elevated agent that connects to its name.
            match ClientOptions::new().open(path) {
                Ok(pipe) => {
                    // The name and a server-supplied DACL prove nothing:
                    // another account could have claimed this name first.
                    // Query this connected handle, never the path, before
                    // sending Hello, marks, or any other protocol data.
                    if pipe_owner(pipe.as_raw_handle())? != user {
                        return Err(io::Error::new(
                            io::ErrorKind::PermissionDenied,
                            "the PwrDisk pipe belongs to another user",
                        ));
                    }
                    return Ok(pipe);
                }
                Err(error)
                    if error.raw_os_error()
                        == Some(ERROR_PIPE_BUSY.cast_signed()) =>
                {
                    sleep(CONNECT_TICK).await;
                }
                Err(error) => return Err(error),
            }
        }
    })
    .await?
}

fn pipe_owner(handle: HANDLE) -> io::Result<String> {
    let mut owner = ptr::null_mut();
    let mut descriptor = ptr::null_mut();
    // SAFETY: Windows validates the handle; both outputs are valid pointers.
    // The client's GENERIC_READ/WRITE rights include READ_CONTROL.
    let status = unsafe {
        GetSecurityInfo(
            handle,
            SE_KERNEL_OBJECT,
            OWNER_SECURITY_INFORMATION,
            &raw mut owner,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            &raw mut descriptor,
        )
    };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status.cast_signed()));
    }
    let _allocation = LocalAllocation(descriptor);
    if owner.is_null() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "the PwrDisk pipe has no verifiable owner",
        ));
    }
    // SAFETY: owner points into the live security descriptor above.
    unsafe { sid_text(owner) }
}

/// Mio deliberately lets pending writes finish after dropping a pipe.
/// Cancel them explicitly: a peer that never reads must not retain this
/// instance after a timeout or prevent a new window from claiming its name.
#[derive(Debug)]
struct PipeIo<T: AsRawHandle> {
    pipe: T,
    server: bool,
}

impl<T: AsRawHandle + AsyncRead + Unpin> tokio::io::AsyncRead for PipeIo<T> {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        std::pin::Pin::new(&mut self.get_mut().pipe).poll_read(cx, buf)
    }
}

impl<T: AsRawHandle + tokio::io::AsyncWrite + Unpin> tokio::io::AsyncWrite
    for PipeIo<T>
{
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<io::Result<usize>> {
        std::pin::Pin::new(&mut self.get_mut().pipe).poll_write(cx, buf)
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        std::pin::Pin::new(&mut self.get_mut().pipe).poll_flush(cx)
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        std::pin::Pin::new(&mut self.get_mut().pipe).poll_shutdown(cx)
    }
}

/// MCP shares the typed link's SID, protected DACL, remote rejection,
/// connected-owner verification and cancellation on drop.
#[derive(Debug)]
pub(crate) struct McpListener {
    path: PathBuf,
    server: NamedPipeServer,
}

impl McpListener {
    pub(crate) fn bind(path: &Path) -> io::Result<Self> {
        validate_path(path)?;
        Ok(Self {
            path: path.into(),
            server: Security::current_user()?.create(path, true)?,
        })
    }

    pub(crate) async fn accept(
        &mut self,
    ) -> io::Result<crate::mcp_stream::Stream> {
        self.server.connect().await?;
        // Keep the existing instance until its successor has claimed the
        // name. A descriptor is allocated only during this synchronous call.
        let next = Security::current_user()?.create(&self.path, false)?;
        let pipe = std::mem::replace(&mut self.server, next);
        Ok(Box::new(PipeIo { pipe, server: true }))
    }
}

pub(crate) async fn mcp_connect(
    path: &Path,
) -> io::Result<crate::mcp_stream::Stream> {
    validate_path(path)?;
    Ok(Box::new(PipeIo {
        pipe: connect(path).await?,
        server: false,
    }))
}

impl<T: AsRawHandle> Drop for PipeIo<T> {
    fn drop(&mut self) {
        // SAFETY: pipe owns a live overlapped handle. Null means every I/O
        // operation on it; Mio retains each OVERLAPPED until completion.
        unsafe {
            CancelIoEx(self.pipe.as_raw_handle(), ptr::null());
        }
        if self.server {
            // SAFETY: only a server pipe sets this flag. Disconnect also
            // invalidates idle client handles before the server closes.
            unsafe {
                DisconnectNamedPipe(self.pipe.as_raw_handle());
            }
        }
    }
}

fn runtime() -> io::Result<Runtime> {
    Builder::new_current_thread().enable_all().build()
}

fn gone() -> io::Error {
    io::Error::new(io::ErrorKind::UnexpectedEof, "the window closed the link")
}

fn validate_path(path: &Path) -> io::Result<()> {
    let name = path.to_string_lossy();
    if name
        .get(..9)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(r"\\.\pipe\"))
        && name.len() > 9
        && !name[9..].contains(['\\', '/'])
        && !name.contains('\0')
    {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "expected a local \\\\.\\pipe\\name",
        ))
    }
}

pub fn default_path() -> io::Result<PathBuf> {
    Ok(PathBuf::from(format!(
        r"\\.\pipe\pwrdisk-{}",
        current_sid()?
    )))
}

/// Memory Windows allocated with `LocalAlloc`; both SID strings and security
/// descriptors must be released with `LocalFree`, including on error paths.
struct LocalAllocation(*mut c_void);

impl Drop for LocalAllocation {
    fn drop(&mut self) {
        // SAFETY: this pointer came from a successful allocating Win32 API.
        unsafe {
            LocalFree(self.0);
        }
    }
}

fn current_sid() -> io::Result<String> {
    let mut token = ptr::null_mut();
    // SAFETY: the process pseudo-handle is valid; token is an out pointer.
    if unsafe {
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &raw mut token)
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: OpenProcessToken returned a unique owned handle.
    let _token = unsafe { OwnedHandle::from_raw_handle(token) };
    let mut bytes = 0;
    // SAFETY: null buffer and zero size query the required length only.
    unsafe {
        GetTokenInformation(
            token,
            TokenUser,
            ptr::null_mut(),
            0,
            &raw mut bytes,
        );
    }
    // TOKEN_USER contains pointers, so allocate aligned storage for it and
    // the SID Windows writes after it, rather than casting a byte buffer.
    let mut buffer =
        vec![0_usize; (bytes as usize).div_ceil(size_of::<usize>())];
    // SAFETY: buffer has the queried size and TOKEN_USER's pointer alignment.
    if unsafe {
        GetTokenInformation(
            token,
            TokenUser,
            buffer.as_mut_ptr().cast(),
            bytes,
            &raw mut bytes,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: a successful TokenUser query filled a TOKEN_USER at this address.
    let user = unsafe { &*buffer.as_ptr().cast::<TOKEN_USER>() };
    // SAFETY: the SID belongs to buffer and remains alive through conversion.
    unsafe { sid_text(user.User.Sid) }
}

/// `sid` must point to a valid SID for the duration of this call.
unsafe fn sid_text(sid: *mut c_void) -> io::Result<String> {
    let mut text = ptr::null_mut();
    // SAFETY: the caller guarantees a valid SID; text is an out pointer.
    if unsafe { ConvertSidToStringSidW(sid, &raw mut text) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let _allocation = LocalAllocation(text.cast());
    let mut len = 0;
    // SAFETY: successful conversion returns a null-terminated UTF-16 string.
    unsafe {
        while *text.add(len) != 0 {
            len += 1;
        }
    }
    // SAFETY: len stops at the terminator inside the allocated SID string.
    Ok(String::from_utf16_lossy(unsafe {
        std::slice::from_raw_parts(text, len)
    }))
}

struct Security(LocalAllocation);

impl Security {
    fn current_user() -> io::Result<Self> {
        // A protected DACL grants only this SID access. No Everyone,
        // authenticated-users or administrators ACE is inherited.
        let sid = current_sid()?;
        // An elevated token's default owner can be Administrators rather
        // than TokenUser. Both sides identify the account by TokenUser.
        Self::from_sddl(&format!("O:{sid}D:P(A;;GA;;;{sid})"))
    }

    fn from_sddl(sddl: &str) -> io::Result<Self> {
        let sddl: Vec<u16> = std::ffi::OsStr::new(sddl)
            .encode_wide()
            .chain(Some(0))
            .collect();
        let mut descriptor = ptr::null_mut();
        // SAFETY: SDDL is terminated and descriptor is an out pointer.
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &raw mut descriptor,
                ptr::null_mut(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(LocalAllocation(descriptor)))
    }

    fn create(&self, path: &Path, first: bool) -> io::Result<NamedPipeServer> {
        let mut attributes = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: self.0.0,
            bInheritHandle: 0,
        };
        // SAFETY: the descriptor and attributes live through CreateNamedPipeW,
        // which copies the descriptor; the handle is not inheritable.
        let result = unsafe {
            ServerOptions::new()
                .first_pipe_instance(first)
                .reject_remote_clients(true)
                .create_with_security_attributes_raw(
                    path,
                    (&raw mut attributes).cast(),
                )
        };
        match result {
            Err(error)
                if first
                    && error.raw_os_error()
                        == Some(ERROR_ACCESS_DENIED.cast_signed()) =>
            {
                Err(io::Error::new(
                    io::ErrorKind::AddrInUse,
                    "another PwrDisk window owns this pipe, or access was denied",
                ))
            }
            result => result,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::Security::{
        ACCESS_ALLOWED_ACE, DACL_SECURITY_INFORMATION, GetAce,
        GetSecurityDescriptorControl, SE_DACL_PROTECTED,
    };

    fn endpoint() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = PathBuf::from(format!(
            r"\\.\pipe\pwrdisk-test-{}",
            dir.path().file_name().expect("temp name").to_string_lossy()
        ));
        (dir, path)
    }

    #[test]
    fn the_default_pipe_is_stable_and_names_the_current_sid() {
        let path = default_path().expect("default pipe");
        assert_eq!(path, default_path().expect("same pipe"));
        assert_eq!(
            path,
            PathBuf::from(format!(
                r"\\.\pipe\pwrdisk-{}",
                current_sid().expect("SID")
            ))
        );
        validate_path(&path).expect("local pipe");
    }

    #[test]
    fn paths_cannot_address_files_or_remote_pipes() {
        for path in [
            r"\\server\pipe\pwrdisk",
            r"C:\pwrdisk.sock",
            r"\\.\pipe\",
            r"\\.\pipe\nested\name",
            "",
        ] {
            assert_eq!(
                Listener::bind_at(Path::new(path))
                    .expect_err("invalid path")
                    .kind(),
                io::ErrorKind::InvalidInput
            );
            assert_eq!(
                Link::connect_at(Path::new(path))
                    .expect_err("invalid path")
                    .kind(),
                io::ErrorKind::InvalidInput
            );
        }
    }

    #[test]
    fn the_kernel_pipe_has_one_protected_allow_ace_for_this_user() {
        let (_dir, path) = endpoint();
        let runtime = runtime().expect("runtime");
        let _entered = runtime.enter();
        let pipe = Security::current_user()
            .expect("security")
            .create(&path, true)
            .expect("pipe");
        assert_eq!(
            pipe_owner(pipe.as_raw_handle()).expect("pipe owner"),
            current_sid().expect("user SID")
        );
        let mut descriptor = ptr::null_mut();
        let mut acl = ptr::null_mut();
        // SAFETY: pipe is live, and the requested outputs are valid pointers.
        let status = unsafe {
            GetSecurityInfo(
                pipe.as_raw_handle(),
                SE_KERNEL_OBJECT,
                DACL_SECURITY_INFORMATION,
                ptr::null_mut(),
                ptr::null_mut(),
                &raw mut acl,
                ptr::null_mut(),
                &raw mut descriptor,
            )
        };
        assert_eq!(status, 0, "GetSecurityInfo");
        let _allocation = LocalAllocation(descriptor);
        assert!(!acl.is_null(), "a null DACL allows everyone");
        let mut control = 0;
        let mut revision = 0;
        // SAFETY: descriptor is the live allocation returned above.
        assert_ne!(
            unsafe {
                GetSecurityDescriptorControl(
                    descriptor,
                    &raw mut control,
                    &raw mut revision,
                )
            },
            0
        );
        assert_ne!(control & SE_DACL_PROTECTED, 0, "no inherited grants");
        // SAFETY: acl points inside descriptor and is not null.
        assert_eq!(unsafe { (*acl).AceCount }, 1, "only one identity");
        let mut ace = ptr::null_mut();
        // SAFETY: the queried ACL has one ACE, and ace is an out pointer.
        assert_ne!(unsafe { GetAce(acl, 0, &raw mut ace) }, 0);
        // SAFETY: the descriptor was constructed with one ACCESS_ALLOWED_ACE.
        let ace = unsafe { &*ace.cast::<ACCESS_ALLOWED_ACE>() };
        assert_eq!(ace.Header.AceType, 0, "ACCESS_ALLOWED_ACE_TYPE");
        // SAFETY: SidStart is the start of the variable-length SID in this ACE.
        let sid =
            unsafe { sid_text((&raw const ace.SidStart).cast_mut().cast()) }
                .expect("ACE SID");
        assert_eq!(sid, current_sid().expect("user SID"));
    }

    #[test]
    fn a_permissive_pipe_owned_by_a_different_sid_is_rejected() {
        let (_dir, path) = endpoint();
        let runtime = runtime().expect("runtime");
        let owner = current_sid().expect("server SID");
        // Model the client as a different account without requiring a
        // second Windows login or privileges on the CI runner. This pipe
        // admits everyone, just like a squatter's pipe, but its real owner
        // must still match the client's expected account.
        let client = "S-1-5-21-0-0-0-1234";
        assert_ne!(owner, client);
        let _server = {
            let _entered = runtime.enter();
            Security::from_sddl(&format!("O:{owner}D:P(A;;GA;;;WD)"))
                .expect("permissive security")
                .create(&path, true)
                .expect("squatter's pipe")
        };
        let error = runtime
            .block_on(connect_for_user(&path, client))
            .expect_err("a matching name and permissive ACL are not identity");
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert!(error.to_string().contains("belongs to another user"));
    }

    #[test]
    fn an_unreadable_pipe_owner_is_not_trusted() {
        assert!(pipe_owner(ptr::null_mut()).is_err());
    }

    #[test]
    fn synchronous_calls_work_inside_a_tokio_runtime() {
        let (_dir, path) = endpoint();
        runtime().expect("runtime").block_on(async {
            let listener = Listener::bind_at(&path).expect("bind");
            let mut link = Link::connect_at(&path).expect("connect");
            let answer = thread::spawn(move || {
                loop {
                    for incoming in listener.drain() {
                        if let Incoming::Request { reply, .. } = incoming {
                            reply.send(Response::Ok);
                            // Keep the listener alive until the response
                            // has left its background runtime.
                            for _ in 0..200 {
                                if listener.connections() == 0 {
                                    return;
                                }
                                thread::sleep(CONNECT_TICK);
                            }
                            panic!("client did not disconnect");
                        }
                    }
                    thread::sleep(CONNECT_TICK);
                }
            });
            assert_eq!(
                link.call(&Request::State).expect("response"),
                Response::Ok
            );
            drop(link);
            answer.join().expect("window thread");
        });
    }

    #[test]
    fn dropping_a_listener_cancels_idle_reads_and_pending_replies() {
        let (_dir, path) = endpoint();
        let listener = Listener::bind_at(&path).expect("bind");
        let _idle = Link::connect_at(&path).expect("idle");
        let mut link = Link::connect_at(&path).expect("pending");
        let request = thread::spawn(move || link.call(&Request::State));
        let pending = (0..200)
            .find_map(|_| {
                let incoming = listener.drain().into_iter().next();
                if incoming.is_none() {
                    thread::sleep(CONNECT_TICK);
                }
                incoming
            })
            .expect("request must reach the listener");
        assert!(matches!(pending, Incoming::Request { .. }));
        drop(listener);
        assert!(request.join().expect("client thread").is_err());
        // Retaining the reply cannot keep a pipe instance alive.
        let _replacement =
            Listener::bind_at(&path).expect("rebind immediately");
        drop(pending);
    }

    #[test]
    fn a_client_that_never_reads_cannot_hold_listener_shutdown() {
        use std::io::Write as _;
        use std::os::windows::fs::OpenOptionsExt as _;
        use windows_sys::Win32::Storage::FileSystem::{
            SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT,
        };

        let (_dir, path) = endpoint();
        let listener = Listener::bind_at(&path).expect("bind");
        let mut client = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(SECURITY_IDENTIFICATION | SECURITY_SQOS_PRESENT)
            .open(&path)
            .expect("raw client");
        client
            .write_all(encode(&Request::State).expect("encode").as_bytes())
            .expect("request");
        let incoming = (0..200)
            .find_map(|_| {
                let incoming = listener.drain().into_iter().next();
                if incoming.is_none() {
                    thread::sleep(CONNECT_TICK);
                }
                incoming
            })
            .expect("request");
        let Incoming::Request { reply, .. } = incoming else {
            panic!("closed first");
        };
        // Much larger than the pipe buffer: this write cannot complete
        // while the client retains its handle without reading.
        reply.send(Response::error("x".repeat(MAX_LINE / 2)));
        thread::sleep(Duration::from_millis(100));
        let start = std::time::Instant::now();
        drop(listener);
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "shutdown waited for a reader"
        );
        drop(client);
        let _replacement =
            Listener::bind_at(&path).expect("rebind after cancelled write");
    }
}
