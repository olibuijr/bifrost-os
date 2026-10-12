//! Endpoints, transport parameters, and connection and listener states (std only).
//!
//! An endpoint names where a connection goes: a host and port, a DNS-SD service instance, or a
//! local socket path. `Parameters` chooses the transport (TCP, TLS over TCP, or UDP) and the TCP
//! options. `Connection` and `Listener` hold the state of one connection or listener and move it
//! through the states an app observes: setup, preparing, ready, waiting, failed and cancelled.
//! This module opens no socket. The transport owner feeds it events and reads the state back.

use std::fmt;

/// Highest TCP or UDP port number.
const MAX_PORT: u32 = 65535;

/// The transport protocol under a connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transport {
    Tcp,
    Udp,
}

/// Where a connection or listener goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Endpoint {
    /// A host name or IP address with a port.
    HostPort { host: String, port: u16 },
    /// A DNS-SD service instance, found on the local network.
    Service { name: String, kind: String, domain: String },
    /// A local UNIX socket path.
    Unix(String),
}

/// Why an endpoint string was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndpointError {
    /// The string is empty.
    Empty,
    /// There is no `:port` after the host.
    MissingPort,
    /// The host is empty, unclosed, or an IPv6 address without brackets.
    BadHost,
    /// The port is not a number from 1 to 65535.
    BadPort,
}

impl fmt::Display for EndpointError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            EndpointError::Empty => "endpoint is empty",
            EndpointError::MissingPort => "endpoint has no port",
            EndpointError::BadHost => "endpoint host is invalid",
            EndpointError::BadPort => "endpoint port must be 1 to 65535",
        };
        f.write_str(text)
    }
}

impl std::error::Error for EndpointError {}

impl Endpoint {
    /// Parses `host:port` or `[ipv6]:port`. The port must be 1 to 65535, and a bare IPv6
    /// address without brackets is refused because its colons are ambiguous.
    pub fn host_port(s: &str) -> Result<Endpoint, EndpointError> {
        let s = s.trim();
        if s.is_empty() {
            return Err(EndpointError::Empty);
        }
        let (host, port_str) = if let Some(rest) = s.strip_prefix('[') {
            let close = rest.find(']').ok_or(EndpointError::BadHost)?;
            let after = &rest[close + 1..];
            let port_str = after.strip_prefix(':').ok_or(EndpointError::MissingPort)?;
            (&rest[..close], port_str)
        } else {
            let colon = s.rfind(':').ok_or(EndpointError::MissingPort)?;
            let host = &s[..colon];
            if host.contains(':') {
                return Err(EndpointError::BadHost);
            }
            (host, &s[colon + 1..])
        };
        if host.is_empty() || host.contains(char::is_whitespace) {
            return Err(EndpointError::BadHost);
        }
        if port_str.is_empty() || !port_str.bytes().all(|b| b.is_ascii_digit()) {
            return Err(EndpointError::BadPort);
        }
        let port: u32 = port_str.parse().map_err(|_| EndpointError::BadPort)?;
        if port == 0 || port > MAX_PORT {
            return Err(EndpointError::BadPort);
        }
        Ok(Endpoint::HostPort { host: host.to_string(), port: port as u16 })
    }
}

/// TCP settings. Every field is optional or off by default.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TcpOptions {
    /// Disables Nagle's algorithm, so small writes leave at once.
    pub no_delay: bool,
    /// Idle seconds before keepalive probes start. `None` leaves keepalives off.
    pub keepalive_idle_secs: Option<u32>,
    /// Seconds to wait for the handshake. `None` uses the system default.
    pub connection_timeout_secs: Option<u32>,
}

/// Why a parameter set was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParametersError {
    /// TCP options were given for a UDP transport, which has none.
    TcpOptionsOnUdp,
    /// A timeout or idle interval is zero seconds.
    ZeroTimeout,
}

/// The protocol stack and options a connection or listener is built with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Parameters {
    pub transport: Transport,
    /// Wraps the TCP stream in TLS. Only meaningful for `Transport::Tcp`.
    pub tls: bool,
    pub tcp: TcpOptions,
}

impl Parameters {
    /// Plain TCP with default options.
    pub fn tcp() -> Self {
        Parameters { transport: Transport::Tcp, tls: false, tcp: TcpOptions::default() }
    }

    /// TLS over TCP with default options.
    pub fn tls() -> Self {
        Parameters { tls: true, ..Self::tcp() }
    }

    /// UDP with no TCP options.
    pub fn udp() -> Self {
        Parameters { transport: Transport::Udp, tls: false, tcp: TcpOptions::default() }
    }

    /// Replaces the TCP options. A UDP parameter set refuses them, and a zero
    /// keepalive idle or connection timeout is refused because the stack cannot use it.
    pub fn with_tcp(mut self, tcp: TcpOptions) -> Result<Self, ParametersError> {
        if self.transport == Transport::Udp {
            return Err(ParametersError::TcpOptionsOnUdp);
        }
        if tcp.keepalive_idle_secs == Some(0) || tcp.connection_timeout_secs == Some(0) {
            return Err(ParametersError::ZeroTimeout);
        }
        self.tcp = tcp;
        Ok(self)
    }
}

/// The state of one connection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConnState {
    /// Created, not started.
    Setup,
    /// Being established.
    Preparing,
    /// Established and able to send and receive.
    Ready,
    /// Paused until the path changes. Carries the reason.
    Waiting(String),
    /// Ended by an error. Terminal.
    Failed(String),
    /// Cancelled by the owner. Terminal.
    Cancelled,
}

/// An input to a connection's state machine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConnEvent {
    /// The owner starts the connection.
    Start,
    /// The owner restarts a connection that is waiting.
    Restart,
    /// The handshake completed.
    Established,
    /// The network path went away. Carries the reason.
    PathUnavailable(String),
    /// The connection hit a fatal error. Carries the reason.
    Error(String),
    /// The owner cancels the connection.
    Cancel,
}

/// One connection to an endpoint with fixed parameters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Connection {
    endpoint: Endpoint,
    parameters: Parameters,
    state: ConnState,
}

impl Connection {
    /// A connection in the setup state.
    pub fn new(endpoint: Endpoint, parameters: Parameters) -> Self {
        Connection { endpoint, parameters, state: ConnState::Setup }
    }

    /// The remote endpoint the connection was created with.
    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    /// The parameters the connection was created with.
    pub fn parameters(&self) -> &Parameters {
        &self.parameters
    }

    /// The current state.
    pub fn state(&self) -> &ConnState {
        &self.state
    }

    /// Applies an event. Returns false when the event does not apply in the current state.
    /// `Failed` and `Cancelled` are terminal, so every event after them is refused.
    pub fn handle(&mut self, event: ConnEvent) -> bool {
        use ConnEvent as E;
        use ConnState as S;
        let next = match (&self.state, event) {
            (S::Setup, E::Start) => S::Preparing,
            (S::Preparing, E::Established) => S::Ready,
            (S::Preparing, E::PathUnavailable(r)) | (S::Ready, E::PathUnavailable(r)) => S::Waiting(r),
            (S::Waiting(_), E::Restart) => S::Preparing,
            (S::Preparing | S::Ready | S::Waiting(_), E::Error(r)) => S::Failed(r),
            (S::Setup | S::Preparing | S::Ready | S::Waiting(_), E::Cancel) => S::Cancelled,
            _ => return false,
        };
        self.state = next;
        true
    }
}

/// The state of a listener.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ListenerState {
    /// Created, not started.
    Setup,
    /// Bound and accepting connections.
    Ready,
    /// Waiting for a usable network. Carries the reason.
    Waiting(String),
    /// Stopped by a fatal error. Terminal.
    Failed(String),
    /// Cancelled by the owner. Terminal.
    Cancelled,
}

/// An input to a listener's state machine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ListenEvent {
    /// The owner starts listening.
    Start,
    /// The network became usable again.
    PathRestored,
    /// The network went away. Carries the reason.
    PathUnavailable(String),
    /// A fatal error. Carries the reason.
    Error(String),
    /// The owner stops the listener.
    Cancel,
}

/// A listener that accepts inbound connections on one port.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listener {
    port: u16,
    parameters: Parameters,
    state: ListenerState,
    /// Inbound connections still allowed. `None` is unlimited.
    limit: Option<u32>,
}

impl Listener {
    /// A listener in the setup state. Port 0 asks the system to choose a port.
    pub fn new(port: u16, parameters: Parameters) -> Self {
        Listener { port, parameters, state: ListenerState::Setup, limit: None }
    }

    /// The requested port.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// The parameters the listener was created with.
    pub fn parameters(&self) -> &Parameters {
        &self.parameters
    }

    /// The current state.
    pub fn state(&self) -> &ListenerState {
        &self.state
    }

    /// Sets how many more inbound connections to accept. `None` removes the limit.
    pub fn set_connection_limit(&mut self, limit: Option<u32>) {
        self.limit = limit;
    }

    /// Applies an event. Returns false when the event does not apply in the current state.
    pub fn handle(&mut self, event: ListenEvent) -> bool {
        use ListenEvent as E;
        use ListenerState as S;
        let next = match (&self.state, event) {
            (S::Setup, E::Start) => S::Ready,
            (S::Waiting(_), E::PathRestored) => S::Ready,
            (S::Ready, E::PathUnavailable(r)) => S::Waiting(r),
            (S::Ready | S::Waiting(_), E::Error(r)) => S::Failed(r),
            (S::Setup | S::Ready | S::Waiting(_), E::Cancel) => S::Cancelled,
            _ => return false,
        };
        self.state = next;
        true
    }

    /// Takes one inbound connection from the limit. Returns true when the connection
    /// may be accepted. Only a ready listener accepts, and a limit of zero refuses.
    pub fn accept(&mut self) -> bool {
        if self.state != ListenerState::Ready {
            return false;
        }
        let remaining = self.limit;
        match remaining {
            None => true,
            Some(0) => false,
            Some(n) => {
                self.limit = Some(n - 1);
                true
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_port_parses_names_and_bracketed_ipv6() {
        assert_eq!(
            Endpoint::host_port("example.com:443"),
            Ok(Endpoint::HostPort { host: "example.com".to_string(), port: 443 })
        );
        assert_eq!(
            Endpoint::host_port("[::1]:8080"),
            Ok(Endpoint::HostPort { host: "::1".to_string(), port: 8080 })
        );
    }

    #[test]
    fn host_port_refuses_bad_forms() {
        assert_eq!(Endpoint::host_port(""), Err(EndpointError::Empty));
        assert_eq!(Endpoint::host_port("example.com"), Err(EndpointError::MissingPort));
        assert_eq!(Endpoint::host_port("fe80::1:80"), Err(EndpointError::BadHost));
        assert_eq!(Endpoint::host_port("example.com:0"), Err(EndpointError::BadPort));
        assert_eq!(Endpoint::host_port("example.com:65536"), Err(EndpointError::BadPort));
        assert_eq!(Endpoint::host_port("example.com:+80"), Err(EndpointError::BadPort));
    }

    #[test]
    fn connection_runs_setup_to_ready_then_cancels() {
        let ep = Endpoint::host_port("host.test:9").unwrap();
        let mut c = Connection::new(ep, Parameters::tcp());
        assert_eq!(c.state(), &ConnState::Setup);
        assert!(c.handle(ConnEvent::Start));
        assert_eq!(c.state(), &ConnState::Preparing);
        assert!(c.handle(ConnEvent::Established));
        assert_eq!(c.state(), &ConnState::Ready);
        assert!(c.handle(ConnEvent::Cancel));
        assert_eq!(c.state(), &ConnState::Cancelled);
        // Cancelled is terminal: a second start does nothing.
        assert!(!c.handle(ConnEvent::Start));
        assert_eq!(c.state(), &ConnState::Cancelled);
    }

    #[test]
    fn waiting_restarts_and_failed_is_terminal() {
        let ep = Endpoint::Unix("/run/test.sock".to_string());
        let mut c = Connection::new(ep, Parameters::tls());
        c.handle(ConnEvent::Start);
        c.handle(ConnEvent::Established);
        assert!(c.handle(ConnEvent::PathUnavailable("no route".to_string())));
        assert_eq!(c.state(), &ConnState::Waiting("no route".to_string()));
        // A ready connection cannot restart, but a waiting one can.
        assert!(c.handle(ConnEvent::Restart));
        assert_eq!(c.state(), &ConnState::Preparing);
        assert!(c.handle(ConnEvent::Error("reset".to_string())));
        assert_eq!(c.state(), &ConnState::Failed("reset".to_string()));
        assert!(!c.handle(ConnEvent::Cancel));
        assert_eq!(c.state(), &ConnState::Failed("reset".to_string()));
    }

    #[test]
    fn parameters_refuse_tcp_options_on_udp_and_zero_timeouts() {
        let keep = TcpOptions { keepalive_idle_secs: Some(60), ..TcpOptions::default() };
        assert_eq!(Parameters::udp().with_tcp(keep.clone()), Err(ParametersError::TcpOptionsOnUdp));
        let zero = TcpOptions { connection_timeout_secs: Some(0), ..TcpOptions::default() };
        assert_eq!(Parameters::tls().with_tcp(zero), Err(ParametersError::ZeroTimeout));
        let ok = Parameters::tls().with_tcp(keep.clone()).unwrap();
        assert!(ok.tls);
        assert_eq!(ok.tcp, keep);
        assert_eq!(ok.transport, Transport::Tcp);
    }

    #[test]
    fn listener_follows_path_changes_and_honours_its_limit() {
        let mut l = Listener::new(0, Parameters::tcp());
        // A listener that is not ready accepts nothing.
        assert!(!l.accept());
        assert!(l.handle(ListenEvent::Start));
        assert_eq!(l.state(), &ListenerState::Ready);
        l.set_connection_limit(Some(2));
        assert!(l.accept());
        assert!(l.accept());
        assert!(!l.accept());
        // Losing the path stops accepts until it comes back.
        assert!(l.handle(ListenEvent::PathUnavailable("down".to_string())));
        l.set_connection_limit(None);
        assert!(!l.accept());
        assert!(l.handle(ListenEvent::PathRestored));
        assert!(l.accept());
        assert!(l.handle(ListenEvent::Cancel));
        assert_eq!(l.state(), &ListenerState::Cancelled);
        assert!(!l.handle(ListenEvent::PathRestored));
    }
}
