//! TIFF: strips of 1 to 16 bit gray, palette, RGB, CMYK and alpha samples, uncompressed / PackBits / LZW / Deflate
//! with the horizontal predictor, several images per file; and a writer for RGB(A) 8-bit pages.

use super::exif::{self, Block, ColorModel, Dict, Entry, OutEntry, PropValue, Properties};
use super::{Decoded, Frame, MAX_SIDE};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compression {
    None,
    PackBits,
    Deflate,
}

fn first(b: &Block, e: &[Entry], tag: u16) -> Option<i64> {
    e.iter().find(|x| x.tag == tag).and_then(|x| b.uints(x).first().copied())
}

fn all(b: &Block, e: &[Entry], tag: u16) -> Vec<i64> {
    e.iter().find(|x| x.tag == tag).map(|x| b.uints(x)).unwrap_or_default()
}

/// Offsets of every image file directory.
fn ifds(b: &Block) -> Vec<usize> {
    let mut out = Vec::new();
    let mut off = b.first_ifd().unwrap_or(0);
    while off != 0 && out.len() < 4096 && !out.contains(&off) {
        match b.ifd(off) {
            Some((_, next)) => {
                out.push(off);
                off = next;
            }
            None => break,
        }
    }
    out
}

pub fn count(data: &[u8]) -> Option<usize> {
    let b = Block::new(data)?;
    Some(ifds(&b).len()).filter(|&n| n > 0)
}

fn props_of(b: &Block, entries: &[Entry], data: &[u8], page0: bool) -> Option<Properties> {
    let w = first(b, entries, 256)? as usize;
    let h = first(b, entries, 257)? as usize;
    if w == 0 || h == 0 {
        return None;
    }
    let mut p = Properties::new(w, h);
    let spp = first(b, entries, 277).unwrap_or(1);
    let photo = first(b, entries, 262).unwrap_or(1);
    p.depth = first(b, entries, 258).unwrap_or(1) as u32;
    p.color_model = match photo {
        0 | 1 => ColorModel::Gray,
        3 => ColorModel::Indexed,
        5 => ColorModel::Cmyk,
        _ => ColorModel::Rgb,
    };
    p.is_indexed = photo == 3;
    p.has_alpha = (photo == 2 && spp >= 4) || (matches!(photo, 0 | 1) && spp >= 2);
    if page0 {
        p.absorb_exif(data);
    }
    for e in entries {
        if e.tag == 34675 {
            p.icc_profile = data.get(e.at..e.at + e.count).map(|s| s.to_vec());
        }
    }
    if let Some(c) = first(b, entries, 259) {
        p.tiff.insert("Compression".into(), PropValue::Int(c));
    }
    Some(p)
}

pub fn probe(data: &[u8]) -> Option<Properties> {
    let b = Block::new(data)?;
    let (entries, _) = b.ifd(b.first_ifd()?)?;
    props_of(&b, &entries, data, true)
}

fn unpackbits(src: &[u8], want: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(want);
    let mut i = 0;
    while i < src.len() && out.len() < want {
        let n = src[i] as i8;
        i += 1;
        if n >= 0 {
            let c = n as usize + 1;
            out.extend_from_slice(src.get(i..(i + c).min(src.len())).unwrap_or(&[]));
            i += c;
        } else if n != -128 {
            let c = 1 - n as isize;
            if let Some(&v) = src.get(i) {
                out.extend(std::iter::repeat(v).take(c as usize));
            }
            i += 1;
        }
    }
    out
}

fn lzw(src: &[u8], want: usize) -> Vec<u8> {
    let mut prefix = vec![0u16; 4096];
    let mut suffix = vec![0u8; 4096];
    let mut out = Vec::with_capacity(want);
    let (mut width, mut next, mut prev): (u32, u32, Option<u32>) = (9, 258, None);
    let (mut acc, mut nbits, mut pos) = (0u32, 0u32, 0usize);
    let mut buf = Vec::new();
    let expand = |code: u32, prefix: &[u16], suffix: &[u8], buf: &mut Vec<u8>| {
        buf.clear();
        let mut c = code;
        while c >= 258 {
            buf.push(suffix[c as usize]);
            c = prefix[c as usize] as u32;
        }
        buf.push(c as u8);
        buf.reverse();
    };
    while out.len() < want {
        while nbits < width {
            let Some(&b) = src.get(pos) else { return out };
            acc = (acc << 8) | b as u32;
            nbits += 8;
            pos += 1;
        }
        let code = (acc >> (nbits - width)) & ((1 << width) - 1);
        nbits -= width;
        acc &= (1 << nbits) - 1;
        if code == 256 {
            width = 9;
            next = 258;
            prev = None;
            continue;
        }
        if code == 257 {
            break;
        }
        match prev {
            None => {
                if code >= 256 {
                    break;
                }
                out.push(code as u8);
            }
            Some(p) => {
                if code > next {
                    break;
                }
                let f;
                if code < next {
                    expand(code, &prefix, &suffix, &mut buf);
                    f = buf[0];
                    out.extend_from_slice(&buf);
                } else {
                    expand(p, &prefix, &suffix, &mut buf);
                    f = buf[0];
                    out.extend_from_slice(&buf);
                    out.push(f);
                }
                if next < 4096 {
                    prefix[next as usize] = p as u16;
                    suffix[next as usize] = f;
                    next += 1;
                    if next == (1 << width) - 1 && width < 12 {
                        width += 1;
                    }
                }
            }
        }
        prev = Some(code);
    }
    out
}

fn decode_page(b: &Block, entries: &[Entry], data: &[u8], page0: bool) -> Option<(Frame, Properties)> {
    let props = props_of(b, entries, data, page0)?;
    let (w, h) = (props.pixel_width, props.pixel_height);
    if w > MAX_SIDE || h > MAX_SIDE || entries.iter().any(|e| e.tag == 322) {
        return None; // tiled images are not read
    }
    let spp = first(b, entries, 277).unwrap_or(1) as usize;
    let bps = first(b, entries, 258).unwrap_or(1) as usize;
    let comp = first(b, entries, 259).unwrap_or(1);
    let photo = first(b, entries, 262).unwrap_or(1);
    let planar = first(b, entries, 284).unwrap_or(1);
    let predictor = first(b, entries, 317).unwrap_or(1);
    let sample_format = first(b, entries, 339).unwrap_or(1);
    if planar != 1 || sample_format != 1 || !matches!(bps, 1 | 2 | 4 | 8 | 16) || spp == 0 || spp > 5 {
        return None;
    }
    let rps = (first(b, entries, 278).unwrap_or(h as i64) as usize).clamp(1, h);
    let offsets = all(b, entries, 273);
    let counts = all(b, entries, 279);
    let row_len = (w * spp * bps).div_ceil(8);
    let mut raw = Vec::with_capacity(row_len * h);
    for (i, &off) in offsets.iter().enumerate() {
        let rows = rps.min(h.saturating_sub(i * rps));
        if rows == 0 {
            break;
        }
        let len = counts.get(i).copied().unwrap_or(row_len as i64 * rows as i64) as usize;
        let src = data.get(off as usize..(off as usize).checked_add(len)?)?;
        let want = row_len * rows;
        let mut strip = match comp {
            1 => src.to_vec(),
            5 => lzw(src, want),
            8 | 32946 => miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(src, want + 16).ok()?,
            32773 => unpackbits(src, want),
            _ => return None,
        };
        strip.resize(want, 0);
        if predictor == 2 {
            for r in 0..rows {
                let row = &mut strip[r * row_len..(r + 1) * row_len];
                match bps {
                    8 => {
                        for x in spp..w * spp {
                            row[x] = row[x].wrapping_add(row[x - spp]);
                        }
                    }
                    16 => {
                        for x in spp..w * spp {
                            let get = |i: usize| if b.le { u16::from_le_bytes([row[i * 2], row[i * 2 + 1]]) } else { u16::from_be_bytes([row[i * 2], row[i * 2 + 1]]) };
                            let v = get(x).wrapping_add(get(x - spp));
                            let bytes = if b.le { v.to_le_bytes() } else { v.to_be_bytes() };
                            row[x * 2] = bytes[0];
                            row[x * 2 + 1] = bytes[1];
                        }
                    }
                    _ => {}
                }
            }
        }
        raw.extend(strip);
    }
    if raw.len() < row_len * h {
        return None;
    }
    let extra = first(b, entries, 338).unwrap_or(0);
    let palette: Vec<u16> = entries.iter().find(|e| e.tag == 320).map(|e| (0..e.count).filter_map(|i| b.u16(e.at + i * 2)).collect()).unwrap_or_default();
    // sample (x, y, c) scaled to 0..=65535
    let sample = |x: usize, y: usize, c: usize| -> u32 {
        let row = &raw[y * row_len..];
        match bps {
            8 => row[x * spp + c] as u32 * 257,
            16 => {
                let i = (x * spp + c) * 2;
                (if b.le { u16::from_le_bytes([row[i], row[i + 1]]) } else { u16::from_be_bytes([row[i], row[i + 1]]) }) as u32
            }
            _ => {
                let bit = (x * spp + c) * bps;
                let v = (row[bit / 8] >> (8 - bps - bit % 8)) as u32 & ((1 << bps) - 1);
                v * 65535 / ((1 << bps) - 1)
            }
        }
    };
    let raw_index = |x: usize, y: usize| -> usize {
        let bit = x * spp * bps;
        let row = &raw[y * row_len..];
        match bps {
            8 => row[x * spp] as usize,
            16 => 0,
            _ => ((row[bit / 8] >> (8 - bps - bit % 8)) as usize) & ((1 << bps) - 1),
        }
    };
    let wide = bps == 16;
    let mut rgba16 = Vec::with_capacity(if wide { w * h * 4 } else { 0 });
    let mut rgba8 = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            let mut px: [u32; 4] = match photo {
                0 | 1 => {
                    let mut g = sample(x, y, 0);
                    if photo == 0 {
                        g = 65535 - g;
                    }
                    [g, g, g, if spp >= 2 { sample(x, y, 1) } else { 65535 }]
                }
                2 => [sample(x, y, 0), sample(x, y, 1), sample(x, y, 2), if spp >= 4 { sample(x, y, 3) } else { 65535 }],
                3 => {
                    let i = raw_index(x, y);
                    let n = palette.len() / 3;
                    if n == 0 || i >= n {
                        [0, 0, 0, 65535]
                    } else {
                        [palette[i] as u32, palette[n + i] as u32, palette[2 * n + i] as u32, 65535]
                    }
                }
                5 if spp >= 4 => {
                    let k = 65535 - sample(x, y, 3);
                    let ch = |c: usize| (65535 - sample(x, y, c)) * k / 65535;
                    [ch(0), ch(1), ch(2), 65535]
                }
                _ => return None,
            };
            if extra == 1 && px[3] > 0 && px[3] < 65535 {
                let a = px[3];
                for c in px.iter_mut().take(3) {
                    *c = (*c * 65535 / a).min(65535);
                }
            }
            rgba8.extend(px.iter().map(|&v| (v >> 8) as u8));
            if wide {
                rgba16.extend(px.iter().map(|&v| v as u16));
            }
        }
    }
    Some((Frame { w, h, rgba8, rgba16: wide.then_some(rgba16), delay_ms: 0 }, props))
}

pub fn decode(data: &[u8]) -> Option<Decoded> {
    let b = Block::new(data)?;
    let mut frames = Vec::new();
    let mut props = Vec::new();
    for (i, off) in ifds(&b).into_iter().enumerate() {
        let (entries, _) = b.ifd(off)?;
        if let Some((f, p)) = decode_page(&b, &entries, data, i == 0) {
            frames.push(f);
            props.push(p);
        }
    }
    (!frames.is_empty()).then_some(Decoded { frames, props, loop_count: None })
}

fn packbits(row: &[u8], out: &mut Vec<u8>) {
    let mut i = 0;
    while i < row.len() {
        let mut run = 1;
        while i + run < row.len() && row[i + run] == row[i] && run < 128 {
            run += 1;
        }
        if run >= 2 {
            out.push((1 - run as i32) as u8);
            out.push(row[i]);
            i += run;
        } else {
            let start = i;
            while i < row.len() && i - start < 128 && !(i + 1 < row.len() && row[i + 1] == row[i]) {
                i += 1;
            }
            if i == start {
                i += 1;
            }
            out.push((i - start - 1) as u8);
            out.extend_from_slice(&row[start..i]);
        }
    }
}

fn ent(tag: u16, typ: u16, vals: &[u32]) -> OutEntry {
    let data: Vec<u8> = if typ == 3 { vals.iter().flat_map(|&v| (v as u16).to_le_bytes()).collect() } else { vals.iter().flat_map(|v| v.to_le_bytes()).collect() };
    OutEntry { tag, typ, count: vals.len() as u32, data }
}

/// One page to write: straight 8-bit RGBA pixels with its metadata dictionaries.
pub struct Page<'a> {
    pub w: usize,
    pub h: usize,
    pub rgba: &'a [u8],
    pub dpi: Option<(f64, f64)>,
    pub tiff: &'a Dict,
    pub exif: &'a Dict,
    pub gps: &'a Dict,
    pub icc: Option<&'a [u8]>,
}

/// Writes a little-endian TIFF with one image directory per page.
pub fn encode(pages: &[Page], compression: Compression) -> Option<Vec<u8>> {
    if pages.is_empty() {
        return None;
    }
    let mut out = vec![b'I', b'I', 42, 0, 0, 0, 0, 0];
    let mut prev_next_at: Option<usize> = None;
    for pg in pages {
        if pg.w == 0 || pg.h == 0 || pg.rgba.len() < pg.w * pg.h * 4 {
            return None;
        }
        let alpha = pg.rgba.chunks_exact(4).any(|p| p[3] != 255);
        let spp = if alpha { 4 } else { 3 };
        let mut raw = Vec::with_capacity(pg.w * pg.h * spp);
        let mut row_bufs: Vec<u8> = Vec::new();
        for y in 0..pg.h {
            let mut row = Vec::with_capacity(pg.w * spp);
            for x in 0..pg.w {
                let p = &pg.rgba[(y * pg.w + x) * 4..][..4];
                row.extend_from_slice(&p[..spp]);
            }
            if compression == Compression::PackBits {
                packbits(&row, &mut row_bufs);
            } else {
                raw.extend(row);
            }
        }
        let (code, strip) = match compression {
            Compression::None => (1u32, raw),
            Compression::PackBits => (32773, row_bufs),
            Compression::Deflate => (8, miniz_oxide::deflate::compress_to_vec_zlib(&raw, 6)),
        };
        let strip_off = out.len();
        out.extend(&strip);
        if out.len() % 2 == 1 {
            out.push(0);
        }
        let sub = |out: &mut Vec<u8>, table: &[(u16, &str)], d: &Dict| -> u32 {
            let at = out.len();
            let b = exif::ifd_bytes(exif::entries_of(table, d), at);
            out.extend(b);
            at as u32
        };
        let exif_ptr = (!pg.exif.is_empty()).then(|| sub(&mut out, exif::EXIF_TAGS, pg.exif));
        let gps_ptr = (!pg.gps.is_empty()).then(|| sub(&mut out, exif::GPS_TAGS, pg.gps));
        let mut meta = pg.tiff.clone();
        for k in ["XResolution", "YResolution", "ResolutionUnit"] {
            meta.remove(k);
        }
        let mut e = exif::entries_of(exif::TIFF_TAGS, &meta);
        e.push(ent(256, 4, &[pg.w as u32]));
        e.push(ent(257, 4, &[pg.h as u32]));
        e.push(ent(258, 3, &vec![8; spp]));
        e.push(ent(259, 3, &[code]));
        e.push(ent(262, 3, &[2]));
        e.push(ent(273, 4, &[strip_off as u32]));
        e.push(ent(277, 3, &[spp as u32]));
        e.push(ent(278, 4, &[pg.h as u32]));
        e.push(ent(279, 4, &[strip.len() as u32]));
        e.push(ent(284, 3, &[1]));
        if alpha {
            e.push(ent(338, 3, &[2]));
        }
        if let Some((dx, dy)) = pg.dpi {
            for (tag, v) in [(282u16, dx), (283, dy)] {
                let mut d = ((v * 1000.0).round() as u32).to_le_bytes().to_vec();
                d.extend(1000u32.to_le_bytes());
                e.push(OutEntry { tag, typ: 5, count: 1, data: d });
            }
            e.push(ent(296, 3, &[2]));
        }
        if let Some(icc) = pg.icc {
            e.push(OutEntry { tag: 34675, typ: 7, count: icc.len() as u32, data: icc.to_vec() });
        }
        if let Some(p) = exif_ptr {
            e.push(ent(0x8769, 4, &[p]));
        }
        if let Some(p) = gps_ptr {
            e.push(ent(0x8825, 4, &[p]));
        }
        let n = e.len();
        let ifd_at = out.len();
        out.extend(exif::ifd_bytes(e, ifd_at));
        let next_at = ifd_at + 2 + n * 12;
        match prev_next_at {
            Some(at) => out[at..at + 4].copy_from_slice(&(ifd_at as u32).to_le_bytes()),
            None => out[4..8].copy_from_slice(&(ifd_at as u32).to_le_bytes()),
        }
        prev_next_at = Some(next_at);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixels() -> Vec<u8> {
        let mut v = Vec::new();
        for y in 0..5u8 {
            for x in 0..7u8 {
                v.extend([x * 30, y * 50, 200, if x == 6 && y == 4 { 90 } else { 255 }]);
            }
        }
        v
    }

    fn page<'a>(px: &'a [u8], d: &'a Dict) -> Page<'a> {
        Page { w: 7, h: 5, rgba: px, dpi: Some((300.0, 300.0)), tiff: d, exif: d, gps: d, icc: None }
    }

    #[test]
    fn every_compression_round_trips() {
        let px = pixels();
        let d = Dict::new();
        for c in [Compression::None, Compression::PackBits, Compression::Deflate] {
            let t = encode(&[page(&px, &d)], c).unwrap();
            let p = probe(&t).unwrap();
            assert_eq!((p.pixel_width, p.pixel_height), (7, 5), "{c:?}");
            assert!(p.has_alpha);
            assert_eq!(p.dpi_width, Some(300.0));
            let got = decode(&t).unwrap();
            assert_eq!(got.frames[0].rgba8, px, "{c:?}");
        }
    }

    #[test]
    fn several_pages_and_metadata() {
        let px = pixels();
        let mut tiff = Dict::new();
        tiff.insert("Make".into(), PropValue::Text("BifrOSt".into()));
        tiff.insert("Orientation".into(), PropValue::Int(3));
        let mut ex = Dict::new();
        ex.insert("ISOSpeedRatings".into(), PropValue::Int(100));
        let empty = Dict::new();
        let p0 = Page { w: 7, h: 5, rgba: &px, dpi: None, tiff: &tiff, exif: &ex, gps: &empty, icc: None };
        let t = encode(&[p0, page(&px, &empty)], Compression::None).unwrap();
        assert_eq!(count(&t), Some(2));
        let d = decode(&t).unwrap();
        assert_eq!(d.frames.len(), 2);
        assert_eq!(d.props[0].orientation, 3);
        assert_eq!(d.props[0].tiff["Make"].as_str(), Some("BifrOSt"));
        assert_eq!(d.props[0].exif["ISOSpeedRatings"], PropValue::Int(100));
    }

    #[test]
    fn packbits_and_lzw_decode_known_streams() {
        // PackBits example from the specification.
        let enc = [0xFE, 0xAA, 0x02, 0x80, 0x00, 0x2A, 0xFD, 0xAA, 0x03, 0x80, 0x00, 0x2A, 0x22, 0xF7, 0xAA];
        assert_eq!(unpackbits(&enc, 24).len(), 24);
        assert_eq!(&unpackbits(&enc, 24)[..3], &[0xAA, 0xAA, 0xAA]);
        let mut r = Vec::new();
        packbits(&[5, 5, 5, 5, 1, 2, 3], &mut r);
        assert_eq!(unpackbits(&r, 7), vec![5, 5, 5, 5, 1, 2, 3]);
    }

    #[test]
    fn unsupported_and_bad_data() {
        assert!(decode(b"II*\0\x08\0\0\0").is_none());
        assert!(probe(b"nope").is_none());
        assert!(encode(&[], Compression::None).is_none());
    }
}
