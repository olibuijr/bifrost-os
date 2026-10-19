//! The socket side of the client protocol (`bifrost_midi::wire`): accepts app connections on a unix socket, turns the
//! bytes into requests for the `Broker`, and writes back the replies and events it returns. Everything is
//! non-blocking and runs inside the daemon's one poll loop: `fds` adds the listener and every client to the poll set,
//! `service` does the accept, read, handle, pump and write steps once.
//!
//! A client that sends a malformed frame, closes, or stops reading (more than `MAX_OUT` bytes waiting) is dropped,
//! and the broker removes its client, virtual endpoints and queued packets with it.

use bifrost_midi::broker::{Broker, ConnId};
use bifrost_midi::wire::{frame, FrameReader, Message, Request};
use std::io::{self, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::io::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;

/// Bytes one client may have waiting to be written before it is dropped as stuck.
const MAX_OUT: usize = 1 << 20;
/// Connections accepted at once.
const MAX_CLIENTS: usize = 64;
/// Reads one client may get per `service` call, so a flood cannot starve the hardware ports.
const MAX_READS: usize = 16;

struct Link {
    stream: UnixStream,
    reader: FrameReader,
    out: Vec<u8>,
    conn: ConnId,
    dead: bool,
}

pub struct Server {
    listener: UnixListener,
    links: Vec<Link>,
}

impl Server {
    /// Binds the socket (replacing a stale file) and makes it reachable by every local user: any app may be a client.
    pub fn bind(path: &Path) -> io::Result<Server> {
        let _ = std::fs::remove_file(path);
        let listener = UnixListener::bind(path)?;
        listener.set_nonblocking(true)?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o666))?;
        Ok(Server { listener, links: Vec::new() })
    }

    /// The number of connected clients.
    #[allow(dead_code)]
    pub fn clients(&self) -> usize {
        self.links.len()
    }

    /// Adds the listener and every client to a poll set; a client with bytes waiting is polled for writing too.
    pub fn fds(&self, out: &mut Vec<libc::pollfd>) {
        out.push(libc::pollfd { fd: self.listener.as_raw_fd(), events: libc::POLLIN, revents: 0 });
        for l in &self.links {
            let mut events = libc::POLLIN;
            if !l.out.is_empty() {
                events |= libc::POLLOUT;
            }
            out.push(libc::pollfd { fd: l.stream.as_raw_fd(), events, revents: 0 });
        }
    }

    /// True while a client has bytes it has not taken yet (the poll loop wakes soon to retry).
    pub fn has_unsent(&self) -> bool {
        self.links.iter().any(|l| !l.out.is_empty())
    }

    /// One round: accept, read and handle requests, push the broker's due events, write, and drop dead clients.
    pub fn service(&mut self, broker: &mut Broker, now_ns: u64) {
        self.accept(broker);
        for l in self.links.iter_mut() {
            Self::read_requests(l, broker, now_ns);
        }
        for (conn, msg) in broker.pump(now_ns) {
            if let Some(l) = self.links.iter_mut().find(|l| l.conn == conn && !l.dead) {
                Self::queue(l, &msg);
            }
        }
        for l in self.links.iter_mut() {
            Self::flush(l);
        }
        let mut closed = false;
        self.links.retain(|l| {
            if l.dead {
                broker.close(l.conn);
                closed = true;
            }
            !l.dead
        });
        if closed {
            // the closed clients' endpoints are gone: the others learn it on the next pump
            let _ = broker.pump(now_ns);
        }
    }

    fn accept(&mut self, broker: &mut Broker) {
        loop {
            match self.listener.accept() {
                Ok((stream, _)) => {
                    if self.links.len() >= MAX_CLIENTS || stream.set_nonblocking(true).is_err() {
                        continue;
                    }
                    let conn = broker.open();
                    self.links.push(Link { stream, reader: FrameReader::new(), out: Vec::new(), conn, dead: false });
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
    }

    fn queue(l: &mut Link, msg: &Message) {
        l.out.extend_from_slice(&frame(&msg.encode()));
        if l.out.len() > MAX_OUT {
            l.dead = true;
        }
    }

    fn read_requests(l: &mut Link, broker: &mut Broker, now_ns: u64) {
        let mut buf = [0u8; 16 * 1024];
        for _ in 0..MAX_READS {
            if l.dead {
                return;
            }
            match l.stream.read(&mut buf) {
                Ok(0) => {
                    l.dead = true;
                    return;
                }
                Ok(n) => l.reader.push(&buf[..n]),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => {
                    l.dead = true;
                    return;
                }
            }
            loop {
                match l.reader.next_frame() {
                    Ok(Some(payload)) => match Request::decode(&payload) {
                        Ok(req) => {
                            for reply in broker.handle(l.conn, req, now_ns) {
                                Self::queue(l, &reply);
                            }
                        }
                        Err(_) => {
                            l.dead = true;
                            return;
                        }
                    },
                    Ok(None) => break,
                    Err(_) => {
                        l.dead = true;
                        return;
                    }
                }
            }
        }
    }

    fn flush(l: &mut Link) {
        while !l.out.is_empty() && !l.dead {
            match l.stream.write(&l.out) {
                Ok(0) => l.dead = true,
                Ok(n) => {
                    l.out.drain(..n);
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => l.dead = true,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_midi::events::MidiEvent;
    use bifrost_midi::hub::Hub;
    use bifrost_midi::ump::from_midi1;
    use bifrost_midi::wire::{Refusal, VERSION};
    use std::time::{Duration, Instant};

    fn socket_path(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("midid-test-{}-{tag}.sock", std::process::id()))
    }

    /// A non-blocking test client that keeps everything the server sent it.
    struct Client {
        stream: UnixStream,
        reader: FrameReader,
        seen: Vec<Message>,
    }

    impl Client {
        fn connect(path: &Path) -> Client {
            let stream = UnixStream::connect(path).unwrap();
            stream.set_nonblocking(true).unwrap();
            Client { stream, reader: FrameReader::new(), seen: Vec::new() }
        }

        fn send(&mut self, r: Request) {
            self.stream.write_all(&frame(&r.encode())).unwrap();
        }

        /// Runs the server and reads until `done` holds for what was seen, or two seconds pass.
        fn wait(&mut self, server: &mut Server, broker: &mut Broker, done: impl Fn(&[Message]) -> bool) {
            let deadline = Instant::now() + Duration::from_secs(2);
            let mut buf = [0u8; 4096];
            while !done(&self.seen) && Instant::now() < deadline {
                server.service(broker, 1_000);
                match self.stream.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => self.reader.push(&buf[..n]),
                    Err(_) => std::thread::sleep(Duration::from_millis(2)),
                }
                while let Some(p) = self.reader.next_frame().unwrap() {
                    self.seen.push(Message::decode(&p).unwrap());
                }
            }
        }
    }

    fn drain_until_no_clients(server: &mut Server, broker: &mut Broker) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while server.clients() > 0 && Instant::now() < deadline {
            server.service(broker, 1_000);
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn hello_and_ping_over_a_real_socket() {
        let path = socket_path("hello");
        let mut server = Server::bind(&path).unwrap();
        let mut broker = Broker::new(Hub::new());
        let mut c = Client::connect(&path);
        c.send(Request::Hello { name: "t".into(), version: VERSION });
        c.send(Request::Ping);
        c.wait(&mut server, &mut broker, |m| m.len() >= 2);
        assert!(matches!(c.seen.first(), Some(Message::Welcome { .. })), "{:?}", c.seen);
        assert_eq!(c.seen.get(1), Some(&Message::Pong));
        assert_eq!(server.clients(), 1);
        assert_eq!(broker.connections(), 1);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_closed_client_is_removed_from_the_broker() {
        let path = socket_path("close");
        let mut server = Server::bind(&path).unwrap();
        let mut broker = Broker::new(Hub::new());
        {
            let mut c = Client::connect(&path);
            c.send(Request::Hello { name: "t".into(), version: VERSION });
            c.wait(&mut server, &mut broker, |m| !m.is_empty());
            assert_eq!(c.seen.len(), 1);
        }
        drain_until_no_clients(&mut server, &mut broker);
        assert_eq!(server.clients(), 0);
        assert_eq!(broker.connections(), 0);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_malformed_frame_drops_the_client_and_a_request_before_hello_is_refused() {
        let path = socket_path("bad");
        let mut server = Server::bind(&path).unwrap();
        let mut broker = Broker::new(Hub::new());
        let mut c = Client::connect(&path);
        c.send(Request::List);
        c.wait(&mut server, &mut broker, |m| !m.is_empty());
        assert_eq!(c.seen, vec![Message::Failed(Refusal::NoClient)]);
        c.stream.write_all(&frame(&[0xEF, 0xBE])).unwrap();
        drain_until_no_clients(&mut server, &mut broker);
        assert_eq!(server.clients(), 0);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn two_clients_exchange_packets_through_a_virtual_source() {
        let path = socket_path("pair");
        let mut server = Server::bind(&path).unwrap();
        let mut broker = Broker::new(Hub::new());
        let mut a = Client::connect(&path);
        let mut b = Client::connect(&path);
        a.send(Request::Hello { name: "a".into(), version: VERSION });
        b.send(Request::Hello { name: "b".into(), version: VERSION });
        a.send(Request::CreateSource { name: "out".into() });
        a.wait(&mut server, &mut broker, |m| m.iter().any(|x| matches!(x, Message::Created { .. })));
        let src = a
            .seen
            .iter()
            .find_map(|m| match m {
                Message::Created { id } => Some(*id),
                _ => None,
            })
            .expect("the source was created");
        b.wait(&mut server, &mut broker, |m| m.iter().any(|x| matches!(x, Message::Welcome { .. })));
        b.send(Request::Connect { source: src });
        b.wait(&mut server, &mut broker, |m| m.contains(&Message::Done));
        let packet = from_midi1(0, MidiEvent::NoteOn { channel: 0, key: 60, velocity: 100 });
        a.send(Request::Inject { source: src, host_ns: 0, packets: vec![packet] });
        b.wait(&mut server, &mut broker, |m| m.iter().any(|x| matches!(x, Message::Received { .. })));
        assert!(
            b.seen.iter().any(|m| matches!(m, Message::Received { source, packet: p, .. } if *source == src && *p == packet)),
            "{:?}",
            b.seen
        );
        let _ = std::fs::remove_file(&path);
    }
}
