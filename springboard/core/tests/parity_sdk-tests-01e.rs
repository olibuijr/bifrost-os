//! Parity tests for the custom VPN tunnel model: the session life cycle, profile validation, routing policy and the IPv4
//! header check. Pure logic, headless: no socket, interface or clock is involved, and every packet is built in memory.

use bifrost_core::tunnel::{
    classify_packet, parse_ipv4, IllegalTransition, Ipv4Route, PacketError, RouteAction, RoutePolicy, StopReason, TunnelError,
    TunnelSession, TunnelState, VpnProfile, VpnProtocol,
};
use std::net::Ipv4Addr;

#[test]
fn a_session_connects_renegotiates_and_stops() {
    let mut s = TunnelSession::new();
    assert_eq!(s.state(), TunnelState::Disconnected);
    s.start().unwrap();
    assert_eq!(s.state(), TunnelState::Connecting);
    s.established().unwrap();
    assert_eq!(s.connect_count(), 1);
    s.reassert().unwrap();
    assert_eq!(s.state(), TunnelState::Reasserting);
    s.established().unwrap();
    assert_eq!(s.state(), TunnelState::Connected);
    assert_eq!(s.connect_count(), 1, "renegotiating after a network change is not a new connection");
    s.stop(StopReason::UserRequested).unwrap();
    assert_eq!(s.state(), TunnelState::Disconnecting);
    s.stopped().unwrap();
    assert_eq!(s.state(), TunnelState::Disconnected);
    assert_eq!(s.stop_reason(), Some(StopReason::UserRequested), "the reason survives the return to Disconnected");
}

#[test]
fn an_illegal_call_is_refused_and_leaves_the_state_alone() {
    let mut s = TunnelSession::new();
    let err = s.established().unwrap_err();
    assert_eq!(err, IllegalTransition { from: TunnelState::Disconnected, action: "finish connecting" });
    assert_eq!(err.to_string(), "cannot finish connecting a tunnel that is Disconnected");
    assert_eq!(s.state(), TunnelState::Disconnected);
    s.start().unwrap();
    assert!(s.start().is_err(), "a second start while connecting is refused");
    assert_eq!(s.state(), TunnelState::Connecting);
}

#[test]
fn a_failure_records_its_error_and_stops_with_provider_failed() {
    let mut s = TunnelSession::new();
    s.start().unwrap();
    s.established().unwrap();
    s.fail(TunnelError::Timeout);
    assert_eq!(s.state(), TunnelState::Disconnecting);
    assert_eq!(s.stop_reason(), Some(StopReason::ProviderFailed));
    assert_eq!(s.last_error(), Some(&TunnelError::Timeout));
    s.stopped().unwrap();
    s.fail(TunnelError::Cancelled);
    assert_eq!(s.last_error(), Some(&TunnelError::Timeout), "a failure once the tunnel is down is ignored");
    s.start().unwrap();
    assert_eq!(s.last_error(), None, "a new start clears the previous error");
}

#[test]
fn tunnel_errors_read_as_user_messages() {
    assert_eq!(TunnelError::InvalidConfiguration("no server".into()).to_string(), "invalid tunnel configuration: no server");
    assert_eq!(TunnelError::ServerUnreachable.to_string(), "the tunnel server cannot be reached");
    assert_eq!(TunnelError::AuthenticationFailed.to_string(), "the tunnel server rejected the credentials");
    assert_eq!(TunnelError::Internal("boom".into()).to_string(), "tunnel failure: boom");
}

#[test]
fn a_profile_takes_the_protocol_port_and_normalises_its_server() {
    let p = VpnProfile::new("  Office  ", "  VPN.Example.Test ", VpnProtocol::Ikev2);
    assert_eq!((p.name.as_str(), p.server.as_str(), p.port), ("Office", "vpn.example.test", 500));
    assert!(!p.on_demand);
    assert_eq!(VpnProtocol::Tunnel.default_port(), 443);
    assert_eq!(VpnProtocol::IpsecIkev1.default_port(), 500);
    assert_eq!(p.validate(), Ok(()));
}

#[test]
fn a_profile_with_a_scheme_path_space_or_no_port_is_refused() {
    let base = VpnProfile::new("Office", "vpn.example.test", VpnProtocol::Tunnel);
    let mut url = base.clone();
    url.server = "https://vpn.example.test".into();
    assert!(matches!(url.validate(), Err(TunnelError::InvalidConfiguration(_))));
    let mut path = base.clone();
    path.server = "vpn.example.test/x".into();
    assert!(path.validate().is_err());
    let mut space = base.clone();
    space.server = "vpn example".into();
    assert!(space.validate().is_err());
    let mut port = base.clone();
    port.port = 0;
    assert!(port.validate().is_err());
    let mut unnamed = base.clone();
    unnamed.name.clear();
    assert_eq!(unnamed.validate(), Err(TunnelError::InvalidConfiguration("the profile needs a name".into())));
    let mut no_server = base;
    no_server.server.clear();
    assert_eq!(no_server.validate(), Err(TunnelError::InvalidConfiguration("the profile needs a server".into())));
}

#[test]
fn a_route_clears_host_bits_and_tests_membership() {
    let r = Ipv4Route::parse("10.1.2.3/8").unwrap();
    assert_eq!((r.network(), r.prefix()), (Ipv4Addr::new(10, 0, 0, 0), 8));
    assert!(r.contains(Ipv4Addr::new(10, 200, 0, 1)));
    assert!(!r.contains(Ipv4Addr::new(11, 0, 0, 1)));
    let host = Ipv4Route::parse("192.168.1.9").unwrap();
    assert_eq!(host.prefix(), 32, "a bare address is one host");
    assert!(host.contains(Ipv4Addr::new(192, 168, 1, 9)));
    assert!(!host.contains(Ipv4Addr::new(192, 168, 1, 10)));
    assert!(Ipv4Route::parse("0.0.0.0/0").unwrap().contains(Ipv4Addr::new(8, 8, 8, 8)));
    assert!(Ipv4Route::parse("10.0.0.0/33").is_none());
    assert!(Ipv4Route::parse("nonsense").is_none());
}

#[test]
fn the_longest_matching_route_decides_the_destination() {
    let mut policy = RoutePolicy::new(RouteAction::Tunnel);
    policy.exclude(Ipv4Route::parse("192.168.0.0/16").unwrap());
    policy.include(Ipv4Route::parse("192.168.5.0/24").unwrap());
    assert_eq!(policy.decide(Ipv4Addr::new(8, 8, 8, 8)), RouteAction::Tunnel, "the default covers other destinations");
    assert_eq!(policy.decide(Ipv4Addr::new(192, 168, 1, 1)), RouteAction::Direct, "the local network stays off the tunnel");
    assert_eq!(policy.decide(Ipv4Addr::new(192, 168, 5, 7)), RouteAction::Tunnel, "the more specific include wins");
}

#[test]
fn between_routes_of_equal_length_the_later_one_wins() {
    let mut policy = RoutePolicy::new(RouteAction::Direct);
    policy.include(Ipv4Route::parse("203.0.113.0/24").unwrap());
    policy.exclude(Ipv4Route::parse("203.0.113.0/24").unwrap());
    assert_eq!(policy.decide(Ipv4Addr::new(203, 0, 113, 9)), RouteAction::Direct);
}

/// A 20-byte IPv4 header with a correct checksum for a packet of `total` bytes in all.
fn ipv4_header(src: [u8; 4], dst: [u8; 4], protocol: u8, total: u16) -> Vec<u8> {
    let mut h = vec![0x45, 0, (total >> 8) as u8, total as u8, 0, 0, 0, 0, 64, protocol, 0, 0];
    h.extend_from_slice(&src);
    h.extend_from_slice(&dst);
    let mut sum: u32 = 0;
    for pair in h.chunks(2) {
        sum += u16::from_be_bytes([pair[0], pair[1]]) as u32;
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    let check = !(sum as u16);
    h[10..12].copy_from_slice(&check.to_be_bytes());
    h
}

#[test]
fn a_well_formed_header_is_read_and_its_checksum_checked() {
    let mut packet = ipv4_header([10, 0, 0, 2], [93, 184, 216, 34], 6, 24);
    packet.extend_from_slice(b"data");
    let h = parse_ipv4(&packet).unwrap();
    assert_eq!(h.source, Ipv4Addr::new(10, 0, 0, 2));
    assert_eq!(h.destination, Ipv4Addr::new(93, 184, 216, 34));
    assert_eq!((h.protocol, h.total_length, h.header_length), (6, 24, 20));
    packet[15] ^= 0xFF;
    assert_eq!(parse_ipv4(&packet), Err(PacketError::BadChecksum));
}

#[test]
fn malformed_headers_are_named_by_their_fault() {
    assert_eq!(parse_ipv4(&[0x45; 19]), Err(PacketError::TooShort));
    let mut v6 = ipv4_header([1, 1, 1, 1], [2, 2, 2, 2], 17, 20);
    v6[0] = 0x60;
    assert_eq!(parse_ipv4(&v6), Err(PacketError::NotIpv4));
    let mut short_ihl = ipv4_header([1, 1, 1, 1], [2, 2, 2, 2], 17, 20);
    short_ihl[0] = 0x44;
    assert_eq!(parse_ipv4(&short_ihl), Err(PacketError::BadHeaderLength));
    let too_long = ipv4_header([1, 1, 1, 1], [2, 2, 2, 2], 17, 100);
    assert_eq!(parse_ipv4(&too_long), Err(PacketError::BadTotalLength));
}

#[test]
fn a_packet_is_routed_by_its_destination() {
    let mut policy = RoutePolicy::new(RouteAction::Tunnel);
    policy.exclude(Ipv4Route::parse("10.0.0.0/8").unwrap());
    let local = ipv4_header([10, 0, 0, 2], [10, 1, 1, 1], 17, 20);
    let remote = ipv4_header([10, 0, 0, 2], [1, 1, 1, 1], 17, 20);
    assert_eq!(classify_packet(&local, &policy), Ok(RouteAction::Direct));
    assert_eq!(classify_packet(&remote, &policy), Ok(RouteAction::Tunnel));
    assert_eq!(classify_packet(&[0u8; 4], &policy), Err(PacketError::TooShort));
}
