//! A real connection for apps: `NWConnection` with a socket behind it.
//!
//! `network_connection` holds the endpoint, the parameters and the state machine and opens nothing. `NetworkStream`
//! is the owner of the transport it describes: it opens the socket the parameters ask for (TCP, TCP under TLS, UDP, or a
//! local socket path), applies the TCP options (no delay, keepalive idle time, connection timeout), moves the state from
//! setup through preparing to ready, and sends and receives. A failure moves the state to failed, a cancel to cancelled,
//! and both close the socket. TLS uses the options model of `tls_client`; the handshake runs on the first send or
//! receive. A service endpoint has to be resolved to a host and port first (`network_browser`).

use crate::tls_client;
use bifrost_core::net_connection::{ConnEvent, ConnState, Connection, Endpoint, Parameters, TcpOptions, Transport};
use bifrost_ui::tls_options::TlsOptions;
use rustls::{ClientConnection, StreamOwned};
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs, UdpSocket};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::time::Duration;

extern "C" {
    fn setsockopt(fd: i32, level: i32, name: i32, value: *const i32, len: u32) -> i32;
}

const SOL_SOCKET: i32 = 1;
const SO_KEEPALIVE: i32 = 9;
const IPPROTO_TCP: i32 = 6;
const TCP_KEEPIDLE: i32 = 4;

fn set_int_option(fd: i32, level: i32, name: i32, value: i32) -> io::Result<()> {
    // SAFETY: `fd` is a socket the caller owns for the duration of the call, and `value` is a live 4-byte integer,
    // which is what every option set here takes.
    let rc = unsafe { setsockopt(fd, level, name, &value as *const i32, 4) };
    if rc < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Applies the TCP options to a connected socket.
fn apply_tcp_options(tcp: &TcpStream, options: &TcpOptions) -> io::Result<()> {
    tcp.set_nodelay(options.no_delay)?;
    if let Some(idle) = options.keepalive_idle_secs {
        let fd = tcp.as_raw_fd();
        set_int_option(fd, SOL_SOCKET, SO_KEEPALIVE, 1)?;
        set_int_option(fd, IPPROTO_TCP, TCP_KEEPIDLE, idle.min(i32::MAX as u32) as i32)?;
    }
    Ok(())
}

fn connect_any(addrs: &[SocketAddr], timeout: Option<Duration>) -> io::Result<TcpStream> {
    let mut last = io::Error::new(io::ErrorKind::NotFound, "the host name has no address");
    for addr in addrs {
        let attempt = match timeout {
            Some(t) => TcpStream::connect_timeout(addr, t),
            None => TcpStream::connect(addr),
        };
        match attempt {
            Ok(stream) => return Ok(stream),
            Err(e) => last = e,
        }
    }
    Err(last)
}

enum Link {
    Tcp(TcpStream),
    Tls(Box<StreamOwned<ClientConnection, TcpStream>>),
    Unix(UnixStream),
    Udp(UdpSocket),
}

impl Link {
    fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
        match self {
            Link::Tcp(s) => s.write_all(bytes),
            Link::Tls(s) => {
                s.write_all(bytes)?;
                s.flush()
            }
            Link::Unix(s) => s.write_all(bytes),
            Link::Udp(s) => s.send(bytes).map(|_| ()),
        }
    }

    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Link::Tcp(s) => s.read(buf),
            Link::Tls(s) => s.read(buf),
            Link::Unix(s) => s.read(buf),
            Link::Udp(s) => s.recv(buf),
        }
    }

    fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        match self {
            Link::Tcp(s) => s.set_read_timeout(timeout),
            Link::Tls(s) => s.sock.set_read_timeout(timeout),
            Link::Unix(s) => s.set_read_timeout(timeout),
            Link::Udp(s) => s.set_read_timeout(timeout),
        }
    }
}

fn open(endpoint: &Endpoint, parameters: &Parameters, tls: &TlsOptions) -> io::Result<Link> {
    match (endpoint, parameters.transport) {
        (Endpoint::HostPort { host, port }, Transport::Tcp) => {
            let addrs: Vec<SocketAddr> = (host.as_str(), *port).to_socket_addrs()?.collect();
            let timeout = parameters.tcp.connection_timeout_secs.map(|s| Duration::from_secs(u64::from(s)));
            let tcp = connect_any(&addrs, timeout)?;
            apply_tcp_options(&tcp, &parameters.tcp)?;
            if parameters.tls {
                Ok(Link::Tls(Box::new(tls_client::connect(tls, host, tcp)?)))
            } else {
                Ok(Link::Tcp(tcp))
            }
        }
        (Endpoint::HostPort { host, port }, Transport::Udp) => {
            let addr = (host.as_str(), *port)
                .to_socket_addrs()?
                .next()
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "the host name has no address"))?;
            let socket = UdpSocket::bind(if addr.is_ipv4() { "0.0.0.0:0" } else { "[::]:0" })?;
            socket.connect(addr)?;
            Ok(Link::Udp(socket))
        }
        (Endpoint::Unix(path), Transport::Tcp) => {
            if parameters.tls {
                return Err(io::Error::new(io::ErrorKind::InvalidInput, "TLS needs a host name, not a socket path"));
            }
            Ok(Link::Unix(UnixStream::connect(path)?))
        }
        (Endpoint::Unix(_), Transport::Udp) => Err(io::Error::new(io::ErrorKind::Unsupported, "UDP has no socket path form")),
        (Endpoint::Service { .. }, _) => {
            Err(io::Error::new(io::ErrorKind::Unsupported, "a service endpoint has to be resolved to a host and port first"))
        }
    }
}

/// A connection to an endpoint, with its state and its socket.
pub struct NetworkStream {
    connection: Connection,
    link: Option<Link>,
}

impl NetworkStream {
    /// Connects with the default TLS options (system roots, TLS 1.2 to 1.3). The result is in the ready state.
    pub fn connect(endpoint: Endpoint, parameters: Parameters) -> io::Result<NetworkStream> {
        NetworkStream::connect_with(endpoint, parameters, &TlsOptions::default())
    }

    /// Connects, using `tls` when the parameters ask for TLS.
    pub fn connect_with(endpoint: Endpoint, parameters: Parameters, tls: &TlsOptions) -> io::Result<NetworkStream> {
        let mut connection = Connection::new(endpoint, parameters);
        connection.handle(ConnEvent::Start);
        match open(connection.endpoint(), connection.parameters(), tls) {
            Ok(link) => {
                connection.handle(ConnEvent::Established);
                Ok(NetworkStream { connection, link: Some(link) })
            }
            Err(e) => {
                connection.handle(ConnEvent::Error(e.to_string()));
                Err(e)
            }
        }
    }

    /// The state machine: `Ready` while the socket is usable, `Failed` after an I/O error, `Cancelled` after `cancel`.
    pub fn state(&self) -> &ConnState {
        self.connection.state()
    }

    pub fn connection(&self) -> &Connection {
        &self.connection
    }

    fn link(&mut self) -> io::Result<&mut Link> {
        match self.link.as_mut() {
            Some(link) if *self.connection.state() == ConnState::Ready => Ok(link),
            _ => Err(io::Error::new(io::ErrorKind::NotConnected, "the connection is not ready")),
        }
    }

    fn fail(&mut self, e: io::Error) -> io::Error {
        // a read that times out leaves the connection usable; anything else ends it
        if !matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) {
            self.connection.handle(ConnEvent::Error(e.to_string()));
            self.link = None;
        }
        e
    }

    /// Sends bytes (a stream: all of them; UDP: one datagram).
    pub fn send(&mut self, bytes: &[u8]) -> io::Result<()> {
        let result = self.link()?.write_all(bytes);
        result.map_err(|e| self.fail(e))
    }

    /// Reads up to `max` bytes. For a stream an empty answer means the peer closed; for UDP it is one datagram.
    pub fn receive(&mut self, max: usize) -> io::Result<Vec<u8>> {
        let mut buf = vec![0u8; max.max(1)];
        let result = self.link()?.read(&mut buf);
        match result {
            Ok(n) => {
                buf.truncate(n);
                Ok(buf)
            }
            Err(e) => Err(self.fail(e)),
        }
    }

    /// Reads exactly `n` bytes of a stream, or fails.
    pub fn receive_exact(&mut self, n: usize) -> io::Result<Vec<u8>> {
        let mut out = Vec::with_capacity(n);
        while out.len() < n {
            let chunk = self.receive(n - out.len())?;
            if chunk.is_empty() {
                return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "the peer closed the connection"));
            }
            out.extend_from_slice(&chunk);
        }
        Ok(out)
    }

    /// How long `receive` waits for data. `None` waits for ever.
    pub fn set_receive_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        self.link()?.set_read_timeout(timeout)
    }

    /// The address of the peer, for a socket that has one.
    pub fn peer_addr(&self) -> Option<SocketAddr> {
        match self.link.as_ref()? {
            Link::Tcp(s) => s.peer_addr().ok(),
            Link::Tls(s) => s.sock.peer_addr().ok(),
            Link::Udp(s) => s.peer_addr().ok(),
            Link::Unix(_) => None,
        }
    }

    /// Whether Nagle's algorithm is off on a TCP socket.
    pub fn no_delay(&self) -> Option<bool> {
        match self.link.as_ref()? {
            Link::Tcp(s) => s.nodelay().ok(),
            Link::Tls(s) => s.sock.nodelay().ok(),
            _ => None,
        }
    }

    /// Ends the connection and closes the socket.
    pub fn cancel(&mut self) {
        self.connection.handle(ConnEvent::Cancel);
        self.link = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_core::net_connection::TcpOptions;
    use std::net::TcpListener;
    use std::os::unix::net::UnixListener;
    use std::thread;

    fn loopback(port: u16) -> Endpoint {
        Endpoint::HostPort { host: "127.0.0.1".into(), port }
    }

    /// A TCP server that echoes what it reads until the client closes.
    fn echo_server() -> (u16, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            let mut buf = [0u8; 256];
            loop {
                match sock.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if sock.write_all(&buf[..n]).is_err() {
                            break;
                        }
                    }
                }
            }
        });
        (port, handle)
    }

    #[test]
    fn a_tcp_connection_becomes_ready_and_echoes() {
        let (port, server) = echo_server();
        let mut s = NetworkStream::connect(loopback(port), Parameters::tcp()).unwrap();
        assert_eq!(*s.state(), ConnState::Ready);
        s.send(b"hello world").unwrap();
        assert_eq!(s.receive_exact(11).unwrap(), b"hello world");
        assert_eq!(s.peer_addr().unwrap().port(), port);
        s.cancel();
        server.join().unwrap();
    }

    #[test]
    fn cancel_ends_the_connection_and_later_sends_fail() {
        let (port, server) = echo_server();
        let mut s = NetworkStream::connect(loopback(port), Parameters::tcp()).unwrap();
        s.cancel();
        assert_eq!(*s.state(), ConnState::Cancelled);
        let err = s.send(b"x").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotConnected);
        server.join().unwrap();
    }

    #[test]
    fn a_refused_connection_is_an_error() {
        let port = {
            let l = TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let err = NetworkStream::connect(loopback(port), Parameters::tcp()).err().unwrap();
        assert_eq!(err.kind(), io::ErrorKind::ConnectionRefused);
    }

    #[test]
    fn tcp_options_are_applied() {
        let (port, server) = echo_server();
        let options = TcpOptions { no_delay: true, keepalive_idle_secs: Some(30), connection_timeout_secs: Some(2) };
        let parameters = Parameters::tcp().with_tcp(options).unwrap();
        let mut s = NetworkStream::connect(loopback(port), parameters).unwrap();
        assert_eq!(s.no_delay(), Some(true));
        s.cancel();
        server.join().unwrap();
    }

    #[test]
    fn a_closed_peer_reads_as_empty_and_receive_exact_reports_it() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            sock.write_all(b"ab").unwrap();
        });
        let mut s = NetworkStream::connect(loopback(port), Parameters::tcp()).unwrap();
        server.join().unwrap();
        let err = s.receive_exact(5).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn a_receive_timeout_leaves_the_connection_ready() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let mut s = NetworkStream::connect(loopback(port), Parameters::tcp()).unwrap();
        let (_peer, _) = listener.accept().unwrap();
        s.set_receive_timeout(Some(Duration::from_millis(30))).unwrap();
        let err = s.receive(8).unwrap_err();
        assert!(matches!(err.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut));
        assert_eq!(*s.state(), ConnState::Ready);
    }

    #[test]
    fn udp_sends_and_receives_datagrams() {
        let server = UdpSocket::bind("127.0.0.1:0").unwrap();
        let port = server.local_addr().unwrap().port();
        let mut s = NetworkStream::connect(loopback(port), Parameters::udp()).unwrap();
        s.send(b"ping").unwrap();
        let mut buf = [0u8; 16];
        let (n, from) = server.recv_from(&mut buf).unwrap();
        assert_eq!(&buf[..n], b"ping");
        server.send_to(b"pong", from).unwrap();
        assert_eq!(s.receive(16).unwrap(), b"pong");
    }

    #[test]
    fn a_local_socket_path_connects() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("echo.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let server = thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            let mut buf = [0u8; 16];
            let n = sock.read(&mut buf).unwrap();
            sock.write_all(&buf[..n]).unwrap();
        });
        let mut s = NetworkStream::connect(Endpoint::Unix(path.to_string_lossy().into_owned()), Parameters::tcp()).unwrap();
        s.send(b"local").unwrap();
        assert_eq!(s.receive_exact(5).unwrap(), b"local");
        server.join().unwrap();
    }

    #[test]
    fn impossible_combinations_are_refused_with_a_failed_state() {
        let service = Endpoint::Service { name: "x".into(), kind: "_http._tcp".into(), domain: "local".into() };
        let err = NetworkStream::connect(service, Parameters::tcp()).err().unwrap();
        assert_eq!(err.kind(), io::ErrorKind::Unsupported);
        let err = NetworkStream::connect(Endpoint::Unix("/nonexistent/s".into()), Parameters::tls()).err().unwrap();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        let err = NetworkStream::connect(Endpoint::Unix("/x".into()), Parameters::udp()).err().unwrap();
        assert_eq!(err.kind(), io::ErrorKind::Unsupported);
    }

    #[test]
    fn tls_over_tcp_opens_the_socket_and_defers_the_handshake() {
        // the handshake runs on the first send, so a plain listener that never speaks TLS still accepts the connect
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let s = NetworkStream::connect(Endpoint::HostPort { host: "localhost".into(), port }, Parameters::tls()).unwrap();
        assert_eq!(*s.state(), ConnState::Ready);
        assert!(s.connection().parameters().tls);
    }
}
