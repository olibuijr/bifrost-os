//! WebP: container and header reading for every variant (size, alpha, ICC, EXIF), a full lossless (VP8L)
//! decoder and a lossless encoder (predictor and subtract-green transforms, backward copies, Huffman codes). Lossy
//! (VP8) and animated files report their size and properties but do not decode; the encoder writes lossless only.

use super::exif::{Dict, PropValue, Properties};
use super::{Decoded, Frame, MAX_SIDE};

struct Chunks<'a> {
    list: Vec<([u8; 4], &'a [u8])>,
}

fn chunks(d: &[u8]) -> Option<Chunks<'_>> {
    if d.get(..4)? != b"RIFF" || d.get(8..12)? != b"WEBP" {
        return None;
    }
    let mut list = Vec::new();
    let mut p = 12;
    while p + 8 <= d.len() {
        let ty = [d[p], d[p + 1], d[p + 2], d[p + 3]];
        let len = u32::from_le_bytes([d[p + 4], d[p + 5], d[p + 6], d[p + 7]]) as usize;
        let end = (p + 8).saturating_add(len).min(d.len());
        list.push((ty, &d[p + 8..end]));
        p = end + (len & 1);
    }
    Some(Chunks { list })
}

impl<'a> Chunks<'a> {
    fn get(&self, ty: &[u8; 4]) -> Option<&'a [u8]> {
        self.list.iter().find(|c| &c.0 == ty).map(|c| c.1)
    }
}

fn u24(d: &[u8], o: usize) -> Option<usize> {
    let b = d.get(o..o + 3)?;
    Some(b[0] as usize | (b[1] as usize) << 8 | (b[2] as usize) << 16)
}

pub fn probe(data: &[u8]) -> Option<Properties> {
    let c = chunks(data)?;
    let (w, h, mut alpha) = if let Some(x) = c.get(b"VP8X") {
        (u24(x, 4)? + 1, u24(x, 7)? + 1, x.first()? & 0x10 != 0)
    } else if let Some(l) = c.get(b"VP8L") {
        if *l.first()? != 0x2f || l.len() < 5 {
            return None;
        }
        let bits = u32::from_le_bytes([l[1], l[2], l[3], l[4]]);
        ((bits & 0x3fff) as usize + 1, ((bits >> 14) & 0x3fff) as usize + 1, (bits >> 28) & 1 == 1)
    } else {
        let v = c.get(b"VP8 ")?;
        if v.get(3..6)? != [0x9d, 0x01, 0x2a] {
            return None;
        }
        ((u16::from_le_bytes([v[6], v[7]]) & 0x3fff) as usize, (u16::from_le_bytes([v[8], v[9]]) & 0x3fff) as usize, false)
    };
    alpha |= c.get(b"ALPH").is_some();
    let mut p = Properties::new(w, h);
    p.has_alpha = alpha;
    p.icc_profile = c.get(b"ICCP").map(|b| b.to_vec());
    if let Some(e) = c.get(b"EXIF") {
        p.absorb_exif(e.strip_prefix(b"Exif\0\0").unwrap_or(e));
    }
    let mut d = Dict::new();
    d.insert("Lossless".into(), PropValue::Int(c.get(b"VP8L").is_some() as i64));
    d.insert("Animated".into(), PropValue::Int(c.get(b"ANMF").is_some() as i64));
    p.webp = d;
    Some(p)
}

struct Br<'a> {
    d: &'a [u8],
    idx: usize,
    buf: u64,
    n: u32,
    eof: bool,
}

impl<'a> Br<'a> {
    fn new(d: &'a [u8]) -> Self {
        Br { d, idx: 0, buf: 0, n: 0, eof: false }
    }
    fn read(&mut self, bits: u32) -> u32 {
        if bits == 0 {
            return 0;
        }
        while self.n <= 56 && self.idx < self.d.len() {
            self.buf |= (self.d[self.idx] as u64) << self.n;
            self.idx += 1;
            self.n += 8;
        }
        if self.n < bits {
            self.eof = true;
            self.n = bits;
        }
        let v = (self.buf & ((1u64 << bits) - 1)) as u32;
        self.buf >>= bits;
        self.n -= bits;
        v
    }
}

struct Code {
    counts: [u16; 16],
    symbols: Vec<u16>,
    single: Option<u16>,
}

impl Code {
    fn build(lengths: &[u8]) -> Option<Code> {
        let mut counts = [0u16; 16];
        for &l in lengths {
            counts[l as usize] += 1;
        }
        let used: usize = counts[1..].iter().map(|&c| c as usize).sum();
        if used == 0 {
            return None;
        }
        if used == 1 {
            let s = lengths.iter().position(|&l| l != 0)? as u16;
            return Some(Code { counts, symbols: Vec::new(), single: Some(s) });
        }
        let mut left = 1i32;
        for &count in &counts[1..16] {
            left = (left << 1) - count as i32;
            if left < 0 {
                return None;
            }
        }
        if left != 0 {
            return None;
        }
        let mut symbols = Vec::with_capacity(used);
        for l in 1..16u8 {
            for (s, &len) in lengths.iter().enumerate() {
                if len == l {
                    symbols.push(s as u16);
                }
            }
        }
        Some(Code { counts, symbols, single: None })
    }

    fn decode(&self, br: &mut Br) -> u16 {
        if let Some(s) = self.single {
            return s;
        }
        let (mut code, mut first, mut index) = (0i32, 0i32, 0i32);
        for len in 1..16 {
            code |= br.read(1) as i32;
            let count = self.counts[len] as i32;
            if code - count < first {
                return self.symbols[(index + (code - first)) as usize];
            }
            index += count;
            first = (first + count) << 1;
            code <<= 1;
        }
        br.eof = true;
        0
    }
}

const CODE_LENGTH_ORDER: [usize; 19] = [17, 18, 0, 1, 2, 3, 4, 5, 16, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];

fn read_code(br: &mut Br, alphabet: usize) -> Option<Code> {
    if br.read(1) == 1 {
        let n = br.read(1) + 1;
        let first8 = br.read(1) == 1;
        let s0 = br.read(if first8 { 8 } else { 1 }) as usize;
        let mut lengths = vec![0u8; alphabet.max(256)];
        if s0 >= alphabet {
            return None;
        }
        lengths[s0] = 1;
        if n == 2 {
            let s1 = br.read(8) as usize;
            if s1 >= alphabet || s1 == s0 {
                return None;
            }
            lengths[s1] = 1;
        }
        return Code::build(&lengths[..alphabet]);
    }
    let n = br.read(4) as usize + 4;
    let mut cl = [0u8; 19];
    for &o in CODE_LENGTH_ORDER.iter().take(n) {
        cl[o] = br.read(3) as u8;
    }
    let clc = Code::build(&cl)?;
    let mut max_symbol = if br.read(1) == 1 {
        let nbits = 2 + 2 * br.read(3);
        2 + br.read(nbits) as usize
    } else {
        alphabet
    };
    let mut lengths = vec![0u8; alphabet];
    let (mut symbol, mut prev) = (0usize, 8u8);
    while symbol < alphabet {
        if max_symbol == 0 {
            break;
        }
        max_symbol -= 1;
        let l = clc.decode(br) as u8;
        if br.eof {
            return None;
        }
        if l < 16 {
            lengths[symbol] = l;
            symbol += 1;
            if l != 0 {
                prev = l;
            }
        } else {
            let slot = (l - 16) as usize;
            let repeat = br.read([2, 3, 7][slot]) as usize + [3, 3, 11][slot];
            if symbol + repeat > alphabet {
                return None;
            }
            let v = if l == 16 { prev } else { 0 };
            for k in 0..repeat {
                lengths[symbol + k] = v;
            }
            symbol += repeat;
        }
    }
    Code::build(&lengths)
}

const DIST_MAP: [(i8, i8); 120] = [
    (0, 1), (1, 0), (1, 1), (-1, 1), (0, 2), (2, 0), (1, 2), (-1, 2), (2, 1), (-2, 1), (2, 2), (-2, 2), (0, 3), (3, 0), (1, 3),
    (-1, 3), (3, 1), (-3, 1), (2, 3), (-2, 3), (3, 2), (-3, 2), (0, 4), (4, 0), (1, 4), (-1, 4), (4, 1), (-4, 1), (3, 3), (-3, 3),
    (2, 4), (-2, 4), (4, 2), (-4, 2), (0, 5), (3, 4), (-3, 4), (4, 3), (-4, 3), (5, 0), (1, 5), (-1, 5), (5, 1), (-5, 1), (2, 5),
    (-2, 5), (5, 2), (-5, 2), (4, 4), (-4, 4), (3, 5), (-3, 5), (5, 3), (-5, 3), (0, 6), (6, 0), (1, 6), (-1, 6), (6, 1), (-6, 1),
    (2, 6), (-2, 6), (6, 2), (-6, 2), (4, 5), (-4, 5), (5, 4), (-5, 4), (3, 6), (-3, 6), (6, 3), (-6, 3), (0, 7), (7, 0), (1, 7),
    (-1, 7), (5, 5), (-5, 5), (7, 1), (-7, 1), (4, 6), (-4, 6), (6, 4), (-6, 4), (2, 7), (-2, 7), (7, 2), (-7, 2), (3, 7), (-3, 7),
    (7, 3), (-7, 3), (5, 6), (-5, 6), (6, 5), (-6, 5), (8, 0), (4, 7), (-4, 7), (7, 4), (-7, 4), (8, 1), (8, 2), (6, 6), (-6, 6),
    (8, 3), (5, 7), (-5, 7), (7, 5), (-7, 5), (8, 4), (6, 7), (-6, 7), (7, 6), (-7, 6), (8, 5), (7, 7), (-7, 7), (8, 6), (8, 7),
];

fn prefix_value(br: &mut Br, sym: u32) -> u32 {
    if sym < 4 {
        return sym + 1;
    }
    let extra = (sym - 2) >> 1;
    let offset = (2 + (sym & 1)) << extra;
    offset + br.read(extra) + 1
}

fn div_ceil(a: usize, bits: u32) -> usize {
    (a + (1 << bits) - 1) >> bits
}

fn decode_stream(br: &mut Br, w: usize, h: usize, level0: bool) -> Option<Vec<u32>> {
    let mut cache_bits = 0;
    if br.read(1) == 1 {
        cache_bits = br.read(4);
        if !(1..=11).contains(&cache_bits) {
            return None;
        }
    }
    let mut meta: Option<(u32, Vec<u32>, usize)> = None;
    if level0 && br.read(1) == 1 {
        let bits = br.read(3) + 2;
        let mw = div_ceil(w, bits);
        let img = decode_stream(br, mw, div_ceil(h, bits), false)?;
        meta = Some((bits, img, mw));
    }
    let groups_n = match &meta {
        Some((_, img, _)) => img.iter().map(|p| (p >> 8) & 0xffff).max().unwrap_or(0) as usize + 1,
        None => 1,
    };
    if groups_n > 4096 {
        return None;
    }
    let cache_size = if cache_bits > 0 { 1usize << cache_bits } else { 0 };
    let sizes = [256 + 24 + cache_size, 256, 256, 256, 40];
    let mut groups = Vec::with_capacity(groups_n);
    for _ in 0..groups_n {
        let mut g = Vec::with_capacity(5);
        for &s in &sizes {
            g.push(read_code(br, s)?);
        }
        groups.push(g);
    }
    let total = w.checked_mul(h)?;
    let mut out: Vec<u32> = Vec::with_capacity(total);
    let mut cache = vec![0u32; cache_size];
    let insert = |cache: &mut Vec<u32>, argb: u32| {
        if cache_bits > 0 {
            cache[(0x1e35_a7bdu32.wrapping_mul(argb) >> (32 - cache_bits)) as usize] = argb;
        }
    };
    let mut group = 0usize;
    while out.len() < total {
        if br.eof {
            return None;
        }
        let pos = out.len();
        let (x, y) = (pos % w, pos / w);
        if let Some((bits, img, mw)) = &meta {
            if x & ((1 << bits) - 1) == 0 || pos == 0 {
                group = ((img[(y >> bits) * mw + (x >> bits)] >> 8) & 0xffff) as usize;
            }
        }
        let g = &groups[group];
        let s = g[0].decode(br) as usize;
        if s < 256 {
            let r = g[1].decode(br) as u32;
            let b = g[2].decode(br) as u32;
            let a = g[3].decode(br) as u32;
            let argb = a << 24 | r << 16 | (s as u32) << 8 | b;
            out.push(argb);
            insert(&mut cache, argb);
        } else if s < 280 {
            let len = prefix_value(br, s as u32 - 256) as usize;
            let ds = g[4].decode(br) as u32;
            let code = prefix_value(br, ds) as usize;
            let dist = if code > 120 {
                code - 120
            } else {
                let (dx, dy) = DIST_MAP[code - 1];
                (dx as i64 + dy as i64 * w as i64).max(1) as usize
            };
            if dist > pos || pos + len > total {
                return None;
            }
            for _ in 0..len {
                let v = out[out.len() - dist];
                out.push(v);
                insert(&mut cache, v);
            }
        } else {
            let idx = s - 280;
            if idx >= cache_size {
                return None;
            }
            let v = cache[idx];
            out.push(v);
            insert(&mut cache, v);
        }
    }
    if br.eof {
        return None;
    }
    Some(out)
}

fn add_px(a: u32, b: u32) -> u32 {
    let mut r = 0;
    for s in [0, 8, 16, 24] {
        r |= ((((a >> s) & 255) + ((b >> s) & 255)) & 255) << s;
    }
    r
}

fn avg2(a: u32, b: u32) -> u32 {
    let mut r = 0;
    for s in [0, 8, 16, 24] {
        r |= ((((a >> s) & 255) + ((b >> s) & 255)) / 2) << s;
    }
    r
}

fn clamp8(v: i32) -> u32 {
    v.clamp(0, 255) as u32
}

fn predict(mode: u32, l: u32, t: u32, tr: u32, tl: u32) -> u32 {
    match mode {
        0 => 0xff00_0000,
        1 => l,
        2 => t,
        3 => tr,
        4 => tl,
        5 => avg2(avg2(l, tr), t),
        6 => avg2(l, tl),
        7 => avg2(l, t),
        8 => avg2(tl, t),
        9 => avg2(t, tr),
        10 => avg2(avg2(l, tl), avg2(t, tr)),
        11 => {
            let (mut pl, mut pt) = (0i32, 0i32);
            for s in [0, 8, 16, 24] {
                let (lc, tc, tlc) = (((l >> s) & 255) as i32, ((t >> s) & 255) as i32, ((tl >> s) & 255) as i32);
                let p = lc + tc - tlc;
                pl += (p - lc).abs();
                pt += (p - tc).abs();
            }
            if pl < pt {
                l
            } else {
                t
            }
        }
        12 => {
            let mut r = 0;
            for s in [0, 8, 16, 24] {
                r |= clamp8(((l >> s) & 255) as i32 + ((t >> s) & 255) as i32 - ((tl >> s) & 255) as i32) << s;
            }
            r
        }
        13 => {
            let a = avg2(l, t);
            let mut r = 0;
            for s in [0, 8, 16, 24] {
                let ac = ((a >> s) & 255) as i32;
                let bc = ((tl >> s) & 255) as i32;
                r |= clamp8(ac + (ac - bc) / 2) << s;
            }
            r
        }
        _ => 0xff00_0000,
    }
}

enum Transform {
    Predictor { bits: u32, data: Vec<u32>, w: usize },
    Cross { bits: u32, data: Vec<u32>, w: usize },
    SubtractGreen,
    Palette { table: Vec<u32>, bits: u32 },
}

fn delta(t: u32, c: u32) -> i32 {
    ((t as u8 as i8 as i32) * (c as u8 as i8 as i32)) >> 5
}

fn inverse(t: &Transform, px: Vec<u32>, w: usize, h: usize, out_w: usize) -> Vec<u32> {
    match t {
        Transform::SubtractGreen => px
            .into_iter()
            .map(|p| {
                let g = (p >> 8) & 255;
                let r = (((p >> 16) & 255) + g) & 255;
                let b = ((p & 255) + g) & 255;
                (p & 0xff00_ff00) | r << 16 | b
            })
            .collect(),
        Transform::Cross { bits, data, w: tw } => {
            let mut out = px;
            for y in 0..h {
                for x in 0..w {
                    let e = data[(y >> bits) * tw + (x >> bits)];
                    let (g2r, g2b, r2b) = (e & 255, (e >> 8) & 255, (e >> 16) & 255);
                    let p = out[y * w + x];
                    let g = (p >> 8) & 255;
                    let mut r = (p >> 16) & 255;
                    let mut b = p & 255;
                    r = (r as i32 + delta(g2r, g)) as u32 & 255;
                    b = (b as i32 + delta(g2b, g)) as u32 & 255;
                    b = (b as i32 + delta(r2b, r)) as u32 & 255;
                    out[y * w + x] = (p & 0xff00_ff00) | r << 16 | b;
                }
            }
            out
        }
        Transform::Predictor { bits, data, w: tw } => {
            let mut out = px;
            for y in 0..h {
                for x in 0..w {
                    let i = y * w + x;
                    let pred = if y == 0 && x == 0 {
                        0xff00_0000
                    } else if y == 0 {
                        out[i - 1]
                    } else if x == 0 {
                        out[i - w]
                    } else {
                        let mode = (data[(y >> bits) * tw + (x >> bits)] >> 8) & 15;
                        predict(mode, out[i - 1], out[i - w], out[i - w + 1], out[i - w - 1])
                    };
                    out[i] = add_px(out[i], pred);
                }
            }
            out
        }
        Transform::Palette { table, bits } => {
            let per = 1usize << bits;
            let bpp = 8 >> bits;
            let mut out = vec![0u32; out_w * h];
            for y in 0..h {
                for x in 0..out_w {
                    let p = px[y * w + x / per];
                    let idx = if *bits == 0 { (p >> 8) & 255 } else { ((p >> 8) >> (bpp * (x & (per - 1)))) & ((1 << bpp) - 1) } as usize;
                    out[y * out_w + x] = table.get(idx).copied().unwrap_or(0);
                }
            }
            out
        }
    }
}

fn decode_vp8l(d: &[u8]) -> Option<(usize, usize, Vec<u32>)> {
    if *d.first()? != 0x2f {
        return None;
    }
    let mut br = Br::new(&d[1..]);
    let w = br.read(14) as usize + 1;
    let h = br.read(14) as usize + 1;
    let _alpha = br.read(1);
    if br.read(3) != 0 || w > MAX_SIDE || h > MAX_SIDE {
        return None;
    }
    let mut cur_w = w;
    let mut transforms: Vec<(Transform, usize)> = Vec::new();
    let mut seen = [false; 4];
    while br.read(1) == 1 {
        let ty = br.read(2) as usize;
        if seen[ty] {
            return None;
        }
        seen[ty] = true;
        let t = match ty {
            0 | 1 => {
                let bits = br.read(3) + 2;
                let tw = div_ceil(cur_w, bits);
                let data = decode_stream(&mut br, tw, div_ceil(h, bits), false)?;
                if ty == 0 {
                    Transform::Predictor { bits, data, w: tw }
                } else {
                    Transform::Cross { bits, data, w: tw }
                }
            }
            2 => Transform::SubtractGreen,
            _ => {
                let n = br.read(8) as usize + 1;
                let mut table = decode_stream(&mut br, n, 1, false)?;
                for i in 1..n {
                    table[i] = add_px(table[i], table[i - 1]);
                }
                let bits = match n {
                    0..=2 => 3,
                    3..=4 => 2,
                    5..=16 => 1,
                    _ => 0,
                };
                let before = cur_w;
                cur_w = div_ceil(cur_w, bits);
                transforms.push((Transform::Palette { table, bits }, before));
                continue;
            }
        };
        transforms.push((t, cur_w));
    }
    let mut px = decode_stream(&mut br, cur_w, h, true)?;
    let mut width = cur_w;
    for (t, out_w) in transforms.iter().rev() {
        px = inverse(t, px, width, h, *out_w);
        width = *out_w;
    }
    Some((w, h, px))
}

pub fn decode(data: &[u8]) -> Option<Decoded> {
    let c = chunks(data)?;
    let props = probe(data)?;
    let l = c.get(b"VP8L")?;
    let (w, h, px) = decode_vp8l(l)?;
    let mut rgba8 = Vec::with_capacity(w * h * 4);
    for p in px {
        rgba8.extend([(p >> 16) as u8, (p >> 8) as u8, p as u8, (p >> 24) as u8]);
    }
    Some(Decoded { frames: vec![Frame { w, h, rgba8, rgba16: None, delay_ms: 0 }], props: vec![props], loop_count: None })
}

// ---------------------------------------------------------------------------------------------------------------
// lossless encoder

/// The largest side a lossless file can hold (14 bits).
const MAX_ENCODE_SIDE: usize = 1 << 14;
/// Predictor blocks are 2^PREDICTOR_BITS pixels square.
const PREDICTOR_BITS: u32 = 4;
/// The longest backward copy the length prefix codes reach.
const MAX_COPY: usize = 4096;
/// The farthest plain distance whose code (distance + 120) stays inside the 40-symbol distance alphabet.
const MAX_DIST: usize = (1 << 20) - 120;

/// LSB-first bit writer.
struct Bw {
    out: Vec<u8>,
    acc: u64,
    n: u32,
}

impl Bw {
    fn new() -> Bw {
        Bw { out: Vec::new(), acc: 0, n: 0 }
    }

    fn put(&mut self, v: u32, bits: u32) {
        if bits == 0 {
            return;
        }
        self.acc |= (v as u64 & ((1u64 << bits) - 1)) << self.n;
        self.n += bits;
        while self.n >= 8 {
            self.out.push(self.acc as u8);
            self.acc >>= 8;
            self.n -= 8;
        }
    }

    fn finish(mut self) -> Vec<u8> {
        if self.n > 0 {
            self.out.push(self.acc as u8);
        }
        self.out
    }
}

/// Huffman code lengths for `freq`, none longer than `limit`. One used symbol gets length 1 (the decoder reads it
/// with no bits); a code with two or more symbols is complete.
fn huffman_lengths(freq: &[u32], limit: u32) -> Vec<u8> {
    use std::cmp::Reverse;
    use std::collections::BinaryHeap;
    let mut lens = vec![0u8; freq.len()];
    let used: Vec<usize> = (0..freq.len()).filter(|&i| freq[i] > 0).collect();
    match used.len() {
        0 => return lens,
        1 => {
            lens[used[0]] = 1;
            return lens;
        }
        _ => {}
    }
    let n = used.len();
    let mut w: Vec<u64> = used.iter().map(|&i| freq[i] as u64).collect();
    loop {
        let mut parent = vec![usize::MAX; 2 * n - 1];
        let mut heap = BinaryHeap::with_capacity(n);
        for (k, &wk) in w.iter().enumerate() {
            heap.push(Reverse((wk, k)));
        }
        let mut next = n;
        while heap.len() > 1 {
            let (Some(Reverse((wa, a))), Some(Reverse((wb, b)))) = (heap.pop(), heap.pop()) else { break };
            parent[a] = next;
            parent[b] = next;
            heap.push(Reverse((wa + wb, next)));
            next += 1;
        }
        // a parent is always numbered after its children, so walking down from the root fills every depth
        let mut depth = vec![0u32; next];
        for k in (0..next - 1).rev() {
            depth[k] = depth[parent[k]] + 1;
        }
        if depth[..n].iter().all(|&d| d <= limit) {
            for (k, &i) in used.iter().enumerate() {
                lens[i] = depth[k] as u8;
            }
            return lens;
        }
        for x in &mut w {
            *x = (*x >> 1).max(1);
        }
    }
}

/// A prefix code ready for writing: per symbol its code and length. A code with one symbol writes no bits.
struct Hc {
    codes: Vec<u16>,
    lens: Vec<u8>,
    single: bool,
}

impl Hc {
    fn new(lens: Vec<u8>) -> Hc {
        let mut count = [0u16; 16];
        for &l in &lens {
            if l > 0 {
                count[l as usize] += 1;
            }
        }
        let mut next = [0u16; 16];
        let mut code = 0u16;
        for l in 1..16 {
            code = (code + count[l - 1]) << 1;
            next[l] = code;
        }
        let codes = lens
            .iter()
            .map(|&l| {
                if l == 0 {
                    0
                } else {
                    let c = next[l as usize];
                    next[l as usize] += 1;
                    c
                }
            })
            .collect();
        let single = lens.iter().filter(|&&l| l > 0).count() <= 1;
        Hc { codes, lens, single }
    }

    /// Writes `sym`, most significant code bit first (the order the decoder reads them in).
    fn put(&self, bw: &mut Bw, sym: usize) {
        if self.single {
            return;
        }
        let (code, len) = (self.codes[sym], self.lens[sym] as u32);
        for i in (0..len).rev() {
            bw.put((code >> i) as u32 & 1, 1);
        }
    }
}

/// Writes the code lengths `lens` of one alphabet as a simple code when that is possible, else as a normal code.
fn write_lengths(bw: &mut Bw, lens: &[u8]) {
    let used: Vec<usize> = (0..lens.len()).filter(|&i| lens[i] > 0).collect();
    if used.is_empty() {
        // one symbol, 0, read with no bits
        bw.put(1, 1);
        bw.put(0, 1);
        bw.put(0, 1);
        bw.put(0, 1);
        return;
    }
    if used.len() <= 2 && used.iter().all(|&s| s < 256) {
        bw.put(1, 1);
        bw.put(used.len() as u32 - 1, 1);
        if used[0] < 2 {
            bw.put(0, 1);
            bw.put(used[0] as u32, 1);
        } else {
            bw.put(1, 1);
            bw.put(used[0] as u32, 8);
        }
        if used.len() == 2 {
            bw.put(used[1] as u32, 8);
        }
        return;
    }
    bw.put(0, 1);
    // code length symbols: 0..=15 literally, 17 for 3..=10 zeros, 18 for 11..=138 zeros
    let mut toks: Vec<(usize, u32, u32)> = Vec::new();
    let mut i = 0;
    while i < lens.len() {
        if lens[i] == 0 {
            let mut run = 1;
            while i + run < lens.len() && lens[i + run] == 0 && run < 138 {
                run += 1;
            }
            if run >= 11 {
                toks.push((18, 7, run as u32 - 11));
                i += run;
                continue;
            }
            if run >= 3 {
                toks.push((17, 3, run as u32 - 3));
                i += run;
                continue;
            }
        }
        toks.push((lens[i] as usize, 0, 0));
        i += 1;
    }
    let mut freq = [0u32; 19];
    for t in &toks {
        freq[t.0] += 1;
    }
    let cl = huffman_lengths(&freq, 7);
    let n = CODE_LENGTH_ORDER.iter().rposition(|&o| cl[o] != 0).map_or(0, |k| k + 1).max(4);
    bw.put(n as u32 - 4, 4);
    for &o in CODE_LENGTH_ORDER.iter().take(n) {
        bw.put(cl[o] as u32, 3);
    }
    bw.put(0, 1); // the lengths cover the whole alphabet
    let hc = Hc::new(cl);
    for (sym, bits, extra) in toks {
        hc.put(bw, sym);
        bw.put(extra, bits);
    }
}

/// The prefix symbol, extra bit count and extra bits of a length or distance value (1 or more).
fn prefix_code(v: u32) -> (usize, u32, u32) {
    let d = v - 1;
    if d < 4 {
        return (d as usize, 0, 0);
    }
    let h = 31 - d.leading_zeros();
    let second = (d >> (h - 1)) & 1;
    (2 * h as usize + second as usize, h - 1, d & ((1 << (h - 1)) - 1))
}

enum Tok {
    Lit(u32),
    Copy { len: usize, code: u32 },
}

/// Literal pixels and backward copies for `px`, a `w` pixel wide image: candidates are the pixel to the left, the
/// pixel above and the last position with the same colour.
fn tokens(px: &[u32], w: usize) -> Vec<Tok> {
    const HASH_BITS: u32 = 16;
    let mut last = vec![usize::MAX; 1 << HASH_BITS];
    let hash = |p: u32| (p.wrapping_mul(0x1e35_a7bd) >> (32 - HASH_BITS)) as usize;
    let mut out = Vec::new();
    let mut i = 0;
    while i < px.len() {
        let limit = (px.len() - i).min(MAX_COPY);
        let mut best = (0usize, 0usize);
        let try_dist = |dist: usize, best: &mut (usize, usize)| {
            if dist == 0 || dist > i || dist > MAX_DIST {
                return;
            }
            let mut len = 0;
            while len < limit && px[i + len] == px[i + len - dist] {
                len += 1;
            }
            if len > best.0 {
                *best = (len, dist);
            }
        };
        try_dist(1, &mut best);
        try_dist(w, &mut best);
        let h = hash(px[i]);
        if last[h] != usize::MAX {
            try_dist(i - last[h], &mut best);
        }
        let (len, dist) = best;
        if len >= 3 {
            let code = if dist == w {
                1
            } else if dist == 1 {
                2
            } else {
                dist as u32 + 120
            };
            out.push(Tok::Copy { len, code });
            for k in i..i + len {
                last[hash(px[k])] = k;
            }
            i += len;
        } else {
            out.push(Tok::Lit(px[i]));
            last[h] = i;
            i += 1;
        }
    }
    out
}

/// Writes one entropy-coded image: no colour cache, no meta prefix codes (`level0` adds that bit), one group of
/// five codes built from the tokens' frequencies, then the tokens.
fn write_stream(bw: &mut Bw, px: &[u32], w: usize, level0: bool) {
    bw.put(0, 1); // no colour cache
    if level0 {
        bw.put(0, 1); // no meta prefix codes
    }
    let toks = tokens(px, w);
    let mut green = vec![0u32; 256 + 24];
    let mut red = vec![0u32; 256];
    let mut blue = vec![0u32; 256];
    let mut alpha = vec![0u32; 256];
    let mut dist = vec![0u32; 40];
    for t in &toks {
        match *t {
            Tok::Lit(p) => {
                green[(p >> 8 & 255) as usize] += 1;
                red[(p >> 16 & 255) as usize] += 1;
                blue[(p & 255) as usize] += 1;
                alpha[(p >> 24) as usize] += 1;
            }
            Tok::Copy { len, code } => {
                green[256 + prefix_code(len as u32).0] += 1;
                dist[prefix_code(code).0] += 1;
            }
        }
    }
    let codes: Vec<Hc> = [green, red, blue, alpha, dist]
        .iter()
        .map(|f| {
            let lens = huffman_lengths(f, 15);
            write_lengths(bw, &lens);
            Hc::new(lens)
        })
        .collect();
    for t in &toks {
        match *t {
            Tok::Lit(p) => {
                codes[0].put(bw, (p >> 8 & 255) as usize);
                codes[1].put(bw, (p >> 16 & 255) as usize);
                codes[2].put(bw, (p & 255) as usize);
                codes[3].put(bw, (p >> 24) as usize);
            }
            Tok::Copy { len, code } => {
                let (s, bits, extra) = prefix_code(len as u32);
                codes[0].put(bw, 256 + s);
                bw.put(extra, bits);
                let (s, bits, extra) = prefix_code(code);
                codes[4].put(bw, s);
                bw.put(extra, bits);
            }
        }
    }
}

fn sub_px(a: u32, b: u32) -> u32 {
    let mut r = 0;
    for s in [0, 8, 16, 24] {
        r |= (((a >> s) & 255).wrapping_sub((b >> s) & 255) & 255) << s;
    }
    r
}

/// How costly a residual is to code: the sum of its channels read as signed bytes.
fn residual_cost(r: u32) -> u32 {
    [0u32, 8, 16, 24].iter().map(|&s| ((r >> s) as u8 as i8 as i32).unsigned_abs()).sum()
}

/// The predictor residuals of `px` (`w` x `h`) and the mode chosen for each block: per block the mode whose residuals
/// cost least. The edges follow the decoder: the first pixel predicts opaque black, the first row the pixel to the
/// left and the first column the pixel above.
fn predict_forward(px: &[u32], w: usize, h: usize) -> (Vec<u32>, Vec<u32>, usize) {
    let bw = div_ceil(w, PREDICTOR_BITS);
    let bh = div_ceil(h, PREDICTOR_BITS);
    let pred_at = |mode: u32, x: usize, y: usize| -> u32 {
        let i = y * w + x;
        if y == 0 && x == 0 {
            0xff00_0000
        } else if y == 0 {
            px[i - 1]
        } else if x == 0 {
            px[i - w]
        } else {
            predict(mode, px[i - 1], px[i - w], px[i - w + 1], px[i - w - 1])
        }
    };
    let side = 1usize << PREDICTOR_BITS;
    let mut modes = vec![0u32; bw * bh];
    for by in 0..bh {
        for bx in 0..bw {
            let (x0, y0) = (bx * side, by * side);
            let (x1, y1) = ((x0 + side).min(w), (y0 + side).min(h));
            let mut best = (u64::MAX, 1u32);
            for mode in 0..14u32 {
                let mut cost = 0u64;
                for y in y0..y1 {
                    for x in x0..x1 {
                        cost += residual_cost(sub_px(px[y * w + x], pred_at(mode, x, y))) as u64;
                    }
                }
                if cost < best.0 {
                    best = (cost, mode);
                }
            }
            modes[by * bw + bx] = best.1;
        }
    }
    let mut res = vec![0u32; w * h];
    for y in 0..h {
        for x in 0..w {
            let mode = modes[(y >> PREDICTOR_BITS) * bw + (x >> PREDICTOR_BITS)];
            res[y * w + x] = sub_px(px[y * w + x], pred_at(mode, x, y));
        }
    }
    let sub: Vec<u32> = modes.iter().map(|&m| 0xff00_0000 | m << 8).collect();
    (res, sub, bw)
}

/// The VP8L bitstream for straight RGBA pixels: a predictor transform, the subtract-green transform, then the
/// entropy-coded residuals.
fn encode_vp8l(w: usize, h: usize, rgba: &[u8]) -> Vec<u8> {
    let px: Vec<u32> = rgba
        .as_chunks::<4>()
        .0
        .iter()
        .take(w * h)
        .map(|p| (p[3] as u32) << 24 | (p[0] as u32) << 16 | (p[1] as u32) << 8 | p[2] as u32)
        .collect();
    let alpha_used = px.iter().any(|p| p >> 24 != 255);
    let mut bw = Bw::new();
    bw.put(0x2f, 8);
    bw.put(w as u32 - 1, 14);
    bw.put(h as u32 - 1, 14);
    bw.put(alpha_used as u32, 1);
    bw.put(0, 3);
    // predictor
    let (res, modes, mw) = predict_forward(&px, w, h);
    bw.put(1, 1);
    bw.put(0, 2);
    bw.put(PREDICTOR_BITS - 2, 3);
    write_stream(&mut bw, &modes, mw, false);
    // subtract green
    bw.put(1, 1);
    bw.put(2, 2);
    let res: Vec<u32> = res
        .into_iter()
        .map(|p| {
            let g = (p >> 8) & 255;
            let r = (((p >> 16) & 255).wrapping_sub(g)) & 255;
            let b = ((p & 255).wrapping_sub(g)) & 255;
            (p & 0xff00_ff00) | r << 16 | b
        })
        .collect();
    bw.put(0, 1); // no more transforms
    write_stream(&mut bw, &res, w, true);
    bw.finish()
}

/// A lossless WebP file (a RIFF container with one `VP8L` chunk) for straight RGBA pixels, rows top to bottom.
/// `None` for an empty image, too few bytes or a side above 16384. Colour profiles and metadata are not written.
pub fn encode(w: usize, h: usize, rgba: &[u8]) -> Option<Vec<u8>> {
    if w == 0 || h == 0 || w > MAX_ENCODE_SIDE || h > MAX_ENCODE_SIDE || rgba.len() < w.checked_mul(h)?.checked_mul(4)? {
        return None;
    }
    let body = encode_vp8l(w, h, rgba);
    let mut out = b"RIFF".to_vec();
    let padded = body.len() + (body.len() & 1);
    out.extend(((4 + 8 + padded) as u32).to_le_bytes());
    out.extend(b"WEBPVP8L");
    out.extend((body.len() as u32).to_le_bytes());
    out.extend(&body);
    if body.len() & 1 == 1 {
        out.push(0);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distance_table_is_complete() {
        assert_eq!(DIST_MAP.len(), 120);
        assert_eq!(DIST_MAP[0], (0, 1));
        assert_eq!(DIST_MAP[119], (8, 7));
    }

    fn riff(chunk: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut o = b"RIFF".to_vec();
        o.extend(((body.len() + 12) as u32).to_le_bytes());
        o.extend(b"WEBP");
        o.extend(chunk);
        o.extend((body.len() as u32).to_le_bytes());
        o.extend(body);
        if body.len() % 2 == 1 {
            o.push(0);
        }
        o
    }

    /// A 1x1 lossless image written by hand: no transforms, no cache, simple one-symbol codes.
    fn tiny() -> Vec<u8> {
        // bit writer, LSB first
        let mut bits: Vec<u8> = Vec::new();
        let mut put = |v: u32, n: u32| {
            for i in 0..n {
                bits.push(((v >> i) & 1) as u8);
            }
        };
        put(0x2f, 8);
        put(0, 14); // width - 1
        put(0, 14); // height - 1
        put(1, 1); // alpha used
        put(0, 3); // version
        put(0, 1); // no transform
        put(0, 1); // no colour cache
        put(0, 1); // no meta prefix
        // five simple codes with one 8-bit symbol each: green=0x80, red=0x10, blue=0x20, alpha=0xff, dist=0
        for sym in [0x80u32, 0x10, 0x20, 0xff] {
            put(1, 1);
            put(0, 1); // one symbol
            put(1, 1); // 8-bit symbol
            put(sym, 8);
        }
        put(1, 1);
        put(0, 1);
        put(0, 1); // 1-bit symbol 0
        put(0, 1);
        let mut bytes = Vec::new();
        for c in bits.chunks(8) {
            bytes.push(c.iter().enumerate().fold(0u8, |a, (i, &b)| a | b << i));
        }
        bytes
    }

    #[test]
    fn hand_built_lossless_image_decodes() {
        let file = riff(b"VP8L", &tiny());
        let p = probe(&file).unwrap();
        assert_eq!((p.pixel_width, p.pixel_height), (1, 1));
        assert!(p.has_alpha);
        let d = decode(&file).unwrap();
        assert_eq!(d.frames[0].rgba8, vec![0x10, 0x80, 0x20, 0xff]);
    }

    #[test]
    fn lossy_header_is_probed_but_not_decoded() {
        let mut v = vec![0, 0, 0, 0x9d, 0x01, 0x2a];
        v.extend(10u16.to_le_bytes());
        v.extend(7u16.to_le_bytes());
        let file = riff(b"VP8 ", &v);
        let p = probe(&file).unwrap();
        assert_eq!((p.pixel_width, p.pixel_height), (10, 7));
        assert!(decode(&file).is_none());
    }

    #[test]
    fn garbage_is_refused() {
        assert!(probe(b"RIFF....WEBX").is_none());
        assert!(decode(&riff(b"VP8L", &[0x2f, 0, 0])).is_none());
    }

    #[test]
    fn huffman_codes_are_canonical() {
        let c = Code::build(&[1, 2, 2]).unwrap();
        let mut br = Br::new(&[0b0000_0110]);
        // bits LSB first: 0 -> symbol 0; 1,1 -> symbol 2 (code 11);
        assert_eq!(c.decode(&mut br), 0);
        assert_eq!(c.decode(&mut br), 2);
        assert!(Code::build(&[1, 1, 1]).is_none(), "over-subscribed");
    }

    fn gradient_rgba(w: usize, h: usize) -> Vec<u8> {
        let mut v = Vec::with_capacity(w * h * 4);
        for y in 0..h {
            for x in 0..w {
                v.extend([(x * 255 / w.max(1)) as u8, (y * 255 / h.max(1)) as u8, ((x ^ y) & 255) as u8, if (x + y) % 7 == 0 { 128 } else { 255 }]);
            }
        }
        v
    }

    #[test]
    fn the_lossless_encoder_round_trips_exactly() {
        for (w, h) in [(1, 1), (2, 1), (1, 3), (17, 9), (64, 40)] {
            let rgba = gradient_rgba(w, h);
            let file = encode(w, h, &rgba).unwrap();
            let p = probe(&file).unwrap();
            assert_eq!((p.pixel_width, p.pixel_height), (w, h));
            let d = decode(&file).unwrap_or_else(|| panic!("{w}x{h} decodes"));
            assert_eq!(d.frames[0].rgba8, rgba, "{w}x{h}");
        }
    }

    #[test]
    fn flat_and_repeating_images_compress_through_copies() {
        let (w, h) = (128, 128);
        let mut rgba = Vec::with_capacity(w * h * 4);
        for i in 0..w * h {
            rgba.extend(if (i / 5) % 2 == 0 { [10, 200, 30, 255] } else { [250, 250, 250, 255] });
        }
        let file = encode(w, h, &rgba).unwrap();
        assert!(file.len() < w * h, "{} bytes for {} raw", file.len(), w * h * 4);
        assert_eq!(decode(&file).unwrap().frames[0].rgba8, rgba);
        let p = probe(&file).unwrap();
        assert!(!p.has_alpha, "an opaque image does not claim alpha");
    }

    #[test]
    fn the_encoder_refuses_what_the_format_cannot_hold() {
        assert!(encode(0, 1, &[]).is_none());
        assert!(encode(2, 2, &[0; 8]).is_none(), "too few bytes");
        assert!(encode(MAX_ENCODE_SIDE + 1, 1, &vec![0; (MAX_ENCODE_SIDE + 1) * 4]).is_none());
    }

    #[test]
    fn length_limited_codes_stay_complete() {
        // Fibonacci-like frequencies force a deep tree; the limit must hold and the code stay decodable
        let mut freq = vec![0u32; 40];
        let (mut a, mut b) = (1u32, 1u32);
        for f in freq.iter_mut().take(30) {
            *f = a;
            let c = a.saturating_add(b);
            a = b;
            b = c;
        }
        let lens = huffman_lengths(&freq, 15);
        assert!(lens.iter().all(|&l| l <= 15));
        assert!(Code::build(&lens).is_some(), "complete");
        assert_eq!(prefix_code(1), (0, 0, 0));
        assert_eq!(prefix_code(5), (4, 1, 0));
        assert_eq!(prefix_code(4096).0, 23);
    }
}
