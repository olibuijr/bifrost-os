//! A small baseline JPEG encoder (pure Rust): 8-bit, YCbCr 4:2:0, the standard
//! Annex K quantisation and Huffman tables, a quality setting and an optional
//! EXIF block (orientation + capture date). Camera captures are written with it.

const ZIGZAG: [usize; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20, 13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59,
    52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];

/// Annex K luminance table at quality 50, natural (row-major) order.
const LUMA_Q: [u16; 64] = [
    16, 11, 10, 16, 24, 40, 51, 61, 12, 12, 14, 19, 26, 58, 60, 55, 14, 13, 16, 24, 40, 57, 69, 56, 14, 17, 22, 29, 51, 87, 80, 62, 18, 22, 37, 56, 68, 109, 103, 77, 24, 35, 55, 64, 81, 104, 113,
    92, 49, 64, 78, 87, 103, 121, 120, 101, 72, 92, 95, 98, 112, 100, 103, 99,
];

/// Annex K chrominance table at quality 50, natural order.
const CHROMA_Q: [u16; 64] = [
    17, 18, 24, 47, 99, 99, 99, 99, 18, 21, 26, 66, 99, 99, 99, 99, 24, 26, 56, 99, 99, 99, 99, 99, 47, 66, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99,
    99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99,
];

pub const DC_LUMA_BITS: [u8; 16] = [0, 1, 5, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0];
pub const DC_CHROMA_BITS: [u8; 16] = [0, 3, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0];
pub const DC_VALS: [u8; 12] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];

pub const AC_LUMA_BITS: [u8; 16] = [0, 2, 1, 3, 3, 2, 4, 3, 5, 5, 4, 4, 0, 0, 1, 0x7d];
pub const AC_LUMA_VALS: [u8; 162] = [
    0x01, 0x02, 0x03, 0x00, 0x04, 0x11, 0x05, 0x12, 0x21, 0x31, 0x41, 0x06, 0x13, 0x51, 0x61, 0x07, 0x22, 0x71, 0x14, 0x32, 0x81, 0x91, 0xA1, 0x08, 0x23, 0x42, 0xB1, 0xC1, 0x15, 0x52, 0xD1,
    0xF0, 0x24, 0x33, 0x62, 0x72, 0x82, 0x09, 0x0A, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x25, 0x26, 0x27, 0x28, 0x29, 0x2A, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3A, 0x43, 0x44, 0x45, 0x46, 0x47,
    0x48, 0x49, 0x4A, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5A, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6A, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7A, 0x83, 0x84, 0x85, 0x86,
    0x87, 0x88, 0x89, 0x8A, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9A, 0xA2, 0xA3, 0xA4, 0xA5, 0xA6, 0xA7, 0xA8, 0xA9, 0xAA, 0xB2, 0xB3, 0xB4, 0xB5, 0xB6, 0xB7, 0xB8, 0xB9, 0xBA,
    0xC2, 0xC3, 0xC4, 0xC5, 0xC6, 0xC7, 0xC8, 0xC9, 0xCA, 0xD2, 0xD3, 0xD4, 0xD5, 0xD6, 0xD7, 0xD8, 0xD9, 0xDA, 0xE1, 0xE2, 0xE3, 0xE4, 0xE5, 0xE6, 0xE7, 0xE8, 0xE9, 0xEA, 0xF1, 0xF2, 0xF3,
    0xF4, 0xF5, 0xF6, 0xF7, 0xF8, 0xF9, 0xFA,
];

pub const AC_CHROMA_BITS: [u8; 16] = [0, 2, 1, 2, 4, 4, 3, 4, 7, 5, 4, 4, 0, 1, 2, 0x77];
pub const AC_CHROMA_VALS: [u8; 162] = [
    0x00, 0x01, 0x02, 0x03, 0x11, 0x04, 0x05, 0x21, 0x31, 0x06, 0x12, 0x41, 0x51, 0x07, 0x61, 0x71, 0x13, 0x22, 0x32, 0x81, 0x08, 0x14, 0x42, 0x91, 0xA1, 0xB1, 0xC1, 0x09, 0x23, 0x33, 0x52,
    0xF0, 0x15, 0x62, 0x72, 0xD1, 0x0A, 0x16, 0x24, 0x34, 0xE1, 0x25, 0xF1, 0x17, 0x18, 0x19, 0x1A, 0x26, 0x27, 0x28, 0x29, 0x2A, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3A, 0x43, 0x44, 0x45, 0x46,
    0x47, 0x48, 0x49, 0x4A, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5A, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6A, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7A, 0x82, 0x83, 0x84,
    0x85, 0x86, 0x87, 0x88, 0x89, 0x8A, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9A, 0xA2, 0xA3, 0xA4, 0xA5, 0xA6, 0xA7, 0xA8, 0xA9, 0xAA, 0xB2, 0xB3, 0xB4, 0xB5, 0xB6, 0xB7, 0xB8,
    0xB9, 0xBA, 0xC2, 0xC3, 0xC4, 0xC5, 0xC6, 0xC7, 0xC8, 0xC9, 0xCA, 0xD2, 0xD3, 0xD4, 0xD5, 0xD6, 0xD7, 0xD8, 0xD9, 0xDA, 0xE2, 0xE3, 0xE4, 0xE5, 0xE6, 0xE7, 0xE8, 0xE9, 0xEA, 0xF2, 0xF3,
    0xF4, 0xF5, 0xF6, 0xF7, 0xF8, 0xF9, 0xFA,
];

/// Metadata written as an EXIF APP1 segment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Exif {
    /// EXIF orientation 1..=8 (1 = as stored)
    pub orientation: u16,
    /// "YYYY:MM:DD HH:MM:SS"; anything else is padded or cut to 19 characters
    pub date: String,
}

/// The quantisation table for `quality` (1..=100), natural order, entries 1..=255.
pub fn scaled_table(base: &[u16; 64], quality: u8) -> [u16; 64] {
    let q = quality.clamp(1, 100) as u32;
    let scale = if q < 50 { 5000 / q } else { 200 - 2 * q };
    let mut out = [0u16; 64];
    for (o, &b) in out.iter_mut().zip(base) {
        *o = ((b as u32 * scale + 50) / 100).clamp(1, 255) as u16;
    }
    out
}

/// `(code, length)` for every symbol of a Huffman table given as Annex C bits/values.
fn huffman_codes(bits: &[u8; 16], vals: &[u8]) -> [(u16, u8); 256] {
    let mut table = [(0u16, 0u8); 256];
    let mut code = 0u16;
    let mut k = 0;
    for (len, &count) in bits.iter().enumerate() {
        for _ in 0..count {
            table[vals[k] as usize] = (code, len as u8 + 1);
            code += 1;
            k += 1;
        }
        code <<= 1;
    }
    table
}

struct BitWriter {
    out: Vec<u8>,
    acc: u64,
    n: u32,
}

impl BitWriter {
    fn put(&mut self, code: u32, len: u32) {
        if len == 0 {
            return;
        }
        self.acc = (self.acc << len) | (code as u64 & ((1u64 << len) - 1));
        self.n += len;
        while self.n >= 8 {
            let byte = (self.acc >> (self.n - 8)) as u8;
            self.out.push(byte);
            if byte == 0xFF {
                self.out.push(0);
            }
            self.n -= 8;
        }
        self.acc &= (1u64 << self.n) - 1;
    }

    fn finish(mut self) -> Vec<u8> {
        if self.n > 0 {
            let pad = 8 - self.n;
            self.put((1 << pad) - 1, pad);
        }
        self.out
    }
}

/// The number of bits needed for |v| and the bits to write for v.
fn category(v: i32) -> (u32, u32) {
    let a = v.unsigned_abs();
    let size = 32 - a.leading_zeros();
    let bits = if v >= 0 { v as u32 } else { (v - 1) as u32 } & ((1u32 << size) - 1);
    (size, bits)
}

struct Dct {
    /// c[u][x] = a(u) cos((2x+1) u pi / 16)
    c: [[f32; 8]; 8],
}

impl Dct {
    fn new() -> Self {
        let mut c = [[0f32; 8]; 8];
        for (u, row) in c.iter_mut().enumerate() {
            let a = if u == 0 { (1.0f32 / 8.0).sqrt() } else { 0.5 };
            for (x, v) in row.iter_mut().enumerate() {
                *v = a * ((2 * x + 1) as f32 * u as f32 * std::f32::consts::PI / 16.0).cos();
            }
        }
        Dct { c }
    }

    /// Forward 8x8 DCT of level-shifted samples.
    fn forward(&self, block: &[f32; 64]) -> [f32; 64] {
        let mut tmp = [0f32; 64];
        for y in 0..8 {
            for u in 0..8 {
                let mut s = 0.0;
                for x in 0..8 {
                    s += block[y * 8 + x] * self.c[u][x];
                }
                tmp[y * 8 + u] = s;
            }
        }
        let mut out = [0f32; 64];
        for u in 0..8 {
            for v in 0..8 {
                let mut s = 0.0;
                for y in 0..8 {
                    s += tmp[y * 8 + u] * self.c[v][y];
                }
                out[v * 8 + u] = s;
            }
        }
        out
    }
}

struct Plane {
    w: usize,
    data: Vec<f32>,
}

struct Coder<'a> {
    dct: Dct,
    bw: BitWriter,
    dc_luma: [(u16, u8); 256],
    dc_chroma: [(u16, u8); 256],
    ac_luma: [(u16, u8); 256],
    ac_chroma: [(u16, u8); 256],
    luma_q: &'a [u16; 64],
    chroma_q: &'a [u16; 64],
}

impl Coder<'_> {
    /// Codes the 8x8 block at (bx, by) of `plane`; returns the new DC predictor.
    fn block(&mut self, plane: &Plane, bx: usize, by: usize, chroma: bool, pred: i32) -> i32 {
        let mut b = [0f32; 64];
        for y in 0..8 {
            for x in 0..8 {
                b[y * 8 + x] = plane.data[(by * 8 + y) * plane.w + bx * 8 + x] - 128.0;
            }
        }
        let f = self.dct.forward(&b);
        let q = if chroma { self.chroma_q } else { self.luma_q };
        let mut z = [0i32; 64];
        for (i, &nat) in ZIGZAG.iter().enumerate() {
            z[i] = (f[nat] / q[nat] as f32).round() as i32;
        }
        let (dc_t, ac_t) = if chroma { (self.dc_chroma, self.ac_chroma) } else { (self.dc_luma, self.ac_luma) };
        let diff = z[0] - pred;
        let (size, bits) = category(diff);
        let (code, len) = dc_t[size as usize];
        self.bw.put(code as u32, len as u32);
        self.bw.put(bits, size);
        let mut run = 0;
        for &v in &z[1..] {
            if v == 0 {
                run += 1;
                continue;
            }
            while run > 15 {
                let (code, len) = ac_t[0xF0];
                self.bw.put(code as u32, len as u32);
                run -= 16;
            }
            let (size, bits) = category(v);
            let (code, len) = ac_t[(run << 4) | size as usize];
            self.bw.put(code as u32, len as u32);
            self.bw.put(bits, size);
            run = 0;
        }
        if run > 0 {
            let (code, len) = ac_t[0];
            self.bw.put(code as u32, len as u32);
        }
        z[0]
    }
}

fn segment(out: &mut Vec<u8>, marker: u8, body: &[u8]) {
    out.extend_from_slice(&[0xFF, marker]);
    out.extend_from_slice(&((body.len() + 2) as u16).to_be_bytes());
    out.extend_from_slice(body);
}

fn dqt(out: &mut Vec<u8>, id: u8, table: &[u16; 64]) {
    let mut body = vec![id];
    body.extend(ZIGZAG.iter().map(|&n| table[n] as u8));
    segment(out, 0xDB, &body);
}

fn dht(out: &mut Vec<u8>, class_id: u8, bits: &[u8; 16], vals: &[u8]) {
    let mut body = vec![class_id];
    body.extend_from_slice(bits);
    body.extend_from_slice(vals);
    segment(out, 0xC4, &body);
}

/// The APP1 body: "Exif\0\0" + a big-endian TIFF with Orientation, DateTime and an
/// Exif IFD carrying DateTimeOriginal / DateTimeDigitized.
pub fn exif_block(e: &Exif) -> Vec<u8> {
    let mut date: Vec<u8> = e.date.bytes().filter(|b| b.is_ascii() && *b != 0).take(19).collect();
    date.resize(19, b' ');
    date.push(0);
    let mut t: Vec<u8> = Vec::new();
    t.extend_from_slice(b"MM\x00\x2A");
    t.extend_from_slice(&8u32.to_be_bytes());
    // IFD0 at 8: 3 entries -> 2 + 36 + 4 = 42 bytes, date text at 50, Exif IFD at 70
    t.extend_from_slice(&3u16.to_be_bytes());
    let entry = |tag: u16, typ: u16, count: u32, value: [u8; 4]| {
        let mut v = Vec::with_capacity(12);
        v.extend_from_slice(&tag.to_be_bytes());
        v.extend_from_slice(&typ.to_be_bytes());
        v.extend_from_slice(&count.to_be_bytes());
        v.extend_from_slice(&value);
        v
    };
    let o = e.orientation.clamp(1, 8);
    t.extend(entry(0x0112, 3, 1, [(o >> 8) as u8, o as u8, 0, 0]));
    t.extend(entry(0x0132, 2, 20, 50u32.to_be_bytes()));
    t.extend(entry(0x8769, 4, 1, 70u32.to_be_bytes()));
    t.extend_from_slice(&0u32.to_be_bytes());
    t.extend_from_slice(&date);
    // Exif IFD at 70
    t.extend_from_slice(&2u16.to_be_bytes());
    t.extend(entry(0x9003, 2, 20, 50u32.to_be_bytes()));
    t.extend(entry(0x9004, 2, 20, 50u32.to_be_bytes()));
    t.extend_from_slice(&0u32.to_be_bytes());
    let mut body = b"Exif\0\0".to_vec();
    body.extend(t);
    body
}

/// Encodes `w` x `h` RGB (3 bytes per pixel, row-major) as a baseline JPEG.
/// `quality` is 1..=100. Returns an empty vector for an empty or inconsistent image.
pub fn encode_rgb(w: usize, h: usize, rgb: &[u8], quality: u8, exif: Option<&Exif>) -> Vec<u8> {
    if w == 0 || h == 0 || w > 65_535 || h > 65_535 || rgb.len() < w * h * 3 {
        return Vec::new();
    }
    let luma_q = scaled_table(&LUMA_Q, quality);
    let chroma_q = scaled_table(&CHROMA_Q, quality);

    let mut out = vec![0xFF, 0xD8];
    segment(&mut out, 0xE0, &[b'J', b'F', b'I', b'F', 0, 1, 1, 0, 0, 1, 0, 1, 0, 0]);
    if let Some(e) = exif {
        segment(&mut out, 0xE1, &exif_block(e));
    }
    dqt(&mut out, 0, &luma_q);
    dqt(&mut out, 1, &chroma_q);
    let mut sof = vec![8];
    sof.extend_from_slice(&(h as u16).to_be_bytes());
    sof.extend_from_slice(&(w as u16).to_be_bytes());
    sof.extend_from_slice(&[3, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1]);
    segment(&mut out, 0xC0, &sof);
    dht(&mut out, 0x00, &DC_LUMA_BITS, &DC_VALS);
    dht(&mut out, 0x10, &AC_LUMA_BITS, &AC_LUMA_VALS);
    dht(&mut out, 0x01, &DC_CHROMA_BITS, &DC_VALS);
    dht(&mut out, 0x11, &AC_CHROMA_BITS, &AC_CHROMA_VALS);
    segment(&mut out, 0xDA, &[3, 1, 0x00, 2, 0x11, 3, 0x11, 0, 63, 0]);

    // planes padded to whole 16x16 MCUs by replicating the edge
    let (mw, mh) = (w.div_ceil(16), h.div_ceil(16));
    let (pw, ph) = (mw * 16, mh * 16);
    let mut y_plane = Plane { w: pw, data: vec![0.0; pw * ph] };
    let (cw, ch) = (pw / 2, ph / 2);
    let mut cb_plane = Plane { w: cw, data: vec![0.0; cw * ch] };
    let mut cr_plane = Plane { w: cw, data: vec![0.0; cw * ch] };
    let (mut cb_full, mut cr_full) = (vec![0f32; pw * ph], vec![0f32; pw * ph]);
    for py in 0..ph {
        let sy = py.min(h - 1);
        for px in 0..pw {
            let i = (sy * w + px.min(w - 1)) * 3;
            let (r, g, b) = (rgb[i] as f32, rgb[i + 1] as f32, rgb[i + 2] as f32);
            y_plane.data[py * pw + px] = 0.299 * r + 0.587 * g + 0.114 * b;
            cb_full[py * pw + px] = 128.0 - 0.168_736 * r - 0.331_264 * g + 0.5 * b;
            cr_full[py * pw + px] = 128.0 + 0.5 * r - 0.418_688 * g - 0.081_312 * b;
        }
    }
    for cy in 0..ch {
        for cx in 0..cw {
            let at = |x: usize, y: usize, p: &Vec<f32>| p[y * pw + x];
            let (x, y) = (cx * 2, cy * 2);
            cb_plane.data[cy * cw + cx] = (at(x, y, &cb_full) + at(x + 1, y, &cb_full) + at(x, y + 1, &cb_full) + at(x + 1, y + 1, &cb_full)) / 4.0;
            cr_plane.data[cy * cw + cx] = (at(x, y, &cr_full) + at(x + 1, y, &cr_full) + at(x, y + 1, &cr_full) + at(x + 1, y + 1, &cr_full)) / 4.0;
        }
    }

    let mut coder = Coder {
        dct: Dct::new(),
        bw: BitWriter { out: Vec::new(), acc: 0, n: 0 },
        dc_luma: huffman_codes(&DC_LUMA_BITS, &DC_VALS),
        dc_chroma: huffman_codes(&DC_CHROMA_BITS, &DC_VALS),
        ac_luma: huffman_codes(&AC_LUMA_BITS, &AC_LUMA_VALS),
        ac_chroma: huffman_codes(&AC_CHROMA_BITS, &AC_CHROMA_VALS),
        luma_q: &luma_q,
        chroma_q: &chroma_q,
    };
    let (mut pred_y, mut pred_cb, mut pred_cr) = (0, 0, 0);
    for my in 0..mh {
        for mx in 0..mw {
            for (dy, dx) in [(0, 0), (0, 1), (1, 0), (1, 1)] {
                pred_y = coder.block(&y_plane, mx * 2 + dx, my * 2 + dy, false, pred_y);
            }
            pred_cb = coder.block(&cb_plane, mx, my, true, pred_cb);
            pred_cr = coder.block(&cr_plane, mx, my, true, pred_cr);
        }
    }
    out.extend(coder.bw.finish());
    out.extend_from_slice(&[0xFF, 0xD9]);
    out
}

/// Encodes packed RGBA pixels (`gfx` layout: R in the low byte) as a JPEG.
pub fn encode_pixels(w: usize, h: usize, px: &[u32], quality: u8, exif: Option<&Exif>) -> Vec<u8> {
    if px.len() < w * h {
        return Vec::new();
    }
    let mut rgb = Vec::with_capacity(w * h * 3);
    for &p in &px[..w * h] {
        rgb.extend_from_slice(&[p as u8, (p >> 8) as u8, (p >> 16) as u8]);
    }
    encode_rgb(w, h, &rgb, quality, exif)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gradient(w: usize, h: usize) -> Vec<u8> {
        let mut v = Vec::with_capacity(w * h * 3);
        for y in 0..h {
            for x in 0..w {
                v.extend_from_slice(&[(x * 255 / w.max(1)) as u8, (y * 255 / h.max(1)) as u8, ((x + y) * 255 / (w + h)) as u8]);
            }
        }
        v
    }

    /// (marker, body) of every segment up to and including SOS.
    fn segments(jpg: &[u8]) -> Vec<(u8, Vec<u8>)> {
        let mut out = Vec::new();
        let mut i = 2;
        while i + 4 <= jpg.len() {
            assert_eq!(jpg[i], 0xFF, "marker at {i}");
            let m = jpg[i + 1];
            let len = u16::from_be_bytes([jpg[i + 2], jpg[i + 3]]) as usize;
            out.push((m, jpg[i + 4..i + 2 + len].to_vec()));
            if m == 0xDA {
                break;
            }
            i += 2 + len;
        }
        out
    }

    fn decode(jpg: &[u8]) -> (usize, usize, Vec<u8>) {
        use zune_core::colorspace::ColorSpace;
        use zune_core::options::DecoderOptions;
        let opts = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::RGB);
        let mut d = zune_jpeg::JpegDecoder::new_with_options(std::io::Cursor::new(jpg), opts);
        let data = d.decode().expect("decodes");
        let info = d.info().unwrap();
        (info.width as usize, info.height as usize, data)
    }

    fn mean_abs_error(a: &[u8], b: &[u8]) -> f64 {
        a.iter().zip(b).map(|(&x, &y)| (x as f64 - y as f64).abs()).sum::<f64>() / a.len() as f64
    }

    #[test]
    fn huffman_tables_are_consistent() {
        assert_eq!(AC_LUMA_BITS.iter().map(|&b| b as usize).sum::<usize>(), AC_LUMA_VALS.len());
        assert_eq!(AC_CHROMA_BITS.iter().map(|&b| b as usize).sum::<usize>(), AC_CHROMA_VALS.len());
        assert_eq!(DC_LUMA_BITS.iter().map(|&b| b as usize).sum::<usize>(), DC_VALS.len());
        assert_eq!(DC_CHROMA_BITS.iter().map(|&b| b as usize).sum::<usize>(), DC_VALS.len());
        for vals in [&AC_LUMA_VALS[..], &AC_CHROMA_VALS[..]] {
            let mut seen = [false; 256];
            for &v in vals {
                assert!(!seen[v as usize], "duplicate symbol {v:02x}");
                seen[v as usize] = true;
            }
            assert!(seen[0x00] && seen[0xF0], "EOB and ZRL are present");
            // all 160 (run, size) pairs with run 0..=15, size 1..=10
            for run in 0..16u8 {
                for size in 1..=10u8 {
                    assert!(seen[(run << 4 | size) as usize], "missing {run}/{size}");
                }
            }
        }
        // the known first codes: DC luma size 0 is 00, AC luma EOB is 1010
        assert_eq!(huffman_codes(&DC_LUMA_BITS, &DC_VALS)[0], (0, 2));
        assert_eq!(huffman_codes(&AC_LUMA_BITS, &AC_LUMA_VALS)[0x00], (0b1010, 4));
    }

    #[test]
    fn quality_scales_the_standard_tables() {
        assert_eq!(scaled_table(&LUMA_Q, 50), LUMA_Q);
        assert!(scaled_table(&LUMA_Q, 90).iter().zip(&LUMA_Q).all(|(a, b)| a <= b));
        assert!(scaled_table(&LUMA_Q, 10).iter().zip(&LUMA_Q).all(|(a, b)| a >= b));
        assert!(scaled_table(&LUMA_Q, 100).iter().all(|&v| v == 1));
        assert!(scaled_table(&CHROMA_Q, 1).iter().all(|&v| v == 255));
    }

    #[test]
    fn output_has_soi_eoi_and_the_standard_segments() {
        let jpg = encode_rgb(48, 32, &gradient(48, 32), 85, None);
        assert_eq!(&jpg[..2], &[0xFF, 0xD8]);
        assert_eq!(&jpg[jpg.len() - 2..], &[0xFF, 0xD9]);
        let segs = segments(&jpg);
        let count = |m: u8| segs.iter().filter(|(k, _)| *k == m).count();
        assert_eq!(count(0xDB), 2, "two DQT");
        assert_eq!(count(0xC4), 4, "four DHT");
        assert_eq!(count(0xC0), 1, "baseline SOF0");
        assert_eq!(segs.last().unwrap().0, 0xDA);
        let sof = &segs.iter().find(|(k, _)| *k == 0xC0).unwrap().1;
        assert_eq!(&sof[1..5], &[0, 32, 0, 48], "height then width");
        assert_eq!(&sof[6..], &[1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1], "4:2:0");
        // the luma DQT carries the quality-scaled table in zigzag order
        let q = scaled_table(&LUMA_Q, 85);
        let dqt0 = &segs.iter().find(|(k, b)| *k == 0xDB && b[0] == 0).unwrap().1;
        assert_eq!(dqt0[1], q[0] as u8);
        assert_eq!(dqt0[2], q[1] as u8);
        assert_eq!(dqt0[3], q[8] as u8);
        // no stray marker inside the entropy data: every FF is followed by 00
        let scan = &jpg[..jpg.len() - 2];
        let start = scan.windows(2).position(|w| w == [0xFF, 0xDA]).unwrap();
        let data = &scan[start + 2 + 2 + 10..];
        for (i, &b) in data.iter().enumerate() {
            if b == 0xFF {
                assert_eq!(data.get(i + 1), Some(&0), "stuffed FF at {i}");
            }
        }
    }

    #[test]
    fn gradient_round_trips_within_tolerance() {
        let (w, h) = (64, 48);
        let src = gradient(w, h);
        let jpg = encode_rgb(w, h, &src, 92, None);
        let (dw, dh, px) = decode(&jpg);
        assert_eq!((dw, dh), (w, h));
        assert_eq!(px.len(), w * h * 3);
        let err = mean_abs_error(&src, &px);
        assert!(err < 4.0, "mean abs error {err}");
    }

    #[test]
    fn odd_sizes_are_padded_and_cropped() {
        for (w, h) in [(1, 1), (17, 9), (37, 23), (33, 65)] {
            let src = gradient(w, h);
            let jpg = encode_rgb(w, h, &src, 90, None);
            let (dw, dh, px) = decode(&jpg);
            assert_eq!((dw, dh), (w, h), "{w}x{h}");
            assert!(mean_abs_error(&src, &px) < 8.0, "{w}x{h}");
        }
    }

    #[test]
    fn flat_colour_survives() {
        let src: Vec<u8> = (0..32 * 32).flat_map(|_| [200u8, 40, 90]).collect();
        let (_, _, px) = decode(&encode_rgb(32, 32, &src, 90, None));
        assert!(mean_abs_error(&src, &px) < 3.0);
    }

    #[test]
    fn higher_quality_is_bigger_and_closer() {
        // a gradient with some texture so the quality matters
        let (w, h) = (64, 64);
        let mut src = gradient(w, h);
        for (i, v) in src.iter_mut().enumerate() {
            *v = v.saturating_add(((i * 2654435761usize) >> 28) as u8 % 24);
        }
        let low = encode_rgb(w, h, &src, 20, None);
        let high = encode_rgb(w, h, &src, 95, None);
        assert!(high.len() > low.len());
        let (_, _, a) = decode(&low);
        let (_, _, b) = decode(&high);
        assert!(mean_abs_error(&src, &b) < mean_abs_error(&src, &a));
    }

    #[test]
    fn exif_carries_orientation_and_date() {
        let e = Exif { orientation: 6, date: "2026:10:04 00:23:15".into() };
        let jpg = encode_rgb(16, 16, &gradient(16, 16), 80, Some(&e));
        let segs = segments(&jpg);
        let app1 = &segs.iter().find(|(k, _)| *k == 0xE1).expect("APP1").1;
        assert_eq!(&app1[..6], b"Exif\0\0");
        let tiff = &app1[6..];
        assert_eq!(&tiff[..4], b"MM\x00\x2A");
        // IFD0: orientation first
        assert_eq!(&tiff[10..12], &[0x01, 0x12]);
        assert_eq!(&tiff[18..20], &[0, 6]);
        let date = &tiff[50..70];
        assert_eq!(&date[..19], b"2026:10:04 00:23:15");
        assert_eq!(date[19], 0);
        // the Exif IFD is where IFD0 says
        assert_eq!(&tiff[70..72], &[0, 2]);
        assert_eq!(&tiff[72..74], &[0x90, 0x03]);
        // the file still decodes with the metadata in it
        assert_eq!(decode(&jpg).0, 16);
    }

    #[test]
    fn bad_input_yields_nothing() {
        assert!(encode_rgb(0, 4, &[], 80, None).is_empty());
        assert!(encode_rgb(4, 4, &[0; 10], 80, None).is_empty());
        assert!(encode_pixels(4, 4, &[0; 3], 80, None).is_empty());
    }

    #[test]
    fn packed_pixels_match_rgb() {
        let (w, h) = (16, 16);
        let rgb = gradient(w, h);
        let px: Vec<u32> = rgb.chunks(3).map(|c| crate::gfx::pack(c[0], c[1], c[2], 255)).collect();
        assert_eq!(encode_pixels(w, h, &px, 80, None), encode_rgb(w, h, &rgb, 80, None));
    }

    #[test]
    fn category_bits_follow_the_standard() {
        assert_eq!(category(0), (0, 0));
        assert_eq!(category(1), (1, 1));
        assert_eq!(category(-1), (1, 0));
        assert_eq!(category(-3), (2, 0));
        assert_eq!(category(5), (3, 5));
        assert_eq!(category(-5), (3, 2));
    }
}
