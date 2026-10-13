//! BMP and ICO: device-independent bitmaps of 1, 4, 8, 16, 24 and 32 bits, run-length coded 4/8-bit data, bit
//! masks, palettes and bottom-up or top-down rows; the ICO/CUR directory with PNG or bitmap frames; and a
//! BMP writer.

use super::exif::{ColorModel, Properties};
use super::{Decoded, Frame, MAX_SIDE};

fn le16(d: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(d.get(o..o + 2)?.try_into().ok()?))
}
fn le32(d: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(d.get(o..o + 4)?.try_into().ok()?))
}

/// The size of the bitmap in a `BM` file, from its header alone.
pub fn probe(data: &[u8]) -> Option<Properties> {
    if data.get(..2)? != b"BM" {
        return None;
    }
    probe_dib(data.get(14..)?, false)
}

fn probe_dib(dib: &[u8], ico: bool) -> Option<Properties> {
    let hs = le32(dib, 0)? as usize;
    let (w, h, bpp, xppm, yppm) = if hs == 12 {
        (le16(dib, 4)? as i64, le16(dib, 6)? as i64, le16(dib, 10)?, 0.0, 0.0)
    } else if hs >= 40 {
        (le32(dib, 4)? as i32 as i64, le32(dib, 8)? as i32 as i64, le16(dib, 14)?, le32(dib, 24)? as i32 as f64, le32(dib, 28)? as i32 as f64)
    } else {
        return None;
    };
    let mut h = h.abs();
    if ico {
        h /= 2;
    }
    if w <= 0 || h <= 0 {
        return None;
    }
    let mut p = Properties::new(w as usize, h as usize);
    p.depth = if bpp <= 8 { bpp as u32 } else { 8 };
    p.is_indexed = bpp <= 8;
    p.color_model = if p.is_indexed { ColorModel::Indexed } else { ColorModel::Rgb };
    p.has_alpha = bpp == 32;
    if xppm > 0.0 && yppm > 0.0 {
        p.dpi_width = Some(xppm * 0.0254);
        p.dpi_height = Some(yppm * 0.0254);
    }
    Some(p)
}

pub fn decode(data: &[u8]) -> Option<Decoded> {
    if data.get(..2)? != b"BM" {
        return None;
    }
    let off = le32(data, 10)? as usize;
    let (frame, props) = decode_dib(data.get(14..)?, off.checked_sub(14), false)?;
    Some(Decoded { frames: vec![frame], props: vec![props], loop_count: None })
}

fn shift_bits(mask: u32) -> (u32, u32) {
    if mask == 0 {
        (0, 0)
    } else {
        (mask.trailing_zeros(), mask.count_ones())
    }
}

fn extract(px: u32, mask: u32) -> u8 {
    let (s, bits) = shift_bits(mask);
    if bits == 0 {
        return 0;
    }
    let v = (px & mask) >> s;
    let max = if bits >= 32 { u32::MAX } else { (1u32 << bits) - 1 };
    ((v as u64 * 255 + max as u64 / 2) / max as u64) as u8
}

/// Decodes a bitmap that starts at its info header. `pix_off` is the pixel data offset within `dib` (None: right
/// after the palette). With `ico`, the height is doubled and a 1-bit AND mask follows the colour data.
pub(super) fn decode_dib(dib: &[u8], pix_off: Option<usize>, ico: bool) -> Option<(Frame, Properties)> {
    let mut props = probe_dib(dib, ico)?;
    let (w, h) = (props.pixel_width, props.pixel_height);
    if w > MAX_SIDE || h > MAX_SIDE {
        return None;
    }
    let hs = le32(dib, 0)? as usize;
    let core = hs == 12;
    let raw_h = if core { le16(dib, 6)? as i32 } else { le32(dib, 8)? as i32 };
    let top_down = raw_h < 0 && !ico;
    let bpp = if core { le16(dib, 10)? } else { le16(dib, 14)? } as usize;
    let comp = if core { 0 } else { le32(dib, 16)? };
    let clr_used = if core { 0 } else { le32(dib, 32)? as usize };
    if !matches!(bpp, 1 | 2 | 4 | 8 | 16 | 24 | 32) || comp > 6 {
        return None;
    }
    let (mut mr, mut mg, mut mb, mut ma) = match bpp {
        16 => (0x7C00, 0x03E0, 0x001F, 0),
        _ => (0x00FF_0000, 0x0000_FF00, 0x0000_00FF, 0),
    };
    let mut pal_off = hs;
    if comp == 3 || comp == 6 || hs >= 52 {
        if hs >= 52 {
            (mr, mg, mb) = (le32(dib, 40)?, le32(dib, 44)?, le32(dib, 48)?);
            if hs >= 56 {
                ma = le32(dib, 52)?;
            }
        } else {
            (mr, mg, mb) = (le32(dib, 40)?, le32(dib, 44)?, le32(dib, 48)?);
            pal_off = 52;
            if comp == 6 {
                ma = le32(dib, 52)?;
                pal_off = 56;
            }
        }
    }
    let esz = if core { 3 } else { 4 };
    let ncol = if bpp <= 8 { if clr_used > 0 { clr_used.min(1 << bpp) } else { 1 << bpp } } else { 0 };
    let mut palette = Vec::with_capacity(ncol);
    for i in 0..ncol {
        let o = pal_off + i * esz;
        let e = dib.get(o..o + 3)?;
        palette.push([e[2], e[1], e[0]]);
    }
    let pix = pix_off.unwrap_or(pal_off + ncol * esz);
    let stride = (w * bpp).div_ceil(32) * 4;
    let mut rgba = vec![0u8; w * h * 4];
    let row_out = |y: usize| if top_down { y } else { h - 1 - y };

    if comp == 1 || comp == 2 {
        // run-length coded indices, rows bottom-up
        let mut idx = vec![0u8; w * h];
        let (mut x, mut y) = (0usize, 0usize);
        let mut i = pix;
        'rle: while i + 1 < dib.len() && y < h {
            let (n, v) = (dib[i] as usize, dib[i + 1]);
            i += 2;
            if n > 0 {
                for k in 0..n {
                    let val = if comp == 1 { v } else if k % 2 == 0 { v >> 4 } else { v & 15 };
                    if x < w {
                        idx[row_out(y) * w + x] = val;
                    }
                    x += 1;
                }
                continue;
            }
            match v {
                0 => {
                    x = 0;
                    y += 1;
                }
                1 => break 'rle,
                2 => {
                    x += *dib.get(i)? as usize;
                    y += *dib.get(i + 1)? as usize;
                    i += 2;
                }
                cnt => {
                    let cnt = cnt as usize;
                    let bytes = if comp == 1 { cnt } else { cnt.div_ceil(2) };
                    let src = dib.get(i..i + bytes)?;
                    for k in 0..cnt {
                        let val = if comp == 1 { src[k] } else if k % 2 == 0 { src[k / 2] >> 4 } else { src[k / 2] & 15 };
                        if x < w {
                            idx[row_out(y) * w + x] = val;
                        }
                        x += 1;
                    }
                    i += bytes + (bytes & 1);
                }
            }
        }
        for (k, &v) in idx.iter().enumerate() {
            let c = palette.get(v as usize).copied().unwrap_or([0, 0, 0]);
            rgba[k * 4..k * 4 + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
        }
    } else {
        for y in 0..h {
            let row = dib.get(pix + y * stride..pix + y * stride + (w * bpp).div_ceil(8))?;
            let oy = row_out(y);
            for x in 0..w {
                let o = (oy * w + x) * 4;
                let px: [u8; 4] = match bpp {
                    1 | 2 | 4 | 8 => {
                        let bit = x * bpp;
                        let v = (row[bit / 8] >> (8 - bpp - bit % 8)) & ((1u16 << bpp) - 1) as u8;
                        let c = palette.get(v as usize).copied().unwrap_or([0, 0, 0]);
                        [c[0], c[1], c[2], 255]
                    }
                    16 => {
                        let v = u16::from_le_bytes([row[x * 2], row[x * 2 + 1]]) as u32;
                        [extract(v, mr), extract(v, mg), extract(v, mb), if ma != 0 { extract(v, ma) } else { 255 }]
                    }
                    24 => [row[x * 3 + 2], row[x * 3 + 1], row[x * 3], 255],
                    _ => {
                        let v = u32::from_le_bytes([row[x * 4], row[x * 4 + 1], row[x * 4 + 2], row[x * 4 + 3]]);
                        [extract(v, mr), extract(v, mg), extract(v, mb), if ma != 0 { extract(v, ma) } else { row[x * 4 + 3] }]
                    }
                };
                rgba[o..o + 4].copy_from_slice(&px);
            }
        }
        if bpp == 32 && ma == 0 && rgba.chunks_exact(4).all(|p| p[3] == 0) {
            rgba.chunks_exact_mut(4).for_each(|p| p[3] = 255);
        }
    }
    if ico {
        let color_bytes = if comp == 1 || comp == 2 { 0 } else { stride * h };
        let mask_at = pix + color_bytes;
        let mstride = w.div_ceil(32) * 4;
        let has_alpha = bpp == 32 && rgba.chunks_exact(4).any(|p| p[3] != 255);
        if !has_alpha && comp != 1 && comp != 2 {
            for y in 0..h {
                let Some(row) = dib.get(mask_at + y * mstride..mask_at + y * mstride + w.div_ceil(8)) else { break };
                let oy = h - 1 - y;
                for x in 0..w {
                    if row[x / 8] >> (7 - x % 8) & 1 == 1 {
                        rgba[(oy * w + x) * 4 + 3] = 0;
                    }
                }
            }
        }
    }
    props.has_alpha = props.has_alpha || ico;
    Some((Frame { w, h, rgba8: rgba, rgba16: None, delay_ms: 0 }, props))
}

/// Encodes straight RGBA as a BMP: 24-bit when fully opaque, otherwise 32-bit with an alpha mask.
pub fn encode(w: usize, h: usize, rgba: &[u8], dpi: Option<(f64, f64)>) -> Vec<u8> {
    let alpha = rgba.chunks_exact(4).any(|p| p[3] != 255);
    let bpp = if alpha { 32 } else { 24 };
    let stride = (w * bpp).div_ceil(32) * 4;
    let hs = if alpha { 108 } else { 40 };
    let off = 14 + hs;
    let size = off + stride * h;
    let mut o = Vec::with_capacity(size);
    o.extend(b"BM");
    o.extend((size as u32).to_le_bytes());
    o.extend([0u8; 4]);
    o.extend((off as u32).to_le_bytes());
    o.extend((hs as u32).to_le_bytes());
    o.extend((w as i32).to_le_bytes());
    o.extend((h as i32).to_le_bytes());
    o.extend(1u16.to_le_bytes());
    o.extend((bpp as u16).to_le_bytes());
    o.extend((if alpha { 3u32 } else { 0 }).to_le_bytes());
    o.extend(((stride * h) as u32).to_le_bytes());
    let ppm = |d: f64| (d / 0.0254).round() as i32;
    let (dx, dy) = dpi.unwrap_or((72.0, 72.0));
    o.extend(ppm(dx).to_le_bytes());
    o.extend(ppm(dy).to_le_bytes());
    o.extend([0u8; 8]);
    if alpha {
        for m in [0x00FF_0000u32, 0x0000_FF00, 0x0000_00FF, 0xFF00_0000] {
            o.extend(m.to_le_bytes());
        }
        o.extend(*b"sRGB");
        o.extend([0u8; 48]); // endpoints and gamma
    }
    for y in (0..h).rev() {
        let start = o.len();
        for x in 0..w {
            let p = &rgba[(y * w + x) * 4..][..4];
            o.extend([p[2], p[1], p[0]]);
            if alpha {
                o.push(p[3]);
            }
        }
        o.resize(start + stride, 0);
    }
    o
}

/// The ICO/CUR directory: number of images.
pub fn ico_count(data: &[u8]) -> Option<usize> {
    if le16(data, 0)? != 0 || !matches!(le16(data, 2)?, 1 | 2) {
        return None;
    }
    let n = le16(data, 4)? as usize;
    (n > 0).then_some(n)
}

fn ico_entry(data: &[u8], i: usize) -> Option<&[u8]> {
    let e = 6 + i * 16;
    let size = le32(data, e + 8)? as usize;
    let off = le32(data, e + 12)? as usize;
    data.get(off..off.checked_add(size)?)
}

pub fn ico_probe(data: &[u8]) -> Option<Properties> {
    ico_count(data)?;
    let img = ico_entry(data, 0)?;
    if img.starts_with(&super::png::SIGNATURE) {
        super::png::probe(img)
    } else {
        probe_dib(img, true)
    }
}

pub fn ico_decode(data: &[u8]) -> Option<Decoded> {
    let n = ico_count(data)?;
    let mut frames = Vec::new();
    let mut props = Vec::new();
    for i in 0..n {
        let Some(img) = ico_entry(data, i) else { continue };
        if img.starts_with(&super::png::SIGNATURE) {
            if let Some(mut d) = super::png::decode(img) {
                frames.push(d.frames.remove(0));
                props.push(d.props.remove(0));
            }
        } else if let Some((f, p)) = decode_dib(img, None, true) {
            frames.push(f);
            props.push(p);
        }
    }
    (!frames.is_empty()).then_some(Decoded { frames, props, loop_count: None })
}

/// Builds an ICO from PNG-encoded images of at most 256 x 256 pixels, each `(width, height, png bytes)`.
pub fn ico_encode(images: &[(usize, usize, Vec<u8>)]) -> Vec<u8> {
    let mut o = vec![0, 0, 1, 0];
    o.extend((images.len() as u16).to_le_bytes());
    let mut off = 6 + 16 * images.len();
    for (w, h, png) in images {
        o.extend([(*w % 256) as u8, (*h % 256) as u8, 0, 0]);
        o.extend(1u16.to_le_bytes());
        o.extend(32u16.to_le_bytes());
        o.extend((png.len() as u32).to_le_bytes());
        o.extend((off as u32).to_le_bytes());
        off += png.len();
    }
    for (_, _, png) in images {
        o.extend(png);
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<u8> {
        // 3x2: red, green, blue / white, black, translucent grey
        vec![255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255, 0, 0, 0, 255, 128, 128, 128, 100]
    }

    #[test]
    fn opaque_24_bit_round_trips() {
        let mut px = sample();
        px[23] = 255;
        let bmp = encode(3, 2, &px, Some((144.0, 144.0)));
        let p = probe(&bmp).unwrap();
        assert_eq!((p.pixel_width, p.pixel_height), (3, 2));
        assert!((p.dpi_width.unwrap() - 144.0).abs() < 0.5);
        let d = decode(&bmp).unwrap();
        assert_eq!(d.frames[0].rgba8, px);
    }

    #[test]
    fn alpha_32_bit_round_trips() {
        let px = sample();
        let d = decode(&encode(3, 2, &px, None)).unwrap();
        assert_eq!(d.frames[0].rgba8, px);
        assert!(d.props[0].has_alpha);
    }

    #[test]
    fn palette_8_bit_and_rle8() {
        // 4x2, 8-bit palette (red, green), RLE: row0 (bottom) = 4 x idx 0; row1 = 2 x idx 1, 2 x idx 0
        let mut b = vec![0u8; 14];
        b[0] = b'B';
        b[1] = b'M';
        let mut dib = Vec::new();
        for v in [40u32, 4, 2] {
            dib.extend(v.to_le_bytes());
        }
        dib.extend(1u16.to_le_bytes());
        dib.extend(8u16.to_le_bytes());
        dib.extend(1u32.to_le_bytes()); // RLE8
        dib.extend([0u8; 12]);
        dib.extend(2u32.to_le_bytes()); // colours used
        dib.extend([0u8; 4]);
        dib.extend([0, 0, 255, 0, 0, 255, 0, 0]); // red, green (BGRx)
        let rle = [4, 0, 0, 0, 2, 1, 2, 0, 0, 1];
        dib.extend(rle);
        let off = (14 + dib.len() - rle.len()) as u32;
        b[10..14].copy_from_slice(&off.to_le_bytes());
        b.extend(dib);
        let d = decode(&b).unwrap();
        let f = &d.frames[0];
        assert_eq!(&f.rgba8[..4], &[0, 255, 0, 255], "top row starts green");
        assert_eq!(&f.rgba8[8..12], &[255, 0, 0, 255]);
        assert_eq!(&f.rgba8[16..20], &[255, 0, 0, 255], "bottom row red");
    }

    #[test]
    fn ico_with_bitmap_and_png_frames() {
        // one 2x2 32-bit DIB frame, doubled height, AND mask all zero
        let mut dib = Vec::new();
        for v in [40u32, 2, 4] {
            dib.extend(v.to_le_bytes());
        }
        dib.extend(1u16.to_le_bytes());
        dib.extend(32u16.to_le_bytes());
        dib.extend([0u8; 24]);
        dib.extend([0, 0, 255, 255].repeat(4));
        dib.extend([0u8; 8]);
        let mut ico = vec![0, 0, 1, 0, 1, 0, 2, 2, 0, 0, 1, 0, 32, 0];
        ico.extend((dib.len() as u32).to_le_bytes());
        ico.extend(22u32.to_le_bytes());
        ico.extend(&dib);
        assert_eq!(ico_count(&ico), Some(1));
        let d = ico_decode(&ico).unwrap();
        assert_eq!(&d.frames[0].rgba8[..4], &[255, 0, 0, 255]);
        assert_eq!((d.frames[0].w, d.frames[0].h), (2, 2));
        assert!(ico_decode(b"\0\0\x01\0\0\0").is_none());
    }

    #[test]
    fn short_data_is_refused() {
        assert!(decode(b"BM").is_none());
        assert!(decode(&encode(3, 2, &sample(), None)[..40]).is_none());
    }

    #[test]
    fn ico_encoder_writes_a_directory() {
        let ico = ico_encode(&[(16, 16, vec![1, 2, 3])]);
        assert_eq!(ico_count(&ico), Some(1));
        assert_eq!(&ico[22..], &[1, 2, 3]);
    }
}
