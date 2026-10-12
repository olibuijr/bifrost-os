//! Connection and listener parity: the app-facing names for `bifrost_core::net_connection`.
//!
//! `Endpoint` names the remote side (a host and port, a DNS-SD service instance, or a local socket
//! path) and `Parameters` picks the transport and TCP options. `Connection` and `Listener` track
//! the state an app observes, and `Listener::accept` applies the connection limit. The models open
//! no socket. An app owns its transport and feeds the transport's results in as events.

pub use bifrost_core::net_connection::{
    ConnEvent, ConnState, Connection, Endpoint, EndpointError, ListenEvent, Listener,
    ListenerState, Parameters, ParametersError, TcpOptions, Transport,
};
