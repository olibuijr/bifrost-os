//! JPEG reading: the segment headers give size, components, density, EXIF and XMP (APP1), ICC profile (APP2), image
//! resources with IPTC (APP13) and comments without decoding; baseline and progressive pixels are decoded by `zune-jpeg`.

use super::exif::{ColorModel, Dict, PropValue, Properties};
use super::{Decoded, Frame, MAX_SIDE};
use std::io::Cursor;

/// Reads the segment headers up to the first scan. `None` when no frame header is found.
pub fn probe(data: &[u8]) -> Option<Properties> {
    if data.get(..2)? != [0xFF, 0xD8] {
        return None;
    }
    let mut p = 2;
    let mut props: Option<Properties> = None;
    let mut jfif = Dict::new();
    let mut icc: Vec<(u8, Vec<u8>)> = Vec::new();
    let mut exif: Option<Vec<u8>> = None;
    let mut xmp: Option<Vec<u8>> = None;
    let mut resources: Option<Vec<u8>> = None;
    let mut dpi: Option<(f64, f64)> = None;
    let mut progressive = false;
    while p + 4 <= data.len() {
        if data[p] != 0xFF {
            p += 1;
            continue;
        }
        let m = data[p + 1];
        if m == 0xFF {
            p += 1;
            continue;
        }
        if m == 0xD8 || m == 0x01 || (0xD0..=0xD7).contains(&m) || m == 0 {
            p += 2;
            continue;
        }
        if m == 0xD9 || m == 0xDA {
            break;
        }
        let len = u16::from_be_bytes([data[p + 2], data[p + 3]]) as usize;
        let seg = data.get(p + 4..(p + 2 + len).min(data.len()))?;
        match m {
            0xC0..=0xCF if !matches!(m, 0xC4 | 0xC8 | 0xCC) => {
                if seg.len() >= 6 {
                    let h = u16::from_be_bytes([seg[1], seg[2]]) as usize;
                    let w = u16::from_be_bytes([seg[3], seg[4]]) as usize;
                    let n = seg[5];
                    progressive = matches!(m, 0xC2 | 0xC6 | 0xCA | 0xCE);
                    let mut pr = Properties::new(w, h);
                    pr.depth = seg[0] as u32;
                    pr.has_alpha = false;
                    pr.color_model = match n {
                        1 => ColorModel::Gray,
                        4 => ColorModel::Cmyk,
                        _ => ColorModel::Rgb,
                    };
                    props = Some(pr);
                }
            }
            0xE0 if seg.starts_with(b"JFIF\0") && seg.len() >= 12 => {
                jfif.insert("JFIFVersion".into(), PropValue::Ints(vec![seg[5] as i64, seg[6] as i64]));
                let (unit, xd, yd) = (seg[7], u16::from_be_bytes([seg[8], seg[9]]) as f64, u16::from_be_bytes([seg[10], seg[11]]) as f64);
                jfif.insert("DensityUnit".into(), PropValue::Int(unit as i64));
                jfif.insert("XDensity".into(), PropValue::Int(xd as i64));
                jfif.insert("YDensity".into(), PropValue::Int(yd as i64));
                match unit {
                    1 => dpi = Some((xd, yd)),
                    2 => dpi = Some((xd * 2.54, yd * 2.54)),
                    _ => {}
                }
            }
            0xE1 if seg.starts_with(b"Exif\0\0") => exif = Some(seg[6..].to_vec()),
            0xE1 if seg.starts_with(super::encode::XMP_APP1_HEADER) => xmp = Some(seg[super::encode::XMP_APP1_HEADER.len()..].to_vec()),
            0xED if seg.starts_with(b"Photoshop 3.0\0") => resources = Some(seg.to_vec()),
            0xE2 if seg.starts_with(b"ICC_PROFILE\0") && seg.len() > 14 => icc.push((seg[12], seg[14..].to_vec())),
            0xFE => {
                jfif.insert("Comment".into(), PropValue::Text(String::from_utf8_lossy(seg).into_owned()));
            }
            _ => {}
        }
        p += 2 + len;
    }
    let mut props = props?;
    props.jfif = jfif;
    props.jfif.insert("IsProgressive".into(), PropValue::Int(progressive as i64));
    if let Some((x, y)) = dpi {
        props.dpi_width = Some(x);
        props.dpi_height = Some(y);
    }
    if let Some(e) = exif {
        props.absorb_exif(&e);
    }
    props.xmp = xmp;
    props.image_resources = resources;
    if !icc.is_empty() {
        icc.sort_by_key(|c| c.0);
        props.icc_profile = Some(icc.into_iter().flat_map(|c| c.1).collect());
    }
    Some(props)
}

pub fn decode(data: &[u8]) -> Option<Decoded> {
    use zune_core::colorspace::ColorSpace;
    use zune_core::options::DecoderOptions;
    let props = probe(data)?;
    if props.pixel_width > MAX_SIDE || props.pixel_height > MAX_SIDE || props.pixel_width == 0 || props.pixel_height == 0 {
        return None;
    }
    let opts = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::RGB).set_max_width(MAX_SIDE).set_max_height(MAX_SIDE);
    let mut d = zune_jpeg::JpegDecoder::new_with_options(Cursor::new(data), opts);
    let px = d.decode().ok()?;
    let info = d.info()?;
    let (w, h) = (info.width as usize, info.height as usize);
    if px.len() < w * h * 3 {
        return None;
    }
    let mut rgba8 = Vec::with_capacity(w * h * 4);
    for c in px.as_chunks::<3>().0.iter().take(w * h) {
        rgba8.extend([c[0], c[1], c[2], 255]);
    }
    Some(Decoded { frames: vec![Frame { w, h, rgba8, rgba16: None, delay_ms: 0 }], props: vec![props], loop_count: None })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_probes_and_decodes() {
        let bytes = include_bytes!("../../../../src/deildu/fixtures/dot.jpg");
        let p = probe(bytes).unwrap();
        assert_eq!((p.pixel_width, p.pixel_height), (8, 8));
        let d = decode(bytes).unwrap();
        assert_eq!((d.frames[0].w, d.frames[0].h), (8, 8));
        assert!(d.frames[0].rgba8[(3 * 8 + 3) * 4] > 200, "red centre");
    }

    #[test]
    fn truncated_and_foreign_data_is_refused() {
        assert!(probe(b"GIF89a").is_none());
        assert!(probe(&[0xFF, 0xD8, 0xFF]).is_none());
    }

    #[test]
    fn app_segments_fill_density_exif_and_icc() {
        let mut j = vec![0xFF, 0xD8];
        // JFIF 72 dpi
        j.extend([0xFF, 0xE0, 0, 16, b'J', b'F', b'I', b'F', 0, 1, 1, 1, 0, 72, 0, 72, 0, 0]);
        // EXIF orientation 6 (little-endian block, one SHORT entry)
        let exif = [b'I', b'I', 42, 0, 8, 0, 0, 0, 1, 0, 0x12, 0x01, 3, 0, 1, 0, 0, 0, 6, 0, 0, 0, 0, 0, 0, 0];
        let mut seg = b"Exif\0\0".to_vec();
        seg.extend(exif);
        j.extend([0xFF, 0xE1]);
        j.extend(((seg.len() + 2) as u16).to_be_bytes());
        j.extend(seg);
        // ICC in two chunks, stored out of order
        for (seq, body) in [(2u8, b"def".as_slice()), (1u8, b"abc".as_slice())] {
            let mut s = b"ICC_PROFILE\0".to_vec();
            s.extend([seq, 2]);
            s.extend(body);
            j.extend([0xFF, 0xE2]);
            j.extend(((s.len() + 2) as u16).to_be_bytes());
            j.extend(s);
        }
        // SOF0 4x3 RGB progressive-off
        j.extend([0xFF, 0xC0, 0, 17, 8, 0, 3, 0, 4, 3, 1, 0x11, 0, 2, 0x11, 0, 3, 0x11, 0]);
        let p = probe(&j).unwrap();
        assert_eq!((p.pixel_width, p.pixel_height), (4, 3));
        assert_eq!(p.dpi_width, Some(72.0));
        assert_eq!(p.orientation, 6);
        assert_eq!(p.icc_profile.as_deref(), Some(b"abcdef".as_slice()));
    }
}
