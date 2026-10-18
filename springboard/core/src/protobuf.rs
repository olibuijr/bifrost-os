//! Protocol buffer wire format: a std-only writer, a bounded reader and a schema-less field extractor.
//!
//! The wire format is a run of fields. Each field starts with a varint key, `(number << 3) | wire type`, followed
//! by a payload that depends on the wire type: a varint (0), eight fixed bytes (1), a length-delimited run of bytes
//! (2) or four fixed bytes (5). Groups (types 3 and 4) are the obsolete form and are refused. Messages, strings,
//! bytes and packed repeated numbers are all wire type 2, so the format itself cannot say which one a payload is;
//! the caller picks the reading with [`Message::string`], [`Message::message`] or [`Message::packed_varints`], and
//! [`describe`] guesses when no schema is known (the way a raw decode tool prints an unknown message).
//!
//! Reading is bounded: a message is at most [`MAX_MESSAGE_LEN`] bytes, nesting stops at [`MAX_DEPTH`], a varint is
//! at most ten bytes, field numbers run 1 to 2^29-1, and a length that runs past the input is refused before any
//! allocation. Writing never fails: the writer takes the numbers it is given and the caller owns the schema.

use std::fmt;

/// Largest message the reader accepts, in bytes.
pub const MAX_MESSAGE_LEN: usize = 16 * 1024 * 1024;
/// Deepest message nesting that [`Message::message`] and [`describe`] follow.
pub const MAX_DEPTH: usize = 32;
/// Largest field number the format allows.
pub const MAX_FIELD_NUMBER: u32 = (1 << 29) - 1;

/// Why a buffer was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProtoError {
    /// The input ended in the middle of a key, a value or a length-delimited run.
    Truncated,
    /// A varint is longer than ten bytes or sets bits past the 64th.
    BadVarint,
    /// A key carries wire type 3, 4, 6 or 7, or field number 0 or one past [`MAX_FIELD_NUMBER`].
    BadKey,
    /// The input is longer than [`MAX_MESSAGE_LEN`].
    TooLong,
    /// Nested messages are deeper than [`MAX_DEPTH`].
    TooDeep,
    /// A string field is not valid UTF-8.
    BadUtf8,
    /// A field was read as another type than the one it holds.
    WrongType,
}

impl fmt::Display for ProtoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ProtoError::Truncated => "protocol buffer data is truncated",
            ProtoError::BadVarint => "protocol buffer varint is malformed",
            ProtoError::BadKey => "protocol buffer field key is not valid",
            ProtoError::TooLong => "protocol buffer message is too long",
            ProtoError::TooDeep => "protocol buffer message nests too deeply",
            ProtoError::BadUtf8 => "protocol buffer string is not UTF-8",
            ProtoError::WrongType => "protocol buffer field holds another type",
        })
    }
}

impl std::error::Error for ProtoError {}

/// Appends `value` as a base-128 varint, low group first.
pub fn put_varint(out: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        out.push((value as u8) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

/// Reads one varint from the front of `input`; returns the value and the bytes it used.
pub fn get_varint(input: &[u8]) -> Result<(u64, usize), ProtoError> {
    let mut value = 0u64;
    for (i, &byte) in input.iter().enumerate() {
        if i == 10 {
            return Err(ProtoError::BadVarint);
        }
        let bits = (byte & 0x7f) as u64;
        // The tenth byte may only carry the 64th bit.
        if i == 9 && bits > 1 {
            return Err(ProtoError::BadVarint);
        }
        value |= bits << (7 * i as u32);
        if byte & 0x80 == 0 {
            return Ok((value, i + 1));
        }
    }
    Err(ProtoError::Truncated)
}

/// Zigzag-maps a signed value so small magnitudes stay short (`sint32` and `sint64`).
pub fn zigzag(value: i64) -> u64 {
    ((value << 1) ^ (value >> 63)) as u64
}

/// Undoes [`zigzag`].
pub fn unzigzag(value: u64) -> i64 {
    ((value >> 1) as i64) ^ -((value & 1) as i64)
}

/// The payload shape of a field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WireType {
    Varint,
    Fixed64,
    Len,
    Fixed32,
}

impl WireType {
    fn bits(self) -> u64 {
        match self {
            WireType::Varint => 0,
            WireType::Fixed64 => 1,
            WireType::Len => 2,
            WireType::Fixed32 => 5,
        }
    }

    fn from_bits(bits: u64) -> Result<Self, ProtoError> {
        match bits {
            0 => Ok(WireType::Varint),
            1 => Ok(WireType::Fixed64),
            2 => Ok(WireType::Len),
            5 => Ok(WireType::Fixed32),
            _ => Err(ProtoError::BadKey),
        }
    }
}

/// Builds a message field by field.
#[derive(Clone, Debug, Default)]
pub struct Writer {
    buf: Vec<u8>,
}

impl Writer {
    pub fn new() -> Self {
        Self::default()
    }

    fn key(&mut self, number: u32, wire: WireType) -> &mut Self {
        put_varint(&mut self.buf, ((number as u64) << 3) | wire.bits());
        self
    }

    /// `uint32`, `uint64`, `int32`, `int64` (as the two's complement bit pattern), `bool` and enum fields.
    pub fn varint(&mut self, number: u32, value: u64) -> &mut Self {
        self.key(number, WireType::Varint);
        put_varint(&mut self.buf, value);
        self
    }

    pub fn bool(&mut self, number: u32, value: bool) -> &mut Self {
        self.varint(number, value as u64)
    }

    /// `sint32` and `sint64` fields.
    pub fn sint(&mut self, number: u32, value: i64) -> &mut Self {
        self.varint(number, zigzag(value))
    }

    pub fn fixed32(&mut self, number: u32, value: u32) -> &mut Self {
        self.key(number, WireType::Fixed32);
        self.buf.extend_from_slice(&value.to_le_bytes());
        self
    }

    pub fn fixed64(&mut self, number: u32, value: u64) -> &mut Self {
        self.key(number, WireType::Fixed64);
        self.buf.extend_from_slice(&value.to_le_bytes());
        self
    }

    pub fn float(&mut self, number: u32, value: f32) -> &mut Self {
        self.fixed32(number, value.to_bits())
    }

    pub fn double(&mut self, number: u32, value: f64) -> &mut Self {
        self.fixed64(number, value.to_bits())
    }

    /// `bytes` fields, and the framing of strings and nested messages.
    pub fn bytes(&mut self, number: u32, value: &[u8]) -> &mut Self {
        self.key(number, WireType::Len);
        put_varint(&mut self.buf, value.len() as u64);
        self.buf.extend_from_slice(value);
        self
    }

    pub fn string(&mut self, number: u32, value: &str) -> &mut Self {
        self.bytes(number, value.as_bytes())
    }

    /// A nested message field.
    pub fn message(&mut self, number: u32, inner: &Writer) -> &mut Self {
        self.bytes(number, &inner.buf)
    }

    /// A packed repeated varint field (one length-delimited run of varints).
    pub fn packed_varints(&mut self, number: u32, values: &[u64]) -> &mut Self {
        let mut run = Vec::new();
        for &v in values {
            put_varint(&mut run, v);
        }
        self.bytes(number, &run)
    }

    pub fn finish(self) -> Vec<u8> {
        self.buf
    }
}

/// The payload of one parsed field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Value<'a> {
    Varint(u64),
    Fixed64(u64),
    Fixed32(u32),
    Bytes(&'a [u8]),
}

/// One parsed field: its number and payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Field<'a> {
    pub number: u32,
    pub value: Value<'a>,
}

/// A parsed message: its fields in wire order. Repeated fields and unknown fields are all kept, so a caller reads
/// what it knows and the rest is simply not asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message<'a> {
    fields: Vec<Field<'a>>,
}

impl<'a> Message<'a> {
    /// Parses a whole buffer as one message.
    pub fn parse(input: &'a [u8]) -> Result<Self, ProtoError> {
        if input.len() > MAX_MESSAGE_LEN {
            return Err(ProtoError::TooLong);
        }
        let mut fields = Vec::new();
        let mut at = 0;
        while at < input.len() {
            let (key, used) = get_varint(&input[at..])?;
            at += used;
            let number = key >> 3;
            if number == 0 || number > MAX_FIELD_NUMBER as u64 {
                return Err(ProtoError::BadKey);
            }
            let value = match WireType::from_bits(key & 7)? {
                WireType::Varint => {
                    let (v, used) = get_varint(&input[at..])?;
                    at += used;
                    Value::Varint(v)
                }
                WireType::Fixed64 => {
                    let end = at.checked_add(8).filter(|&e| e <= input.len()).ok_or(ProtoError::Truncated)?;
                    let v = u64::from_le_bytes(input[at..end].try_into().unwrap());
                    at = end;
                    Value::Fixed64(v)
                }
                WireType::Fixed32 => {
                    let end = at.checked_add(4).filter(|&e| e <= input.len()).ok_or(ProtoError::Truncated)?;
                    let v = u32::from_le_bytes(input[at..end].try_into().unwrap());
                    at = end;
                    Value::Fixed32(v)
                }
                WireType::Len => {
                    let (len, used) = get_varint(&input[at..])?;
                    at += used;
                    let len = usize::try_from(len).map_err(|_| ProtoError::Truncated)?;
                    let end = at.checked_add(len).filter(|&e| e <= input.len()).ok_or(ProtoError::Truncated)?;
                    let bytes = &input[at..end];
                    at = end;
                    Value::Bytes(bytes)
                }
            };
            fields.push(Field { number: number as u32, value });
        }
        Ok(Self { fields })
    }

    pub fn fields(&self) -> &[Field<'a>] {
        &self.fields
    }

    /// Every value of field `number`, in wire order (a repeated field has several).
    pub fn all(&self, number: u32) -> impl Iterator<Item = Value<'a>> + '_ {
        self.fields.iter().filter(move |f| f.number == number).map(|f| f.value)
    }

    /// The last value of field `number`: when a singular field appears twice, the later one wins.
    pub fn last(&self, number: u32) -> Option<Value<'a>> {
        self.all(number).last()
    }

    /// A varint field; `None` when absent, an error when the field holds another wire type.
    pub fn varint(&self, number: u32) -> Result<Option<u64>, ProtoError> {
        match self.last(number) {
            None => Ok(None),
            Some(Value::Varint(v)) => Ok(Some(v)),
            Some(_) => Err(ProtoError::WrongType),
        }
    }

    pub fn sint(&self, number: u32) -> Result<Option<i64>, ProtoError> {
        Ok(self.varint(number)?.map(unzigzag))
    }

    pub fn bool(&self, number: u32) -> Result<Option<bool>, ProtoError> {
        Ok(self.varint(number)?.map(|v| v != 0))
    }

    pub fn double(&self, number: u32) -> Result<Option<f64>, ProtoError> {
        match self.last(number) {
            None => Ok(None),
            Some(Value::Fixed64(v)) => Ok(Some(f64::from_bits(v))),
            Some(_) => Err(ProtoError::WrongType),
        }
    }

    pub fn float(&self, number: u32) -> Result<Option<f32>, ProtoError> {
        match self.last(number) {
            None => Ok(None),
            Some(Value::Fixed32(v)) => Ok(Some(f32::from_bits(v))),
            Some(_) => Err(ProtoError::WrongType),
        }
    }

    pub fn bytes(&self, number: u32) -> Result<Option<&'a [u8]>, ProtoError> {
        match self.last(number) {
            None => Ok(None),
            Some(Value::Bytes(b)) => Ok(Some(b)),
            Some(_) => Err(ProtoError::WrongType),
        }
    }

    pub fn string(&self, number: u32) -> Result<Option<&'a str>, ProtoError> {
        match self.bytes(number)? {
            None => Ok(None),
            Some(b) => std::str::from_utf8(b).map(Some).map_err(|_| ProtoError::BadUtf8),
        }
    }

    /// A nested message field. `depth` counts the messages already entered; start with 0.
    pub fn message(&self, number: u32, depth: usize) -> Result<Option<Message<'a>>, ProtoError> {
        if depth >= MAX_DEPTH {
            return Err(ProtoError::TooDeep);
        }
        match self.bytes(number)? {
            None => Ok(None),
            Some(b) => Message::parse(b).map(Some),
        }
    }

    /// A packed repeated varint field, concatenated with any unpacked entries of the same number.
    pub fn packed_varints(&self, number: u32) -> Result<Vec<u64>, ProtoError> {
        let mut out = Vec::new();
        for value in self.all(number) {
            match value {
                Value::Varint(v) => out.push(v),
                Value::Bytes(mut run) => {
                    while !run.is_empty() {
                        let (v, used) = get_varint(run)?;
                        out.push(v);
                        run = &run[used..];
                    }
                }
                _ => return Err(ProtoError::WrongType),
            }
        }
        Ok(out)
    }
}

/// Prints an unknown message as indented `number: value` lines, one per field. A length-delimited payload is shown
/// as a nested message when it parses completely as one and is not printable text, as text when it is printable
/// UTF-8, and as hex otherwise. Nesting past [`MAX_DEPTH`] is printed as hex rather than followed.
pub fn describe(input: &[u8]) -> Result<String, ProtoError> {
    let mut out = String::new();
    describe_into(&mut out, input, 0)?;
    Ok(out)
}

fn describe_into(out: &mut String, input: &[u8], depth: usize) -> Result<(), ProtoError> {
    let message = Message::parse(input)?;
    let pad = "  ".repeat(depth);
    for field in message.fields() {
        let n = field.number;
        match field.value {
            Value::Varint(v) => out.push_str(&format!("{pad}{n}: {v}\n")),
            Value::Fixed64(v) => out.push_str(&format!("{pad}{n}: 0x{v:016x}\n")),
            Value::Fixed32(v) => out.push_str(&format!("{pad}{n}: 0x{v:08x}\n")),
            Value::Bytes(b) => {
                if let Some(text) = printable(b) {
                    out.push_str(&format!("{pad}{n}: \"{text}\"\n"));
                } else if !b.is_empty() && depth + 1 < MAX_DEPTH && Message::parse(b).is_ok() {
                    out.push_str(&format!("{pad}{n} {{\n"));
                    describe_into(out, b, depth + 1)?;
                    out.push_str(&format!("{pad}}}\n"));
                } else {
                    let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
                    out.push_str(&format!("{pad}{n}: <{hex}>\n"));
                }
            }
        }
    }
    Ok(())
}

fn printable(bytes: &[u8]) -> Option<&str> {
    let text = std::str::from_utf8(bytes).ok()?;
    if text.is_empty() || text.chars().any(|c| c.is_control() || c == '"' || c == '\\') {
        return None;
    }
    Some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varint_round_trips_and_has_the_documented_bytes() {
        let mut out = Vec::new();
        put_varint(&mut out, 300);
        assert_eq!(out, [0xac, 0x02]);
        assert_eq!(get_varint(&out), Ok((300, 2)));
        for v in [0, 1, 127, 128, 16_383, 16_384, u32::MAX as u64, u64::MAX] {
            let mut b = Vec::new();
            put_varint(&mut b, v);
            assert_eq!(get_varint(&b), Ok((v, b.len())));
        }
    }

    #[test]
    fn varint_refuses_truncated_and_overlong_input() {
        assert_eq!(get_varint(&[0x80]), Err(ProtoError::Truncated));
        assert_eq!(get_varint(&[]), Err(ProtoError::Truncated));
        assert_eq!(get_varint(&[0xff; 11]), Err(ProtoError::BadVarint));
        let mut too_big = vec![0xff; 9];
        too_big.push(0x02);
        assert_eq!(get_varint(&too_big), Err(ProtoError::BadVarint));
    }

    #[test]
    fn zigzag_keeps_small_magnitudes_short() {
        assert_eq!([zigzag(0), zigzag(-1), zigzag(1), zigzag(-2)], [0, 1, 2, 3]);
        for v in [0, -1, 1, i64::MIN, i64::MAX, -123_456] {
            assert_eq!(unzigzag(zigzag(v)), v);
        }
    }

    #[test]
    fn writer_matches_a_known_encoding() {
        // Field 1 = 150 is the textbook example: 08 96 01.
        let mut w = Writer::new();
        w.varint(1, 150);
        assert_eq!(w.finish(), [0x08, 0x96, 0x01]);
        // Field 2 = "testing": 12 07 74 65 73 74 69 6e 67.
        let mut w = Writer::new();
        w.string(2, "testing");
        assert_eq!(w.finish(), [0x12, 0x07, b't', b'e', b's', b't', b'i', b'n', b'g']);
    }

    #[test]
    fn extractor_reads_every_wire_type() {
        let mut inner = Writer::new();
        inner.string(1, "inner").sint(2, -7);
        let mut w = Writer::new();
        w.varint(1, 42)
            .bool(2, true)
            .sint(3, -300)
            .fixed32(4, 0xdead_beef)
            .fixed64(5, 0x0102_0304_0506_0708)
            .double(6, 2.5)
            .float(7, -1.5)
            .string(8, "hello")
            .message(9, &inner)
            .packed_varints(10, &[1, 300, 70_000]);
        let bytes = w.finish();
        let m = Message::parse(&bytes).unwrap();
        assert_eq!(m.varint(1), Ok(Some(42)));
        assert_eq!(m.bool(2), Ok(Some(true)));
        assert_eq!(m.sint(3), Ok(Some(-300)));
        assert_eq!(m.last(4), Some(Value::Fixed32(0xdead_beef)));
        assert_eq!(m.last(5), Some(Value::Fixed64(0x0102_0304_0506_0708)));
        assert_eq!(m.double(6), Ok(Some(2.5)));
        assert_eq!(m.float(7), Ok(Some(-1.5)));
        assert_eq!(m.string(8), Ok(Some("hello")));
        let nested = m.message(9, 0).unwrap().unwrap();
        assert_eq!(nested.string(1), Ok(Some("inner")));
        assert_eq!(nested.sint(2), Ok(Some(-7)));
        assert_eq!(m.packed_varints(10), Ok(vec![1, 300, 70_000]));
        assert_eq!(m.varint(99), Ok(None));
    }

    #[test]
    fn repeated_and_duplicate_fields_keep_wire_order_and_last_wins() {
        let mut w = Writer::new();
        w.varint(1, 1).string(2, "a").varint(1, 2).string(2, "b").varint(1, 3);
        let bytes = w.finish();
        let m = Message::parse(&bytes).unwrap();
        assert_eq!(m.all(1).collect::<Vec<_>>(), [Value::Varint(1), Value::Varint(2), Value::Varint(3)]);
        assert_eq!(m.string(2), Ok(Some("b")));
        assert_eq!(m.fields().len(), 5);
    }

    #[test]
    fn unpacked_entries_join_the_packed_run() {
        let mut w = Writer::new();
        w.varint(1, 5).packed_varints(1, &[6, 7]).varint(1, 8);
        let bytes = w.finish();
        assert_eq!(Message::parse(&bytes).unwrap().packed_varints(1), Ok(vec![5, 6, 7, 8]));
    }

    #[test]
    fn reading_a_field_as_the_wrong_type_is_an_error() {
        let mut w = Writer::new();
        w.varint(1, 1).string(2, "x");
        let bytes = w.finish();
        let m = Message::parse(&bytes).unwrap();
        assert_eq!(m.string(1), Err(ProtoError::WrongType));
        assert_eq!(m.varint(2), Err(ProtoError::WrongType));
        assert_eq!(m.double(1), Err(ProtoError::WrongType));
    }

    #[test]
    fn invalid_utf8_in_a_string_field_is_refused() {
        let mut w = Writer::new();
        w.bytes(1, &[0xff, 0xfe]);
        let bytes = w.finish();
        let m = Message::parse(&bytes).unwrap();
        assert_eq!(m.string(1), Err(ProtoError::BadUtf8));
        assert_eq!(m.bytes(1), Ok(Some(&[0xff, 0xfe][..])));
    }

    #[test]
    fn hostile_input_is_refused_without_allocating() {
        // Length says 1 GiB, input has two bytes.
        let mut b = vec![0x0a];
        put_varint(&mut b, 1 << 30);
        b.extend_from_slice(&[1, 2]);
        assert_eq!(Message::parse(&b), Err(ProtoError::Truncated));
        // Field number 0, group start (type 3), type 7.
        assert_eq!(Message::parse(&[0x00, 0x01]), Err(ProtoError::BadKey));
        assert_eq!(Message::parse(&[0x0b]), Err(ProtoError::BadKey));
        assert_eq!(Message::parse(&[0x0f]), Err(ProtoError::BadKey));
        // Fixed widths cut short.
        assert_eq!(Message::parse(&[0x0d, 1, 2, 3]), Err(ProtoError::Truncated));
        assert_eq!(Message::parse(&[0x09, 1, 2, 3, 4, 5, 6, 7]), Err(ProtoError::Truncated));
        // A length that overflows usize arithmetic.
        let mut b = vec![0x0a];
        put_varint(&mut b, u64::MAX);
        assert!(Message::parse(&b).is_err());
    }

    #[test]
    fn field_number_range_is_enforced() {
        let mut ok = Writer::new();
        ok.varint(MAX_FIELD_NUMBER, 1);
        assert!(Message::parse(&ok.finish()).is_ok());
        let mut b = Vec::new();
        put_varint(&mut b, (MAX_FIELD_NUMBER as u64 + 1) << 3);
        b.push(1);
        assert_eq!(Message::parse(&b), Err(ProtoError::BadKey));
    }

    #[test]
    fn nesting_depth_is_bounded() {
        let mut bytes = {
            let mut w = Writer::new();
            w.varint(1, 1);
            w.finish()
        };
        for _ in 0..4 {
            let mut outer = Writer::new();
            outer.bytes(1, &bytes);
            bytes = outer.finish();
        }
        let m = Message::parse(&bytes).unwrap();
        assert!(m.message(1, 0).unwrap().is_some());
        assert_eq!(m.message(1, MAX_DEPTH).err(), Some(ProtoError::TooDeep));
    }

    #[test]
    fn describe_prints_an_unknown_message() {
        let mut inner = Writer::new();
        inner.varint(1, 7).string(2, "name");
        let mut w = Writer::new();
        w.varint(1, 150).message(2, &inner).bytes(3, &[0xff, 0x00, 0x80]).fixed32(4, 1);
        let text = describe(&w.finish()).unwrap();
        assert_eq!(text, "1: 150\n2 {\n  1: 7\n  2: \"name\"\n}\n3: <ff0080>\n4: 0x00000001\n");
    }

    #[test]
    fn describe_reports_a_malformed_buffer() {
        assert_eq!(describe(&[0x0a, 0x05, 1]), Err(ProtoError::Truncated));
        assert_eq!(describe(&[]), Ok(String::new()));
    }
}
