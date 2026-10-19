//! The connection to one RFB server. A thread connects, runs the handshake and reads updates into the shared
//! framebuffer; a second thread writes the viewer's messages in order. The window never blocks on the socket: it
//! polls `Shared` on its own clock and sends through a channel.

use bifrost_rfb::{handshake, message, Error, Framebuffer, PixelFormat};
use bifrost_app::dispatch::{DispatchQueue, Qos};
use std::io::Write;
use std::net::{Shutdown, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex, MutexGuard};
use std::time::Duration;

/// How long a connect attempt may take before it is given up.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(8);

/// Where the connection is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    Connecting,
    /// The handshake succeeded; `name` is the desktop name the server gave.
    Connected { name: String },
    /// The connection ended. The text is the reason (`Disconnected` after the viewer hung up).
    Closed(String),
}

/// State the reader thread writes and the window reads.
pub struct Shared {
    pub frame: Mutex<Framebuffer>,
    /// Counts applied updates, so the window knows when to repaint.
    pub generation: AtomicU64,
    pub status: Mutex<Status>,
}

/// The window's handle on a connection. Dropping it closes the socket.
pub struct Connection {
    pub shared: Arc<Shared>,
    /// Text the server put on its clipboard, for the window to copy locally.
    pub cut_text: mpsc::Receiver<String>,
    outbox: mpsc::Sender<Vec<u8>>,
    socket: Arc<Mutex<Option<TcpStream>>>,
}

/// Locks a mutex, reading through a poisoned lock (the data is still valid after a panic in another thread).
pub fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Connection {
    /// Starts connecting to `addr` (`host`, `host:port`, or an IPv6 literal) and returns at once.
    #[cfg(test)]
    pub fn open(addr: &str) -> Connection {
        Connection::open_with(addr, None)
    }

    /// Like `open`, with the password to answer VNC authentication with when the server asks for one (a server that
    /// offers no authentication never sees it).
    pub fn open_with(addr: &str, password: Option<String>) -> Connection {
        let shared = Arc::new(Shared {
            frame: Mutex::new(Framebuffer::new(0, 0)),
            generation: AtomicU64::new(0),
            status: Mutex::new(Status::Connecting),
        });
        let (outbox, outbox_rx) = mpsc::channel::<Vec<u8>>();
        let (cut_tx, cut_text) = mpsc::channel::<String>();
        let socket = Arc::new(Mutex::new(None));
        let target = bifrost_rfb::with_default_port(addr);
        let (s, sock, out) = (shared.clone(), socket.clone(), outbox.clone());
        DispatchQueue::global(Qos::Utility).async_(move || {
            let result = run(&target, password.as_deref(), &s, &sock, out, outbox_rx, &cut_tx);
            let mut st = lock(&s.status);
            if !matches!(*st, Status::Closed(_)) {
                *st = Status::Closed(match result {
                    Ok(()) => "The server closed the connection.".to_string(),
                    Err(e) if handshake::needs_password(&e) => "This computer needs a password. Type it in the Password box, then Connect.".to_string(),
                    Err(e) => e.to_string(),
                });
            }
            drop(st);
            DispatchQueue::main().async_(|| {});
        });
        Connection { shared, cut_text, outbox, socket }
    }

    /// Queues one client message. A message after the connection ended is dropped.
    pub fn send(&self, msg: Vec<u8>) {
        let _ = self.outbox.send(msg);
    }

    pub fn pointer(&self, buttons: u8, x: u16, y: u16) {
        self.send(message::pointer_event(buttons, x, y));
    }

    pub fn key(&self, down: bool, keysym: u32) {
        self.send(message::key_event(down, keysym));
    }

    /// Sends the viewer's clipboard text to the server.
    pub fn paste(&self, text: &str) {
        self.send(message::client_cut_text(text));
    }

    /// Ends the connection. The reader thread sees the socket close and records the reason.
    pub fn close(&self) {
        if let Some(s) = lock(&self.socket).as_ref() {
            let _ = s.shutdown(Shutdown::Both);
        }
        *lock(&self.shared.status) = Status::Closed("Disconnected.".to_string());
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.close();
    }
}

fn run(
    target: &str,
    password: Option<&str>,
    shared: &Arc<Shared>,
    socket: &Arc<Mutex<Option<TcpStream>>>,
    outbox: mpsc::Sender<Vec<u8>>,
    outbox_rx: mpsc::Receiver<Vec<u8>>,
    cut_tx: &mpsc::Sender<String>,
) -> Result<(), Error> {
    let addr = target.to_socket_addrs()?.next().ok_or_else(|| Error::Io(format!("no address for {target}")))?;
    let mut stream = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT)?;
    stream.set_nodelay(true)?;
    let init = handshake::connect_with(&mut stream, password)?;
    *lock(socket) = Some(stream.try_clone()?);
    {
        // The window may have hung up while the handshake ran; a close already recorded is kept.
        let mut st = lock(&shared.status);
        if matches!(*st, Status::Closed(_)) {
            return Ok(());
        }
        *lock(&shared.frame) = Framebuffer::new(init.width as usize, init.height as usize);
        *st = Status::Connected { name: init.name.clone() };
    }
    DispatchQueue::main().async_(|| {});

    let fmt = PixelFormat::VIEWER;
    let (w, h) = (init.width, init.height);
    stream.write_all(&message::set_pixel_format(&fmt))?;
    stream.write_all(&message::set_encodings(&message::ENCODINGS))?;
    stream.write_all(&message::update_request(false, 0, 0, w, h))?;

    // The writer owns a second handle, so the reader can keep reading while the window sends input.
    let mut writer = stream.try_clone()?;
    DispatchQueue::global(Qos::Utility).async_(move || {
        for msg in outbox_rx {
            if writer.write_all(&msg).is_err() {
                break;
            }
        }
    });

    loop {
        match message::read_server_message(&mut stream, &fmt)? {
            message::ServerMessage::Update(rects) => {
                let (fw, fh) = {
                    let mut fb = lock(&shared.frame);
                    for r in &rects {
                        fb.apply(r, &fmt)?;
                    }
                    (fb.w, fb.h)
                };
                shared.generation.fetch_add(1, Ordering::SeqCst);
                DispatchQueue::main().async_(|| {});
                // Ask for what changes next: an incremental update of the whole desktop.
                let _ = outbox.send(message::update_request(true, 0, 0, fw as u16, fh as u16));
            }
            message::ServerMessage::CutText(text) => {
                let _ = cut_tx.send(text);
                DispatchQueue::main().async_(|| {});
            }
            message::ServerMessage::ColourMapIgnored | message::ServerMessage::Bell => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::net::TcpListener;
    use std::time::Instant;

    /// A minimal RFB server on loopback: security None, one 2x1 raw update, then one incremental request from the
    /// viewer. The viewer's handshake and request bytes are checked as they arrive.
    #[test]
    fn connects_and_applies_a_raw_update() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            s.write_all(b"RFB 003.008\n").unwrap();
            let mut version = [0u8; 12];
            s.read_exact(&mut version).unwrap();
            assert_eq!(&version, b"RFB 003.008\n");
            s.write_all(&[1, 1]).unwrap();
            let mut choice = [0u8; 1];
            s.read_exact(&mut choice).unwrap();
            assert_eq!(choice[0], 1);
            s.write_all(&0u32.to_be_bytes()).unwrap();
            let mut client_init = [0u8; 1];
            s.read_exact(&mut client_init).unwrap();
            let mut init = Vec::new();
            init.extend_from_slice(&2u16.to_be_bytes());
            init.extend_from_slice(&1u16.to_be_bytes());
            init.extend_from_slice(&PixelFormat::VIEWER.encode());
            init.extend_from_slice(&1u32.to_be_bytes());
            init.push(b'x');
            s.write_all(&init).unwrap();
            let mut format = [0u8; 20];
            s.read_exact(&mut format).unwrap();
            assert_eq!(format[0], 0, "SetPixelFormat");
            let mut head = [0u8; 4];
            s.read_exact(&mut head).unwrap();
            assert_eq!(head[0], 2, "SetEncodings");
            let mut encodings = vec![0u8; 4 * u16::from_be_bytes([head[2], head[3]]) as usize];
            s.read_exact(&mut encodings).unwrap();
            let mut request = [0u8; 10];
            s.read_exact(&mut request).unwrap();
            assert_eq!(request[1], 0, "the first request is a full update");
            // One FramebufferUpdate with a 2x1 raw rectangle: red, then green (B G R pad in memory).
            let mut update = vec![0u8, 0, 0, 1, 0, 0, 0, 0, 0, 2, 0, 1, 0, 0, 0, 0];
            update.extend_from_slice(&[0, 0, 0xff, 0, 0, 0xff, 0, 0]);
            s.write_all(&update).unwrap();
            let mut next = [0u8; 10];
            s.read_exact(&mut next).unwrap();
            assert_eq!((next[0], next[1]), (3, 1), "an incremental request follows each update");
        });
        let conn = Connection::open(&format!("127.0.0.1:{port}"));
        let deadline = Instant::now() + Duration::from_secs(5);
        while conn.shared.generation.load(Ordering::SeqCst) == 0 {
            assert!(Instant::now() < deadline, "no update arrived");
            std::thread::sleep(Duration::from_millis(10));
        }
        {
            let fb = lock(&conn.shared.frame);
            assert_eq!((fb.w, fb.h), (2, 1));
            assert_eq!(fb.px, vec![0x00ff_0000, 0x0000_ff00]);
        }
        server.join().unwrap();
    }

    /// A password server on loopback: the viewer answers the challenge with the DES response for its password.
    #[test]
    fn connects_to_a_password_server_with_the_right_password() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let challenge: [u8; 16] = *b"abcdefghijklmnop";
        let server = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            s.write_all(b"RFB 003.008\n").unwrap();
            let mut version = [0u8; 12];
            s.read_exact(&mut version).unwrap();
            s.write_all(&[1, 2]).unwrap();
            let mut choice = [0u8; 1];
            s.read_exact(&mut choice).unwrap();
            assert_eq!(choice[0], 2, "the viewer picks VNC authentication");
            s.write_all(&challenge).unwrap();
            let mut response = [0u8; 16];
            s.read_exact(&mut response).unwrap();
            let ok = response == bifrost_rfb::des::vnc_response(&challenge, "letmein");
            s.write_all(&(if ok { 0u32 } else { 1u32 }).to_be_bytes()).unwrap();
            if !ok {
                let reason = b"Authentication failed";
                s.write_all(&(reason.len() as u32).to_be_bytes()).unwrap();
                s.write_all(reason).unwrap();
                return false;
            }
            let mut client_init = [0u8; 1];
            s.read_exact(&mut client_init).unwrap();
            let mut init = Vec::new();
            init.extend_from_slice(&2u16.to_be_bytes());
            init.extend_from_slice(&1u16.to_be_bytes());
            init.extend_from_slice(&PixelFormat::VIEWER.encode());
            init.extend_from_slice(&1u32.to_be_bytes());
            init.push(b'x');
            s.write_all(&init).unwrap();
            // keep the connection open until the viewer hangs up
            let mut sink = [0u8; 64];
            while s.read(&mut sink).map_or(false, |n| n > 0) {}
            true
        });
        let conn = Connection::open_with(&format!("127.0.0.1:{port}"), Some("letmein".to_string()));
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match lock(&conn.shared.status).clone() {
                Status::Connected { .. } => break,
                Status::Closed(reason) => panic!("closed: {reason}"),
                Status::Connecting => {}
            }
            assert!(Instant::now() < deadline, "the handshake never finished");
            std::thread::sleep(Duration::from_millis(10));
        }
        drop(conn); // hangs up, which lets the server thread finish
        assert!(server.join().unwrap(), "the server accepted the response");
    }

    #[test]
    fn a_wrong_password_is_reported_with_the_servers_reason() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let challenge = [9u8; 16];
        std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let _ = s.write_all(b"RFB 003.008\n");
            let mut version = [0u8; 12];
            let _ = s.read_exact(&mut version);
            let _ = s.write_all(&[1, 2]);
            let mut choice = [0u8; 1];
            let _ = s.read_exact(&mut choice);
            let _ = s.write_all(&challenge);
            let mut response = [0u8; 16];
            let _ = s.read_exact(&mut response);
            let _ = s.write_all(&1u32.to_be_bytes());
            let _ = s.write_all(&6u32.to_be_bytes());
            let _ = s.write_all(b"denied");
        });
        let conn = Connection::open_with(&format!("127.0.0.1:{port}"), Some("wrong".to_string()));
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Status::Closed(reason) = lock(&conn.shared.status).clone() {
                assert!(reason.contains("denied"), "{reason}");
                break;
            }
            assert!(Instant::now() < deadline, "the refusal never arrived");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn a_refused_connection_reports_why() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let _ = s.write_all(b"RFB 003.008\n");
            let mut version = [0u8; 12];
            let _ = s.read_exact(&mut version);
            let _ = s.write_all(&[1, 2]); // password only
        });
        let conn = Connection::open(&format!("127.0.0.1:{port}"));
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let st = lock(&conn.shared.status).clone();
            if let Status::Closed(reason) = st {
                assert!(reason.contains("password"), "{reason}");
                break;
            }
            assert!(Instant::now() < deadline, "the refusal never arrived");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
