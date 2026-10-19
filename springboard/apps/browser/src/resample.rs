//! `browser.screenshot` `scale`: resamples the engine's PNG in the app. The engine always captures at the view's
//! device pixel size, so a scale below 1 averages source pixels (box filter) and above 1 interpolates (bilinear).
use png::{BitDepth, ColorType, Decoder, Transformations};

/// Most pixels a scaled screenshot may have.
pub const MAX_OUT_PIXELS: u64 = 64_000_000;

/// New size for `scale`, at least 1 x 1.
pub fn scaled_size(w: u32, h: u32, scale: f32) -> (u32, u32) {
    (((w as f32 * scale).round() as u32).max(1), ((h as f32 * scale).round() as u32).max(1))
}

/// Resamples RGBA8 pixels (`w * h * 4` bytes) to `nw x nh`.
pub fn resample_rgba(src: &[u8], w: u32, h: u32, nw: u32, nh: u32) -> Vec<u8> {
    let (w, h, nw, nh) = (w as usize, h as usize, nw as usize, nh as usize);
    let mut out = vec![0u8; nw * nh * 4];
    let (sx, sy) = (w as f32 / nw as f32, h as f32 / nh as f32);
    let px = |x: usize, y: usize, c: usize| src[(y * w + x) * 4 + c] as f32;
    for dy in 0..nh {
        for dx in 0..nw {
            let o = (dy * nw + dx) * 4;
            if sx > 1.0 || sy > 1.0 {
                // box average over the source footprint of this pixel
                let x0 = ((dx as f32 * sx) as usize).min(w - 1);
                let y0 = ((dy as f32 * sy) as usize).min(h - 1);
                let x1 = (((dx + 1) as f32 * sx).ceil() as usize).clamp(x0 + 1, w);
                let y1 = (((dy + 1) as f32 * sy).ceil() as usize).clamp(y0 + 1, h);
                let n = ((x1 - x0) * (y1 - y0)) as f32;
                for c in 0..4 {
                    let mut sum = 0.0;
                    for y in y0..y1 {
                        for x in x0..x1 {
                            sum += px(x, y, c);
                        }
                    }
                    out[o + c] = (sum / n).round() as u8;
                }
            } else {
                // bilinear at the pixel centre
                let fx = ((dx as f32 + 0.5) * sx - 0.5).clamp(0.0, (w - 1) as f32);
                let fy = ((dy as f32 + 0.5) * sy - 0.5).clamp(0.0, (h - 1) as f32);
                let (x0, y0) = (fx as usize, fy as usize);
                let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
                let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
                for c in 0..4 {
                    let top = px(x0, y0, c) * (1.0 - tx) + px(x1, y0, c) * tx;
                    let bot = px(x0, y1, c) * (1.0 - tx) + px(x1, y1, c) * tx;
                    out[o + c] = (top * (1.0 - ty) + bot * ty).round() as u8;
                }
            }
        }
    }
    out
}

/// The PNG's pixels as RGBA8 with its width and height.
pub fn decode_rgba(png: &[u8]) -> Result<(Vec<u8>, u32, u32), String> {
    let mut dec = Decoder::new(std::io::Cursor::new(png));
    dec.set_transformations(Transformations::EXPAND | Transformations::STRIP_16);
    let mut reader = dec.read_info().map_err(|e| format!("cannot decode the screenshot: {e}"))?;
    let mut buf = vec![0; reader.output_buffer_size().ok_or("screenshot too large")?];
    let info = reader.next_frame(&mut buf).map_err(|e| format!("cannot decode the screenshot: {e}"))?;
    let data = &buf[..info.buffer_size()];
    let rgba: Vec<u8> = match info.color_type {
        ColorType::Rgba => data.to_vec(),
        ColorType::Rgb => data.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        ColorType::Grayscale => data.iter().flat_map(|g| [*g, *g, *g, 255]).collect(),
        ColorType::GrayscaleAlpha => data.chunks_exact(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        ColorType::Indexed => return Err("indexed PNG after expansion".into()),
    };
    Ok((rgba, info.width, info.height))
}

/// The PNG as a printed page: a paginated PDF (`bifrost_app::web_print`) titled `title`. Returns the PDF and its
/// page count.
pub fn png_to_pdf(png: &[u8], title: &str, layout: &bifrost_app::web_print::PrintLayout) -> Result<(Vec<u8>, usize), String> {
    let (rgba, w, h) = decode_rgba(png)?;
    let rgb = bifrost_app::web_print::rgba_to_rgb(&rgba, w as usize, h as usize)?;
    let pages = bifrost_app::web_print::plan_pages(w as usize, h as usize, layout).len();
    let pdf = bifrost_app::web_print::pdf_from_rgb(&rgb, w as usize, h as usize, title, layout)?;
    Ok((pdf, pages))
}

/// The PNG scaled by `scale` (0.1 to 4). A scale within 0.5 % of 1 returns the input unchanged.
pub fn scale_png(png: &[u8], scale: f32) -> Result<Vec<u8>, String> {
    if (scale - 1.0).abs() < 0.005 {
        return Ok(png.to_vec());
    }
    let (rgba, w, h) = decode_rgba(png)?;
    let (nw, nh) = scaled_size(w, h, scale);
    if nw as u64 * nh as u64 > MAX_OUT_PIXELS {
        return Err(format!("scaled screenshot would be {nw} x {nh} pixels: use a smaller scale"));
    }
    let out = resample_rgba(&rgba, w, h, nw, nh);
    let mut bytes = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut bytes, nw, nh);
        enc.set_color(ColorType::Rgba);
        enc.set_depth(BitDepth::Eight);
        let mut wr = enc.write_header().map_err(|e| e.to_string())?;
        wr.write_image_data(&out).map_err(|e| e.to_string())?;
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn halves_and_doubles() {
        let src: Vec<u8> = (0..16).flat_map(|i| [(i * 10) as u8, 0, 0, 255]).collect();
        let half = resample_rgba(&src, 4, 4, 2, 2);
        assert_eq!(half.len(), 16);
        assert_eq!(half[0], (((10 + 40 + 50) as f32) / 4.0).round() as u8);
        let big = resample_rgba(&half, 2, 2, 4, 4);
        assert_eq!(big.len(), 64);
        assert_eq!(scaled_size(1, 1, 0.1), (1, 1));
    }

    fn tiny_png(w: u32, h: u32, fill: u8) -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut bytes, w, h);
            enc.set_color(ColorType::Rgba);
            enc.set_depth(BitDepth::Eight);
            let mut wr = enc.write_header().unwrap();
            wr.write_image_data(&vec![fill; (w * h * 4) as usize]).unwrap();
        }
        bytes
    }

    #[test]
    fn a_png_prints_as_a_paginated_pdf() {
        let layout = bifrost_app::web_print::PrintLayout { paper_width: 200.0, paper_height: 300.0, margin: 20.0, header_footer: false };
        // 80 px wide scales 2 points per pixel, 130 rows per page: 300 rows are 3 pages
        let (pdf, pages) = png_to_pdf(&tiny_png(80, 300, 200), "t", &layout).unwrap();
        assert_eq!(pages, 3);
        assert!(pdf.starts_with(b"%PDF"));
        let doc = bifrost_app::graphics_pdf::Document::open(pdf).unwrap();
        assert_eq!(doc.page_count(), 3);
        assert!(png_to_pdf(b"not a png", "t", &layout).is_err());
    }

    #[test]
    fn scale_png_changes_the_size() {
        let mut bytes = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut bytes, 8, 6);
            enc.set_color(ColorType::Rgba);
            enc.set_depth(BitDepth::Eight);
            let mut w = enc.write_header().unwrap();
            w.write_image_data(&[200u8; 8 * 6 * 4]).unwrap();
        }
        let out = scale_png(&bytes, 0.5).unwrap();
        let be = |i: usize| u32::from_be_bytes([out[i], out[i + 1], out[i + 2], out[i + 3]]);
        assert_eq!((be(16), be(20)), (4, 3));
        assert_eq!(scale_png(&bytes, 1.0).unwrap(), bytes);
    }
}
