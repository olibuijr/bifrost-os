//! Minimal DNS queries for mail autoconfiguration: the SRV records of RFC 6186, MX records and address checks, over UDP.
//! The shell's own resolver (`springboard::deildu::dns`) answers A records only, so Mail keeps this small client for the
//! other record types. Name servers come from `/etc/resolv.conf`, then the default gateway when there are none. A public
//! resolver is never asked on Mail's own: the names of a person's mail domain are not sent to a third party unless the
//! system's own configuration says so.

use std::io::{self, Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const TYPE_A: u16 = 1;
pub const TYPE_MX: u16 = 15;
pub const TYPE_AAAA: u16 = 28;
pub const TYPE_SRV: u16 = 33;

/// How long one name server gets to answer.
const TIMEOUT: Duration = Duration::from_secs(2);
/// How long the servers count as out of reach after none of them answered in time.
const DOWN_FOR: Duration = Duration::from_secs(20);
/// Compression pointers followed in one name before the message is called malformed.
const MAX_JUMPS: usize = 16;

/// A service record (RFC 2782): where a service of a domain runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Srv {
    pub priority: u16,
    pub weight: u16,
    pub port: u16,
    pub target: String,
}

/// A mail exchanger record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mx {
    pub preference: u16,
    pub host: String,
}

/// One answer of a reply that Mail reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Record {
    Srv(Srv),
    Mx(Mx),
    Address(IpAddr),
}

/// A standard recursive query of `qtype` for `name`, with transaction id `id`.
pub fn query(id: u16, name: &str, qtype: u16) -> Vec<u8> {
    let mut q = Vec::with_capacity(32 + name.len());
    q.extend(id.to_be_bytes());
    q.extend([1, 0, 0, 1, 0, 0, 0, 0, 0, 0]);
    for label in name.trim_end_matches('.').split('.') {
        let bytes = &label.as_bytes()[..label.len().min(63)];
        q.push(bytes.len() as u8);
        q.extend(bytes);
    }
    q.push(0);
    q.extend(qtype.to_be_bytes());
    q.extend([0, 1]);
    q
}

/// Reads the (possibly compressed) name at `pos`: the dotted name and the offset after it in the record.
fn read_name(msg: &[u8], mut pos: usize) -> Option<(String, usize)> {
    let mut name = String::new();
    let mut after = None;
    let mut jumps = 0;
    loop {
        let len = *msg.get(pos)? as usize;
        if len == 0 {
            return Some((name, after.unwrap_or(pos + 1)));
        }
        if len & 0xC0 == 0xC0 {
            let low = *msg.get(pos + 1)? as usize;
            after.get_or_insert(pos + 2);
            jumps += 1;
            if jumps > MAX_JUMPS {
                return None;
            }
            pos = ((len & 0x3F) << 8) | low;
            continue;
        }
        if len & 0xC0 != 0 {
            return None;
        }
        let label = msg.get(pos + 1..pos + 1 + len)?;
        if !name.is_empty() {
            name.push('.');
        }
        name.push_str(&String::from_utf8_lossy(label));
        pos += 1 + len;
    }
}

/// The records of a reply to transaction `id`. `None` for a reply that is not an answer to this query or that the server
/// failed (so the next server is asked); a name that does not exist or has no such record is an empty list.
pub fn parse(msg: &[u8], id: u16) -> Option<Vec<Record>> {
    if msg.len() < 12 || msg[..2] != id.to_be_bytes() || msg[2] & 0x80 == 0 {
        return None;
    }
    match msg[3] & 0x0F {
        0 => {}
        3 => return Some(Vec::new()),
        _ => return None,
    }
    let (questions, count) = (u16::from_be_bytes([msg[4], msg[5]]), u16::from_be_bytes([msg[6], msg[7]]));
    let mut pos = 12;
    for _ in 0..questions {
        pos = read_name(msg, pos)?.1 + 4;
    }
    let mut out = Vec::new();
    for _ in 0..count {
        pos = read_name(msg, pos)?.1;
        let head = msg.get(pos..pos + 10)?;
        let (ty, class) = (u16::from_be_bytes([head[0], head[1]]), u16::from_be_bytes([head[2], head[3]]));
        let len = u16::from_be_bytes([head[8], head[9]]) as usize;
        let start = pos + 10;
        let data = msg.get(start..start + len)?;
        if class == 1 {
            match ty {
                TYPE_SRV if len >= 7 => {
                    let (target, _) = read_name(msg, start + 6)?;
                    out.push(Record::Srv(Srv {
                        priority: u16::from_be_bytes([data[0], data[1]]),
                        weight: u16::from_be_bytes([data[2], data[3]]),
                        port: u16::from_be_bytes([data[4], data[5]]),
                        target,
                    }));
                }
                TYPE_MX if len >= 3 => {
                    let (host, _) = read_name(msg, start + 2)?;
                    out.push(Record::Mx(Mx { preference: u16::from_be_bytes([data[0], data[1]]), host }));
                }
                TYPE_A if len == 4 => out.push(Record::Address(IpAddr::V4(Ipv4Addr::new(data[0], data[1], data[2], data[3])))),
                TYPE_AAAA if len == 16 => {
                    let mut octets = [0u8; 16];
                    octets.copy_from_slice(data);
                    out.push(Record::Address(IpAddr::from(octets)));
                }
                _ => {}
            }
        }
        pos = start + len;
    }
    Some(out)
}

/// The usable service records in the order a client tries them (RFC 2782): lowest priority first, then the heaviest.
/// A target of `.` says the service is not offered, and is dropped.
pub fn order_srv(mut records: Vec<Srv>) -> Vec<Srv> {
    records.retain(|r| !r.target.is_empty() && r.target != "." && r.port != 0);
    records.sort_by(|a, b| a.priority.cmp(&b.priority).then(b.weight.cmp(&a.weight)));
    records
}

/// The mail exchanger hosts, the most preferred first.
pub fn order_mx(mut records: Vec<Mx>) -> Vec<String> {
    records.retain(|r| !r.host.is_empty() && r.host != ".");
    records.sort_by_key(|r| r.preference);
    records.into_iter().map(|r| r.host).collect()
}

/// The name servers a `resolv.conf` lists, in order. Comments and other keywords are skipped.
pub fn nameservers(resolv_conf: &str) -> Vec<IpAddr> {
    resolv_conf
        .lines()
        .filter_map(|line| {
            let line = line.split(['#', ';']).next().unwrap_or("").trim();
            let mut words = line.split_ascii_whitespace();
            if words.next()? != "nameserver" {
                return None;
            }
            // a zone suffix (`fe80::1%eth0`) is not part of the address
            words.next()?.split('%').next()?.parse().ok()
        })
        .collect()
}

/// A resolver over UDP with the servers it asks, in order.
pub struct Resolver {
    servers: Vec<SocketAddr>,
    /// When every server last failed to answer in time: until [`DOWN_FOR`] has passed, later queries fail at once, so a
    /// machine without a network does not make the person wait for every record of the discovery. A server that answers
    /// with an error (SERVFAIL, REFUSED) does not count: it is up, and other questions may work.
    down_since: Mutex<Option<Instant>>,
}

impl Resolver {
    pub fn new(servers: Vec<SocketAddr>) -> Resolver {
        Resolver { servers, down_since: Mutex::new(None) }
    }

    /// The resolver of this computer: the servers of `/etc/resolv.conf`, or, when it lists none, the default gateway.
    /// The shell's public resolvers are not used.
    pub fn system() -> Resolver {
        let mut ips = std::fs::read_to_string("/etc/resolv.conf").map(|t| nameservers(&t)).unwrap_or_default();
        if ips.is_empty() {
            ips = springboard::deildu::dns::servers().into_iter().filter(|ip| crate::transport::is_local(*ip)).map(IpAddr::V4).collect();
        }
        Resolver::new(ips.into_iter().map(|ip| SocketAddr::new(ip, 53)).collect())
    }

    /// True while the servers are counted as out of reach.
    pub fn is_down(&self) -> bool {
        let mut down = self.down_since.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        match *down {
            Some(since) if since.elapsed() < DOWN_FOR => true,
            Some(_) => {
                *down = None;
                false
            }
            None => false,
        }
    }

    /// The records of `qtype` for `name`. An empty list when the name has none; an error when no server answered.
    pub fn lookup(&self, name: &str, qtype: u16) -> io::Result<Vec<Record>> {
        if self.is_down() {
            return Err(io::Error::other("no name server answered"));
        }
        let mut silent = true;
        for server in &self.servers {
            match ask(*server, name, qtype) {
                Ok(records) => return Ok(records),
                Err(e) => silent &= is_silence(&e),
            }
        }
        if silent {
            *self.down_since.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Instant::now());
        }
        Err(io::Error::other("no name server answered"))
    }

    /// True when `host` has an address.
    pub fn resolves(&self, host: &str) -> bool {
        if host.parse::<IpAddr>().is_ok() {
            return true;
        }
        match self.lookup(host, TYPE_A) {
            Ok(a) if !a.is_empty() => true,
            Ok(_) => self.lookup(host, TYPE_AAAA).is_ok_and(|a| !a.is_empty()),
            // the name servers are out of reach: the system's own resolver may still know the name
            Err(_) => std::net::ToSocketAddrs::to_socket_addrs(&(host, 0)).is_ok_and(|mut it| it.next().is_some()),
        }
    }
}

static NEXT_ID: AtomicU16 = AtomicU16::new(0);

/// True for the errors of a server that said nothing: a timeout, or a refused connection.
fn is_silence(e: &io::Error) -> bool {
    matches!(e.kind(), io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock | io::ErrorKind::ConnectionRefused)
}

/// A transaction id the operating system's random source chose, so a forged reply has to guess it. The clock mix is only
/// the fallback when the source cannot be read.
fn transaction_id() -> u16 {
    let mut bytes = [0u8; 2];
    if std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut bytes)).is_ok() {
        return u16::from_be_bytes(bytes);
    }
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.subsec_nanos());
    (nanos ^ std::process::id()) as u16 ^ NEXT_ID.fetch_add(0x9e37, Ordering::Relaxed)
}

/// Asks one server; an error for a timeout or a reply that is not an answer.
fn ask(server: SocketAddr, name: &str, qtype: u16) -> io::Result<Vec<Record>> {
    let sock = UdpSocket::bind(if server.is_ipv4() { "0.0.0.0:0" } else { "[::]:0" })?;
    sock.set_read_timeout(Some(TIMEOUT))?;
    sock.connect(server)?;
    let id = transaction_id();
    sock.send(&query(id, name, qtype))?;
    let mut buf = [0u8; 4096];
    let n = sock.recv(&mut buf)?;
    // the reply was cut to fit one datagram (the TC bit): ask again over TCP, which carries all of it
    if n > 2 && buf[2] & 0x02 != 0 {
        return ask_tcp(server, name, qtype, id);
    }
    parse(&buf[..n], id).ok_or_else(|| io::Error::other("bad reply"))
}

/// Asks one server over TCP (RFC 7766): each message is preceded by its length in two bytes.
fn ask_tcp(server: SocketAddr, name: &str, qtype: u16, id: u16) -> io::Result<Vec<Record>> {
    let mut stream = TcpStream::connect_timeout(&server, TIMEOUT)?;
    stream.set_read_timeout(Some(TIMEOUT))?;
    stream.set_write_timeout(Some(TIMEOUT))?;
    let q = query(id, name, qtype);
    let mut out = (q.len() as u16).to_be_bytes().to_vec();
    out.extend(q);
    stream.write_all(&out)?;
    let mut len = [0u8; 2];
    stream.read_exact(&mut len)?;
    let mut msg = vec![0u8; u16::from_be_bytes(len) as usize];
    stream.read_exact(&mut msg)?;
    parse(&msg, id).ok_or_else(|| io::Error::other("bad reply"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A reply to `query(id, name, qtype)` with `answers` (type, rdata), each named by a pointer to the question.
    fn reply(id: u16, name: &str, qtype: u16, answers: &[(u16, Vec<u8>)]) -> Vec<u8> {
        let mut r = query(id, name, qtype);
        r[2] = 0x81;
        r[3] = 0x80;
        r[7] = answers.len() as u8;
        for (ty, data) in answers {
            r.extend([0xC0, 12]);
            r.extend(ty.to_be_bytes());
            r.extend([0, 1, 0, 0, 0x0e, 0x10]);
            r.extend((data.len() as u16).to_be_bytes());
            r.extend(data);
        }
        r
    }

    fn name(labels: &[&str]) -> Vec<u8> {
        let mut out = Vec::new();
        for l in labels {
            out.push(l.len() as u8);
            out.extend(l.as_bytes());
        }
        out.push(0);
        out
    }

    fn srv_data(priority: u16, weight: u16, port: u16, target: &[&str]) -> Vec<u8> {
        let mut d = Vec::new();
        d.extend(priority.to_be_bytes());
        d.extend(weight.to_be_bytes());
        d.extend(port.to_be_bytes());
        d.extend(name(target));
        d
    }

    #[test]
    fn a_query_encodes_the_labels_and_the_type() {
        let q = query(7, "_imaps._tcp.example.net", TYPE_SRV);
        assert_eq!(&q[..4], &[0, 7, 1, 0]);
        assert_eq!(&q[12..], b"\x06_imaps\x04_tcp\x07example\x03net\0\0\x21\0\x01");
    }

    #[test]
    fn srv_records_parse_with_their_target() {
        let r = reply(9, "_imaps._tcp.example.net", TYPE_SRV, &[(TYPE_SRV, srv_data(10, 5, 993, &["mail", "example", "net"]))]);
        assert_eq!(
            parse(&r, 9),
            Some(vec![Record::Srv(Srv { priority: 10, weight: 5, port: 993, target: "mail.example.net".into() })])
        );
    }

    #[test]
    fn mx_records_and_compressed_targets_parse() {
        // the MX host is a pointer back to the question name, as servers compress it
        let mut data = 5u16.to_be_bytes().to_vec();
        data.extend([0xC0, 12]);
        let r = reply(3, "example.net", TYPE_MX, &[(TYPE_MX, data)]);
        assert_eq!(parse(&r, 3), Some(vec![Record::Mx(Mx { preference: 5, host: "example.net".into() })]));
    }

    #[test]
    fn address_records_parse_and_other_types_are_skipped() {
        let r = reply(4, "example.net", TYPE_A, &[(5, name(&["x"])), (TYPE_A, vec![10, 0, 0, 1])]);
        assert_eq!(parse(&r, 4), Some(vec![Record::Address(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)))]));
        let v6 = reply(4, "example.net", TYPE_AAAA, &[(TYPE_AAAA, vec![0; 16])]);
        assert_eq!(parse(&v6, 4), Some(vec![Record::Address(IpAddr::from([0u8; 16]))]));
    }

    #[test]
    fn replies_that_are_not_answers_are_refused() {
        let r = reply(9, "a.b", TYPE_A, &[(TYPE_A, vec![1, 2, 3, 4])]);
        assert!(parse(&r, 8).is_none(), "wrong transaction");
        assert!(parse(&r[..20], 9).is_none(), "truncated");
        let mut query_bit = r.clone();
        query_bit[2] = 0x01;
        assert!(parse(&query_bit, 9).is_none(), "not a response");
        let mut fail = r.clone();
        fail[3] = 0x82;
        assert!(parse(&fail, 9).is_none(), "server failure");
        let mut nx = r.clone();
        nx[3] = 0x83;
        assert_eq!(parse(&nx, 9), Some(vec![]), "no such name is an empty answer");
    }

    #[test]
    fn a_pointer_loop_is_malformed_not_a_hang() {
        let mut r = reply(9, "a.b", TYPE_SRV, &[(TYPE_SRV, srv_data(1, 1, 1, &["x"]))]);
        let at = r.len() - 3;
        // the target now points at itself
        r[at..].copy_from_slice(&[0xC0, at as u8, 0]);
        assert!(parse(&r, 9).is_none());
    }

    #[test]
    fn srv_records_are_tried_by_priority_then_weight() {
        let srv = |priority, weight, target: &str| Srv { priority, weight, port: 993, target: target.into() };
        let ordered = order_srv(vec![srv(20, 1, "c"), srv(10, 1, "b"), srv(10, 9, "a"), srv(5, 0, "."), srv(5, 0, "")]);
        assert_eq!(ordered.iter().map(|s| s.target.as_str()).collect::<Vec<_>>(), ["a", "b", "c"]);
    }

    #[test]
    fn mail_exchangers_are_ordered_by_preference() {
        let mx = |preference, host: &str| Mx { preference, host: host.into() };
        assert_eq!(order_mx(vec![mx(20, "b.example"), mx(10, "a.example"), mx(0, ".")]), ["a.example", "b.example"]);
    }

    #[test]
    fn nameservers_come_from_resolv_conf() {
        let text = "# generated\nsearch lan\nnameserver 192.168.1.1\nnameserver fe80::1%eth0 # local\n;nameserver 9.9.9.9\nnameserver nonsense\n";
        let ips = nameservers(text);
        assert_eq!(ips, vec!["192.168.1.1".parse::<IpAddr>().unwrap(), "fe80::1".parse::<IpAddr>().unwrap()]);
    }

    #[test]
    fn a_server_failure_does_not_disable_the_later_lookups() {
        let server = UdpSocket::bind("127.0.0.1:0").unwrap();
        let addr = server.local_addr().unwrap();
        let t = std::thread::spawn(move || {
            let mut buf = [0u8; 512];
            let (n, from) = server.recv_from(&mut buf).unwrap();
            let mut answer = buf[..n].to_vec();
            answer[2] = 0x81;
            answer[3] = 0x82; // SERVFAIL
            server.send_to(&answer, from).unwrap();
        });
        let resolver = Resolver::new(vec![addr]);
        assert!(resolver.lookup("_imaps._tcp.example.net", TYPE_SRV).is_err());
        t.join().unwrap();
        assert!(!resolver.is_down(), "a server that answered with an error is up");
    }

    #[test]
    fn the_down_state_ends_after_a_while() {
        let resolver = Resolver::new(vec![]);
        assert!(resolver.lookup("example.net", TYPE_A).is_err());
        assert!(resolver.is_down());
        // Instant cannot go back before boot: right after start-up there is nothing to age.
        let Some(past) = Instant::now().checked_sub(DOWN_FOR + Duration::from_secs(1)) else { return };
        *resolver.down_since.lock().unwrap() = Some(past);
        assert!(!resolver.is_down(), "the servers are tried again");
    }

    #[test]
    fn a_resolver_asks_a_server_over_udp_and_gives_up_when_none_answers() {
        let server = UdpSocket::bind("127.0.0.1:0").unwrap();
        let addr = server.local_addr().unwrap();
        let t = std::thread::spawn(move || {
            let mut buf = [0u8; 512];
            let (n, from) = server.recv_from(&mut buf).unwrap();
            let id = u16::from_be_bytes([buf[0], buf[1]]);
            assert_eq!(&buf[12..n], &query(id, "_imaps._tcp.example.net", TYPE_SRV)[12..]);
            let answer = reply(id, "_imaps._tcp.example.net", TYPE_SRV, &[(TYPE_SRV, srv_data(0, 1, 993, &["mail", "example", "net"]))]);
            server.send_to(&answer, from).unwrap();
        });
        let resolver = Resolver::new(vec![addr]);
        let found = resolver.lookup("_imaps._tcp.example.net", TYPE_SRV).unwrap();
        assert_eq!(found, vec![Record::Srv(Srv { priority: 0, weight: 1, port: 993, target: "mail.example.net".into() })]);
        t.join().unwrap();

        let none = Resolver::new(vec![]);
        assert!(none.lookup("example.net", TYPE_A).is_err());
        assert!(none.is_down(), "after a silent failure the later lookups fail at once");
        assert!(none.resolves("10.1.2.3"), "an address needs no lookup");
    }
}
