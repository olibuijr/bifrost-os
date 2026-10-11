//! DHCPv4 client: DISCOVER/OFFER/REQUEST/ACK with the broadcast flag set,
//! so replies arrive before the interface has an address.

use std::io;
use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use std::time::{Duration, Instant};

const MAGIC: [u8; 4] = [99, 130, 83, 99];
pub const DISCOVER: u8 = 1;
pub const OFFER: u8 = 2;
pub const REQUEST: u8 = 3;
pub const ACK: u8 = 5;
pub const NAK: u8 = 6;

#[derive(Clone, Debug, PartialEq)]
pub struct Lease {
    pub ip: Ipv4Addr,
    pub prefix: u8,
    pub router: Option<Ipv4Addr>,
    pub dns: Vec<Ipv4Addr>,
    pub server: Ipv4Addr,
    pub seconds: u32,
}

/// Builds a client message. `requested`/`server` are set for REQUEST.
pub fn build(kind: u8, xid: u32, mac: [u8; 6], requested: Option<Ipv4Addr>, server: Option<Ipv4Addr>, hostname: &str) -> Vec<u8> {
    let mut p = vec![0u8; 236];
    p[0] = 1; // BOOTREQUEST
    p[1] = 1; // ethernet
    p[2] = 6;
    p[4..8].copy_from_slice(&xid.to_be_bytes());
    p[10] = 0x80; // broadcast flag
    p[28..34].copy_from_slice(&mac);
    p.extend(MAGIC);
    p.extend([53, 1, kind]);
    let mut cid = vec![61, 7, 1];
    cid.extend(mac);
    p.extend(cid);
    if let Some(ip) = requested {
        p.extend([50, 4]);
        p.extend(ip.octets());
    }
    if let Some(s) = server {
        p.extend([54, 4]);
        p.extend(s.octets());
    }
    let name = &hostname.as_bytes()[..hostname.len().min(63)];
    p.extend([12, name.len() as u8]);
    p.extend(name);
    p.extend([55, 4, 1, 3, 6, 51]);
    p.push(255);
    p.resize(p.len().max(300), 0);
    p
}

/// Parses a server reply for `xid` addressed to `mac`: (message type, lease).
/// OFFER and ACK must name their server (option 54), or they are rejected.
pub fn parse(p: &[u8], xid: u32, mac: [u8; 6]) -> Option<(u8, Lease)> {
    if p.len() < 240 || p[0] != 2 || p[4..8] != xid.to_be_bytes() || p[28..34] != mac || p[236..240] != MAGIC {
        return None;
    }
    let ip = Ipv4Addr::new(p[16], p[17], p[18], p[19]);
    let (mut kind, mut prefix, mut router, mut dns, mut server, mut seconds) = (0, 24, None, vec![], None, 3600);
    let mut i = 240;
    while i < p.len() {
        let code = p[i];
        if code == 255 {
            break;
        }
        if code == 0 {
            i += 1;
            continue;
        }
        let len = *p.get(i + 1)? as usize;
        let v = p.get(i + 2..i + 2 + len)?;
        let addr = |o: usize| v.get(o..o + 4).map(|b| Ipv4Addr::new(b[0], b[1], b[2], b[3]));
        match code {
            53 => kind = *v.first()?,
            1 => prefix = u32::from(addr(0)?).count_ones() as u8,
            3 => router = addr(0),
            6 => dns = (0..len / 4).filter_map(|k| addr(4 * k)).collect(),
            51 => seconds = u32::from_be_bytes(v.get(..4)?.try_into().ok()?),
            54 => server = Some(addr(0)?),
            _ => {}
        }
        i += 2 + len;
    }
    let server = match (server, kind) {
        (Some(s), _) => s,
        (None, OFFER | ACK) => return None,
        (None, _) => Ipv4Addr::UNSPECIFIED,
    };
    Some((kind, Lease { ip, prefix, router, dns, server, seconds }))
}

extern "C" {
    fn setsockopt(fd: i32, level: i32, name: i32, val: *const std::ffi::c_void, len: u32) -> i32;
}

fn bind_to_device(sock: &UdpSocket, dev: &str) -> io::Result<()> {
    use std::os::fd::AsRawFd;
    const SOL_SOCKET: i32 = 1;
    const SO_BINDTODEVICE: i32 = 25;
    let name = format!("{dev}\0");
    // SAFETY: valid fd and a NUL-terminated buffer of the stated length.
    if unsafe { setsockopt(sock.as_raw_fd(), SOL_SOCKET, SO_BINDTODEVICE, name.as_ptr().cast(), name.len() as u32) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Runs a full exchange on `dev`; `prefer` asks for the previous address.
pub fn acquire(dev: &str, mac: [u8; 6], prefer: Option<Ipv4Addr>, hostname: &str, timeout: Duration) -> io::Result<Lease> {
    let sock = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 68))?;
    sock.set_broadcast(true)?;
    bind_to_device(&sock, dev)?;
    sock.set_read_timeout(Some(Duration::from_millis(700)))?;
    let to = SocketAddrV4::new(Ipv4Addr::BROADCAST, 67);
    let xid = (crate::bare::sys::now_ms() as u64 as u32) ^ u32::from_be_bytes([mac[2], mac[3], mac[4], mac[5]]);
    let deadline = Instant::now() + timeout;
    let mut buf = [0u8; 1500];
    let mut offer: Option<Lease> = None;
    while Instant::now() < deadline {
        let msg = match &offer {
            None => build(DISCOVER, xid, mac, prefer, None, hostname),
            Some(o) => build(REQUEST, xid, mac, Some(o.ip), Some(o.server), hostname),
        };
        sock.send_to(&msg, to)?;
        let wait_until = Instant::now() + Duration::from_secs(2);
        while Instant::now() < wait_until {
            let n = match sock.recv_from(&mut buf) {
                Ok((n, _)) => n,
                Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted) => continue,
                Err(e) => return Err(e),
            };
            match parse(&buf[..n], xid, mac) {
                Some((OFFER, l)) if offer.is_none() => {
                    offer = Some(l);
                    break;
                }
                Some((ACK, l)) if offer.is_some() => return Ok(l),
                Some((NAK, _)) => {
                    offer = None;
                    break;
                }
                _ => {}
            }
        }
    }
    Err(io::Error::new(io::ErrorKind::TimedOut, "no DHCP lease"))
}

/// Network address for `ip/prefix`.
pub fn network(ip: Ipv4Addr, prefix: u8) -> Ipv4Addr {
    let mask = if prefix == 0 { 0 } else { u32::MAX << (32 - prefix.min(32)) };
    Ipv4Addr::from(u32::from(ip) & mask)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAC: [u8; 6] = [0x9e, 0xb8, 0x73, 0xb2, 0x65, 0x13];

    fn reply(kind: u8, xid: u32) -> Vec<u8> {
        let mut p = vec![0u8; 236];
        p[0] = 2;
        p[4..8].copy_from_slice(&xid.to_be_bytes());
        p[16..20].copy_from_slice(&[192, 168, 1, 171]);
        p[28..34].copy_from_slice(&MAC);
        p.extend(MAGIC);
        p.extend([53, 1, kind, 1, 4, 255, 255, 255, 0, 3, 4, 192, 168, 1, 1, 6, 8, 1, 1, 1, 1, 8, 8, 8, 8]);
        p.extend([51, 4, 0, 0, 0x0e, 0x10, 54, 4, 192, 168, 1, 1, 255]);
        p
    }

    #[test]
    fn discover_layout() {
        let p = build(DISCOVER, 0xdeadbeef, MAC, Some(Ipv4Addr::new(192, 168, 1, 171)), None, "tab");
        assert_eq!(p[0], 1);
        assert_eq!(&p[4..8], &0xdeadbeefu32.to_be_bytes());
        assert_eq!(p[10], 0x80, "broadcast flag");
        assert_eq!(&p[28..34], &MAC);
        assert_eq!(&p[236..240], &MAGIC);
        assert_eq!(&p[240..243], &[53, 1, DISCOVER]);
        assert!(p.windows(6).any(|w| w == [50, 4, 192, 168, 1, 171]));
        assert!(p.len() >= 300);
    }

    #[test]
    fn request_names_server() {
        let p = build(REQUEST, 1, MAC, Some(Ipv4Addr::new(10, 0, 0, 5)), Some(Ipv4Addr::new(10, 0, 0, 1)), "tab");
        assert!(p.windows(6).any(|w| w == [54, 4, 10, 0, 0, 1]));
        assert!(p.windows(3).any(|w| w == [53, 1, REQUEST]));
    }

    #[test]
    fn parses_ack() {
        let (kind, l) = parse(&reply(ACK, 7), 7, MAC).unwrap();
        assert_eq!(kind, ACK);
        assert_eq!(l.ip, Ipv4Addr::new(192, 168, 1, 171));
        assert_eq!(l.prefix, 24);
        assert_eq!(l.router, Some(Ipv4Addr::new(192, 168, 1, 1)));
        assert_eq!(l.dns, vec![Ipv4Addr::new(1, 1, 1, 1), Ipv4Addr::new(8, 8, 8, 8)]);
        assert_eq!(l.seconds, 3600);
        assert_eq!(l.server, Ipv4Addr::new(192, 168, 1, 1));
    }

    #[test]
    fn rejects_foreign_or_truncated_replies() {
        assert!(parse(&reply(ACK, 7), 8, MAC).is_none());
        assert!(parse(&reply(ACK, 7)[..200], 7, MAC).is_none());
        let mut bad = reply(ACK, 7);
        bad.truncate(bad.len() - 6); // option runs past the end
        assert!(parse(&bad, 7, MAC).is_none());
    }

    #[test]
    fn rejects_replies_for_another_client() {
        assert!(parse(&reply(ACK, 7), 7, [1, 2, 3, 4, 5, 6]).is_none());
    }

    #[test]
    fn offer_or_ack_without_server_identifier_is_rejected() {
        let mut p = reply(OFFER, 7);
        let n = p.len();
        p.truncate(n - 7); // drop option 54 (6 bytes) and the end marker
        p.push(255);
        assert!(parse(&p, 7, MAC).is_none());
        let mut nak = reply(NAK, 7);
        let n = nak.len();
        nak.truncate(n - 7);
        nak.push(255);
        assert_eq!(parse(&nak, 7, MAC).map(|(k, _)| k), Some(NAK));
    }

    #[test]
    fn network_masks() {
        assert_eq!(network(Ipv4Addr::new(192, 168, 1, 171), 24), Ipv4Addr::new(192, 168, 1, 0));
        assert_eq!(network(Ipv4Addr::new(10, 1, 2, 3), 8), Ipv4Addr::new(10, 0, 0, 0));
        assert_eq!(network(Ipv4Addr::new(10, 1, 2, 3), 0), Ipv4Addr::UNSPECIFIED);
    }
}
