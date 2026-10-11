//! Correct time: an SNTP client (RFC 4330) and the
//! background task that keeps the system clock right once Wi-Fi has an IP.
//! Packet encoding and the offset arithmetic are pure and unit tested.

use super::wifi::{State, Status};
use crate::bare::{sys, timezone};
use std::net::{ToSocketAddrs, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const SERVERS: [&str; 2] = ["pool.ntp.org", "time.cloudflare.com"];
const PORT: u16 = 123;
const TIMEOUT: Duration = Duration::from_secs(3);
const RETRIES: usize = 3;
/// Seconds between NTP epoch (1900) and Unix epoch (1970).
const UNIX_DELTA: i64 = 2_208_988_800;
/// The clock is only set when it is off by more than this.
pub const STEP_US: i64 = 2_000_000;
const RESYNC: Duration = Duration::from_secs(6 * 3600);
/// After a failed attempt, look again sooner.
const RETRY_AFTER: Duration = Duration::from_secs(300);
const POLL: Duration = Duration::from_secs(5);

// ---- packets ----

/// An NTP timestamp (32.32 fixed point, seconds since 1900) as Unix
/// microseconds. Seconds with the top bit clear are read as era 1 (2036 on).
pub fn to_unix_us(secs: u32, frac: u32) -> i64 {
    let s = if secs >= 0x8000_0000 { secs as i64 - UNIX_DELTA } else { secs as i64 + (1i64 << 32) - UNIX_DELTA };
    // rounded to the nearest microsecond, matching from_unix_us
    s * 1_000_000 + ((frac as i64 * 1_000_000 + (1 << 31)) >> 32)
}

/// Unix microseconds as the 8 bytes of an NTP timestamp.
pub fn from_unix_us(us: i64) -> [u8; 8] {
    let secs = us.div_euclid(1_000_000) + UNIX_DELTA;
    // rounded, so a microsecond survives the 2^-32 s grid; the top of the second saturates
    let frac = (((us.rem_euclid(1_000_000) << 32) + 500_000) / 1_000_000).min(u32::MAX as i64);
    let mut b = [0u8; 8];
    b[..4].copy_from_slice(&(secs as u32).to_be_bytes());
    b[4..].copy_from_slice(&(frac as u32).to_be_bytes());
    b
}

fn stamp(b: &[u8]) -> i64 {
    to_unix_us(u32::from_be_bytes(b[..4].try_into().unwrap()), u32::from_be_bytes(b[4..8].try_into().unwrap()))
}

/// A client request: LI 0, version 4, mode 3, our send time `t1` in the
/// transmit field (the server echoes it back as the origin).
pub fn encode_request(t1_us: i64) -> [u8; 48] {
    let mut p = [0u8; 48];
    p[0] = 0x23;
    p[40..].copy_from_slice(&from_unix_us(t1_us));
    p
}

/// The server's receive (t2) and transmit (t3) times, Unix microseconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reply {
    pub t2: i64,
    pub t3: i64,
}

/// Checks and reads a server reply to the request `sent`: it must be a
/// server-mode packet from a synchronised stratum 1-15 server that echoes
/// our transmit time.
pub fn decode_response(sent: &[u8; 48], buf: &[u8]) -> Result<Reply, &'static str> {
    if buf.len() < 48 {
        return Err("short reply");
    }
    if buf[0] & 7 != 4 {
        return Err("not a server reply");
    }
    if buf[0] >> 6 == 3 {
        return Err("server clock unsynchronised");
    }
    if buf[1] == 0 || buf[1] > 15 {
        return Err("kiss-of-death or bad stratum");
    }
    if buf[24..32] != sent[40..48] {
        return Err("origin timestamp mismatch");
    }
    if buf[40..48] == [0; 8] {
        return Err("no transmit timestamp");
    }
    Ok(Reply { t2: stamp(&buf[32..40]), t3: stamp(&buf[40..48]) })
}

/// One exchange: how far the local clock is behind the server (positive =
/// local clock is slow) and the network round trip, both in microseconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    pub offset_us: i64,
    pub delay_us: i64,
}

/// RFC 4330: offset = ((t2 - t1) + (t3 - t4)) / 2, delay = (t4 - t1) - (t3 - t2).
pub fn measure(t1: i64, r: Reply, t4: i64) -> Sample {
    Sample { offset_us: ((r.t2 - t1) + (r.t3 - t4)) / 2, delay_us: (t4 - t1) - (r.t3 - r.t2) }
}

/// Whether an offset is large enough to set the clock.
pub fn should_step(offset_us: i64) -> bool {
    offset_us.abs() > STEP_US
}

fn now_us() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_micros() as i64)
}

/// Asks one server (host name or address) for the time.
pub fn query(server: &str) -> Result<Sample, String> {
    let addr = (server, PORT).to_socket_addrs().map_err(|e| format!("{server}: {e}"))?.find(|a| a.is_ipv4()).ok_or_else(|| format!("{server}: no address"))?;
    let sock = UdpSocket::bind("0.0.0.0:0").map_err(|e| e.to_string())?;
    sock.set_read_timeout(Some(TIMEOUT)).and_then(|_| sock.set_write_timeout(Some(TIMEOUT))).map_err(|e| e.to_string())?;
    sock.connect(addr).map_err(|e| e.to_string())?;
    let t1 = now_us();
    let req = encode_request(t1);
    sock.send(&req).map_err(|e| format!("{server}: {e}"))?;
    let mut buf = [0u8; 128];
    let n = sock.recv(&mut buf).map_err(|e| format!("{server}: {e}"))?;
    let t4 = now_us();
    let reply = decode_response(&req, &buf[..n]).map_err(|e| format!("{server}: {e}"))?;
    Ok(measure(t1, reply, t4))
}

/// Tries each server up to `RETRIES` times; the first answer wins.
pub fn sync_once() -> Result<Sample, String> {
    let mut last = String::from("no server");
    for server in SERVERS {
        for _ in 0..RETRIES {
            match query(server) {
                Ok(s) => return Ok(s),
                Err(e) => last = e,
            }
        }
    }
    Err(last)
}

// ---- settings and status shared with the Settings page ----

static AUTO: AtomicBool = AtomicBool::new(true);
static KICK: AtomicBool = AtomicBool::new(false);

enum Outcome {
    Synced { offset_us: i64 },
    Failed(String),
}

static LAST: Mutex<Option<(Instant, Outcome)>> = Mutex::new(None);

/// systemd-timesyncd owns the clock; this module only reports and toggles it.
fn desktop() -> bool {
    true
}

/// "Synchronized by systemd-timesyncd" from timedatectl's NTPSynchronized/NTP properties.
pub fn timesyncd_text(synchronized: Option<&str>, enabled: Option<&str>) -> String {
    match (synchronized, enabled) {
        (Some("yes"), _) => "Synchronized by systemd-timesyncd".into(),
        (_, Some("no")) => "Network time is off (timedatectl set-ntp true)".into(),
        (Some(_), _) => "Waiting for systemd-timesyncd".into(),
        _ => "timedatectl is not available".into(),
    }
}

/// Whether the clock is kept right automatically (Set Automatically).
pub fn auto() -> bool {
    AUTO.load(Ordering::Relaxed)
}

/// Turns Set Automatically on or off; turning it on syncs at once.
pub fn set_auto(on: bool) {
    AUTO.store(on, Ordering::Relaxed);
    if desktop() {
        timezone::set_ntp(on); // systemd-timesyncd does the syncing
        return;
    }
    if on {
        KICK.store(true, Ordering::Relaxed);
    }
}

/// "+0.3 s" style offset.
fn offset_text(offset_us: i64) -> String {
    format!("{:+.1} s", offset_us as f64 / 1e6)
}

/// "Synced 2 min ago, offset +0.3 s".
pub fn describe(ago_secs: u64, offset_us: i64) -> String {
    let ago = match ago_secs {
        0..=59 => "just now".to_string(),
        60..=3599 => format!("{} min ago", ago_secs / 60),
        _ => format!("{} h ago", ago_secs / 3600),
    };
    format!("Synced {ago}, offset {}", offset_text(offset_us))
}

/// The last sync result for the Date & Time page.
pub fn status_text() -> String {
    if desktop() {
        return timesyncd_text(timezone::timesyncd_synchronized(), timezone::timesyncd_enabled());
    }
    match LAST.lock().ok().as_deref() {
        Some(Some((at, Outcome::Synced { offset_us }))) => describe(at.elapsed().as_secs(), *offset_us),
        Some(Some((_, Outcome::Failed(e)))) => format!("Last sync failed: {e}"),
        _ => "Not synced yet".to_string(),
    }
}

fn record(o: Outcome) {
    if let Ok(mut l) = LAST.lock() {
        *l = Some((Instant::now(), o));
    }
}

/// One sync: measure, and set the clock when it is off by more than 2 s.
fn run_sync() -> bool {
    let sample = match sync_once() {
        Ok(s) => s,
        Err(e) => {
            record(Outcome::Failed(e));
            return false;
        }
    };
    if should_step(sample.offset_us) {
        let target = now_us() + sample.offset_us;
        if let Err(e) = sys::set_realtime(target.div_euclid(1_000_000), target.rem_euclid(1_000_000) * 1000) {
            record(Outcome::Failed(format!("cannot set clock: {e}")));
            return false;
        }
    }
    record(Outcome::Synced { offset_us: sample.offset_us });
    true
}

fn online(status: &Mutex<Status>) -> bool {
    status.lock().is_ok_and(|s| (s.state == State::Connected && s.ip.is_some()) || s.ethernet.as_ref().is_some_and(|e| e.ip.is_some()))
}

/// Starts the sync task: once Wi-Fi has an IP (and whenever it gets one
/// again) it syncs, then every 6 h while Set Automatically is on.
pub fn spawn(status: Arc<Mutex<Status>>) {
    if desktop() {
        return; // systemd-timesyncd keeps the time; two clients would fight over it
    }
    crate::bare::sys::spawn_named("ntp", move || {
        let mut was_online = false;
        let mut next = Instant::now();
        loop {
            std::thread::sleep(POLL);
            let up = online(&status);
            if up && !was_online {
                next = Instant::now();
            }
            was_online = up;
            let kicked = KICK.swap(false, Ordering::Relaxed);
            if up && auto() && (kicked || Instant::now() >= next) {
                next = Instant::now() + if run_sync() { RESYNC } else { RETRY_AFTER };
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timesyncd_status_wording() {
        assert_eq!(timesyncd_text(Some("yes"), Some("yes")), "Synchronized by systemd-timesyncd");
        assert!(timesyncd_text(Some("no"), Some("no")).starts_with("Network time is off"));
        assert_eq!(timesyncd_text(Some("no"), Some("yes")), "Waiting for systemd-timesyncd");
        assert_eq!(timesyncd_text(None, None), "timedatectl is not available");
    }

    /// A well-formed server reply to `req` with the given times.
    fn reply(req: &[u8; 48], t2: i64, t3: i64) -> [u8; 48] {
        let mut p = [0u8; 48];
        p[0] = 0x24;
        p[1] = 2;
        p[24..32].copy_from_slice(&req[40..48]);
        p[32..40].copy_from_slice(&from_unix_us(t2));
        p[40..48].copy_from_slice(&from_unix_us(t3));
        p
    }

    #[test]
    fn timestamps_round_trip_between_ntp_and_unix() {
        // 2024-01-01 00:00:00 UTC
        let unix_s = 1_704_067_200i64;
        let b = from_unix_us(unix_s * 1_000_000 + 500_000);
        assert_eq!(u32::from_be_bytes(b[..4].try_into().unwrap()) as i64, unix_s + UNIX_DELTA);
        assert_eq!(stamp(&b), unix_s * 1_000_000 + 500_000);
        assert_eq!(to_unix_us(0, 0), (1i64 << 32) * 1_000_000 - UNIX_DELTA * 1_000_000, "era 1 starts in 2036");
    }

    #[test]
    fn request_is_a_client_packet_carrying_the_send_time() {
        let p = encode_request(1_704_067_200_000_000);
        assert_eq!(p[0], 0x23);
        assert_eq!(&p[1..40], &[0u8; 39][..]);
        assert_eq!(stamp(&p[40..]), 1_704_067_200_000_000);
    }

    #[test]
    fn response_is_decoded_and_validated() {
        let req = encode_request(1_000_000_000_000_000);
        let ok = reply(&req, 1_000_000_100_000, 1_000_000_200_000);
        assert_eq!(decode_response(&req, &ok), Ok(Reply { t2: 1_000_000_100_000, t3: 1_000_000_200_000 }));
        assert_eq!(decode_response(&req, &ok[..40]), Err("short reply"));
        let mut bad = ok;
        bad[0] = 0x23;
        assert_eq!(decode_response(&req, &bad), Err("not a server reply"));
        let mut bad = ok;
        bad[0] |= 0xC0;
        assert_eq!(decode_response(&req, &bad), Err("server clock unsynchronised"));
        let mut bad = ok;
        bad[1] = 0;
        assert_eq!(decode_response(&req, &bad), Err("kiss-of-death or bad stratum"));
        let mut bad = ok;
        bad[24] ^= 1;
        assert_eq!(decode_response(&req, &bad), Err("origin timestamp mismatch"));
        let mut bad = ok;
        bad[40..48].fill(0);
        assert_eq!(decode_response(&req, &bad), Err("no transmit timestamp"));
    }

    #[test]
    fn offset_and_delay_follow_rfc_4330() {
        // local clock 10 s slow, 100 ms each way, 20 ms server turnaround
        let t1 = 1_000_000_000;
        let slow = 10_000_000;
        let t2 = t1 + 100_000 + slow;
        let t3 = t2 + 20_000;
        let s = measure(t1, Reply { t2, t3 }, t1 + 220_000);
        assert_eq!(s, Sample { offset_us: slow, delay_us: 200_000 });
        // local clock fast by 3 s gives a negative offset
        let fast = measure(0, Reply { t2: -3_000_000 + 50_000, t3: -3_000_000 + 60_000 }, 110_000);
        assert_eq!(fast.offset_us, -3_000_000);
    }

    #[test]
    fn the_clock_is_only_set_beyond_two_seconds() {
        assert!(!should_step(0) && !should_step(2_000_000) && !should_step(-2_000_000));
        assert!(should_step(2_000_001) && should_step(-5_000_000));
    }

    #[test]
    fn last_sync_reads_like_mobile() {
        assert_eq!(describe(5, 300_000), "Synced just now, offset +0.3 s");
        assert_eq!(describe(120, -1_500_000), "Synced 2 min ago, offset -1.5 s");
        assert_eq!(describe(7_300, 0), "Synced 2 h ago, offset +0.0 s");
    }
}
