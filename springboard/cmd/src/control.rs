//! The shell's control socket, shared by the command-line tools. One request per connection: a single text line, then the
//! shell writes its whole answer and closes the connection. Text answers start with `ok` or `error: <why>`; `shot` answers
//! with a raw `RIMG` frame (see `screencapture`), so the client reads to the end rather than to the first line.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The longest request line the tools send (the limit `bifrost-open-url` keeps).
pub const MAX_LINE: usize = 510;
const TIMEOUT: Duration = Duration::from_secs(5);

/// The shell's control socket: `/run/bifrost/shell-control.sock`, or `SPRINGBOARD_CONTROL_SOCKET` when it is set to an
/// absolute path, as in the shell. The shell serves root and the installed account only.
pub fn socket() -> PathBuf {
    springboard::bare::control::socket_path()
}

/// Checks that a control line can be sent: short enough, and one line.
pub fn check_line(line: &str) -> Result<(), String> {
    if line.len() > MAX_LINE {
        Err("the request is too long".into())
    } else if line.chars().any(|c| c == '\n' || c == '\r') {
        Err("a name with a line break cannot be sent".into())
    } else {
        Ok(())
    }
}

/// Sends one request line to the shell on its control socket and returns every byte of its answer.
pub fn request(line: &str, socket: &Path) -> Result<Vec<u8>, String> {
    request_within(line, socket, TIMEOUT)
}

/// Like `request`, with a longer or shorter wait for the answer (`image-attach` can take a while).
pub fn request_within(line: &str, socket: &Path, timeout: Duration) -> Result<Vec<u8>, String> {
    check_line(line)?;
    let mut stream = UnixStream::connect(socket).map_err(|e| format!("the desktop is not answering on {}: {e}", socket.display()))?;
    stream.set_read_timeout(Some(timeout)).map_err(|e| e.to_string())?;
    stream.set_write_timeout(Some(timeout)).map_err(|e| e.to_string())?;
    stream.write_all(format!("{line}\n").as_bytes()).map_err(|e| e.to_string())?;
    let mut answer = Vec::new();
    stream.read_to_end(&mut answer).map_err(|e| format!("no answer from the desktop: {e}"))?;
    Ok(answer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_socket_is_the_shells_control_socket() {
        assert_eq!(springboard::bare::control::SOCKET, "/run/bifrost/shell-control.sock");
        assert!(socket().is_absolute());
    }

    #[test]
    fn lines_are_one_short_line() {
        assert!(check_line("open-folder a /x\nopen-app b").is_err());
        assert!(check_line(&"a".repeat(MAX_LINE + 1)).is_err());
        assert!(check_line("shot 1").is_ok());
    }

    #[test]
    fn request_reads_the_whole_answer_until_the_shell_closes() {
        let dir = std::env::temp_dir().join(format!("bifrost-cmd-control-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("c.sock");
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        let server = std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            let mut buf = [0u8; 64];
            let n = conn.read(&mut buf).unwrap();
            // a body larger than one read, then the close that ends the answer
            conn.write_all(&vec![7u8; 10_000]).unwrap();
            String::from_utf8_lossy(&buf[..n]).into_owned()
        });
        let answer = request("shot 1", &path).unwrap();
        assert_eq!(answer.len(), 10_000);
        assert_eq!(server.join().unwrap(), "shot 1\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_socket_says_where_it_looked() {
        let e = request("state", Path::new("/nonexistent/shell-control.sock")).unwrap_err();
        assert!(e.contains("/nonexistent/shell-control.sock"), "{e}");
    }
}
