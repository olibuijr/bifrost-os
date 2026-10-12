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
    for block in msg.chunks_exact(64) {
        let mut w = [0u32; 80];
        for (i, word) in block.chunks_exact(4).enumerate() {
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
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
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
