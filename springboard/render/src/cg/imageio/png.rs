//! PNG reading: header and metadata chunks are scanned directly (so a size or a text chunk costs no pixel decode);
//! pixels go through the `png` crate, which handles every colour type, 16-bit samples and Adam7 interlacing.

use super::exif::{ColorModel, Dict, PropValue, Properties};
use super::{Decoded, Frame, MAX_SIDE};
use std::io::Cursor;

pub const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

/// `(type, data)` for each chunk up to and including IEND; stops quietly at truncation.
fn chunks(data: &[u8]) -> Vec<([u8; 4], &[u8])> {
    let mut out = Vec::new();
    let mut p = 8;
    while p + 8 <= data.len() {
        let len = u32::from_be_bytes([data[p], data[p + 1], data[p + 2], data[p + 3]]) as usize;
        let ty = [data[p + 4], data[p + 5], data[p + 6], data[p + 7]];
        let end = (p + 8).saturating_add(len).min(data.len());
        out.push((ty, &data[p + 8..end]));
        if &ty == b"IEND" {
            break;
        }
        p = end.saturating_add(4);
    }
    out
}

fn latin1(b: &[u8]) -> String {
    b.iter().map(|&c| c as char).collect()
}

fn inflate(b: &[u8]) -> Option<Vec<u8>> {
    miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(b, 64 << 20).ok()
}

/// Reads the header and metadata of the first image. `None` when it is not a PNG with a valid IHDR.
pub fn probe(data: &[u8]) -> Option<Properties> {
    if data.get(..8)? != SIGNATURE {
        return None;
    }
    let cs = chunks(data);
    let (ty, ihdr) = cs.first()?;
    if ty != b"IHDR" || ihdr.len() < 13 {
        return None;
    }
    let w = u32::from_be_bytes(ihdr[0..4].try_into().ok()?) as usize;
    let h = u32::from_be_bytes(ihdr[4..8].try_into().ok()?) as usize;
    if w == 0 || h == 0 {
        return None;
    }
    let (depth, ctype, interlace) = (ihdr[8], ihdr[9], ihdr[12]);
    let mut p = Properties::new(w, h);
    p.depth = depth as u32;
    p.color_model = match ctype {
        0 | 4 => ColorModel::Gray,
        3 => ColorModel::Indexed,
        _ => ColorModel::Rgb,
    };
    p.is_indexed = ctype == 3;
    p.has_alpha = matches!(ctype, 4 | 6);
    let mut png = Dict::new();
    png.insert("InterlaceType".into(), PropValue::Int(interlace as i64));
    for (ty, d) in &cs[1..] {
        match ty {
            b"tRNS" => p.has_alpha = true,
            b"pHYs" if d.len() >= 9 => {
                let x = u32::from_be_bytes([d[0], d[1], d[2], d[3]]) as f64;
                let y = u32::from_be_bytes([d[4], d[5], d[6], d[7]]) as f64;
                png.insert("XPixelsPerMeter".into(), PropValue::Int(x as i64));
                png.insert("YPixelsPerMeter".into(), PropValue::Int(y as i64));
                if d[8] == 1 {
                    p.dpi_width = Some(x * 0.0254);
                    p.dpi_height = Some(y * 0.0254);
                }
            }
            b"gAMA" if d.len() >= 4 => {
                let g = u32::from_be_bytes([d[0], d[1], d[2], d[3]]) as f64 / 100_000.0;
                png.insert("Gamma".into(), PropValue::Float(g));
            }
            b"sRGB" => {
                png.insert("sRGBIntent".into(), PropValue::Int(d.first().copied().unwrap_or(0) as i64));
            }
            b"iCCP" => {
                if let Some(z) = d.iter().position(|&c| c == 0) {
                    p.profile_name = Some(latin1(&d[..z]));
                    if d.len() > z + 2 {
                        p.icc_profile = inflate(&d[z + 2..]);
                    }
                }
            }
            b"acTL" if d.len() >= 8 => {
                png.insert("FrameCount".into(), PropValue::Int(u32::from_be_bytes([d[0], d[1], d[2], d[3]]) as i64));
                png.insert("LoopCount".into(), PropValue::Int(u32::from_be_bytes([d[4], d[5], d[6], d[7]]) as i64));
            }
            b"tEXt" => {
                if let Some(z) = d.iter().position(|&c| c == 0) {
                    png.insert(latin1(&d[..z]), PropValue::Text(latin1(&d[z + 1..])));
                }
            }
            b"zTXt" => {
                if let Some(z) = d.iter().position(|&c| c == 0) {
                    if let Some(t) = d.get(z + 2..).and_then(inflate) {
                        png.insert(latin1(&d[..z]), PropValue::Text(String::from_utf8_lossy(&t).into_owned()));
                    }
                }
            }
            b"iTXt" => {
                if let Some(z) = d.iter().position(|&c| c == 0) {
                    let key = latin1(&d[..z]);
                    let rest = d.get(z + 1..).unwrap_or(&[]);
                    if rest.len() >= 2 {
                        let (flag, rest) = (rest[0], &rest[2..]);
                        // language tag and translated keyword, each NUL-terminated
                        let mut parts = rest.splitn(3, |&c| c == 0);
                        let (_, _, text) = (parts.next(), parts.next(), parts.next().unwrap_or(&[]));
                        let text = if flag == 1 { inflate(text).unwrap_or_default() } else { text.to_vec() };
                        png.insert(key, PropValue::Text(String::from_utf8_lossy(&text).into_owned()));
                    }
                }
            }
            b"eXIf" => p.absorb_exif(d),
            _ => {}
        }
    }
    p.png = png;
    Some(p)
}

/// Decodes the first image into straight RGBA (and 16-bit samples when the file has them).
pub fn decode(data: &[u8]) -> Option<Decoded> {
    use ::png::{BitDepth, ColorType, Transformations};
    let props = probe(data)?;
    if props.pixel_width > MAX_SIDE || props.pixel_height > MAX_SIDE {
        return None;
    }
    let mut d = ::png::Decoder::new(Cursor::new(data));
    d.set_transformations(Transformations::EXPAND);
    let mut r = d.read_info().ok()?;
    let mut buf = vec![0u8; r.output_buffer_size()?];
    let out = r.next_frame(&mut buf).ok()?;
    let (w, h) = (out.width as usize, out.height as usize);
    let n = match out.color_type {
        ColorType::Grayscale => 1,
        ColorType::GrayscaleAlpha => 2,
        ColorType::Rgb => 3,
        ColorType::Rgba => 4,
        ColorType::Indexed => return None,
    };
    let wide = out.bit_depth == BitDepth::Sixteen;
    let bps = if wide { 2 } else { 1 };
    let stride = out.line_size;
    if stride < w * n * bps || buf.len() < stride * h {
        return None;
    }
    // sample(x, y, c) as u16 (8-bit widened by 257)
    let get = |x: usize, y: usize, c: usize| -> u16 {
        let i = y * stride + (x * n + c) * bps;
        if wide {
            u16::from_be_bytes([buf[i], buf[i + 1]])
        } else {
            buf[i] as u16 * 257
        }
    };
    let mut rgba8 = Vec::with_capacity(w * h * 4);
    let mut rgba16 = if wide { Some(Vec::with_capacity(w * h * 4)) } else { None };
    for y in 0..h {
        for x in 0..w {
            let px: [u16; 4] = match n {
                1 => [get(x, y, 0), get(x, y, 0), get(x, y, 0), 65535],
                2 => [get(x, y, 0), get(x, y, 0), get(x, y, 0), get(x, y, 1)],
                3 => [get(x, y, 0), get(x, y, 1), get(x, y, 2), 65535],
                _ => [get(x, y, 0), get(x, y, 1), get(x, y, 2), get(x, y, 3)],
            };
            rgba8.extend(px.iter().map(|&v| (v >> 8) as u8));
            if let Some(v) = rgba16.as_mut() {
                v.extend(px);
            }
        }
    }
    Some(Decoded { frames: vec![Frame { w, h, rgba8, rgba16, delay_ms: 0 }], props: vec![props], loop_count: None })
}
