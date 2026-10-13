//! `NWListener` parity: an app-facing TCP or UDP listener over `std::net`.
//!
//! `Listener::new(parameters, port)` mirrors `NWListener(using:on:)`. `start` binds the socket
//! (port 0 picks a free port, read back with `port`), the state moves through `.setup`, `.ready`,
//! `.failed` and `.cancelled`, and the handlers fire from `poll`, which the owner calls from its
//! run loop. `poll` never blocks. `set_new_connection_limit` caps accepted connections; extra
//! peers are closed at once. A `Service` names the Bonjour instance to advertise, and
//! `advertiser` turns it into the `service_discovery` publication for the bound port.

use crate::network_connection::{ListenEvent, Listener as Machine, ListenerState, Parameters, Transport};
use crate::service_discovery::ServiceAdvertiser;
use bifrost_core::txt_record::TxtRecord;
use std::io::ErrorKind;
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream, UdpSocket};

/// `NWListener.State`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    Setup,
    Waiting(String),
    Ready,
    Failed(String),
    Cancelled,
}

impl From<&ListenerState> for State {
    fn from(s: &ListenerState) -> Self {
        match s {
            ListenerState::Setup => State::Setup,
            ListenerState::Ready => State::Ready,
            ListenerState::Waiting(r) => State::Waiting(r.clone()),
            ListenerState::Failed(r) => State::Failed(r.clone()),
            ListenerState::Cancelled => State::Cancelled,
        }
    }
}

/// `NWListener.Service`: the Bonjour instance a listener advertises.
#[derive(Clone, Debug)]
pub struct Service {
    pub name: String,
    pub kind: String,
    pub domain: String,
    pub txt: TxtRecord,
}

impl Service {
    /// `Service(name:type:domain:txtRecord:)`. An empty domain means `local`.
    pub fn new(name: &str, kind: &str, domain: &str, txt: TxtRecord) -> Self {
        let domain = if domain.is_empty() { "local" } else { domain };
        Service { name: name.into(), kind: kind.into(), domain: domain.into(), txt }
    }
}

/// Why a listener could not be created or started.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ListenerError {
    /// Listening with TLS needs an identity this listener does not hold.
    TlsUnsupported,
    /// The socket could not be bound or configured. Carries the system message.
    Bind(String),
}

/// One accepted inbound TCP connection.
#[derive(Debug)]
pub struct Accepted {
    pub stream: TcpStream,
    pub remote: SocketAddr,
}

/// One received UDP datagram.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Datagram {
    pub from: SocketAddr,
    pub data: Vec<u8>,
}

enum Socket {
    None,
    Tcp(TcpListener),
    Udp(UdpSocket),
}

type StateHandler = Box<dyn FnMut(&State)>;
type ConnectionHandler = Box<dyn FnMut(Accepted)>;
type DatagramHandler = Box<dyn FnMut(Datagram)>;

/// `NWListener`.
pub struct Listener {
    machine: Machine,
    socket: Socket,
    bound: Option<u16>,
    service: Option<Service>,
    state_update_handler: Option<StateHandler>,
    new_connection_handler: Option<ConnectionHandler>,
    new_datagram_handler: Option<DatagramHandler>,
}

impl Listener {
    /// `NWListener(using:on:)`. Port 0 lets the system choose.
    pub fn new(parameters: Parameters, port: u16) -> Result<Self, ListenerError> {
        if parameters.tls {
            return Err(ListenerError::TlsUnsupported);
        }
        Ok(Listener {
            machine: Machine::new(port, parameters),
            socket: Socket::None,
            bound: None,
            service: None,
            state_update_handler: None,
            new_connection_handler: None,
            new_datagram_handler: None,
        })
    }

    pub fn state(&self) -> State {
        State::from(self.machine.state())
    }

    /// The bound port once ready, otherwise the requested port (`NWListener.port`).
    pub fn port(&self) -> u16 {
        self.bound.unwrap_or_else(|| self.machine.port())
    }

    pub fn service(&self) -> Option<&Service> {
        self.service.as_ref()
    }

    pub fn set_service(&mut self, service: Option<Service>) {
        self.service = service;
    }

    /// `newConnectionLimit`. `None` removes the limit.
    pub fn set_new_connection_limit(&mut self, limit: Option<u32>) {
        self.machine.set_connection_limit(limit);
    }

    pub fn set_state_update_handler(&mut self, handler: impl FnMut(&State) + 'static) {
        self.state_update_handler = Some(Box::new(handler));
    }

    pub fn set_new_connection_handler(&mut self, handler: impl FnMut(Accepted) + 'static) {
        self.new_connection_handler = Some(Box::new(handler));
    }

    /// Receives UDP datagrams, the datagram counterpart of `newConnectionHandler`.
    pub fn set_new_datagram_handler(&mut self, handler: impl FnMut(Datagram) + 'static) {
        self.new_datagram_handler = Some(Box::new(handler));
    }

    /// `start(queue:)`: binds and moves to ready, or to failed when binding fails.
    pub fn start(&mut self) -> Result<(), ListenerError> {
        if self.machine.state() != &ListenerState::Setup {
            return Ok(());
        }
        let addr = SocketAddr::from(([0, 0, 0, 0], self.machine.port()));
        let bound = match self.machine.parameters().transport {
            Transport::Tcp => TcpListener::bind(addr).and_then(|l| {
                l.set_nonblocking(true)?;
                let port = l.local_addr()?.port();
                Ok((Socket::Tcp(l), port))
            }),
            Transport::Udp => UdpSocket::bind(addr).and_then(|s| {
                s.set_nonblocking(true)?;
                let port = s.local_addr()?.port();
                Ok((Socket::Udp(s), port))
            }),
        };
        match bound {
            Ok((socket, port)) => {
                self.socket = socket;
                self.bound = Some(port);
                self.machine.handle(ListenEvent::Start);
                self.notify();
                Ok(())
            }
            Err(e) => {
                let reason = e.to_string();
                self.machine.handle(ListenEvent::Start);
                self.machine.handle(ListenEvent::Error(reason.clone()));
                self.notify();
                Err(ListenerError::Bind(reason))
            }
        }
    }

    /// `cancel()`: closes the socket and moves to cancelled.
    pub fn cancel(&mut self) {
        if self.machine.handle(ListenEvent::Cancel) {
            self.socket = Socket::None;
            self.notify();
        }
    }

    /// Accepts what is pending without blocking and fires the handlers. Returns how many
    /// connections were handed over or datagrams delivered.
    pub fn poll(&mut self) -> usize {
        let mut delivered = 0;
        let mut pending = Vec::new();
        let mut datagrams = Vec::new();
        match &self.socket {
            Socket::Tcp(l) => loop {
                match l.accept() {
                    Ok((stream, remote)) => pending.push(Accepted { stream, remote }),
                    Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            },
            Socket::Udp(s) => {
                let mut buf = vec![0u8; 65535];
                while let Ok((n, from)) = s.recv_from(&mut buf) {
                    datagrams.push(Datagram { from, data: buf[..n].to_vec() });
                }
            }
            Socket::None => {}
        }
        for conn in pending {
            if !self.machine.accept() {
                continue; // over the limit: dropping the stream closes it
            }
            delivered += 1;
            if let Some(h) = self.new_connection_handler.as_mut() {
                h(conn);
            }
        }
        for d in datagrams {
            delivered += 1;
            if let Some(h) = self.new_datagram_handler.as_mut() {
                h(d);
            }
        }
        delivered
    }

    /// The Bonjour publication for the service on the bound port, ready to answer queries.
    /// `None` without a service, before the listener is ready, or for an invalid name.
    pub fn advertiser(&self, host: &str, addresses: Vec<IpAddr>) -> Option<ServiceAdvertiser> {
        let svc = self.service.as_ref()?;
        let port = self.bound?;
        ServiceAdvertiser::new(&svc.name, &svc.kind, &svc.domain, host, port, svc.txt.clone(), addresses)
    }

    fn notify(&mut self) {
        let state = self.state();
        if let Some(h) = self.state_update_handler.as_mut() {
            h(&state);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::io::{Read, Write};
    use std::rc::Rc;
    use std::time::{Duration, Instant};

    fn poll_until(l: &mut Listener, want: usize) -> usize {
        let end = Instant::now() + Duration::from_secs(2);
        let mut got = 0;
        while got < want && Instant::now() < end {
            got += l.poll();
            std::thread::sleep(Duration::from_millis(5));
        }
        got
    }

    #[test]
    fn start_on_port_zero_is_ready_with_a_bound_port() {
        let mut l = Listener::new(Parameters::tcp(), 0).unwrap();
        assert_eq!(l.state(), State::Setup);
        l.start().unwrap();
        assert_eq!(l.state(), State::Ready);
        assert_ne!(l.port(), 0);
    }

    #[test]
    fn state_handler_sees_ready_then_cancelled() {
        let seen = Rc::new(RefCell::new(Vec::new()));
        let s = seen.clone();
        let mut l = Listener::new(Parameters::tcp(), 0).unwrap();
        l.set_state_update_handler(move |st| s.borrow_mut().push(st.clone()));
        l.start().unwrap();
        l.cancel();
        assert_eq!(*seen.borrow(), vec![State::Ready, State::Cancelled]);
    }

    #[test]
    fn accepts_a_tcp_stream_and_hands_it_to_the_handler() {
        let got = Rc::new(RefCell::new(Vec::new()));
        let g = got.clone();
        let mut l = Listener::new(Parameters::tcp(), 0).unwrap();
        l.set_new_connection_handler(move |c| g.borrow_mut().push(c));
        l.start().unwrap();
        let mut client = TcpStream::connect(("127.0.0.1", l.port())).unwrap();
        client.write_all(b"hi").unwrap();
        assert_eq!(poll_until(&mut l, 1), 1);
        let mut conn = got.borrow_mut().remove(0);
        let mut buf = [0u8; 2];
        conn.stream.set_nonblocking(false).unwrap();
        conn.stream.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"hi");
    }

    #[test]
    fn new_connection_limit_rejects_extra_peers() {
        let count = Rc::new(RefCell::new(0));
        let c = count.clone();
        let mut l = Listener::new(Parameters::tcp(), 0).unwrap();
        l.set_new_connection_limit(Some(1));
        l.set_new_connection_handler(move |_| *c.borrow_mut() += 1);
        l.start().unwrap();
        let _a = TcpStream::connect(("127.0.0.1", l.port())).unwrap();
        let _b = TcpStream::connect(("127.0.0.1", l.port())).unwrap();
        std::thread::sleep(Duration::from_millis(50));
        l.poll();
        assert_eq!(*count.borrow(), 1);
    }

    #[test]
    fn cancel_closes_the_socket() {
        let mut l = Listener::new(Parameters::tcp(), 0).unwrap();
        l.start().unwrap();
        let port = l.port();
        l.cancel();
        assert_eq!(l.state(), State::Cancelled);
        assert!(TcpStream::connect(("127.0.0.1", port)).is_err());
    }

    #[test]
    fn udp_listener_receives_a_datagram() {
        let got = Rc::new(RefCell::new(Vec::new()));
        let g = got.clone();
        let mut l = Listener::new(Parameters::udp(), 0).unwrap();
        l.set_new_datagram_handler(move |d| g.borrow_mut().push(d));
        l.start().unwrap();
        let client = UdpSocket::bind("127.0.0.1:0").unwrap();
        client.send_to(b"ping", ("127.0.0.1", l.port())).unwrap();
        assert_eq!(poll_until(&mut l, 1), 1);
        assert_eq!(got.borrow()[0].data, b"ping");
    }

    #[test]
    fn tls_listener_is_refused() {
        assert_eq!(Listener::new(Parameters::tls(), 0).err(), Some(ListenerError::TlsUnsupported));
    }

    #[test]
    fn busy_port_fails_the_listener() {
        let mut a = Listener::new(Parameters::tcp(), 0).unwrap();
        a.start().unwrap();
        let mut b = Listener::new(Parameters::tcp(), a.port()).unwrap();
        assert!(matches!(b.start(), Err(ListenerError::Bind(_))));
        assert!(matches!(b.state(), State::Failed(_)));
    }

    #[test]
    fn service_advertises_on_the_bound_port() {
        let mut l = Listener::new(Parameters::tcp(), 0).unwrap();
        l.set_service(Some(Service::new("Den", "_http._tcp", "", TxtRecord::default())));
        assert!(l.advertiser("den.local", vec![]).is_none());
        l.start().unwrap();
        let adv = l.advertiser("den.local", vec![]).unwrap();
        assert_eq!(adv.instance_name(), "Den._http._tcp.local");
    }
}
