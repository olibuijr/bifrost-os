//! CBOR (RFC 8949) data items: a std-only encoder and a bounded decoder.
//!
//! CBOR is the binary data format that credential, attestation and device-protocol
//! messages use. A data item is a major type in the top three bits of the initial byte
//! and an argument in the remaining five bits, followed by 1, 2, 4 or 8 extra bytes when
//! the argument does not fit. This module covers all eight major types: unsigned and
//! negative integers, byte and text strings, arrays, maps, tags, and simple values with
//! floats. Indefinite-length items are refused, and decoding stops at a fixed nesting
//! depth so a hostile message cannot overflow the stack. Map keys are kept in the order
//! they were written.

use std::fmt;

/// Deepest array, map and tag nesting that decoding accepts.
pub const MAX_DEPTH: usize = 64;

/// One decoded CBOR data item.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// Major type 0: an unsigned integer.
    Unsigned(u64),
    /// Major type 1: a negative integer, stored as the value it encodes (-1 - n).
    Negative(u64),
    /// Major type 2: a byte string.
    Bytes(Vec<u8>),
    /// Major type 3: a UTF-8 text string.
    Text(String),
    /// Major type 4: an array.
    Array(Vec<Value>),
    /// Major type 5: a map, as key and value pairs in encoded order.
    Map(Vec<(Value, Value)>),
    /// Major type 6: a tagged item.
    Tag(u64, Box<Value>),
    /// Major type 7: false, true or null.
    Simple(u8),
    /// Major type 7: a half, single or double precision float, widened to f64.
    Float(f64),
}

/// Why a CBOR item was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CborError {
    /// The input ended in the middle of an item.
    Truncated,
    /// An indefinite-length item, a reserved argument or an unassigned simple value.
    Unsupported,
    /// A text string is not valid UTF-8.
    BadUtf8,
    /// Nesting is deeper than [`MAX_DEPTH`].
    TooDeep,
    /// Bytes remain after the top-level item.
    Trailing,
    /// A length does not fit in memory on this platform.
    TooLong,
}

impl fmt::Display for CborError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            CborError::Truncated => "CBOR item is truncated",
            CborError::Unsupported => "CBOR item uses an unsupported or reserved encoding",
            CborError::BadUtf8 => "CBOR text string is not valid UTF-8",
            CborError::TooDeep => "CBOR nesting is too deep",
            CborError::Trailing => "CBOR input has bytes after the top-level item",
            CborError::TooLong => "CBOR length does not fit in memory",
        };
        f.write_str(text)
    }
}

/// Encodes a value to its shortest definite-length CBOR form.
pub fn encode(value: &Value) -> Vec<u8> {
    let mut out = Vec::new();
    write_value(value, &mut out);
    out
}

/// Decodes exactly one data item that fills the whole input.
pub fn decode(bytes: &[u8]) -> Result<Value, CborError> {
    let mut reader = Reader { bytes, pos: 0 };
    let value = reader.value(0)?;
    if reader.pos != bytes.len() {
        return Err(CborError::Trailing);
    }
    Ok(value)
}

fn write_head(major: u8, arg: u64, out: &mut Vec<u8>) {
    let m = major << 5;
    if arg < 24 {
        out.push(m | arg as u8);
    } else if arg <= 0xff {
        out.push(m | 24);
        out.push(arg as u8);
    } else if arg <= 0xffff {
        out.push(m | 25);
        out.extend_from_slice(&(arg as u16).to_be_bytes());
    } else if arg <= 0xffff_ffff {
        out.push(m | 26);
        out.extend_from_slice(&(arg as u32).to_be_bytes());
    } else {
        out.push(m | 27);
        out.extend_from_slice(&arg.to_be_bytes());
    }
}

fn write_value(value: &Value, out: &mut Vec<u8>) {
    match value {
        Value::Unsigned(n) => write_head(0, *n, out),
        Value::Negative(n) => write_head(1, *n, out),
        Value::Bytes(b) => {
            write_head(2, b.len() as u64, out);
            out.extend_from_slice(b);
        }
        Value::Text(s) => {
            write_head(3, s.len() as u64, out);
            out.extend_from_slice(s.as_bytes());
        }
        Value::Array(items) => {
            write_head(4, items.len() as u64, out);
            for item in items {
                write_value(item, out);
            }
        }
        Value::Map(pairs) => {
            write_head(5, pairs.len() as u64, out);
            for (k, v) in pairs {
                write_value(k, out);
                write_value(v, out);
            }
        }
        Value::Tag(tag, inner) => {
            write_head(6, *tag, out);
            write_value(inner, out);
        }
        Value::Simple(v) => {
            if *v < 24 {
                out.push(0xe0 | v);
            } else {
                out.push(0xf8);
                out.push(*v);
            }
        }
        Value::Float(f) => {
            // Doubles are always written in full width, which round-trips exactly.
            out.push(0xfb);
            out.extend_from_slice(&f.to_bits().to_be_bytes());
        }
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], CborError> {
        let end = self.pos.checked_add(n).ok_or(CborError::TooLong)?;
        if end > self.bytes.len() {
            return Err(CborError::Truncated);
        }
        let slice = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    fn byte(&mut self) -> Result<u8, CborError> {
        Ok(self.take(1)?[0])
    }

    /// Reads the argument of an initial byte whose additional information is `info`.
    fn argument(&mut self, info: u8) -> Result<u64, CborError> {
        match info {
            0..=23 => Ok(u64::from(info)),
            24 => Ok(u64::from(self.byte()?)),
            25 => Ok(u64::from(u16::from_be_bytes(self.take(2)?.try_into().unwrap()))),
            26 => Ok(u64::from(u32::from_be_bytes(self.take(4)?.try_into().unwrap()))),
            27 => Ok(u64::from_be_bytes(self.take(8)?.try_into().unwrap())),
            _ => Err(CborError::Unsupported),
        }
    }

    fn length(&mut self, info: u8) -> Result<usize, CborError> {
        usize::try_from(self.argument(info)?).map_err(|_| CborError::TooLong)
    }

    fn value(&mut self, depth: usize) -> Result<Value, CborError> {
        if depth > MAX_DEPTH {
            return Err(CborError::TooDeep);
        }
        let initial = self.byte()?;
        let major = initial >> 5;
        let info = initial & 0x1f;
        match major {
            0 => Ok(Value::Unsigned(self.argument(info)?)),
            1 => Ok(Value::Negative(self.argument(info)?)),
            2 => {
                let n = self.length(info)?;
                Ok(Value::Bytes(self.take(n)?.to_vec()))
            }
            3 => {
                let n = self.length(info)?;
                let raw = self.take(n)?;
                let text = std::str::from_utf8(raw).map_err(|_| CborError::BadUtf8)?;
                Ok(Value::Text(text.to_string()))
            }
            4 => {
                let n = self.length(info)?;
                let mut items = Vec::with_capacity(n.min(self.bytes.len()));
                for _ in 0..n {
                    items.push(self.value(depth + 1)?);
                }
                Ok(Value::Array(items))
            }
            5 => {
                let n = self.length(info)?;
                let mut pairs = Vec::with_capacity(n.min(self.bytes.len()));
                for _ in 0..n {
                    let k = self.value(depth + 1)?;
                    let v = self.value(depth + 1)?;
                    pairs.push((k, v));
                }
                Ok(Value::Map(pairs))
            }
            6 => {
                let tag = self.argument(info)?;
                Ok(Value::Tag(tag, Box::new(self.value(depth + 1)?)))
            }
            _ => match info {
                20 => Ok(Value::Simple(20)),
                21 => Ok(Value::Simple(21)),
                22 => Ok(Value::Simple(22)),
                23 => Ok(Value::Simple(23)),
                24 => {
                    let v = self.byte()?;
                    if v < 32 {
                        return Err(CborError::Unsupported);
                    }
                    Ok(Value::Simple(v))
                }
                25 => {
                    let bits = u16::from_be_bytes(self.take(2)?.try_into().unwrap());
                    Ok(Value::Float(half_to_f64(bits)))
                }
                26 => {
                    let bits = u32::from_be_bytes(self.take(4)?.try_into().unwrap());
                    Ok(Value::Float(f64::from(f32::from_bits(bits))))
                }
                27 => {
                    let bits = u64::from_be_bytes(self.take(8)?.try_into().unwrap());
                    Ok(Value::Float(f64::from_bits(bits)))
                }
                _ => Err(CborError::Unsupported),
            },
        }
    }
}

/// Converts an IEEE 754 binary16 bit pattern to f64.
fn half_to_f64(bits: u16) -> f64 {
    let sign = if bits & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exp = i32::from((bits >> 10) & 0x1f);
    let frac = f64::from(bits & 0x3ff);
    match exp {
        0 => sign * frac * 2f64.powi(-24),
        31 => {
            if frac == 0.0 {
                sign * f64::INFINITY
            } else {
                f64::NAN
            }
        }
        _ => sign * (1.0 + frac / 1024.0) * 2f64.powi(exp - 15),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_shortest_integer_heads() {
        assert_eq!(encode(&Value::Unsigned(0)), vec![0x00]);
        assert_eq!(encode(&Value::Unsigned(23)), vec![0x17]);
        assert_eq!(encode(&Value::Unsigned(24)), vec![0x18, 0x18]);
        assert_eq!(encode(&Value::Unsigned(256)), vec![0x19, 0x01, 0x00]);
        assert_eq!(encode(&Value::Unsigned(65536)), vec![0x1a, 0x00, 0x01, 0x00, 0x00]);
        assert_eq!(encode(&Value::Negative(0)), vec![0x20]);
    }

    #[test]
    fn rfc_examples_decode() {
        assert_eq!(decode(&[0x18, 0x64]).unwrap(), Value::Unsigned(100));
        assert_eq!(decode(&[0x39, 0x03, 0xe7]).unwrap(), Value::Negative(999));
        assert_eq!(decode(&[0x44, 1, 2, 3, 4]).unwrap(), Value::Bytes(vec![1, 2, 3, 4]));
        assert_eq!(decode(&[0x62, b'I', b'E']).unwrap(), Value::Text("IE".into()));
        assert_eq!(decode(&[0xf5]).unwrap(), Value::Simple(21));
        assert_eq!(decode(&[0xf4]).unwrap(), Value::Simple(20));
        assert_eq!(decode(&[0xf6]).unwrap(), Value::Simple(22));
    }

    #[test]
    fn floats_decode_by_width() {
        assert_eq!(decode(&[0xf9, 0x3c, 0x00]).unwrap(), Value::Float(1.0));
        assert_eq!(decode(&[0xf9, 0xc4, 0x00]).unwrap(), Value::Float(-4.0));
        assert_eq!(decode(&[0xf9, 0x7c, 0x00]).unwrap(), Value::Float(f64::INFINITY));
        assert_eq!(decode(&[0xfb, 0x3f, 0xf1, 0x99, 0x99, 0x99, 0x99, 0x99, 0x9a]).unwrap(), Value::Float(1.1));
        match decode(&[0xf9, 0x7e, 0x00]).unwrap() {
            Value::Float(f) => assert!(f.is_nan()),
            other => panic!("expected a float, got {other:?}"),
        }
    }

    #[test]
    fn map_and_array_round_trip() {
        let value = Value::Map(vec![
            (Value::Text("fmt".into()), Value::Text("none".into())),
            (
                Value::Unsigned(2),
                Value::Array(vec![Value::Bytes(vec![0xde, 0xad]), Value::Negative(7), Value::Simple(22)]),
            ),
            (Value::Unsigned(3), Value::Tag(1, Box::new(Value::Unsigned(1_700_000_000)))),
        ]);
        let bytes = encode(&value);
        assert_eq!(decode(&bytes).unwrap(), value);
    }

    #[test]
    fn map_keeps_encoded_order() {
        let bytes = [0xa2, 0x61, b'b', 0x01, 0x61, b'a', 0x02];
        match decode(&bytes).unwrap() {
            Value::Map(pairs) => {
                assert_eq!(pairs[0].0, Value::Text("b".into()));
                assert_eq!(pairs[1].0, Value::Text("a".into()));
            }
            other => panic!("expected a map, got {other:?}"),
        }
    }

    #[test]
    fn refuses_truncated_and_trailing_input() {
        assert_eq!(decode(&[0x44, 1, 2]), Err(CborError::Truncated));
        assert_eq!(decode(&[0x18]), Err(CborError::Truncated));
        assert_eq!(decode(&[0x01, 0x02]), Err(CborError::Trailing));
        assert_eq!(decode(&[]), Err(CborError::Truncated));
    }

    #[test]
    fn refuses_indefinite_length_and_bad_utf8() {
        assert_eq!(decode(&[0x5f, 0xff]), Err(CborError::Unsupported));
        assert_eq!(decode(&[0x1c]), Err(CborError::Unsupported));
        assert_eq!(decode(&[0x62, 0xff, 0xfe]), Err(CborError::BadUtf8));
    }

    #[test]
    fn refuses_deep_nesting() {
        let mut bytes = vec![0x81; MAX_DEPTH + 2];
        bytes.push(0x00);
        assert_eq!(decode(&bytes), Err(CborError::TooDeep));
        let mut ok = vec![0x81; MAX_DEPTH];
        ok.push(0x00);
        assert!(decode(&ok).is_ok());
    }
}
