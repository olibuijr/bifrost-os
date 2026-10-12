//! Bluetooth SDP data elements: the typed values in a service record (std only).
//!
//! Every element starts with one header byte: five bits of type descriptor (0 nil, 1 unsigned
//! integer, 2 signed integer, 3 UUID, 4 text string, 5 boolean, 6 sequence, 7 alternative,
//! 8 URL) and three bits of size descriptor. For integers and UUIDs the size descriptor
//! names a fixed size (0 to 4: 1, 2, 4, 8 or 16 bytes, depending on the type). For text,
//! URL, sequence and alternative elements, size descriptors 5, 6 and 7 mean that a length
//! of 1, 2 or 4 bytes follows the header. All multi-byte values are big-endian.
//! This module encodes and decodes that byte form. It opens no socket and does no SDP query;
//! the transport hands it the bytes it received.

/// Deepest nesting the decoder accepts, so a hostile record cannot exhaust the stack.
const MAX_DEPTH: usize = 16;

const TYPE_NIL: u8 = 0;
const TYPE_UINT: u8 = 1;
const TYPE_SINT: u8 = 2;
const TYPE_UUID: u8 = 3;
const TYPE_TEXT: u8 = 4;
const TYPE_BOOL: u8 = 5;
const TYPE_SEQ: u8 = 6;
const TYPE_ALT: u8 = 7;
const TYPE_URL: u8 = 8;

/// A service class or attribute UUID: 16, 32 or 128 bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Uuid {
    /// A 16-bit UUID assigned by the Bluetooth SIG.
    U16(u16),
    /// A 32-bit UUID.
    U32(u32),
    /// A full 128-bit UUID, most significant byte first.
    U128([u8; 16]),
}

/// One SDP data element.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Element {
    /// The nil element: no value.
    Nil,
    /// An unsigned integer, up to 64 bits.
    UInt(u64),
    /// A signed integer, up to 64 bits.
    SInt(i64),
    /// A UUID.
    Uuid(Uuid),
    /// A text string, as raw bytes.
    Text(Vec<u8>),
    /// A boolean.
    Bool(bool),
    /// An ordered list of elements.
    Sequence(Vec<Element>),
    /// A list of elements from which a reader picks one.
    Alternative(Vec<Element>),
    /// A URL, as raw bytes.
    Url(Vec<u8>),
}

/// Why bytes could not be decoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The bytes end before the element they describe.
    Truncated,
    /// The type descriptor is not one of the nine defined types.
    BadType(u8),
    /// The size descriptor does not fit the type.
    BadSize(u8),
    /// Sequences are nested deeper than the decoder accepts.
    TooDeep,
    /// Bytes remain after the top-level element.
    TrailingBytes,
}

/// Writes a header for a variable-length element, choosing the shortest length field.
fn var_header(out: &mut Vec<u8>, type_code: u8, len: usize) {
    if len <= 0xFF {
        out.push((type_code << 3) | 5);
        out.push(len as u8);
    } else if len <= 0xFFFF {
        out.push((type_code << 3) | 6);
        out.extend_from_slice(&(len as u16).to_be_bytes());
    } else {
        out.push((type_code << 3) | 7);
        out.extend_from_slice(&(len as u32).to_be_bytes());
    }
}

fn put(out: &mut Vec<u8>, e: &Element) {
    match e {
        Element::Nil => out.push(TYPE_NIL << 3),
        Element::UInt(v) => {
            let (size, n) = if *v <= 0xFF {
                (0, 1)
            } else if *v <= 0xFFFF {
                (1, 2)
            } else if *v <= 0xFFFF_FFFF {
                (2, 4)
            } else {
                (3, 8)
            };
            out.push((TYPE_UINT << 3) | size);
            out.extend_from_slice(&v.to_be_bytes()[8 - n..]);
        }
        Element::SInt(v) => {
            let (size, n) = if (-128..=127).contains(v) {
                (0, 1)
            } else if (-32768..=32767).contains(v) {
                (1, 2)
            } else if (i32::MIN as i64..=i32::MAX as i64).contains(v) {
                (2, 4)
            } else {
                (3, 8)
            };
            out.push((TYPE_SINT << 3) | size);
            out.extend_from_slice(&v.to_be_bytes()[8 - n..]);
        }
        Element::Uuid(u) => match u {
            Uuid::U16(x) => {
                out.push((TYPE_UUID << 3) | 1);
                out.extend_from_slice(&x.to_be_bytes());
            }
            Uuid::U32(x) => {
                out.push((TYPE_UUID << 3) | 2);
                out.extend_from_slice(&x.to_be_bytes());
            }
            Uuid::U128(x) => {
                out.push((TYPE_UUID << 3) | 4);
                out.extend_from_slice(x);
            }
        },
        Element::Text(s) => {
            var_header(out, TYPE_TEXT, s.len());
            out.extend_from_slice(s);
        }
        Element::Url(s) => {
            var_header(out, TYPE_URL, s.len());
            out.extend_from_slice(s);
        }
        Element::Bool(b) => {
            out.push(TYPE_BOOL << 3);
            out.push(*b as u8);
        }
        Element::Sequence(items) | Element::Alternative(items) => {
            let type_code = if matches!(e, Element::Sequence(_)) { TYPE_SEQ } else { TYPE_ALT };
            let mut body = Vec::new();
            for item in items {
                put(&mut body, item);
            }
            var_header(out, type_code, body.len());
            out.extend_from_slice(&body);
        }
    }
}

/// Encodes one element to its wire bytes.
pub fn encode(e: &Element) -> Vec<u8> {
    let mut out = Vec::new();
    put(&mut out, e);
    out
}

/// The byte length of a fixed-size integer or UUID for a size descriptor.
fn fixed_len(type_code: u8, size: u8) -> Result<usize, Error> {
    match (type_code, size) {
        (TYPE_UUID, 1) => Ok(2),
        (TYPE_UUID, 2) => Ok(4),
        (TYPE_UUID, 4) => Ok(16),
        (TYPE_UUID, _) => Err(Error::BadSize(size)),
        (_, 0) => Ok(1),
        (_, 1) => Ok(2),
        (_, 2) => Ok(4),
        (_, 3) => Ok(8),
        _ => Err(Error::BadSize(size)),
    }
}

/// The bytes at `pos..pos + n`, or `Truncated` when the input is too short.
fn take(b: &[u8], pos: usize, n: usize) -> Result<&[u8], Error> {
    b.get(pos..pos + n).ok_or(Error::Truncated)
}

/// Reads one element from the start of `b`; returns it with the number of bytes it used.
fn read(b: &[u8], depth: usize) -> Result<(Element, usize), Error> {
    if depth > MAX_DEPTH {
        return Err(Error::TooDeep);
    }
    let head = *b.first().ok_or(Error::Truncated)?;
    let type_code = head >> 3;
    let size = head & 0x07;
    match type_code {
        TYPE_NIL => {
            if size != 0 {
                return Err(Error::BadSize(size));
            }
            Ok((Element::Nil, 1))
        }
        TYPE_UINT | TYPE_SINT | TYPE_UUID => {
            let n = fixed_len(type_code, size)?;
            let raw = take(b, 1, n)?;
            let elem = match type_code {
                TYPE_UUID => Element::Uuid(match n {
                    2 => Uuid::U16(u16::from_be_bytes([raw[0], raw[1]])),
                    4 => Uuid::U32(u32::from_be_bytes([raw[0], raw[1], raw[2], raw[3]])),
                    _ => {
                        let mut full = [0u8; 16];
                        full.copy_from_slice(raw);
                        Uuid::U128(full)
                    }
                }),
                _ => {
                    let value = raw.iter().fold(0u64, |acc, &x| (acc << 8) | x as u64);
                    if type_code == TYPE_UINT {
                        Element::UInt(value)
                    } else {
                        // Sign-extend from n bytes: move the top bit of the field to bit 63.
                        let shift = 64 - 8 * n as u32;
                        Element::SInt(((value << shift) as i64) >> shift)
                    }
                }
            };
            Ok((elem, 1 + n))
        }
        TYPE_BOOL => {
            if size != 0 {
                return Err(Error::BadSize(size));
            }
            let v = take(b, 1, 1)?[0];
            Ok((Element::Bool(v != 0), 2))
        }
        TYPE_TEXT | TYPE_URL | TYPE_SEQ | TYPE_ALT => {
            let (len, pos) = match size {
                5 => (take(b, 1, 1)?[0] as usize, 2),
                6 => {
                    let raw = take(b, 1, 2)?;
                    (u16::from_be_bytes([raw[0], raw[1]]) as usize, 3)
                }
                7 => {
                    let raw = take(b, 1, 4)?;
                    (u32::from_be_bytes([raw[0], raw[1], raw[2], raw[3]]) as usize, 5)
                }
                _ => return Err(Error::BadSize(size)),
            };
            let body = take(b, pos, len)?;
            let elem = match type_code {
                TYPE_TEXT => Element::Text(body.to_vec()),
                TYPE_URL => Element::Url(body.to_vec()),
                _ => {
                    let mut items = Vec::new();
                    let mut off = 0;
                    while off < body.len() {
                        let (item, used) = read(&body[off..], depth + 1)?;
                        items.push(item);
                        off += used;
                    }
                    if type_code == TYPE_SEQ {
                        Element::Sequence(items)
                    } else {
                        Element::Alternative(items)
                    }
                }
            };
            Ok((elem, pos + len))
        }
        other => Err(Error::BadType(other)),
    }
}

/// Decodes bytes that hold exactly one top-level element.
pub fn decode(b: &[u8]) -> Result<Element, Error> {
    let (elem, used) = read(b, 0)?;
    if used != b.len() {
        return Err(Error::TrailingBytes);
    }
    Ok(elem)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsigned_integers_use_the_smallest_size() {
        assert_eq!(encode(&Element::UInt(5)), vec![0x08, 0x05]);
        assert_eq!(encode(&Element::UInt(0x1234)), vec![0x09, 0x12, 0x34]);
        assert_eq!(encode(&Element::UInt(0x0001_0000)), vec![0x0A, 0x00, 0x01, 0x00, 0x00]);
    }

    #[test]
    fn a_16_bit_uuid_is_type_three_size_one() {
        // Type 3 shifted by three is 0x18; size descriptor 1 gives 0x19, then the two bytes.
        assert_eq!(encode(&Element::Uuid(Uuid::U16(0x1101))), vec![0x19, 0x11, 0x01]);
    }

    #[test]
    fn text_and_sequence_carry_their_byte_length() {
        assert_eq!(encode(&Element::Text(b"ab".to_vec())), vec![0x25, 0x02, 0x61, 0x62]);
        let seq = Element::Sequence(vec![Element::UInt(5), Element::Bool(true)]);
        // Body is 08 05 28 01 (four bytes); header is type 6 with a one-byte length: 35 04.
        assert_eq!(encode(&seq), vec![0x35, 0x04, 0x08, 0x05, 0x28, 0x01]);
    }

    #[test]
    fn signed_integers_round_trip_with_sign_extension() {
        assert_eq!(encode(&Element::SInt(-1)), vec![0x10, 0xFF]);
        // -200 is 0xFF38 in 16 bits, under the two-byte signed size (size descriptor 1).
        assert_eq!(encode(&Element::SInt(-200)), vec![0x11, 0xFF, 0x38]);
        assert_eq!(decode(&[0x11, 0xFF, 0x38]).unwrap(), Element::SInt(-200));
        assert_eq!(decode(&[0x10, 0xFF]).unwrap(), Element::SInt(-1));
    }

    #[test]
    fn nested_tree_round_trips() {
        let tree = Element::Sequence(vec![
            Element::Text(b"x".to_vec()),
            Element::Alternative(vec![Element::Uuid(Uuid::U32(0x0001_1101)), Element::Nil]),
            Element::Url(b"http://a".to_vec()),
            Element::Sequence(vec![Element::UInt(0xFFFF_FFFF_FF), Element::Bool(false)]),
        ]);
        assert_eq!(decode(&encode(&tree)).unwrap(), tree);
    }

    #[test]
    fn a_length_longer_than_the_input_is_truncated() {
        // A sequence declaring five body bytes with only two present.
        assert_eq!(decode(&[0x35, 0x05, 0x08, 0x05]), Err(Error::Truncated));
    }

    #[test]
    fn extra_bytes_after_the_element_are_refused() {
        assert_eq!(decode(&[0x00, 0x00]), Err(Error::TrailingBytes));
    }

    #[test]
    fn unknown_types_and_wrong_sizes_are_refused() {
        // Type 9 is not defined: header 9 << 3 = 0x48.
        assert_eq!(decode(&[0x48]), Err(Error::BadType(9)));
        // Unsigned integer with size descriptor 5 is not a fixed size.
        assert_eq!(decode(&[0x0D]), Err(Error::BadSize(5)));
    }
}
