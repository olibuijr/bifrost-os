//! Byte and bit readers shared by the container parsers.

use super::DemuxError;

type R<T> = Result<T, DemuxError>;

const TRUNC: DemuxError = DemuxError::Invalid("truncated data");

/// Big-endian reader over a byte slice. Every read is bounds-checked.
pub struct ByteReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> ByteReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        ByteReader { data, pos: 0 }
    }

    pub fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    pub fn take(&mut self, n: usize) -> R<&'a [u8]> {
        if n > self.remaining() {
            return Err(TRUNC);
        }
        let s = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    pub fn skip(&mut self, n: usize) -> R<()> {
        self.take(n).map(|_| ())
    }

    pub fn u8(&mut self) -> R<u8> {
        Ok(self.take(1)?[0])
    }

    pub fn u16(&mut self) -> R<u16> {
        let b = self.take(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }

    pub fn u24(&mut self) -> R<u32> {
        let b = self.take(3)?;
        Ok(u32::from_be_bytes([0, b[0], b[1], b[2]]))
    }

    pub fn u32(&mut self) -> R<u32> {
        let b = self.take(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn i32(&mut self) -> R<i32> {
        Ok(self.u32()? as i32)
    }

    pub fn u64(&mut self) -> R<u64> {
        let b = self.take(8)?;
        let mut a = [0u8; 8];
        a.copy_from_slice(b);
        Ok(u64::from_be_bytes(a))
    }

    pub fn rest(&mut self) -> &'a [u8] {
        let s = &self.data[self.pos..];
        self.pos = self.data.len();
        s
    }
}

/// MSB-first bit reader (AudioSpecificConfig, Exp-Golomb codes).
pub struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> BitReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        BitReader { data, pos: 0 }
    }

    pub fn bits_left(&self) -> usize {
        self.data.len() * 8 - self.pos
    }

    pub fn bit(&mut self) -> R<bool> {
        if self.bits_left() == 0 {
            return Err(DemuxError::Invalid("truncated bitstream"));
        }
        let b = (self.data[self.pos / 8] >> (7 - self.pos % 8)) & 1;
        self.pos += 1;
        Ok(b == 1)
    }

    /// Read `n` (0..=32) bits.
    pub fn bits(&mut self, n: u32) -> R<u32> {
        if n > 32 {
            return Err(DemuxError::Invalid("bit count"));
        }
        let mut v: u64 = 0;
        for _ in 0..n {
            v = (v << 1) | self.bit()? as u64;
        }
        Ok(v as u32)
    }

    pub fn skip(&mut self, n: usize) -> R<()> {
        if n > self.bits_left() {
            return Err(DemuxError::Invalid("truncated bitstream"));
        }
        self.pos += n;
        Ok(())
    }

    /// Unsigned Exp-Golomb.
    pub fn ue(&mut self) -> R<u32> {
        let mut zeros = 0u32;
        while !self.bit()? {
            zeros += 1;
            if zeros > 32 {
                return Err(DemuxError::Invalid("exp-golomb code"));
            }
        }
        if zeros == 0 {
            return Ok(0);
        }
        let rest = self.bits(zeros)? as u64;
        Ok((((1u64 << zeros) - 1) + rest) as u32)
    }

    /// Signed Exp-Golomb.
    pub fn se(&mut self) -> R<i32> {
        let k = self.ue()? as i64;
        let v = (k + 1) / 2;
        Ok(if k % 2 == 1 { v as i32 } else { -(v as i32) })
    }

    pub fn byte_align(&mut self) {
        self.pos = (self.pos + 7) & !7;
        if self.pos > self.data.len() * 8 {
            self.pos = self.data.len() * 8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_reader_reads_big_endian() {
        let d = [1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];
        let mut r = ByteReader::new(&d);
        assert_eq!(r.u8().unwrap(), 1);
        assert_eq!(r.u16().unwrap(), 0x0203);
        assert_eq!(r.u24().unwrap(), 0x040506);
        assert_eq!(r.u32().unwrap(), 0x0708090a);
        assert_eq!(r.remaining(), 5);
        assert_eq!(r.pos(), 10);
        assert_eq!(r.rest(), &[11, 12, 13, 14, 15]);
        assert_eq!(r.remaining(), 0);
    }

    #[test]
    fn byte_reader_u64_and_i32() {
        let d = [0, 0, 0, 1, 0, 0, 0, 2, 0xff, 0xff, 0xff, 0xfe];
        let mut r = ByteReader::new(&d);
        assert_eq!(r.u64().unwrap(), 0x1_0000_0002);
        assert_eq!(r.i32().unwrap(), -2);
    }

    #[test]
    fn byte_reader_truncation_is_an_error() {
        let mut r = ByteReader::new(&[1, 2]);
        assert!(r.u32().is_err());
        assert_eq!(r.remaining(), 2, "failed read must not consume");
        assert!(r.skip(3).is_err());
        assert!(r.take(2).is_ok());
        assert!(r.u8().is_err());
    }

    #[test]
    fn bit_reader_reads_msb_first() {
        let mut b = BitReader::new(&[0b1010_0101, 0xff]);
        assert!(b.bit().unwrap());
        assert_eq!(b.bits(3).unwrap(), 0b010);
        assert_eq!(b.bits(4).unwrap(), 0b0101);
        assert_eq!(b.bits(8).unwrap(), 0xff);
        assert!(b.bit().is_err());
    }

    #[test]
    fn bit_reader_32_bits_and_overflow() {
        let mut b = BitReader::new(&[0xde, 0xad, 0xbe, 0xef, 0x01]);
        assert_eq!(b.bits(32).unwrap(), 0xdeadbeef);
        assert!(b.bits(33).is_err());
        assert_eq!(b.bits_left(), 8);
    }

    #[test]
    fn exp_golomb_codes() {
        // ue: 1 -> 0, 010 -> 1, 011 -> 2, 00100 -> 3, 00111 -> 6
        // bits: 1 010 011 00100 00111 -> 1010 0110 0100 0011 1000 0000
        let mut b = BitReader::new(&[0b1010_0110, 0b0100_0011, 0b1000_0000]);
        assert_eq!(b.ue().unwrap(), 0);
        assert_eq!(b.ue().unwrap(), 1);
        assert_eq!(b.ue().unwrap(), 2);
        assert_eq!(b.ue().unwrap(), 3);
        assert_eq!(b.ue().unwrap(), 6);
    }

    #[test]
    fn signed_exp_golomb_codes() {
        // codeNum 1 -> +1, 2 -> -1, 3 -> +2, 4 -> -2
        // bits: 010 011 00100 00101 -> 0100 1100 1000 0101
        let mut b = BitReader::new(&[0b0100_1100, 0b1000_0101]);
        assert_eq!(b.se().unwrap(), 1);
        assert_eq!(b.se().unwrap(), -1);
        assert_eq!(b.se().unwrap(), 2);
        assert_eq!(b.se().unwrap(), -2);
    }

    #[test]
    fn exp_golomb_rejects_runaway_zeros() {
        let mut b = BitReader::new(&[0, 0, 0, 0, 0, 0]);
        assert!(b.ue().is_err());
    }

    #[test]
    fn skip_and_align() {
        let mut b = BitReader::new(&[0x0f, 0xa0]);
        b.skip(3).unwrap();
        b.byte_align();
        assert_eq!(b.bits(4).unwrap(), 0xa);
        assert!(b.skip(100).is_err());
    }
}
