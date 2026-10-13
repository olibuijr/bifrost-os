//! Custom VPN tunnels: the life cycle of one tunnel session, the error categories a tunnel reports, the profile a
//! user saves for a tunnel server, the routing policy that decides which IPv4 destinations go into the tunnel, and
//! a header check for the IP packets read from a tunnel's virtual interface.
//!
//! The model owns no thread, no socket and no interface. A tunnel provider (a program that speaks its own protocol to
//! a server) drives [`TunnelSession`] with its own events, asks [`RoutePolicy`] where each packet goes, and uses
//! [`parse_ipv4`] to read the addresses and checksum of the packets it takes from the interface. Nothing here sends
//! a packet. The split between "include" and "exclude" routes follows the usual full-tunnel and split-tunnel
//! layouts: a full tunnel sends everything by default and excludes the local network, a split tunnel sends only the
//! included networks.
use std::fmt;
use std::net::Ipv4Addr;

/// Where a tunnel session is in its life cycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TunnelState {
    Disconnected,
    Connecting,
    Connected,
    /// The network under the tunnel changed and the provider is negotiating again.
    Reasserting,
    Disconnecting,
}

/// Why a session was asked to stop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopReason {
    UserRequested,
    ProviderFailed,
    NoNetwork,
    Idle,
    Restarted,
}

/// Failure categories a tunnel reports to the user and to its caller.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TunnelError {
    InvalidConfiguration(String),
    ServerUnreachable,
    AuthenticationFailed,
    Timeout,
    Cancelled,
    Internal(String),
}

impl fmt::Display for TunnelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TunnelError::InvalidConfiguration(why) => write!(f, "invalid tunnel configuration: {why}"),
            TunnelError::ServerUnreachable => write!(f, "the tunnel server cannot be reached"),
            TunnelError::AuthenticationFailed => write!(f, "the tunnel server rejected the credentials"),
            TunnelError::Timeout => write!(f, "the tunnel did not come up in time"),
            TunnelError::Cancelled => write!(f, "the tunnel was cancelled"),
            TunnelError::Internal(what) => write!(f, "tunnel failure: {what}"),
        }
    }
}

/// A life-cycle call that the current state does not allow, for example `start` on a connected session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IllegalTransition {
    pub from: TunnelState,
    pub action: &'static str,
}

impl fmt::Display for IllegalTransition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "cannot {} a tunnel that is {:?}", self.action, self.from)
    }
}

/// One tunnel session. Every call either moves the state along the life cycle or returns [`IllegalTransition`] and
/// leaves the state as it was.
///
/// The life cycle is `Disconnected -> Connecting -> Connected`, with `Connected <-> Reasserting`, and
/// `Connecting | Connected | Reasserting -> Disconnecting -> Disconnected` on stop or failure.
#[derive(Clone, Debug)]
pub struct TunnelSession {
    state: TunnelState,
    stop_reason: Option<StopReason>,
    last_error: Option<TunnelError>,
    connects: u32,
}

impl Default for TunnelSession {
    fn default() -> Self {
        TunnelSession { state: TunnelState::Disconnected, stop_reason: None, last_error: None, connects: 0 }
    }
}

impl TunnelSession {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn state(&self) -> TunnelState {
        self.state
    }

    /// The reason for the last stop, kept after the session returns to `Disconnected`.
    pub fn stop_reason(&self) -> Option<StopReason> {
        self.stop_reason
    }

    /// The error that ended the last failed session, if any.
    pub fn last_error(&self) -> Option<&TunnelError> {
        self.last_error.as_ref()
    }

    /// How many times the session reached `Connected`.
    pub fn connect_count(&self) -> u32 {
        self.connects
    }

    /// Begin connecting. Only from `Disconnected`; a new start clears the previous error.
    pub fn start(&mut self) -> Result<(), IllegalTransition> {
        self.expect(self.state == TunnelState::Disconnected, "start")?;
        self.state = TunnelState::Connecting;
        self.stop_reason = None;
        self.last_error = None;
        Ok(())
    }

    /// The provider finished negotiating. From `Connecting`, or from `Reasserting` after a network change.
    pub fn established(&mut self) -> Result<(), IllegalTransition> {
        let ok = matches!(self.state, TunnelState::Connecting | TunnelState::Reasserting);
        self.expect(ok, "finish connecting")?;
        if self.state == TunnelState::Connecting {
            self.connects += 1;
        }
        self.state = TunnelState::Connected;
        Ok(())
    }

    /// The network under a connected tunnel changed. The provider keeps the session up while it renegotiates.
    pub fn reassert(&mut self) -> Result<(), IllegalTransition> {
        self.expect(self.state == TunnelState::Connected, "reassert")?;
        self.state = TunnelState::Reasserting;
        Ok(())
    }

    /// Ask an active session to stop. The state becomes `Disconnecting` until [`TunnelSession::stopped`].
    pub fn stop(&mut self, reason: StopReason) -> Result<(), IllegalTransition> {
        let active = matches!(
            self.state,
            TunnelState::Connecting | TunnelState::Connected | TunnelState::Reasserting
        );
        self.expect(active, "stop")?;
        self.state = TunnelState::Disconnecting;
        self.stop_reason = Some(reason);
        Ok(())
    }

    /// The session failed. It records the error and starts the stop. Unlike [`TunnelSession::stop`], this is
    /// allowed while the session is connecting, connected or reasserting, and it is ignored once it is down.
    pub fn fail(&mut self, error: TunnelError) {
        if matches!(
            self.state,
            TunnelState::Connecting | TunnelState::Connected | TunnelState::Reasserting
        ) {
            self.state = TunnelState::Disconnecting;
            self.stop_reason = Some(StopReason::ProviderFailed);
            self.last_error = Some(error);
        }
    }

    /// The provider finished tearing down. From `Disconnecting` only.
    pub fn stopped(&mut self) -> Result<(), IllegalTransition> {
        self.expect(self.state == TunnelState::Disconnecting, "finish stopping")?;
        self.state = TunnelState::Disconnected;
        Ok(())
    }

    fn expect(&self, allowed: bool, action: &'static str) -> Result<(), IllegalTransition> {
        if allowed {
            Ok(())
        } else {
            Err(IllegalTransition { from: self.state, action })
        }
    }
}

/// The tunnel protocol a profile names. `Tunnel` is a protocol the provider implements itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VpnProtocol {
    Ikev2,
    IpsecIkev1,
    Tunnel,
}

impl VpnProtocol {
    /// The UDP port the protocol normally uses when the profile does not name one.
    pub fn default_port(self) -> u16 {
        match self {
            VpnProtocol::Ikev2 | VpnProtocol::IpsecIkev1 => 500,
            VpnProtocol::Tunnel => 443,
        }
    }
}

/// A saved tunnel server. It holds no secrets: credentials belong in the keychain, and the profile names them only
/// through the caller.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VpnProfile {
    pub name: String,
    pub server: String,
    pub port: u16,
    pub protocol: VpnProtocol,
    /// Bring the tunnel up when a network that is not in the trusted list appears.
    pub on_demand: bool,
}

impl VpnProfile {
    /// A profile for `server` with the protocol's default port.
    pub fn new(name: &str, server: &str, protocol: VpnProtocol) -> Self {
        VpnProfile {
            name: name.trim().to_string(),
            server: server.trim().to_ascii_lowercase(),
            port: protocol.default_port(),
            protocol,
            on_demand: false,
        }
    }

    /// Check the profile before it is saved. The server must be a host name or an IPv4 address with no scheme, path
    /// or spaces, and the name and port must be usable.
    pub fn validate(&self) -> Result<(), TunnelError> {
        let bad = |why: &str| Err(TunnelError::InvalidConfiguration(why.to_string()));
        if self.name.is_empty() {
            return bad("the profile needs a name");
        }
        if self.server.is_empty() {
            return bad("the profile needs a server");
        }
        if self.server.contains("://") || self.server.contains('/') || self.server.chars().any(char::is_whitespace) {
            return bad("the server must be a host name or an address, without a scheme or path");
        }
        if self.port == 0 {
            return bad("the port must be between 1 and 65535");
        }
        Ok(())
    }
}

/// An IPv4 network: a masked network address and a prefix length from 0 to 32.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ipv4Route {
    network: Ipv4Addr,
    prefix: u8,
}

impl Ipv4Route {
    /// Parse `a.b.c.d/len`, or a bare address as a single host (`/32`). Host bits are cleared.
    pub fn parse(text: &str) -> Option<Ipv4Route> {
        let (addr, prefix) = match text.trim().split_once('/') {
            Some((addr, len)) => (addr, len.parse::<u8>().ok()?),
            None => (text.trim(), 32),
        };
        if prefix > 32 {
            return None;
        }
        let addr: Ipv4Addr = addr.parse().ok()?;
        let network = Ipv4Addr::from(u32::from(addr) & mask(prefix));
        Some(Ipv4Route { network, prefix })
    }

    pub fn network(&self) -> Ipv4Addr {
        self.network
    }

    pub fn prefix(&self) -> u8 {
        self.prefix
    }

    pub fn contains(&self, addr: Ipv4Addr) -> bool {
        (u32::from(addr) & mask(self.prefix)) == u32::from(self.network)
    }
}

fn mask(prefix: u8) -> u32 {
    if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - prefix as u32)
    }
}

/// Whether a destination's packets go into the tunnel or leave on the normal network.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RouteAction {
    Tunnel,
    Direct,
}

/// Include and exclude routes over a default action. For a destination, the matching route with the longest
/// prefix decides. Between two routes of the same length, the one added later wins.
#[derive(Clone, Debug)]
pub struct RoutePolicy {
    default: RouteAction,
    rules: Vec<(Ipv4Route, RouteAction)>,
}

impl RoutePolicy {
    /// A policy that sends every destination `default` unless a rule says otherwise.
    pub fn new(default: RouteAction) -> Self {
        RoutePolicy { default, rules: Vec::new() }
    }

    /// Send the network through the tunnel.
    pub fn include(&mut self, route: Ipv4Route) {
        self.rules.push((route, RouteAction::Tunnel));
    }

    /// Keep the network off the tunnel, for example the local network under a full tunnel.
    pub fn exclude(&mut self, route: Ipv4Route) {
        self.rules.push((route, RouteAction::Direct));
    }

    pub fn decide(&self, addr: Ipv4Addr) -> RouteAction {
        let mut best: Option<(u8, RouteAction)> = None;
        for (route, action) in &self.rules {
            if route.contains(addr) && best.is_none_or( |(prefix, _)| route.prefix() >= prefix) {
                best = Some((route.prefix(), *action));
            }
        }
        best.map_or(self.default, |(_, action)| action)
    }
}

/// Why a packet's header was rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PacketError {
    TooShort,
    NotIpv4,
    BadHeaderLength,
    BadTotalLength,
    BadChecksum,
}

/// The fields of an IPv4 header that routing and the tunnel need.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ipv4Header {
    pub source: Ipv4Addr,
    pub destination: Ipv4Addr,
    pub protocol: u8,
    pub total_length: u16,
    pub header_length: usize,
}

/// Ones' complement sum of big-endian 16-bit words, with the carries folded back in.
fn ones_complement_sum(bytes: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    for pair in bytes.chunks(2) {
        let word = if pair.len() == 2 { u16::from_be_bytes([pair[0], pair[1]]) } else { u16::from_be_bytes([pair[0], 0]) };
        sum += word as u32;
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    sum as u16
}

/// Read and check an IPv4 header from the start of `packet`. The header checksum must be valid, and the total length
/// must fit in the bytes given. Payload bytes are not inspected.
pub fn parse_ipv4(packet: &[u8]) -> Result<Ipv4Header, PacketError> {
    if packet.len() < 20 {
        return Err(PacketError::TooShort);
    }
    if packet[0] >> 4 != 4 {
        return Err(PacketError::NotIpv4);
    }
    let header_length = (packet[0] & 0x0F) as usize * 4;
    if header_length < 20 || header_length > packet.len() {
        return Err(PacketError::BadHeaderLength);
    }
    let total_length = u16::from_be_bytes([packet[2], packet[3]]);
    if (total_length as usize) < header_length || (total_length as usize) > packet.len() {
        return Err(PacketError::BadTotalLength);
    }
    if ones_complement_sum(&packet[..header_length]) != 0xFFFF {
        return Err(PacketError::BadChecksum);
    }
    Ok(Ipv4Header {
        source: Ipv4Addr::new(packet[12], packet[13], packet[14], packet[15]),
        destination: Ipv4Addr::new(packet[16], packet[17], packet[18], packet[19]),
        protocol: packet[9],
        total_length,
        header_length,
    })
}

/// The tunnel's decision for one packet read from the virtual interface.
pub fn classify_packet(packet: &[u8], policy: &RoutePolicy) -> Result<RouteAction, PacketError> {
    let header = parse_ipv4(packet)?;
    Ok(policy.decide(header.destination))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 20-byte IPv4 header with a correct checksum.
    fn header(src: [u8; 4], dst: [u8; 4], protocol: u8, total: u16) -> Vec<u8> {
        let mut h = vec![0x45, 0, (total >> 8) as u8, total as u8, 0, 0, 0, 0, 64, protocol, 0, 0];
        h.extend_from_slice(&src);
        h.extend_from_slice(&dst);
        let sum = !ones_complement_sum(&h);
        h[10] = (sum >> 8) as u8;
        h[11] = sum as u8;
        h
    }

    #[test]
    fn a_session_follows_the_life_cycle_and_counts_connections() {
        let mut s = TunnelSession::new();
        assert_eq!(s.state(), TunnelState::Disconnected);
        s.start().unwrap();
        s.established().unwrap();
        assert_eq!(s.state(), TunnelState::Connected);
        s.reassert().unwrap();
        s.established().unwrap();
        s.stop(StopReason::UserRequested).unwrap();
        assert_eq!(s.state(), TunnelState::Disconnecting);
        s.stopped().unwrap();
        assert_eq!(s.state(), TunnelState::Disconnected);
        assert_eq!(s.stop_reason(), Some(StopReason::UserRequested));
        assert_eq!(s.connect_count(), 1, "a renegotiation is not a new connection");
    }

    #[test]
    fn an_illegal_call_is_refused_and_leaves_the_state_alone() {
        let mut s = TunnelSession::new();
        let err = s.established().unwrap_err();
        assert_eq!(err, IllegalTransition { from: TunnelState::Disconnected, action: "finish connecting" });
        s.start().unwrap();
        assert!(s.start().is_err());
        assert_eq!(s.state(), TunnelState::Connecting);
        assert!(s.stopped().is_err());
    }

    #[test]
    fn a_failure_records_the_error_and_the_next_start_clears_it() {
        let mut s = TunnelSession::new();
        s.start().unwrap();
        s.fail(TunnelError::AuthenticationFailed);
        assert_eq!(s.state(), TunnelState::Disconnecting);
        assert_eq!(s.stop_reason(), Some(StopReason::ProviderFailed));
        assert_eq!(s.last_error(), Some(&TunnelError::AuthenticationFailed));
        s.stopped().unwrap();
        s.fail(TunnelError::Timeout);
        assert_eq!(s.last_error(), Some(&TunnelError::AuthenticationFailed), "a failure while down is ignored");
        s.start().unwrap();
        assert_eq!(s.last_error(), None);
        assert!(TunnelError::Timeout.to_string().contains("in time"));
    }

    #[test]
    fn the_longest_matching_route_decides() {
        let mut policy = RoutePolicy::new(RouteAction::Tunnel);
        policy.exclude(Ipv4Route::parse("192.168.0.0/16").unwrap());
        policy.include(Ipv4Route::parse("192.168.44.0/24").unwrap());
        assert_eq!(policy.decide("8.8.8.8".parse().unwrap()), RouteAction::Tunnel);
        assert_eq!(policy.decide("192.168.1.9".parse().unwrap()), RouteAction::Direct);
        assert_eq!(policy.decide("192.168.44.7".parse().unwrap()), RouteAction::Tunnel);
    }

    #[test]
    fn a_split_tunnel_sends_only_the_included_networks() {
        let mut policy = RoutePolicy::new(RouteAction::Direct);
        policy.include(Ipv4Route::parse("10.20.0.0/16").unwrap());
        assert_eq!(policy.decide("10.20.3.4".parse().unwrap()), RouteAction::Tunnel);
        assert_eq!(policy.decide("10.21.0.1".parse().unwrap()), RouteAction::Direct);
    }

    #[test]
    fn route_parsing_masks_host_bits_and_rejects_bad_prefixes() {
        let r = Ipv4Route::parse("10.1.2.3/8").unwrap();
        assert_eq!(r.network(), Ipv4Addr::new(10, 0, 0, 0));
        assert!(r.contains(Ipv4Addr::new(10, 255, 0, 1)));
        assert!(!r.contains(Ipv4Addr::new(11, 0, 0, 1)));
        assert_eq!(Ipv4Route::parse("10.0.0.1").unwrap().prefix(), 32);
        assert!(Ipv4Route::parse("10.0.0.0/33").is_none());
        assert!(Ipv4Route::parse("10.0.0/8").is_none());
        assert!(Ipv4Route::parse("0.0.0.0/0").unwrap().contains(Ipv4Addr::new(203, 0, 113, 9)));
    }

    #[test]
    fn a_packet_header_is_read_and_its_destination_routed() {
        let mut policy = RoutePolicy::new(RouteAction::Direct);
        policy.include(Ipv4Route::parse("10.0.0.0/8").unwrap());
        let good = header([10, 0, 0, 2], [10, 9, 8, 7], 17, 20);
        let parsed = parse_ipv4(&good).unwrap();
        assert_eq!(parsed.source, Ipv4Addr::new(10, 0, 0, 2));
        assert_eq!(parsed.protocol, 17);
        assert_eq!(classify_packet(&good, &policy), Ok(RouteAction::Tunnel));
    }

    #[test]
    fn a_corrupt_or_truncated_packet_is_rejected() {
        let mut bad = header([10, 0, 0, 2], [10, 9, 8, 7], 6, 20);
        bad[8] ^= 0x01;
        assert_eq!(parse_ipv4(&bad), Err(PacketError::BadChecksum));
        assert_eq!(parse_ipv4(&[0x45; 10]), Err(PacketError::TooShort));
        let mut v6 = header([10, 0, 0, 2], [10, 9, 8, 7], 6, 20);
        v6[0] = 0x65;
        assert_eq!(parse_ipv4(&v6), Err(PacketError::NotIpv4));
        let long = header([10, 0, 0, 2], [10, 9, 8, 7], 6, 60);
        assert_eq!(parse_ipv4(&long), Err(PacketError::BadTotalLength));
    }

    #[test]
    fn a_profile_is_validated_before_it_is_saved() {
        let ok = VpnProfile::new(" Office ", "VPN.Example.com ", VpnProtocol::Ikev2);
        assert_eq!(ok.name, "Office");
        assert_eq!(ok.server, "vpn.example.com");
        assert_eq!(ok.port, 500);
        assert_eq!(ok.validate(), Ok(()));
        let mut url = ok.clone();
        url.server = "https://vpn.example.com/x".into();
        assert!(matches!(url.validate(), Err(TunnelError::InvalidConfiguration(_))));
        let mut no_port = ok.clone();
        no_port.port = 0;
        assert!(no_port.validate().is_err());
        assert!(VpnProfile::new("", "a.example", VpnProtocol::Tunnel).validate().is_err());
    }
}
