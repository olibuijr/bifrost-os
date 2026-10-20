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
    io,
    os::{fd::FromRawFd, unix::net::UnixListener},
    path::PathBuf,
    sync::Arc,
};

use smithay::reexports::{
    calloop::{generic::Generic, Interest, LoopHandle, Mode, PostAction},
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

/// Opens the listener for `endpoint`.
pub fn listener(endpoint: &Endpoint) -> io::Result<UnixListener> {
    let listener = match endpoint {
        Endpoint::Fd(fd) => {
            // SAFETY: the session helper put a listening Unix socket on this descriptor for this process and nothing else
            // in the process owns it; the window server takes it over here, once.
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

/// Accepts the clients of `listener` on `handle` as privileged clients.
pub fn register(handle: &LoopHandle<'static, WmState>, listener: UnixListener) -> Result<(), String> {
    handle
        .insert_source(Generic::new(listener, Interest::READ, Mode::Level), |_, listener, state: &mut WmState| {
            loop {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let _ = stream.set_nonblocking(true);
                        let data = Arc::new(ClientState { peer_uid: crate::state::peer_uid(&stream), privileged: true, ..ClientState::default() });
                        if let Err(e) = state.dh.insert_client(stream, data) {
                            eprintln!("bifrost-wm: privileged client refused: {e}");
                        }
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                    Err(e) => {
                        eprintln!("bifrost-wm: privileged socket: {e}");
                        break;
                    }
                }
            }
            Ok(PostAction::Continue)
        })
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Starts listening on the privileged socket the environment names, if it names one.
pub fn start(handle: &LoopHandle<'static, WmState>) {
    let Some(endpoint) = endpoint_from_env() else { return };
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
    fn the_socket_file_is_for_its_owner_only() {
        assert_eq!(SOCKET_MODE & 0o077, 0, "no group or other access");
    }
}
