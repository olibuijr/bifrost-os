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
                        if key == "XML:com.adobe.xmp" {
                            p.xmp = Some(text.clone());
                        }
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

/// An animated PNG is decoded only when its composited frames hold at most this many pixels in all; a larger one
/// shows its first image.
pub const MAX_ANIMATION_PIXELS: usize = 96 << 20;

/// What happens to a frame's rectangle after it was shown (APNG `dispose_op`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dispose {
    None,
    Background,
    Previous,
}

/// How a frame's pixels meet the canvas (APNG `blend_op`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Blend {
    Source,
    Over,
}

/// One animation frame as stored in the file: a rectangle of straight RGBA pixels placed on the canvas.
#[derive(Clone, Debug, PartialEq)]
pub struct SubFrame {
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
    pub delay_ms: u32,
    pub dispose: Dispose,
    pub blend: Blend,
    pub rgba8: Vec<u8>,
}

/// Source-over of one straight-alpha pixel onto another.
fn blend_over(dst: &mut [u8], src: &[u8]) {
    let (sa, da) = (src[3] as u32, dst[3] as u32);
    if sa == 255 || da == 0 {
        dst.copy_from_slice(&src[..4]);
        return;
    }
    if sa == 0 {
        return;
    }
    // out_a in 0..=255 scaled by 255 for the divisions below
    let out_a = sa * 255 + da * (255 - sa);
    for c in 0..3 {
        let num = src[c] as u32 * sa * 255 + dst[c] as u32 * da * (255 - sa);
        dst[c] = ((num + out_a / 2) / out_a).min(255) as u8;
    }
    dst[3] = ((out_a + 127) / 255).min(255) as u8;
}

/// Plays the frames of an animation onto a `cw` x `ch` canvas (starting transparent) in the order of the APNG
/// specification: blend the rectangle, show the canvas, then dispose the rectangle. Returns the full canvas as it
/// is shown for each frame. Rectangles that stick out of the canvas are clipped.
pub fn composite(cw: usize, ch: usize, frames: &[SubFrame]) -> Vec<Vec<u8>> {
    let mut canvas = vec![0u8; cw * ch * 4];
    let mut shown = Vec::with_capacity(frames.len());
    for (i, f) in frames.iter().enumerate() {
        // The first frame cannot go back to a state before it, so Previous acts as Background there.
        let dispose = if i == 0 && f.dispose == Dispose::Previous { Dispose::Background } else { f.dispose };
        let before = (dispose == Dispose::Previous).then(|| canvas.clone());
        let (x1, y1) = ((f.x + f.w).min(cw), (f.y + f.h).min(ch));
        if f.rgba8.len() >= f.w * f.h * 4 {
            for y in f.y..y1 {
                for x in f.x..x1 {
                    let s = ((y - f.y) * f.w + (x - f.x)) * 4;
                    let d = (y * cw + x) * 4;
                    match f.blend {
                        Blend::Source => canvas[d..d + 4].copy_from_slice(&f.rgba8[s..s + 4]),
                        Blend::Over => blend_over(&mut canvas[d..d + 4], &f.rgba8[s..s + 4]),
                    }
                }
            }
        }
        shown.push(canvas.clone());
        match dispose {
            Dispose::None => {}
            Dispose::Background => {
                for y in f.y..y1 {
                    for x in f.x..x1 {
                        let d = (y * cw + x) * 4;
                        canvas[d..d + 4].fill(0);
                    }
                }
            }
            Dispose::Previous => {
                if let Some(b) = before {
                    canvas = b;
                }
            }
        }
    }
    shown
}

/// Straight 8-bit RGBA from a decoded buffer of `n` samples per pixel (1 gray, 2 gray + alpha, 3 RGB, 4 RGBA).
fn rgba8_from(buf: &[u8], w: usize, h: usize, stride: usize, n: usize) -> Option<Vec<u8>> {
    if stride < w * n || buf.len() < stride * h {
        return None;
    }
    let mut out = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        let row = &buf[y * stride..y * stride + w * n];
        for p in row.chunks_exact(n) {
            out.extend(match n {
                1 => [p[0], p[0], p[0], 255],
                2 => [p[0], p[0], p[0], p[1]],
                3 => [p[0], p[1], p[2], 255],
                _ => [p[0], p[1], p[2], p[3]],
            });
        }
    }
    Some(out)
}

/// The number of animation frames in an APNG (`acTL`), 1 for an ordinary PNG.
pub fn frame_count(data: &[u8]) -> usize {
    if data.get(..8) != Some(&SIGNATURE[..]) {
        return 1;
    }
    for (ty, d) in chunks(data) {
        match &ty {
            b"acTL" if d.len() >= 8 => return (u32::from_be_bytes([d[0], d[1], d[2], d[3]]) as usize).max(1),
            // The animation control chunk comes before the first image data.
            b"IDAT" => break,
            _ => {}
        }
    }
    1
}

/// Reads the frames of an APNG as stored in the file (the default image of a file that does not animate it is
/// skipped). `None` for an ordinary PNG, a single-frame animation or damaged data.
pub fn read_animation(data: &[u8]) -> Option<(Vec<SubFrame>, u32)> {
    use ::png::{ColorType, Transformations};
    if frame_count(data) < 2 {
        return None;
    }
    let mut d = ::png::Decoder::new(Cursor::new(data));
    d.set_transformations(Transformations::EXPAND | Transformations::STRIP_16);
    let mut r = d.read_info().ok()?;
    let plays = r.info().animation_control()?.num_plays;
    let canvas_len = r.output_buffer_size()?;
    let mut frames = Vec::new();
    loop {
        let mut buf = vec![0u8; canvas_len];
        // The decoder reports an error once every frame was read.
        let Ok(out) = r.next_frame(&mut buf) else { break };
        let n = match out.color_type {
            ColorType::Grayscale => 1,
            ColorType::GrayscaleAlpha => 2,
            ColorType::Rgb => 3,
            ColorType::Rgba => 4,
            ColorType::Indexed => return None,
        };
        let (w, h) = (out.width as usize, out.height as usize);
        let rgba8 = rgba8_from(&buf, w, h, out.line_size, n)?;
        // A frame without control data is the default image of a file that animates other frames.
        if let Some(fc) = r.info().frame_control() {
            let den = if fc.delay_den == 0 { 100 } else { fc.delay_den as u32 };
            frames.push(SubFrame {
                x: fc.x_offset as usize,
                y: fc.y_offset as usize,
                w,
                h,
                delay_ms: fc.delay_num as u32 * 1000 / den,
                dispose: match fc.dispose_op {
                    ::png::DisposeOp::None => Dispose::None,
                    ::png::DisposeOp::Background => Dispose::Background,
                    ::png::DisposeOp::Previous => Dispose::Previous,
                },
                blend: match fc.blend_op {
                    ::png::BlendOp::Source => Blend::Source,
                    ::png::BlendOp::Over => Blend::Over,
                },
                rgba8,
            });
        }
        if frames.len() > 4096 {
            return None;
        }
    }
    (frames.len() >= 2).then_some((frames, plays))
}

/// Decodes an animated PNG into one composited full-canvas frame per animation frame.
fn decode_animated(data: &[u8], props: &Properties) -> Option<Decoded> {
    let (cw, ch) = (props.pixel_width, props.pixel_height);
    // every composited frame is a whole canvas: refuse an animation that would hold more than this many pixels
    if cw.saturating_mul(ch).saturating_mul(frame_count(data)) > MAX_ANIMATION_PIXELS {
        return None;
    }
    let (subs, plays) = read_animation(data)?;
    let shown = composite(cw, ch, &subs);
    let frames: Vec<Frame> = shown.into_iter().zip(&subs).map(|(rgba8, s)| Frame { w: cw, h: ch, rgba8, rgba16: None, delay_ms: s.delay_ms }).collect();
    let props = vec![props.clone(); frames.len()];
    Some(Decoded { frames, props, loop_count: Some(plays) })
}

/// Decodes the first image into straight RGBA (and 16-bit samples when the file has them). An animated PNG gives
/// one composited frame per animation frame with its delay and loop count.
pub fn decode(data: &[u8]) -> Option<Decoded> {
    use ::png::{BitDepth, ColorType, Transformations};
    let props = probe(data)?;
    if props.pixel_width > MAX_SIDE || props.pixel_height > MAX_SIDE {
        return None;
    }
    if let Some(a) = decode_animated(data, &props) {
        return Some(a);
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

#[cfg(test)]
mod tests {
    use super::super::encode::{encode_apng, encode_png, ApngFrame, PngOptions};
    use super::*;

    fn solid(w: usize, h: usize, c: [u8; 4]) -> Vec<u8> {
        (0..w * h).flat_map(|_| c).collect()
    }

    fn sub(x: usize, y: usize, w: usize, h: usize, dispose: Dispose, blend: Blend, c: [u8; 4]) -> SubFrame {
        SubFrame { x, y, w, h, delay_ms: 10, dispose, blend, rgba8: solid(w, h, c) }
    }

    #[test]
    fn an_animated_png_round_trips_frames_delays_and_loop_count() {
        let (a, b, c) = (solid(3, 2, [255, 0, 0, 255]), solid(3, 2, [0, 255, 0, 255]), solid(3, 2, [0, 0, 255, 128]));
        let frames = [ApngFrame { rgba: &a, delay_ms: 120 }, ApngFrame { rgba: &b, delay_ms: 250 }, ApngFrame { rgba: &c, delay_ms: 40 }];
        let bytes = encode_apng(3, 2, &frames, 4, &PngOptions::default()).unwrap();
        assert_eq!(frame_count(&bytes), 3);
        let d = decode(&bytes).unwrap();
        assert_eq!(d.frames.len(), 3);
        assert_eq!(d.loop_count, Some(4));
        assert_eq!(d.frames.iter().map(|f| f.delay_ms).collect::<Vec<_>>(), vec![120, 250, 40]);
        assert_eq!(d.frames[0].rgba8, a);
        assert_eq!(d.frames[1].rgba8, b);
        assert_eq!(&d.frames[2].rgba8[..4], &[0, 0, 255, 128]);
        assert_eq!(d.props.len(), 3);
    }

    #[test]
    fn an_ordinary_png_is_one_frame_and_bad_animation_input_is_refused() {
        let plain = encode_png(2, 2, &solid(2, 2, [9, 8, 7, 255]), &PngOptions::default()).unwrap();
        assert_eq!(frame_count(&plain), 1);
        assert!(read_animation(&plain).is_none());
        assert_eq!(decode(&plain).unwrap().frames.len(), 1);
        let a = solid(2, 2, [1, 2, 3, 255]);
        let b = solid(2, 2, [4, 5, 6, 255]);
        let one = encode_apng(2, 2, &[ApngFrame { rgba: &a, delay_ms: 10 }], 0, &PngOptions::default()).unwrap();
        assert_eq!(decode(&one).unwrap().frames.len(), 1, "a one-frame animation is a still picture");
        assert!(encode_apng(2, 2, &[], 0, &PngOptions::default()).is_none());
        assert!(encode_apng(2, 2, &[ApngFrame { rgba: &a[..4], delay_ms: 10 }], 0, &PngOptions::default()).is_none());
        let two = encode_apng(2, 2, &[ApngFrame { rgba: &a, delay_ms: 10 }, ApngFrame { rgba: &b, delay_ms: 10 }], 1, &PngOptions::default()).unwrap();
        assert_eq!(probe(&two).unwrap().png["FrameCount"], PropValue::Int(2));
    }

    #[test]
    fn blending_over_keeps_what_is_below_and_source_replaces_it() {
        let red = [255, 0, 0, 255];
        let half_blue = [0, 0, 255, 128];
        let over = composite(2, 1, &[sub(0, 0, 2, 1, Dispose::None, Blend::Source, red), sub(0, 0, 1, 1, Dispose::None, Blend::Over, half_blue)]);
        let px = &over[1][..4];
        assert_eq!(px[3], 255, "an opaque backdrop stays opaque");
        assert!(px[0] > 100 && px[0] < 155 && px[2] > 100 && px[2] < 155, "red and blue mixed about evenly: {px:?}");
        assert_eq!(&over[1][4..8], &red, "outside the rectangle nothing changes");
        let src = composite(2, 1, &[sub(0, 0, 2, 1, Dispose::None, Blend::Source, red), sub(0, 0, 1, 1, Dispose::None, Blend::Source, half_blue)]);
        assert_eq!(&src[1][..4], &half_blue);
    }

    #[test]
    fn dispose_ops_clear_or_restore_the_rectangle_after_the_frame() {
        let (red, green, blue) = ([255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255]);
        // Background: the rectangle goes transparent before the next frame is drawn.
        let bg = composite(2, 1, &[sub(0, 0, 2, 1, Dispose::None, Blend::Source, red), sub(0, 0, 1, 1, Dispose::Background, Blend::Source, green), sub(1, 0, 1, 1, Dispose::None, Blend::Source, blue)]);
        assert_eq!(&bg[1][..4], &green);
        assert_eq!(&bg[2][..4], &[0, 0, 0, 0], "the second frame's area was cleared");
        assert_eq!(&bg[2][4..], &blue);
        // Previous: the canvas returns to what it was before the frame.
        let prev = composite(2, 1, &[sub(0, 0, 2, 1, Dispose::None, Blend::Source, red), sub(0, 0, 1, 1, Dispose::Previous, Blend::Source, green), sub(1, 0, 1, 1, Dispose::None, Blend::Source, blue)]);
        assert_eq!(&prev[2][..4], &red);
        assert_eq!(&prev[2][4..], &blue);
        // The first frame cannot restore a state before it, so it clears.
        let first = composite(1, 1, &[sub(0, 0, 1, 1, Dispose::Previous, Blend::Source, red), sub(0, 0, 1, 1, Dispose::None, Blend::Over, [0, 0, 0, 0])]);
        assert_eq!(&first[0][..], &red);
        assert_eq!(&first[1][..], &[0, 0, 0, 0]);
    }

    #[test]
    fn a_frame_that_sticks_out_of_the_canvas_is_clipped() {
        let out = composite(2, 2, &[sub(1, 1, 3, 3, Dispose::None, Blend::Source, [5, 5, 5, 255])]);
        assert_eq!(out[0].len(), 16);
        assert_eq!(&out[0][12..], &[5, 5, 5, 255]);
        assert_eq!(&out[0][..12], &[0u8; 12]);
    }
}
