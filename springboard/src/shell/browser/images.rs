//! Memory-lean image decoding and caching for the browser.
//!
//! Images are decoded straight to their display size: rows are fed through an
//! area-averaging downsampler, so a PNG is never fully resident at source
//! size (it streams row by row). JPEG output from zune-jpeg is one transient
//! RGB buffer (3 bytes/pixel) that is downsampled and freed; zune-jpeg 0.5
//! has no row or DCT-scaled API to avoid even that. Anything over
//! `MAX_PIXELS` is rejected from the header alone, before any decode.
//!
//! Supported: JPEG, PNG. Sniffed and probed but not decoded (their decoder
//! crates are not direct dependencies of this crate, and SVG would need a
//! rasteriser): GIF, WebP, SVG. For those `decode` returns
//! `ImgError::Unsupported`; the page shows its alt text or a placeholder.
//! Animated GIFs therefore cost nothing (no frames are kept).

use std::collections::HashMap;
use std::io::Cursor;
use std::sync::Arc;

/// Source images above this many pixels (64 MP) are refused.
pub const MAX_PIXELS: u64 = 64 * 1024 * 1024;
/// Largest accepted side, matching the shared decoder.
pub const MAX_SIDE: u32 = 16384;

/// How decoded pixels are stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PixelFormat {
    /// 4 bytes per pixel, straight (non-premultiplied) alpha: R, G, B, A.
    Rgba8,
    /// 2 bytes per pixel little-endian 5-6-5, opaque only. Requesting it for an
    /// image with transparency silently yields `Rgba8`.
    Rgb565,
}

/// A decoded, display-sized bitmap.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bitmap {
    pub w: u32,
    pub h: u32,
    pub format: PixelFormat,
    pub data: Vec<u8>,
}

impl Bitmap {
    /// Resident bytes of the pixel data.
    pub fn bytes(&self) -> usize {
        self.data.len()
    }

    /// The pixel at (x, y) as straight RGBA.
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = (y * self.w + x) as usize;
        match self.format {
            PixelFormat::Rgba8 => [self.data[i * 4], self.data[i * 4 + 1], self.data[i * 4 + 2], self.data[i * 4 + 3]],
            PixelFormat::Rgb565 => {
                let v = u16::from_le_bytes([self.data[i * 2], self.data[i * 2 + 1]]);
                let (r, g, b) = ((v >> 11) as u8 & 31, (v >> 5) as u8 & 63, v as u8 & 31);
                [(r << 3) | (r >> 2), (g << 2) | (g >> 4), (b << 3) | (b >> 2), 255]
            }
        }
    }
}

/// Why a decode did not produce a bitmap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImgError {
    /// Recognised format without a decoder here (GIF, WebP, SVG) or unknown bytes.
    Unsupported,
    /// Header declares more than `MAX_PIXELS` or `MAX_SIDE`.
    TooLarge,
    /// Damaged or truncated data.
    Corrupt,
}

/// Container formats the sniffer knows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Jpeg,
    Png,
    Gif,
    WebP,
    Svg,
}

/// Identifies the format from the leading bytes.
pub fn sniff(b: &[u8]) -> Option<Format> {
    if b.starts_with(&[0xFF, 0xD8]) {
        Some(Format::Jpeg)
    } else if b.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        Some(Format::Png)
    } else if b.starts_with(b"GIF87a") || b.starts_with(b"GIF89a") {
        Some(Format::Gif)
    } else if b.len() >= 12 && &b[..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        Some(Format::WebP)
    } else {
        let head = &b[..b.len().min(512)];
        let s = String::from_utf8_lossy(head);
        (s.contains("<svg") || (s.trim_start().starts_with("<?xml") && s.contains("svg"))).then_some(Format::Svg)
    }
}

fn be32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

fn le16(b: &[u8]) -> u32 {
    u16::from_le_bytes([b[0], b[1]]) as u32
}

fn probe_jpeg(b: &[u8]) -> Option<(u32, u32)> {
    let mut i = 2;
    while i + 4 <= b.len() {
        if b[i] != 0xFF {
            i += 1;
            continue;
        }
        let m = b[i + 1];
        if m == 0xFF {
            i += 1;
            continue;
        }
        if m == 0xD8 || m == 0x01 || (0xD0..=0xD7).contains(&m) || m == 0x00 {
            i += 2;
            continue;
        }
        let len = u16::from_be_bytes([b[i + 2], b[i + 3]]) as usize;
        let sof = (0xC0..=0xCF).contains(&m) && m != 0xC4 && m != 0xC8 && m != 0xCC;
        if sof {
            let s = b.get(i + 5..i + 9)?;
            let (h, w) = (u16::from_be_bytes([s[0], s[1]]) as u32, u16::from_be_bytes([s[2], s[3]]) as u32);
            return (w > 0 && h > 0).then_some((w, h));
        }
        if m == 0xDA {
            return None;
        }
        i += 2 + len;
    }
    None
}

fn probe_webp(b: &[u8]) -> Option<(u32, u32)> {
    let tag = b.get(12..16)?;
    match tag {
        b"VP8X" => {
            let d = b.get(24..30)?;
            let w = 1 + (d[0] as u32 | (d[1] as u32) << 8 | (d[2] as u32) << 16);
            let h = 1 + (d[3] as u32 | (d[4] as u32) << 8 | (d[5] as u32) << 16);
            Some((w, h))
        }
        b"VP8L" => {
            let d = b.get(20..25)?;
            if d[0] != 0x2F {
                return None;
            }
            let bits = u32::from_le_bytes([d[1], d[2], d[3], d[4]]);
            Some((1 + (bits & 0x3FFF), 1 + ((bits >> 14) & 0x3FFF)))
        }
        b"VP8 " => {
            let d = b.get(20..30)?;
            if d[3..6] != [0x9D, 0x01, 0x2A] {
                return None;
            }
            Some((le16(&d[6..8]) & 0x3FFF, le16(&d[8..10]) & 0x3FFF))
        }
        _ => None,
    }
}

/// Reads the pixel dimensions from the header alone, without decoding.
/// Works for JPEG, PNG, GIF and WebP; None for SVG (no intrinsic size here)
/// and for unrecognised or truncated data. Use it for layout before the
/// decode finishes.
pub fn probe(bytes: &[u8]) -> Option<(u32, u32)> {
    match sniff(bytes)? {
        Format::Png => {
            let ihdr = bytes.get(8..24)?;
            (&ihdr[4..8] == b"IHDR").then(|| (be32(&ihdr[8..12]), be32(&ihdr[12..16])))
        }
        Format::Jpeg => probe_jpeg(bytes),
        Format::Gif => {
            let d = bytes.get(6..10)?;
            Some((le16(&d[0..2]), le16(&d[2..4])))
        }
        Format::WebP => probe_webp(bytes),
        Format::Svg => None,
    }
}

/// The size an image of `sw x sh` is decoded at for a `tw x th` target: fits
/// inside the target keeping aspect, never enlarges; a zero target side is
/// unconstrained.
pub fn fit_size(sw: u32, sh: u32, tw: u32, th: u32) -> (u32, u32) {
    if sw == 0 || sh == 0 || (tw == 0 && th == 0) || (tw >= sw && th >= sh) {
        return (sw, sh);
    }
    let mut scale = f64::INFINITY;
    if tw > 0 {
        scale = scale.min(tw as f64 / sw as f64);
    }
    if th > 0 {
        scale = scale.min(th as f64 / sh as f64);
    }
    let scale = scale.min(1.0);
    (((sw as f64 * scale).round() as u32).clamp(1, sw), ((sh as f64 * scale).round() as u32).clamp(1, sh))
}

// -------------------------------------------------------------- downsampler

/// Streaming area-average downsampler: feed source rows top to bottom as
/// straight RGBA; it keeps one accumulator row plus the output. Colour is
/// alpha-weighted so transparent pixels do not bleed their colour.
pub struct Downsampler {
    sw: usize,
    sh: usize,
    dw: usize,
    dh: usize,
    out: Vec<u8>,
    acc: Vec<u64>,
    cols: Vec<u32>,
    col_of: Vec<u32>,
    rows: u64,
    cur: usize,
    y: usize,
}

impl Downsampler {
    /// Destination must not exceed the source in either dimension.
    pub fn new(sw: u32, sh: u32, dw: u32, dh: u32) -> Downsampler {
        let (sw, sh, dw, dh) = (sw as usize, sh as usize, (dw as usize).clamp(1, sw as usize), (dh as usize).clamp(1, sh as usize));
        let col_of: Vec<u32> = (0..sw).map(|x| (x * dw / sw) as u32).collect();
        let mut cols = vec![0u32; dw];
        for &c in &col_of {
            cols[c as usize] += 1;
        }
        Downsampler { sw, sh, dw, dh, out: vec![0; dw * dh * 4], acc: vec![0; dw * 4], cols, col_of, rows: 0, cur: 0, y: 0 }
    }

    fn flush(&mut self) {
        if self.rows == 0 || self.cur >= self.dh {
            return;
        }
        for dx in 0..self.dw {
            let n = self.cols[dx] as u64 * self.rows;
            let a = self.acc[dx * 4 + 3];
            let o = (self.cur * self.dw + dx) * 4;
            if n > 0 && a > 0 {
                for c in 0..3 {
                    self.out[o + c] = ((self.acc[dx * 4 + c] + a / 2) / a).min(255) as u8;
                }
                self.out[o + 3] = ((a + n / 2) / n).min(255) as u8;
            }
        }
        self.acc.iter_mut().for_each(|v| *v = 0);
        self.rows = 0;
    }

    /// Adds the next source row (`sw * 4` bytes of RGBA). Extra rows are ignored.
    pub fn push_row(&mut self, row: &[u8]) {
        if self.y >= self.sh || row.len() < self.sw * 4 {
            return;
        }
        let dy = self.y * self.dh / self.sh;
        if dy != self.cur {
            self.flush();
            self.cur = dy;
        }
        for x in 0..self.sw {
            let d = self.col_of[x] as usize * 4;
            let p = &row[x * 4..x * 4 + 4];
            let a = p[3] as u64;
            self.acc[d] += p[0] as u64 * a;
            self.acc[d + 1] += p[1] as u64 * a;
            self.acc[d + 2] += p[2] as u64 * a;
            self.acc[d + 3] += a;
        }
        self.rows += 1;
        self.y += 1;
    }

    /// Source rows consumed so far.
    pub fn rows_pushed(&self) -> usize {
        self.y
    }

    /// Finishes and returns the destination as straight RGBA8.
    pub fn finish(mut self) -> Vec<u8> {
        self.flush();
        self.out
    }
}

fn finish_bitmap(rgba: Vec<u8>, w: u32, h: u32, fmt: PixelFormat) -> Bitmap {
    if fmt == PixelFormat::Rgb565 && rgba.chunks_exact(4).all(|p| p[3] == 255) {
        let mut data = Vec::with_capacity(rgba.len() / 2);
        for p in rgba.chunks_exact(4) {
            let v = ((p[0] as u16 >> 3) << 11) | ((p[1] as u16 >> 2) << 5) | (p[2] as u16 >> 3);
            data.extend_from_slice(&v.to_le_bytes());
        }
        return Bitmap { w, h, format: PixelFormat::Rgb565, data };
    }
    Bitmap { w, h, format: PixelFormat::Rgba8, data: rgba }
}

// ------------------------------------------------------------------ decode

fn decode_jpeg(bytes: &[u8], sw: u32, sh: u32, dw: u32, dh: u32) -> Result<Vec<u8>, ImgError> {
    use zune_core::colorspace::ColorSpace;
    use zune_core::options::DecoderOptions;
    let opts = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::RGB).set_max_width(MAX_SIDE as usize).set_max_height(MAX_SIDE as usize);
    let mut d = zune_jpeg::JpegDecoder::new_with_options(Cursor::new(bytes), opts);
    let data = d.decode().map_err(|_| ImgError::Corrupt)?;
    let info = d.info().ok_or(ImgError::Corrupt)?;
    let (w, h) = (info.width as u32, info.height as u32);
    if (w, h) != (sw, sh) || data.len() < w as usize * h as usize * 3 {
        return Err(ImgError::Corrupt);
    }
    let mut ds = Downsampler::new(w, h, dw, dh);
    let mut row = vec![255u8; w as usize * 4];
    for src in data.chunks_exact(w as usize * 3).take(h as usize) {
        for (o, p) in row.chunks_exact_mut(4).zip(src.chunks_exact(3)) {
            o[..3].copy_from_slice(p);
        }
        ds.push_row(&row);
    }
    drop(data);
    Ok(ds.finish())
}

fn decode_png(bytes: &[u8], sw: u32, sh: u32, dw: u32, dh: u32) -> Result<Vec<u8>, ImgError> {
    use png::{BitDepth, ColorType, Transformations};
    let mut d = png::Decoder::new(Cursor::new(bytes));
    d.set_transformations(Transformations::EXPAND | Transformations::STRIP_16);
    let mut r = d.read_info().map_err(|_| ImgError::Corrupt)?;
    let (ct, depth) = r.output_color_type();
    if depth != BitDepth::Eight {
        return Err(ImgError::Unsupported);
    }
    let w = r.info().width;
    if (w, r.info().height) != (sw, sh) {
        return Err(ImgError::Corrupt);
    }
    let mut ds = Downsampler::new(sw, sh, dw, dh);
    let mut line = vec![0u8; sw as usize * 4];
    while let Some(row) = r.next_row().map_err(|_| ImgError::Corrupt)? {
        let src = row.data();
        match ct {
            ColorType::Rgba => {
                let n = line.len();
                line.copy_from_slice(src.get(..n).ok_or(ImgError::Corrupt)?)
            }
            ColorType::Rgb => {
                for (o, p) in line.chunks_exact_mut(4).zip(src.chunks_exact(3)) {
                    o[..3].copy_from_slice(p);
                    o[3] = 255;
                }
            }
            ColorType::GrayscaleAlpha => {
                for (o, p) in line.chunks_exact_mut(4).zip(src.chunks_exact(2)) {
                    o.copy_from_slice(&[p[0], p[0], p[0], p[1]]);
                }
            }
            ColorType::Grayscale => {
                for (o, &g) in line.chunks_exact_mut(4).zip(src.iter()) {
                    o.copy_from_slice(&[g, g, g, 255]);
                }
            }
            ColorType::Indexed => return Err(ImgError::Unsupported),
        }
        ds.push_row(&line);
    }
    if ds.rows_pushed() < sh as usize {
        return Err(ImgError::Corrupt);
    }
    Ok(ds.finish())
}

/// Decodes `bytes` to fit inside `target_w x target_h` (0 = unconstrained on
/// that side), never enlarging. The header is checked first, so oversized
/// images cost nothing.
pub fn decode(bytes: &[u8], target_w: u32, target_h: u32, fmt: PixelFormat) -> Result<Bitmap, ImgError> {
    let format = sniff(bytes).ok_or(ImgError::Unsupported)?;
    if !matches!(format, Format::Jpeg | Format::Png) {
        return Err(ImgError::Unsupported);
    }
    let (sw, sh) = probe(bytes).ok_or(ImgError::Corrupt)?;
    if sw == 0 || sh == 0 {
        return Err(ImgError::Corrupt);
    }
    if sw > MAX_SIDE || sh > MAX_SIDE || sw as u64 * sh as u64 > MAX_PIXELS {
        return Err(ImgError::TooLarge);
    }
    let (dw, dh) = fit_size(sw, sh, target_w, target_h);
    let rgba = match format {
        Format::Jpeg => decode_jpeg(bytes, sw, sh, dw, dh)?,
        _ => decode_png(bytes, sw, sh, dw, dh)?,
    };
    Ok(finish_bitmap(rgba, dw, dh, fmt))
}

// ------------------------------------------------------------------- cache

/// FNV-1a hash of a URL, for `ImageKey`.
pub fn hash_url(url: &str) -> u64 {
    url.bytes().fold(0xcbf29ce484222325, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3))
}

/// A cached bitmap is identified by source and display size.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ImageKey {
    pub url_hash: u64,
    pub target_w: u32,
    pub target_h: u32,
}

impl ImageKey {
    pub fn new(url: &str, target_w: u32, target_h: u32) -> ImageKey {
        ImageKey { url_hash: hash_url(url), target_w, target_h }
    }
}

struct Entry {
    bmp: Arc<Bitmap>,
    used: u64,
}

/// Decoded images under an LRU byte budget.
pub struct ImageCache {
    map: HashMap<ImageKey, Entry>,
    bytes: usize,
    budget: usize,
    tick: u64,
}

impl ImageCache {
    pub fn new(budget: usize) -> ImageCache {
        ImageCache { map: HashMap::new(), bytes: 0, budget, tick: 0 }
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn budget(&self) -> usize {
        self.budget
    }

    /// Looks up and marks as most recently used.
    pub fn get(&mut self, key: &ImageKey) -> Option<Arc<Bitmap>> {
        self.tick += 1;
        let t = self.tick;
        self.map.get_mut(key).map(|e| {
            e.used = t;
            e.bmp.clone()
        })
    }

    /// Adds a decoded bitmap (replacing any under the key) and trims to budget.
    /// A bitmap larger than the whole budget is returned but not retained.
    pub fn insert(&mut self, key: ImageKey, bmp: Bitmap) -> Arc<Bitmap> {
        self.tick += 1;
        let bmp = Arc::new(bmp);
        if let Some(old) = self.map.insert(key, Entry { bmp: bmp.clone(), used: self.tick }) {
            self.bytes -= old.bmp.bytes();
        }
        self.bytes += bmp.bytes();
        self.evict_to(self.budget);
        bmp
    }

    /// Cached bitmap, or decode `bytes` at the key's size and cache it.
    pub fn get_or_decode(&mut self, key: ImageKey, bytes: &[u8], fmt: PixelFormat) -> Result<Arc<Bitmap>, ImgError> {
        if let Some(b) = self.get(&key) {
            return Ok(b);
        }
        let bmp = decode(bytes, key.target_w, key.target_h, fmt)?;
        Ok(self.insert(key, bmp))
    }

    /// Evicts least recently used entries until at most `budget` bytes remain.
    pub fn evict_to(&mut self, budget: usize) {
        while self.bytes > budget {
            let Some((&k, _)) = self.map.iter().min_by_key(|(_, e)| e.used) else { break };
            if let Some(e) = self.map.remove(&k) {
                self.bytes -= e.bmp.bytes();
            }
        }
    }

    /// Empties the cache (tab discard, memory pressure).
    pub fn drop_all(&mut self) {
        self.map.clear();
        self.bytes = 0;
    }

    /// Changes the budget and trims to it.
    pub fn set_budget(&mut self, budget: usize) {
        self.budget = budget;
        self.evict_to(budget);
    }
}

// ------------------------------------------------------- worker interface

/// The caller supplies the thread: anything that can run a job off the UI thread.
pub trait DecodeExecutor {
    fn execute(&self, job: Box<dyn FnOnce() + Send + 'static>);
}

/// Runs jobs immediately on the calling thread (tests, tiny images).
pub struct InlineExecutor;

impl DecodeExecutor for InlineExecutor {
    fn execute(&self, job: Box<dyn FnOnce() + Send + 'static>) {
        job()
    }
}

/// Decodes on the executor's thread and hands the result to `done` there. The
/// UI thread then calls `ImageCache::insert` with it.
pub fn decode_async<F>(ex: &dyn DecodeExecutor, key: ImageKey, bytes: Arc<Vec<u8>>, fmt: PixelFormat, done: F)
where
    F: FnOnce(ImageKey, Result<Bitmap, ImgError>) + Send + 'static,
{
    ex.execute(Box::new(move || {
        let r = decode(&bytes, key.target_w, key.target_h, fmt);
        done(key, r);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png_of(w: u32, h: u32, ct: png::ColorType, data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut e = png::Encoder::new(&mut out, w, h);
            e.set_color(ct);
            e.set_depth(png::BitDepth::Eight);
            let mut wr = e.write_header().unwrap();
            wr.write_image_data(data).unwrap();
        }
        out
    }

    fn rgba_png(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
        let mut d = Vec::new();
        for y in 0..h {
            for x in 0..w {
                d.extend_from_slice(&f(x, y));
            }
        }
        png_of(w, h, png::ColorType::Rgba, &d)
    }

    fn rgb_png(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 3]) -> Vec<u8> {
        let mut d = Vec::new();
        for y in 0..h {
            for x in 0..w {
                d.extend_from_slice(&f(x, y));
            }
        }
        png_of(w, h, png::ColorType::Rgb, &d)
    }

    #[test]
    fn probes_png_header_only() {
        let mut b = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13, b'I', b'H', b'D', b'R'];
        b.extend_from_slice(&640u32.to_be_bytes());
        b.extend_from_slice(&480u32.to_be_bytes());
        assert_eq!(probe(&b), Some((640, 480)));
        assert_eq!(probe(&b[..20]), None);
    }

    #[test]
    fn probes_jpeg_sof() {
        // SOI, APP0 (len 4), SOF0 (len 11): precision 8, h=300, w=500
        let b = [0xFF, 0xD8, 0xFF, 0xE0, 0, 4, 0, 0, 0xFF, 0xC0, 0, 11, 8, 0x01, 0x2C, 0x01, 0xF4, 1, 1, 0x11, 0];
        assert_eq!(probe(&b), Some((500, 300)));
        // DHT (C4) must not be taken for a SOF
        let c = [0xFF, 0xD8, 0xFF, 0xC4, 0, 4, 0, 0, 0xFF, 0xC2, 0, 11, 8, 0, 20, 0, 10, 1, 1, 0x11, 0];
        assert_eq!(probe(&c), Some((10, 20)));
        assert_eq!(probe(&[0xFF, 0xD8, 0xFF]), None);
    }

    #[test]
    fn probes_gif_and_webp() {
        let mut g = b"GIF89a".to_vec();
        g.extend_from_slice(&[0x40, 0x01, 0xC8, 0x00]);
        assert_eq!(probe(&g), Some((320, 200)));
        // VP8X canvas 1000x500 -> stored as 999, 499
        let mut w = b"RIFF\0\0\0\0WEBPVP8X\x0a\0\0\0\0\0\0\0".to_vec();
        w.extend_from_slice(&[0xE7, 0x03, 0x00, 0xF3, 0x01, 0x00]);
        assert_eq!(probe(&w), Some((1000, 500)));
        // VP8L 100x50: bits = 99 | 49<<14
        let bits = 99u32 | (49 << 14);
        let mut l = b"RIFF\0\0\0\0WEBPVP8L\0\0\0\0".to_vec();
        l.push(0x2F);
        l.extend_from_slice(&bits.to_le_bytes());
        assert_eq!(probe(&l), Some((100, 50)));
    }

    #[test]
    fn sniffs_formats() {
        assert_eq!(sniff(b"<svg xmlns='x'/>"), Some(Format::Svg));
        assert_eq!(sniff(b"hello"), None);
        assert_eq!(probe(b"<svg/>"), None);
    }

    #[test]
    fn fit_never_enlarges_and_keeps_aspect() {
        assert_eq!(fit_size(100, 50, 200, 200), (100, 50));
        assert_eq!(fit_size(1000, 500, 100, 100), (100, 50));
        assert_eq!(fit_size(1000, 500, 0, 100), (200, 100));
        assert_eq!(fit_size(1000, 500, 0, 0), (1000, 500));
        assert_eq!(fit_size(1000, 1, 10, 10), (10, 1));
    }

    #[test]
    fn downsampler_averages_blocks() {
        let mut ds = Downsampler::new(4, 4, 2, 2);
        for y in 0..4 {
            let row: Vec<u8> = (0..4).flat_map(|x| if (x + y) % 2 == 0 { [0, 0, 0, 255] } else { [254, 254, 254, 255] }).collect();
            ds.push_row(&row);
        }
        let out = ds.finish();
        assert_eq!(out.len(), 16);
        for p in out.chunks(4) {
            assert_eq!(p, [127, 127, 127, 255]);
        }
    }

    #[test]
    fn downsampler_uneven_ratio_covers_all_pixels() {
        let mut ds = Downsampler::new(7, 5, 3, 2);
        for _ in 0..5 {
            ds.push_row(&[200u8, 100, 50, 255].repeat(7));
        }
        for p in ds.finish().chunks(4) {
            assert_eq!(p, [200, 100, 50, 255]);
        }
    }

    #[test]
    fn downsampler_alpha_weighted() {
        // one opaque red and one fully transparent (colour garbage) pixel -> red at half alpha
        let mut ds = Downsampler::new(2, 1, 1, 1);
        ds.push_row(&[255, 0, 0, 255, 0, 255, 0, 0]);
        assert_eq!(ds.finish(), vec![255, 0, 0, 128]);
    }

    #[test]
    fn decodes_png_at_full_size() {
        let b = rgba_png(4, 2, |x, _| [x as u8 * 60, 1, 2, 255]);
        let bmp = decode(&b, 0, 0, PixelFormat::Rgba8).unwrap();
        assert_eq!((bmp.w, bmp.h), (4, 2));
        assert_eq!(bmp.pixel(3, 1), [180, 1, 2, 255]);
    }

    #[test]
    fn decodes_png_directly_at_display_size() {
        let b = rgb_png(8, 8, |x, _| if x < 4 { [255, 0, 0] } else { [0, 0, 255] });
        let bmp = decode(&b, 4, 4, PixelFormat::Rgba8).unwrap();
        assert_eq!((bmp.w, bmp.h), (4, 4));
        assert_eq!(bmp.bytes(), 4 * 4 * 4);
        assert_eq!(bmp.pixel(0, 0), [255, 0, 0, 255]);
        assert_eq!(bmp.pixel(1, 3), [255, 0, 0, 255]);
        assert_eq!(bmp.pixel(2, 0), [0, 0, 255, 255]);
        assert_eq!(bmp.pixel(3, 3), [0, 0, 255, 255]);
    }

    #[test]
    fn decodes_grayscale_png() {
        let b = png_of(2, 2, png::ColorType::Grayscale, &[0, 100, 200, 255]);
        let bmp = decode(&b, 0, 0, PixelFormat::Rgba8).unwrap();
        assert_eq!(bmp.pixel(1, 0), [100, 100, 100, 255]);
        assert_eq!(bmp.pixel(1, 1), [255, 255, 255, 255]);
    }

    #[test]
    fn rgb565_halves_memory_for_opaque() {
        let b = rgb_png(16, 16, |_, _| [255, 128, 0]);
        let full = decode(&b, 0, 0, PixelFormat::Rgba8).unwrap();
        let small = decode(&b, 0, 0, PixelFormat::Rgb565).unwrap();
        assert_eq!(small.format, PixelFormat::Rgb565);
        assert_eq!(small.bytes() * 2, full.bytes());
        let p = small.pixel(5, 5);
        assert!(p[0] >= 248 && (p[1] as i32 - 128).abs() <= 4 && p[2] <= 7 && p[3] == 255);
    }

    #[test]
    fn rgb565_falls_back_for_transparency() {
        let b = rgba_png(2, 2, |x, _| [10, 20, 30, if x == 0 { 0 } else { 255 }]);
        let bmp = decode(&b, 0, 0, PixelFormat::Rgb565).unwrap();
        assert_eq!(bmp.format, PixelFormat::Rgba8);
    }

    #[test]
    fn rejects_over_64_megapixels_from_header() {
        let mut b = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13, b'I', b'H', b'D', b'R'];
        b.extend_from_slice(&9000u32.to_be_bytes());
        b.extend_from_slice(&9000u32.to_be_bytes()); // 81 MP, no pixel data follows
        assert_eq!(decode(&b, 100, 100, PixelFormat::Rgba8), Err(ImgError::TooLarge));
        let mut c = b.clone();
        c[16..20].copy_from_slice(&20000u32.to_be_bytes());
        c[20..24].copy_from_slice(&1u32.to_be_bytes());
        assert_eq!(decode(&c, 0, 0, PixelFormat::Rgba8), Err(ImgError::TooLarge));
    }

    #[test]
    fn unsupported_and_corrupt() {
        assert_eq!(decode(b"GIF89a\x01\0\x01\0", 0, 0, PixelFormat::Rgba8), Err(ImgError::Unsupported));
        assert_eq!(decode(b"junk", 0, 0, PixelFormat::Rgba8), Err(ImgError::Unsupported));
        let good = rgba_png(4, 4, |_, _| [1, 2, 3, 255]);
        assert!(decode(&good[..good.len() / 2], 0, 0, PixelFormat::Rgba8).is_err());
    }

    fn bmp(n: usize) -> Bitmap {
        Bitmap { w: 1, h: n as u32, format: PixelFormat::Rgb565, data: vec![0; n * 2] }
    }

    #[test]
    fn cache_evicts_lru_to_budget() {
        let mut c = ImageCache::new(100);
        let (a, b, d) = (ImageKey::new("a", 1, 1), ImageKey::new("b", 1, 1), ImageKey::new("c", 1, 1));
        c.insert(a, bmp(20)); // 40 bytes
        c.insert(b, bmp(20));
        assert_eq!(c.bytes(), 80);
        assert!(c.get(&a).is_some()); // a is now newer than b
        c.insert(d, bmp(20)); // 120 > 100 -> evict b
        assert!(c.get(&b).is_none());
        assert!(c.get(&a).is_some() && c.get(&d).is_some());
        assert_eq!(c.bytes(), 80);
        c.evict_to(40);
        assert_eq!(c.len(), 1);
        c.drop_all();
        assert_eq!((c.bytes(), c.len()), (0, 0));
    }

    #[test]
    fn cache_does_not_retain_oversized() {
        let mut c = ImageCache::new(10);
        let r = c.insert(ImageKey::new("big", 0, 0), bmp(100));
        assert_eq!(r.bytes(), 200);
        assert!(c.is_empty());
        assert_eq!(c.bytes(), 0);
    }

    #[test]
    fn cache_replace_keeps_accounting() {
        let mut c = ImageCache::new(1000);
        let k = ImageKey::new("x", 0, 0);
        c.insert(k, bmp(10));
        c.insert(k, bmp(30));
        assert_eq!((c.len(), c.bytes()), (1, 60));
    }

    #[test]
    fn get_or_decode_caches_by_size_key() {
        let b = rgb_png(8, 8, |_, _| [9, 9, 9]);
        let mut c = ImageCache::new(1 << 20);
        let k1 = ImageKey::new("u", 4, 4);
        let one = c.get_or_decode(k1, &b, PixelFormat::Rgba8).unwrap();
        assert_eq!((one.w, one.h), (4, 4));
        // second call needs no bytes
        assert!(Arc::ptr_eq(&one, &c.get_or_decode(k1, &[], PixelFormat::Rgba8).unwrap()));
        let two = c.get_or_decode(ImageKey::new("u", 2, 2), &b, PixelFormat::Rgba8).unwrap();
        assert_eq!(two.w, 2);
        assert_eq!(c.len(), 2);
        assert_eq!(c.get_or_decode(ImageKey::new("bad", 0, 0), b"nope", PixelFormat::Rgba8), Err(ImgError::Unsupported));
    }

    #[test]
    fn async_decode_runs_on_supplied_executor() {
        use std::sync::mpsc;
        let b = Arc::new(rgb_png(4, 4, |_, _| [1, 2, 3]));
        let (tx, rx) = mpsc::channel();
        let key = ImageKey::new("z", 2, 2);
        decode_async(&InlineExecutor, key, b, PixelFormat::Rgba8, move |k, r| tx.send((k, r)).unwrap());
        let (k, r) = rx.recv().unwrap();
        assert_eq!(k, key);
        assert_eq!(r.unwrap().w, 2);
    }

    #[test]
    fn hash_is_stable_and_distinct() {
        assert_eq!(hash_url("a"), hash_url("a"));
        assert_ne!(hash_url("a"), hash_url("b"));
    }
}
