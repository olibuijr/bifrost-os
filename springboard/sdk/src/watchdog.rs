//! Service watchdog over systemd: a long-running service tells its manager it is alive.
//!
//! systemd arms a watchdog for a unit that sets `WatchdogSec=`. It exports the timeout as `WATCHDOG_USEC` and
//! the socket to write to as `NOTIFY_SOCKET`. The service must send `WATCHDOG=1` before the timeout runs out, or
//! systemd restarts it. [`interval`] gives the period to ping at (half the timeout, as systemd recommends), and
//! [`ping`] sends the datagram. Both read the environment through pure helpers so the parsing is tested without
//! a manager. The machine-wide hardware watchdog is configured separately in `bifrost-watchdog.conf`; this module
//! is the per-service side.

use std::io;
use std::os::unix::net::UnixDatagram;
use std::time::Duration;

/// The datagram that resets a unit's watchdog.
pub const PING: &[u8] = b"WATCHDOG=1";

/// The ping period for a timeout given in microseconds: half of it, so one late ping is not fatal.
/// Returns `None` when the value is missing, not a number, or zero (the unit has no watchdog).
pub fn interval_for_usec(usec: Option<&str>) -> Option<Duration> {
    let usec: u64 = usec?.trim().parse().ok()?;
    if usec == 0 {
        return None;
    }
    Some(Duration::from_micros(usec / 2))
}

/// The ping period for this process, from `WATCHDOG_USEC`. `None` when the unit has no watchdog.
pub fn interval() -> Option<Duration> {
    interval_for_usec(std::env::var("WATCHDOG_USEC").ok().as_deref())
}

/// Sends a ping to `NOTIFY_SOCKET`. Returns `Ok(false)` when the variable is unset (not run under systemd), so a
/// caller can ping unconditionally.
pub fn ping() -> io::Result<bool> {
    match std::env::var("NOTIFY_SOCKET") {
        Ok(path) if !path.is_empty() => {
            send_to(&path, PING)?;
            Ok(true)
        }
        _ => Ok(false),
    }
}

/// Writes `message` to a notify socket. A leading `@` names an abstract socket, as systemd exports it.
pub fn send_to(path: &str, message: &[u8]) -> io::Result<()> {
    let socket = UnixDatagram::unbound()?;
    if let Some(name) = path.strip_prefix('@') {
        use std::os::linux::net::SocketAddrExt;
        let addr = std::os::unix::net::SocketAddr::from_abstract_name(name.as_bytes())?;
        socket.send_to_addr(message, &addr)?;
    } else {
        socket.send_to(message, path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ping_period_is_half_the_timeout() {
        assert_eq!(interval_for_usec(Some("60000000")), Some(Duration::from_secs(30)));
        assert_eq!(interval_for_usec(Some(" 3000000 ")), Some(Duration::from_millis(1500)));
    }

    #[test]
    fn no_watchdog_means_no_period() {
        assert_eq!(interval_for_usec(None), None);
        assert_eq!(interval_for_usec(Some("0")), None);
        assert_eq!(interval_for_usec(Some("soon")), None);
    }

    #[test]
    fn the_ping_is_the_systemd_message() {
        assert_eq!(PING, b"WATCHDOG=1");
    }

    #[test]
    fn a_ping_reaches_a_notify_socket() {
        let dir = std::env::temp_dir().join(format!("watchdog-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("notify.sock");
        let _ = std::fs::remove_file(&path);
        let server = UnixDatagram::bind(&path).unwrap();
        send_to(path.to_str().unwrap(), PING).unwrap();
        let mut buf = [0u8; 64];
        let n = server.recv(&mut buf).unwrap();
        assert_eq!(&buf[..n], PING);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
