//! PNG and JPEG writers. PNG picks the smallest colour type that holds the pixels, filters each row adaptively and
//! writes text, resolution, gamma, profile and EXIF chunks. JPEG is baseline Huffman with quality 1..=100,
//! 4:2:0 or 4:4:4 chroma and JFIF density, EXIF, profile and comment segments.

fn crc_table() -> &'static [u32; 256] {
    use std::sync::OnceLock;
    static T: OnceLock<[u32; 256]> = OnceLock::new();
    T.get_or_init(|| {
        let mut t = [0u32; 256];
        for (i, e) in t.iter_mut().enumerate() {
            let mut c = i as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            }
            *e = c;
        }
        t
    })
}

/// CRC-32 (IEEE) of `data`, as PNG chunks carry it.
pub fn crc32(data: &[u8]) -> u32 {
    let t = crc_table();
    !data.iter().fold(!0u32, |c, &b| t[((c ^ b as u32) & 255) as usize] ^ (c >> 8))
}

fn chunk(out: &mut Vec<u8>, ty: &[u8; 4], data: &[u8]) {
    out.extend((data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend(ty);
    out.extend(data);
    let c = crc32(&out[start..]);
    out.extend(c.to_be_bytes());
}

/// Optional PNG content besides the pixels.
#[derive(Clone, Debug, Default)]
pub struct PngOptions {
    pub text: Vec<(String, String)>,
    pub dpi: Option<(f64, f64)>,
    /// TIFF-structured EXIF block (the `eXIf` chunk body).
    pub exif: Option<Vec<u8>>,
    /// Profile name and raw profile bytes.
    pub icc: Option<(String, Vec<u8>)>,
    pub gamma: Option<f64>,
    /// zlib level 0..=10; default 6.
    pub level: Option<u8>,
    /// An XMP packet, written as the `iTXt` chunk `XML:com.adobe.xmp` (uncompressed, as the XMP specification asks).
    pub xmp: Option<Vec<u8>>,
}

fn paeth(a: i32, b: i32, c: i32) -> i32 {
    let p = a + b - c;
    let (pa, pb, pc) = ((p - a).abs(), (p - b).abs(), (p - c).abs());
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

fn filter_rows(raw: &[u8], row_len: usize, h: usize, bpp: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity((row_len + 1) * h);
    let zero = vec![0u8; row_len];
    for y in 0..h {
        let row = &raw[y * row_len..(y + 1) * row_len];
        let prev = if y == 0 { &zero[..] } else { &raw[(y - 1) * row_len..y * row_len] };
        let mut best: Option<(u64, u8, Vec<u8>)> = None;
        for f in 0..5u8 {
            let mut v = Vec::with_capacity(row_len);
            for i in 0..row_len {
                let a = if i >= bpp { row[i - bpp] as i32 } else { 0 };
                let b = prev[i] as i32;
                let c = if i >= bpp { prev[i - bpp] as i32 } else { 0 };
                let x = row[i] as i32;
                v.push(match f {
                    0 => x,
                    1 => x - a,
                    2 => x - b,
                    3 => x - (a + b) / 2,
                    _ => x - paeth(a, b, c),
                } as u8);
            }
            let score: u64 = v.iter().map(|&b| (b as i8).unsigned_abs() as u64).sum();
            if best.as_ref().is_none_or(|b| score < b.0) {
                best = Some((score, f, v));
            }
        }
        let (_, f, v) = best.unwrap();
        out.push(f);
        out.extend(v);
    }
    out
}

/// The signature and every chunk before the image data: header, gamma, profile, resolution, EXIF and text.
/// `extra` is written right after the header (the animation control chunk of an animated picture).
fn png_preamble(w: usize, h: usize, depth: u8, ctype: u8, extra: Option<(&[u8; 4], &[u8])>, o: &PngOptions) -> Vec<u8> {
    let mut out = super::png::SIGNATURE.to_vec();
    let mut ihdr = Vec::new();
    ihdr.extend((w as u32).to_be_bytes());
    ihdr.extend((h as u32).to_be_bytes());
    ihdr.extend([depth, ctype, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    if let Some((ty, d)) = extra {
        chunk(&mut out, ty, d);
    }
    if let Some(g) = o.gamma {
        chunk(&mut out, b"gAMA", &((g * 100_000.0).round() as u32).to_be_bytes());
    }
    let level = o.level.unwrap_or(6).min(10);
    if let Some((name, profile)) = &o.icc {
        let mut d: Vec<u8> = name.bytes().take(79).collect();
        d.extend([0, 0]);
        d.extend(miniz_oxide::deflate::compress_to_vec_zlib(profile, level));
        chunk(&mut out, b"iCCP", &d);
    }
    if let Some((dx, dy)) = o.dpi {
        let mut d = Vec::new();
        d.extend(((dx / 0.0254).round() as u32).to_be_bytes());
        d.extend(((dy / 0.0254).round() as u32).to_be_bytes());
        d.push(1);
        chunk(&mut out, b"pHYs", &d);
    }
    if let Some(e) = &o.exif {
        chunk(&mut out, b"eXIf", e);
    }
    if let Some(x) = o.xmp.as_ref().filter(|x| !x.is_empty()) {
        // keyword, NUL, compression flag 0, method 0, empty language tag and translated keyword, then the packet
        let mut d = b"XML:com.adobe.xmp\0\0\0\0\0".to_vec();
        d.extend(x);
        chunk(&mut out, b"iTXt", &d);
    }
    for (k, v) in &o.text {
        let key: Vec<u8> = k.chars().filter(|&c| (c as u32) < 256 && c != '\0').take(79).map(|c| c as u8).collect();
        if key.is_empty() {
            continue;
        }
        if v.chars().all(|c| (c as u32) < 256 && c != '\0') {
            let mut d = key;
            d.push(0);
            d.extend(v.chars().map(|c| c as u8));
            chunk(&mut out, b"tEXt", &d);
        } else {
            let mut d = key;
            d.extend([0, 0, 0, 0, 0]);
            d.extend(v.as_bytes());
            chunk(&mut out, b"iTXt", &d);
        }
    }
    out
}

#[allow(clippy::too_many_arguments)] // PNG header fields plus pixel buffer; a struct would only be used here
fn finish_png(w: usize, h: usize, depth: u8, ctype: u8, raw: &[u8], bpp: usize, row_len: usize, o: &PngOptions) -> Vec<u8> {
    let level = o.level.unwrap_or(6).min(10);
    let mut out = png_preamble(w, h, depth, ctype, None, o);
    let filtered = filter_rows(raw, row_len, h, bpp);
    chunk(&mut out, b"IDAT", &miniz_oxide::deflate::compress_to_vec_zlib(&filtered, level));
    chunk(&mut out, b"IEND", &[]);
    out
}

/// Encodes straight 8-bit RGBA. Gray and opaque images use the smaller colour types.
pub fn encode_png(w: usize, h: usize, rgba: &[u8], o: &PngOptions) -> Option<Vec<u8>> {
    if w == 0 || h == 0 || rgba.len() < w * h * 4 {
        return None;
    }
    let px = rgba.as_chunks::<4>().0.iter().take(w * h);
    let opaque = px.clone().all(|p| p[3] == 255);
    let gray = px.clone().all(|p| p[0] == p[1] && p[1] == p[2]);
    let (ctype, ch): (u8, usize) = match (gray, opaque) {
        (true, true) => (0, 1),
        (true, false) => (4, 2),
        (false, true) => (2, 3),
        (false, false) => (6, 4),
    };
    let mut raw = Vec::with_capacity(w * h * ch);
    for p in px {
        match ctype {
            0 => raw.push(p[0]),
            4 => raw.extend([p[0], p[3]]),
            2 => raw.extend(&p[..3]),
            _ => raw.extend(p),
        }
    }
    Some(finish_png(w, h, 8, ctype, &raw, ch, w * ch, o))
}

/// One full-canvas picture of an animated PNG.
#[derive(Clone, Copy, Debug)]
pub struct ApngFrame<'a> {
    /// Straight 8-bit RGBA, `w * h * 4` bytes.
    pub rgba: &'a [u8],
    pub delay_ms: u32,
}

/// Encodes an animated PNG: every frame covers the whole canvas and replaces what was there, so a reader that does
/// not know animation shows the first frame. `loop_count` 0 repeats forever. `None` for no frames or a frame
/// that is too small for the canvas.
pub fn encode_apng(w: usize, h: usize, frames: &[ApngFrame], loop_count: u32, o: &PngOptions) -> Option<Vec<u8>> {
    if w == 0 || h == 0 || frames.is_empty() || frames.iter().any(|f| f.rgba.len() < w * h * 4) {
        return None;
    }
    let level = o.level.unwrap_or(6).min(10);
    let mut actl = Vec::new();
    actl.extend((frames.len() as u32).to_be_bytes());
    actl.extend(loop_count.to_be_bytes());
    let mut out = png_preamble(w, h, 8, 6, Some((b"acTL", &actl[..])), o);
    let mut seq = 0u32;
    for (i, f) in frames.iter().enumerate() {
        let mut fctl = Vec::new();
        fctl.extend(seq.to_be_bytes());
        seq += 1;
        fctl.extend((w as u32).to_be_bytes());
        fctl.extend((h as u32).to_be_bytes());
        fctl.extend([0u8; 8]); // x and y offsets
        fctl.extend((f.delay_ms.min(65535) as u16).to_be_bytes());
        fctl.extend(1000u16.to_be_bytes());
        fctl.extend([0, 0]); // dispose none, blend source
        chunk(&mut out, b"fcTL", &fctl);
        let filtered = filter_rows(&f.rgba[..w * h * 4], w * 4, h, 4);
        let z = miniz_oxide::deflate::compress_to_vec_zlib(&filtered, level);
        if i == 0 {
            chunk(&mut out, b"IDAT", &z);
        } else {
            let mut d = seq.to_be_bytes().to_vec();
            seq += 1;
            d.extend(z);
            chunk(&mut out, b"fdAT", &d);
        }
    }
    chunk(&mut out, b"IEND", &[]);
    Some(out)
}

/// Encodes straight 16-bit RGBA (always RGBA, 16 bits per sample).
pub fn encode_png16(w: usize, h: usize, rgba: &[u16], o: &PngOptions) -> Option<Vec<u8>> {
    if w == 0 || h == 0 || rgba.len() < w * h * 4 {
        return None;
    }
    let raw: Vec<u8> = rgba.iter().take(w * h * 4).flat_map(|v| v.to_be_bytes()).collect();
    Some(finish_png(w, h, 16, 6, &raw, 8, w * 8, o))
}

// ---- JPEG ----

#[derive(Clone, Debug)]
pub struct JpegOptions {
    /// 1..=100.
    pub quality: u8,
    /// 4:2:0 chroma subsampling; false keeps full-resolution chroma.
    pub subsample: bool,
    pub dpi: Option<(f64, f64)>,
    pub exif: Option<Vec<u8>>,
    pub icc: Option<Vec<u8>>,
    pub comment: Option<String>,
    /// An XMP packet, written as an APP1 segment after the EXIF one (left out when it does not fit one segment).
    pub xmp: Option<Vec<u8>>,
    /// An APP13 payload (image resource blocks with their header, which carry the IPTC datasets).
    pub image_resources: Option<Vec<u8>>,
}

impl Default for JpegOptions {
    fn default() -> Self {
        JpegOptions { quality: 80, subsample: true, dpi: None, exif: None, icc: None, comment: None, xmp: None, image_resources: None }
    }
}

/// The prefix of an APP1 segment that holds an XMP packet.
pub const XMP_APP1_HEADER: &[u8] = b"http://ns.adobe.com/xap/1.0/\0";

const ZIGZAG: [usize; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20, 13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44,
    51, 58, 59, 52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];

const Q_LUM: [u16; 64] = [
    16, 11, 10, 16, 24, 40, 51, 61, 12, 12, 14, 19, 26, 58, 60, 55, 14, 13, 16, 24, 40, 57, 69, 56, 14, 17, 22, 29, 51, 87, 80, 62, 18, 22, 37, 56, 68, 109, 103, 77, 24, 35, 55, 64, 81,
    104, 113, 92, 49, 64, 78, 87, 103, 121, 120, 101, 72, 92, 95, 98, 112, 100, 103, 99,
];
const Q_CHROMA: [u16; 64] = [
    17, 18, 24, 47, 99, 99, 99, 99, 18, 21, 26, 66, 99, 99, 99, 99, 24, 26, 56, 99, 99, 99, 99, 99, 47, 66, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99,
    99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99,
];

const DC_LUM_BITS: [u8; 16] = [0, 1, 5, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0];
const DC_CHROMA_BITS: [u8; 16] = [0, 3, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0];
const AC_LUM_BITS: [u8; 16] = [0, 2, 1, 3, 3, 2, 4, 3, 5, 5, 4, 4, 0, 0, 1, 0x7d];
const AC_CHROMA_BITS: [u8; 16] = [0, 2, 1, 2, 4, 4, 3, 4, 7, 5, 4, 4, 0, 1, 2, 0x77];

fn ac_lum_vals() -> Vec<u8> {
    let mut v: Vec<u8> = vec![
        0x01, 0x02, 0x03, 0x00, 0x04, 0x11, 0x05, 0x12, 0x21, 0x31, 0x41, 0x06, 0x13, 0x51, 0x61, 0x07, 0x22, 0x71, 0x14, 0x32, 0x81, 0x91, 0xa1, 0x08, 0x23, 0x42, 0xb1, 0xc1, 0x15, 0x52,
        0xd1, 0xf0, 0x24, 0x33, 0x62, 0x72, 0x82, 0x09, 0x0a, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x25, 0x26, 0x27, 0x28, 0x29, 0x2a,
    ];
    for (a, b) in [(0x34, 0x3a), (0x43, 0x4a), (0x53, 0x5a), (0x63, 0x6a), (0x73, 0x7a), (0x83, 0x8a), (0x92, 0x9a), (0xa2, 0xaa), (0xb2, 0xba), (0xc2, 0xca), (0xd2, 0xda), (0xe1, 0xea), (0xf1, 0xfa)] {
        v.extend(a..=b);
    }
    v
}

fn ac_chroma_vals() -> Vec<u8> {
    let mut v: Vec<u8> = vec![
        0x00, 0x01, 0x02, 0x03, 0x11, 0x04, 0x05, 0x21, 0x31, 0x06, 0x12, 0x41, 0x51, 0x07, 0x61, 0x71, 0x13, 0x22, 0x32, 0x81, 0x08, 0x14, 0x42, 0x91, 0xa1, 0xb1, 0xc1, 0x09, 0x23, 0x33,
        0x52, 0xf0, 0x15, 0x62, 0x72, 0xd1, 0x0a, 0x16, 0x24, 0x34, 0xe1, 0x25, 0xf1, 0x17, 0x18, 0x19, 0x1a, 0x26, 0x27, 0x28, 0x29, 0x2a,
    ];
    for (a, b) in [(0x35, 0x3a), (0x43, 0x4a), (0x53, 0x5a), (0x63, 0x6a), (0x73, 0x7a), (0x82, 0x8a), (0x92, 0x9a), (0xa2, 0xaa), (0xb2, 0xba), (0xc2, 0xca), (0xd2, 0xda), (0xe2, 0xea), (0xf2, 0xfa)] {
        v.extend(a..=b);
    }
    v
}

struct Huff {
    bits: [u8; 16],
    vals: Vec<u8>,
    code: [(u16, u8); 256],
}

impl Huff {
    fn new(bits: [u8; 16], vals: Vec<u8>) -> Huff {
        let mut code = [(0u16, 0u8); 256];
        let (mut c, mut k) = (0u32, 0usize);
        for len in 1..=16u8 {
            for _ in 0..bits[len as usize - 1] {
                code[vals[k] as usize] = (c as u16, len);
                c += 1;
                k += 1;
            }
            c <<= 1;
        }
        Huff { bits, vals, code }
    }
}

struct Bits {
    out: Vec<u8>,
    acc: u32,
    n: u32,
}

impl Bits {
    fn put(&mut self, code: u32, len: u32) {
        if len == 0 {
            return;
        }
        self.acc = (self.acc << len) | (code & ((1 << len) - 1));
        self.n += len;
        while self.n >= 8 {
            let b = (self.acc >> (self.n - 8)) as u8;
            self.out.push(b);
            if b == 0xFF {
                self.out.push(0);
            }
            self.n -= 8;
        }
        self.acc &= (1 << self.n) - 1;
    }
    fn flush(&mut self) {
        if self.n > 0 {
            let pad = 8 - self.n;
            self.put((1 << pad) - 1, pad);
        }
    }
}

fn category(v: i32) -> u32 {
    32 - (v.unsigned_abs()).leading_zeros()
}

fn cos_table() -> [[f32; 8]; 8] {
    let mut t = [[0f32; 8]; 8];
    for (x, row) in t.iter_mut().enumerate() {
        for (u, e) in row.iter_mut().enumerate() {
            let cu = if u == 0 { std::f32::consts::FRAC_1_SQRT_2 } else { 1.0 };
            *e = 0.5 * cu * (((2 * x + 1) as f32) * u as f32 * std::f32::consts::PI / 16.0).cos();
        }
    }
    t
}

fn fdct(block: &[f32; 64], t: &[[f32; 8]; 8]) -> [f32; 64] {
    let mut tmp = [0f32; 64];
    for y in 0..8 {
        for u in 0..8 {
            tmp[y * 8 + u] = (0..8).map(|x| block[y * 8 + x] * t[x][u]).sum();
        }
    }
    let mut out = [0f32; 64];
    for v in 0..8 {
        for u in 0..8 {
            out[v * 8 + u] = (0..8).map(|y| tmp[y * 8 + u] * t[y][v]).sum();
        }
    }
    out
}

fn scaled_table(base: &[u16; 64], quality: u8) -> [u16; 64] {
    let q = quality.clamp(1, 100) as u32;
    let scale = if q < 50 { 5000 / q } else { 200 - 2 * q };
    let mut t = [0u16; 64];
    for i in 0..64 {
        t[i] = ((base[i] as u32 * scale + 50) / 100).clamp(1, 255) as u16;
    }
    t
}

struct Encoder<'a> {
    bits: Bits,
    cos: [[f32; 8]; 8],
    q: [&'a [u16; 64]; 2],
    dc: [&'a Huff; 2],
    ac: [&'a Huff; 2],
}

impl Encoder<'_> {
    fn block(&mut self, plane: &[f32], stride: usize, x0: usize, y0: usize, comp_table: usize, pred: &mut i32) {
        let mut b = [0f32; 64];
        for y in 0..8 {
            for x in 0..8 {
                b[y * 8 + x] = plane[(y0 + y) * stride + x0 + x] - 128.0;
            }
        }
        let d = fdct(&b, &self.cos);
        let q = self.q[comp_table];
        let mut zz = [0i32; 64];
        for k in 0..64 {
            let n = ZIGZAG[k];
            zz[k] = (d[n] / q[n] as f32).round() as i32;
        }
        let diff = zz[0] - *pred;
        *pred = zz[0];
        let cat = category(diff);
        let (code, len) = self.dc[comp_table].code[cat as usize];
        self.bits.put(code as u32, len as u32);
        if cat > 0 {
            let v = if diff < 0 { diff - 1 } else { diff };
            self.bits.put(v as u32, cat);
        }
        let mut run = 0;
        for &v in zz.iter().skip(1) {
            if v == 0 {
                run += 1;
                continue;
            }
            while run > 15 {
                let (c, l) = self.ac[comp_table].code[0xF0];
                self.bits.put(c as u32, l as u32);
                run -= 16;
            }
            let cat = category(v);
            let (c, l) = self.ac[comp_table].code[(run << 4) as usize | cat as usize];
            self.bits.put(c as u32, l as u32);
            let vv = if v < 0 { v - 1 } else { v };
            self.bits.put(vv as u32, cat);
            run = 0;
        }
        if run > 0 {
            let (c, l) = self.ac[comp_table].code[0];
            self.bits.put(c as u32, l as u32);
        }
    }
}

fn segment(out: &mut Vec<u8>, marker: u8, body: &[u8]) {
    out.extend([0xFF, marker]);
    out.extend(((body.len() + 2) as u16).to_be_bytes());
    out.extend(body);
}

/// Encodes straight 8-bit RGBA as a baseline JPEG (alpha is composited over white; gray images are written
/// as one component).
pub fn encode_jpeg(w: usize, h: usize, rgba: &[u8], o: &JpegOptions) -> Option<Vec<u8>> {
    if w == 0 || h == 0 || w > 65535 || h > 65535 || rgba.len() < w * h * 4 {
        return None;
    }
    let gray = rgba.as_chunks::<4>().0.iter().take(w * h).all(|p| p[0] == p[1] && p[1] == p[2]);
    let (hs, vs) = if gray || !o.subsample { (1usize, 1usize) } else { (2, 2) };
    let (mcu_w, mcu_h) = (8 * hs, 8 * vs);
    let (pw, ph) = (w.div_ceil(mcu_w) * mcu_w, h.div_ceil(mcu_h) * mcu_h);
    // planes at full resolution, padded by replicating edges
    let mut yp = vec![0f32; pw * ph];
    let mut cbp = vec![0f32; pw * ph];
    let mut crp = vec![0f32; pw * ph];
    for y in 0..ph {
        for x in 0..pw {
            let p = &rgba[(y.min(h - 1) * w + x.min(w - 1)) * 4..][..4];
            let a = p[3] as f32 / 255.0;
            let f = |c: u8| c as f32 * a + 255.0 * (1.0 - a);
            let (r, g, b) = (f(p[0]), f(p[1]), f(p[2]));
            let i = y * pw + x;
            yp[i] = 0.299 * r + 0.587 * g + 0.114 * b;
            cbp[i] = -0.168_736 * r - 0.331_264 * g + 0.5 * b + 128.0;
            crp[i] = 0.5 * r - 0.418_688 * g - 0.081_312 * b + 128.0;
        }
    }
    let (cw, ch) = (pw / hs, ph / vs);
    let down = |p: &[f32]| -> Vec<f32> {
        if hs == 1 {
            return p.to_vec();
        }
        let mut d = vec![0f32; cw * ch];
        for y in 0..ch {
            for x in 0..cw {
                d[y * cw + x] = (p[2 * y * pw + 2 * x] + p[2 * y * pw + 2 * x + 1] + p[(2 * y + 1) * pw + 2 * x] + p[(2 * y + 1) * pw + 2 * x + 1]) / 4.0;
            }
        }
        d
    };
    let (cb, cr) = if gray { (Vec::new(), Vec::new()) } else { (down(&cbp), down(&crp)) };

    let ql = scaled_table(&Q_LUM, o.quality);
    let qc = scaled_table(&Q_CHROMA, o.quality);
    let dc_l = Huff::new(DC_LUM_BITS, (0..12).collect());
    let dc_c = Huff::new(DC_CHROMA_BITS, (0..12).collect());
    let ac_l = Huff::new(AC_LUM_BITS, ac_lum_vals());
    let ac_c = Huff::new(AC_CHROMA_BITS, ac_chroma_vals());

    let mut out = vec![0xFF, 0xD8];
    let (dx, dy) = o.dpi.unwrap_or((72.0, 72.0));
    let mut jfif = b"JFIF\0".to_vec();
    jfif.extend([1, 1, 1]);
    jfif.extend((dx.round().clamp(1.0, 65535.0) as u16).to_be_bytes());
    jfif.extend((dy.round().clamp(1.0, 65535.0) as u16).to_be_bytes());
    jfif.extend([0, 0]);
    segment(&mut out, 0xE0, &jfif);
    if let Some(e) = &o.exif {
        if e.len() < 65_000 {
            let mut b = b"Exif\0\0".to_vec();
            b.extend(e);
            segment(&mut out, 0xE1, &b);
        }
    }
    if let Some(x) = o.xmp.as_ref().filter(|x| !x.is_empty() && x.len() + XMP_APP1_HEADER.len() < 65_000) {
        let mut b = XMP_APP1_HEADER.to_vec();
        b.extend(x);
        segment(&mut out, 0xE1, &b);
    }
    if let Some(r) = o.image_resources.as_ref().filter(|r| !r.is_empty() && r.len() < 65_000) {
        segment(&mut out, 0xED, r);
    }
    if let Some(icc) = &o.icc {
        let chunks: Vec<&[u8]> = icc.chunks(60_000).collect();
        for (i, c) in chunks.iter().enumerate() {
            let mut b = b"ICC_PROFILE\0".to_vec();
            b.extend([(i + 1) as u8, chunks.len() as u8]);
            b.extend(*c);
            segment(&mut out, 0xE2, &b);
        }
    }
    if let Some(c) = &o.comment {
        segment(&mut out, 0xFE, &c.as_bytes()[..c.len().min(60_000)]);
    }
    for (id, t) in [(0u8, &ql), (1, &qc)].iter().take(if gray { 1 } else { 2 }) {
        let mut b = vec![*id];
        b.extend(ZIGZAG.iter().map(|&n| t[n] as u8));
        segment(&mut out, 0xDB, &b);
    }
    let mut sof = vec![8];
    sof.extend((h as u16).to_be_bytes());
    sof.extend((w as u16).to_be_bytes());
    if gray {
        sof.extend([1, 1, 0x11, 0]);
    } else {
        sof.extend([3, 1, ((hs << 4) | vs) as u8, 0, 2, 0x11, 1, 3, 0x11, 1]);
    }
    segment(&mut out, 0xC0, &sof);
    for (class_id, hf) in [(0x00u8, &dc_l), (0x10, &ac_l), (0x01, &dc_c), (0x11, &ac_c)] {
        if gray && class_id & 1 == 1 {
            continue;
        }
        let mut b = vec![class_id];
        b.extend(hf.bits);
        b.extend(&hf.vals);
        segment(&mut out, 0xC4, &b);
    }
    let mut sos = vec![if gray { 1 } else { 3 }, 1, 0x00];
    if !gray {
        sos.extend([2, 0x11, 3, 0x11]);
    }
    sos.extend([0, 63, 0]);
    segment(&mut out, 0xDA, &sos);

    let mut enc = Encoder { bits: Bits { out: Vec::new(), acc: 0, n: 0 }, cos: cos_table(), q: [&ql, &qc], dc: [&dc_l, &dc_c], ac: [&ac_l, &ac_c] };
    let (mut p_y, mut p_cb, mut p_cr) = (0i32, 0i32, 0i32);
    for my in 0..ph / mcu_h {
        for mx in 0..pw / mcu_w {
            for by in 0..vs {
                for bx in 0..hs {
                    enc.block(&yp, pw, mx * mcu_w + bx * 8, my * mcu_h + by * 8, 0, &mut p_y);
                }
            }
            if !gray {
                enc.block(&cb, cw, mx * 8, my * 8, 1, &mut p_cb);
                enc.block(&cr, cw, mx * 8, my * 8, 1, &mut p_cr);
            }
        }
    }
    enc.bits.flush();
    out.extend(enc.bits.out);
    out.extend([0xFF, 0xD9]);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cg::imageio::{jpeg, png};

    fn gradient(w: usize, h: usize) -> Vec<u8> {
        (0..h).flat_map(|y| (0..w).flat_map(move |x| [(x * 255 / w) as u8, (y * 255 / h) as u8, 120, 255])).collect()
    }

    #[test]
    fn crc_matches_the_check_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn png_round_trips_in_every_colour_type() {
        let rgb = gradient(9, 7);
        let d = png::decode(&encode_png(9, 7, &rgb, &PngOptions::default()).unwrap()).unwrap();
        assert_eq!(d.frames[0].rgba8, rgb);
        let mut rgba = rgb.clone();
        rgba[3] = 40;
        let d = png::decode(&encode_png(9, 7, &rgba, &PngOptions::default()).unwrap()).unwrap();
        assert_eq!(d.frames[0].rgba8, rgba);
        let gray: Vec<u8> = (0..16u8).flat_map(|v| [v * 16, v * 16, v * 16, 255]).collect();
        let bytes = encode_png(4, 4, &gray, &PngOptions::default()).unwrap();
        assert_eq!(bytes[25], 0, "grayscale colour type");
        assert_eq!(png::decode(&bytes).unwrap().frames[0].rgba8, gray);
        let ga: Vec<u8> = (0..4u8).flat_map(|v| [v * 60, v * 60, v * 60, 100 + v]).collect();
        let bytes = encode_png(2, 2, &ga, &PngOptions::default()).unwrap();
        assert_eq!(bytes[25], 4);
        assert_eq!(png::decode(&bytes).unwrap().frames[0].rgba8, ga);
    }

    #[test]
    fn png_16_bit_keeps_precision() {
        let px: Vec<u16> = vec![65535, 1000, 40000, 65535, 0, 257, 12345, 30000];
        let d = png::decode(&encode_png16(2, 1, &px, &PngOptions::default()).unwrap()).unwrap();
        assert_eq!(d.frames[0].rgba16.as_deref(), Some(&px[..]));
        assert_eq!(d.props[0].depth, 16);
    }

    #[test]
    fn png_metadata_is_written_and_read_back() {
        let mut tiff = crate::cg::imageio::exif::Dict::new();
        tiff.insert("Orientation".into(), crate::cg::imageio::exif::PropValue::Int(8));
        let exif = crate::cg::imageio::exif::build_block(&tiff, &Default::default(), &Default::default());
        let o = PngOptions {
            text: vec![("Title".into(), "Fjord".into()), ("Note".into(), "sn\u{f3}r \u{2603}".into())],
            dpi: Some((144.0, 144.0)),
            exif,
            icc: Some(("test".into(), vec![1, 2, 3, 4])),
            gamma: Some(0.45455),
            level: None,
            xmp: Some(b"<x:xmpmeta>png</x:xmpmeta>".to_vec()),
        };
        let bytes = encode_png(3, 3, &gradient(3, 3), &o).unwrap();
        let p = png::probe(&bytes).unwrap();
        assert_eq!(p.png["Title"].as_str(), Some("Fjord"));
        assert_eq!(p.xmp.as_deref(), Some(b"<x:xmpmeta>png</x:xmpmeta>".as_slice()), "the XMP packet has its own iTXt chunk");
        assert_eq!(p.png["Note"].as_str(), Some("sn\u{f3}r \u{2603}"));
        assert!((p.dpi_width.unwrap() - 144.0).abs() < 0.1);
        assert_eq!(p.orientation, 8);
        assert_eq!(p.icc_profile.as_deref(), Some(&[1u8, 2, 3, 4][..]));
        assert!((p.png["Gamma"].as_f64().unwrap() - 0.45455).abs() < 1e-4);
    }

    #[test]
    fn jpeg_tables_are_consistent() {
        assert_eq!(AC_LUM_BITS.iter().map(|&b| b as usize).sum::<usize>(), ac_lum_vals().len());
        assert_eq!(AC_CHROMA_BITS.iter().map(|&b| b as usize).sum::<usize>(), ac_chroma_vals().len());
        for vals in [ac_lum_vals(), ac_chroma_vals()] {
            let mut s = vals.clone();
            s.sort();
            s.dedup();
            assert_eq!(s.len(), 162);
            assert!(s.contains(&0x00) && s.contains(&0xF0));
        }
    }

    #[test]
    fn jpeg_round_trips_approximately() {
        let src = gradient(37, 21);
        for sub in [true, false] {
            let o = JpegOptions { quality: 92, subsample: sub, dpi: Some((96.0, 96.0)), exif: None, icc: None, comment: Some("hi".into()), ..Default::default() };
            let bytes = encode_jpeg(37, 21, &src, &o).unwrap();
            let p = jpeg::probe(&bytes).unwrap();
            assert_eq!((p.pixel_width, p.pixel_height), (37, 21));
            assert_eq!(p.dpi_width, Some(96.0));
            let d = jpeg::decode(&bytes).unwrap();
            let f = &d.frames[0];
            let i = (10 * 37 + 20) * 4;
            for k in 0..3 {
                assert!((f.rgba8[i + k] as i32 - src[i + k] as i32).abs() < 14, "sub={sub} ch={k} {} vs {}", f.rgba8[i + k], src[i + k]);
            }
        }
    }

    #[test]
    fn jpeg_gray_and_quality_order() {
        let gray: Vec<u8> = (0..64u32).flat_map(|i| [(i * 4) as u8, (i * 4) as u8, (i * 4) as u8, 255]).collect();
        let lo = encode_jpeg(8, 8, &gray, &JpegOptions { quality: 10, ..Default::default() }).unwrap();
        let hi = encode_jpeg(8, 8, &gray, &JpegOptions { quality: 95, ..Default::default() }).unwrap();
        assert!(lo.len() <= hi.len());
        assert!(jpeg::decode(&lo).is_some() && jpeg::decode(&hi).is_some());
        assert!(encode_jpeg(0, 8, &[], &JpegOptions::default()).is_none());
    }
}
