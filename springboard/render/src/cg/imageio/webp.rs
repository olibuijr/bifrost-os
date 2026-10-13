//! WebP: container and header reading for every variant (size, alpha, ICC, EXIF) and a full lossless (VP8L)
//! decoder. Lossy (VP8) and animated files report their size and properties but do not decode.

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
}
