//! OBEX packets: the object exchange protocol over an RFCOMM or L2CAP link (std only).
//!
//! An OBEX packet is one opcode byte (a request) or response byte, a 16-bit big-endian length that counts the whole
//! packet, a fixed part for the operations that carry one (CONNECT holds version, flags and maximum packet length;
//! SETPATH holds flags and constants), and then a run of headers. Bit 7 of a request opcode is the final bit.
//! The top two bits of a header identifier give its value type: `00` is null-terminated UTF-16BE text with a 16-bit
//! length, `01` a byte sequence with a 16-bit length, `10` one byte, `11` a four-byte value.
//!
//! This module encodes and decodes those bytes. It opens no link and no socket; the caller owns the transport and
//! feeds it the bytes it received. The decoder needs the fixed length of the packet, which depends on the operation,
//! so the caller passes it (see `fixed_len`).

/// Request opcode CONNECT (sent with the final bit: `0x80`).
pub const OP_CONNECT: u8 = 0x00;
/// Request opcode DISCONNECT (`0x81` with the final bit).
pub const OP_DISCONNECT: u8 = 0x01;
/// Request opcode PUT (`0x82` with the final bit).
pub const OP_PUT: u8 = 0x02;
/// Request opcode GET (`0x83` with the final bit).
pub const OP_GET: u8 = 0x03;
/// Request opcode SETPATH (`0x85` with the final bit).
pub const OP_SETPATH: u8 = 0x05;
/// Request opcode ABORT (`0xFF` with the final bit).
pub const OP_ABORT: u8 = 0x7F;
/// The final bit of a request opcode, or of a response code.
pub const FINAL: u8 = 0x80;
/// OBEX protocol version 1.0, the first byte of the CONNECT fixed part.
pub const VERSION_1_0: u8 = 0x10;

/// Response: continue, more packets follow.
pub const RSP_CONTINUE: u8 = 0x90;
/// Response: success.
pub const RSP_SUCCESS: u8 = 0xA0;
/// Response: bad request.
pub const RSP_BAD_REQUEST: u8 = 0xC0;
/// Response: unauthorized.
pub const RSP_UNAUTHORIZED: u8 = 0xC1;
/// Response: forbidden.
pub const RSP_FORBIDDEN: u8 = 0xC3;
/// Response: not found.
pub const RSP_NOT_FOUND: u8 = 0xC4;
/// Response: not implemented.
pub const RSP_NOT_IMPLEMENTED: u8 = 0xD1;

/// Header: name of the object, UTF-16BE text.
pub const H_NAME: u8 = 0x01;
/// Header: description, UTF-16BE text.
pub const H_DESCRIPTION: u8 = 0x05;
/// Header: ISO 8601 time, a byte sequence.
pub const H_TIME_ISO: u8 = 0x44;
/// Header: type of the object (MIME type), a byte sequence.
pub const H_TYPE: u8 = 0x42;
/// Header: target service, a byte sequence.
pub const H_TARGET: u8 = 0x46;
/// Header: body fragment, a byte sequence.
pub const H_BODY: u8 = 0x48;
/// Header: final body fragment, a byte sequence.
pub const H_END_OF_BODY: u8 = 0x49;
/// Header: who the peer is, a byte sequence.
pub const H_WHO: u8 = 0x4A;
/// Header: number of objects, a four-byte value.
pub const H_COUNT: u8 = 0xC0;
/// Header: object length in bytes, a four-byte value.
pub const H_LENGTH: u8 = 0xC3;
/// Header: time as seconds, a four-byte value.
pub const H_TIME: u8 = 0xC4;
/// Header: connection identifier, a four-byte value.
pub const H_CONNECTION_ID: u8 = 0xCB;

/// The value of one header, of the type its identifier implies.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    /// Text (identifier top bits `00`). Encoded as UTF-16BE with a terminating zero code unit.
    Text(String),
    /// A byte sequence (top bits `01`).
    Bytes(Vec<u8>),
    /// One byte (top bits `10`).
    Byte(u8),
    /// A four-byte big-endian value (top bits `11`).
    Dword(u32),
}

/// One header: its identifier and its value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    pub id: u8,
    pub value: Value,
}

/// One OBEX packet: the code (request opcode with its final bit, or a response code), the fixed part, and the headers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Packet {
    pub code: u8,
    pub fixed: Vec<u8>,
    pub headers: Vec<Header>,
}

/// Why bytes could not be encoded or decoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// Fewer bytes than the three-byte packet head.
    TooShort,
    /// The length field does not match the bytes, or a value runs past the end of the packet.
    BadLength,
    /// The packet or one header would be longer than 65535 bytes.
    TooLong,
    /// A text header is odd-length or lacks its terminating zero, or is not valid UTF-16.
    BadText,
    /// The value does not have the type that the header identifier implies.
    TypeMismatch(u8),
}

/// The fixed-part length of an operation. CONNECT has four bytes (in both directions). SETPATH has two bytes in a
/// request and none in its response. Other operations have none.
pub fn fixed_len(op: u8, response: bool) -> usize {
    match op {
        OP_CONNECT => 4,
        OP_SETPATH if !response => 2,
        _ => 0,
    }
}

/// A CONNECT request: version 1.0, no flags, and the largest packet the sender accepts.
pub fn connect_request(max_len: u16, headers: Vec<Header>) -> Packet {
    Packet { code: OP_CONNECT | FINAL, fixed: connect_fixed(max_len), headers }
}

/// A CONNECT success response with the same fixed part.
pub fn connect_response(max_len: u16, headers: Vec<Header>) -> Packet {
    Packet { code: RSP_SUCCESS, fixed: connect_fixed(max_len), headers }
}

fn connect_fixed(max_len: u16) -> Vec<u8> {
    let m = max_len.to_be_bytes();
    vec![VERSION_1_0, 0x00, m[0], m[1]]
}

impl Packet {
    /// The first header with identifier `id`.
    pub fn find(&self, id: u8) -> Option<&Value> {
        self.headers.iter().find(|h| h.id == id).map(|h| &h.value)
    }

    /// Encodes the packet. The length field is filled in from the encoded size.
    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        let mut out = vec![self.code, 0, 0];
        out.extend_from_slice(&self.fixed);
        for header in &self.headers {
            encode_header(header, &mut out)?;
        }
        if out.len() > 0xFFFF {
            return Err(Error::TooLong);
        }
        let len = (out.len() as u16).to_be_bytes();
        out[1] = len[0];
        out[2] = len[1];
        Ok(out)
    }
}

fn encode_header(header: &Header, out: &mut Vec<u8>) -> Result<(), Error> {
    out.push(header.id);
    match (header.id >> 6, &header.value) {
        (0, Value::Text(s)) => {
            let mut units: Vec<u16> = s.encode_utf16().collect();
            units.push(0);
            let len = 3 + units.len() * 2;
            if len > 0xFFFF {
                return Err(Error::TooLong);
            }
            out.extend_from_slice(&(len as u16).to_be_bytes());
            for u in units {
                out.extend_from_slice(&u.to_be_bytes());
            }
        }
        (1, Value::Bytes(b)) => {
            let len = 3 + b.len();
            if len > 0xFFFF {
                return Err(Error::TooLong);
            }
            out.extend_from_slice(&(len as u16).to_be_bytes());
            out.extend_from_slice(b);
        }
        (2, Value::Byte(v)) => out.push(*v),
        (3, Value::Dword(v)) => out.extend_from_slice(&v.to_be_bytes()),
        _ => return Err(Error::TypeMismatch(header.id)),
    }
    Ok(())
}

/// Decodes one complete packet. `bytes` must hold exactly one packet; `fixed` is the fixed-part length of its
/// operation (see `fixed_len`).
pub fn decode(bytes: &[u8], fixed: usize) -> Result<Packet, Error> {
    if bytes.len() < 3 {
        return Err(Error::TooShort);
    }
    let declared = u16::from_be_bytes([bytes[1], bytes[2]]) as usize;
    if declared != bytes.len() || 3 + fixed > bytes.len() {
        return Err(Error::BadLength);
    }
    let mut headers = Vec::new();
    let mut at = 3 + fixed;
    while at < bytes.len() {
        let id = bytes[at];
        let (value, used) = decode_value(id, &bytes[at + 1..])?;
        headers.push(Header { id, value });
        at += 1 + used;
    }
    Ok(Packet { code: bytes[0], fixed: bytes[3..3 + fixed].to_vec(), headers })
}

/// Reads one header value after its identifier. Returns the value and the number of bytes it used.
fn decode_value(id: u8, rest: &[u8]) -> Result<(Value, usize), Error> {
    match id >> 6 {
        0 => {
            let (value, used) = length_prefixed(rest)?;
            if value.len() < 2 || value.len() % 2 != 0 || value[value.len() - 2..] != [0, 0] {
                return Err(Error::BadText);
            }
            let units: Vec<u16> =
                value[..value.len() - 2].as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
            let text = String::from_utf16(&units).map_err(|_| Error::BadText)?;
            Ok((Value::Text(text), used))
        }
        1 => {
            let (value, used) = length_prefixed(rest)?;
            Ok((Value::Bytes(value.to_vec()), used))
        }
        2 => match rest.first() {
            Some(&b) => Ok((Value::Byte(b), 1)),
            None => Err(Error::BadLength),
        },
        _ => {
            if rest.len() < 4 {
                return Err(Error::BadLength);
            }
            Ok((Value::Dword(u32::from_be_bytes([rest[0], rest[1], rest[2], rest[3]])), 4))
        }
    }
}

/// Splits a length-prefixed value. The 16-bit length counts the identifier byte and the two length bytes, so `rest`
/// (which starts after the identifier) holds the length, then the value. Returns the value and the bytes used.
fn length_prefixed(rest: &[u8]) -> Result<(&[u8], usize), Error> {
    if rest.len() < 2 {
        return Err(Error::BadLength);
    }
    let total = u16::from_be_bytes([rest[0], rest[1]]) as usize;
    if total < 3 || total - 1 > rest.len() {
        return Err(Error::BadLength);
    }
    Ok((&rest[2..total - 1], total - 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(s: &str) -> Header {
        Header { id: H_NAME, value: Value::Text(s.to_string()) }
    }

    #[test]
    fn connect_request_encodes_version_flags_and_max_length() {
        let bytes = connect_request(0x2000, vec![]).encode().unwrap();
        assert_eq!(bytes, vec![0x80, 0x00, 0x07, 0x10, 0x00, 0x20, 0x00]);
    }

    #[test]
    fn a_text_name_is_utf16be_with_a_terminator_and_a_seven_byte_header() {
        let p = Packet { code: OP_PUT | FINAL, fixed: vec![], headers: vec![name("a")] };
        let bytes = p.encode().unwrap();
        assert_eq!(bytes, vec![0x82, 0x00, 0x0A, 0x01, 0x00, 0x07, 0x00, 0x61, 0x00, 0x00]);
    }

    #[test]
    fn a_four_byte_connection_id_makes_an_eight_byte_disconnect() {
        let p = Packet {
            code: OP_DISCONNECT | FINAL,
            fixed: vec![],
            headers: vec![Header { id: H_CONNECTION_ID, value: Value::Dword(1) }],
        };
        assert_eq!(p.encode().unwrap(), vec![0x81, 0x00, 0x08, 0xCB, 0x00, 0x00, 0x00, 0x01]);
    }

    #[test]
    fn decode_reads_back_the_fixed_part_and_the_text_name() {
        let connect = decode(&[0x80, 0x00, 0x07, 0x10, 0x00, 0x20, 0x00], fixed_len(OP_CONNECT, false)).unwrap();
        assert_eq!(connect.code, 0x80);
        assert_eq!(connect.fixed, vec![0x10, 0x00, 0x20, 0x00]);
        assert!(connect.headers.is_empty());
        let put = decode(&[0x82, 0x00, 0x0A, 0x01, 0x00, 0x07, 0x00, 0x61, 0x00, 0x00], 0).unwrap();
        assert_eq!(put.find(H_NAME), Some(&Value::Text("a".to_string())));
    }

    #[test]
    fn every_value_type_survives_an_encode_decode_round_trip() {
        let p = Packet {
            code: RSP_SUCCESS,
            fixed: vec![0x10, 0x00, 0x10, 0x00],
            headers: vec![
                name("Ísafjörður.txt"),
                Header { id: H_TYPE, value: Value::Bytes(b"text/plain\0".to_vec()) },
                Header { id: 0x97, value: Value::Byte(0x05) },
                Header { id: H_TIME, value: Value::Dword(0x0102_0304) },
            ],
        };
        let bytes = p.encode().unwrap();
        assert_eq!(decode(&bytes, 4).unwrap(), p);
    }

    #[test]
    fn a_declared_length_that_disagrees_with_the_bytes_is_refused() {
        let bytes = [0x80, 0x00, 0x08, 0x10, 0x00, 0x20, 0x00];
        assert_eq!(decode(&bytes, 4), Err(Error::BadLength));
    }

    #[test]
    fn a_text_header_without_its_terminator_is_refused() {
        let bytes = [0x82, 0x00, 0x0A, 0x01, 0x00, 0x07, 0x00, 0x61, 0x00, 0x41];
        assert_eq!(decode(&bytes, 0), Err(Error::BadText));
    }

    #[test]
    fn a_value_of_the_wrong_type_for_its_identifier_is_refused() {
        let p = Packet {
            code: OP_PUT,
            fixed: vec![],
            headers: vec![Header { id: H_TYPE, value: Value::Dword(1) }],
        };
        assert_eq!(p.encode(), Err(Error::TypeMismatch(H_TYPE)));
    }

    #[test]
    fn a_packet_shorter_than_its_head_is_refused() {
        assert_eq!(decode(&[0x80, 0x00], 4), Err(Error::TooShort));
    }
}
