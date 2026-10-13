//! WebSocket parity: the app-facing names for the RFC 6455 codec and handshake in `bifrost_core::websocket`, and
//! `URLSessionWebSocketTask`, a blocking client over plain TCP (`ws://`).
//!
//! The codec encodes and decodes frames and computes the `Sec-WebSocket-Accept` value of the opening
//! handshake. The task opens the connection, runs the handshake (`connect`), sends whole `Message` values (`send`),
//! receives text, binary and close messages (`receive`), answers server pings with pongs, sends pings and waits for
//! the pong (`send_ping`) and closes with a code and reason (`cancel`). Client frames are masked. TLS (`wss://`) is not
//! provided by the task.

pub use bifrost_core::websocket::{
    accept_key, check_handshake_response, decode, encode, frame_for, handshake_request, Assembler, AssemblyError, Error,
    Frame, HandshakeError, Message, Opcode,
};

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

/// Errors of `URLSessionWebSocketTask`.
#[derive(Debug, PartialEq, Eq)]
pub enum TaskError {
    CannotConnect(String),
    Handshake(HandshakeError),
    TimedOut,
    Io(String),
    Protocol(String),
    /// The connection is closed (after a close frame or `cancel`).
    Closed,
}

/// `URLSessionWebSocketTask`.
pub struct URLSessionWebSocketTask {
    stream: TcpStream,
    inbox: Vec<u8>,
    assembler: Assembler,
    ready: VecDeque<Message>,
    counter: u32,
    closed: bool,
    pub close_code: Option<u16>,
    pub close_reason: Option<String>,
}

impl URLSessionWebSocketTask {
    /// `webSocketTask(with:)` plus `resume()`: connects and runs the opening handshake.
    pub fn connect(host: &str, port: u16, path: &str, protocols: &[&str], timeout: Duration) -> Result<Self, TaskError> {
        let addr = (host, port)
            .to_socket_addrs()
            .map_err(|e| TaskError::CannotConnect(e.to_string()))?
            .next()
            .ok_or_else(|| TaskError::CannotConnect("no address".into()))?;
        let mut stream = TcpStream::connect_timeout(&addr, timeout).map_err(|e| TaskError::CannotConnect(e.to_string()))?;
        let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(1);
        let key = crate::http_auth::base64_encode(&seed.to_le_bytes());
        let hostport = format!("{host}:{port}");
        stream.write_all(&handshake_request(&hostport, path, &key, protocols)).map_err(|e| TaskError::Io(e.to_string()))?;
        stream.set_read_timeout(Some(timeout)).map_err(|e| TaskError::Io(e.to_string()))?;
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            match stream.read(&mut byte) {
                Ok(0) => return Err(TaskError::Closed),
                Ok(_) => head.push(byte[0]),
                Err(e) => return Err(io_error(e)),
            }
        }
        check_handshake_response(&head, &key).map_err(TaskError::Handshake)?;
        Ok(URLSessionWebSocketTask {
            stream,
            inbox: Vec::new(),
            assembler: Assembler::new(16 << 20),
            ready: VecDeque::new(),
            counter: (seed as u32) | 1,
            closed: false,
            close_code: None,
            close_reason: None,
        })
    }

    fn next_mask(&mut self) -> [u8; 4] {
        self.counter = self.counter.wrapping_mul(1664525).wrapping_add(1013904223);
        self.counter.to_be_bytes()
    }

    /// `send(_:completionHandler:)`.
    pub fn send(&mut self, message: &Message) -> Result<(), TaskError> {
        if self.closed {
            return Err(TaskError::Closed);
        }
        let mask = self.next_mask();
        let bytes = encode(&frame_for(message), Some(mask));
        self.stream.write_all(&bytes).map_err(|e| TaskError::Io(e.to_string()))
    }

    /// Reads frames until one whole message is ready. A ping from the server is answered with a pong and skipped.
    fn pump(&mut self, timeout: Duration) -> Result<Message, TaskError> {
        loop {
            if let Some(m) = self.ready.pop_front() {
                return Ok(m);
            }
            if self.closed {
                return Err(TaskError::Closed);
            }
            match decode(&self.inbox).map_err(|e| TaskError::Protocol(format!("{e:?}")))? {
                Some((frame, used)) => {
                    self.inbox.drain(..used);
                    let assembled = self.assembler.push(frame).map_err(|e| TaskError::Protocol(format!("{e:?}")))?;
                    match assembled {
                        Some(Message::Ping(p)) => self.send(&Message::Pong(p))?,
                        Some(Message::Close { code, reason }) => {
                            self.close_code = code;
                            self.close_reason = Some(reason.clone());
                            self.closed = true;
                            return Ok(Message::Close { code, reason });
                        }
                        Some(other) => return Ok(other),
                        None => {}
                    }
                }
                None => {
                    self.stream.set_read_timeout(Some(timeout.max(Duration::from_millis(1)))).map_err(|e| TaskError::Io(e.to_string()))?;
                    let mut chunk = [0u8; 4096];
                    match self.stream.read(&mut chunk) {
                        Ok(0) => {
                            self.closed = true;
                            return Err(TaskError::Closed);
                        }
                        Ok(n) => self.inbox.extend_from_slice(&chunk[..n]),
                        Err(e) => return Err(io_error(e)),
                    }
                }
            }
        }
    }

    /// `receive()`: the next text, binary or close message. Stray pongs are skipped.
    pub fn receive(&mut self, timeout: Duration) -> Result<Message, TaskError> {
        loop {
            match self.pump(timeout)? {
                Message::Pong(_) => continue,
                m => return Ok(m),
            }
        }
    }

    /// `sendPing(pongReceiveHandler:)`: sends a ping and waits for its pong; messages that arrive first are kept for `receive`.
    pub fn send_ping(&mut self, timeout: Duration) -> Result<(), TaskError> {
        self.send(&Message::Ping(b"bifrost".to_vec()))?;
        let mut held = Vec::new();
        let result = loop {
            match self.pump(timeout) {
                Ok(Message::Pong(_)) => break Ok(()),
                Ok(m) => held.push(m),
                Err(e) => break Err(e),
            }
        };
        for m in held.into_iter().rev() {
            self.ready.push_front(m);
        }
        result
    }

    /// `cancel(with:reason:)`: sends a close frame and closes the task.
    pub fn cancel(&mut self, code: u16, reason: &str) -> Result<(), TaskError> {
        let res = self.send(&Message::Close { code: Some(code), reason: reason.to_string() });
        self.closed = true;
        let _ = self.stream.shutdown(std::net::Shutdown::Write);
        res
    }

    pub fn is_closed(&self) -> bool {
        self.closed
    }
}

fn io_error(e: std::io::Error) -> TaskError {
    match e.kind() {
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut => TaskError::TimedOut,
        _ => TaskError::Io(e.to_string()),
    }
}
