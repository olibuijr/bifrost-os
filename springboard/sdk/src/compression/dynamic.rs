//! Dynamic-Huffman DEFLATE encoder (RFC 1951) behind the SDK compression API.
//!
//! The encoder finds matches with hash chains over a 32 KiB window, then writes each block with its own
//! literal/length and distance Huffman tables (block type 2). The chain depth is the compression level.
//! Fast parses greedily; Default and Best use lazy matching (a match is dropped when a longer one starts
//! one byte later). A block is stored (block type 0) when the Huffman form would be larger. `Encoder`
//! is the incremental form: it emits a block every `STREAM_BLOCK` bytes of input, and its output does
//! not depend on how the input was split into writes. Inputs of at least two split sizes can be encoded on several threads: each
//! piece becomes its own block and the blocks are joined bit by bit, so the result is one DEFLATE
//! stream. Matches do not cross the piece boundaries, which costs little on large inputs.
//!
//! Code lengths are limited to 15 bits (literal and distance codes) and 7 bits (code length codes) by
//! halving the symbol counts and rebuilding the tree until the depths fit.

use std::collections::BinaryHeap;
use std::cmp::Reverse;

/// How hard the matcher searches. A deeper search finds longer matches and runs slower.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    /// Few candidates per position; the fastest encoding.
    Fast,
    /// The default trade-off.
    Default,
    /// Long candidate chains; the smallest output this encoder makes.
    Best,
}

impl Level {
    fn chain_limit(self) -> usize {
        match self {
            Level::Fast => 8,
            Level::Default => 128,
            Level::Best => 2048,
        }
    }
}

const WINDOW: usize = 32768;
const MIN_MATCH: usize = 3;
const MAX_MATCH: usize = 258;
const HASH_BITS: u32 = 15;
const LIT_CODES: usize = 286;
const DIST_CODES: usize = 30;
const CL_CODES: usize = 19;
/// Inputs below this size are never split across threads.
const MIN_SPLIT: usize = 1 << 20;

const LENGTH_BASE: [u16; 29] = [3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258];
const LENGTH_EXTRA: [u8; 29] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
const DIST_BASE: [u16; 30] = [1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577];
const DIST_EXTRA: [u8; 30] = [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];
const CL_ORDER: [usize; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Token {
    Literal(u8),
    Match { len: u16, dist: u16 },
}

/// Raw DEFLATE of `data` on one thread.
pub fn encode(data: &[u8], level: Level) -> Vec<u8> {
    let mut encoder = Encoder::new(level);
    let mut out = encoder.write(data);
    out.extend(encoder.finish());
    out
}

/// Raw DEFLATE of `data`, using up to `threads` threads for inputs of at least two split sizes.
/// The output is one valid DEFLATE stream, but not byte-identical to `encode`.
pub fn encode_threaded(data: &[u8], level: Level, threads: usize) -> Vec<u8> {
    let parts = threads.max(1).min(data.len() / MIN_SPLIT);
    if parts <= 1 {
        return encode(data, level);
    }
    let chunk = (data.len() + parts - 1) / parts;
    let pieces: Vec<&[u8]> = data.chunks(chunk).collect();
    let last = pieces.len() - 1;
    let blocks: Vec<(Vec<u8>, usize)> = std::thread::scope(|scope| {
        let handles: Vec<_> = pieces
            .iter()
            .enumerate()
            .map(|(i, &piece)| scope.spawn(move || encode_block(piece, level, i == last)))
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap_or_else(|panic| std::panic::resume_unwind(panic)))
            .collect()
    });
    let mut w = BitWriter::new();
    for (bytes, bits) in &blocks {
        w.append(bytes, *bits);
    }
    w.finish().0
}

fn hash3(data: &[u8], i: usize) -> usize {
    let v = ((data[i] as u32) << 16) | ((data[i + 1] as u32) << 8) | data[i + 2] as u32;
    (v.wrapping_mul(2654435761) >> (32 - HASH_BITS)) as usize
}

fn common_len(data: &[u8], a: usize, b: usize, max: usize) -> usize {
    let mut n = 0;
    while n < max && data[a + n] == data[b + n] {
        n += 1;
    }
    n
}

/// Largest stored block (RFC 1951 section 3.2.4).
const STORED_MAX: usize = 65_535;

/// Input bytes per block in the streaming encoder. Equal to the largest stored block, so a block
/// that is stored is always one stored block, and the 5-byte stored header is paid once per block.
pub const STREAM_BLOCK: usize = STORED_MAX;

/// Hash chains over the input, with the search depth of a level. Positions are indexes into the
/// slice the caller passes in, so the streaming encoder keeps history across blocks.
#[derive(Debug)]
struct Matcher {
    head: Vec<usize>,
    prev: Vec<usize>,
    chain: usize,
    /// Lazy matching (Default and Best): a match is dropped for a literal when a longer match starts
    /// one byte later.
    lazy: bool,
}

impl Matcher {
    fn new(level: Level) -> Matcher {
        Matcher {
            head: vec![usize::MAX; 1 << HASH_BITS],
            prev: Vec::new(),
            chain: level.chain_limit(),
            lazy: level != Level::Fast,
        }
    }

    /// Longest match at `i` among the earlier positions in the chain, as (length, distance).
    fn find(&self, data: &[u8], i: usize) -> (usize, usize) {
        let n = data.len();
        if i + MIN_MATCH > n {
            return (0, 0);
        }
        let max_len = MAX_MATCH.min(n - i);
        let (mut best_len, mut best_dist) = (0, 0);
        let mut cand = self.head[hash3(data, i)];
        let mut chain = 0;
        while cand != usize::MAX && chain < self.chain {
            let dist = i - cand;
            if dist > WINDOW {
                break;
            }
            if best_len < max_len && data[cand + best_len] == data[i + best_len] {
                let len = common_len(data, cand, i, max_len);
                if len > best_len {
                    best_len = len;
                    best_dist = dist;
                    if len == max_len {
                        break;
                    }
                }
            }
            cand = self.prev[cand];
            chain += 1;
        }
        (best_len, best_dist)
    }

    fn insert(&mut self, data: &[u8], p: usize) {
        if p + MIN_MATCH <= data.len() {
            let h = hash3(data, p);
            self.prev[p] = self.head[h];
            self.head[h] = p;
        }
    }

    /// Tokens for `data[start..]`. Matches may reach back into `data[..start]` (the window).
    fn tokenize(&mut self, data: &[u8], start: usize) -> Vec<Token> {
        let n = data.len();
        self.prev.resize(n, usize::MAX);
        let mut tokens = Vec::with_capacity(n - start);
        let mut i = start;
        let mut cur = self.find(data, i);
        while i < n {
            let (len, dist) = cur;
            if self.lazy && len >= MIN_MATCH && i + 1 < n {
                let next = self.find(data, i + 1);
                if next.0 > len {
                    tokens.push(Token::Literal(data[i]));
                    self.insert(data, i);
                    i += 1;
                    cur = next;
                    continue;
                }
            }
            if len >= MIN_MATCH {
                tokens.push(Token::Match { len: len as u16, dist: dist as u16 });
                for p in i..i + len {
                    self.insert(data, p);
                }
                i += len;
            } else {
                tokens.push(Token::Literal(data[i]));
                self.insert(data, i);
                i += 1;
            }
            if i < n {
                cur = self.find(data, i);
            }
        }
        tokens
    }
}

/// Incremental raw DEFLATE (RFC 1951) encoder. `write` returns the bytes of each block as soon as
/// `STREAM_BLOCK` bytes of input are in, and `finish` closes the stream with a final block. Matches
/// reach back 32 KiB into earlier blocks, so the output does not depend on how the input was split
/// into writes.
#[derive(Debug)]
pub struct Encoder {
    matcher: Matcher,
    /// The window history followed by the block being collected.
    buf: Vec<u8>,
    /// Index in `buf` where the block being collected starts.
    start: usize,
    w: BitWriter,
}

impl Encoder {
    pub fn new(level: Level) -> Encoder {
        Encoder { matcher: Matcher::new(level), buf: Vec::new(), start: 0, w: BitWriter::new() }
    }

    /// Adds `data` and returns the bytes of every block that it completed.
    pub fn write(&mut self, mut data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        while !data.is_empty() {
            let room = STREAM_BLOCK - (self.buf.len() - self.start);
            let take = room.min(data.len());
            self.buf.extend_from_slice(&data[..take]);
            data = &data[take..];
            if self.buf.len() - self.start == STREAM_BLOCK {
                self.emit(false);
                out.append(&mut self.w.out);
            }
        }
        out
    }

    /// Closes the stream with a final block and returns the remaining bytes.
    pub fn finish(mut self) -> Vec<u8> {
        self.emit(true);
        self.w.finish().0
    }

    fn emit(&mut self, last: bool) {
        let tokens = self.matcher.tokenize(&self.buf, self.start);
        write_block(&mut self.w, &tokens, &self.buf[self.start..], last);
        self.slide();
    }

    /// Makes the end of the buffer the next block start, and drops history older than the window.
    fn slide(&mut self) {
        self.start = self.buf.len();
        let drop = self.buf.len().saturating_sub(WINDOW);
        if drop == 0 {
            return;
        }
        self.buf.drain(..drop);
        self.matcher.prev.drain(..drop);
        self.start -= drop;
        for p in self.matcher.prev.iter_mut().chain(self.matcher.head.iter_mut()) {
            *p = if *p != usize::MAX && *p >= drop { *p - drop } else { usize::MAX };
        }
    }
}

/// Writes one block of `data` (covered by `tokens`): dynamic Huffman, or stored when that is smaller.
/// The stored size ignores the up to 7 bits of byte padding it may need.
fn write_block(w: &mut BitWriter, tokens: &[Token], data: &[u8], last: bool) {
    let (bytes, bits) = write_dynamic(tokens, last);
    let stored_bits = 8 * (data.len() + 5 * data.len().div_ceil(STORED_MAX));
    if !data.is_empty() && bits > stored_bits {
        write_stored(w, data, last);
    } else {
        w.append(&bytes, bits);
    }
}

/// Writes `data` as stored blocks of at most 65535 bytes, flagging only the last one when `last` is set.
fn write_stored(w: &mut BitWriter, data: &[u8], last: bool) {
    let count = data.len().div_ceil(STORED_MAX);
    for (i, chunk) in data.chunks(STORED_MAX).enumerate() {
        w.put((last && i + 1 == count) as u32, 1);
        w.put(0, 2);
        w.align();
        let n = chunk.len() as u16;
        w.out.extend_from_slice(&n.to_le_bytes());
        w.out.extend_from_slice(&(!n).to_le_bytes());
        w.out.extend_from_slice(chunk);
    }
}

/// Length symbol index (0 to 28), extra value and extra bit count for a match length of 3 to 258.
fn length_code(len: u16) -> (usize, u32, u32) {
    let idx = LENGTH_BASE.iter().rposition(|&b| b <= len).unwrap_or(0);
    (idx, (len - LENGTH_BASE[idx]) as u32, LENGTH_EXTRA[idx] as u32)
}

/// Distance symbol (0 to 29), extra value and extra bit count for a distance of 1 to 32768.
fn dist_code(dist: u16) -> (usize, u32, u32) {
    let idx = DIST_BASE.iter().rposition(|&b| b <= dist).unwrap_or(0);
    (idx, (dist - DIST_BASE[idx]) as u32, DIST_EXTRA[idx] as u32)
}

/// Makes sure a tree has at least two symbols with a non-zero count, so its code is complete.
fn ensure_two(freq: &mut [u32]) {
    let mut used = freq.iter().filter(|&&f| f > 0).count();
    for f in freq.iter_mut() {
        if used >= 2 {
            break;
        }
        if *f == 0 {
            *f = 1;
            used += 1;
        }
    }
}

/// Depth of each symbol in a Huffman tree over `freq` (0 for symbols with no count).
fn huffman_depths(freq: &[u64]) -> Vec<u32> {
    let mut depth = vec![0u32; freq.len()];
    let leaves: Vec<usize> = (0..freq.len()).filter(|&s| freq[s] > 0).collect();
    match leaves.len() {
        0 => return depth,
        1 => {
            depth[leaves[0]] = 1;
            return depth;
        }
        _ => {}
    }
    let m = leaves.len();
    let mut weight: Vec<u64> = leaves.iter().map(|&s| freq[s]).collect();
    let mut children: Vec<(usize, usize)> = Vec::with_capacity(m - 1);
    let mut heap: BinaryHeap<Reverse<(u64, usize)>> = weight.iter().enumerate().map(|(i, &w)| Reverse((w, i))).collect();
    while heap.len() > 1 {
        let Reverse((wa, a)) = heap.pop().unwrap_or(Reverse((0, 0)));
        let Reverse((wb, b)) = heap.pop().unwrap_or(Reverse((0, 0)));
        let id = weight.len();
        weight.push(wa + wb);
        children.push((a, b));
        heap.push(Reverse((wa + wb, id)));
    }
    // Internal nodes have larger indices than their children, so walking down from the root works.
    let mut node_depth = vec![0u32; weight.len()];
    for k in (m..weight.len()).rev() {
        let (a, b) = children[k - m];
        node_depth[a] = node_depth[k] + 1;
        node_depth[b] = node_depth[k] + 1;
    }
    for (i, &s) in leaves.iter().enumerate() {
        depth[s] = node_depth[i];
    }
    depth
}

/// Code lengths for `freq`, none longer than `limit`. Counts are halved until the tree fits.
fn code_lengths(freq: &[u32], limit: u32) -> Vec<u8> {
    let mut counts: Vec<u64> = freq.iter().map(|&f| f as u64).collect();
    loop {
        let depth = huffman_depths(&counts);
        if depth.iter().all(|&d| d <= limit) {
            return depth.iter().map(|&d| d as u8).collect();
        }
        for c in counts.iter_mut() {
            if *c > 0 {
                *c = (*c >> 1).max(1);
            }
        }
    }
}

/// Canonical codes (RFC 1951 section 3.2.2) for the given lengths. Codes are not yet bit-reversed.
fn canonical_codes(lens: &[u8]) -> Vec<u16> {
    let mut bl_count = [0u32; 16];
    for &l in lens {
        bl_count[l as usize] += 1;
    }
    bl_count[0] = 0;
    let mut next = [0u32; 16];
    let mut code = 0u32;
    for bits in 1..16 {
        code = (code + bl_count[bits - 1]) << 1;
        next[bits] = code;
    }
    lens.iter()
        .map(|&l| {
            if l == 0 {
                0
            } else {
                let c = next[l as usize];
                next[l as usize] += 1;
                c as u16
            }
        })
        .collect()
}

/// Run-length codes for a list of code lengths: (symbol, extra value, extra bit count).
fn run_length_codes(lens: &[u8]) -> Vec<(usize, u32, u32)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < lens.len() {
        let v = lens[i];
        let mut run = 1;
        while i + run < lens.len() && lens[i + run] == v {
            run += 1;
        }
        let mut left = run;
        if v == 0 {
            while left >= 11 {
                let r = left.min(138);
                out.push((18, (r - 11) as u32, 7));
                left -= r;
            }
            if left >= 3 {
                let r = left.min(10);
                out.push((17, (r - 3) as u32, 3));
                left -= r;
            }
            for _ in 0..left {
                out.push((0, 0, 0));
            }
        } else {
            out.push((v as usize, 0, 0));
            left -= 1;
            while left >= 3 {
                let r = left.min(6);
                out.push((16, (r - 3) as u32, 2));
                left -= r;
            }
            for _ in 0..left {
                out.push((v as usize, 0, 0));
            }
        }
        i += run;
    }
    out
}

#[derive(Debug)]
struct BitWriter {
    out: Vec<u8>,
    acc: u64,
    n: u32,
}

impl BitWriter {
    fn new() -> Self {
        BitWriter { out: Vec::new(), acc: 0, n: 0 }
    }

    /// Writes the low `count` bits of `value`, least significant bit first.
    fn put(&mut self, value: u32, count: u32) {
        self.acc |= (value as u64) << self.n;
        self.n += count;
        while self.n >= 8 {
            self.out.push(self.acc as u8);
            self.acc >>= 8;
            self.n -= 8;
        }
    }

    /// Writes a Huffman code of `len` bits, most significant bit first.
    fn put_code(&mut self, code: u16, len: u8) {
        let len = len as u32;
        let reversed = (code as u32).reverse_bits() >> (32 - len);
        self.put(reversed, len);
    }

    /// Appends the first `bits` bits of `bytes`.
    fn append(&mut self, bytes: &[u8], bits: usize) {
        let full = bits / 8;
        for &b in &bytes[..full] {
            self.put(b as u32, 8);
        }
        let rest = (bits % 8) as u32;
        if rest > 0 {
            self.put(bytes[full] as u32 & ((1 << rest) - 1), rest);
        }
    }

    /// Pads the current byte with zero bits, so the next write starts on a byte boundary.
    fn align(&mut self) {
        if self.n > 0 {
            self.out.push(self.acc as u8);
            self.acc = 0;
            self.n = 0;
        }
    }

    /// The written bytes (the last one zero-padded) and the exact bit count.
    fn finish(mut self) -> (Vec<u8>, usize) {
        let bits = self.out.len() * 8 + self.n as usize;
        if self.n > 0 {
            self.out.push(self.acc as u8);
        }
        (self.out, bits)
    }
}

/// Writes one dynamic-Huffman block for `tokens`. Returns the bytes and the exact bit count.
fn write_dynamic(tokens: &[Token], last: bool) -> (Vec<u8>, usize) {
    let mut lit_freq = vec![0u32; LIT_CODES];
    let mut dist_freq = vec![0u32; DIST_CODES];
    for t in tokens {
        match *t {
            Token::Literal(b) => lit_freq[b as usize] += 1,
            Token::Match { len, dist } => {
                lit_freq[257 + length_code(len).0] += 1;
                dist_freq[dist_code(dist).0] += 1;
            }
        }
    }
    lit_freq[256] += 1;
    ensure_two(&mut lit_freq);
    ensure_two(&mut dist_freq);
    let lit_lens = code_lengths(&lit_freq, 15);
    let dist_lens = code_lengths(&dist_freq, 15);
    let lit_codes = canonical_codes(&lit_lens);
    let dist_codes = canonical_codes(&dist_lens);

    let hlit = (last_used(&lit_lens) + 1).max(257);
    let hdist = (last_used(&dist_lens) + 1).max(1);
    let mut runs = run_length_codes(&lit_lens[..hlit]);
    runs.extend(run_length_codes(&dist_lens[..hdist]));

    let mut cl_freq = vec![0u32; CL_CODES];
    for &(sym, _, _) in &runs {
        cl_freq[sym] += 1;
    }
    ensure_two(&mut cl_freq);
    let cl_lens = code_lengths(&cl_freq, 7);
    let cl_codes = canonical_codes(&cl_lens);
    let hclen = (CL_ORDER.iter().rposition(|&s| cl_lens[s] > 0).unwrap_or(0) + 1).max(4);

    let mut w = BitWriter::new();
    w.put(last as u32, 1);
    w.put(2, 2);
    w.put((hlit - 257) as u32, 5);
    w.put((hdist - 1) as u32, 5);
    w.put((hclen - 4) as u32, 4);
    for &s in &CL_ORDER[..hclen] {
        w.put(cl_lens[s] as u32, 3);
    }
    for &(sym, extra, extra_bits) in &runs {
        w.put_code(cl_codes[sym], cl_lens[sym]);
        if extra_bits > 0 {
            w.put(extra, extra_bits);
        }
    }
    for t in tokens {
        match *t {
            Token::Literal(b) => w.put_code(lit_codes[b as usize], lit_lens[b as usize]),
            Token::Match { len, dist } => {
                let (li, lextra, lbits) = length_code(len);
                w.put_code(lit_codes[257 + li], lit_lens[257 + li]);
                w.put(lextra, lbits);
                let (di, dextra, dbits) = dist_code(dist);
                w.put_code(dist_codes[di], dist_lens[di]);
                w.put(dextra, dbits);
            }
        }
    }
    w.put_code(lit_codes[256], lit_lens[256]);
    w.finish()
}

/// Writes `data` as one block (dynamic or stored, see `write_block`) on its own writer. Returns the
/// bytes and the exact bit count, for joining the pieces of a threaded encode.
fn encode_block(data: &[u8], level: Level, last: bool) -> (Vec<u8>, usize) {
    let mut matcher = Matcher::new(level);
    let tokens = matcher.tokenize(data, 0);
    let mut w = BitWriter::new();
    write_block(&mut w, &tokens, data, last);
    w.finish()
}

fn last_used(lens: &[u8]) -> usize {
    lens.iter().rposition(|&l| l > 0).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_render::cg::pdf::inflate;

    fn pseudo_random(len: usize, seed: u32) -> Vec<u8> {
        let mut s = seed;
        (0..len)
            .map(|_| {
                s = s.wrapping_mul(1664525).wrapping_add(1013904223);
                (s >> 24) as u8
            })
            .collect()
    }

    fn round_trip(data: &[u8], level: Level) {
        let enc = encode(data, level);
        assert_eq!(inflate::inflate(&enc).unwrap(), data.to_vec());
    }

    #[test]
    fn empty_input_round_trips() {
        round_trip(b"", Level::Default);
        round_trip(b"", Level::Best);
    }

    #[test]
    fn short_text_round_trips_at_every_level() {
        let text = b"the quick brown fox jumps over the lazy dog, the quick brown fox";
        for level in [Level::Fast, Level::Default, Level::Best] {
            round_trip(text, level);
        }
    }

    #[test]
    fn long_runs_and_random_bytes_round_trip() {
        round_trip(&vec![7u8; 5000], Level::Default);
        round_trip(&pseudo_random(20000, 1), Level::Fast);
        let mut mixed = pseudo_random(3000, 2);
        mixed.extend(std::iter::repeat(b'a').take(70000));
        mixed.extend(pseudo_random(3000, 2));
        round_trip(&mixed, Level::Best);
    }

    #[test]
    fn repeated_text_compresses_well() {
        let data = vec![b'a'; 10000];
        assert!(encode(&data, Level::Default).len() < 100);
    }

    #[test]
    fn length_and_distance_symbols_match_the_tables() {
        assert_eq!(length_code(3), (0, 0, 0));
        assert_eq!(length_code(257), (27, 30, 5));
        assert_eq!(length_code(258), (28, 0, 0));
        assert_eq!(dist_code(1), (0, 0, 0));
        assert_eq!(dist_code(32768), (29, 8191, 13));
    }

    #[test]
    fn code_lengths_respect_the_limit_and_stay_complete() {
        // Fibonacci counts give a deep tree, which must be pulled in to 7 bits.
        let mut freq = vec![0u32; 19];
        let (mut a, mut b) = (1u32, 1u32);
        for f in freq.iter_mut() {
            *f = a;
            let next = a + b;
            a = b;
            b = next;
        }
        let lens = code_lengths(&freq, 7);
        assert!(lens.iter().all(|&l| l <= 7));
        let kraft: u32 = lens.iter().filter(|&&l| l > 0).map(|&l| 1u32 << (7 - l)).sum();
        assert_eq!(kraft, 1 << 7);
    }

    #[test]
    fn canonical_codes_match_the_rfc_example() {
        // RFC 1951 section 3.2.2: lengths (3,3,3,3,3,2,4,4) give A=010 ... H=1111.
        let codes = canonical_codes(&[3, 3, 3, 3, 3, 2, 4, 4]);
        assert_eq!(codes, vec![2, 3, 4, 5, 6, 0, 14, 15]);
    }

    #[test]
    fn run_length_codes_use_the_repeat_symbols() {
        // Eleven zeros become symbol 18 with extra 0; a run of seven 5s becomes 5 then 16 repeating it six times.
        assert_eq!(run_length_codes(&[0; 11]), vec![(18, 0, 7)]);
        let codes = run_length_codes(&[5; 7]);
        assert_eq!(codes, vec![(5, 0, 0), (16, 3, 2)]);
    }

    #[test]
    fn threaded_encoding_of_a_large_input_round_trips() {
        let mut data = pseudo_random(MIN_SPLIT * 2 + 1234, 9);
        for chunk in data.chunks_mut(4096).step_by(3) {
            for b in chunk.iter_mut() {
                *b = b'x';
            }
        }
        let enc = encode_threaded(&data, Level::Fast, 3);
        assert_eq!(inflate::inflate(&enc).unwrap(), data);
    }

    #[test]
    fn lazy_levels_round_trip_and_are_deterministic() {
        let mut corpus: Vec<Vec<u8>> = vec![b"abcabcabcabcxabcabcabcy".to_vec(), vec![0u8; 3000], pseudo_random(5000, 4)];
        let mut text = Vec::new();
        for i in 0..400u32 {
            text.extend_from_slice(format!("line {} gamma kappa beta {}\n", i, i * i % 37).as_bytes());
        }
        corpus.push(text);
        for data in &corpus {
            for level in [Level::Default, Level::Best, Level::Fast] {
                let enc = encode(data, level);
                assert_eq!(inflate::inflate(&enc).unwrap(), *data);
                assert_eq!(encode(data, level), enc);
            }
        }
    }

    #[test]
    fn incompressible_input_is_stored_within_five_bytes_per_block() {
        let data = pseudo_random(200_000, 21);
        let enc = encode(&data, Level::Default);
        let blocks = data.len().div_ceil(STREAM_BLOCK);
        assert!(enc.len() <= data.len() + 5 * blocks, "{} bytes for {} input", enc.len(), data.len());
        assert_eq!(inflate::inflate(&enc).unwrap(), data);
    }

    #[test]
    fn streamed_output_does_not_depend_on_the_write_sizes() {
        let mut data = pseudo_random(STREAM_BLOCK + 500, 3);
        data.extend_from_slice(&b"repeat me ".repeat(12000));
        let whole = encode(&data, Level::Default);
        for piece in [1usize, 300, 65_536] {
            let mut encoder = Encoder::new(Level::Default);
            let mut out = Vec::new();
            for chunk in data.chunks(piece) {
                out.extend(encoder.write(chunk));
            }
            out.extend(encoder.finish());
            assert_eq!(out, whole, "piece size {}", piece);
        }
        assert_eq!(inflate::inflate(&whole).unwrap(), data);
    }

    #[test]
    fn threaded_encoding_of_a_small_input_is_the_single_thread_output() {
        let data = b"small input";
        assert_eq!(encode_threaded(data, Level::Default, 4), encode(data, Level::Default));
    }
}
