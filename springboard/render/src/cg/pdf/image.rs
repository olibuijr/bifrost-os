//! PDF image XObjects and inline images: raw samples in any colour space
//! (1 to 16 bits), stencil masks, colour-key, stencil and soft masks, and
//! embedded JPEG data.

use super::color::ColorSpace;
use super::doc::Doc;
use super::obj::{Dict, Obj, Stream};
use super::sink::PdfImage;
use crate::gfx::{rgba, Image};

/// Images with more pixels than this are refused.
pub const MAX_PIXELS: usize = 64_000_000;

/// Reads sample `i` of a row at `bpc` bits.
fn sample(row: &[u8], i: usize, bpc: usize) -> u32 {
    match bpc {
        8 => row.get(i).copied().unwrap_or(0) as u32,
        16 => (row.get(i * 2).copied().unwrap_or(0) as u32) << 8 | row.get(i * 2 + 1).copied().unwrap_or(0) as u32,
        b => {
            let bit = i * b;
            let byte = row.get(bit / 8).copied().unwrap_or(0) as u32;
            (byte >> (8 - b - bit % 8)) & ((1 << b) - 1)
        }
    }
}

fn nearest(src: &[u8], sw: usize, sh: usize, dw: usize, dh: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(dw * dh);
    for y in 0..dh {
        let sy = (y * sh / dh).min(sh - 1);
        for x in 0..dw {
            out.push(src[sy * sw + (x * sw / dw).min(sw - 1)]);
        }
    }
    out
}

fn from_gfx(img: &Image) -> PdfImage {
    let mut rgba = Vec::with_capacity(img.w * img.h * 4);
    for &p in &img.px {
        let (r, g, b, a) = (p & 255, (p >> 8) & 255, (p >> 16) & 255, p >> 24);
        if a == 0 {
            rgba.extend_from_slice(&[0, 0, 0, 0]);
        } else if a == 255 {
            rgba.extend_from_slice(&[r as u8, g as u8, b as u8, 255]);
        } else {
            let un = |c: u32| (c * 255 / a).min(255) as u8;
            rgba.extend_from_slice(&[un(r), un(g), un(b), a as u8]);
        }
    }
    PdfImage { w: img.w, h: img.h, rgba, stencil: false }
}

/// A premultiplied software image from a decoded one.
pub fn to_gfx(img: &PdfImage) -> Image {
    Image { w: img.w, h: img.h, px: img.rgba.chunks_exact(4).map(|p| rgba(p[0], p[1], p[2], p[3])).collect() }
}

fn flag(doc: &Doc, d: &Dict, key: &str) -> bool {
    matches!(doc.field(d, key), Obj::Bool(true))
}

fn decode_array(doc: &Doc, d: &Dict) -> Option<Vec<f32>> {
    match doc.field(d, "Decode") {
        Obj::Array(a) => Some(a.iter().filter_map(|o| doc.get(o).num()).map(|v| v as f32).collect()),
        _ => None,
    }
}

/// The coverage plane of a stencil image: 255 where it paints.
fn stencil_plane(doc: &Doc, s: &Stream) -> Option<(usize, usize, Vec<u8>)> {
    let d = &s.dict;
    let (w, h) = (doc.field(d, "Width").int()?, doc.field(d, "Height").int()?);
    if w <= 0 || h <= 0 || (w as usize).checked_mul(h as usize)? > MAX_PIXELS {
        return None;
    }
    let (w, h) = (w as usize, h as usize);
    let (data, left) = doc.decode(s);
    if left.is_some() {
        return None;
    }
    let invert = decode_array(doc, d).is_some_and(|a| a.first() == Some(&1.0));
    let row_bytes = w.div_ceil(8);
    let mut out = Vec::with_capacity(w * h);
    for y in 0..h {
        let row = data.get(y * row_bytes..).unwrap_or(&[]);
        for x in 0..w {
            let bit = sample(row, x, 1) == 1;
            // sample 0 paints unless /Decode is [1 0]
            out.push(if bit == invert { 255 } else { 0 });
        }
    }
    Some((w, h, out))
}

/// Decodes an image XObject (or an inline image whose dictionary was expanded to full names).
pub fn decode(doc: &Doc, s: &Stream, res: Option<&Dict>) -> Option<PdfImage> {
    decode_depth(doc, s, res, 0)
}

fn decode_depth(doc: &Doc, s: &Stream, res: Option<&Dict>, depth: u8) -> Option<PdfImage> {
    let d = &s.dict;
    if flag(doc, d, "ImageMask") {
        let (w, h, cov) = stencil_plane(doc, s)?;
        let mut img = PdfImage::new(w, h);
        for (i, c) in cov.into_iter().enumerate() {
            img.rgba[i * 4 + 3] = c;
        }
        img.stencil = true;
        return Some(img);
    }
    let (w, h) = (doc.field(d, "Width").int()?, doc.field(d, "Height").int()?);
    if w <= 0 || h <= 0 || (w as usize).checked_mul(h as usize)? > MAX_PIXELS {
        return None;
    }
    let (w, h) = (w as usize, h as usize);
    let (data, left) = doc.decode(s);
    let mut img = match left.as_deref() {
        Some("DCTDecode") => from_gfx(&crate::imgdec::decode(&data)?),
        Some(_) => return None,
        None => raw_image(doc, d, &data, w, h, res)?,
    };
    if depth == 0 {
        apply_masks(doc, d, &mut img, res);
    }
    Some(img)
}

fn raw_image(doc: &Doc, d: &Dict, data: &[u8], w: usize, h: usize, res: Option<&Dict>) -> Option<PdfImage> {
    let bpc = doc.field(d, "BitsPerComponent").int().unwrap_or(8) as usize;
    if ![1, 2, 4, 8, 16].contains(&bpc) {
        return None;
    }
    let cs = ColorSpace::parse(doc, d.get("ColorSpace")?, res)?;
    let comps = cs.n();
    if comps == 0 {
        return None;
    }
    let decode = decode_array(doc, d).filter(|a| a.len() >= 2 * comps);
    let maxv = ((1u64 << bpc) - 1) as f32;
    let dec = |k: usize, raw: u32| -> f32 {
        let (lo, hi) = match &decode {
            Some(a) => (a[2 * k], a[2 * k + 1]),
            None => cs.default_decode(k, bpc),
        };
        lo + raw as f32 / maxv * (hi - lo)
    };
    let key: Option<Vec<u32>> = match doc.field(d, "Mask") {
        Obj::Array(a) => {
            let v: Vec<u32> = a.iter().filter_map(|o| doc.get(o).int()).map(|v| v.max(0) as u32).collect();
            (v.len() >= 2 * comps).then_some(v)
        }
        _ => None,
    };
    let row_bytes = (w * comps * bpc).div_ceil(8);
    let mut img = PdfImage::new(w, h);
    // one component with at most 8 bits: a lookup table covers every value
    let lut: Option<Vec<[u8; 3]>> = (comps == 1 && bpc <= 8).then(|| {
        (0..(1u32 << bpc)).map(|v| {
            let c = cs.to_rgb(&[dec(0, v)]);
            [(c[0] * 255.0 + 0.5) as u8, (c[1] * 255.0 + 0.5) as u8, (c[2] * 255.0 + 0.5) as u8]
        }).collect()
    });
    let plain_rgb = matches!(cs, ColorSpace::Rgb | ColorSpace::Icc { n: 3 }) && bpc == 8 && decode.is_none();
    let paints_nothing = cs.paints_nothing();
    let mut raw = vec![0u32; comps];
    let mut vals = vec![0f32; comps];
    for y in 0..h {
        let row = data.get(y * row_bytes..).unwrap_or(&[]);
        let row = &row[..row.len().min(row_bytes)];
        for x in 0..w {
            for (k, slot) in raw.iter_mut().enumerate() {
                *slot = sample(row, x * comps + k, bpc);
            }
            let o = (y * w + x) * 4;
            let rgb: [u8; 3] = if let Some(lut) = &lut {
                let idx = raw[0] as usize;
                lut.get(idx).copied().unwrap_or([0, 0, 0])
            } else if plain_rgb {
                [raw[0] as u8, raw[1] as u8, raw[2] as u8]
            } else {
                for k in 0..comps {
                    vals[k] = dec(k, raw[k]);
                }
                let c = cs.to_rgb(&vals);
                [(c[0] * 255.0 + 0.5) as u8, (c[1] * 255.0 + 0.5) as u8, (c[2] * 255.0 + 0.5) as u8]
            };
            let masked = paints_nothing || key.as_ref().is_some_and(|k| (0..comps).all(|i| raw[i] >= k[2 * i] && raw[i] <= k[2 * i + 1]));
            img.rgba[o..o + 4].copy_from_slice(&[rgb[0], rgb[1], rgb[2], if masked { 0 } else { 255 }]);
        }
    }
    Some(img)
}

/// Applies /SMask or a stencil /Mask stream to the alpha channel.
fn apply_masks(doc: &Doc, d: &Dict, img: &mut PdfImage, res: Option<&Dict>) {
    let plane: Option<(usize, usize, Vec<u8>)> = if let Obj::Stream(sm) = doc.field(d, "SMask") {
        decode_depth(doc, &sm, res, 1).map(|m| (m.w, m.h, m.rgba.chunks_exact(4).map(|p| p[0]).collect()))
    } else if let Obj::Stream(m) = doc.field(d, "Mask") {
        stencil_plane(doc, &m)
    } else {
        None
    };
    let Some((mw, mh, mut alpha)) = plane else { return };
    if mw == 0 || mh == 0 {
        return;
    }
    // bring both to the larger of the two sizes
    if mw * mh > img.w * img.h {
        let (nw, nh) = (mw, mh);
        let mut big = PdfImage::new(nw, nh);
        for y in 0..nh {
            let sy = (y * img.h / nh).min(img.h - 1);
            for x in 0..nw {
                let sx = (x * img.w / nw).min(img.w - 1);
                let s = (sy * img.w + sx) * 4;
                let o = (y * nw + x) * 4;
                big.rgba[o..o + 4].copy_from_slice(&img.rgba[s..s + 4]);
            }
        }
        *img = big;
    } else if mw != img.w || mh != img.h {
        alpha = nearest(&alpha, mw, mh, img.w, img.h);
    }
    for (p, a) in img.rgba.chunks_exact_mut(4).zip(alpha) {
        p[3] = ((p[3] as u32 * a as u32 + 127) / 255) as u8;
    }
}

/// Smallest side shown by the text extractor: icons and rules are chrome, not content.
const MIN_SIDE: usize = 24;

/// A premultiplied image for reflowed reading; None for tiny images and ones this reader cannot decode.
pub fn load(doc: &Doc, s: &Stream) -> Option<Image> {
    let (w, h) = (doc.field(&s.dict, "Width").int()?, doc.field(&s.dict, "Height").int()?);
    if w < MIN_SIDE as i64 || h < MIN_SIDE as i64 || w * h > 24_000_000 {
        return None;
    }
    let img = decode(doc, s, None)?;
    if img.stencil {
        return None;
    }
    Some(to_gfx(&img))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn doc() -> Doc {
        Doc::open(super::super::doc::tests::tiny_pdf("")).unwrap()
    }

    fn stream(entries: Vec<(&str, Obj)>, data: Vec<u8>) -> Stream {
        let dict: HashMap<String, Obj> = entries.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
        Stream { dict, data }
    }

    fn px(img: &PdfImage, i: usize) -> [u8; 4] {
        [img.rgba[i * 4], img.rgba[i * 4 + 1], img.rgba[i * 4 + 2], img.rgba[i * 4 + 3]]
    }

    #[test]
    fn raw_rgb_and_gray() {
        let rgb: Vec<u8> = (0..24 * 24).flat_map(|_| [10u8, 20, 30]).collect();
        let s = stream(vec![("Width", Obj::Int(24)), ("Height", Obj::Int(24)), ("ColorSpace", Obj::Name("DeviceRGB".into()))], rgb);
        let img = decode(&doc(), &s, None).unwrap();
        assert_eq!((img.w, img.h, px(&img, 0)), (24, 24, [10, 20, 30, 255]));
        let g = stream(vec![("Width", Obj::Int(24)), ("Height", Obj::Int(24)), ("ColorSpace", Obj::Name("DeviceGray".into())), ("BitsPerComponent", Obj::Int(1))], vec![0b1000_0000; 3 * 24]);
        let img = decode(&doc(), &g, None).unwrap();
        assert_eq!(px(&img, 0), [255, 255, 255, 255]);
        assert_eq!(px(&img, 1), [0, 0, 0, 255]);
    }

    #[test]
    fn indexed_palette_and_small_images() {
        let pal = Obj::Array(vec![Obj::Name("Indexed".into()), Obj::Name("DeviceRGB".into()), Obj::Int(1), Obj::Str(vec![255, 0, 0, 0, 0, 255])]);
        let s = stream(vec![("Width", Obj::Int(24)), ("Height", Obj::Int(24)), ("ColorSpace", pal), ("BitsPerComponent", Obj::Int(8))], vec![1; 24 * 24]);
        assert_eq!(px(&decode(&doc(), &s, None).unwrap(), 5), [0, 0, 255, 255]);
        let tiny = stream(vec![("Width", Obj::Int(4)), ("Height", Obj::Int(4)), ("ColorSpace", Obj::Name("DeviceGray".into()))], vec![0; 16]);
        assert!(load(&doc(), &tiny).is_none(), "icons and rules are skipped for reading");
        assert!(decode(&doc(), &tiny, None).is_some(), "but they decode for drawing");
    }

    #[test]
    fn stencil_masks_paint_where_the_sample_is_zero() {
        let s = stream(vec![("Width", Obj::Int(8)), ("Height", Obj::Int(1)), ("ImageMask", Obj::Bool(true))], vec![0b0101_0101]);
        let img = decode(&doc(), &s, None).unwrap();
        assert!(img.stencil);
        let alphas: Vec<u8> = (0..8).map(|i| px(&img, i)[3]).collect();
        assert_eq!(alphas, vec![255, 0, 255, 0, 255, 0, 255, 0]);
        let inv = stream(vec![("Width", Obj::Int(8)), ("Height", Obj::Int(1)), ("ImageMask", Obj::Bool(true)), ("Decode", Obj::Array(vec![Obj::Int(1), Obj::Int(0)]))], vec![0b0101_0101]);
        assert_eq!(px(&decode(&doc(), &inv, None).unwrap(), 0)[3], 0);
    }

    #[test]
    fn colour_key_mask_makes_pixels_transparent() {
        let key = Obj::Array(vec![Obj::Int(255), Obj::Int(255)]);
        let s = stream(vec![("Width", Obj::Int(2)), ("Height", Obj::Int(1)), ("ColorSpace", Obj::Name("DeviceGray".into())), ("Mask", key)], vec![255, 7]);
        let img = decode(&doc(), &s, None).unwrap();
        assert_eq!((px(&img, 0)[3], px(&img, 1)[3]), (0, 255));
    }

    #[test]
    fn sixteen_bit_and_decode_inversion() {
        let s = stream(
            vec![("Width", Obj::Int(1)), ("Height", Obj::Int(1)), ("ColorSpace", Obj::Name("DeviceGray".into())), ("BitsPerComponent", Obj::Int(16)), ("Decode", Obj::Array(vec![Obj::Int(1), Obj::Int(0)]))],
            vec![0xff, 0xff],
        );
        assert_eq!(px(&decode(&doc(), &s, None).unwrap(), 0), [0, 0, 0, 255]);
    }
}
