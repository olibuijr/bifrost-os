//! Network packet parser: reads a captured frame into its layers, and reads a classic capture file (the libpcap
//! format that `tcpdump -w` writes) into frames. It is the decoding half of packet capture on BifrOSt: `tcpdump` (the
//! `tcpdump` package) records, and this module says what is in each record, as the one-line summary `summary` gives.
//!
//! Layers: Ethernet II (with 802.1Q / 802.1ad VLAN tags), ARP for IPv4, IPv4 (header checksum checked), IPv6 (with the
//! hop-by-hop, routing, destination-options and fragment extension headers skipped), TCP, UDP and ICMP / ICMPv6.
//! Capture link types 1 (Ethernet), 101 (raw IP) and 113 (Linux cooked) are read. Everything here is pure: it takes
//! bytes and opens no socket and no file, and a bad or cut-short frame is an error, never a panic.

use std::fmt;
use std::net::{Ipv4Addr, Ipv6Addr};

/// Why bytes could not be read as a packet or a capture file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParseError {
    /// The data ends before the named part is complete.
    Truncated(&'static str),
    /// The named part is there but its fields cannot be right.
    Malformed(&'static str),
    /// The first four bytes of a capture file are not a known magic number.
    BadMagic,
    /// The capture's link type is not one this parser reads.
    UnsupportedLink(u32),
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::Truncated(what) => write!(f, "truncated {what}"),
            ParseError::Malformed(what) => write!(f, "malformed {what}"),
            ParseError::BadMagic => write!(f, "not a capture file"),
            ParseError::UnsupportedLink(n) => write!(f, "unsupported link type {n}"),
        }
    }
}

/// A hardware address.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mac(pub [u8; 6]);

impl fmt::Display for Mac {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let b = self.0;
        write!(f, "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}", b[0], b[1], b[2], b[3], b[4], b[5])
    }
}

/// The Ethernet header of a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ethernet {
    pub dst: Mac,
    pub src: Mac,
    /// The VLAN id of the outermost tag, when the frame is tagged.
    pub vlan: Option<u16>,
    /// The type of what follows, after any tags.
    pub ethertype: u16,
}

/// Ethertypes this parser knows.
pub const ETHERTYPE_IPV4: u16 = 0x0800;
pub const ETHERTYPE_ARP: u16 = 0x0806;
pub const ETHERTYPE_IPV6: u16 = 0x86dd;
const ETHERTYPE_VLAN: u16 = 0x8100;
const ETHERTYPE_QINQ: u16 = 0x88a8;

/// IP protocol numbers this parser knows.
pub const PROTO_ICMP: u8 = 1;
pub const PROTO_TCP: u8 = 6;
pub const PROTO_UDP: u8 = 17;
pub const PROTO_ICMPV6: u8 = 58;

/// An ARP request or reply for IPv4 over Ethernet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Arp {
    /// 1 for a request, 2 for a reply.
    pub op: u16,
    pub sender_mac: Mac,
    pub sender_ip: Ipv4Addr,
    pub target_mac: Mac,
    pub target_ip: Ipv4Addr,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ipv4 {
    pub src: Ipv4Addr,
    pub dst: Ipv4Addr,
    pub protocol: u8,
    pub ttl: u8,
    /// Bytes of the header, options included.
    pub header_len: usize,
    pub total_len: usize,
    pub more_fragments: bool,
    /// Byte offset of this fragment in the original datagram (0 for an unfragmented packet).
    pub fragment_offset: usize,
    /// Whether the header checksum adds up.
    pub checksum_ok: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ipv6 {
    pub src: Ipv6Addr,
    pub dst: Ipv6Addr,
    /// The protocol after the extension headers.
    pub next_header: u8,
    pub hop_limit: u8,
    pub payload_len: usize,
}

/// The network layer of a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Network {
    Arp(Arp),
    Ipv4(Ipv4),
    Ipv6(Ipv6),
    /// An ethertype this parser does not read.
    Other(u16),
}

/// TCP flag bits, as the header carries them.
pub const TCP_FIN: u8 = 0x01;
pub const TCP_SYN: u8 = 0x02;
pub const TCP_RST: u8 = 0x04;
pub const TCP_PSH: u8 = 0x08;
pub const TCP_ACK: u8 = 0x10;
pub const TCP_URG: u8 = 0x20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tcp {
    pub src_port: u16,
    pub dst_port: u16,
    pub seq: u32,
    pub ack: u32,
    pub flags: u8,
    pub window: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Udp {
    pub src_port: u16,
    pub dst_port: u16,
    /// The length field: header and data.
    pub length: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Icmp {
    /// ICMPv6 rather than ICMP for IPv4.
    pub v6: bool,
    pub kind: u8,
    pub code: u8,
}

/// The transport layer of a packet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transport {
    Tcp(Tcp),
    Udp(Udp),
    Icmp(Icmp),
    /// An IP protocol number this parser does not read.
    Other(u8),
}

/// One parsed frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Packet {
    pub ethernet: Option<Ethernet>,
    pub network: Network,
    /// None for ARP, an unknown ethertype, and fragments after the first (which carry no transport header).
    pub transport: Option<Transport>,
    /// Bytes of data after the innermost header that was read.
    pub payload_len: usize,
}

fn need(data: &[u8], len: usize, what: &'static str) -> Result<(), ParseError> {
    if data.len() < len {
        Err(ParseError::Truncated(what))
    } else {
        Ok(())
    }
}

fn be16(b: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([b[at], b[at + 1]])
}

fn be32(b: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn mac_at(b: &[u8], at: usize) -> Mac {
    let mut m = [0u8; 6];
    m.copy_from_slice(&b[at..at + 6]);
    Mac(m)
}

fn ipv4_at(b: &[u8], at: usize) -> Ipv4Addr {
    Ipv4Addr::new(b[at], b[at + 1], b[at + 2], b[at + 3])
}

fn ipv6_at(b: &[u8], at: usize) -> Ipv6Addr {
    let mut a = [0u8; 16];
    a.copy_from_slice(&b[at..at + 16]);
    Ipv6Addr::from(a)
}

/// The Internet checksum (RFC 1071) of `data`: the ones' complement of the ones' complement sum of its 16-bit words.
/// A header whose checksum field is filled in sums to 0.
pub fn checksum(data: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    let mut i = 0;
    while i + 1 < data.len() {
        sum += u32::from(be16(data, i));
        i += 2;
    }
    if i < data.len() {
        sum += u32::from(data[i]) << 8;
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

/// Reads an Ethernet frame (without the trailing frame check sequence).
pub fn parse_ethernet(frame: &[u8]) -> Result<Packet, ParseError> {
    need(frame, 14, "ethernet header")?;
    let dst = mac_at(frame, 0);
    let src = mac_at(frame, 6);
    let mut ethertype = be16(frame, 12);
    let mut offset = 14;
    let mut vlan = None;
    while ethertype == ETHERTYPE_VLAN || ethertype == ETHERTYPE_QINQ {
        need(frame, offset + 4, "vlan tag")?;
        if vlan.is_none() {
            vlan = Some(be16(frame, offset) & 0x0fff);
        }
        ethertype = be16(frame, offset + 2);
        offset += 4;
    }
    let eth = Ethernet { dst, src, vlan, ethertype };
    let mut packet = parse_network(ethertype, &frame[offset..])?;
    packet.ethernet = Some(eth);
    Ok(packet)
}

/// Reads a packet that starts at the IP header (capture link type 101). The version nibble says IPv4 or IPv6.
pub fn parse_ip(data: &[u8]) -> Result<Packet, ParseError> {
    need(data, 1, "ip header")?;
    match data[0] >> 4 {
        4 => parse_network(ETHERTYPE_IPV4, data),
        6 => parse_network(ETHERTYPE_IPV6, data),
        _ => Err(ParseError::Malformed("ip version")),
    }
}

/// Reads one captured record according to the capture's link type.
pub fn parse_record(link: u32, data: &[u8]) -> Result<Packet, ParseError> {
    match link {
        LINK_ETHERNET => parse_ethernet(data),
        LINK_RAW_IP => parse_ip(data),
        LINK_LINUX_COOKED => {
            // 16 bytes: packet type, address type, address length, address (8), protocol.
            need(data, 16, "cooked header")?;
            parse_network(be16(data, 14), &data[16..])
        }
        other => Err(ParseError::UnsupportedLink(other)),
    }
}

fn parse_network(ethertype: u16, data: &[u8]) -> Result<Packet, ParseError> {
    match ethertype {
        ETHERTYPE_ARP => {
            need(data, 28, "arp packet")?;
            // Hardware type 1 (Ethernet) with 6-byte addresses, protocol 0x0800 with 4-byte addresses.
            if be16(data, 0) != 1 || be16(data, 2) != ETHERTYPE_IPV4 || data[4] != 6 || data[5] != 4 {
                return Err(ParseError::Malformed("arp packet"));
            }
            let arp = Arp {
                op: be16(data, 6),
                sender_mac: mac_at(data, 8),
                sender_ip: ipv4_at(data, 14),
                target_mac: mac_at(data, 18),
                target_ip: ipv4_at(data, 24),
            };
            Ok(Packet { ethernet: None, network: Network::Arp(arp), transport: None, payload_len: data.len() - 28 })
        }
        ETHERTYPE_IPV4 => parse_ipv4(data),
        ETHERTYPE_IPV6 => parse_ipv6(data),
        other => Ok(Packet { ethernet: None, network: Network::Other(other), transport: None, payload_len: data.len() }),
    }
}

fn parse_ipv4(data: &[u8]) -> Result<Packet, ParseError> {
    need(data, 20, "ipv4 header")?;
    if data[0] >> 4 != 4 {
        return Err(ParseError::Malformed("ipv4 version"));
    }
    let header_len = usize::from(data[0] & 0x0f) * 4;
    if header_len < 20 {
        return Err(ParseError::Malformed("ipv4 header length"));
    }
    need(data, header_len, "ipv4 options")?;
    let total_len = usize::from(be16(data, 2));
    if total_len < header_len {
        return Err(ParseError::Malformed("ipv4 total length"));
    }
    let frag = be16(data, 6);
    let ip = Ipv4 {
        src: ipv4_at(data, 12),
        dst: ipv4_at(data, 16),
        protocol: data[9],
        ttl: data[8],
        header_len,
        total_len,
        more_fragments: frag & 0x2000 != 0,
        fragment_offset: usize::from(frag & 0x1fff) * 8,
        checksum_ok: checksum(&data[..header_len]) == 0,
    };
    // Ethernet pads short frames, so the datagram ends where its own length says, not where the data does.
    let end = total_len.min(data.len());
    let payload = &data[header_len..end];
    let (transport, payload_len) = if ip.fragment_offset == 0 {
        parse_transport(ip.protocol, payload)?
    } else {
        (None, payload.len())
    };
    Ok(Packet { ethernet: None, network: Network::Ipv4(ip), transport, payload_len })
}

fn parse_ipv6(data: &[u8]) -> Result<Packet, ParseError> {
    need(data, 40, "ipv6 header")?;
    if data[0] >> 4 != 6 {
        return Err(ParseError::Malformed("ipv6 version"));
    }
    let payload_len = usize::from(be16(data, 4));
    let mut next = data[6];
    let mut offset = 40;
    let end = (40 + payload_len).min(data.len());
    let mut first_fragment = true;
    // Skip extension headers; a chain longer than eight is not a real packet.
    for _ in 0..8 {
        match next {
            0 | 43 | 60 => {
                need(&data[..end], offset + 2, "ipv6 extension header")?;
                let len = (usize::from(data[offset + 1]) + 1) * 8;
                next = data[offset];
                offset += len;
            }
            44 => {
                need(&data[..end], offset + 8, "ipv6 fragment header")?;
                first_fragment = be16(data, offset + 2) >> 3 == 0;
                next = data[offset];
                offset += 8;
            }
            _ => break,
        }
    }
    if offset > end {
        return Err(ParseError::Truncated("ipv6 extension header"));
    }
    let ip = Ipv6 { src: ipv6_at(data, 8), dst: ipv6_at(data, 24), next_header: next, hop_limit: data[7], payload_len };
    let payload = &data[offset..end];
    let (transport, len) = if first_fragment { parse_transport(next, payload)? } else { (None, payload.len()) };
    Ok(Packet { ethernet: None, network: Network::Ipv6(ip), transport, payload_len: len })
}

fn parse_transport(protocol: u8, p: &[u8]) -> Result<(Option<Transport>, usize), ParseError> {
    match protocol {
        PROTO_TCP => {
            need(p, 20, "tcp header")?;
            let header_len = usize::from(p[12] >> 4) * 4;
            if header_len < 20 || header_len > p.len() {
                return Err(ParseError::Malformed("tcp header length"));
            }
            let tcp = Tcp {
                src_port: be16(p, 0),
                dst_port: be16(p, 2),
                seq: be32(p, 4),
                ack: be32(p, 8),
                flags: p[13],
                window: be16(p, 14),
            };
            Ok((Some(Transport::Tcp(tcp)), p.len() - header_len))
        }
        PROTO_UDP => {
            need(p, 8, "udp header")?;
            let udp = Udp { src_port: be16(p, 0), dst_port: be16(p, 2), length: be16(p, 4) };
            if udp.length < 8 {
                return Err(ParseError::Malformed("udp length"));
            }
            // The length field bounds the data when the frame carries more (padding).
            let data = usize::from(udp.length) - 8;
            Ok((Some(Transport::Udp(udp)), data.min(p.len() - 8)))
        }
        PROTO_ICMP | PROTO_ICMPV6 => {
            need(p, 4, "icmp header")?;
            let icmp = Icmp { v6: protocol == PROTO_ICMPV6, kind: p[0], code: p[1] };
            Ok((Some(Transport::Icmp(icmp)), p.len() - 4))
        }
        other => Ok((Some(Transport::Other(other)), p.len())),
    }
}

/// The TCP flags as letters, in the order S F R P U then `.` for ACK; `none` when no flag is set.
pub fn tcp_flags_text(flags: u8) -> String {
    let mut s = String::new();
    for (bit, ch) in [(TCP_SYN, 'S'), (TCP_FIN, 'F'), (TCP_RST, 'R'), (TCP_PSH, 'P'), (TCP_URG, 'U'), (TCP_ACK, '.')] {
        if flags & bit != 0 {
            s.push(ch);
        }
    }
    if s.is_empty() {
        s.push_str("none");
    }
    s
}

fn icmp_text(i: &Icmp) -> String {
    let name = match (i.v6, i.kind) {
        (false, 0) | (true, 129) => "echo reply",
        (false, 8) | (true, 128) => "echo request",
        (false, 3) | (true, 1) => "destination unreachable",
        (false, 11) | (true, 3) => "time exceeded",
        (true, 135) => "neighbor solicitation",
        (true, 136) => "neighbor advertisement",
        _ => "",
    };
    if name.is_empty() {
        format!("type {} code {}", i.kind, i.code)
    } else {
        name.to_string()
    }
}

fn endpoint(addr: &str, port: Option<u16>) -> String {
    match port {
        Some(p) => format!("{addr}.{p}"),
        None => addr.to_string(),
    }
}

/// One line saying what the packet is, in the style of a capture tool:
/// `IP 10.0.0.2.5555 > 10.0.0.1.443: TCP Flags [S], seq 1, ack 0, win 1000, length 0`.
pub fn summary(p: &Packet) -> String {
    let (family, src, dst) = match &p.network {
        Network::Arp(a) => {
            return match a.op {
                1 => format!("ARP who-has {} tell {}", a.target_ip, a.sender_ip),
                2 => format!("ARP reply {} is-at {}", a.sender_ip, a.sender_mac),
                n => format!("ARP op {n}"),
            };
        }
        Network::Other(t) => return format!("ethertype 0x{t:04x}, length {}", p.payload_len),
        Network::Ipv4(ip) => ("IP", ip.src.to_string(), ip.dst.to_string()),
        Network::Ipv6(ip) => ("IP6", ip.src.to_string(), ip.dst.to_string()),
    };
    match &p.transport {
        Some(Transport::Tcp(t)) => format!(
            "{family} {} > {}: TCP Flags [{}], seq {}, ack {}, win {}, length {}",
            endpoint(&src, Some(t.src_port)),
            endpoint(&dst, Some(t.dst_port)),
            tcp_flags_text(t.flags),
            t.seq,
            t.ack,
            t.window,
            p.payload_len
        ),
        Some(Transport::Udp(u)) => format!(
            "{family} {} > {}: UDP, length {}",
            endpoint(&src, Some(u.src_port)),
            endpoint(&dst, Some(u.dst_port)),
            p.payload_len
        ),
        Some(Transport::Icmp(i)) => format!("{family} {src} > {dst}: ICMP {}, length {}", icmp_text(i), p.payload_len),
        Some(Transport::Other(n)) => format!("{family} {src} > {dst}: protocol {n}, length {}", p.payload_len),
        None => format!("{family} {src} > {dst}: fragment, length {}", p.payload_len),
    }
}

/// Capture link types this parser reads.
pub const LINK_ETHERNET: u32 = 1;
pub const LINK_RAW_IP: u32 = 101;
pub const LINK_LINUX_COOKED: u32 = 113;

/// One record of a capture file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub ts_sec: u32,
    /// The fraction of the second, always in nanoseconds (microsecond files are scaled up).
    pub ts_nsec: u32,
    /// The length on the wire; longer than `data` when the capture cut the frame short.
    pub orig_len: u32,
    pub data: Vec<u8>,
}

/// A capture file read into memory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Capture {
    pub link: u32,
    pub snaplen: u32,
    pub records: Vec<Record>,
    /// The file ended inside a record; the complete records before it are kept.
    pub truncated: bool,
}

impl Capture {
    /// Parses every record; each result is that record's packet or why it could not be read.
    pub fn packets(&self) -> Vec<Result<Packet, ParseError>> {
        self.records.iter().map(|r| parse_record(self.link, &r.data)).collect()
    }
}

/// Reads a classic capture file: either byte order, microsecond or nanosecond timestamps.
pub fn read_pcap(bytes: &[u8]) -> Result<Capture, ParseError> {
    need(bytes, 24, "capture header")?;
    let magic = [bytes[0], bytes[1], bytes[2], bytes[3]];
    let (big, nano) = match magic {
        [0xa1, 0xb2, 0xc3, 0xd4] => (true, false),
        [0xd4, 0xc3, 0xb2, 0xa1] => (false, false),
        [0xa1, 0xb2, 0x3c, 0x4d] => (true, true),
        [0x4d, 0x3c, 0xb2, 0xa1] => (false, true),
        _ => return Err(ParseError::BadMagic),
    };
    let rd = |at: usize| -> u32 {
        let w = [bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]];
        if big {
            u32::from_be_bytes(w)
        } else {
            u32::from_le_bytes(w)
        }
    };
    let snaplen = rd(16);
    let link = rd(20);
    let mut records = Vec::new();
    let mut truncated = false;
    let mut at = 24;
    while at < bytes.len() {
        if bytes.len() - at < 16 {
            truncated = true;
            break;
        }
        let ts_sec = rd(at);
        let frac = rd(at + 4);
        let incl = rd(at + 8) as usize;
        let orig_len = rd(at + 12);
        at += 16;
        if bytes.len() - at < incl {
            truncated = true;
            break;
        }
        let ts_nsec = if nano { frac } else { frac.saturating_mul(1000) };
        records.push(Record { ts_sec, ts_nsec, orig_len, data: bytes[at..at + incl].to_vec() });
        at += incl;
    }
    Ok(Capture { link, snaplen, records, truncated })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ethernet(ethertype: u16, payload: &[u8]) -> Vec<u8> {
        let mut f = vec![0x02, 0, 0, 0, 0, 0x01, 0x02, 0, 0, 0, 0, 0x02];
        f.extend_from_slice(&ethertype.to_be_bytes());
        f.extend_from_slice(payload);
        f
    }

    fn ipv4(protocol: u8, payload: &[u8]) -> Vec<u8> {
        let total = (20 + payload.len()) as u16;
        let mut h = vec![0x45, 0, 0, 0, 0, 0, 0, 0, 64, protocol, 0, 0, 10, 0, 0, 2, 10, 0, 0, 1];
        h[2..4].copy_from_slice(&total.to_be_bytes());
        let sum = checksum(&h);
        h[10..12].copy_from_slice(&sum.to_be_bytes());
        h.extend_from_slice(payload);
        h
    }

    fn tcp(src: u16, dst: u16, flags: u8, data: &[u8]) -> Vec<u8> {
        let mut t = vec![0u8; 20];
        t[0..2].copy_from_slice(&src.to_be_bytes());
        t[2..4].copy_from_slice(&dst.to_be_bytes());
        t[4..8].copy_from_slice(&1u32.to_be_bytes());
        t[12] = 5 << 4;
        t[13] = flags;
        t[14..16].copy_from_slice(&1000u16.to_be_bytes());
        t.extend_from_slice(data);
        t
    }

    fn udp(src: u16, dst: u16, data: &[u8]) -> Vec<u8> {
        let mut u = Vec::new();
        u.extend_from_slice(&src.to_be_bytes());
        u.extend_from_slice(&dst.to_be_bytes());
        u.extend_from_slice(&((8 + data.len()) as u16).to_be_bytes());
        u.extend_from_slice(&[0, 0]);
        u.extend_from_slice(data);
        u
    }

    #[test]
    fn the_checksum_of_a_filled_header_is_zero() {
        let h = ipv4(PROTO_UDP, &[]);
        assert_eq!(checksum(&h[..20]), 0);
        // RFC 1071 example words.
        assert_eq!(checksum(&[0x00, 0x01, 0xf2, 0x03, 0xf4, 0xf5, 0xf6, 0xf7]), !0xddf2u16);
        // An odd length pads with a zero byte.
        assert_eq!(checksum(&[0xff]), !0xff00u16);
    }

    #[test]
    fn a_tcp_syn_is_read_through_all_layers() {
        let frame = ethernet(ETHERTYPE_IPV4, &ipv4(PROTO_TCP, &tcp(5555, 443, TCP_SYN, &[])));
        let p = parse_ethernet(&frame).unwrap();
        let eth = p.ethernet.unwrap();
        assert_eq!(eth.ethertype, ETHERTYPE_IPV4);
        assert_eq!(eth.vlan, None);
        assert_eq!(eth.src.to_string(), "02:00:00:00:00:02");
        match p.network {
            Network::Ipv4(ip) => {
                assert_eq!(ip.src, Ipv4Addr::new(10, 0, 0, 2));
                assert_eq!(ip.dst, Ipv4Addr::new(10, 0, 0, 1));
                assert!(ip.checksum_ok);
                assert_eq!(ip.ttl, 64);
            }
            other => panic!("not ipv4: {other:?}"),
        }
        assert_eq!(p.payload_len, 0);
        assert_eq!(
            summary(&p),
            "IP 10.0.0.2.5555 > 10.0.0.1.443: TCP Flags [S], seq 1, ack 0, win 1000, length 0"
        );
    }

    #[test]
    fn a_bad_header_checksum_is_reported_not_rejected() {
        let mut ip = ipv4(PROTO_TCP, &tcp(1, 2, TCP_ACK, b"hi"));
        ip[8] = 1; // changes the ttl after the checksum was filled in
        let p = parse_ip(&ip).unwrap();
        match p.network {
            Network::Ipv4(h) => assert!(!h.checksum_ok),
            other => panic!("not ipv4: {other:?}"),
        }
        assert_eq!(p.payload_len, 2);
    }

    #[test]
    fn ethernet_padding_does_not_count_as_data() {
        let mut frame = ethernet(ETHERTYPE_IPV4, &ipv4(PROTO_UDP, &udp(1000, 53, b"abc")));
        frame.extend_from_slice(&[0u8; 20]);
        let p = parse_ethernet(&frame).unwrap();
        assert_eq!(p.payload_len, 3);
        assert_eq!(summary(&p), "IP 10.0.0.2.1000 > 10.0.0.1.53: UDP, length 3");
    }

    #[test]
    fn a_vlan_tag_is_skipped_and_its_id_kept() {
        let inner = ipv4(PROTO_UDP, &udp(1, 2, &[]));
        let mut tagged = vec![0u8; 12];
        tagged.extend_from_slice(&ETHERTYPE_VLAN.to_be_bytes());
        tagged.extend_from_slice(&0x2064u16.to_be_bytes()); // priority 1, vlan 100
        tagged.extend_from_slice(&ETHERTYPE_IPV4.to_be_bytes());
        tagged.extend_from_slice(&inner);
        let p = parse_ethernet(&tagged).unwrap();
        assert_eq!(p.ethernet.unwrap().vlan, Some(100));
        assert!(matches!(p.transport, Some(Transport::Udp(_))));
    }

    #[test]
    fn arp_requests_and_replies_read_and_summarise() {
        let mut a = vec![0, 1, 8, 0, 6, 4, 0, 1];
        a.extend_from_slice(&[2, 0, 0, 0, 0, 2]);
        a.extend_from_slice(&[10, 0, 0, 2]);
        a.extend_from_slice(&[0; 6]);
        a.extend_from_slice(&[10, 0, 0, 1]);
        let p = parse_ethernet(&ethernet(ETHERTYPE_ARP, &a)).unwrap();
        assert_eq!(summary(&p), "ARP who-has 10.0.0.1 tell 10.0.0.2");
        a[7] = 2;
        let p = parse_ethernet(&ethernet(ETHERTYPE_ARP, &a)).unwrap();
        assert_eq!(summary(&p), "ARP reply 10.0.0.2 is-at 02:00:00:00:00:02");
        // Another hardware type is refused.
        a[1] = 6;
        assert_eq!(parse_ethernet(&ethernet(ETHERTYPE_ARP, &a)), Err(ParseError::Malformed("arp packet")));
    }

    fn ipv6(next: u8, payload: &[u8]) -> Vec<u8> {
        let mut h = vec![0x60, 0, 0, 0, 0, 0, next, 64];
        h[4..6].copy_from_slice(&(payload.len() as u16).to_be_bytes());
        let mut src = [0u8; 16];
        src[0] = 0xfe;
        src[1] = 0x80;
        src[15] = 1;
        let mut dst = src;
        dst[15] = 2;
        h.extend_from_slice(&src);
        h.extend_from_slice(&dst);
        h.extend_from_slice(payload);
        h
    }

    #[test]
    fn ipv6_extension_headers_are_skipped_to_the_transport() {
        // A hop-by-hop header (8 bytes) announcing UDP, then the UDP datagram.
        let mut payload = vec![PROTO_UDP, 0, 0, 0, 0, 0, 0, 0];
        payload.extend_from_slice(&udp(5353, 5353, b"xy"));
        let p = parse_ip(&ipv6(0, &payload)).unwrap();
        match p.network {
            Network::Ipv6(ip) => {
                assert_eq!(ip.next_header, PROTO_UDP);
                assert_eq!(ip.hop_limit, 64);
            }
            other => panic!("not ipv6: {other:?}"),
        }
        assert_eq!(p.payload_len, 2);
        assert_eq!(summary(&p), "IP6 fe80::1.5353 > fe80::2.5353: UDP, length 2");
    }

    #[test]
    fn icmp_echo_has_a_name_and_other_types_a_number() {
        let echo = ipv4(PROTO_ICMP, &[8, 0, 0, 0, 0, 1, 0, 1]);
        assert_eq!(summary(&parse_ip(&echo).unwrap()), "IP 10.0.0.2 > 10.0.0.1: ICMP echo request, length 4");
        let odd = ipv4(PROTO_ICMP, &[40, 3, 0, 0]);
        assert_eq!(summary(&parse_ip(&odd).unwrap()), "IP 10.0.0.2 > 10.0.0.1: ICMP type 40 code 3, length 0");
    }

    #[test]
    fn a_later_fragment_has_no_transport_header() {
        let mut ip = ipv4(PROTO_UDP, &[1, 2, 3, 4]);
        ip[6] = 0x00;
        ip[7] = 0x05; // offset 5 * 8
        ip[10] = 0;
        ip[11] = 0;
        let sum = checksum(&ip[..20]);
        ip[10..12].copy_from_slice(&sum.to_be_bytes());
        let p = parse_ip(&ip).unwrap();
        assert_eq!(p.transport, None);
        assert_eq!(p.payload_len, 4);
        assert_eq!(summary(&p), "IP 10.0.0.2 > 10.0.0.1: fragment, length 4");
    }

    #[test]
    fn cut_short_and_malformed_frames_are_errors() {
        assert_eq!(parse_ethernet(&[0; 10]), Err(ParseError::Truncated("ethernet header")));
        let short = ethernet(ETHERTYPE_IPV4, &[0x45, 0, 0, 20]);
        assert_eq!(parse_ethernet(&short), Err(ParseError::Truncated("ipv4 header")));
        let mut bad_ihl = ipv4(PROTO_UDP, &[]);
        bad_ihl[0] = 0x44;
        assert_eq!(parse_ip(&bad_ihl), Err(ParseError::Malformed("ipv4 header length")));
        let mut bad_tcp = ipv4(PROTO_TCP, &tcp(1, 2, 0, &[]));
        bad_tcp[20 + 12] = 4 << 4; // data offset below 5 words
        assert_eq!(parse_ip(&bad_tcp), Err(ParseError::Malformed("tcp header length")));
        assert_eq!(parse_ip(&[0x10]), Err(ParseError::Malformed("ip version")));
    }

    #[test]
    fn unknown_ethertypes_and_protocols_are_kept_not_refused() {
        let p = parse_ethernet(&ethernet(0x88cc, &[1, 2, 3])).unwrap();
        assert_eq!(summary(&p), "ethertype 0x88cc, length 3");
        let p = parse_ip(&ipv4(47, &[0; 6])).unwrap();
        assert_eq!(summary(&p), "IP 10.0.0.2 > 10.0.0.1: protocol 47, length 6");
    }

    #[test]
    fn tcp_flags_read_as_letters() {
        assert_eq!(tcp_flags_text(TCP_SYN | TCP_ACK), "S.");
        assert_eq!(tcp_flags_text(TCP_FIN | TCP_PSH | TCP_ACK), "FP.");
        assert_eq!(tcp_flags_text(0), "none");
    }

    fn pcap(big: bool, nano: bool, link: u32, frames: &[&[u8]]) -> Vec<u8> {
        let w32 = |v: u32| if big { v.to_be_bytes() } else { v.to_le_bytes() };
        let w16 = |v: u16| if big { v.to_be_bytes() } else { v.to_le_bytes() };
        let magic: u32 = if nano { 0xa1b2_3c4d } else { 0xa1b2_c3d4 };
        let mut f = Vec::new();
        f.extend_from_slice(&w32(magic));
        f.extend_from_slice(&w16(2));
        f.extend_from_slice(&w16(4));
        f.extend_from_slice(&w32(0));
        f.extend_from_slice(&w32(0));
        f.extend_from_slice(&w32(65535));
        f.extend_from_slice(&w32(link));
        for (i, d) in frames.iter().enumerate() {
            f.extend_from_slice(&w32(1000 + i as u32));
            f.extend_from_slice(&w32(250));
            f.extend_from_slice(&w32(d.len() as u32));
            f.extend_from_slice(&w32(d.len() as u32 + 4));
            f.extend_from_slice(d);
        }
        f
    }

    #[test]
    fn a_capture_file_reads_in_either_byte_order_and_timestamp_unit() {
        let a = ethernet(ETHERTYPE_IPV4, &ipv4(PROTO_UDP, &udp(1, 2, b"a")));
        let b = ethernet(ETHERTYPE_IPV4, &ipv4(PROTO_TCP, &tcp(3, 4, TCP_ACK, &[])));
        for big in [false, true] {
            for nano in [false, true] {
                let c = read_pcap(&pcap(big, nano, LINK_ETHERNET, &[&a, &b])).unwrap();
                assert_eq!(c.link, LINK_ETHERNET);
                assert_eq!(c.snaplen, 65535);
                assert_eq!(c.records.len(), 2);
                assert!(!c.truncated);
                assert_eq!(c.records[0].ts_sec, 1000);
                assert_eq!(c.records[0].ts_nsec, if nano { 250 } else { 250_000 });
                assert_eq!(c.records[0].orig_len, a.len() as u32 + 4);
                let packets = c.packets();
                assert!(matches!(packets[0].as_ref().unwrap().transport, Some(Transport::Udp(_))));
                assert!(matches!(packets[1].as_ref().unwrap().transport, Some(Transport::Tcp(_))));
            }
        }
    }

    #[test]
    fn a_capture_cut_short_keeps_its_complete_records() {
        let a = ethernet(ETHERTYPE_IPV4, &ipv4(PROTO_UDP, &udp(1, 2, b"a")));
        let mut f = pcap(false, false, LINK_ETHERNET, &[&a, &a]);
        f.truncate(f.len() - 5);
        let c = read_pcap(&f).unwrap();
        assert_eq!(c.records.len(), 1);
        assert!(c.truncated);
        // A header cut inside a record header is also a truncation.
        let mut g = pcap(false, false, LINK_ETHERNET, &[&a]);
        g.extend_from_slice(&[0; 7]);
        assert!(read_pcap(&g).unwrap().truncated);
    }

    #[test]
    fn capture_headers_that_are_not_captures_are_refused() {
        assert_eq!(read_pcap(&[0; 10]), Err(ParseError::Truncated("capture header")));
        assert_eq!(read_pcap(&[0; 30]), Err(ParseError::BadMagic));
    }

    #[test]
    fn raw_ip_and_linux_cooked_captures_are_read_and_other_links_named() {
        let ip = ipv4(PROTO_UDP, &udp(9, 9, &[]));
        let raw = read_pcap(&pcap(false, false, LINK_RAW_IP, &[&ip])).unwrap();
        assert!(raw.packets()[0].is_ok());
        let mut cooked = vec![0u8; 14];
        cooked.extend_from_slice(&ETHERTYPE_IPV4.to_be_bytes());
        cooked.extend_from_slice(&ip);
        let c = read_pcap(&pcap(false, false, LINK_LINUX_COOKED, &[&cooked])).unwrap();
        assert!(matches!(c.packets()[0].as_ref().unwrap().network, Network::Ipv4(_)));
        let odd = read_pcap(&pcap(false, false, 147, &[&ip])).unwrap();
        assert_eq!(odd.packets()[0], Err(ParseError::UnsupportedLink(147)));
    }
}
