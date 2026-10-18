//! Game Mode's priority request to `bifrostd` (`bifrostd::priority`): the game in front gets a negative nice and a
//! uclamp minimum, and both go back to normal when it leaves the front. The request runs on its own thread, so the
//! shell's frame never waits for it, and a refused request only logs.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

/// The socket `bifrostd` serves the priority requests on, under the system data root.
pub const SOCKET: &str = "/var/lib/springboard/run/priority.sock";
/// The nice value a game is raised to (`bifrostd` accepts -5 to 0).
pub const NICE: i32 = -5;
/// The uclamp minimum a game is raised to, on the kernel's 0..1024 scale.
pub const UTIL_MIN: u32 = 512;

/// The one request line for a process (`bifrostd` reads `priority <pid> <nice> <util_min>`).
pub fn line(pid: u32, nice: i32, util_min: u32) -> String {
    format!("priority {pid} {nice} {util_min}\n")
}

/// Sends one request and reads the reply: `ok`, or the reason it was refused.
pub fn send(socket: &str, pid: u32, nice: i32, util_min: u32) -> Result<(), String> {
    let mut stream = UnixStream::connect(socket).map_err(|e| e.to_string())?;
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    stream.write_all(line(pid, nice, util_min).as_bytes()).map_err(|e| e.to_string())?;
    let mut reply = String::new();
    BufReader::new(stream).read_line(&mut reply).map_err(|e| e.to_string())?;
    match reply.trim() {
        "ok" => Ok(()),
        other => Err(other.to_string()),
    }
}

/// Raises `pid` to the game priority.
pub fn raise(pid: u32) {
    report(send(SOCKET, pid, NICE, UTIL_MIN));
}

/// Puts `pid` back to nice 0 and no uclamp minimum.
pub fn restore(pid: u32) {
    report(send(SOCKET, pid, 0, 0));
}

fn report(result: Result<(), String>) {
    if let Err(e) = result {
        eprintln!("springboard: game priority: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_request_line_is_the_one_bifrostd_reads() {
        assert_eq!(line(4242, NICE, UTIL_MIN), "priority 4242 -5 512\n");
        assert_eq!(line(4242, 0, 0), "priority 4242 0 0\n");
        // the shell's values are inside what bifrostd accepts
        assert!((-5..=0).contains(&NICE) && UTIL_MIN <= 1024);
    }

    #[test]
    fn a_missing_socket_is_an_error_not_a_panic() {
        assert!(send("/nonexistent/bifrost-test-priority.sock", 4242, NICE, UTIL_MIN).is_err());
    }
}
