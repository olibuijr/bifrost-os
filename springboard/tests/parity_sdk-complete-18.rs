//! Parity sdk-complete-18: headless tests for the nearby-share service model: typed share errors, beacon
//! datagrams (browse and resolve), advertised registration and peer bookkeeping. No socket is opened and
//! no clock is read: instants are built from one base value.

use springboard::net::share::{
    advertised_beacon, check_incoming, fingerprint, peer_from_datagram, Beacon, Header, Peer, Peers, ShareError, MAX_FILE_SIZE, PEER_TTL,
};
use springboard::shell::sha256;
use std::io;
use std::net::{IpAddr, Ipv4Addr};
use std::time::{Duration, Instant};

fn lan_host(last: u8) -> IpAddr {
    IpAddr::V4(Ipv4Addr::new(192, 168, 1, last))
}

#[test]
fn share_error_codes_are_stable_and_distinct() {
    let all = [
        ShareError::Declined,
        ShareError::WrongSecret,
        ShareError::Malformed,
        ShareError::TooLarge,
        ShareError::NoSpace,
        ShareError::ConnectionLost,
        ShareError::TimedOut,
        ShareError::ChecksumMismatch,
        ShareError::Rejected,
        ShareError::NameTaken,
        ShareError::Io("x".into()),
    ];
    let codes: Vec<i32> = all.iter().map(ShareError::code).collect();
    assert_eq!(codes, vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 99]);
    assert_eq!(ShareError::DOMAIN, "net.share");
}

#[test]
fn share_error_survives_a_trip_through_io_error() {
    for e in [ShareError::Declined, ShareError::WrongSecret, ShareError::ChecksumMismatch, ShareError::NoSpace] {
        let io = e.clone().into_io();
        assert_eq!(io.kind(), e.kind(), "the io kind is kept for callers that match on it");
        assert_eq!(ShareError::from_io(&io), e);
    }
}

#[test]
fn share_error_messages_read_as_plain_sentences() {
    assert_eq!(ShareError::Declined.to_string(), "declined");
    assert_eq!(ShareError::NoSpace.to_string(), "not enough free space");
    assert_eq!(ShareError::Rejected.to_string(), "receiver rejected the file");
    assert_eq!(ShareError::Io("disk fault".into()).to_string(), "disk fault");
}

#[test]
fn plain_io_errors_map_to_the_closest_share_error() {
    let timed_out = io::Error::new(io::ErrorKind::TimedOut, "slow");
    assert_eq!(ShareError::from_io(&timed_out), ShareError::TimedOut);
    let broken = io::Error::new(io::ErrorKind::BrokenPipe, "gone");
    assert_eq!(ShareError::from_io(&broken), ShareError::ConnectionLost);
    let eof = io::Error::new(io::ErrorKind::UnexpectedEof, "short");
    assert_eq!(ShareError::from_io(&eof), ShareError::ConnectionLost);
    let full = io::Error::new(io::ErrorKind::StorageFull, "full");
    assert_eq!(ShareError::from_io(&full), ShareError::NoSpace);
    let other = io::Error::new(io::ErrorKind::Other, "odd fault");
    assert_eq!(ShareError::from_io(&other), ShareError::Io("odd fault".into()));
}

#[test]
fn a_header_with_the_wrong_secret_reports_wrong_secret() {
    let h = Header { name: "a.txt".into(), size: 3, sha256: sha256::digest(b"abc") };
    let bytes = h.encode("nearby");
    let err = Header::read(&mut &bytes[..], "elsewhere").unwrap_err();
    assert_eq!(ShareError::from_io(&err), ShareError::WrongSecret);
}

#[test]
fn a_header_with_bad_magic_reports_malformed() {
    let h = Header { name: "a.txt".into(), size: 3, sha256: [0; 32] };
    let mut bytes = h.encode("nearby");
    bytes[0] = b'Z';
    let err = Header::read(&mut &bytes[..], "nearby").unwrap_err();
    assert_eq!(ShareError::from_io(&err), ShareError::Malformed);
}

#[test]
fn a_cut_off_header_reports_a_lost_connection() {
    let h = Header { name: "a.txt".into(), size: 3, sha256: [0; 32] };
    let bytes = h.encode("nearby");
    let err = Header::read(&mut &bytes[..bytes.len() - 4], "nearby").unwrap_err();
    assert_eq!(ShareError::from_io(&err), ShareError::ConnectionLost);
}

#[test]
fn incoming_files_over_the_size_cap_report_too_large() {
    let err = check_incoming(MAX_FILE_SIZE + 1, None).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    assert_eq!(ShareError::from_io(&err), ShareError::TooLarge);
}

#[test]
fn incoming_files_without_room_report_no_space() {
    let err = check_incoming(1 << 30, Some(1 << 30)).unwrap_err();
    assert_eq!(ShareError::from_io(&err), ShareError::NoSpace);
    assert!(check_incoming(1000, Some(1 << 30)).is_ok());
}

#[test]
fn a_beacon_from_the_same_secret_becomes_a_peer_at_the_sender_address() {
    let bytes = Beacon { name: "Tablet".into(), port: 5000, fingerprint: fingerprint("nearby") }.encode();
    let peer = peer_from_datagram(&bytes, lan_host(20), fingerprint("nearby"), 6000).expect("a peer");
    assert_eq!(peer, Peer { name: "Tablet".into(), addr: "192.168.1.20:5000".parse().unwrap() });
}

#[test]
fn beacons_from_another_secret_are_not_peers() {
    let bytes = Beacon { name: "Other".into(), port: 5000, fingerprint: fingerprint("other") }.encode();
    assert!(peer_from_datagram(&bytes, lan_host(21), fingerprint("nearby"), 6000).is_none());
}

#[test]
fn this_devices_own_beacon_is_not_a_peer() {
    let bytes = Beacon { name: "Me".into(), port: 6000, fingerprint: fingerprint("nearby") }.encode();
    assert!(peer_from_datagram(&bytes, lan_host(22), fingerprint("nearby"), 6000).is_none());
}

#[test]
fn garbage_datagrams_are_not_peers() {
    assert!(peer_from_datagram(b"hello", lan_host(23), fingerprint("nearby"), 6000).is_none());
    assert!(peer_from_datagram(&[], lan_host(23), fingerprint("nearby"), 6000).is_none());
}

#[test]
fn an_advertised_beacon_is_decoded_by_peers_with_the_same_secret() {
    let beacon = advertised_beacon("Desk Mac", 42424, "nearby").expect("valid");
    assert_eq!(beacon.fingerprint, fingerprint("nearby"));
    let peer = peer_from_datagram(&beacon.encode(), lan_host(30), fingerprint("nearby"), 1).expect("a peer");
    assert_eq!(peer.name, "Desk Mac");
    assert_eq!(peer.addr.port(), 42424);
}

#[test]
fn advertising_refuses_a_blank_name_and_port_zero() {
    assert!(advertised_beacon("   ", 42424, "nearby").is_err());
    assert!(advertised_beacon("", 42424, "nearby").is_err());
    assert!(advertised_beacon("Desk", 0, "nearby").is_err());
}

#[test]
fn a_peer_heard_twice_is_listed_once_under_its_latest_name() {
    let mut peers = Peers::default();
    let t0 = Instant::now();
    let addr = "192.168.1.40:5000".parse().unwrap();
    peers.heard(Peer { name: "Old".into(), addr }, t0);
    peers.heard(Peer { name: "New".into(), addr }, t0 + Duration::from_secs(2));
    let live = peers.live(t0 + Duration::from_secs(2));
    assert_eq!(live.len(), 1);
    assert_eq!(live[0].name, "New");
}

#[test]
fn peers_on_different_addresses_are_listed_separately() {
    let mut peers = Peers::default();
    let t0 = Instant::now();
    peers.heard(Peer { name: "A".into(), addr: "192.168.1.41:5000".parse().unwrap() }, t0);
    peers.heard(Peer { name: "B".into(), addr: "192.168.1.42:5000".parse().unwrap() }, t0);
    assert_eq!(peers.live(t0).len(), 2);
}

#[test]
fn a_peer_stays_listed_for_the_ttl_and_then_expires() {
    let mut peers = Peers::default();
    let t0 = Instant::now();
    peers.heard(Peer { name: "A".into(), addr: "192.168.1.43:5000".parse().unwrap() }, t0);
    assert_eq!(peers.live(t0 + PEER_TTL - Duration::from_millis(1)).len(), 1);
    assert!(peers.live(t0 + PEER_TTL).is_empty());
}
