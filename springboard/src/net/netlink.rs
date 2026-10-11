//! rtnetlink: bring links up, set MAC, add/flush IPv4 addresses, routes
//! and policy rules. Message builders are pure and unit-tested.

use std::io;
use std::net::Ipv4Addr;

const NETLINK_ROUTE: i32 = 0;
const AF_NETLINK: i32 = 16;
const AF_INET: u8 = 2;
const AF_UNSPEC: u8 = 0;

pub const RTM_NEWLINK: u16 = 16;
pub const RTM_NEWADDR: u16 = 20;
pub const RTM_DELADDR: u16 = 21;
pub const RTM_NEWROUTE: u16 = 24;
pub const RTM_DELROUTE: u16 = 25;
pub const RTM_NEWRULE: u16 = 32;
pub const RTM_DELRULE: u16 = 33;
const NLMSG_ERROR: u16 = 2;

const NLM_F_REQUEST: u16 = 1;
const NLM_F_ACK: u16 = 4;
const NLM_F_REPLACE: u16 = 0x100;
const NLM_F_EXCL: u16 = 0x200;
const NLM_F_CREATE: u16 = 0x400;

const IFF_UP: u32 = 1;
const IFLA_ADDRESS: u16 = 1;
const IFA_ADDRESS: u16 = 1;
const IFA_LOCAL: u16 = 2;
const IFA_BROADCAST: u16 = 4;
const RTA_DST: u16 = 1;
const RTA_OIF: u16 = 4;
const RTA_GATEWAY: u16 = 5;
const RTA_TABLE: u16 = 15;
const FRA_PRIORITY: u16 = 6;
const FRA_TABLE: u16 = 15;
pub const RT_TABLE_MAIN: u32 = 254;
const RTPROT_STATIC: u8 = 4;
const RT_SCOPE_UNIVERSE: u8 = 0;
const RT_SCOPE_LINK: u8 = 253;
const RTN_UNICAST: u8 = 1;
const FR_ACT_TO_TBL: u8 = 1;

/// Priority of our rule: ahead of other network managers' rules (which end in
/// "unreachable"), so plain sockets use the main table.
pub const RULE_PRIORITY: u32 = 90;

fn align4(n: usize) -> usize {
    (n + 3) & !3
}

pub struct Msg {
    buf: Vec<u8>,
}

impl Msg {
    fn new(kind: u16, flags: u16, body: &[u8]) -> Self {
        let mut buf = vec![0u8; 16];
        buf[4..6].copy_from_slice(&kind.to_ne_bytes());
        buf[6..8].copy_from_slice(&(flags | NLM_F_REQUEST | NLM_F_ACK).to_ne_bytes());
        buf.extend_from_slice(body);
        buf.resize(align4(buf.len()), 0);
        Msg { buf }
    }

    fn attr(mut self, kind: u16, data: &[u8]) -> Self {
        let len = 4 + data.len();
        self.buf.extend_from_slice(&(len as u16).to_ne_bytes());
        self.buf.extend_from_slice(&kind.to_ne_bytes());
        self.buf.extend_from_slice(data);
        self.buf.resize(align4(self.buf.len()), 0);
        self
    }

    pub fn bytes(mut self, seq: u32) -> Vec<u8> {
        let len = self.buf.len() as u32;
        self.buf[0..4].copy_from_slice(&len.to_ne_bytes());
        self.buf[8..12].copy_from_slice(&seq.to_ne_bytes());
        self.buf
    }

    pub fn kind(&self) -> u16 {
        u16::from_ne_bytes([self.buf[4], self.buf[5]])
    }
}

fn ifinfo(index: u32, flags: u32, change: u32) -> Vec<u8> {
    let mut b = vec![AF_UNSPEC, 0, 0, 0];
    b.extend((index as i32).to_ne_bytes());
    b.extend(flags.to_ne_bytes());
    b.extend(change.to_ne_bytes());
    b
}

pub fn link_up(index: u32, up: bool) -> Msg {
    Msg::new(RTM_NEWLINK, 0, &ifinfo(index, if up { IFF_UP } else { 0 }, IFF_UP))
}

pub fn link_mac(index: u32, mac: [u8; 6]) -> Msg {
    Msg::new(RTM_NEWLINK, 0, &ifinfo(index, 0, 0)).attr(IFLA_ADDRESS, &mac)
}

fn ifaddr(index: u32, prefix: u8) -> Vec<u8> {
    let mut b = vec![AF_INET, prefix, 0, RT_SCOPE_UNIVERSE];
    b.extend(index.to_ne_bytes());
    b
}

pub fn add_addr(index: u32, ip: Ipv4Addr, prefix: u8) -> Msg {
    let bcast = u32::from(ip) | (u32::MAX >> prefix.min(32)).checked_shr(0).unwrap_or(0);
    Msg::new(RTM_NEWADDR, NLM_F_CREATE | NLM_F_REPLACE, &ifaddr(index, prefix))
        .attr(IFA_LOCAL, &ip.octets())
        .attr(IFA_ADDRESS, &ip.octets())
        .attr(IFA_BROADCAST, &Ipv4Addr::from(bcast).octets())
}

pub fn del_addr(index: u32, ip: Ipv4Addr, prefix: u8) -> Msg {
    Msg::new(RTM_DELADDR, 0, &ifaddr(index, prefix)).attr(IFA_LOCAL, &ip.octets())
}

fn rtmsg(dst_len: u8, scope: u8, table: u8) -> Vec<u8> {
    let mut b = vec![AF_INET, dst_len, 0, 0, table, RTPROT_STATIC, scope, RTN_UNICAST];
    b.extend(0u32.to_ne_bytes());
    b
}

/// Default route via `gw` out of `index`, in the main table.
pub fn default_route(index: u32, gw: Ipv4Addr, add: bool) -> Msg {
    let (kind, flags) = if add { (RTM_NEWROUTE, NLM_F_CREATE | NLM_F_REPLACE) } else { (RTM_DELROUTE, 0) };
    Msg::new(kind, flags, &rtmsg(0, RT_SCOPE_UNIVERSE, RT_TABLE_MAIN as u8))
        .attr(RTA_GATEWAY, &gw.octets())
        .attr(RTA_OIF, &index.to_ne_bytes())
        .attr(RTA_TABLE, &RT_TABLE_MAIN.to_ne_bytes())
}

/// The on-link subnet route (the kernel adds one to `main` too, but this
/// also covers tables after a flush).
pub fn subnet_route(index: u32, net: Ipv4Addr, prefix: u8) -> Msg {
    Msg::new(RTM_NEWROUTE, NLM_F_CREATE | NLM_F_REPLACE, &rtmsg(prefix, RT_SCOPE_LINK, RT_TABLE_MAIN as u8)).attr(RTA_DST, &net.octets()).attr(RTA_OIF, &index.to_ne_bytes())
}

/// "from all lookup main" at RULE_PRIORITY.
pub fn main_rule(add: bool) -> Msg {
    let (kind, flags) = if add { (RTM_NEWRULE, NLM_F_CREATE | NLM_F_EXCL) } else { (RTM_DELRULE, 0) };
    let mut hdr = vec![AF_INET, 0, 0, 0, RT_TABLE_MAIN as u8, 0, 0, FR_ACT_TO_TBL];
    hdr.extend(0u32.to_ne_bytes());
    Msg::new(kind, flags, &hdr).attr(FRA_PRIORITY, &RULE_PRIORITY.to_ne_bytes()).attr(FRA_TABLE, &RT_TABLE_MAIN.to_ne_bytes())
}

/// Why a received datagram is not the ACK of our request.
#[derive(Debug, PartialEq)]
pub enum AckError {
    /// Too short, another sequence number, or not an NLMSG_ERROR message: keep waiting.
    Unrelated,
    /// The kernel refused the request with this (positive) errno.
    Errno(i32),
}

/// Parses a netlink ACK/error reply to request `seq`: Ok(()) for errno 0.
pub fn parse_ack(reply: &[u8], seq: u32) -> Result<(), AckError> {
    if reply.len() < 20 || u16::from_ne_bytes([reply[4], reply[5]]) != NLMSG_ERROR || u32::from_ne_bytes(reply[8..12].try_into().unwrap()) != seq {
        return Err(AckError::Unrelated);
    }
    match i32::from_ne_bytes(reply[16..20].try_into().unwrap()) {
        0 => Ok(()),
        e => Err(AckError::Errno(-e)),
    }
}

extern "C" {
    fn socket(domain: i32, ty: i32, proto: i32) -> i32;
    fn send(fd: i32, buf: *const u8, len: usize, flags: i32) -> isize;
    fn recv(fd: i32, buf: *mut u8, len: usize, flags: i32) -> isize;
    fn close(fd: i32) -> i32;
    fn setsockopt(fd: i32, level: i32, name: i32, val: *const std::ffi::c_void, len: u32) -> i32;
}

/// How long `run` waits for the kernel's reply before giving up.
const REPLY_TIMEOUT_SECS: i64 = 5;

pub struct Netlink {
    fd: i32,
    seq: u32,
}

impl Netlink {
    pub fn open() -> io::Result<Self> {
        let fd = unsafe { socket(AF_NETLINK, 3 | 0o2000000, NETLINK_ROUTE) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let nl = Netlink { fd, seq: 1 };
        nl.set_recv_timeout(REPLY_TIMEOUT_SECS)?;
        Ok(nl)
    }

    fn set_recv_timeout(&self, secs: i64) -> io::Result<()> {
        const SOL_SOCKET: i32 = 1;
        const SO_RCVTIMEO: i32 = 20;
        // struct timeval { time_t, suseconds_t }: two longs on 64-bit Linux.
        let tv: [std::os::raw::c_long; 2] = [secs as std::os::raw::c_long, 0];
        // SAFETY: valid fd and a buffer of the stated length.
        let r = unsafe { setsockopt(self.fd, SOL_SOCKET, SO_RCVTIMEO, tv.as_ptr().cast(), std::mem::size_of_val(&tv) as u32) };
        if r < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// Sends one request and waits for its ACK. EEXIST counts as success.
    pub fn run(&mut self, m: Msg) -> io::Result<()> {
        self.seq += 1;
        let b = m.bytes(self.seq);
        if unsafe { send(self.fd, b.as_ptr(), b.len(), 0) } < 0 {
            return Err(io::Error::last_os_error());
        }
        let mut reply = [0u8; 4096];
        loop {
            let n = unsafe { recv(self.fd, reply.as_mut_ptr(), reply.len(), 0) };
            if n < 0 {
                let e = io::Error::last_os_error();
                return Err(if e.kind() == io::ErrorKind::WouldBlock { io::Error::new(io::ErrorKind::TimedOut, "netlink reply timed out") } else { e });
            }
            match parse_ack(&reply[..n as usize], self.seq) {
                Ok(()) | Err(AckError::Errno(17)) => return Ok(()), // 17 = EEXIST
                Err(AckError::Errno(e)) => return Err(io::Error::from_raw_os_error(e)),
                Err(AckError::Unrelated) => {}
            }
        }
    }
}

impl Drop for Netlink {
    fn drop(&mut self) {
        unsafe { close(self.fd) };
    }
}

pub fn ifindex(name: &str) -> Option<u32> {
    std::fs::read_to_string(format!("/sys/class/net/{name}/ifindex")).ok()?.trim().parse().ok()
}

pub fn parse_mac(s: &str) -> Option<[u8; 6]> {
    let v: Vec<u8> = s.trim().split(':').filter_map(|p| u8::from_str_radix(p, 16).ok()).collect();
    v.try_into().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attrs(b: &[u8], body: usize) -> Vec<(u16, Vec<u8>)> {
        let mut out = vec![];
        let mut p = 16 + body;
        while p + 4 <= b.len() {
            let len = u16::from_ne_bytes([b[p], b[p + 1]]) as usize;
            let kind = u16::from_ne_bytes([b[p + 2], b[p + 3]]);
            out.push((kind, b[p + 4..p + len].to_vec()));
            p += align4(len);
        }
        out
    }

    #[test]
    fn header_carries_length_type_and_ack_flags() {
        let b = link_up(5, true).bytes(9);
        assert_eq!(u32::from_ne_bytes(b[0..4].try_into().unwrap()) as usize, b.len());
        assert_eq!(u16::from_ne_bytes([b[4], b[5]]), RTM_NEWLINK);
        let flags = u16::from_ne_bytes([b[6], b[7]]);
        assert_eq!(flags & (NLM_F_REQUEST | NLM_F_ACK), NLM_F_REQUEST | NLM_F_ACK);
        assert_eq!(u32::from_ne_bytes(b[8..12].try_into().unwrap()), 9);
        // ifinfomsg: index 5, IFF_UP set and in the change mask
        assert_eq!(i32::from_ne_bytes(b[20..24].try_into().unwrap()), 5);
        assert_eq!(u32::from_ne_bytes(b[24..28].try_into().unwrap()), IFF_UP);
        assert_eq!(u32::from_ne_bytes(b[28..32].try_into().unwrap()), IFF_UP);
    }

    #[test]
    fn addr_message_has_local_and_broadcast() {
        let b = add_addr(3, Ipv4Addr::new(192, 168, 1, 171), 24).bytes(1);
        assert_eq!(b[16], AF_INET);
        assert_eq!(b[17], 24);
        let a = attrs(&b, 8);
        assert!(a.contains(&(IFA_LOCAL, vec![192, 168, 1, 171])));
        assert!(a.contains(&(IFA_BROADCAST, vec![192, 168, 1, 255])));
    }

    #[test]
    fn default_route_and_rule() {
        let b = default_route(3, Ipv4Addr::new(192, 168, 1, 1), true).bytes(1);
        assert_eq!(b[17], 0, "dst_len 0 = default");
        let a = attrs(&b, 12);
        assert!(a.contains(&(RTA_GATEWAY, vec![192, 168, 1, 1])));
        assert!(a.contains(&(RTA_OIF, 3u32.to_ne_bytes().to_vec())));
        let r = main_rule(true).bytes(1);
        assert_eq!(u16::from_ne_bytes([r[4], r[5]]), RTM_NEWRULE);
        assert_eq!(r[16 + 7], FR_ACT_TO_TBL);
        assert!(attrs(&r, 12).contains(&(FRA_PRIORITY, RULE_PRIORITY.to_ne_bytes().to_vec())));
    }

    #[test]
    fn ack_parsing() {
        let mut ok = vec![0u8; 36];
        ok[4..6].copy_from_slice(&NLMSG_ERROR.to_ne_bytes());
        ok[8..12].copy_from_slice(&7u32.to_ne_bytes());
        assert_eq!(parse_ack(&ok, 7), Ok(()));
        ok[16..20].copy_from_slice(&(-17i32).to_ne_bytes());
        assert_eq!(parse_ack(&ok, 7), Err(AckError::Errno(17)));
        ok[16..20].copy_from_slice(&(-1i32).to_ne_bytes());
        assert_eq!(parse_ack(&ok, 7), Err(AckError::Errno(1)), "EPERM is not 'not an ack'");
    }

    #[test]
    fn replies_for_other_requests_or_of_other_types_are_unrelated() {
        let mut m = vec![0u8; 36];
        m[4..6].copy_from_slice(&NLMSG_ERROR.to_ne_bytes());
        m[8..12].copy_from_slice(&6u32.to_ne_bytes());
        assert_eq!(parse_ack(&m, 7), Err(AckError::Unrelated), "stale sequence number");
        m[8..12].copy_from_slice(&7u32.to_ne_bytes());
        m[4..6].copy_from_slice(&RTM_NEWLINK.to_ne_bytes());
        assert_eq!(parse_ack(&m, 7), Err(AckError::Unrelated), "not an NLMSG_ERROR");
        assert_eq!(parse_ack(&[0; 4], 7), Err(AckError::Unrelated));
    }

    #[test]
    fn mac_parsing() {
        assert_eq!(parse_mac("9e:b8:73:b2:65:13\n"), Some([0x9e, 0xb8, 0x73, 0xb2, 0x65, 0x13]));
        assert_eq!(parse_mac("nope"), None);
    }
}
