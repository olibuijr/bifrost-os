//! WebP: container and header reading for every variant (size, alpha, ICC, EXIF, XMP, animation loop count and
//! canvas), our own lossless (VP8L) decoder for still lossless files, and the `image-webp` crate (MIT or Apache-2.0)
//! for lossy (VP8, with or without an ALPH plane) and animated files, whose frames it composites onto the canvas.
//! [`encode`] writes a lossless still picture with its EXIF block and ICC profile; animated and lossy writing are not
//! offered.

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

/// The TIFF-structured block of the `EXIF` chunk (an `Exif\0\0` prefix some writers add is skipped).
pub fn exif_chunk(data: &[u8]) -> Option<&[u8]> {
    let e = chunks(data)?.get(b"EXIF")?;
    Some(e.strip_prefix(b"Exif\0\0").unwrap_or(e))
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
    if let Some(a) = c.get(b"ANIM").filter(|a| a.len() >= 6) {
        // ANIM: background colour (4 bytes), then the loop count (0 repeats forever).
        d.insert("LoopCount".into(), PropValue::Int(u16::from_le_bytes([a[4], a[5]]) as i64));
        d.insert("CanvasPixelWidth".into(), PropValue::Int(w as i64));
        d.insert("CanvasPixelHeight".into(), PropValue::Int(h as i64));
    }
    if let Some(x) = c.get(b"XMP ") {
        p.xmp = Some(x.to_vec());
    }
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

/// The number of images: the animation frames of an animated file, otherwise one.
pub fn frame_count(data: &[u8]) -> usize {
    chunks(data).map_or(1, |c| c.list.iter().filter(|x| &x.0 == b"ANMF").count().max(1))
}

pub fn decode(data: &[u8]) -> Option<Decoded> {
    let c = chunks(data)?;
    let props = probe(data)?;
    if c.get(b"ANMF").is_some() || c.get(b"VP8L").is_none() {
        return decode_with_library(data, props);
    }
    let l = c.get(b"VP8L")?;
    let (w, h, px) = decode_vp8l(l)?;
    let mut rgba8 = Vec::with_capacity(w * h * 4);
    for p in px {
        rgba8.extend([(p >> 16) as u8, (p >> 8) as u8, p as u8, (p >> 24) as u8]);
    }
    Some(Decoded { frames: vec![Frame { w, h, rgba8, rgba16: None, delay_ms: 0 }], props: vec![props], loop_count: None })
}

/// Lossy and animated files, through `image-webp`. Animation frames come back composited onto the full canvas with
/// their display times; a file whose frames would hold more than `png::MAX_ANIMATION_PIXELS` pixels keeps the frames
/// that fit.
fn decode_with_library(data: &[u8], props: Properties) -> Option<Decoded> {
    use image_webp::{LoopCount, WebPDecoder};
    let mut dec = WebPDecoder::new(std::io::Cursor::new(data)).ok()?;
    let (w, h) = dec.dimensions();
    let (w, h) = (w as usize, h as usize);
    if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE {
        return None;
    }
    let alpha = dec.has_alpha();
    let mut buf = vec![0u8; dec.output_buffer_size()?];
    let rgba = |buf: &[u8]| -> Vec<u8> {
        if alpha {
            buf.to_vec()
        } else {
            buf.as_chunks::<3>().0.iter().flat_map(|c| [c[0], c[1], c[2], 255]).collect()
        }
    };
    if !dec.is_animated() {
        dec.read_image(&mut buf).ok()?;
        return Some(Decoded { frames: vec![Frame { w, h, rgba8: rgba(&buf), rgba16: None, delay_ms: 0 }], props: vec![props], loop_count: None });
    }
    let loop_count = match dec.loop_count() {
        LoopCount::Forever => 0,
        LoopCount::Times(n) => n.get() as u32,
    };
    let mut frames = Vec::new();
    for _ in 0..dec.num_frames() {
        if (frames.len() + 1).saturating_mul(w * h) > super::png::MAX_ANIMATION_PIXELS && !frames.is_empty() {
            break;
        }
        match dec.read_frame(&mut buf) {
            Ok(delay_ms) => frames.push(Frame { w, h, rgba8: rgba(&buf), rgba16: None, delay_ms }),
            Err(_) if !frames.is_empty() => break,
            Err(_) => return None,
        }
    }
    let props = vec![props; frames.len()];
    Some(Decoded { frames, props, loop_count: Some(loop_count) })
}

/// Writes one lossless WebP picture from straight RGBA (an opaque picture is stored without alpha). `exif` is a
/// TIFF-structured block (stored in the EXIF chunk without a prefix) and `icc` an ICC profile; either makes the file
/// an extended (VP8X) one. `None` for an empty or oversized picture or a pixel count that does not match.
pub fn encode(w: usize, h: usize, rgba: &[u8], exif: Option<&[u8]>, icc: Option<&[u8]>, xmp: Option<&[u8]>) -> Option<Vec<u8>> {
    use image_webp::{ColorType, WebPEncoder};
    if w == 0 || h == 0 || w > 16384 || h > 16384 || rgba.len() != w.checked_mul(h)?.checked_mul(4)? {
        return None;
    }
    let mut out = Vec::new();
    let mut enc = WebPEncoder::new(&mut out);
    if let Some(e) = exif.filter(|e| !e.is_empty()) {
        enc.set_exif_metadata(e.to_vec());
    }
    if let Some(p) = icc.filter(|p| !p.is_empty()) {
        enc.set_icc_profile(p.to_vec());
    }
    if let Some(x) = xmp.filter(|x| !x.is_empty()) {
        enc.set_xmp_metadata(x.to_vec());
    }
    if rgba.as_chunks::<4>().0.iter().all(|p| p[3] == 255) {
        let rgb: Vec<u8> = rgba.as_chunks::<4>().0.iter().flat_map(|p| [p[0], p[1], p[2]]).collect();
        enc.encode(&rgb, w as u32, h as u32, ColorType::Rgb8).ok()?;
    } else {
        enc.encode(rgba, w as u32, h as u32, ColorType::Rgba8).ok()?;
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

    fn gradient(w: usize, h: usize, alpha: u8) -> Vec<u8> {
        (0..w * h).flat_map(|i| [(i * 7) as u8, (i * 13) as u8, (i / w * 30) as u8, alpha]).collect()
    }

    #[test]
    fn the_lossless_writer_round_trips_through_our_own_decoder() {
        for alpha in [255u8, 128] {
            let px = gradient(9, 5, alpha);
            let file = encode(9, 5, &px, None, None, None).unwrap();
            let p = probe(&file).unwrap();
            assert_eq!((p.pixel_width, p.pixel_height), (9, 5));
            assert_eq!(p.has_alpha, alpha != 255, "an opaque picture is stored without alpha");
            assert_eq!(decode(&file).unwrap().frames[0].rgba8, px, "lossless: every pixel comes back");
        }
    }

    #[test]
    fn the_writer_keeps_exif_icc_and_xmp_in_an_extended_file() {
        let mut tiff = Dict::new();
        tiff.insert("Orientation".into(), PropValue::Int(6));
        let exif = crate::cg::imageio::exif::build_block(&tiff, &Dict::new(), &Dict::new()).unwrap();
        let file = encode(2, 2, &gradient(2, 2, 255), Some(&exif), Some(b"profile-bytes"), Some(b"<x:xmpmeta/>")).unwrap();
        assert!(chunks(&file).unwrap().get(b"VP8X").is_some());
        let p = probe(&file).unwrap();
        assert_eq!(p.orientation, 6);
        assert_eq!(p.icc_profile.as_deref(), Some(b"profile-bytes".as_slice()));
        assert_eq!(p.xmp.as_deref(), Some(b"<x:xmpmeta/>".as_slice()));
        assert_eq!(decode(&file).unwrap().frames[0].rgba8, gradient(2, 2, 255));
    }

    #[test]
    fn the_writer_refuses_a_wrong_pixel_count() {
        assert!(encode(2, 2, &[0; 15], None, None, None).is_none());
        assert!(encode(0, 2, &[], None, None, None).is_none());
    }

    #[test]
    fn a_still_file_counts_one_frame() {
        assert_eq!(frame_count(&encode(1, 1, &[1, 2, 3, 255], None, None, None).unwrap()), 1);
    }

    // Two pictures made for these tests from solid colours (8 x 6): a lossy red still, and a two-frame lossless
    // animation (red for 200 ms, not blended, then blue for 350 ms, alpha-blended over it; played three times).
    const LOSSY_RED_8X6: [u8; 72] = [
        82, 73, 70, 70, 64, 0, 0, 0, 87, 69, 66, 80, 86, 80, 56, 32, 52, 0, 0, 0, 48, 2, 0, 157,
        1, 42, 8, 0, 6, 0, 0, 192, 18, 37, 160, 2, 116, 186, 1, 248, 1, 248, 0, 4, 104, 0, 0, 254,
        250, 33, 151, 255, 119, 154, 176, 211, 119, 245, 173, 255, 245, 163, 159, 174, 137, 254, 180, 115, 255, 89, 88, 0,
    ];
    const ANIMATED_RED_BLUE_8X6: [u8; 148] = [
        82, 73, 70, 70, 140, 0, 0, 0, 87, 69, 66, 80, 86, 80, 56, 88, 10, 0, 0, 0, 2, 0, 0, 0,
        7, 0, 0, 5, 0, 0, 65, 78, 73, 77, 6, 0, 0, 0, 255, 255, 255, 255, 3, 0, 65, 78, 77, 70,
        44, 0, 0, 0, 0, 0, 0, 0, 0, 0, 7, 0, 0, 5, 0, 0, 200, 0, 0, 2, 86, 80, 56, 76,
        20, 0, 0, 0, 47, 7, 64, 1, 0, 7, 16, 253, 143, 254, 7, 0, 130, 240, 191, 109, 34, 162, 255, 33,
        65, 78, 77, 70, 44, 0, 0, 0, 0, 0, 0, 0, 0, 0, 7, 0, 0, 5, 0, 0, 94, 1, 0, 0,
        86, 80, 56, 76, 20, 0, 0, 0, 47, 7, 64, 1, 0, 7, 16, 209, 255, 254, 7, 0, 130, 240, 191, 109,
        34, 162, 255, 33,
    ];

    #[test]
    fn a_lossy_picture_decodes() {
        let p = probe(&LOSSY_RED_8X6).unwrap();
        assert_eq!((p.pixel_width, p.pixel_height, p.has_alpha), (8, 6, false));
        assert_eq!(p.webp["Lossless"], PropValue::Int(0));
        let d = decode(&LOSSY_RED_8X6).unwrap();
        assert_eq!((d.frames[0].w, d.frames[0].h, d.frames.len()), (8, 6, 1));
        let px = &d.frames[0].rgba8[(2 * 8 + 3) * 4..(2 * 8 + 3) * 4 + 4];
        assert!(px[0] > 200 && px[1] < 60 && px[2] < 60 && px[3] == 255, "red within lossy error: {px:?}");
    }

    #[test]
    fn an_animated_picture_gives_every_frame_its_delay_and_the_loop_count() {
        let p = probe(&ANIMATED_RED_BLUE_8X6).unwrap();
        assert_eq!(p.webp["Animated"], PropValue::Int(1));
        assert_eq!(p.webp["LoopCount"], PropValue::Int(3));
        assert_eq!(frame_count(&ANIMATED_RED_BLUE_8X6), 2);
        let d = decode(&ANIMATED_RED_BLUE_8X6).unwrap();
        assert_eq!(d.frames.iter().map(|f| f.delay_ms).collect::<Vec<_>>(), vec![200, 350]);
        assert_eq!(d.loop_count, Some(3));
        assert_eq!(d.props.len(), 2);
        assert_eq!(&d.frames[0].rgba8[..4], &[255, 0, 0, 255]);
        // The second frame is alpha-blended (its ANMF flags are 0). The blend is libwebp's integer one, which the
        // decoder follows: scale = 2^24 / alpha, so an opaque source can come out one level low (255 -> 254).
        let second = &d.frames[1].rgba8[..4];
        let want = [0u8, 0, 255, 255];
        assert!(
            second.iter().zip(want).all(|(got, want)| got.abs_diff(want) <= 1),
            "the opaque second frame covers the canvas: {second:?}"
        );
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
