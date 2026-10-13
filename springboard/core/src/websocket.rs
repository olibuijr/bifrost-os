//! WebSocket frames (RFC 6455, std only).
//!
//! This module encodes and decodes the frame layer of a WebSocket connection and computes the
//! `Sec-WebSocket-Accept` value of the opening handshake. A frame is a first byte (FIN flag, three
//! reserved bits, four-bit opcode), a second byte (mask flag and a seven-bit length, or an
//! extended 16-bit or 64-bit length), an optional four-byte masking key, and the payload. Client
//! frames are always masked; server frames never are. The caller owns the transport: it feeds
//! `decode` the bytes it received and writes the bytes `encode` returns.

use std::fmt;

/// The GUID appended to the client key before hashing (RFC 6455 section 1.3).
const HANDSHAKE_GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

/// The frame kind carried by the low four bits of the first byte.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Opcode {
    Continuation = 0,
    Text = 1,
    Binary = 2,
    Close = 8,
    Ping = 9,
    Pong = 10,
}

impl Opcode {
    /// Maps a four-bit opcode. Reserved values return `None`.
    pub fn from_u8(v: u8) -> Option<Opcode> {
        match v {
            0 => Some(Opcode::Continuation),
            1 => Some(Opcode::Text),
            2 => Some(Opcode::Binary),
            8 => Some(Opcode::Close),
            9 => Some(Opcode::Ping),
            10 => Some(Opcode::Pong),
            _ => None,
        }
    }

    /// Control frames (close, ping, pong) have opcodes 8 and up.
    pub fn is_control(self) -> bool {
        self as u8 >= 8
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub fin: bool,
    pub opcode: Opcode,
    /// The unmasked payload.
    pub payload: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// RSV1, RSV2 or RSV3 is set, and no extension was negotiated.
    ReservedBits,
    /// The opcode is one of the reserved values.
    UnknownOpcode(u8),
    /// A control frame has FIN clear. Control frames may not be fragmented.
    FragmentedControl,
    /// A control frame's payload is longer than 125 bytes.
    ControlTooLong,
    /// The 64-bit length has its top bit set, or does not fit in memory.
    LengthTooLarge,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::ReservedBits => write!(f, "reserved bits set"),
            Error::UnknownOpcode(v) => write!(f, "unknown opcode {}", v),
            Error::FragmentedControl => write!(f, "fragmented control frame"),
            Error::ControlTooLong => write!(f, "control frame payload over 125 bytes"),
            Error::LengthTooLarge => write!(f, "frame length too large"),
        }
    }
}

/// XORs the payload with the masking key, in place. Masking and unmasking are the same operation.
fn unmask(data: &mut [u8], key: [u8; 4]) {
    for (i, b) in data.iter_mut().enumerate() {
        *b ^= key[i % 4];
    }
}

/// Encodes one frame. Pass a masking key for a client frame, `None` for a server frame.
pub fn encode(frame: &Frame, mask: Option<[u8; 4]>) -> Vec<u8> {
    let len = frame.payload.len();
    let mut out = Vec::with_capacity(len + 14);
    out.push((if frame.fin { 0x80 } else { 0 }) | frame.opcode as u8);
    let mask_bit: u8 = if mask.is_some() { 0x80 } else { 0 };
    if len < 126 {
        out.push(mask_bit | len as u8);
    } else if len <= 0xFFFF {
        out.push(mask_bit | 126);
        out.extend_from_slice(&(len as u16).to_be_bytes());
    } else {
        out.push(mask_bit | 127);
        out.extend_from_slice(&(len as u64).to_be_bytes());
    }
    match mask {
        Some(key) => {
            out.extend_from_slice(&key);
            let start = out.len();
            out.extend_from_slice(&frame.payload);
            unmask(&mut out[start..], key);
        }
        None => out.extend_from_slice(&frame.payload),
    }
    out
}

/// Decodes the first frame in `buf`.
///
/// Returns `Ok(None)` when `buf` does not yet hold a whole frame, so the caller reads more and
/// tries again. On success returns the frame and the number of bytes it used.
pub fn decode(buf: &[u8]) -> Result<Option<(Frame, usize)>, Error> {
    if buf.len() < 2 {
        return Ok(None);
    }
    let b0 = buf[0];
    let b1 = buf[1];
    if b0 & 0x70 != 0 {
        return Err(Error::ReservedBits);
    }
    let fin = b0 & 0x80 != 0;
    let raw_op = b0 & 0x0F;
    let opcode = Opcode::from_u8(raw_op).ok_or(Error::UnknownOpcode(raw_op))?;
    let masked = b1 & 0x80 != 0;
    let (len, mut pos) = match b1 & 0x7F {
        126 => {
            if buf.len() < 4 {
                return Ok(None);
            }
            (u64::from(u16::from_be_bytes([buf[2], buf[3]])), 4)
        }
        127 => {
            if buf.len() < 10 {
                return Ok(None);
            }
            let mut l = [0u8; 8];
            l.copy_from_slice(&buf[2..10]);
            let v = u64::from_be_bytes(l);
            if v >> 63 != 0 {
                return Err(Error::LengthTooLarge);
            }
            (v, 10)
        }
        n => (u64::from(n), 2),
    };
    if opcode.is_control() {
        if !fin {
            return Err(Error::FragmentedControl);
        }
        if len > 125 {
            return Err(Error::ControlTooLong);
        }
    }
    let key = if masked {
        if buf.len() < pos + 4 {
            return Ok(None);
        }
        let k = [buf[pos], buf[pos + 1], buf[pos + 2], buf[pos + 3]];
        pos += 4;
        Some(k)
    } else {
        None
    };
    let len = usize::try_from(len).map_err(|_| Error::LengthTooLarge)?;
    let end = pos.checked_add(len).ok_or(Error::LengthTooLarge)?;
    if buf.len() < end {
        return Ok(None);
    }
    let mut payload = buf[pos..end].to_vec();
    if let Some(k) = key {
        unmask(&mut payload, k);
    }
    Ok(Some((Frame { fin, opcode, payload }, end)))
}

/// The `Sec-WebSocket-Accept` value for a client's `Sec-WebSocket-Key`.
pub fn accept_key(client_key: &str) -> String {
    let mut input = String::with_capacity(client_key.len() + HANDSHAKE_GUID.len());
    input.push_str(client_key);
    input.push_str(HANDSHAKE_GUID);
    base64(&sha1(input.as_bytes()))
}

/// SHA-1 (FIPS 180-4). Used only for the handshake; it is not a security primitive here.
fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0];
    let mut msg = data.to_vec();
    let bit_len = (data.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());
    for block in msg.as_chunks::<64>().0 {
        let mut w = [0u32; 80];
        for (i, word) in block.as_chunks::<4>().0.iter().enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let (mut a, mut b, mut c, mut d, mut e) = (h[0], h[1], h[2], h[3], h[4]);
        for (i, wi) in w.iter().enumerate() {
            let (f, k): (u32, u32) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5A827999),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
                _ => (b ^ c ^ d, 0xCA62C1D6),
            };
            let temp = a.rotate_left(5).wrapping_add(f).wrapping_add(e).wrapping_add(k).wrapping_add(*wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
    }
    let mut out = [0u8; 20];
    for (i, word) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

/// Standard base64 with padding (RFC 4648).
fn base64(data: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0];
        let b1 = *chunk.get(1).unwrap_or(&0);
        let b2 = *chunk.get(2).unwrap_or(&0);
        let n = (u32::from(b0) << 16) | (u32::from(b1) << 8) | u32::from(b2);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(TABLE[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Builds the client's opening handshake (RFC 6455 section 4.1): an HTTP/1.1 GET with the upgrade headers.
/// `key` is the base64 of 16 bytes the caller chose at random; `protocols` may be empty.
pub fn handshake_request(host: &str, path: &str, key: &str, protocols: &[&str]) -> Vec<u8> {
    let path = if path.is_empty() { "/" } else { path };
    let mut req = format!(
        "GET {path} HTTP/1.1\r\nHost: {host}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n"
    );
    if !protocols.is_empty() {
        req.push_str("Sec-WebSocket-Protocol: ");
        req.push_str(&protocols.join(", "));
        req.push_str("\r\n");
    }
    req.push_str("\r\n");
    req.into_bytes()
}

/// Why the server's answer to the opening handshake was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HandshakeError {
    /// The answer is not an HTTP status line and header block ending in a blank line.
    Malformed,
    /// The status code is not 101 Switching Protocols.
    Rejected(u16),
    /// The answer has no `Sec-WebSocket-Accept` header.
    MissingAccept,
    /// The accept value is not the one computed from the client's key.
    WrongAccept,
}

impl fmt::Display for HandshakeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HandshakeError::Malformed => write!(f, "malformed handshake response"),
            HandshakeError::Rejected(code) => write!(f, "server answered {code}, not 101"),
            HandshakeError::MissingAccept => write!(f, "no Sec-WebSocket-Accept header"),
            HandshakeError::WrongAccept => write!(f, "Sec-WebSocket-Accept does not match the key"),
        }
    }
}

/// Checks the server's answer to the opening handshake. `response` holds the bytes up to and including the blank
/// line that ends the headers; `client_key` is the key the client sent.
pub fn check_handshake_response(response: &[u8], client_key: &str) -> Result<(), HandshakeError> {
    let text = std::str::from_utf8(response).map_err(|_| HandshakeError::Malformed)?;
    let end = text.find("\r\n\r\n").ok_or(HandshakeError::Malformed)?;
    let mut lines = text[..end].split("\r\n");
    let status = lines.next().ok_or(HandshakeError::Malformed)?;
    let mut parts = status.splitn(3, ' ');
    if !parts.next().unwrap_or("").starts_with("HTTP/") {
        return Err(HandshakeError::Malformed);
    }
    let code: u16 = parts.next().and_then(|c| c.parse().ok()).ok_or(HandshakeError::Malformed)?;
    if code != 101 {
        return Err(HandshakeError::Rejected(code));
    }
    let accept = lines
        .find_map(|line| {
            let (k, v) = line.split_once(':')?;
            if k.trim().eq_ignore_ascii_case("sec-websocket-accept") {
                Some(v.trim().to_string())
            } else {
                None
            }
        })
        .ok_or(HandshakeError::MissingAccept)?;
    if accept == accept_key(client_key) {
        Ok(())
    } else {
        Err(HandshakeError::WrongAccept)
    }
}

/// A whole message, after its fragments are joined.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Message {
    Text(String),
    Binary(Vec<u8>),
    Ping(Vec<u8>),
    Pong(Vec<u8>),
    /// A close frame. The code and reason are absent when the peer sent an empty payload.
    Close { code: Option<u16>, reason: String },
}

/// Why a frame sequence could not be joined into messages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssemblyError {
    /// A continuation frame arrived with no fragmented message open.
    UnexpectedContinuation,
    /// A new data message started while a fragmented one was still open.
    UnexpectedData,
    /// A text message is not valid UTF-8.
    InvalidUtf8,
    /// A close payload is one byte long. It must be empty or hold a two-byte code.
    BadClosePayload,
    /// The message is longer than the limit the assembler was given.
    TooLarge,
}

impl fmt::Display for AssemblyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AssemblyError::UnexpectedContinuation => write!(f, "continuation frame without a message"),
            AssemblyError::UnexpectedData => write!(f, "data frame inside a fragmented message"),
            AssemblyError::InvalidUtf8 => write!(f, "text message is not UTF-8"),
            AssemblyError::BadClosePayload => write!(f, "close payload is one byte"),
            AssemblyError::TooLarge => write!(f, "message is over the size limit"),
        }
    }
}

/// Joins the frames of fragmented messages (RFC 6455 section 5.4). Control frames are returned at once, even in
/// the middle of a fragmented message, as the RFC requires.
pub struct Assembler {
    limit: usize,
    kind: Option<Opcode>,
    buf: Vec<u8>,
}

impl Assembler {
    /// An assembler that refuses messages longer than `limit` bytes.
    pub fn new(limit: usize) -> Self {
        Assembler { limit, kind: None, buf: Vec::new() }
    }

    /// Feeds one decoded frame. Returns the message the frame completes, if any.
    pub fn push(&mut self, frame: Frame) -> Result<Option<Message>, AssemblyError> {
        match frame.opcode {
            Opcode::Ping => return Ok(Some(Message::Ping(frame.payload))),
            Opcode::Pong => return Ok(Some(Message::Pong(frame.payload))),
            Opcode::Close => return close_message(&frame.payload).map(Some),
            Opcode::Continuation => {
                if self.kind.is_none() {
                    return Err(AssemblyError::UnexpectedContinuation);
                }
            }
            Opcode::Text | Opcode::Binary => {
                if self.kind.is_some() {
                    return Err(AssemblyError::UnexpectedData);
                }
                self.kind = Some(frame.opcode);
            }
        }
        if self.buf.len() + frame.payload.len() > self.limit {
            self.kind = None;
            self.buf.clear();
            return Err(AssemblyError::TooLarge);
        }
        self.buf.extend_from_slice(&frame.payload);
        if !frame.fin {
            return Ok(None);
        }
        let kind = self.kind.take().unwrap_or(Opcode::Binary);
        let data = std::mem::take(&mut self.buf);
        match kind {
            Opcode::Text => String::from_utf8(data).map(|s| Some(Message::Text(s))).map_err(|_| AssemblyError::InvalidUtf8),
            _ => Ok(Some(Message::Binary(data))),
        }
    }
}

/// Reads a close payload: a two-byte code, then a UTF-8 reason. An empty payload has neither.
fn close_message(payload: &[u8]) -> Result<Message, AssemblyError> {
    match payload.len() {
        0 => Ok(Message::Close { code: None, reason: String::new() }),
        1 => Err(AssemblyError::BadClosePayload),
        _ => {
            let code = u16::from_be_bytes([payload[0], payload[1]]);
            let reason = String::from_utf8(payload[2..].to_vec()).map_err(|_| AssemblyError::InvalidUtf8)?;
            Ok(Message::Close { code: Some(code), reason })
        }
    }
}

/// The single frame that carries a whole message (FIN set). Close without a code sends an empty payload.
pub fn frame_for(msg: &Message) -> Frame {
    let (opcode, payload) = match msg {
        Message::Text(s) => (Opcode::Text, s.as_bytes().to_vec()),
        Message::Binary(b) => (Opcode::Binary, b.clone()),
        Message::Ping(p) => (Opcode::Ping, p.clone()),
        Message::Pong(p) => (Opcode::Pong, p.clone()),
        Message::Close { code, reason } => {
            let mut p = Vec::new();
            if let Some(c) = code {
                p.extend_from_slice(&c.to_be_bytes());
                p.extend_from_slice(reason.as_bytes());
            }
            (Opcode::Close, p)
        }
    };
    Frame { fin: true, opcode, payload }
}

#[cfg(test)]
mod session_tests {
    use super::*;

    const RFC_KEY: &str = "dGhlIHNhbXBsZSBub25jZQ==";

    #[test]
    fn handshake_request_lists_the_upgrade_headers() {
        let req = handshake_request("example.test", "/chat", RFC_KEY, &["chat", "superchat"]);
        let text = String::from_utf8(req).unwrap();
        assert_eq!(
            text,
            "GET /chat HTTP/1.1\r\nHost: example.test\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
             Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\
             Sec-WebSocket-Protocol: chat, superchat\r\n\r\n"
        );
        // An empty path becomes "/" and no protocol header is sent without protocols.
        let bare = String::from_utf8(handshake_request("h", "", RFC_KEY, &[])).unwrap();
        assert!(bare.starts_with("GET / HTTP/1.1\r\n"));
        assert!(!bare.contains("Sec-WebSocket-Protocol"));
    }

    #[test]
    fn handshake_response_is_checked_against_the_key() {
        let ok = b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
                   Sec-WebSocket-Accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo=\r\n\r\n";
        assert_eq!(check_handshake_response(ok, RFC_KEY), Ok(()));
        let wrong = b"HTTP/1.1 101 Switching Protocols\r\nSec-WebSocket-Accept: AAAAAAAAAAAAAAAAAAAAAAAAAAA=\r\n\r\n";
        assert_eq!(check_handshake_response(wrong, RFC_KEY), Err(HandshakeError::WrongAccept));
        let refused = b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n";
        assert_eq!(check_handshake_response(refused, RFC_KEY), Err(HandshakeError::Rejected(200)));
        let no_accept = b"HTTP/1.1 101 Switching Protocols\r\n\r\n";
        assert_eq!(check_handshake_response(no_accept, RFC_KEY), Err(HandshakeError::MissingAccept));
        assert_eq!(check_handshake_response(b"HTTP/1.1 101", RFC_KEY), Err(HandshakeError::Malformed));
    }

    fn frame(fin: bool, opcode: Opcode, payload: &[u8]) -> Frame {
        Frame { fin, opcode, payload: payload.to_vec() }
    }

    #[test]
    fn fragments_join_into_one_text_message() {
        let mut a = Assembler::new(1024);
        assert_eq!(a.push(frame(false, Opcode::Text, b"Hel")), Ok(None));
        assert_eq!(a.push(frame(true, Opcode::Continuation, b"lo")), Ok(Some(Message::Text("Hello".into()))));
        // The assembler is idle again and takes a new message.
        assert_eq!(a.push(frame(true, Opcode::Binary, &[1, 2])), Ok(Some(Message::Binary(vec![1, 2]))));
    }

    #[test]
    fn ping_inside_a_fragmented_message_is_returned_at_once() {
        let mut a = Assembler::new(1024);
        assert_eq!(a.push(frame(false, Opcode::Binary, &[9])), Ok(None));
        assert_eq!(a.push(frame(true, Opcode::Ping, b"p")), Ok(Some(Message::Ping(b"p".to_vec()))));
        assert_eq!(a.push(frame(true, Opcode::Continuation, &[8])), Ok(Some(Message::Binary(vec![9, 8]))));
    }

    #[test]
    fn out_of_order_frames_are_errors() {
        let mut a = Assembler::new(1024);
        assert_eq!(a.push(frame(true, Opcode::Continuation, b"x")), Err(AssemblyError::UnexpectedContinuation));
        assert_eq!(a.push(frame(false, Opcode::Text, b"a")), Ok(None));
        assert_eq!(a.push(frame(true, Opcode::Text, b"b")), Err(AssemblyError::UnexpectedData));
    }

    #[test]
    fn invalid_utf8_and_size_limit_are_refused() {
        let mut a = Assembler::new(1024);
        // 0xC3 starts a two-byte sequence that is never finished.
        assert_eq!(a.push(frame(true, Opcode::Text, &[0xC3])), Err(AssemblyError::InvalidUtf8));
        let mut small = Assembler::new(4);
        assert_eq!(small.push(frame(false, Opcode::Binary, b"abc")), Ok(None));
        assert_eq!(small.push(frame(true, Opcode::Continuation, b"de")), Err(AssemblyError::TooLarge));
        // The refused message is dropped, so the next one starts clean.
        assert_eq!(small.push(frame(true, Opcode::Binary, b"ok")), Ok(Some(Message::Binary(b"ok".to_vec()))));
    }

    #[test]
    fn close_payload_carries_code_and_reason() {
        let mut a = Assembler::new(1024);
        let close = frame(true, Opcode::Close, &[0x03, 0xE8, b'b', b'y', b'e']);
        assert_eq!(a.push(close), Ok(Some(Message::Close { code: Some(1000), reason: "bye".into() })));
        assert_eq!(a.push(frame(true, Opcode::Close, &[])), Ok(Some(Message::Close { code: None, reason: String::new() })));
        assert_eq!(a.push(frame(true, Opcode::Close, &[0x03])), Err(AssemblyError::BadClosePayload));
        let out = frame_for(&Message::Close { code: Some(1000), reason: "bye".into() });
        assert_eq!(out.payload, vec![0x03, 0xE8, b'b', b'y', b'e']);
        assert!(out.fin);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accept_key_matches_the_rfc_example() {
        assert_eq!(accept_key("dGhlIHNhbXBsZSBub25jZQ=="), "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=");
    }

    #[test]
    fn sha1_and_base64_match_their_known_vectors() {
        let digest = sha1(b"abc");
        let hex: String = digest.iter().map(|b| format!("{:02x}", b)).collect();
        assert_eq!(hex, "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(base64(b"Man"), "TWFu");
        assert_eq!(base64(b"Ma"), "TWE=");
        assert_eq!(base64(b"M"), "TQ==");
    }

    #[test]
    fn decodes_an_unmasked_server_text_frame() {
        let (frame, used) = decode(&[0x81, 0x05, b'H', b'e', b'l', b'l', b'o']).unwrap().unwrap();
        assert_eq!(used, 7);
        assert!(frame.fin);
        assert_eq!(frame.opcode, Opcode::Text);
        assert_eq!(frame.payload, b"Hello");
    }

    #[test]
    fn decodes_the_masked_client_example_from_the_rfc() {
        let wire = [0x81, 0x85, 0x37, 0xfa, 0x21, 0x3d, 0x7f, 0x9f, 0x4d, 0x51, 0x58];
        let (frame, used) = decode(&wire).unwrap().unwrap();
        assert_eq!(used, 11);
        assert_eq!(frame.payload, b"Hello");
        // Encoding the same payload with the same key gives the same bytes back.
        let out = encode(&frame, Some([0x37, 0xfa, 0x21, 0x3d]));
        assert_eq!(out, wire.to_vec());
    }

    #[test]
    fn extended_length_uses_sixteen_bit_field() {
        let frame = Frame { fin: true, opcode: Opcode::Binary, payload: vec![7u8; 126] };
        let out = encode(&frame, None);
        assert_eq!(&out[..4], &[0x82, 0x7E, 0x00, 0x7E]);
        let (back, used) = decode(&out).unwrap().unwrap();
        assert_eq!(used, 130);
        assert_eq!(back.payload.len(), 126);
    }

    #[test]
    fn incomplete_frame_asks_for_more_bytes() {
        assert_eq!(decode(&[0x81]).unwrap(), None);
        // Header says five payload bytes, only two have arrived.
        assert_eq!(decode(&[0x81, 0x05, b'H', b'e']).unwrap(), None);
    }

    #[test]
    fn rejects_reserved_bits_and_long_control_frames() {
        assert_eq!(decode(&[0xC1, 0x00]), Err(Error::ReservedBits));
        // Ping (0x89) with a 126-byte length: control payloads stop at 125.
        assert_eq!(decode(&[0x89, 0x7E, 0x00, 0x7E]), Err(Error::ControlTooLong));
        // Ping with FIN clear is a fragmented control frame.
        assert_eq!(decode(&[0x09, 0x00]), Err(Error::FragmentedControl));
    }
}
