//! Resumable DEFLATE (RFC 1951) and zlib (RFC 1950) decoder for the SDK stream and filter API.
//!
//! `Decoder::feed` takes input in pieces of any size and returns the output they complete. The decoder
//! is a state machine over a buffer of unread input. Each step parses one unit (the zlib header, a block
//! header with its code tables, one stored byte run, one symbol with its extra bits, the trailer) from a
//! checkpoint. A unit that runs out of input is rolled back and parsed again when more input arrives, so
//! the decoder never holds half a symbol. The Books engine keeps the whole-buffer decoder in
//! `bifrost_render::cg::pdf::inflate`; this module does not change it.

use bifrost_render::cg::pdf::inflate::Error;

/// Back-reference window (RFC 1951 section 3.2.2).
const WINDOW: usize = 32 * 1024;
const ADLER_MOD: u64 = 65_521;

const LEN_BASE: [u16; 29] = [3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258];
const LEN_EXTRA: [u8; 29] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
const DIST_BASE: [u16; 30] = [1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577];
const DIST_EXTRA: [u8; 30] = [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];
const CL_ORDER: [usize; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];

/// Why a step stopped: it needs more input (and is rolled back), or the data is bad.
enum Stop {
    More,
    Bad(Error),
}

type Step<T> = Result<T, Stop>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    /// The two-byte zlib header.
    Header,
    /// The first three bits of a block, then its codes or its stored length.
    BlockHeader,
    /// `left` bytes of a stored block still to copy.
    Stored { left: usize },
    /// Symbols of a Huffman-coded block, until end of block.
    Codes,
    /// The Adler-32 trailer of a zlib stream.
    Trailer,
    Done,
}

/// Unread input, and a read position in bits (`bit` is 0 to 7 within `buf[pos]`).
#[derive(Debug)]
struct Input {
    buf: Vec<u8>,
    pos: usize,
    bit: u32,
}

impl Input {
    fn bit(&mut self) -> Step<u32> {
        let b = *self.buf.get(self.pos).ok_or(Stop::More)?;
        let v = ((b >> self.bit) & 1) as u32;
        self.bit += 1;
        if self.bit == 8 {
            self.bit = 0;
            self.pos += 1;
        }
        Ok(v)
    }

    /// `count` bits, least significant first.
    fn bits(&mut self, count: u32) -> Step<u32> {
        let mut v = 0;
        for i in 0..count {
            v |= self.bit()? << i;
        }
        Ok(v)
    }

    /// Skips to the next byte boundary. Callers read whole bytes only after this.
    fn align(&mut self) {
        if self.bit != 0 {
            self.bit = 0;
            self.pos += 1;
        }
    }

    /// The next whole byte; the position must be byte aligned.
    fn byte(&mut self) -> Step<u8> {
        let b = *self.buf.get(self.pos).ok_or(Stop::More)?;
        self.pos += 1;
        Ok(b)
    }

    fn checkpoint(&self) -> (usize, u32) {
        (self.pos, self.bit)
    }

    fn rewind(&mut self, at: (usize, u32)) {
        self.pos = at.0;
        self.bit = at.1;
    }

    /// Drops the bytes already read.
    fn drop_consumed(&mut self) {
        self.buf.drain(..self.pos);
        self.pos = 0;
    }
}

/// A canonical Huffman code: how many symbols have each length, and the symbols in code order.
#[derive(Debug)]
struct Huffman {
    count: [u16; 16],
    symbol: Vec<u16>,
}

impl Huffman {
    fn new(lengths: &[u8]) -> Huffman {
        let mut count = [0u16; 16];
        for &l in lengths {
            count[l as usize] += 1;
        }
        let mut offs = [0u16; 16];
        for i in 1..15 {
            offs[i + 1] = offs[i] + count[i];
        }
        let mut symbol = vec![0u16; lengths.len()];
        for (s, &l) in lengths.iter().enumerate() {
            if l != 0 {
                symbol[offs[l as usize] as usize] = s as u16;
                offs[l as usize] += 1;
            }
        }
        Huffman { count, symbol }
    }

    fn decode(&self, input: &mut Input) -> Step<u16> {
        let (mut code, mut first, mut index) = (0i32, 0i32, 0i32);
        for len in 1..16 {
            code |= input.bit()? as i32;
            let count = self.count[len] as i32;
            if code - count < first {
                return self.symbol.get((index + (code - first)) as usize).copied().ok_or(Stop::Bad(Error::BadCode));
            }
            index += count;
            first += count;
            first <<= 1;
            code <<= 1;
        }
        Err(Stop::Bad(Error::BadCode))
    }
}

fn fixed_tables() -> (Huffman, Huffman) {
    let mut l = [0u8; 288];
    for (i, v) in l.iter_mut().enumerate() {
        *v = match i {
            0..=143 => 8,
            144..=255 => 9,
            256..=279 => 7,
            _ => 8,
        };
    }
    (Huffman::new(&l), Huffman::new(&[5u8; 30]))
}

/// Reads the code tables of a dynamic block. Parsed as one unit, so a rollback re-reads all of it.
fn dynamic_tables(input: &mut Input) -> Step<(Huffman, Huffman)> {
    let nlen = input.bits(5)? as usize + 257;
    let ndist = input.bits(5)? as usize + 1;
    let ncode = input.bits(4)? as usize + 4;
    if nlen > 286 || ndist > 30 {
        return Err(Stop::Bad(Error::BadBlock));
    }
    let mut cl = [0u8; 19];
    for &o in CL_ORDER.iter().take(ncode) {
        cl[o] = input.bits(3)? as u8;
    }
    let clh = Huffman::new(&cl);
    let total = nlen + ndist;
    let mut lengths = vec![0u8; total];
    let mut i = 0;
    while i < total {
        let sym = clh.decode(input)?;
        if sym < 16 {
            lengths[i] = sym as u8;
            i += 1;
            continue;
        }
        let (prev, rep) = match sym {
            16 => {
                if i == 0 {
                    return Err(Stop::Bad(Error::BadBlock));
                }
                (lengths[i - 1], 3 + input.bits(2)? as usize)
            }
            17 => (0, 3 + input.bits(3)? as usize),
            _ => (0, 11 + input.bits(7)? as usize),
        };
        if i + rep > total {
            return Err(Stop::Bad(Error::BadBlock));
        }
        for _ in 0..rep {
            lengths[i] = prev;
            i += 1;
        }
    }
    Ok((Huffman::new(&lengths[..nlen]), Huffman::new(&lengths[nlen..])))
}

/// Incremental zlib (or raw DEFLATE) decoder. Feed it input as it arrives; `finish` checks that the
/// stream ended.
#[derive(Debug)]
pub struct Decoder {
    input: Input,
    phase: Phase,
    /// The current block is the last one.
    last: bool,
    zlib: bool,
    lit: Huffman,
    dist: Huffman,
    /// Output kept for back-references. `hist[delivered..]` has not been returned yet.
    hist: Vec<u8>,
    delivered: usize,
    total: usize,
    limit: usize,
    /// Running Adler-32 of the output (zlib trailer check).
    a: u64,
    b: u64,
}

impl Decoder {
    /// A decoder for zlib (`zlib` set) or raw DEFLATE. Output past `limit` bytes fails with `TooBig`.
    pub fn new(zlib: bool, limit: usize) -> Decoder {
        Decoder {
            input: Input { buf: Vec::new(), pos: 0, bit: 0 },
            phase: if zlib { Phase::Header } else { Phase::BlockHeader },
            last: false,
            zlib,
            lit: Huffman::new(&[]),
            dist: Huffman::new(&[]),
            hist: Vec::new(),
            delivered: 0,
            total: 0,
            limit,
            a: 1,
            b: 0,
        }
    }

    /// Adds `data` and returns the output it completes. Input after the end of the stream is ignored.
    pub fn feed(&mut self, data: &[u8]) -> Result<Vec<u8>, Error> {
        if self.phase != Phase::Done {
            self.input.buf.extend_from_slice(data);
            self.run()?;
        }
        Ok(self.take())
    }

    /// Ends the input. Fails with `Truncated` unless the stream ended; otherwise returns the rest of the output.
    pub fn finish(mut self) -> Result<Vec<u8>, Error> {
        if self.phase != Phase::Done {
            return Err(Error::Truncated);
        }
        Ok(self.take())
    }

    fn run(&mut self) -> Result<(), Error> {
        while self.phase != Phase::Done {
            let at = self.input.checkpoint();
            match self.step() {
                Ok(()) => {}
                Err(Stop::More) => {
                    self.input.rewind(at);
                    break;
                }
                Err(Stop::Bad(e)) => return Err(e),
            }
        }
        self.input.drop_consumed();
        Ok(())
    }

    /// One unit of work. Returns `Ok` only after the phase or the input position has moved on.
    fn step(&mut self) -> Step<()> {
        match self.phase {
            Phase::Header => {
                let cmf = self.input.byte()? as u16;
                let flg = self.input.byte()? as u16;
                if (cmf & 0x0f) != 8 || ((cmf << 8) | flg) % 31 != 0 || (flg & 0x20) != 0 {
                    return Err(Stop::Bad(Error::BadBlock));
                }
                self.phase = Phase::BlockHeader;
                Ok(())
            }
            Phase::BlockHeader => {
                let last = self.input.bits(1)? == 1;
                let kind = self.input.bits(2)?;
                match kind {
                    0 => {
                        self.input.align();
                        let len = u16::from_le_bytes([self.input.byte()?, self.input.byte()?]);
                        let nlen = u16::from_le_bytes([self.input.byte()?, self.input.byte()?]);
                        if len != !nlen {
                            return Err(Stop::Bad(Error::BadBlock));
                        }
                        self.last = last;
                        self.phase = Phase::Stored { left: len as usize };
                    }
                    1 => {
                        let (lit, dist) = fixed_tables();
                        self.lit = lit;
                        self.dist = dist;
                        self.last = last;
                        self.phase = Phase::Codes;
                    }
                    2 => {
                        let (lit, dist) = dynamic_tables(&mut self.input)?;
                        self.lit = lit;
                        self.dist = dist;
                        self.last = last;
                        self.phase = Phase::Codes;
                    }
                    _ => return Err(Stop::Bad(Error::BadBlock)),
                }
                Ok(())
            }
            Phase::Stored { left } => {
                if left == 0 {
                    self.phase = self.end_of_block();
                    return Ok(());
                }
                // Copies what is there now; a partial run is progress, not a rollback.
                let avail = self.input.buf.len() - self.input.pos;
                if avail == 0 {
                    return Err(Stop::More);
                }
                let n = left.min(avail);
                let start = self.input.pos;
                for i in start..start + n {
                    let b = self.input.buf[i];
                    self.put(b)?;
                }
                self.input.pos += n;
                self.phase = if left == n { self.end_of_block() } else { Phase::Stored { left: left - n } };
                Ok(())
            }
            Phase::Codes => {
                let sym = self.lit.decode(&mut self.input)? as usize;
                match sym {
                    0..=255 => self.put(sym as u8)?,
                    256 => self.phase = self.end_of_block(),
                    257..=285 => {
                        let i = sym - 257;
                        let len = LEN_BASE[i] as usize + self.input.bits(LEN_EXTRA[i] as u32)? as usize;
                        let ds = self.dist.decode(&mut self.input)? as usize;
                        if ds >= 30 {
                            return Err(Stop::Bad(Error::BadCode));
                        }
                        let d = DIST_BASE[ds] as usize + self.input.bits(DIST_EXTRA[ds] as u32)? as usize;
                        if d > self.hist.len() {
                            return Err(Stop::Bad(Error::BadDistance));
                        }
                        for _ in 0..len {
                            let b = self.hist[self.hist.len() - d];
                            self.put(b)?;
                        }
                    }
                    _ => return Err(Stop::Bad(Error::BadCode)),
                }
                Ok(())
            }
            Phase::Trailer => {
                self.input.align();
                let want = u32::from_be_bytes([self.input.byte()?, self.input.byte()?, self.input.byte()?, self.input.byte()?]);
                let have = ((self.b << 16) | self.a) as u32;
                if want != have {
                    return Err(Stop::Bad(Error::BadBlock));
                }
                self.phase = Phase::Done;
                Ok(())
            }
            Phase::Done => Ok(()),
        }
    }

    fn end_of_block(&self) -> Phase {
        if !self.last {
            Phase::BlockHeader
        } else if self.zlib {
            Phase::Trailer
        } else {
            Phase::Done
        }
    }

    fn put(&mut self, byte: u8) -> Step<()> {
        self.total += 1;
        if self.total > self.limit {
            return Err(Stop::Bad(Error::TooBig));
        }
        self.a = (self.a + byte as u64) % ADLER_MOD;
        self.b = (self.b + self.a) % ADLER_MOD;
        self.hist.push(byte);
        Ok(())
    }

    /// The output not yet returned. Keeps the last window of output for back-references.
    fn take(&mut self) -> Vec<u8> {
        let out = self.hist[self.delivered..].to_vec();
        self.delivered = self.hist.len();
        let drop = self.delivered.saturating_sub(WINDOW);
        if drop >= WINDOW {
            self.hist.drain(..drop);
            self.delivered -= drop;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::super::dynamic::{encode, Level};
    use super::*;

    fn deflate(data: &[u8]) -> Vec<u8> {
        encode(data, Level::Default)
    }

    fn lcg(n: usize) -> Vec<u8> {
        let mut s = 7u32;
        (0..n)
            .map(|_| {
                s = s.wrapping_mul(1664525).wrapping_add(1013904223);
                (s >> 24) as u8
            })
            .collect()
    }

    /// Decodes `packed` one byte per feed, collecting every piece of output.
    fn decode_bytewise(packed: &[u8], zlib: bool) -> Result<Vec<u8>, Error> {
        let mut d = Decoder::new(zlib, 1 << 24);
        let mut out = Vec::new();
        for b in packed {
            out.extend(d.feed(&[*b])?);
        }
        out.extend(d.finish()?);
        Ok(out)
    }

    #[test]
    fn one_byte_at_a_time_matches_the_whole_input() {
        let mut data = lcg(70_000);
        data.extend_from_slice(&b"hello hello hello, the quick brown fox ".repeat(900));
        let packed = deflate(&data);
        let mut whole = Decoder::new(false, 1 << 24);
        let mut out = whole.feed(&packed).unwrap();
        out.extend(whole.finish().unwrap());
        assert_eq!(out, data);
        assert_eq!(decode_bytewise(&packed, false).unwrap(), data);
    }

    #[test]
    fn zlib_known_vector_decodes_one_byte_at_a_time() {
        let zlib = [0x78, 0x9c, 0x4b, 0x04, 0x00, 0x00, 0x62, 0x00, 0x62];
        assert_eq!(decode_bytewise(&zlib, true).unwrap(), b"a".to_vec());
    }

    #[test]
    fn stored_block_is_copied_as_its_bytes_arrive() {
        let packed = [0x01, 0x05, 0x00, 0xfa, 0xff, b'h', b'e', b'l', b'l', b'o'];
        let mut d = Decoder::new(false, 100);
        let mut got = Vec::new();
        for b in &packed {
            got.extend(d.feed(&[*b]).unwrap());
        }
        assert_eq!(got, b"hello".to_vec());
        assert_eq!(d.finish().unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn output_arrives_before_the_stream_ends() {
        // The stored block at the front puts output in the first half of the input.
        let mut data = lcg(70_000);
        data.extend_from_slice(&b"streamed text, streamed text, streamed text ".repeat(200));
        let packed = deflate(&data);
        let mut d = Decoder::new(false, 1 << 20);
        let early = d.feed(&packed[..packed.len() / 2]).unwrap();
        assert!(!early.is_empty());
        assert_eq!(&data[..early.len()], &early[..]);
        let mut out = early;
        out.extend(d.feed(&packed[packed.len() / 2..]).unwrap());
        out.extend(d.finish().unwrap());
        assert_eq!(out, data);
    }

    #[test]
    fn a_short_stream_is_truncated_at_finish() {
        let mut d = Decoder::new(false, 100);
        d.feed(&[0xcb, 0x48]).unwrap();
        assert_eq!(d.finish(), Err(Error::Truncated));
    }

    #[test]
    fn bad_zlib_header_and_bad_trailer_are_refused() {
        let mut d = Decoder::new(true, 100);
        assert_eq!(d.feed(&[0x79, 0x9c]), Err(Error::BadBlock));
        // a correct stream for "a" with its Adler-32 trailer damaged
        let mut bad = vec![0x78, 0x9c, 0x4b, 0x04, 0x00];
        bad.extend_from_slice(&[0, 0, 0, 0]);
        let mut d = Decoder::new(true, 100);
        assert_eq!(d.feed(&bad), Err(Error::BadBlock));
    }

    #[test]
    fn the_limit_stops_the_stream_as_it_grows() {
        let packed = deflate(&[0u8; 100]);
        let mut d = Decoder::new(false, 10);
        assert_eq!(d.feed(&packed), Err(Error::TooBig));
    }
}
