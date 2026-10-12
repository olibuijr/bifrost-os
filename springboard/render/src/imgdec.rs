//! Image decoding shared by the apps: JPEG (zune-jpeg) and PNG (png) into
//! premultiplied RGBA, plus the scaling helpers that make thumbnails and
//! screen-fitted pictures out of them.

use crate::gfx::{pack, rgba, Image};
use std::io::Cursor;

/// Pictures wider or taller than this are refused rather than decoded.
pub const MAX_SIDE: usize = 8192;

/// Decodes a JPEG or PNG into premultiplied RGBA; None for anything else.
pub fn decode(bytes: &[u8]) -> Option<Image> {
    match bytes {
        [0xFF, 0xD8, ..] => decode_jpeg(bytes),
        [0x89, b'P', b'N', b'G', ..] => decode_png(bytes),
        _ => None,
    }
}

/// True for file names the decoder handles.
pub fn is_image_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    ["jpg", "jpeg", "png"].iter().any(|ext| lower.rsplit_once('.').is_some_and(|(_, e)| e == *ext))
}

fn decode_jpeg(bytes: &[u8]) -> Option<Image> {
    use zune_core::colorspace::ColorSpace;
    use zune_core::options::DecoderOptions;
    let opts = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::RGB).set_max_width(MAX_SIDE).set_max_height(MAX_SIDE);
    let mut d = zune_jpeg::JpegDecoder::new_with_options(Cursor::new(bytes), opts);
    let data = d.decode().ok()?;
    let info = d.info()?;
    let (w, h) = (info.width as usize, info.height as usize);
    if data.len() < w * h * 3 {
        return None;
    }
    Some(Image { w, h, px: data.as_chunks::<3>().0.iter().map(|p| pack(p[0], p[1], p[2], 255)).collect() })
}

fn decode_png(bytes: &[u8]) -> Option<Image> {
    use png::{BitDepth, ColorType, Transformations};
    let mut d = png::Decoder::new(Cursor::new(bytes));
    d.set_transformations(Transformations::EXPAND | Transformations::STRIP_16 | Transformations::ALPHA);
    let mut r = d.read_info().ok()?;
    let (w, h) = (r.info().width as usize, r.info().height as usize);
    if w > MAX_SIDE || h > MAX_SIDE {
        return None;
    }
    let mut buf = vec![0u8; r.output_buffer_size()?];
    let out = r.next_frame(&mut buf).ok()?;
    let (ct, depth) = (out.color_type, out.bit_depth);
    if depth != BitDepth::Eight {
        return None;
    }
    let px = match ct {
        ColorType::Rgba => buf.as_chunks::<4>().0.iter().map(|p| rgba(p[0], p[1], p[2], p[3])).collect(),
        ColorType::Rgb => buf.as_chunks::<3>().0.iter().map(|p| pack(p[0], p[1], p[2], 255)).collect(),
        ColorType::GrayscaleAlpha => buf.as_chunks::<2>().0.iter().map(|p| rgba(p[0], p[0], p[0], p[1])).collect(),
        ColorType::Grayscale => buf.iter().map(|&g| pack(g, g, g, 255)).collect(),
        ColorType::Indexed => return None,
    };
    Some(Image { w, h, px })
}

/// Shrinks by an integer factor with a box filter (keeps fine detail from aliasing).
pub fn box_down(img: &Image, f: usize) -> Image {
    if f <= 1 {
        return img.clone();
    }
    let (w, h) = (img.w / f, img.h / f);
    let mut out = Image::new(w, h);
    let n = (f * f) as u32;
    for y in 0..h {
        for x in 0..w {
            let mut sum = [0u32; 4];
            for sy in 0..f {
                for sx in 0..f {
                    let p = img.px[(y * f + sy) * img.w + x * f + sx];
                    for (c, s) in sum.iter_mut().enumerate() {
                        *s += (p >> (c * 8)) & 255;
                    }
                }
            }
            out.px[y * w + x] = (0..4).fold(0, |acc, c| acc | ((sum[c] + n / 2) / n) << (c * 8));
        }
    }
    out
}

/// Scales to fill `w` x `h` keeping the aspect, cropping the overflow centred.
pub fn cover(img: &Image, w: usize, h: usize) -> Image {
    if img.w == 0 || img.h == 0 || w == 0 || h == 0 {
        return Image::new(w, h);
    }
    let scale = (w as f32 / img.w as f32).max(h as f32 / img.h as f32);
    let (sw, sh) = (((img.w as f32 * scale).round() as usize).max(w), ((img.h as f32 * scale).round() as usize).max(h));
    let factor = ((img.w / sw).min(img.h / sh)).max(1);
    let scaled = box_down(img, factor).resized(sw, sh);
    scaled.crop((sw - w) / 2, (sh - h) / 2, w, h)
}

/// The size `img` takes when scaled to fit inside `w` x `h` (never enlarged).
pub fn fit_size(img: &Image, w: usize, h: usize) -> (usize, usize) {
    fit_dims(img.w, img.h, w, h)
}

/// The fitted size of an `iw` x `ih` picture inside `w` x `h` (never enlarged), from the
/// dimensions alone, so a header-only size needs no pixels. `(0, 0)` for an empty picture.
pub fn fit_dims(iw: usize, ih: usize, w: usize, h: usize) -> (usize, usize) {
    if iw == 0 || ih == 0 {
        return (0, 0);
    }
    let k = (w as f32 / iw as f32).min(h as f32 / ih as f32).min(1.0);
    (((iw as f32 * k).round() as usize).max(1), ((ih as f32 * k).round() as usize).max(1))
}

/// Scales down to fit inside `w` x `h` keeping the aspect; smaller images
/// are returned as they are.
pub fn contain(img: &Image, w: usize, h: usize) -> Image {
    let (fw, fh) = fit_size(img, w, h);
    if (fw, fh) == (img.w, img.h) || fw == 0 {
        return img.clone();
    }
    let factor = ((img.w / fw).min(img.h / fh)).max(1);
    box_down(img, factor).resized(fw, fh)
}

/// Scales `img` down to fit inside `w` x `h`, keeping the aspect; smaller
/// images are returned as they are (never enlarged).
pub fn fit(img: &Image, w: usize, h: usize) -> Image {
    if img.w == 0 || img.h == 0 || w == 0 || h == 0 || (img.w <= w && img.h <= h) {
        return img.clone();
    }
    let k = (w as f32 / img.w as f32).min(h as f32 / img.h as f32);
    img.resized(((img.w as f32 * k).round() as usize).max(1), ((img.h as f32 * k).round() as usize).max(1))
}

/// True when `bytes` start like a format `decode` understands.
pub fn is_image(bytes: &[u8]) -> bool {
    matches!(bytes, [0xFF, 0xD8, ..] | [0x89, b'P', b'N', b'G', ..])
}

#[cfg(any(test, feature = "testkit"))]
pub mod tests {
    #[cfg(test)]
    use super::*;

    pub fn png_bytes(w: u32, h: u32, color: [u8; 4]) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, w, h);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut wr = enc.write_header().unwrap();
            let data: Vec<u8> = (0..w * h).flat_map(|_| color).collect();
            wr.write_image_data(&data).unwrap();
        }
        out
    }

    #[cfg(test)]
    fn solid(w: usize, h: usize, c: u32) -> Image {
        Image { w, h, px: vec![c; w * h] }
    }

    #[test]
    fn png_decodes_to_premultiplied_pixels() {
        let img = decode(&png_bytes(3, 2, [200, 100, 0, 128])).unwrap();
        assert_eq!((img.w, img.h), (3, 2));
        assert_eq!(img.px[0], rgba(200, 100, 0, 128));
        assert!(decode(b"not an image").is_none());
        assert!(decode(&[0xFF, 0xD8, 0xFF, 0x00]).is_none(), "truncated jpeg");
    }

    #[test]
    fn jpeg_decodes_the_embedded_fixture() {
        let img = decode(include_bytes!("../../src/deildu/fixtures/dot.jpg")).unwrap();
        assert_eq!((img.w, img.h), (8, 8));
        assert!(img.px.iter().all(|p| p >> 24 == 255));
        assert!(img.px[27] & 0xff > 200, "red centre: {:08x}", img.px[27]);
    }

    #[test]
    fn image_names_are_matched_by_extension() {
        assert!(is_image_name("IMG_0001.JPG") && is_image_name("a.jpeg") && is_image_name("Screenshot-1.png"));
        assert!(!is_image_name("movie.mp4") && !is_image_name("png") && !is_image_name("notes.txt"));
    }

    #[test]
    fn cover_fills_and_centres() {
        let mut img = solid(100, 50, pack(0, 0, 255, 255));
        for y in 0..50 {
            for x in 40..60 {
                img.px[y * 100 + x] = pack(255, 0, 0, 255);
            }
        }
        let c = cover(&img, 20, 20);
        assert_eq!((c.w, c.h), (20, 20));
        assert_eq!(c.px[10 * 20 + 10] & 0xff, 255, "the red band stays centred");
        assert_eq!(c.px[10 * 20 + 1] >> 16 & 0xff, 255, "blue at the edges");
        let big = cover(&solid(10, 10, 7), 40, 20);
        assert_eq!((big.w, big.h), (40, 20));
        assert!(big.px.iter().all(|&p| p == 7));
        assert_eq!(cover(&Image::new(0, 0), 4, 4).px.len(), 16);
    }

    #[test]
    fn contain_shrinks_to_fit_and_never_enlarges() {
        let img = solid(4000, 3000, 5);
        let c = contain(&img, 1200, 1920);
        assert_eq!((c.w, c.h), (1200, 900));
        assert!(c.px.iter().all(|&p| p == 5));
        let tall = contain(&solid(300, 3000, 5), 1200, 1920);
        assert_eq!((tall.w, tall.h), (192, 1920));
        let small = contain(&solid(10, 10, 5), 1200, 1920);
        assert_eq!((small.w, small.h), (10, 10));
        assert_eq!(fit_size(&Image::new(0, 0), 10, 10), (0, 0));
    }
}

#[cfg(test)]
pub(crate) mod files_tests {
    use super::*;

    /// A solid `w` x `h` RGBA PNG.
    pub(crate) fn png_bytes(w: u32, h: u32, color: [u8; 4]) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, w, h);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut wr = enc.write_header().unwrap();
            let data: Vec<u8> = (0..w * h).flat_map(|_| color).collect();
            wr.write_image_data(&data).unwrap();
        }
        out
    }

    #[test]
    fn png_decodes_to_premultiplied_pixels() {
        let img = decode(&png_bytes(3, 2, [200, 100, 0, 128])).unwrap();
        assert_eq!((img.w, img.h), (3, 2));
        assert_eq!(img.px[0], rgba(200, 100, 0, 128));
        assert!(decode(b"not an image").is_none());
        assert!(decode(&[0xFF, 0xD8, 0xFF, 0x00]).is_none(), "truncated jpeg");
        assert!(is_image(&png_bytes(1, 1, [0; 4])) && !is_image(b"text"));
    }

    #[test]
    fn jpeg_decodes() {
        let img = decode(include_bytes!("../../src/deildu/fixtures/dot.jpg")).unwrap();
        assert_eq!((img.w, img.h), (8, 8));
        assert!(img.px[27] & 0xff > 200, "red centre: {:08x}", img.px[27]);
    }

    #[test]
    fn fit_only_shrinks() {
        let big = Image { w: 400, h: 200, px: vec![1; 400 * 200] };
        let f = fit(&big, 100, 100);
        assert_eq!((f.w, f.h), (100, 50));
        let small = Image { w: 10, h: 20, px: vec![1; 200] };
        assert_eq!(fit(&small, 100, 100), small);
        assert_eq!(fit(&Image::new(0, 0), 10, 10).px.len(), 0);
    }
}
