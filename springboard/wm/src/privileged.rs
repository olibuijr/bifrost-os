//! The privileged socket: who may capture the screen (docs/window-server.md, "Privileged socket and capture globals").
//!
//! `bifrost-wm` runs as the session's account, and so do the apps, so a Unix uid cannot tell the shell from an app.
//! Instead the window server listens on a second socket that only root can open (`/run/bifrost/wm-privileged`, mode
//! 0600, owned by root), and a client that connected through it is privileged (`ClientState::privileged`, set on accept).
//! The capture globals are visible to privileged clients only: `wlr-screencopy`, `ext-image-copy-capture` with its output
//! capture source, and `bifrost_window_capture_v1`. An ordinary client's registry does not list them and a bind is
//! refused. The shell connects through the privileged socket (it runs as root), and the helpers it starts for a
//! screenshot or a recording get an already-connected descriptor in `WAYLAND_SOCKET`.
//!
//! The listening socket is made by root, not by the window server (which runs as the account and could not make a
//! root-owned file): `/usr/lib/bifrost/privileged-socket` binds it, sets the mode and hands the listening descriptor to
//! the window server in `BIFROST_WM_PRIVILEGED_FD`. For development, where the window server is not started by the
//! session, `BIFROST_WM_PRIVILEGED_SOCKET=/absolute/path` makes the window server bind a path itself, mode 0600 (every
//! process of the same user can then open it: development only).

use std::{
    cell::Cell,
    io,
    os::{
        fd::{BorrowedFd, FromRawFd},
        unix::net::UnixListener,
    },
    path::PathBuf,
    rc::Rc,
    sync::Arc,
    time::Duration,
};

use smithay::reexports::{
    calloop::{
        generic::Generic,
        timer::{TimeoutAction, Timer},
        Interest, LoopHandle, Mode, PostAction, RegistrationToken,
    },
    rustix::{
        io::{fcntl_getfd, fcntl_setfd, FdFlags},
        net::{sockopt, AddressFamily},
    },
    wayland_server::Client,
};

use crate::state::{ClientState, WmState};

/// The listening descriptor the session helper passes.
pub const FD_ENV: &str = "BIFROST_WM_PRIVILEGED_FD";
/// A path the window server binds itself (development).
pub const SOCKET_ENV: &str = "BIFROST_WM_PRIVILEGED_SOCKET";

/// Global data of the capture globals: visible to privileged clients only.
#[derive(Clone, Copy, Debug, Default)]
pub struct PrivilegedOnly;

/// Whether `client` connected through the privileged socket. A client of another kind (XWayland's) is not.
pub fn is_privileged(client: &Client) -> bool {
    client.get_data::<ClientState>().is_some_and(|data| data.privileged)
}

/// Where the privileged socket comes from.
#[derive(Debug, PartialEq, Eq)]
pub enum Endpoint {
    /// A listening socket the session helper opened, as a descriptor number.
    Fd(i32),
    /// A path to bind.
    Path(PathBuf),
}

/// The endpoint the environment names. A descriptor wins over a path; a descriptor value that is not a number above 2
/// (stdin, stdout and stderr are not sockets) gives no endpoint at all, and so does a path that is not absolute: a
/// typo leaves the window server without a privileged socket (no one captures), never with an open one.
pub fn endpoint(fd: Option<&str>, path: Option<&str>) -> Option<Endpoint> {
    if let Some(value) = fd.map(str::trim).filter(|v| !v.is_empty()) {
        return value.parse::<i32>().ok().filter(|n| *n > 2).map(Endpoint::Fd);
    }
    path.map(str::trim).filter(|p| p.starts_with('/')).map(|p| Endpoint::Path(PathBuf::from(p)))
}

/// The endpoint of this process.
pub fn endpoint_from_env() -> Option<Endpoint> {
    endpoint(std::env::var(FD_ENV).ok().as_deref(), std::env::var(SOCKET_ENV).ok().as_deref())
}

/// The mode of the socket file the window server binds itself: its owner only.
pub const SOCKET_MODE: u32 = 0o600;

/// Whether `fd` is a listening Unix stream socket: the only thing the session helper may hand over.
fn is_listening_unix_socket(fd: BorrowedFd<'_>) -> bool {
    sockopt::socket_domain(fd).is_ok_and(|domain| domain == AddressFamily::UNIX) && sockopt::socket_acceptconn(fd).unwrap_or(false)
}

/// Opens the listener for `endpoint`.
pub fn listener(endpoint: &Endpoint) -> io::Result<UnixListener> {
    let listener = match endpoint {
        Endpoint::Fd(fd) => {
            // SAFETY: only borrowed for the checks below; the descriptor is not closed or kept past them.
            let borrowed = unsafe { BorrowedFd::borrow_raw(*fd) };
            if !is_listening_unix_socket(borrowed) {
                return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("descriptor {fd} is not a listening Unix socket")));
            }
            // the descriptor is the window server's alone: the children it starts (the X server, the apps' helpers) must
            // not inherit a root-owned listening socket
            let flags = fcntl_getfd(borrowed).map_err(io::Error::from)?;
            fcntl_setfd(borrowed, flags | FdFlags::CLOEXEC).map_err(io::Error::from)?;
            // SAFETY: the session helper put a listening Unix socket on this descriptor for this process (checked
            // above) and nothing else in the process owns it; the window server takes it over here, once.
            unsafe { UnixListener::from_raw_fd(*fd) }
        }
        Endpoint::Path(path) => {
            let _ = std::fs::remove_file(path);
            let listener = UnixListener::bind(path)?;
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(SOCKET_MODE))?;
            listener
        }
    };
    listener.set_nonblocking(true)?;
    Ok(listener)
}

/// How long the accept source rests after an accept error that does not clear by itself (descriptor exhaustion).
pub const ACCEPT_BACKOFF: Duration = Duration::from_millis(500);

/// What the accept loop does with an `accept` error.
#[derive(Debug, PartialEq, Eq)]
pub enum AcceptError {
    /// Nothing more to accept now.
    Drained,
    /// Try the next connection straight away (an interrupted call, a client that gave up while queued).
    Retry,
    /// The error stays while the queue is full (`EMFILE`, `ENFILE`, no memory): rest the source for `ACCEPT_BACKOFF`,
    /// or a level-triggered source fires again at once and spins.
    Pause,
}

/// Classifies an `accept` error.
pub fn classify_accept_error(kind: io::ErrorKind) -> AcceptError {
    match kind {
        io::ErrorKind::WouldBlock => AcceptError::Drained,
        io::ErrorKind::Interrupted | io::ErrorKind::ConnectionAborted => AcceptError::Retry,
        _ => AcceptError::Pause,
    }
}

/// Accepts the clients of `listener` on `handle` as privileged clients.
pub fn register(handle: &LoopHandle<'static, WmState>, listener: UnixListener) -> Result<(), String> {
    // the source's own token, for the timer that wakes it again after a pause
    let own_token: Rc<Cell<Option<RegistrationToken>>> = Rc::new(Cell::new(None));
    let wake_token = own_token.clone();
    let token = handle
        .insert_source(Generic::new(listener, Interest::READ, Mode::Level), move |_, listener, state: &mut WmState| {
            loop {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let _ = stream.set_nonblocking(true);
                        let pid = crate::state::peer_pid(&stream);
                        let uid = crate::state::peer_uid(&stream);
                        let data = Arc::new(ClientState {
                            peer_uid: uid,
                            peer_pid: pid,
                            shares_mount_ns: crate::state::peer_shares_mount_ns(&stream, pid, uid),
                            privileged: true,
                            ..ClientState::default()
                        });
                        if let Err(e) = state.dh.insert_client(stream, data) {
                            eprintln!("bifrost-wm: privileged client refused: {e}");
                        }
                    }
                    Err(e) => match classify_accept_error(e.kind()) {
                        AcceptError::Drained => break,
                        AcceptError::Retry => continue,
                        AcceptError::Pause => {
                            // logged once per pause, so a client flood cannot flood the log either
                            eprintln!("bifrost-wm: privileged socket: {e}; pausing accepts for {} ms", ACCEPT_BACKOFF.as_millis());
                            let token = wake_token.clone();
                            let resumed = state.loop_handle.insert_source(Timer::from_duration(ACCEPT_BACKOFF), move |_, _, state: &mut WmState| {
                                if let Some(token) = token.get() {
                                    let _ = state.loop_handle.enable(&token);
                                }
                                TimeoutAction::Drop
                            });
                            // without the timer the source must keep running rather than stay off for good
                            return Ok(if resumed.is_ok() { PostAction::Disable } else { PostAction::Continue });
                        }
                    },
                }
            }
            Ok(PostAction::Continue)
        })
        .map_err(|e| e.to_string())?;
    own_token.set(Some(token));
    Ok(())
}

/// Starts listening on the privileged socket the environment names, if it names one.
pub fn start(handle: &LoopHandle<'static, WmState>) {
    let Some(endpoint) = endpoint_from_env() else { return };
    if matches!(endpoint, Endpoint::Fd(_)) {
        // the number is meaningless (and misleading) to the children this process starts
        std::env::remove_var(FD_ENV);
    }
    match listener(&endpoint).map_err(|e| e.to_string()).and_then(|l| register(handle, l)) {
        Ok(()) => eprintln!("bifrost-wm: privileged socket {endpoint:?}"),
        Err(e) => eprintln!("bifrost-wm: no privileged socket ({endpoint:?}): {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_descriptor_wins_over_a_path() {
        assert_eq!(endpoint(Some("3"), Some("/run/x")), Some(Endpoint::Fd(3)));
        assert_eq!(endpoint(Some(" 7\n"), None), Some(Endpoint::Fd(7)));
        assert_eq!(endpoint(None, Some("/run/bifrost/wm-privileged")), Some(Endpoint::Path("/run/bifrost/wm-privileged".into())));
    }

    #[test]
    fn an_empty_or_unset_environment_has_no_privileged_socket() {
        assert_eq!(endpoint(None, None), None);
        assert_eq!(endpoint(Some(""), Some("")), None);
        assert_eq!(endpoint(Some("  "), None), None);
    }

    #[test]
    fn a_typo_never_opens_a_socket() {
        assert_eq!(endpoint(Some("three"), Some("/run/x")), None, "a bad descriptor does not fall back to the path");
        assert_eq!(endpoint(Some("2"), None), None, "stderr is not a socket");
        assert_eq!(endpoint(Some("-1"), None), None);
        assert_eq!(endpoint(None, Some("relative/path")), None, "a path must be absolute");
    }

    #[test]
    fn an_adopted_descriptor_is_close_on_exec_and_must_be_a_listening_unix_socket() {
        use std::os::fd::{AsFd, AsRawFd, IntoRawFd};
        let dir = std::env::temp_dir().join(format!("bifrost-wm-priv-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // a listening socket made inheritable, the way the session helper hands it over
        let made = UnixListener::bind(dir.join("p.sock")).unwrap();
        let fd = made.into_raw_fd();
        let borrowed = unsafe { BorrowedFd::borrow_raw(fd) };
        fcntl_setfd(borrowed, FdFlags::empty()).unwrap();
        assert!(is_listening_unix_socket(borrowed));
        let adopted = listener(&Endpoint::Fd(fd)).expect("a listening socket is adopted");
        assert_eq!(adopted.as_raw_fd(), fd);
        assert!(fcntl_getfd(adopted.as_fd()).unwrap().contains(FdFlags::CLOEXEC), "the children must not inherit it");

        // a connected (not listening) socket and a plain file are refused, and stay open for their owner
        let (conn, _peer) = std::os::unix::net::UnixStream::pair().unwrap();
        assert!(!is_listening_unix_socket(conn.as_fd()));
        let file = std::fs::File::open("/proc/self/exe").unwrap();
        assert!(!is_listening_unix_socket(file.as_fd()));
        let err = listener(&Endpoint::Fd(conn.as_raw_fd())).expect_err("a connected socket is not adopted");
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert!(conn.as_fd().as_raw_fd() >= 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn accept_errors_that_persist_pause_the_source_and_the_others_do_not() {
        assert_eq!(classify_accept_error(io::ErrorKind::WouldBlock), AcceptError::Drained);
        assert_eq!(classify_accept_error(io::ErrorKind::Interrupted), AcceptError::Retry);
        assert_eq!(classify_accept_error(io::ErrorKind::ConnectionAborted), AcceptError::Retry);
        for kind in [io::ErrorKind::Other, io::ErrorKind::OutOfMemory, io::ErrorKind::PermissionDenied] {
            assert_eq!(classify_accept_error(kind), AcceptError::Pause, "{kind:?}: a level-triggered source would spin");
        }
        assert!(ACCEPT_BACKOFF >= Duration::from_millis(100));
    }

    #[test]
    fn the_socket_file_is_for_its_owner_only() {
        assert_eq!(SOCKET_MODE & 0o077, 0, "no group or other access");
    }
}
