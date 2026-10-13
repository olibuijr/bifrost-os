//! The shell's control socket, shared by the command-line tools. One request per connection: a single text line, then the
//! shell writes its whole answer and closes the connection. Text answers start with `ok` or `error: <why>`; `shot` answers
//! with a raw `RIMG` frame (see `screencapture`), so the client reads to the end rather than to the first line.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

/// The shell's control port; `SPRINGBOARD_CONTROL_PORT` overrides it, as it does in the shell.
pub const DEFAULT_PORT: u16 = 7777;
/// The longest request line the tools send (the limit `bifrost-open-url` keeps).
pub const MAX_LINE: usize = 510;
const TIMEOUT: Duration = Duration::from_secs(5);

/// The control port from `SPRINGBOARD_CONTROL_PORT`, else the default.
pub fn port() -> u16 {
    port_from(std::env::var("SPRINGBOARD_CONTROL_PORT").ok().as_deref())
}

/// The port for an environment value: a number in range, else the default.
pub fn port_from(v: Option<&str>) -> u16 {
    v.and_then(|s| s.trim().parse().ok()).unwrap_or(DEFAULT_PORT)
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

/// Sends one request line to the shell on `127.0.0.1:port` and returns every byte of its answer.
pub fn request(line: &str, port: u16) -> Result<Vec<u8>, String> {
    check_line(line)?;
    let mut stream = TcpStream::connect(("127.0.0.1", port)).map_err(|e| format!("the desktop is not answering on port {port}: {e}"))?;
    stream.set_read_timeout(Some(TIMEOUT)).map_err(|e| e.to_string())?;
    stream.set_write_timeout(Some(TIMEOUT)).map_err(|e| e.to_string())?;
    stream.write_all(format!("{line}\n").as_bytes()).map_err(|e| e.to_string())?;
    let mut answer = Vec::new();
    stream.read_to_end(&mut answer).map_err(|e| format!("no answer from the desktop: {e}"))?;
    Ok(answer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_comes_from_the_environment_when_it_is_a_number() {
        assert_eq!(port_from(None), DEFAULT_PORT);
        assert_eq!(port_from(Some(" 8000 ")), 8000);
        assert_eq!(port_from(Some("many")), DEFAULT_PORT);
        assert_eq!(port_from(Some("99999")), DEFAULT_PORT);
    }

    #[test]
    fn lines_are_one_short_line() {
        assert!(check_line("open-folder a /x\nopen-app b").is_err());
        assert!(check_line(&"a".repeat(MAX_LINE + 1)).is_err());
        assert!(check_line("shot 1").is_ok());
    }

    #[test]
    fn request_reads_the_whole_answer_until_the_shell_closes() {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            let mut buf = [0u8; 64];
            let n = conn.read(&mut buf).unwrap();
            // a body larger than one read, then the close that ends the answer
            conn.write_all(&vec![7u8; 10_000]).unwrap();
            String::from_utf8_lossy(&buf[..n]).into_owned()
        });
        let answer = request("shot 1", port).unwrap();
        assert_eq!(answer.len(), 10_000);
        assert_eq!(server.join().unwrap(), "shot 1\n");
    }
}
