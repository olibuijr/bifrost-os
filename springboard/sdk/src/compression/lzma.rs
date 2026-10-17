//! LZMA for apps: the LZMA2 codec inside the xz container, written from the public LZMA and LZMA2
//! descriptions and the xz file format specification.
//!
//! The buffer form of `Algorithm::Lzma` is a whole xz stream: the 12-byte stream header, one block whose only
//! filter is LZMA2 (ID 0x21, dictionary property 22, which is the 8 MiB of preset 6) with a CRC64 check, the index
//! and the 12-byte footer.
//! - The decoder accepts streams with a CRC32, CRC64, SHA-256 or no check and any number of blocks. It handles
//!   every LZMA2 chunk kind: uncompressed chunks with or without a dictionary reset, and LZMA chunks with no
//!   reset, a state reset, new properties or a dictionary reset. It checks the stream flags, the block headers,
//!   the block checks, the index records and CRC, and the footer. The dictionary is the output so far, so a
//!   distance is checked against the bytes produced, not against the declared dictionary size.
//! - The encoder is a greedy matcher with hash chains over 3-byte prefixes, feeding a range encoder with lc=3,
//!   lp=0 and pb=2. It writes literals and simple matches only (no rep matches). A chunk that does not shrink is
//!   written as an uncompressed LZMA2 chunk, and the model starts again after it.
//! The encoder ignores the compression level and the thread count, as LZ4 does.

use super::Error;
use crate::digest::sha256;

const XZ_MAGIC: [u8; 6] = [0xFD, b'7', b'z', b'X', b'Z', 0x00];
const XZ_FOOTER_MAGIC: [u8; 2] = *b"YZ";
/// The block filter ID of LZMA2.
const FILTER_LZMA2: u64 = 0x21;
/// The LZMA2 dictionary property: the size is `(2 | p & 1) << (p / 2 + 11)`, so 22 is 8 MiB (preset 6).
const DICT_PROP: u8 = 22;
/// The largest distance the encoder emits, the 8 MiB dictionary above.
const WINDOW: usize = 1 << 23;
/// The LZMA properties byte for lc=3, lp=0, pb=2: `(pb * 5 + lp) * 9 + lc`.
const PROPS_BYTE: u8 = 0x5D;
const LC: u32 = 3;
const LP: u32 = 0;
const PB: u32 = 2;
const MIN_MATCH: usize = 3;
/// The longest match the length coder takes: 2 + 271.
const MAX_MATCH: usize = 273;
/// Candidates examined per position by the matcher.
const MAX_CHAIN: usize = 32;
const HASH_BITS: u32 = 16;
/// Uncompressed bytes per chunk. An uncompressed LZMA2 chunk holds at most 64 KiB, so either kind fits.
const CHUNK: usize = 1 << 16;
/// The largest packed size of an LZMA2 chunk (the 16-bit size field plus one).
const MAX_PACKED: usize = 1 << 16;
/// A probability of one half, in units of 1/2048.
const PROB_INIT: u16 = 1024;
const TOP: u32 = 1 << 24;
const NUM_STATES: usize = 12;
const POS_STATES: usize = 16;
const END_POS_MODEL: usize = 14;
const FULL_DISTANCES: usize = 128;
/// The distance that marks the end of an LZMA stream. LZMA2 chunks never use it.
const END_MARKER: usize = 0xFFFF_FFFF;
const NONE: u32 = u32::MAX;

/// Block check IDs of the xz stream flags.
const CHECK_NONE: u8 = 0x00;
const CHECK_CRC32: u8 = 0x01;
const CHECK_CRC64: u8 = 0x04;
const CHECK_SHA256: u8 = 0x0A;

const fn crc32_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            k += 1;
        }
        table[i] = c;
        i += 1;
    }
    table
}

const fn crc64_table() -> [u64; 256] {
    let mut table = [0u64; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u64;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 { 0xC96C_5795_D787_0F42 ^ (c >> 1) } else { c >> 1 };
            k += 1;
        }
        table[i] = c;
        i += 1;
    }
    table
}

const CRC32_TABLE: [u32; 256] = crc32_table();
const CRC64_TABLE: [u64; 256] = crc64_table();

/// CRC-32 in the reflected form of the IEEE polynomial, as the xz block and index fields use it.
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in data {
        crc = CRC32_TABLE[((crc ^ b as u32) & 0xFF) as usize] ^ (crc >> 8);
    }
    !crc
}

/// CRC-64 of ECMA-182 in the reflected form, the xz CRC64 check.
pub fn crc64(data: &[u8]) -> u64 {
    let mut crc = !0u64;
    for &b in data {
        crc = CRC64_TABLE[((crc ^ b as u64) & 0xFF) as usize] ^ (crc >> 8);
    }
    !crc
}

/// Reads `N` bytes at `at`, or fails with `Truncated` when the input ends first.
fn field<const N: usize>(src: &[u8], at: usize) -> Result<[u8; N], Error> {
    src.get(at..at + N).and_then(|s| <[u8; N]>::try_from(s).ok()).ok_or(Error::Truncated)
}

/// Reads an xz multibyte integer (7 bits per byte, the high bit set on all but the last byte).
fn read_varint(src: &[u8], at: &mut usize) -> Result<u64, Error> {
    let mut value = 0u64;
    for i in 0..9 {
        let byte = *src.get(*at).ok_or(Error::Truncated)?;
        *at += 1;
        value |= u64::from(byte & 0x7F) << (7 * i);
        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }
    Err(Error::InvalidData)
}

fn push_varint(out: &mut Vec<u8>, mut value: u64) {
    loop {
        let low = (value & 0x7F) as u8;
        value >>= 7;
        if value == 0 {
            out.push(low);
            return;
        }
        out.push(low | 0x80);
    }
}

/// Splits an LZMA properties byte into (lc, lp, pb). LZMA2 allows lc + lp of at most 4.
fn parse_props(byte: u8) -> Result<(u32, u32, u32), Error> {
    if byte >= 225 {
        return Err(Error::InvalidData);
    }
    let d = u32::from(byte);
    let (lc, lp, pb) = (d % 9, (d / 9) % 5, d / 45);
    if lc + lp > 4 {
        return Err(Error::InvalidData);
    }
    Ok((lc, lp, pb))
}

/// The distance slot of a zero-based distance: the slots below 4 are the distances themselves, and each
/// slot above that covers a power-of-two range split in two.
fn pos_slot(dist: usize) -> usize {
    if dist < 4 {
        return dist;
    }
    let n = (usize::BITS - 1 - dist.leading_zeros()) as usize;
    2 * n + ((dist >> (n - 1)) & 1)
}

/// The adaptive bit models of LZMA. Each probability is the chance of a zero bit in units of 1/2048.
struct Model {
    is_match: [[u16; POS_STATES]; NUM_STATES],
    is_rep: [u16; NUM_STATES],
    is_rep_g0: [u16; NUM_STATES],
    is_rep_g1: [u16; NUM_STATES],
    is_rep_g2: [u16; NUM_STATES],
    is_rep0_long: [[u16; POS_STATES]; NUM_STATES],
    pos_slot: [[u16; 64]; 4],
    /// Reverse trees for the distance slots 4 to 13. Each slot uses a window of this array.
    pos_special: [u16; FULL_DISTANCES],
    align: [u16; 16],
    match_len: LenModel,
    rep_len: LenModel,
    literal: Vec<u16>,
}

impl Model {
    fn new(lc: u32, lp: u32) -> Model {
        Model {
            is_match: [[PROB_INIT; POS_STATES]; NUM_STATES],
            is_rep: [PROB_INIT; NUM_STATES],
            is_rep_g0: [PROB_INIT; NUM_STATES],
            is_rep_g1: [PROB_INIT; NUM_STATES],
            is_rep_g2: [PROB_INIT; NUM_STATES],
            is_rep0_long: [[PROB_INIT; POS_STATES]; NUM_STATES],
            pos_slot: [[PROB_INIT; 64]; 4],
            pos_special: [PROB_INIT; FULL_DISTANCES],
            align: [PROB_INIT; 16],
            match_len: LenModel::new(),
            rep_len: LenModel::new(),
            literal: vec![PROB_INIT; 0x300 << (lc + lp)],
        }
    }
}

/// The length coder: lengths 0 to 7, 8 to 15 and 16 to 271, each behind a choice bit.
struct LenModel {
    choice: u16,
    choice2: u16,
    low: [[u16; 8]; POS_STATES],
    mid: [[u16; 8]; POS_STATES],
    high: [u16; 256],
}

impl LenModel {
    fn new() -> LenModel {
        LenModel {
            choice: PROB_INIT,
            choice2: PROB_INIT,
            low: [[PROB_INIT; 8]; POS_STATES],
            mid: [[PROB_INIT; 8]; POS_STATES],
            high: [PROB_INIT; 256],
        }
    }

    /// Returns the zero-based length, 0 to 271.
    fn decode(&mut self, rc: &mut RangeDecoder<'_>, pos_state: usize) -> usize {
        if rc.bit(&mut self.choice) == 0 {
            return rc.bit_tree(&mut self.low[pos_state], 3) as usize;
        }
        if rc.bit(&mut self.choice2) == 0 {
            return 8 + rc.bit_tree(&mut self.mid[pos_state], 3) as usize;
        }
        16 + rc.bit_tree(&mut self.high, 8) as usize
    }

    fn encode(&mut self, rc: &mut RangeEncoder, len: usize, pos_state: usize) {
        if len < 8 {
            rc.encode_bit(&mut self.choice, 0);
            rc.bit_tree(&mut self.low[pos_state], 3, len as u32);
        } else if len < 16 {
            rc.encode_bit(&mut self.choice, 1);
            rc.encode_bit(&mut self.choice2, 0);
            rc.bit_tree(&mut self.mid[pos_state], 3, (len - 8) as u32);
        } else {
            rc.encode_bit(&mut self.choice, 1);
            rc.encode_bit(&mut self.choice2, 1);
            rc.bit_tree(&mut self.high, 8, (len - 16) as u32);
        }
    }
}

/// The range decoder over one chunk's packed bytes. It reads past the end as zeros and records the overrun in
/// `pos`, which the callers check.
struct RangeDecoder<'a> {
    data: &'a [u8],
    pos: usize,
    range: u32,
    code: u32,
}

impl<'a> RangeDecoder<'a> {
    /// Starts on a chunk: the first byte is zero and the next four are the code.
    fn new(data: &'a [u8]) -> Result<RangeDecoder<'a>, Error> {
        if data.len() < 5 || data[0] != 0 {
            return Err(Error::InvalidData);
        }
        let code = u32::from_be_bytes([data[1], data[2], data[3], data[4]]);
        if code == 0xFFFF_FFFF {
            return Err(Error::InvalidData);
        }
        Ok(RangeDecoder { data, pos: 5, range: 0xFFFF_FFFF, code })
    }

    fn overrun(&self) -> bool {
        self.pos > self.data.len()
    }

    fn normalize(&mut self) {
        if self.range < TOP {
            self.range <<= 8;
            let byte = self.data.get(self.pos).copied().unwrap_or(0);
            self.pos += 1;
            self.code = (self.code << 8) | u32::from(byte);
        }
    }

    /// Decodes one bit with the adaptive probability `prob` and updates it.
    fn bit(&mut self, prob: &mut u16) -> u32 {
        let bound = (self.range >> 11) * u32::from(*prob);
        let bit = if self.code < bound {
            self.range = bound;
            *prob += (2048 - *prob) >> 5;
            0
        } else {
            self.range -= bound;
            self.code -= bound;
            *prob -= *prob >> 5;
            1
        };
        self.normalize();
        bit
    }

    /// Decodes `count` bits at probability one half, most significant first.
    fn direct_bits(&mut self, count: u32) -> u32 {
        let mut result = 0u32;
        for _ in 0..count {
            self.range >>= 1;
            let bit = if self.code >= self.range {
                self.code -= self.range;
                1
            } else {
                0
            };
            result = (result << 1) | bit;
            self.normalize();
        }
        result
    }

    /// Decodes a `bits`-bit symbol through a binary tree of probabilities, most significant bit first.
    fn bit_tree(&mut self, probs: &mut [u16], bits: u32) -> u32 {
        let mut m = 1usize;
        for _ in 0..bits {
            m = (m << 1) | self.bit(&mut probs[m]) as usize;
        }
        (m - (1 << bits)) as u32
    }

    /// As `bit_tree`, but the symbol's bits come least significant first.
    fn reverse_bit_tree(&mut self, probs: &mut [u16], bits: u32) -> u32 {
        let mut m = 1usize;
        let mut sym = 0u32;
        for i in 0..bits {
            let bit = self.bit(&mut probs[m]);
            m = (m << 1) | bit as usize;
            sym |= bit << i;
        }
        sym
    }
}

/// Decodes one literal byte. After a match (`matched`), the bits are predicted from the byte at the last match
/// distance until the first bit that differs.
fn decode_literal(rc: &mut RangeDecoder<'_>, probs: &mut [u16], matched: bool, mut match_byte: usize) -> u8 {
    let mut sym = 1usize;
    if matched {
        while sym < 0x100 {
            let match_bit = (match_byte >> 7) & 1;
            match_byte <<= 1;
            let bit = rc.bit(&mut probs[((1 + match_bit) << 8) + sym]) as usize;
            sym = (sym << 1) | bit;
            if match_bit != bit {
                break;
            }
        }
    }
    while sym < 0x100 {
        sym = (sym << 1) | rc.bit(&mut probs[sym]) as usize;
    }
    (sym & 0xFF) as u8
}

/// The LZMA decoder state that LZMA2 carries from chunk to chunk: the model, the state machine and the rep
/// distances. Distances are zero-based (the copy source is `distance + 1` bytes back).
struct Decoder {
    lc: u32,
    lp: u32,
    pb: u32,
    model: Model,
    state: usize,
    reps: [usize; 4],
}

impl Decoder {
    fn new(lc: u32, lp: u32, pb: u32) -> Decoder {
        Decoder { lc, lp, pb, model: Model::new(lc, lp), state: 0, reps: [0; 4] }
    }

    /// Starts the probabilities, the state and the reps again, keeping the properties.
    fn reset_state(&mut self) {
        *self = Decoder::new(self.lc, self.lp, self.pb);
    }

    /// Decodes symbols into `out` until it holds `end` bytes. `dict_start` is the index in `out` of the last
    /// dictionary reset: positions and distances count from there.
    fn decode(&mut self, rc: &mut RangeDecoder<'_>, out: &mut Vec<u8>, dict_start: usize, end: usize) -> Result<(), Error> {
        let pb_mask = (1usize << self.pb) - 1;
        let lp_mask = (1usize << self.lp) - 1;
        let lc = self.lc;
        while out.len() < end {
            if rc.overrun() {
                return Err(Error::InvalidData);
            }
            let pos = out.len() - dict_start;
            let pos_state = pos & pb_mask;
            let st = self.state;
            if rc.bit(&mut self.model.is_match[st][pos_state]) == 0 {
                let prev = if pos > 0 { usize::from(out[out.len() - 1]) } else { 0 };
                let base = 0x300 * (((pos & lp_mask) << lc) + (prev >> (8 - lc)));
                let rep0 = self.reps[0];
                let matched = st >= 7;
                if matched && rep0 >= pos {
                    return Err(Error::InvalidData);
                }
                let match_byte = if matched { usize::from(out[out.len() - 1 - rep0]) } else { 0 };
                let probs = &mut self.model.literal[base..base + 0x300];
                let byte = decode_literal(rc, probs, matched, match_byte);
                out.push(byte);
                self.state = if st < 4 {
                    0
                } else if st < 10 {
                    st - 3
                } else {
                    st - 6
                };
                continue;
            }
            let len;
            if rc.bit(&mut self.model.is_rep[st]) == 0 {
                let l = self.model.match_len.decode(rc, pos_state);
                self.state = if st < 7 { 7 } else { 10 };
                let dist = self.decode_distance(rc, l);
                if dist == END_MARKER {
                    return Err(Error::InvalidData);
                }
                self.reps = [dist, self.reps[0], self.reps[1], self.reps[2]];
                len = l + 2;
            } else {
                if rc.bit(&mut self.model.is_rep_g0[st]) == 0 {
                    if rc.bit(&mut self.model.is_rep0_long[st][pos_state]) == 0 {
                        // a short rep: one byte at the last distance
                        self.state = if st < 7 { 9 } else { 11 };
                        let rep0 = self.reps[0];
                        if rep0 >= pos {
                            return Err(Error::InvalidData);
                        }
                        let byte = out[out.len() - 1 - rep0];
                        out.push(byte);
                        continue;
                    }
                } else {
                    let dist = if rc.bit(&mut self.model.is_rep_g1[st]) == 0 {
                        self.reps[1]
                    } else {
                        let d = if rc.bit(&mut self.model.is_rep_g2[st]) == 0 {
                            self.reps[2]
                        } else {
                            let d = self.reps[3];
                            self.reps[3] = self.reps[2];
                            d
                        };
                        self.reps[2] = self.reps[1];
                        d
                    };
                    self.reps[1] = self.reps[0];
                    self.reps[0] = dist;
                }
                let l = self.model.rep_len.decode(rc, pos_state);
                self.state = if st < 7 { 8 } else { 11 };
                len = l + 2;
            }
            let dist = self.reps[0];
            if dist >= pos || len > end - out.len() {
                return Err(Error::InvalidData);
            }
            // byte by byte, so that a match may overlap the bytes it is copying
            let from = out.len() - 1 - dist;
            for k in 0..len {
                let byte = out[from + k];
                out.push(byte);
            }
        }
        if rc.overrun() {
            return Err(Error::InvalidData);
        }
        Ok(())
    }

    /// Decodes a match distance (zero-based) for a match of zero-based length `len`.
    fn decode_distance(&mut self, rc: &mut RangeDecoder<'_>, len: usize) -> usize {
        let slot = rc.bit_tree(&mut self.model.pos_slot[len.min(3)], 6) as usize;
        if slot < 4 {
            return slot;
        }
        let footer_bits = ((slot >> 1) - 1) as u32;
        let base = (2 | (slot & 1)) << footer_bits;
        if slot < END_POS_MODEL {
            base + rc.reverse_bit_tree(&mut self.model.pos_special[base - slot..], footer_bits) as usize
        } else {
            let direct = rc.direct_bits(footer_bits - 4) as usize;
            base + (direct << 4) + rc.reverse_bit_tree(&mut self.model.align, 4) as usize
        }
    }
}

/// Decodes one LZMA2 stream and appends its data to `out`. Returns the number of input bytes it used, through
/// the end marker. The length of `out` may not pass `limit`.
fn lzma2_decode(src: &[u8], out: &mut Vec<u8>, limit: usize) -> Result<usize, Error> {
    let mut dec: Option<Decoder> = None;
    let mut dict_start = 0usize;
    let mut need_dict_reset = true;
    let mut need_props = true;
    let mut i = 0usize;
    loop {
        let control = *src.get(i).ok_or(Error::Truncated)?;
        i += 1;
        if control == 0 {
            return Ok(i);
        }
        if control < 0x80 {
            // an uncompressed chunk: 1 resets the dictionary, 2 does not
            if control > 2 || (control == 2 && need_dict_reset) {
                return Err(Error::InvalidData);
            }
            if control == 1 {
                dict_start = out.len();
                need_dict_reset = false;
            }
            let size = usize::from(u16::from_be_bytes(field(src, i)?)) + 1;
            i += 2;
            let data = src.get(i..i + size).ok_or(Error::Truncated)?;
            if out.len() + size > limit {
                return Err(Error::TooBig);
            }
            out.extend_from_slice(data);
            i += size;
        } else {
            // an LZMA chunk: bits 5 and 6 of the control byte say what is reset (0 none, 1 state, 2 state and
            // properties, 3 everything); the low five bits are the high bits of the unpacked size
            let reset = (control >> 5) & 3;
            let unpacked = ((usize::from(control & 0x1F) << 16) | usize::from(u16::from_be_bytes(field(src, i)?))) + 1;
            let packed = usize::from(u16::from_be_bytes(field(src, i + 2)?)) + 1;
            i += 4;
            if need_dict_reset && reset != 3 {
                return Err(Error::InvalidData);
            }
            if need_props && reset < 2 {
                return Err(Error::InvalidData);
            }
            if reset == 3 {
                dict_start = out.len();
                need_dict_reset = false;
            }
            if reset >= 2 {
                let (lc, lp, pb) = parse_props(*src.get(i).ok_or(Error::Truncated)?)?;
                i += 1;
                dec = Some(Decoder::new(lc, lp, pb));
                need_props = false;
            } else if reset == 1 {
                dec.as_mut().ok_or(Error::InvalidData)?.reset_state();
            }
            let d = dec.as_mut().ok_or(Error::InvalidData)?;
            let end = out.len() + unpacked;
            if end > limit {
                return Err(Error::TooBig);
            }
            let data = src.get(i..i + packed).ok_or(Error::Truncated)?;
            let mut rc = RangeDecoder::new(data)?;
            d.decode(&mut rc, out, dict_start, end)?;
            if rc.pos != data.len() {
                return Err(Error::InvalidData);
            }
            i += packed;
        }
    }
}

/// Checks `data` against the stored check bytes of the block check `kind`.
fn check_matches(kind: u8, data: &[u8], stored: &[u8]) -> bool {
    match kind {
        CHECK_NONE => true,
        CHECK_CRC32 => stored == &crc32(data).to_le_bytes()[..],
        CHECK_CRC64 => stored == &crc64(data).to_le_bytes()[..],
        _ => stored == &sha256(data)[..],
    }
}

/// The size in bytes of the check that the stream flags name, or `InvalidData` for a reserved check ID.
fn check_len(kind: u8) -> Result<usize, Error> {
    match kind {
        CHECK_NONE => Ok(0),
        CHECK_CRC32 => Ok(4),
        CHECK_CRC64 => Ok(8),
        CHECK_SHA256 => Ok(32),
        _ => Err(Error::InvalidData),
    }
}

/// Decodes the block at `start` and appends its data to `out`. Returns the block's unpadded size (header, data
/// and check) and the offset of the next block.
fn decode_block(src: &[u8], start: usize, check: u8, check_size: usize, out: &mut Vec<u8>, limit: usize) -> Result<(u64, usize), Error> {
    let size_byte = *src.get(start).ok_or(Error::Truncated)?;
    let header_len = (usize::from(size_byte) + 1) * 4;
    if header_len < 8 {
        return Err(Error::InvalidData);
    }
    let header = src.get(start..start + header_len).ok_or(Error::Truncated)?;
    let (body, stored) = header.split_at(header_len - 4);
    if crc32(body) != u32::from_le_bytes([stored[0], stored[1], stored[2], stored[3]]) {
        return Err(Error::InvalidData);
    }
    let flags = body[1];
    // one filter, and only the two size fields are defined
    if flags & 0x03 != 0 || flags & 0x3C != 0 {
        return Err(Error::InvalidData);
    }
    let mut p = 2;
    let comp_size = if flags & 0x40 != 0 { Some(read_varint(body, &mut p)?) } else { None };
    let uncomp_size = if flags & 0x80 != 0 { Some(read_varint(body, &mut p)?) } else { None };
    if read_varint(body, &mut p)? != FILTER_LZMA2 || read_varint(body, &mut p)? != 1 {
        return Err(Error::InvalidData);
    }
    let dict = *body.get(p).ok_or(Error::InvalidData)?;
    p += 1;
    if dict > 40 || body[p..].iter().any(|&b| b != 0) {
        return Err(Error::InvalidData);
    }

    let data_start = start + header_len;
    let before = out.len();
    let consumed = lzma2_decode(src.get(data_start..).ok_or(Error::Truncated)?, out, limit)?;
    let uncompressed = out.len() - before;
    if let Some(c) = comp_size {
        if c != consumed as u64 {
            return Err(Error::InvalidData);
        }
    }
    if let Some(u) = uncomp_size {
        if u != uncompressed as u64 {
            return Err(Error::InvalidData);
        }
    }
    let data_end = data_start + consumed;
    let check_start = data_end + (4 - consumed % 4) % 4;
    if src.get(data_end..check_start).ok_or(Error::Truncated)?.iter().any(|&b| b != 0) {
        return Err(Error::InvalidData);
    }
    let stored_check = src.get(check_start..check_start + check_size).ok_or(Error::Truncated)?;
    if !check_matches(check, &out[before..], stored_check) {
        return Err(Error::InvalidData);
    }
    Ok(((header_len + consumed + check_size) as u64, check_start + check_size))
}

/// Decodes an xz stream and returns its data. Output that would pass `limit` bytes fails with `TooBig`.
pub fn decode_buffer(src: &[u8], limit: usize) -> Result<Vec<u8>, Error> {
    if src.len() < XZ_MAGIC.len() {
        return Err(Error::Truncated);
    }
    if src[..XZ_MAGIC.len()] != XZ_MAGIC {
        return Err(Error::InvalidData);
    }
    let flags: [u8; 2] = field(src, 6)?;
    let stored_crc = u32::from_le_bytes(field(src, 8)?);
    if flags[0] != 0 || (flags[1] & 0xF0) != 0 || crc32(&flags) != stored_crc {
        return Err(Error::InvalidData);
    }
    let check = flags[1] & 0x0F;
    let check_size = check_len(check)?;

    let mut out = Vec::new();
    let mut records: Vec<(u64, u64)> = Vec::new();
    let mut pos = 12;
    // a block header's size byte is never zero, so a zero here starts the index
    while *src.get(pos).ok_or(Error::Truncated)? != 0 {
        let before = out.len();
        let (unpadded, next) = decode_block(src, pos, check, check_size, &mut out, limit)?;
        records.push((unpadded, (out.len() - before) as u64));
        pos = next;
    }

    let index_start = pos;
    let mut p = pos + 1;
    if read_varint(src, &mut p)? != records.len() as u64 {
        return Err(Error::InvalidData);
    }
    for &(unpadded, uncompressed) in &records {
        if read_varint(src, &mut p)? != unpadded || read_varint(src, &mut p)? != uncompressed {
            return Err(Error::InvalidData);
        }
    }
    while (p - index_start) % 4 != 0 {
        if *src.get(p).ok_or(Error::Truncated)? != 0 {
            return Err(Error::InvalidData);
        }
        p += 1;
    }
    let index_crc = u32::from_le_bytes(field(src, p)?);
    if crc32(&src[index_start..p]) != index_crc {
        return Err(Error::InvalidData);
    }
    p += 4;
    let index_size = p - index_start;

    let footer: [u8; 12] = field(src, p)?;
    if src.len() != p + 12 {
        return Err(Error::InvalidData);
    }
    let footer_crc = u32::from_le_bytes([footer[0], footer[1], footer[2], footer[3]]);
    if crc32(&footer[4..10]) != footer_crc {
        return Err(Error::InvalidData);
    }
    let backward = u32::from_le_bytes([footer[4], footer[5], footer[6], footer[7]]) as usize;
    if (backward + 1) * 4 != index_size || footer[8..10] != flags[..] || footer[10..12] != XZ_FOOTER_MAGIC {
        return Err(Error::InvalidData);
    }
    Ok(out)
}

/// The range encoder: the mirror of `RangeDecoder`, with the carry handled by a cached byte.
struct RangeEncoder {
    low: u64,
    range: u32,
    cache: u8,
    cache_size: u64,
    out: Vec<u8>,
}

impl RangeEncoder {
    fn new() -> RangeEncoder {
        RangeEncoder { low: 0, range: 0xFFFF_FFFF, cache: 0, cache_size: 1, out: Vec::new() }
    }

    fn shift_low(&mut self) {
        if (self.low as u32) < 0xFF00_0000 || (self.low >> 32) != 0 {
            let carry = (self.low >> 32) as u8;
            let mut temp = self.cache;
            loop {
                self.out.push(temp.wrapping_add(carry));
                temp = 0xFF;
                self.cache_size -= 1;
                if self.cache_size == 0 {
                    break;
                }
            }
            self.cache = ((self.low >> 24) & 0xFF) as u8;
        }
        self.cache_size += 1;
        self.low = (self.low & 0x00FF_FFFF) << 8;
    }

    fn normalize(&mut self) {
        if self.range < TOP {
            self.range <<= 8;
            self.shift_low();
        }
    }

    fn encode_bit(&mut self, prob: &mut u16, bit: u32) {
        let bound = (self.range >> 11) * u32::from(*prob);
        if bit == 0 {
            self.range = bound;
            *prob += (2048 - *prob) >> 5;
        } else {
            self.low += u64::from(bound);
            self.range -= bound;
            *prob -= *prob >> 5;
        }
        self.normalize();
    }

    fn encode_direct(&mut self, value: u32, count: u32) {
        for i in (0..count).rev() {
            self.range >>= 1;
            if (value >> i) & 1 == 1 {
                self.low += u64::from(self.range);
            }
            self.normalize();
        }
    }

    fn bit_tree(&mut self, probs: &mut [u16], bits: u32, value: u32) {
        let mut m = 1usize;
        for i in (0..bits).rev() {
            let bit = (value >> i) & 1;
            self.encode_bit(&mut probs[m], bit);
            m = (m << 1) | bit as usize;
        }
    }

    fn reverse_bit_tree(&mut self, probs: &mut [u16], bits: u32, value: u32) {
        let mut m = 1usize;
        let mut v = value;
        for _ in 0..bits {
            let bit = v & 1;
            v >>= 1;
            self.encode_bit(&mut probs[m], bit);
            m = (m << 1) | bit as usize;
        }
    }

    /// Flushes the five bytes that the decoder reads back as its final code.
    fn finish(mut self) -> Vec<u8> {
        for _ in 0..5 {
            self.shift_low();
        }
        self.out
    }
}

fn encode_distance(rc: &mut RangeEncoder, model: &mut Model, dist: usize, len_state: usize) {
    let slot = pos_slot(dist);
    rc.bit_tree(&mut model.pos_slot[len_state], 6, slot as u32);
    if slot >= 4 {
        let footer_bits = ((slot >> 1) - 1) as u32;
        let base = (2 | (slot & 1)) << footer_bits;
        let reduced = dist - base;
        if slot < END_POS_MODEL {
            rc.reverse_bit_tree(&mut model.pos_special[base - slot..], footer_bits, reduced as u32);
        } else {
            rc.encode_direct((reduced >> 4) as u32, footer_bits - 4);
            rc.reverse_bit_tree(&mut model.align, 4, (reduced & 15) as u32);
        }
    }
}

/// One step of the parse: a literal, or a match of `len` bytes at zero-based-plus-one distance `dist`.
#[derive(Clone, Copy)]
enum Op {
    Literal,
    Match { len: usize, dist: usize },
}

/// Writes the symbols of one chunk, which starts at `start` in `src`. `state` and `rep0` carry the LZMA state
/// across chunks.
fn encode_ops(rc: &mut RangeEncoder, model: &mut Model, state: &mut usize, rep0: &mut usize, src: &[u8], start: usize, ops: &[Op]) {
    let mut pos = start;
    for op in ops {
        let pos_state = pos & ((1usize << PB) - 1);
        let advance = match *op {
            Op::Literal => {
                rc.encode_bit(&mut model.is_match[*state][pos_state], 0);
                let prev = if pos > 0 { usize::from(src[pos - 1]) } else { 0 };
                let base = 0x300 * (prev >> (8 - LC));
                let matched = *state >= 7;
                let mut match_byte = if matched { u32::from(src[pos - 1 - *rep0]) } else { 0 };
                let byte = u32::from(src[pos]);
                let probs = &mut model.literal[base..base + 0x300];
                let mut sym = 1usize;
                let mut same = matched;
                for k in (0..8u32).rev() {
                    let bit = (byte >> k) & 1;
                    if same {
                        let match_bit = (match_byte >> 7) & 1;
                        match_byte <<= 1;
                        rc.encode_bit(&mut probs[((1 + match_bit as usize) << 8) + sym], bit);
                        same = match_bit == bit;
                    } else {
                        rc.encode_bit(&mut probs[sym], bit);
                    }
                    sym = (sym << 1) | bit as usize;
                }
                *state = if *state < 4 {
                    0
                } else if *state < 10 {
                    *state - 3
                } else {
                    *state - 6
                };
                1
            }
            Op::Match { len, dist } => {
                rc.encode_bit(&mut model.is_match[*state][pos_state], 1);
                rc.encode_bit(&mut model.is_rep[*state], 0);
                model.match_len.encode(rc, len - 2, pos_state);
                *state = if *state < 7 { 7 } else { 10 };
                let d = dist - 1;
                encode_distance(rc, model, d, (len - 2).min(3));
                *rep0 = d;
                len
            }
        };
        pos += advance;
    }
}

/// Hash chains over 3-byte prefixes: `head` is the latest position with each hash and `prev` the position before
/// it. Positions are u32; an input of 4 GiB or more is written as literals only.
struct Matcher {
    enabled: bool,
    head: Vec<u32>,
    prev: Vec<u32>,
}

impl Matcher {
    fn new(len: usize) -> Matcher {
        let enabled = len < NONE as usize;
        Matcher { enabled, head: vec![NONE; 1 << HASH_BITS], prev: if enabled { vec![NONE; len] } else { Vec::new() } }
    }

    fn hash(src: &[u8], i: usize) -> usize {
        let v = u32::from(src[i]) | (u32::from(src[i + 1]) << 8) | (u32::from(src[i + 2]) << 16);
        (v.wrapping_mul(2_654_435_761) >> (32 - HASH_BITS)) as usize
    }

    fn insert(&mut self, src: &[u8], i: usize) {
        if self.enabled && i + MIN_MATCH <= src.len() {
            let h = Self::hash(src, i);
            self.prev[i] = self.head[h];
            self.head[h] = i as u32;
        }
    }

    /// The longest match at `i` that ends by `end`, as (length, distance). The length is 0 when there is none.
    fn find(&self, src: &[u8], i: usize, end: usize) -> (usize, usize) {
        let limit = (end - i).min(MAX_MATCH);
        if !self.enabled || limit < MIN_MATCH {
            return (0, 0);
        }
        let mut best = (0usize, 0usize);
        let mut cand = self.head[Self::hash(src, i)];
        let mut depth = 0;
        while cand != NONE && depth < MAX_CHAIN {
            let c = cand as usize;
            if i - c > WINDOW {
                break;
            }
            let mut len = 0;
            while len < limit && src[c + len] == src[i + len] {
                len += 1;
            }
            if len > best.0 {
                best = (len, i - c);
                if len == limit {
                    break;
                }
            }
            cand = self.prev[c];
            depth += 1;
        }
        if best.0 >= MIN_MATCH {
            best
        } else {
            (0, 0)
        }
    }

    /// Splits `src[start..end]` greedily into literals and matches. Every position is entered in the chains as
    /// it passes, so the history is complete for the next call.
    fn parse(&mut self, src: &[u8], start: usize, end: usize) -> Vec<Op> {
        let mut ops = Vec::new();
        let mut i = start;
        while i < end {
            let (len, dist) = self.find(src, i, end);
            self.insert(src, i);
            if len > 0 {
                for j in i + 1..i + len {
                    self.insert(src, j);
                }
                ops.push(Op::Match { len, dist });
                i += len;
            } else {
                ops.push(Op::Literal);
                i += 1;
            }
        }
        ops
    }
}

/// Encodes `src` as LZMA2 chunks. A chunk that does not shrink is stored raw, and the next LZMA chunk then
/// starts with a state reset, so the model is always fresh where a chunk needs one.
fn encode_lzma2(src: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut matcher = Matcher::new(src.len());
    let mut model = Model::new(LC, LP);
    let mut state = 0usize;
    let mut rep0 = 0usize;
    let mut need_reset = true;
    let mut first = true;
    let mut start = 0;
    while start < src.len() {
        let end = (start + CHUNK).min(src.len());
        let ops = matcher.parse(src, start, end);
        if need_reset {
            model = Model::new(LC, LP);
            state = 0;
            rep0 = 0;
        }
        let mut rc = RangeEncoder::new();
        encode_ops(&mut rc, &mut model, &mut state, &mut rep0, src, start, &ops);
        let packed = rc.finish();
        let unpacked = end - start;
        if packed.len() < unpacked && packed.len() <= MAX_PACKED {
            // dictionary reset with properties on the first chunk, state reset with properties after a raw chunk
            let control: u8 = if first {
                0xE0
            } else if need_reset {
                0xC0
            } else {
                0x80
            };
            let u = unpacked - 1;
            out.push(control | (u >> 16) as u8);
            out.extend_from_slice(&((u & 0xFFFF) as u16).to_be_bytes());
            out.extend_from_slice(&((packed.len() - 1) as u16).to_be_bytes());
            if control >= 0xC0 {
                out.push(PROPS_BYTE);
            }
            out.extend_from_slice(&packed);
            need_reset = false;
        } else {
            out.push(if first { 0x01 } else { 0x02 });
            out.extend_from_slice(&((unpacked - 1) as u16).to_be_bytes());
            out.extend_from_slice(&src[start..end]);
            need_reset = true;
        }
        first = false;
        start = end;
    }
    out.push(0x00);
    out
}

/// Encodes `src` as an xz stream: one LZMA2 block with a CRC64 check, an index of that block and the footer.
pub fn encode_buffer(src: &[u8]) -> Vec<u8> {
    let flags = [0x00, CHECK_CRC64];
    let mut out = Vec::new();
    out.extend_from_slice(&XZ_MAGIC);
    out.extend_from_slice(&flags);
    out.extend_from_slice(&crc32(&flags).to_le_bytes());

    // block header: size (12 bytes in all), flags (one filter, no sizes), the LZMA2 filter with its one property
    // byte, zero padding, CRC32
    let mut header = vec![0x02, 0x00, FILTER_LZMA2 as u8, 0x01, DICT_PROP, 0, 0, 0];
    let header_crc = crc32(&header);
    header.extend_from_slice(&header_crc.to_le_bytes());
    out.extend_from_slice(&header);

    let data = encode_lzma2(src);
    out.extend_from_slice(&data);
    out.resize(out.len() + (4 - data.len() % 4) % 4, 0);
    out.extend_from_slice(&crc64(src).to_le_bytes());

    // index: one record of the unpadded block size (header, data and check) and the uncompressed size
    let index_start = out.len();
    out.push(0x00);
    push_varint(&mut out, 1);
    push_varint(&mut out, (header.len() + data.len() + 8) as u64);
    push_varint(&mut out, src.len() as u64);
    while (out.len() - index_start) % 4 != 0 {
        out.push(0);
    }
    let index_crc = crc32(&out[index_start..]);
    out.extend_from_slice(&index_crc.to_le_bytes());
    let index_size = out.len() - index_start;

    // footer: CRC32 of the backward size and the flags, the backward size in 4-byte units, the flags, "YZ"
    let mut footer_body = (((index_size / 4) - 1) as u32).to_le_bytes().to_vec();
    footer_body.extend_from_slice(&flags);
    out.extend_from_slice(&crc32(&footer_body).to_le_bytes());
    out.extend_from_slice(&footer_body);
    out.extend_from_slice(&XZ_FOOTER_MAGIC);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_values_match_the_published_vectors() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc64(b"123456789"), 0x995D_C9BB_DF19_39FA);
    }

    #[test]
    fn varints_round_trip_through_their_bytes() {
        for value in [0u64, 1, 127, 128, 300, 1 << 20, (1 << 62) + 5] {
            let mut bytes = Vec::new();
            push_varint(&mut bytes, value);
            let mut at = 0;
            assert_eq!(read_varint(&bytes, &mut at).unwrap(), value);
            assert_eq!(at, bytes.len());
        }
    }

    #[test]
    fn the_properties_byte_of_the_encoder_is_lc3_lp0_pb2() {
        assert_eq!(parse_props(PROPS_BYTE).unwrap(), (3, 0, 2));
        assert!(parse_props(225).is_err());
        assert_eq!(parse_props(0x04).unwrap(), (4, 0, 0));
        // lc=4, lp=1 breaks the LZMA2 limit of lc + lp <= 4: (pb * 5 + lp) * 9 + lc with pb=0, lp=1, lc=4
        assert!(parse_props(13).is_err());
    }

    #[test]
    fn each_distance_lies_in_the_range_of_its_slot() {
        for dist in [0usize, 1, 3, 4, 5, 6, 7, 8, 100, 4095, 1 << 20, WINDOW - 1] {
            let slot = pos_slot(dist);
            if slot < 4 {
                assert_eq!(slot, dist);
                continue;
            }
            let footer = ((slot >> 1) - 1) as u32;
            let base = (2 | (slot & 1)) << footer;
            assert!(base <= dist && dist < base + (1 << footer), "distance {} in slot {}", dist, slot);
        }
    }
}
