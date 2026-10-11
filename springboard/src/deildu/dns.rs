//! A-record lookups over UDP. The system resolver may be unavailable under
//! SpringBoard, so names are resolved here: the default
//! gateway (the DHCP router forwards DNS) first, then public resolvers.

use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::Mutex;
use std::time::{Duration, Instant};

const FALLBACK: [Ipv4Addr; 2] = [Ipv4Addr::new(1, 1, 1, 1), Ipv4Addr::new(8, 8, 8, 8)];
const TIMEOUT: Duration = Duration::from_secs(3);
const MIN_TTL: Duration = Duration::from_secs(60);

/// Encodes a standard recursive A query with transaction id `id`.
pub fn query(id: u16, name: &str) -> Vec<u8> {
    let mut q = Vec::with_capacity(32 + name.len());
    q.extend(id.to_be_bytes());
    q.extend([1, 0, 0, 1, 0, 0, 0, 0, 0, 0]);
    for label in name.trim_end_matches('.').split('.') {
        q.push(label.len().min(63) as u8);
        q.extend(&label.as_bytes()[..label.len().min(63)]);
    }
    q.extend([0, 0, 1, 0, 1]);
    q
}

/// Skips a (possibly compressed) name starting at `pos`; returns the offset after it.
fn skip_name(msg: &[u8], mut pos: usize) -> Option<usize> {
    loop {
        let len = *msg.get(pos)? as usize;
        if len == 0 {
            return Some(pos + 1);
        }
        if len & 0xC0 == 0xC0 {
            return Some(pos + 2);
        }
        pos += 1 + len;
    }
}

/// Addresses and the smallest TTL from a reply to transaction `id`.
pub fn answers(msg: &[u8], id: u16) -> Option<(Vec<Ipv4Addr>, Duration)> {
    if msg.len() < 12 || msg[..2] != id.to_be_bytes() || msg[2] & 0x80 == 0 || msg[3] & 0x0F != 0 {
        return None;
    }
    let (questions, count) = (u16::from_be_bytes([msg[4], msg[5]]), u16::from_be_bytes([msg[6], msg[7]]));
    let mut pos = 12;
    for _ in 0..questions {
        pos = skip_name(msg, pos)? + 4;
    }
    let mut out = vec![];
    let mut ttl = u32::MAX;
    for _ in 0..count {
        pos = skip_name(msg, pos)?;
        let rr = msg.get(pos..pos + 10)?;
        let (ty, class) = (u16::from_be_bytes([rr[0], rr[1]]), u16::from_be_bytes([rr[2], rr[3]]));
        let len = u16::from_be_bytes([rr[8], rr[9]]) as usize;
        let data = msg.get(pos + 10..pos + 10 + len)?;
        if ty == 1 && class == 1 && len == 4 {
            out.push(Ipv4Addr::new(data[0], data[1], data[2], data[3]));
            ttl = ttl.min(u32::from_be_bytes([rr[4], rr[5], rr[6], rr[7]]));
        }
        pos += 10 + len;
    }
    (!out.is_empty()).then(|| (out, Duration::from_secs(ttl as u64).max(MIN_TTL)))
}

/// The default route's gateway from /proc/net/route (little-endian hex).
pub fn gateway_from(route_table: &str) -> Option<Ipv4Addr> {
    route_table.lines().skip(1).find_map(|l| {
        let f: Vec<&str> = l.split_whitespace().collect();
        if f.len() < 4 || f[1] != "00000000" || u32::from_str_radix(f[3], 16).ok()? & 2 == 0 {
            return None;
        }
        Some(Ipv4Addr::from(u32::from_str_radix(f[2], 16).ok()?.swap_bytes()))
    })
}

/// Resolvers in the order to try.
pub fn servers() -> Vec<Ipv4Addr> {
    let gw = std::fs::read_to_string("/proc/net/route").ok().and_then(|t| gateway_from(&t));
    gw.into_iter().chain(FALLBACK).collect()
}

/// Asks one server; `Err` on timeout or a malformed reply.
pub fn ask(server: SocketAddr, name: &str) -> std::io::Result<(Vec<Ipv4Addr>, Duration)> {
    let sock = UdpSocket::bind("0.0.0.0:0")?;
    sock.set_read_timeout(Some(TIMEOUT))?;
    sock.connect(server)?;
    let id = (Instant::now().elapsed().subsec_nanos() ^ std::process::id()) as u16 ^ 0x5a17;
    sock.send(&query(id, name))?;
    let mut buf = [0u8; 2048];
    let n = sock.recv(&mut buf)?;
    answers(&buf[..n], id).ok_or_else(|| std::io::Error::other("no A record"))
}

/// Most names kept at once; a long session cannot grow the cache without bound.
const CACHE_MAX: usize = 256;

/// Answers with their expiry, bounded: expired entries go first, then the
/// ones expiring soonest.
#[derive(Default)]
struct Cache {
    entries: HashMap<String, (Vec<Ipv4Addr>, Instant)>,
}

impl Cache {
    fn get(&self, host: &str, now: Instant) -> Option<Vec<Ipv4Addr>> {
        self.entries.get(host).filter(|(_, until)| now < *until).map(|(ips, _)| ips.clone())
    }

    fn put(&mut self, host: &str, ips: Vec<Ipv4Addr>, until: Instant, now: Instant) {
        if self.entries.len() >= CACHE_MAX && !self.entries.contains_key(host) {
            self.entries.retain(|_, (_, u)| now < *u);
        }
        while self.entries.len() >= CACHE_MAX && !self.entries.contains_key(host) {
            let Some(soonest) = self.entries.iter().min_by_key(|(_, (_, u))| *u).map(|(k, _)| k.clone()) else { break };
            self.entries.remove(&soonest);
        }
        self.entries.insert(host.to_string(), (ips, until));
    }
}

/// Caching resolver.
#[derive(Default)]
pub struct Resolver {
    cache: Mutex<Cache>,
}

impl Resolver {
    /// The cache stays usable even if a thread panicked while holding it.
    fn cache(&self) -> std::sync::MutexGuard<'_, Cache> {
        self.cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl Resolver {
    pub fn resolve(&self, host: &str) -> std::io::Result<Vec<Ipv4Addr>> {
        if let Ok(ip) = host.parse::<Ipv4Addr>() {
            return Ok(vec![ip]);
        }
        if let Some(ips) = self.cache().get(host, Instant::now()) {
            return Ok(ips);
        }
        let mut err = std::io::Error::other("no resolver reachable");
        for s in servers() {
            match ask(SocketAddr::from((s, 53)), host) {
                Ok((ips, ttl)) => {
                    let now = Instant::now();
                    self.cache().put(host, ips.clone(), now + ttl, now);
                    return Ok(ips);
                }
                Err(e) => err = e,
            }
        }
        // the libc resolver still works on a desktop host
        if let Ok(it) = std::net::ToSocketAddrs::to_socket_addrs(&(host, 0)) {
            let ips: Vec<Ipv4Addr> = it.filter_map(|a| match a { SocketAddr::V4(v) => Some(*v.ip()), _ => None }).collect();
            if !ips.is_empty() {
                return Ok(ips);
            }
        }
        Err(err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cache_is_bounded_and_drops_expired_entries_first() {
        let now = Instant::now();
        let ip = vec![Ipv4Addr::LOCALHOST];
        let mut c = Cache::default();
        c.put("stale", ip.clone(), now, now);
        for i in 0..CACHE_MAX * 2 {
            c.put(&format!("h{i}"), ip.clone(), now + Duration::from_secs(60 + i as u64), now);
        }
        assert!(c.entries.len() <= CACHE_MAX);
        assert!(c.get("stale", now).is_none());
        let last = format!("h{}", CACHE_MAX * 2 - 1);
        assert_eq!(c.get(&last, now), Some(ip), "the newest, longest-lived entry survives");
    }

    #[test]
    fn a_poisoned_cache_lock_is_recovered() {
        let r = std::sync::Arc::new(Resolver::default());
        let r2 = r.clone();
        let _ = std::thread::spawn(move || {
            let _g = r2.cache.lock().unwrap();
            panic!("poison");
        })
        .join();
        assert!(r.cache().get("x", Instant::now()).is_none());
        assert_eq!(r.resolve("127.0.0.1").unwrap(), vec![Ipv4Addr::LOCALHOST]);
    }

    fn reply(id: u16, name: &str, ips: &[[u8; 4]], ttl: u32) -> Vec<u8> {
        let mut r = query(id, name);
        r[2] = 0x81;
        r[3] = 0x80;
        r[7] = ips.len() as u8;
        for ip in ips {
            r.extend([0xC0, 12, 0, 1, 0, 1]);
            r.extend(ttl.to_be_bytes());
            r.extend([0, 4]);
            r.extend(ip);
        }
        r
    }

    #[test]
    fn query_encodes_labels() {
        let q = query(7, "www.example.net");
        assert_eq!(&q[..4], &[0, 7, 1, 0]);
        assert_eq!(&q[12..], b"\x03www\x07example\x03net\0\0\x01\0\x01");
    }

    #[test]
    fn parses_compressed_answers_and_min_ttl() {
        let r = reply(9, "a.b", &[[10, 0, 0, 1], [10, 0, 0, 2]], 30);
        let (ips, ttl) = answers(&r, 9).unwrap();
        assert_eq!(ips, vec![Ipv4Addr::new(10, 0, 0, 1), Ipv4Addr::new(10, 0, 0, 2)]);
        assert_eq!(ttl, MIN_TTL, "short TTLs are stretched");
        assert!(answers(&r, 8).is_none(), "wrong id");
        let mut nx = r.clone();
        nx[3] = 0x83;
        assert!(answers(&nx, 9).is_none(), "NXDOMAIN");
        assert!(answers(&r[..20], 9).is_none(), "truncated");
    }

    #[test]
    fn gateway_comes_from_the_default_route() {
        let table = "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\nwlan0\t00000000\t0101A8C0\t0003\t0\t0\t0\t00000000\nwlan0\t0001A8C0\t00000000\t0001\t0\t0\t0\t00FFFFFF\n";
        assert_eq!(gateway_from(table), Some(Ipv4Addr::new(192, 168, 1, 1)));
        assert_eq!(gateway_from("Iface\n"), None);
    }

    #[test]
    fn asks_a_server_over_udp() {
        let server = UdpSocket::bind("127.0.0.1:0").unwrap();
        let addr = server.local_addr().unwrap();
        let t = std::thread::spawn(move || {
            let mut buf = [0u8; 512];
            let (n, from) = server.recv_from(&mut buf).unwrap();
            let id = u16::from_be_bytes([buf[0], buf[1]]);
            assert_eq!(&buf[12..n], &query(id, "host.test")[12..]);
            server.send_to(&reply(id, "host.test", &[[1, 2, 3, 4]], 3600), from).unwrap();
        });
        let (ips, ttl) = ask(addr, "host.test").unwrap();
        assert_eq!(ips, vec![Ipv4Addr::new(1, 2, 3, 4)]);
        assert_eq!(ttl, Duration::from_secs(3600));
        t.join().unwrap();
        let r = Resolver::default();
        assert_eq!(r.resolve("9.9.9.9").unwrap(), vec![Ipv4Addr::new(9, 9, 9, 9)]);
    }
}
