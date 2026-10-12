//! One little-endian reader and writer for the BifrOSt wire formats (app protocol services, chrome models,
//! layer transactions). Strings and byte runs are a `u32` length then the bytes. The reader never allocates more
//! than the input holds: `count` rejects element counts the remaining bytes cannot back.

/// Why a message did not decode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    /// the input ended inside a field
    Truncated,
    /// a field holds a value the format does not allow (a bool other than 0 or 1, invalid UTF-8)
    BadValue,
    /// a length or count over the caller's limit
    TooLong,
    /// a message tag nobody knows
    UnknownTag(u32),
}

/// Appends fields to a buffer.
#[derive(Debug, Default, Clone)]
pub struct Writer {
    pub buf: Vec<u8>,
}

impl Writer {
    pub fn new() -> Self {
        Writer { buf: Vec::new() }
    }
    pub fn u8(&mut self, v: u8) -> &mut Self {
        self.buf.push(v);
        self
    }
    pub fn u16(&mut self, v: u16) -> &mut Self {
        self.buf.extend_from_slice(&v.to_le_bytes());
        self
    }
    pub fn u32(&mut self, v: u32) -> &mut Self {
        self.buf.extend_from_slice(&v.to_le_bytes());
        self
    }
    pub fn u64(&mut self, v: u64) -> &mut Self {
        self.buf.extend_from_slice(&v.to_le_bytes());
        self
    }
    pub fn i32(&mut self, v: i32) -> &mut Self {
        self.buf.extend_from_slice(&v.to_le_bytes());
        self
    }
    pub fn i64(&mut self, v: i64) -> &mut Self {
        self.buf.extend_from_slice(&v.to_le_bytes());
        self
    }
    pub fn f32(&mut self, v: f32) -> &mut Self {
        self.u32(v.to_bits())
    }
    pub fn f64(&mut self, v: f64) -> &mut Self {
        self.u64(v.to_bits())
    }
    pub fn bool(&mut self, v: bool) -> &mut Self {
        self.u8(v as u8)
    }
    /// A `u32` length, then the bytes.
    pub fn bytes(&mut self, v: &[u8]) -> &mut Self {
        self.u32(v.len() as u32);
        self.buf.extend_from_slice(v);
        self
    }
    /// Same layout as `bytes`.
    pub fn str(&mut self, v: &str) -> &mut Self {
        self.bytes(v.as_bytes())
    }
    pub fn into_inner(self) -> Vec<u8> {
        self.buf
    }
}

/// Reads fields from a slice.
#[derive(Debug, Clone)]
pub struct Reader<'a> {
    b: &'a [u8],
}

impl<'a> Reader<'a> {
    pub fn new(b: &'a [u8]) -> Self {
        Reader { b }
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        if self.b.len() < n {
            return Err(DecodeError::Truncated);
        }
        let (a, rest) = self.b.split_at(n);
        self.b = rest;
        Ok(a)
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N], DecodeError> {
        let mut a = [0u8; N];
        a.copy_from_slice(self.take(N)?);
        Ok(a)
    }
    pub fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.take(1)?[0])
    }
    pub fn u16(&mut self) -> Result<u16, DecodeError> {
        Ok(u16::from_le_bytes(self.array()?))
    }
    pub fn u32(&mut self) -> Result<u32, DecodeError> {
        Ok(u32::from_le_bytes(self.array()?))
    }
    pub fn u64(&mut self) -> Result<u64, DecodeError> {
        Ok(u64::from_le_bytes(self.array()?))
    }
    pub fn i32(&mut self) -> Result<i32, DecodeError> {
        Ok(i32::from_le_bytes(self.array()?))
    }
    pub fn i64(&mut self) -> Result<i64, DecodeError> {
        Ok(i64::from_le_bytes(self.array()?))
    }
    pub fn f32(&mut self) -> Result<f32, DecodeError> {
        Ok(f32::from_bits(self.u32()?))
    }
    pub fn f64(&mut self) -> Result<f64, DecodeError> {
        Ok(f64::from_bits(self.u64()?))
    }
    /// 0 or 1; anything else is `BadValue`.
    pub fn bool(&mut self) -> Result<bool, DecodeError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(DecodeError::BadValue),
        }
    }
    /// A `u32` length, then that many bytes borrowed from the input (never copied or allocated).
    pub fn bytes(&mut self) -> Result<&'a [u8], DecodeError> {
        let n = self.u32()? as usize;
        self.take(n)
    }
    /// Like `bytes`, UTF-8 checked.
    pub fn str(&mut self) -> Result<&'a str, DecodeError> {
        std::str::from_utf8(self.bytes()?).map_err(|_| DecodeError::BadValue)
    }
    /// A `u32` element count: refused above `max`, and when `count * min_elem` exceeds the bytes left (so
    /// `Vec::with_capacity(count)` cannot reserve more than the input could fill). `min_elem` is at least 1.
    pub fn count(&mut self, max: usize, min_elem: usize) -> Result<usize, DecodeError> {
        let n = self.u32()? as usize;
        if n > max {
            return Err(DecodeError::TooLong);
        }
        match n.checked_mul(min_elem.max(1)) {
            Some(need) if need <= self.b.len() => Ok(n),
            _ => Err(DecodeError::Truncated),
        }
    }
    pub fn remaining(&self) -> usize {
        self.b.len()
    }
    /// Ok when every byte was consumed.
    pub fn done(&self) -> Result<(), DecodeError> {
        if self.b.is_empty() {
            Ok(())
        } else {
            Err(DecodeError::BadValue)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_type_round_trips() {
        let mut w = Writer::new();
        w.u8(1).u16(2).u32(3).u64(4).i32(-5).i64(-6).f32(1.5).f64(-2.5).bool(true).bytes(&[9, 8]).str("é");
        let b = w.into_inner();
        let mut r = Reader::new(&b);
        assert_eq!((r.u8(), r.u16(), r.u32(), r.u64()), (Ok(1), Ok(2), Ok(3), Ok(4)));
        assert_eq!((r.i32(), r.i64(), r.f32(), r.f64(), r.bool()), (Ok(-5), Ok(-6), Ok(1.5), Ok(-2.5), Ok(true)));
        assert_eq!(r.bytes(), Ok(&[9u8, 8][..]));
        assert_eq!(r.str(), Ok("é"));
        assert_eq!(r.done(), Ok(()));
    }

    #[test]
    fn exact_layout_is_little_endian() {
        let mut w = Writer::new();
        w.u16(0x0102).u32(1).str("ab");
        assert_eq!(w.buf, [2, 1, 1, 0, 0, 0, 2, 0, 0, 0, b'a', b'b']);
    }

    #[test]
    fn short_input_and_bad_values_are_errors() {
        assert_eq!(Reader::new(&[1, 2]).u32(), Err(DecodeError::Truncated));
        assert_eq!(Reader::new(&[2]).bool(), Err(DecodeError::BadValue));
        assert_eq!(Reader::new(&[2, 0, 0, 0, 0xff, 0xfe]).str(), Err(DecodeError::BadValue));
        assert_eq!(Reader::new(&[9, 0, 0, 0, 1]).bytes(), Err(DecodeError::Truncated));
        assert_eq!(Reader::new(&[1]).done(), Err(DecodeError::BadValue));
    }

    #[test]
    fn count_is_bounded_by_the_input() {
        let mut w = Writer::new();
        w.u32(1000);
        w.buf.extend_from_slice(&[0; 100]);
        assert_eq!(Reader::new(&w.buf).count(10_000, 4), Err(DecodeError::Truncated));
        assert_eq!(Reader::new(&w.buf).count(10, 1), Err(DecodeError::TooLong));
        assert_eq!(Reader::new(&w.buf).count(10_000, 0), Err(DecodeError::Truncated));
        let mut ok = Writer::new();
        ok.u32(25);
        ok.buf.extend_from_slice(&[0; 100]);
        assert_eq!(Reader::new(&ok.buf).count(100, 4), Ok(25));
        assert_eq!(Reader::new(&[255, 255, 255, 255]).count(usize::MAX, usize::MAX), Err(DecodeError::Truncated));
    }

    #[test]
    fn str_borrows_from_the_input() {
        let mut w = Writer::new();
        w.str("hello");
        let b = w.into_inner();
        let s = Reader::new(&b).str().unwrap();
        let (lo, hi) = (b.as_ptr() as usize, b.as_ptr() as usize + b.len());
        assert!((s.as_ptr() as usize) >= lo && (s.as_ptr() as usize + s.len()) <= hi, "no copy beyond the input");
    }
}
