//! The shell's way into the window server's privileged socket (docs/window-server.md, "Privileged socket and capture
//! globals").
//!
//! The window server shows its capture globals (screen capture, window capture) to clients that connected through a socket
//! only root can open. The shell runs as root and names that socket in `BIFROST_WM_PRIVILEGED`; its Wayland connection and
//! the screen recorder's use it. The helpers the shell starts as the session's account (the screenshot tool, `grim`) cannot
//! open it, so the shell connects for them and passes the connected descriptor down in `WAYLAND_SOCKET`, which the
//! Wayland client library takes in place of `WAYLAND_DISPLAY`. Where the variable is unset or the socket cannot be opened
//! (a development session, a window server without one) everything falls back to the ordinary socket, on which the capture
//! globals are not listed: the callers then see no capture and keep their fallbacks.

use std::{
    io,
    os::{
        fd::{AsRawFd, OwnedFd},
        unix::{net::UnixStream, process::CommandExt},
    },
    path::PathBuf,
    process::Command,
};

/// The environment variable that names the privileged socket.
pub const SOCKET_ENV: &str = "BIFROST_WM_PRIVILEGED";

/// The privileged socket the environment names: an absolute path, else none.
pub fn socket_path_from(value: Option<&str>) -> Option<PathBuf> {
    value.map(str::trim).filter(|v| v.starts_with('/')).map(PathBuf::from)
}

/// The privileged socket of this process.
pub fn socket_path() -> Option<PathBuf> {
    socket_path_from(std::env::var(SOCKET_ENV).ok().as_deref())
}

/// A connection to the privileged socket.
pub fn connect_stream() -> io::Result<UnixStream> {
    let path = socket_path().ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("{SOCKET_ENV} is not set")))?;
    UnixStream::connect(path)
}

/// The shell's Wayland connection: through the privileged socket when there is one, else the ordinary socket
/// (`WAYLAND_DISPLAY`), where the capture globals are not listed.
#[cfg(feature = "desktop-wayland")]
pub fn connect() -> Result<wayland_client::Connection, wayland_client::ConnectError> {
    if let Ok(stream) = connect_stream() {
        if let Ok(conn) = wayland_client::Connection::from_socket(stream) {
            return Ok(conn);
        }
    }
    wayland_client::Connection::connect_to_env()
}

/// A connected privileged descriptor for a helper, or `None` without a privileged socket.
pub fn helper_fd() -> Option<OwnedFd> {
    connect_stream().ok().map(OwnedFd::from)
}

/// Makes `command` start with `fd` open as its Wayland connection: the descriptor survives the exec and
/// `WAYLAND_SOCKET` names it. The caller keeps `fd` until the command has been spawned.
pub fn give_to(command: &mut Command, fd: &OwnedFd) {
    let raw = fd.as_raw_fd();
    command.env("WAYLAND_SOCKET", raw.to_string());
    // SAFETY: the hook only calls fcntl on a descriptor that is open in the parent and so in the forked child; fcntl is
    // async-signal-safe and the hook allocates nothing.
    unsafe {
        command.pre_exec(move || {
            const F_SETFD: i32 = 2;
            if super::sys::fcntl(raw, F_SETFD, 0) < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_an_absolute_path_names_the_socket() {
        assert_eq!(socket_path_from(Some("/run/bifrost/wm-privileged")), Some(PathBuf::from("/run/bifrost/wm-privileged")));
        assert_eq!(socket_path_from(Some(" /run/x\n")), Some(PathBuf::from("/run/x")));
        assert_eq!(socket_path_from(Some("wayland-0")), None, "a bare name is the ordinary socket's kind of name");
        assert_eq!(socket_path_from(Some("")), None);
        assert_eq!(socket_path_from(None), None);
    }

    #[test]
    fn a_helper_gets_the_connected_descriptor_in_wayland_socket() {
        let (a, _b) = UnixStream::pair().unwrap();
        let fd = OwnedFd::from(a);
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg("test -n \"$WAYLAND_SOCKET\" && test -e \"/proc/self/fd/$WAYLAND_SOCKET\"");
        give_to(&mut cmd, &fd);
        assert!(cmd.status().unwrap().success(), "the child sees the descriptor WAYLAND_SOCKET names");
    }

    #[test]
    fn without_the_environment_there_is_no_privileged_connection() {
        if std::env::var_os(SOCKET_ENV).is_none() {
            assert!(helper_fd().is_none());
            assert!(connect_stream().is_err());
        }
    }
}
