//! LZ4 for apps, written from the public LZ4 block and frame format specifications.
//!
//! Three layers:
//! - the LZ4 block format (sequences of literals and matches, 64 KiB offsets, minimum match 4);
//! - the buffer framing the compression API uses for LZ4: `bv41` compressed blocks, `bv4-` stored
//!   blocks and a `bv4$` end marker (magic, then little-endian sizes, then the payload);
//! - the LZ4 frame format (magic 0x184D2204, FLG and BD descriptors, xxHash32 header checksum,
//!   optional block and content checksums), exposed as `encode_frame` and `decode_frame`.
//!
//! The encoder is a greedy matcher with a hash table over 4-byte windows. It follows the end rules
//! of the block format: the last 5 bytes are always literals, and the last match starts at least
//! 12 bytes before the end of the block.

use super::Error;

/// Minimum match length of the block format.
const MIN_MATCH: usize = 4;
/// Largest back-reference distance, the 64 KiB window.
const MAX_OFFSET: usize = 65535;
/// A match must start at or before `len - MF_LIMIT`.
const MF_LIMIT: usize = 12;
/// The last `LAST_LITERALS` bytes of a block are always literals.
const LAST_LITERALS: usize = 5;
const HASH_BITS: u32 = 16;
/// Input is split into blocks of this size for the buffer framing (the frame's 64 KiB block size).
const CHUNK: usize = 65536;

const BV4_COMPRESSED: &[u8; 4] = b"bv41";
const BV4_STORED: &[u8; 4] = b"bv4-";
const BV4_END: &[u8; 4] = b"bv4$";

const FRAME_MAGIC: u32 = 0x184D_2204;

fn read_u32(src: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([src[at], src[at + 1], src[at + 2], src[at + 3]])
}

fn hash4(v: u32) -> usize {
    (v.wrapping_mul(2_654_435_761) >> (32 - HASH_BITS)) as usize
}

/// Appends `n` as the extra length bytes that follow a nibble of 15.
fn push_length_extension(out: &mut Vec<u8>, mut n: usize) {
    while n >= 255 {
        out.push(255);
        n -= 255;
    }
    out.push(n as u8);
}

/// Encodes one LZ4 block. The result is never empty: an empty input becomes a single zero token.
pub fn encode_block(src: &[u8]) -> Vec<u8> {
    let n = src.len();
    let mut out = Vec::with_capacity(n / 2 + 16);
    let mut anchor = 0;
    let mut table = vec![usize::MAX; 1 << HASH_BITS];
    // matches may only start while i <= mf_limit; a block shorter than MF_LIMIT has no matches
    let mf_limit = n.checked_sub(MF_LIMIT);
    let mut i = 0;
    if let Some(mf_limit) = mf_limit {
        while i <= mf_limit {
            let seq = read_u32(src, i);
            let h = hash4(seq);
            let cand = table[h];
            table[h] = i;
            if cand != usize::MAX && i - cand <= MAX_OFFSET && read_u32(src, cand) == seq {
                // extend the match; it may not reach into the last LAST_LITERALS bytes
                let limit = n - LAST_LITERALS;
                let mut len = MIN_MATCH;
                while i + len < limit && src[cand + len] == src[i + len] {
                    len += 1;
                }
                emit_sequence(&mut out, &src[anchor..i], Some((i - cand, len)));
                i += len;
                anchor = i;
            } else {
                i += 1;
            }
        }
    }
    emit_sequence(&mut out, &src[anchor..], None);
    out
}

/// Writes one sequence: the token, literal and match length extensions, the literals, and
/// (when there is a match) the 16-bit little-endian offset and the match length extension.
fn emit_sequence(out: &mut Vec<u8>, literals: &[u8], found: Option<(usize, usize)>) {
    let lit_len = literals.len();
    let match_part = found.map(|(offset, len)| (offset, len - MIN_MATCH));
    let lit_nibble = lit_len.min(15) as u8;
    let match_nibble = match_part.map_or(0, |(_, m)| m.min(15) as u8);
    out.push((lit_nibble << 4) | match_nibble);
    if lit_len >= 15 {
        push_length_extension(out, lit_len - 15);
    }
    out.extend_from_slice(literals);
    if let Some((offset, m)) = match_part {
        out.extend_from_slice(&(offset as u16).to_le_bytes());
        if m >= 15 {
            push_length_extension(out, m - 15);
        }
    }
}

/// Decodes one LZ4 block, refusing any output longer than `limit`. Offsets of 0 or beyond the
/// output produced so far are rejected, and a block that ends before its final literal sequence
/// is `Truncated`.
pub fn decode_block(src: &[u8], limit: usize) -> Result<Vec<u8>, Error> {
    let mut out = Vec::new();
    decode_block_into(&mut out, src, limit)?;
    Ok(out)
}

/// Decodes one LZ4 block onto the end of `out`, whose earlier bytes serve as the history: a match
/// may reach back into the blocks decoded before (linked blocks in a frame, and encoders that keep
/// their window across buffer blocks). `limit` caps the whole of `out`.
fn decode_block_into(out: &mut Vec<u8>, src: &[u8], limit: usize) -> Result<(), Error> {
    let mut i = 0;
    loop {
        if i >= src.len() {
            return Err(Error::Truncated);
        }
        let token = src[i];
        i += 1;
        let mut lit = (token >> 4) as usize;
        if lit == 15 {
            lit += read_extension(src, &mut i)?;
        }
        let end = i.checked_add(lit).ok_or(Error::InvalidData)?;
        if end > src.len() {
            return Err(Error::Truncated);
        }
        if out.len() + lit > limit {
            return Err(Error::TooBig);
        }
        out.extend_from_slice(&src[i..end]);
        i = end;
        if i == src.len() {
            // the last sequence carries literals only
            return Ok(());
        }
        if i + 2 > src.len() {
            return Err(Error::Truncated);
        }
        let offset = u16::from_le_bytes([src[i], src[i + 1]]) as usize;
        i += 2;
        if offset == 0 || offset > out.len() {
            return Err(Error::InvalidData);
        }
        let mut m = (token & 0x0f) as usize;
        if m == 15 {
            m += read_extension(src, &mut i)?;
        }
        let m = m + MIN_MATCH;
        if out.len() + m > limit {
            return Err(Error::TooBig);
        }
        // byte by byte, because the source and destination may overlap
        let start = out.len() - offset;
        for k in 0..m {
            let b = out[start + k];
            out.push(b);
        }
    }
}

/// Reads the extra length bytes after a nibble of 15, returning their sum.
fn read_extension(src: &[u8], i: &mut usize) -> Result<usize, Error> {
    let mut total = 0usize;
    loop {
        let b = *src.get(*i).ok_or(Error::Truncated)?;
        *i += 1;
        total = total.checked_add(b as usize).ok_or(Error::InvalidData)?;
        if b != 255 {
            return Ok(total);
        }
    }
}

/// Compresses `src` into the buffer framing: `bv41` blocks for compressible 64 KiB chunks, `bv4-`
/// stored blocks for the rest, and a `bv4$` end marker.
pub fn encode_buffer(src: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(src.len() / 2 + 16);
    for chunk in src.chunks(CHUNK) {
        push_buffer_block(&mut out, chunk);
    }
    out.extend_from_slice(BV4_END);
    out
}

/// Appends one buffer-framing block for `chunk`: compressed when that is smaller, stored otherwise.
fn push_buffer_block(out: &mut Vec<u8>, chunk: &[u8]) {
    let packed = encode_block(chunk);
    if packed.len() < chunk.len() {
        out.extend_from_slice(BV4_COMPRESSED);
        out.extend_from_slice(&(chunk.len() as u32).to_le_bytes());
        out.extend_from_slice(&(packed.len() as u32).to_le_bytes());
        out.extend_from_slice(&packed);
    } else {
        out.extend_from_slice(BV4_STORED);
        out.extend_from_slice(&(chunk.len() as u32).to_le_bytes());
        out.extend_from_slice(chunk);
    }
}

/// Incremental encoder for the buffer framing. `write` returns a block as soon as 64 KiB of input are
/// in, `flush` closes the partial block so a reader can decode everything written so far, and `finish`
/// writes the rest and the end marker. Without `flush` calls the output is `encode_buffer` of the whole
/// input, whatever the split into writes.
#[derive(Debug, Default)]
pub struct Bv4Encoder {
    pending: Vec<u8>,
}

impl Bv4Encoder {
    pub fn new() -> Bv4Encoder {
        Bv4Encoder::default()
    }

    /// Adds `data` and returns the blocks it completed.
    pub fn write(&mut self, data: &[u8]) -> Vec<u8> {
        self.pending.extend_from_slice(data);
        let mut out = Vec::new();
        let mut used = 0;
        while self.pending.len() - used >= CHUNK {
            push_buffer_block(&mut out, &self.pending[used..used + CHUNK]);
            used += CHUNK;
        }
        self.pending.drain(..used);
        out
    }

    /// Writes the input collected so far as a block of its own (nothing when none is pending).
    pub fn flush(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        if !self.pending.is_empty() {
            push_buffer_block(&mut out, &self.pending);
            self.pending.clear();
        }
        out
    }

    /// Writes the last block and the end marker.
    pub fn finish(mut self) -> Vec<u8> {
        let mut out = self.flush();
        out.extend_from_slice(BV4_END);
        out
    }
}

/// The largest block a decoder accepts from the buffer framing. The encoder writes 64 KiB blocks; the
/// cap keeps a damaged size field from making the decoder wait for gigabytes.
const MAX_BLOCK: usize = 16 << 20;

/// Incremental decoder for the buffer framing. `feed` takes input in any split and returns the bytes of
/// every block that is whole; a partial block waits for more input. Output past `limit` bytes fails
/// with `TooBig`, a block whose decoded size disagrees with its header is `InvalidData`, and bytes
/// after the end marker are `InvalidData`.
#[derive(Debug)]
pub struct Bv4Decoder {
    input: Vec<u8>,
    /// The last 64 KiB decoded, the history that a later block may match against.
    history: Vec<u8>,
    produced: usize,
    limit: usize,
    done: bool,
}

impl Bv4Decoder {
    pub fn new(limit: usize) -> Bv4Decoder {
        Bv4Decoder { input: Vec::new(), history: Vec::new(), produced: 0, limit, done: false }
    }

    fn peek_u32(&self, at: usize) -> Option<usize> {
        let b = self.input.get(at..at + 4)?;
        Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
    }

    /// Adds `data` and returns the output it completes.
    pub fn feed(&mut self, data: &[u8]) -> Result<Vec<u8>, Error> {
        if self.done {
            return if data.is_empty() { Ok(Vec::new()) } else { Err(Error::InvalidData) };
        }
        self.input.extend_from_slice(data);
        let mut out = Vec::new();
        let mut pos = 0;
        while let Some(m) = self.input.get(pos..pos + 4) {
            let magic = [m[0], m[1], m[2], m[3]];
            if &magic == BV4_END {
                pos += 4;
                self.done = true;
                break;
            } else if &magic == BV4_STORED {
                let Some(size) = self.peek_u32(pos + 4) else { break };
                if size > MAX_BLOCK {
                    return Err(Error::InvalidData);
                }
                if self.produced + size > self.limit {
                    return Err(Error::TooBig);
                }
                let (from, to) = (pos + 8, pos + 8 + size);
                if self.input.len() < to {
                    break;
                }
                out.extend_from_slice(&self.input[from..to]);
                self.history.extend_from_slice(&self.input[from..to]);
                self.produced += size;
                pos = to;
            } else if &magic == BV4_COMPRESSED {
                let (Some(decoded), Some(encoded)) = (self.peek_u32(pos + 4), self.peek_u32(pos + 8)) else { break };
                if decoded > MAX_BLOCK || encoded > MAX_BLOCK {
                    return Err(Error::InvalidData);
                }
                if self.produced + decoded > self.limit {
                    return Err(Error::TooBig);
                }
                let (from, to) = (pos + 12, pos + 12 + encoded);
                if self.input.len() < to {
                    break;
                }
                let start = self.history.len();
                decode_block_into(&mut self.history, &self.input[from..to], start + decoded).map_err(|e| match e {
                    Error::TooBig => Error::InvalidData,
                    other => other,
                })?;
                if self.history.len() - start != decoded {
                    return Err(Error::InvalidData);
                }
                out.extend_from_slice(&self.history[start..]);
                self.produced += decoded;
                pos = to;
            } else {
                return Err(Error::InvalidData);
            }
            if self.history.len() > CHUNK {
                let cut = self.history.len() - CHUNK;
                self.history.drain(..cut);
            }
        }
        if self.done && self.input.len() > pos {
            return Err(Error::InvalidData);
        }
        self.input.drain(..pos);
        Ok(out)
    }

    /// Ends the input. Fails with `Truncated` unless the end marker was read.
    pub fn finish(self) -> Result<Vec<u8>, Error> {
        if self.done {
            Ok(Vec::new())
        } else {
            Err(Error::Truncated)
        }
    }
}

/// Decompresses the buffer framing, refusing output larger than `limit`. A block whose decoded
/// size does not match its header is `InvalidData`; a missing end marker is `Truncated`.
pub fn decode_buffer(src: &[u8], limit: usize) -> Result<Vec<u8>, Error> {
    let mut out: Vec<u8> = Vec::new();
    let mut pos = 0;
    loop {
        let magic = src.get(pos..pos + 4).ok_or(Error::Truncated)?;
        pos += 4;
        if magic == BV4_END {
            return Ok(out);
        } else if magic == BV4_STORED {
            let size = read_size(src, &mut pos)?;
            if out.len() + size > limit {
                return Err(Error::TooBig);
            }
            let raw = src.get(pos..pos + size).ok_or(Error::Truncated)?;
            out.extend_from_slice(raw);
            pos += size;
        } else if magic == BV4_COMPRESSED {
            let decoded = read_size(src, &mut pos)?;
            let encoded = read_size(src, &mut pos)?;
            if out.len() + decoded > limit {
                return Err(Error::TooBig);
            }
            let payload = src.get(pos..pos + encoded).ok_or(Error::Truncated)?;
            let start = out.len();
            decode_block_into(&mut out, payload, start + decoded).map_err(|e| match e {
                Error::TooBig => Error::InvalidData,
                other => other,
            })?;
            if out.len() - start != decoded {
                return Err(Error::InvalidData);
            }
            pos += encoded;
        } else {
            return Err(Error::InvalidData);
        }
    }
}

fn read_size(src: &[u8], pos: &mut usize) -> Result<usize, Error> {
    let bytes = src.get(*pos..*pos + 4).ok_or(Error::Truncated)?;
    *pos += 4;
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize)
}

/// xxHash32 (public algorithm specification), used for the frame checksums.
pub fn xxh32(input: &[u8], seed: u32) -> u32 {
    const P1: u32 = 2_654_435_761;
    const P2: u32 = 2_246_822_519;
    const P3: u32 = 3_266_489_917;
    const P4: u32 = 668_265_263;
    const P5: u32 = 374_761_393;
    fn round(acc: u32, lane: u32) -> u32 {
        acc.wrapping_add(lane.wrapping_mul(P2)).rotate_left(13).wrapping_mul(P1)
    }
    let len = input.len();
    let mut p = 0;
    let mut h: u32;
    if len >= 16 {
        let mut v1 = seed.wrapping_add(P1).wrapping_add(P2);
        let mut v2 = seed.wrapping_add(P2);
        let mut v3 = seed;
        let mut v4 = seed.wrapping_sub(P1);
        while p + 16 <= len {
            v1 = round(v1, read_u32(input, p));
            v2 = round(v2, read_u32(input, p + 4));
            v3 = round(v3, read_u32(input, p + 8));
            v4 = round(v4, read_u32(input, p + 12));
            p += 16;
        }
        h = v1.rotate_left(1).wrapping_add(v2.rotate_left(7)).wrapping_add(v3.rotate_left(12)).wrapping_add(v4.rotate_left(18));
    } else {
        h = seed.wrapping_add(P5);
    }
    h = h.wrapping_add(len as u32);
    while p + 4 <= len {
        h = h.wrapping_add(read_u32(input, p).wrapping_mul(P3)).rotate_left(17).wrapping_mul(P4);
        p += 4;
    }
    while p < len {
        h = h.wrapping_add((input[p] as u32).wrapping_mul(P5)).rotate_left(11).wrapping_mul(P1);
        p += 1;
    }
    h ^= h >> 15;
    h = h.wrapping_mul(P2);
    h ^= h >> 13;
    h = h.wrapping_mul(P3);
    h ^= h >> 16;
    h
}

/// Encodes an LZ4 frame: version 1, block independence, 64 KiB blocks, a content checksum, and a
/// stored block whenever compression does not make the chunk smaller. Blocks carry no checksum.
pub fn encode_frame(src: &[u8]) -> Vec<u8> {
    let flg: u8 = 0x40 | 0x20 | 0x04; // version 01, block independence, content checksum
    let bd: u8 = 0x40; // block maximum size id 4 (64 KiB)
    let mut out = Vec::with_capacity(src.len() / 2 + 32);
    out.extend_from_slice(&FRAME_MAGIC.to_le_bytes());
    out.push(flg);
    out.push(bd);
    out.push(header_checksum(flg, bd));
    for chunk in src.chunks(CHUNK) {
        let packed = encode_block(chunk);
        if packed.len() < chunk.len() {
            out.extend_from_slice(&(packed.len() as u32).to_le_bytes());
            out.extend_from_slice(&packed);
        } else {
            out.extend_from_slice(&((chunk.len() as u32) | 0x8000_0000).to_le_bytes());
            out.extend_from_slice(chunk);
        }
    }
    out.extend_from_slice(&0u32.to_le_bytes()); // end mark
    out.extend_from_slice(&xxh32(src, 0).to_le_bytes());
    out
}

fn header_checksum(flg: u8, bd: u8) -> u8 {
    ((xxh32(&[flg, bd], 0) >> 8) & 0xff) as u8
}

/// Decodes an LZ4 frame, refusing output larger than `limit`. Frames with a dictionary ID, a
/// reserved bit, an unknown version or block size, a bad header or block checksum, or trailing
/// bytes after the content checksum are `InvalidData`.
pub fn decode_frame(src: &[u8], limit: usize) -> Result<Vec<u8>, Error> {
    if src.len() < 7 {
        return Err(Error::Truncated);
    }
    if read_u32(src, 0) != FRAME_MAGIC {
        return Err(Error::InvalidData);
    }
    let flg = src[4];
    let bd = src[5];
    if flg >> 6 != 1 || flg & 0x03 != 0 || bd & 0x8f != 0 {
        return Err(Error::InvalidData);
    }
    let block_max = match (bd >> 4) & 0x07 {
        4 => 65536,
        5 => 262_144,
        6 => 1_048_576,
        7 => 4_194_304,
        _ => return Err(Error::InvalidData),
    };
    let content_size_flag = flg & 0x08 != 0;
    let block_checksum_flag = flg & 0x10 != 0;
    let content_checksum_flag = flg & 0x04 != 0;
    let mut pos = 6;
    if content_size_flag {
        pos += 8;
    }
    let hc = *src.get(pos).ok_or(Error::Truncated)?;
    if hc != header_checksum(flg, bd) {
        return Err(Error::InvalidData);
    }
    pos += 1;
    let mut out: Vec<u8> = Vec::new();
    loop {
        let raw = src.get(pos..pos + 4).ok_or(Error::Truncated)?;
        let word = u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]);
        pos += 4;
        if word == 0 {
            break;
        }
        let stored = word & 0x8000_0000 != 0;
        let size = (word & 0x7fff_ffff) as usize;
        if size > block_max {
            return Err(Error::InvalidData);
        }
        let data = src.get(pos..pos + size).ok_or(Error::Truncated)?;
        pos += size;
        if block_checksum_flag {
            let sum = src.get(pos..pos + 4).ok_or(Error::Truncated)?;
            if u32::from_le_bytes([sum[0], sum[1], sum[2], sum[3]]) != xxh32(data, 0) {
                return Err(Error::InvalidData);
            }
            pos += 4;
        }
        if stored {
            if out.len() + size > limit {
                return Err(Error::TooBig);
            }
            out.extend_from_slice(data);
        } else {
            let cap = limit.min(out.len() + block_max);
            decode_block_into(&mut out, data, cap)?;
        }
    }
    if content_checksum_flag {
        let sum = src.get(pos..pos + 4).ok_or(Error::Truncated)?;
        if u32::from_le_bytes([sum[0], sum[1], sum[2], sum[3]]) != xxh32(&out, 0) {
            return Err(Error::InvalidData);
        }
        pos += 4;
    }
    if pos != src.len() {
        return Err(Error::InvalidData);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xxh32_of_empty_input_with_seed_zero() {
        assert_eq!(xxh32(b"", 0), 0x02CC_5D05);
    }

    #[test]
    fn literal_only_short_input_is_one_token_and_the_bytes() {
        // under 13 bytes there is no match, so the block is a single literal sequence
        let enc = encode_block(b"abc");
        assert_eq!(enc, vec![0x30, b'a', b'b', b'c']);
        assert_eq!(decode_block(&enc, 16).unwrap(), b"abc".to_vec());
    }

    #[test]
    fn block_round_trip_repetitive_input_is_smaller() {
        let data = vec![b'a'; 1000];
        let enc = encode_block(&data);
        assert!(enc.len() < 40);
        assert_eq!(decode_block(&enc, 1000).unwrap(), data);
    }

    #[test]
    fn decoder_rejects_zero_and_out_of_range_offsets() {
        // token 0x10: one literal 'a', then a match with offset 0
        assert_eq!(decode_block(&[0x10, b'a', 0x00, 0x00], 100), Err(Error::InvalidData));
        // offset 5 with only one byte of output so far
        assert_eq!(decode_block(&[0x10, b'a', 0x05, 0x00], 100), Err(Error::InvalidData));
    }

    #[test]
    fn decoder_reports_truncation_and_honours_the_limit() {
        // literal run longer than the input
        assert_eq!(decode_block(&[0x50, b'a'], 100), Err(Error::Truncated));
        // a block that stops right after a match
        assert_eq!(decode_block(&[0x10, b'a', 0x01, 0x00], 100), Err(Error::Truncated));
        let data = vec![b'z'; 500];
        let enc = encode_block(&data);
        assert_eq!(decode_block(&enc, 100), Err(Error::TooBig));
    }

    #[test]
    fn a_buffer_block_may_match_into_the_previous_block() {
        // block 1 stores "abcdefgh"; block 2 is one match of 8 bytes at offset 8 plus 5 literals
        let mut src = Vec::new();
        src.extend_from_slice(BV4_STORED);
        src.extend_from_slice(&8u32.to_le_bytes());
        src.extend_from_slice(b"abcdefgh");
        let block = [0x04, 0x08, 0x00, 0x50, b'v', b'w', b'x', b'y', b'z'];
        src.extend_from_slice(BV4_COMPRESSED);
        src.extend_from_slice(&13u32.to_le_bytes());
        src.extend_from_slice(&(block.len() as u32).to_le_bytes());
        src.extend_from_slice(&block);
        src.extend_from_slice(BV4_END);
        assert_eq!(decode_buffer(&src, 100).unwrap(), b"abcdefghabcdefghvwxyz".to_vec());
    }

    #[test]
    fn length_extensions_round_trip() {
        // 300 literals then a 300-byte match, so both nibbles overflow into extension bytes
        let mut data: Vec<u8> = (0..300u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 24) as u8).collect();
        let copy = data.clone();
        data.extend_from_slice(&copy);
        let enc = encode_block(&data);
        assert_eq!(decode_block(&enc, data.len()).unwrap(), data);
    }

    fn mixed(n: usize) -> Vec<u8> {
        let mut state = 7u32;
        let mut data = Vec::with_capacity(n);
        while data.len() < n {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            if (state >> 28) < 8 {
                data.extend_from_slice(b"the same phrase again and again ");
            } else {
                data.push((state >> 24) as u8);
            }
        }
        data.truncate(n);
        data
    }

    #[test]
    fn the_incremental_encoder_gives_the_buffer_bytes_for_any_split() {
        let data = mixed(200_000);
        let whole = encode_buffer(&data);
        for piece in [1usize, 777, 65_536, 300_000] {
            let mut encoder = Bv4Encoder::new();
            let mut out = Vec::new();
            for chunk in data.chunks(piece) {
                out.extend(encoder.write(chunk));
            }
            out.extend(encoder.finish());
            assert_eq!(out, whole, "piece size {piece}");
        }
    }

    #[test]
    fn the_incremental_encoder_emits_a_block_once_64_kib_are_in() {
        let mut encoder = Bv4Encoder::new();
        assert!(encoder.write(&mixed(CHUNK - 1)).is_empty());
        assert!(!encoder.write(&[1]).is_empty());
    }

    #[test]
    fn a_flush_lets_the_decoder_reach_the_input_so_far() {
        let first = mixed(1000);
        let second = mixed(5000);
        let mut encoder = Bv4Encoder::new();
        let mut decoder = Bv4Decoder::new(1 << 20);
        encoder.write(&first);
        let flushed = encoder.flush();
        assert!(encoder.flush().is_empty(), "nothing pending, nothing written");
        assert_eq!(decoder.feed(&flushed).unwrap(), first);
        encoder.write(&second);
        let tail = encoder.finish();
        assert_eq!(decoder.feed(&tail).unwrap(), second);
        assert_eq!(decoder.finish().unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn the_incremental_decoder_takes_the_input_one_byte_at_a_time() {
        let data = mixed(150_000);
        let packed = encode_buffer(&data);
        let mut decoder = Bv4Decoder::new(data.len());
        let mut out = Vec::new();
        for b in &packed {
            out.extend(decoder.feed(std::slice::from_ref(b)).unwrap());
        }
        assert_eq!(out, data);
        decoder.finish().unwrap();
    }

    #[test]
    fn the_incremental_decoder_follows_matches_across_blocks() {
        // second block: 4 literals "wxyz", then a match of 8 at offset 12 that reaches into the first block
        let first = b"abcdefghijklmnop";
        let mut src = Vec::new();
        src.extend_from_slice(BV4_STORED);
        src.extend_from_slice(&(first.len() as u32).to_le_bytes());
        src.extend_from_slice(first);
        // token 0x44: 4 literals and a match of 8; then a closing token 0x50 with 5 literals
        let tail = [0x44, b'w', b'x', b'y', b'z', 0x0c, 0x00, 0x50, b'1', b'2', b'3', b'4', b'5'];
        src.extend_from_slice(BV4_COMPRESSED);
        src.extend_from_slice(&17u32.to_le_bytes());
        src.extend_from_slice(&(tail.len() as u32).to_le_bytes());
        src.extend_from_slice(&tail);
        src.extend_from_slice(BV4_END);
        let one_shot = decode_buffer(&src, 100).unwrap();
        assert_eq!(one_shot, b"abcdefghijklmnopwxyzijklmnop12345".to_vec());
        let mut decoder = Bv4Decoder::new(100);
        let mut out = decoder.feed(&src[..20]).unwrap();
        out.extend(decoder.feed(&src[20..]).unwrap());
        assert_eq!(out, one_shot);
    }

    #[test]
    fn the_incremental_decoder_refuses_hostile_input() {
        // a block size beyond the cap is refused before any wait for its bytes
        let mut big = Vec::new();
        big.extend_from_slice(BV4_STORED);
        big.extend_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(Bv4Decoder::new(usize::MAX).feed(&big), Err(Error::InvalidData));
        // output past the limit
        let packed = encode_buffer(&[9u8; 1000]);
        assert_eq!(Bv4Decoder::new(999).feed(&packed), Err(Error::TooBig));
        // an unknown magic
        assert_eq!(Bv4Decoder::new(10).feed(b"zzzzzzzz"), Err(Error::InvalidData));
        // bytes after the end marker
        let mut trailing = encode_buffer(b"abc");
        trailing.push(0);
        assert_eq!(Bv4Decoder::new(10).feed(&trailing), Err(Error::InvalidData));
        // a stream that ends before its end marker
        let mut cut = Bv4Decoder::new(10);
        cut.feed(&encode_buffer(b"abc")[..8]).unwrap();
        assert_eq!(cut.finish(), Err(Error::Truncated));
        // a header that lies about the decoded size
        let mut lie = Vec::new();
        let block = encode_block(&[5u8; 64]);
        lie.extend_from_slice(BV4_COMPRESSED);
        lie.extend_from_slice(&63u32.to_le_bytes());
        lie.extend_from_slice(&(block.len() as u32).to_le_bytes());
        lie.extend_from_slice(&block);
        assert_eq!(Bv4Decoder::new(1000).feed(&lie), Err(Error::InvalidData));
    }
}
