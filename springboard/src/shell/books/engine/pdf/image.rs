//! PDF image XObjects: embedded JPEG (DCTDecode) and raw Flate samples in
//! gray, RGB, CMYK, indexed and ICC-based colour spaces.

use super::doc::Doc;
use super::obj::{Dict, Obj, Stream};
use crate::gfx::{pack, Image};

const MIN_SIDE: i64 = 24;
const MAX_PIXELS: i64 = 24_000_000;

enum Space {
    Gray,
    Rgb,
    Cmyk,
    Indexed { base: Box<Space>, palette: Vec<u8> },
}

impl Space {
    fn comps(&self) -> usize {
        match self {
            Space::Gray | Space::Indexed { .. } => 1,
            Space::Rgb => 3,
            Space::Cmyk => 4,
        }
    }
}

fn space(doc: &Doc, o: &Obj, depth: u8) -> Option<Space> {
    match doc.get(o) {
        Obj::Name(n) => by_name(&n),
        Obj::Array(a) => {
            let head = doc.get(a.first()?);
            match head.name()? {
                "ICCBased" => {
                    let s = doc.get(a.get(1)?);
                    match s.dict().map(|d| doc.field(d, "N").int().unwrap_or(3)) {
                        Some(1) => Some(Space::Gray),
                        Some(4) => Some(Space::Cmyk),
                        _ => Some(Space::Rgb),
                    }
                }
                "Indexed" | "I" if depth < 2 => {
                    let base = space(doc, a.get(1)?, depth + 1)?;
                    let palette = match doc.get(a.get(3)?) {
                        Obj::Str(s) => s,
                        Obj::Stream(s) => doc.decode(&s).0,
                        _ => return None,
                    };
                    Some(Space::Indexed { base: Box::new(base), palette })
                }
                "CalRGB" | "Lab" => Some(Space::Rgb),
                "CalGray" => Some(Space::Gray),
                other => by_name(other),
            }
        }
        _ => None,
    }
}

fn by_name(n: &str) -> Option<Space> {
    match n {
        "DeviceGray" | "G" | "CalGray" => Some(Space::Gray),
        "DeviceRGB" | "RGB" | "CalRGB" => Some(Space::Rgb),
        "DeviceCMYK" | "CMYK" => Some(Space::Cmyk),
        _ => None,
    }
}

fn cmyk(c: u8, m: u8, y: u8, k: u8) -> u32 {
    let f = |v: u8| 255 - ((v as u32 + k as u32).min(255)) as u8;
    pack(f(c), f(m), f(y), 255)
}

fn color(sp: &Space, s: &[u8]) -> u32 {
    match sp {
        Space::Gray => pack(s[0], s[0], s[0], 255),
        Space::Rgb => pack(s[0], s[1], s[2], 255),
        Space::Cmyk => cmyk(s[0], s[1], s[2], s[3]),
        Space::Indexed { base, palette } => {
            let n = base.comps();
            let i = s[0] as usize * n;
            match palette.get(i..i + n) {
                Some(c) => color(base, c),
                None => pack(0, 0, 0, 255),
            }
        }
    }
}

/// Reads sample `i` of `bpc` bits from a row; 16-bit keeps the high byte.
fn sample(row: &[u8], i: usize, bpc: usize, indexed: bool) -> u8 {
    match bpc {
        8 => row.get(i).copied().unwrap_or(0),
        16 => row.get(i * 2).copied().unwrap_or(0),
        b => {
            let bit = i * b;
            let byte = row.get(bit / 8).copied().unwrap_or(0);
            let v = (byte >> (8 - b - bit % 8)) & ((1 << b) - 1);
            if indexed {
                v
            } else {
                // scale to 0..=255
                (v as u32 * 255 / ((1 << b) - 1)) as u8
            }
        }
    }
}

/// Decodes an image XObject; None when it is too small, too big or in a
/// format this reader cannot show (JPEG 2000, CCITT, JBIG2).
pub fn load(doc: &Doc, s: &Stream) -> Option<Image> {
    let d: &Dict = &s.dict;
    let (w, h) = (doc.field(d, "Width").int()?, doc.field(d, "Height").int()?);
    if w < MIN_SIDE || h < MIN_SIDE || w * h > MAX_PIXELS {
        return None;
    }
    let (data, left) = doc.decode(s);
    if let Some(f) = left {
        if f == "DCTDecode" {
            return crate::imgdec::decode(&data).map(super::super::epub::shrink);
        }
        return None;
    }
    let mask = matches!(doc.field(d, "ImageMask"), Obj::Bool(true));
    let bpc = if mask { 1 } else { doc.field(d, "BitsPerComponent").int().unwrap_or(8) as usize };
    if ![1, 2, 4, 8, 16].contains(&bpc) {
        return None;
    }
    let sp = if mask { Space::Gray } else { space(doc, d.get("ColorSpace")?, 0)? };
    let invert = doc.field(d, "Decode").array().is_some_and(|a| a.len() >= 2 && a[0].num() == Some(1.0) && a[1].num() == Some(0.0));
    let (w, h) = (w as usize, h as usize);
    let comps = sp.comps();
    let row_bytes = (w * comps * bpc).div_ceil(8);
    let indexed = matches!(sp, Space::Indexed { .. });
    let mut px = Vec::with_capacity(w * h);
    for y in 0..h {
        let row = data.get(y * row_bytes..).unwrap_or(&[]);
        let row = &row[..row.len().min(row_bytes)];
        for x in 0..w {
            let mut c = [0u8; 4];
            for (k, slot) in c.iter_mut().enumerate().take(comps) {
                *slot = sample(row, x * comps + k, bpc, indexed);
                if invert && !indexed {
                    *slot = 255 - *slot;
                }
            }
            px.push(color(&sp, &c));
        }
    }
    Some(super::super::epub::shrink(Image { w, h, px }))
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

    #[test]
    fn raw_rgb_and_gray() {
        let rgb: Vec<u8> = (0..24 * 24).flat_map(|_| [10u8, 20, 30]).collect();
        let s = stream(vec![("Width", Obj::Int(24)), ("Height", Obj::Int(24)), ("ColorSpace", Obj::Name("DeviceRGB".into()))], rgb);
        let img = load(&doc(), &s).unwrap();
        assert_eq!((img.w, img.h, img.px[0]), (24, 24, pack(10, 20, 30, 255)));
        let g = stream(vec![("Width", Obj::Int(24)), ("Height", Obj::Int(24)), ("ColorSpace", Obj::Name("DeviceGray".into())), ("BitsPerComponent", Obj::Int(1))], vec![0b1000_0000; 3 * 24]);
        let img = load(&doc(), &g).unwrap();
        assert_eq!(img.px[0], pack(255, 255, 255, 255));
        assert_eq!(img.px[1], pack(0, 0, 0, 255));
    }

    #[test]
    fn indexed_palette_and_small_images() {
        let pal = Obj::Array(vec![Obj::Name("Indexed".into()), Obj::Name("DeviceRGB".into()), Obj::Int(1), Obj::Str(vec![255, 0, 0, 0, 0, 255])]);
        let s = stream(vec![("Width", Obj::Int(24)), ("Height", Obj::Int(24)), ("ColorSpace", pal), ("BitsPerComponent", Obj::Int(8))], vec![1; 24 * 24]);
        assert_eq!(load(&doc(), &s).unwrap().px[5], pack(0, 0, 255, 255));
        let tiny = stream(vec![("Width", Obj::Int(4)), ("Height", Obj::Int(4)), ("ColorSpace", Obj::Name("DeviceGray".into()))], vec![0; 16]);
        assert!(load(&doc(), &tiny).is_none(), "icons and rules are skipped");
    }
}
