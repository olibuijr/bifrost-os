//! Nearby sharing on the local network: discovery plus file transfer, pure std.
//!
//! WIRE FORMAT (all integers big endian)
//!
//! Beacon, UDP multicast to 224.0.0.251:5354 every 2 s (also sent once on start):
//!   "TSB1"        4 bytes  magic + version
//!   port          u16      TCP port the sender listens on
//!   fingerprint   8 bytes  first 8 bytes of sha256("tab-share-key:" + secret)
//!   name_len      u8       1..=64
//!   name          utf-8    device name
//! Beacons whose fingerprint differs from ours (other secret) are ignored, and
//! so are our own. A peer not heard for 7 s drops out of the list.
//!
//! Transfer, TCP to the beacon's address and port:
//!   sender -> "TSX1" | name_len u16 | name | size u64 | sha256 32 bytes | mac 32 bytes
//!             mac = sha256(secret | every header byte before the mac)
//!   receiver -> 1 byte: 1 = accept, 0 = decline (after the user decided)
//!   sender -> body, exactly `size` bytes, streamed
//!   receiver -> 1 byte: 1 = sha256 matched and file saved, 0 = failed
//! The name is a bare file name (no '/'); the receiver sanitises it anyway and
//! never overwrites (it appends " 2", " 3", ...). The default secret is
//! "nearby": same LAN plus an explicit Accept is the whole trust model.

use crate::shell::sha256::{self, Sha256};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpListener, TcpStream, UdpSocket};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const GROUP: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 251);
pub const BEACON_PORT: u16 = 5354;
pub const DEFAULT_SECRET: &str = "nearby";
pub const BEACON_EVERY: Duration = Duration::from_secs(2);
pub const PEER_TTL: Duration = Duration::from_secs(7);
/// How long an unanswered offer waits before it counts as declined.
pub const OFFER_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_NAME: usize = 64;
const MAX_FILE_NAME: usize = 255;
/// Largest file the receiver accepts, whatever the sender's header claims.
pub const MAX_FILE_SIZE: u64 = 4 << 30;
/// Free space that must remain after an incoming file is stored.
const FREE_SPACE_RESERVE: u64 = 64 << 20;
/// The whole body of a transfer must arrive within this, however slowly it trickles.
const RECEIVE_DEADLINE: Duration = Duration::from_secs(30 * 60);
/// How many " 2", " 3", ... names `create_unique` tries before giving up.
const MAX_UNIQUE_TRIES: u32 = 1000;
const BEACON_MAGIC: &[u8; 4] = b"TSB1";
const XFER_MAGIC: &[u8; 4] = b"TSX1";

// ---- beacon ----

pub fn fingerprint(secret: &str) -> [u8; 8] {
    let d = sha256::digest(format!("tab-share-key:{secret}").as_bytes());
    d[..8].try_into().unwrap()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Beacon {
    pub name: String,
    pub port: u16,
    pub fingerprint: [u8; 8],
}

impl Beacon {
    pub fn encode(&self) -> Vec<u8> {
        let name: String = self.name.chars().take(MAX_NAME).collect();
        let name = truncate_utf8(&name, MAX_NAME);
        let mut out = Vec::with_capacity(15 + name.len());
        out.extend_from_slice(BEACON_MAGIC);
        out.extend_from_slice(&self.port.to_be_bytes());
        out.extend_from_slice(&self.fingerprint);
        out.push(name.len() as u8);
        out.extend_from_slice(name.as_bytes());
        out
    }

    pub fn decode(b: &[u8]) -> Option<Beacon> {
        if b.len() < 15 || &b[..4] != BEACON_MAGIC {
            return None;
        }
        let port = u16::from_be_bytes([b[4], b[5]]);
        let fingerprint: [u8; 8] = b[6..14].try_into().ok()?;
        let n = b[14] as usize;
        if n == 0 || n > MAX_NAME || b.len() != 15 + n {
            return None;
        }
        let name = std::str::from_utf8(&b[15..]).ok()?.to_string();
        Some(Beacon { name, port, fingerprint })
    }
}

fn truncate_utf8(s: &str, max: usize) -> &str {
    let mut n = s.len().min(max);
    while !s.is_char_boundary(n) {
        n -= 1;
    }
    &s[..n]
}

// ---- transfer header ----

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    pub name: String,
    pub size: u64,
    pub sha256: [u8; 32],
}

fn mac(secret: &str, body: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(secret.as_bytes());
    h.update(body);
    h.finish()
}

impl Header {
    pub fn encode(&self, secret: &str) -> Vec<u8> {
        let name = truncate_utf8(&self.name, MAX_FILE_NAME);
        let mut out = Vec::with_capacity(78 + name.len());
        out.extend_from_slice(XFER_MAGIC);
        out.extend_from_slice(&(name.len() as u16).to_be_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&self.size.to_be_bytes());
        out.extend_from_slice(&self.sha256);
        let m = mac(secret, &out);
        out.extend_from_slice(&m);
        out
    }

    /// Reads one header; fails on a bad magic, an oversize name or a wrong mac.
    pub fn read(r: &mut impl Read, secret: &str) -> io::Result<Header> {
        let bad = |m: &str| io::Error::new(io::ErrorKind::InvalidData, m.to_string());
        let mut raw = Vec::new();
        let mut fixed = [0u8; 6];
        r.read_exact(&mut fixed)?;
        if &fixed[..4] != XFER_MAGIC {
            return Err(bad("not a share header"));
        }
        let n = u16::from_be_bytes([fixed[4], fixed[5]]) as usize;
        if n == 0 || n > MAX_FILE_NAME {
            return Err(bad("bad name length"));
        }
        raw.extend_from_slice(&fixed);
        let mut rest = vec![0u8; n + 8 + 32 + 32];
        r.read_exact(&mut rest)?;
        raw.extend_from_slice(&rest[..n + 40]);
        if mac(secret, &raw)[..] != rest[n + 40..] {
            return Err(bad("wrong secret"));
        }
        let name = String::from_utf8(rest[..n].to_vec()).map_err(|_| bad("name is not utf-8"))?;
        let size = u64::from_be_bytes(rest[n..n + 8].try_into().unwrap());
        let sha256 = rest[n + 8..n + 40].try_into().unwrap();
        Ok(Header { name, size, sha256 })
    }
}

/// A safe single path component: no separators, no leading dots, not empty.
pub fn sanitize_name(name: &str) -> String {
    let last = name.rsplit(['/', '\\']).next().unwrap_or("");
    let cleaned: String = last.chars().filter(|c| !c.is_control()).collect();
    let cleaned = cleaned.trim().trim_start_matches('.').to_string();
    if cleaned.is_empty() {
        "Received".into()
    } else {
        truncate_utf8(&cleaned, MAX_FILE_NAME).to_string()
    }
}

/// Creates `dir/name`, or `dir/stem 2.ext`, `stem 3.ext`... when taken, and returns it open.
/// `create_new` makes the claim atomic: an existing file is never reused or overwritten.
pub fn create_unique(dir: &Path, name: &str) -> io::Result<(PathBuf, std::fs::File)> {
    let p = Path::new(name);
    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or(name);
    let ext = p.extension().and_then(|s| s.to_str());
    let candidates = std::iter::once(dir.join(name)).chain((2..=MAX_UNIQUE_TRIES).map(|i| match ext {
        Some(e) => dir.join(format!("{stem} {i}.{e}")),
        None => dir.join(format!("{stem} {i}")),
    }));
    for path in candidates {
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(f) => return Ok((path, f)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(io::ErrorKind::AlreadyExists, "too many files with that name"))
}

/// Whether a `size` byte file may be received given the free bytes on the target volume (if known).
pub fn check_incoming(size: u64, free: Option<u64>) -> io::Result<()> {
    if size > MAX_FILE_SIZE {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "file is too large"));
    }
    match free {
        Some(free) if free < size.saturating_add(FREE_SPACE_RESERVE) => Err(io::Error::new(io::ErrorKind::StorageFull, "not enough free space")),
        _ => Ok(()),
    }
}

/// A hidden temporary name no other transfer (or file) shares, short enough for any file name limit.
fn part_name() -> String {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    format!(".share-{}-{}.part", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed))
}

// ---- sending ----

/// Sends `size` bytes of `src` (read twice: once to hash, once to stream).
/// `progress(sent, total)` is called as the body goes out.
pub fn send<R: Read + Seek>(addr: SocketAddr, secret: &str, name: &str, src: &mut R, mut progress: impl FnMut(u64, u64)) -> io::Result<()> {
    let size = src.seek(SeekFrom::End(0))?;
    src.seek(SeekFrom::Start(0))?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = src.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    src.seek(SeekFrom::Start(0))?;
    let header = Header { name: sanitize_name(name), size, sha256: h.finish() };
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_secs(5))?;
    s.set_nodelay(true).ok();
    s.write_all(&header.encode(secret))?;
    s.set_read_timeout(Some(OFFER_TIMEOUT + Duration::from_secs(5)))?;
    let mut ack = [0u8; 1];
    s.read_exact(&mut ack)?;
    if ack[0] != 1 {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "declined"));
    }
    let mut sent = 0u64;
    while sent < size {
        let n = src.read(&mut buf)?;
        if n == 0 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "file shrank"));
        }
        s.write_all(&buf[..n])?;
        sent += n as u64;
        progress(sent, size);
    }
    s.read_exact(&mut ack)?;
    if ack[0] == 1 {
        Ok(())
    } else {
        Err(io::Error::new(io::ErrorKind::InvalidData, "receiver rejected the file"))
    }
}

// ---- receiving ----

/// An incoming file waiting for the user's Accept or Decline.
pub struct Offer {
    pub name: String,
    pub size: u64,
    pub from: SocketAddr,
    reply: Sender<bool>,
}

impl Offer {
    pub fn accept(self) {
        let _ = self.reply.send(true);
    }
    pub fn decline(self) {
        let _ = self.reply.send(false);
    }
    /// Text for the banner shown to the user.
    pub fn banner(&self) -> (String, String) {
        (format!("{} wants to share", self.from.ip()), format!("{} ({})", self.name, human_size(self.size)))
    }
}

pub enum Event {
    Offer(Offer),
    /// saved to this path
    Received(PathBuf),
    Failed(String),
}

/// "12 B", "820.0 KB", "1.4 MB": binary (1024) units with one decimal.
pub fn human_size(n: u64) -> String {
    const U: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < 3 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 { format!("{n} B") } else { format!("{v:.1} {}", U[i]) }
}

fn handle(mut s: TcpStream, secret: &str, dir: &Path, events: &Sender<Event>, tracker: &Tracker) -> io::Result<()> {
    s.set_read_timeout(Some(Duration::from_secs(10)))?;
    let from = s.peer_addr()?;
    let h = Header::read(&mut s, secret)?;
    std::fs::create_dir_all(dir)?;
    let free = crate::bare::sys::disk_space(&dir.to_string_lossy()).map(|(_, avail)| avail);
    if let Err(e) = check_incoming(h.size, free) {
        let _ = s.write_all(&[0]);
        return Err(e);
    }
    let (tx, rx) = mpsc::channel();
    let name = sanitize_name(&h.name);
    let _ = events.send(Event::Offer(Offer { name: name.clone(), size: h.size, from, reply: tx }));
    let ok = rx.recv_timeout(OFFER_TIMEOUT).unwrap_or(false);
    s.write_all(&[ok as u8])?;
    if !ok {
        return Ok(());
    }
    let id = tracker.begin(false, &from.ip().to_string(), &name, h.size);
    let deadline = Instant::now() + RECEIVE_DEADLINE;
    let part = dir.join(part_name());
    let result = (|| {
        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).open(&part)?;
        let mut hash = Sha256::new();
        let mut buf = vec![0u8; 64 * 1024];
        let mut left = h.size;
        while left > 0 {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(io::Error::new(io::ErrorKind::TimedOut, "transfer took too long"));
            }
            s.set_read_timeout(Some(remaining.min(Duration::from_secs(15))))?;
            let want = left.min(buf.len() as u64) as usize;
            let n = s.read(&mut buf[..want])?;
            if n == 0 {
                return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "connection closed early"));
            }
            hash.update(&buf[..n]);
            f.write_all(&buf[..n])?;
            left -= n as u64;
            tracker.progress(id, h.size - left, h.size);
        }
        f.sync_all()?;
        if hash.finish() != h.sha256 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "checksum mismatch"));
        }
        // Claim the final name atomically, then replace our own empty claim with the data.
        let (dest, claim) = create_unique(dir, &name)?;
        drop(claim);
        if let Err(e) = std::fs::rename(&part, &dest) {
            let _ = std::fs::remove_file(&dest);
            return Err(e);
        }
        Ok(dest)
    })();
    match result {
        Ok(dest) => {
            tracker.finish(id, None);
            let _ = s.write_all(&[1]);
            let _ = events.send(Event::Received(dest));
            Ok(())
        }
        Err(e) => {
            tracker.finish(id, Some(e.to_string()));
            let _ = std::fs::remove_file(&part);
            let _ = s.write_all(&[0]);
            Err(e)
        }
    }
}

// ---- progress ----

/// A finished transfer stays on the progress card this long, then drops off.
pub const DONE_SHOW: Duration = Duration::from_secs(4);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Running,
    Done,
    /// why it stopped, e.g. "declined" or "checksum mismatch"
    Failed(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transfer {
    pub id: u64,
    /// true for a file this device sends
    pub outgoing: bool,
    /// the other device: its name when known, else its address
    pub peer: String,
    pub name: String,
    pub done: u64,
    pub total: u64,
    pub status: Status,
    /// when it finished; None while it runs
    pub finished: Option<Instant>,
}

/// Every transfer this device runs, outgoing and incoming. The receiver threads
/// and the shell's send threads share clones, so the progress card reads live state.
#[derive(Clone, Default)]
pub struct Tracker {
    list: Arc<Mutex<Vec<Transfer>>>,
    next: Arc<AtomicU64>,
}

impl Tracker {
    /// Starts a transfer and returns its id.
    pub fn begin(&self, outgoing: bool, peer: &str, name: &str, total: u64) -> u64 {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let t = Transfer { id, outgoing, peer: peer.to_string(), name: name.to_string(), done: 0, total, status: Status::Running, finished: None };
        self.list.lock().unwrap_or_else(|p| p.into_inner()).push(t);
        id
    }

    /// Records the bytes moved so far; `total` is the file size.
    pub fn progress(&self, id: u64, done: u64, total: u64) {
        if let Some(t) = self.list.lock().unwrap_or_else(|p| p.into_inner()).iter_mut().find(|t| t.id == id) {
            t.done = done.min(total);
            t.total = total;
        }
    }

    /// Ends a transfer: `None` for success, otherwise the reason.
    pub fn finish(&self, id: u64, error: Option<String>) {
        self.finish_at(id, error, Instant::now());
    }

    fn finish_at(&self, id: u64, error: Option<String>, now: Instant) {
        if let Some(t) = self.list.lock().unwrap_or_else(|p| p.into_inner()).iter_mut().find(|t| t.id == id) {
            t.finished = Some(now);
            match error {
                None => {
                    t.status = Status::Done;
                    t.done = t.total;
                }
                Some(e) => t.status = Status::Failed(e),
            }
        }
    }

    /// The transfers to show at `now`: the running ones and the finished ones within DONE_SHOW.
    /// Older finished ones are dropped from the list.
    pub fn snapshot(&self, now: Instant) -> Vec<Transfer> {
        let mut list = self.list.lock().unwrap_or_else(|p| p.into_inner());
        list.retain(|t| t.finished.map_or(true, |f| now.duration_since(f) < DONE_SHOW));
        list.clone()
    }
}

// ---- the service: announce, browse, listen ----

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Peer {
    pub name: String,
    pub addr: SocketAddr,
}

/// Peers heard recently; pure bookkeeping so it can be tested.
#[derive(Default)]
pub struct Peers {
    seen: Vec<(Peer, Instant)>,
}

impl Peers {
    pub fn heard(&mut self, p: Peer, now: Instant) {
        match self.seen.iter_mut().find(|(q, _)| q.addr == p.addr) {
            Some(e) => *e = (p, now),
            None => self.seen.push((p, now)),
        }
    }
    pub fn live(&mut self, now: Instant) -> Vec<Peer> {
        self.seen.retain(|(_, t)| now.duration_since(*t) < PEER_TTL);
        self.seen.iter().map(|(p, _)| p.clone()).collect()
    }
}

pub struct Service {
    pub events: Receiver<Event>,
    /// the transfers this service receives; the shell adds the ones it sends
    pub tracker: Tracker,
    peers: Arc<Mutex<Peers>>,
    stop: Arc<AtomicBool>,
    pub port: u16,
    pub secret: String,
}

impl Service {
    /// Starts the three threads. Files land in `dir` (Files home/Received).
    pub fn start(name: &str, secret: &str, dir: PathBuf) -> io::Result<Service> {
        let listener = TcpListener::bind(("0.0.0.0", 0))?;
        let port = listener.local_addr()?.port();
        let stop = Arc::new(AtomicBool::new(false));
        let peers = Arc::new(Mutex::new(Peers::default()));
        let (tx, rx) = mpsc::channel();
        let secret_s = secret.to_string();
        let tracker = Tracker::default();

        let (st, tx2, sec, trk) = (stop.clone(), tx.clone(), secret_s.clone(), tracker.clone());
        // blocking accept: no timer; `Drop` connects once to wake it after setting `stop`
        crate::bare::sys::spawn_named("share-accept", move || {
            while !st.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((s, _)) => {
                        if st.load(Ordering::Relaxed) {
                            break;
                        }
                        let (tx3, sec, dir, trk) = (tx2.clone(), sec.clone(), dir.clone(), trk.clone());
                        crate::bare::sys::spawn_named("share-recv", move || {
                            s.set_nonblocking(false).ok();
                            if let Err(e) = handle(s, &sec, &dir, &tx3, &trk) {
                                let _ = tx3.send(Event::Failed(e.to_string()));
                            }
                        });
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(100)),
                }
            }
        });

        let beacon = Beacon { name: name.to_string(), port, fingerprint: fingerprint(secret) };
        let sock = UdpSocket::bind(("0.0.0.0", BEACON_PORT)).or_else(|_| UdpSocket::bind(("0.0.0.0", 0)))?;
        sock.join_multicast_v4(&GROUP, &Ipv4Addr::UNSPECIFIED).ok();
        sock.set_multicast_loop_v4(false).ok();
        sock.set_read_timeout(Some(BEACON_EVERY / 2))?;
        let (st, pr) = (stop.clone(), peers.clone());
        let fp = beacon.fingerprint;
        crate::bare::sys::spawn_named("share-beacon", move || {
            let bytes = beacon.encode();
            let dest = SocketAddrV4::new(GROUP, BEACON_PORT);
            let mut last = None::<Instant>;
            let mut buf = [0u8; 128];
            while !st.load(Ordering::Relaxed) {
                if last.map_or(true, |t| t.elapsed() >= BEACON_EVERY) {
                    let _ = sock.send_to(&bytes, dest);
                    last = Some(Instant::now());
                }
                if let Ok((n, from)) = sock.recv_from(&mut buf) {
                    if let Some(b) = Beacon::decode(&buf[..n]) {
                        if b.fingerprint == fp && b.port != beacon.port {
                            let addr = SocketAddr::new(from.ip(), b.port);
                            pr.lock().unwrap_or_else(|p| p.into_inner()).heard(Peer { name: b.name, addr }, Instant::now());
                        }
                    }
                }
            }
        });
        Ok(Service { events: rx, tracker, peers, stop, port, secret: secret_s })
    }

    pub fn peers(&self) -> Vec<Peer> {
        self.peers.lock().unwrap_or_else(|p| p.into_inner()).live(Instant::now())
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = std::net::TcpStream::connect_timeout(&SocketAddr::from(([127, 0, 0, 1], self.port)), Duration::from_millis(200));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn beacon_round_trips() {
        let b = Beacon { name: "Midget".into(), port: 40123, fingerprint: fingerprint("nearby") };
        let e = b.encode();
        assert_eq!(&e[..4], b"TSB1");
        assert_eq!(Beacon::decode(&e), Some(b));
    }

    #[test]
    fn beacon_rejects_garbage() {
        let b = Beacon { name: "x".into(), port: 1, fingerprint: [0; 8] }.encode();
        assert!(Beacon::decode(&b[..b.len() - 1]).is_none());
        let mut bad = b.clone();
        bad[0] = b'X';
        assert!(Beacon::decode(&bad).is_none());
        assert!(Beacon::decode(b"").is_none());
        let mut empty = b.clone();
        empty[14] = 0;
        assert!(Beacon::decode(&empty[..15]).is_none());
    }

    #[test]
    fn beacon_truncates_long_names_on_a_char_boundary() {
        let b = Beacon { name: "é".repeat(100), port: 1, fingerprint: [1; 8] };
        let d = Beacon::decode(&b.encode()).unwrap();
        assert!(d.name.len() <= MAX_NAME && d.name.chars().all(|c| c == 'é'));
    }

    #[test]
    fn fingerprints_differ_per_secret() {
        assert_ne!(fingerprint("a"), fingerprint("b"));
        assert_eq!(fingerprint("a"), fingerprint("a"));
    }

    #[test]
    fn header_round_trips_and_checks_the_secret() {
        let h = Header { name: "photo é.jpg".into(), size: 1 << 40, sha256: sha256::digest(b"x") };
        let e = h.encode("nearby");
        assert_eq!(&e[..4], b"TSX1");
        assert_eq!(Header::read(&mut &e[..], "nearby").unwrap(), h);
        assert!(Header::read(&mut &e[..], "other").is_err());
        let mut tampered = e.clone();
        tampered[8] ^= 1;
        assert!(Header::read(&mut &tampered[..], "nearby").is_err());
    }

    #[test]
    fn header_rejects_bad_magic_and_truncation() {
        let h = Header { name: "a".into(), size: 3, sha256: [0; 32] };
        let mut e = h.encode("s");
        assert!(Header::read(&mut &e[..e.len() - 1], "s").is_err());
        e[0] = b'Z';
        assert!(Header::read(&mut &e[..], "s").is_err());
    }

    #[test]
    fn names_are_sanitised() {
        assert_eq!(sanitize_name("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_name(".hidden"), "hidden");
        assert_eq!(sanitize_name("a\\b.txt"), "b.txt");
        assert_eq!(sanitize_name("  "), "Received");
        assert_eq!(sanitize_name("ok name.png"), "ok name.png");
    }

    fn scratch_dir(tag: &str) -> PathBuf {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("target").join(format!("share-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn create_unique_never_overwrites() {
        let dir = scratch_dir("unique");
        std::fs::write(dir.join("a.txt"), b"1").unwrap();
        assert_eq!(create_unique(&dir, "a.txt").unwrap().0, dir.join("a 2.txt"));
        assert_eq!(create_unique(&dir, "a.txt").unwrap().0, dir.join("a 3.txt"));
        assert_eq!(create_unique(&dir, "b").unwrap().0, dir.join("b"));
        assert_eq!(std::fs::read(dir.join("a.txt")).unwrap(), b"1");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn create_unique_gives_up_instead_of_looping_forever() {
        let dir = scratch_dir("exhaust");
        for i in 1..=MAX_UNIQUE_TRIES {
            let n = if i == 1 { "x".to_string() } else { format!("x {i}") };
            std::fs::write(dir.join(n), b"").unwrap();
        }
        assert_eq!(create_unique(&dir, "x").unwrap_err().kind(), io::ErrorKind::AlreadyExists);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn incoming_size_is_capped_and_needs_free_space() {
        assert!(check_incoming(1000, Some(1 << 30)).is_ok());
        assert!(check_incoming(1000, None).is_ok());
        assert!(check_incoming(MAX_FILE_SIZE + 1, None).is_err());
        assert_eq!(check_incoming(1 << 30, Some(1 << 30)).unwrap_err().kind(), io::ErrorKind::StorageFull);
    }

    #[test]
    fn part_names_are_unique_and_short() {
        let (a, b) = (part_name(), part_name());
        assert_ne!(a, b);
        assert!(a.len() < 64);
    }

    #[test]
    fn peers_expire() {
        let mut p = Peers::default();
        let t = Instant::now();
        let peer = Peer { name: "n".into(), addr: "10.0.0.2:5000".parse().unwrap() };
        p.heard(peer.clone(), t);
        p.heard(Peer { name: "renamed".into(), ..peer.clone() }, t);
        assert_eq!(p.live(t).len(), 1);
        assert_eq!(p.live(t)[0].name, "renamed");
        assert!(p.live(t + PEER_TTL + Duration::from_secs(1)).is_empty());
    }

    #[test]
    fn sizes_read_well() {
        assert_eq!(human_size(12), "12 B");
        assert_eq!(human_size(1_500_000), "1.4 MB");
        assert_eq!(human_size(820 * 1024), "820.0 KB");
    }

    #[test]
    fn loopback_transfer_accepts_and_verifies() {
        let dir = std::env::temp_dir().join(format!("share-loop-{}", std::process::id()));
        let svc = Service::start("test", "pw", dir.clone()).unwrap();
        let addr: SocketAddr = format!("127.0.0.1:{}", svc.port).parse().unwrap();
        let data: Vec<u8> = (0..200_000u32).map(|i| i as u8).collect();
        let d2 = data.clone();
        let t = std::thread::spawn(move || send(addr, "pw", "blob.bin", &mut io::Cursor::new(d2), |_, _| {}));
        match svc.events.recv_timeout(Duration::from_secs(5)).unwrap() {
            Event::Offer(o) => {
                assert_eq!((o.name.as_str(), o.size), ("blob.bin", 200_000));
                o.accept();
            }
            _ => panic!("expected an offer"),
        }
        t.join().unwrap().unwrap();
        assert_eq!(std::fs::read(dir.join("blob.bin")).unwrap(), data);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn tracker_counts_then_drops_finished_transfers() {
        let trk = Tracker::default();
        let t0 = Instant::now();
        let a = trk.begin(true, "Titan", "a.bin", 100);
        let b = trk.begin(false, "10.0.0.7", "b.bin", 50);
        trk.progress(a, 40, 100);
        trk.progress(b, 500, 50);
        let s = trk.snapshot(t0);
        assert_eq!(s.len(), 2);
        assert_eq!((s[0].done, s[0].total, s[0].status.clone()), (40, 100, Status::Running));
        assert_eq!(s[1].done, 50, "progress never passes the total");
        trk.finish_at(a, None, t0);
        trk.finish_at(b, Some("declined".into()), t0);
        let s = trk.snapshot(t0 + Duration::from_secs(1));
        assert_eq!(s[0].status, Status::Done);
        assert_eq!(s[0].done, 100);
        assert_eq!(s[1].status, Status::Failed("declined".into()));
        assert!(trk.snapshot(t0 + DONE_SHOW + Duration::from_millis(1)).is_empty(), "finished ones age out");
    }

    #[test]
    fn declined_transfer_errors() {
        let dir = std::env::temp_dir().join(format!("share-decl-{}", std::process::id()));
        let svc = Service::start("test", "pw", dir.clone()).unwrap();
        let addr: SocketAddr = format!("127.0.0.1:{}", svc.port).parse().unwrap();
        let t = std::thread::spawn(move || send(addr, "pw", "x.txt", &mut io::Cursor::new(b"hi".to_vec()), |_, _| {}));
        if let Event::Offer(o) = svc.events.recv_timeout(Duration::from_secs(5)).unwrap() {
            o.decline();
        }
        assert!(t.join().unwrap().is_err());
        assert!(!dir.join("x.txt").exists());
        std::fs::remove_dir_all(&dir).ok();
    }
}
