//! Where an app's control socket lives (JSON-RPC 2.0, `bifrost_app::control`). One rule for both ends: the SDK runtime
//! binds it there and the shell dials it there.
//!
//! The socket is `control.sock` in the app's data directory (`<root>/appdata/<app id>`, `BIFROST_DATA`), unless an
//! explicit path (`BIFROST_CONTROL`) is given. The data directory is the one place both sides see at the same path: the
//! sandbox binds it at its own path, while `/run` inside the sandbox is private, so a socket under the host's
//! `$XDG_RUNTIME_DIR` would be invisible to the shell.

use std::path::{Path, PathBuf};

/// The file name of the control socket inside the app's data directory.
pub const FILE_NAME: &str = "control.sock";

/// The control socket of an app whose data directory is `data_dir`; `explicit` (`BIFROST_CONTROL`) wins when given.
pub fn socket_path(explicit: Option<&Path>, data_dir: &Path) -> PathBuf {
    match explicit {
        Some(path) if !path.as_os_str().is_empty() => path.to_path_buf(),
        _ => data_dir.join(FILE_NAME),
    }
}

/// The data directory of app `app_id` under the shell's data root (`/var/lib/springboard` on a device).
pub fn data_dir(root: &Path, app_id: &str) -> PathBuf {
    root.join("appdata").join(app_id)
}

/// The control socket of app `app_id` as the shell dials it: in the app's data directory under `root`.
pub fn socket_for(root: &Path, app_id: &str) -> PathBuf {
    socket_path(None, &data_dir(root, app_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_socket_is_in_the_data_directory() {
        assert_eq!(
            socket_path(None, Path::new("/var/lib/springboard/appdata/is.x")),
            PathBuf::from("/var/lib/springboard/appdata/is.x/control.sock")
        );
    }

    #[test]
    fn an_explicit_path_wins_and_an_empty_one_does_not() {
        assert_eq!(socket_path(Some(Path::new("/run/custom.sock")), Path::new("/d")), PathBuf::from("/run/custom.sock"));
        assert_eq!(socket_path(Some(Path::new("")), Path::new("/d")), PathBuf::from("/d/control.sock"));
    }

    #[test]
    fn the_shell_dials_where_the_sandboxed_app_binds() {
        let root = Path::new("/var/lib/springboard");
        let id = "is.olibuijr.dictionary.desktop";
        // the app side: BIFROST_DATA is the data directory the launcher binds at its own path
        let app_side = socket_path(None, &data_dir(root, id));
        assert_eq!(socket_for(root, id), app_side);
        assert_eq!(app_side, PathBuf::from("/var/lib/springboard/appdata/is.olibuijr.dictionary.desktop/control.sock"));
    }
}
