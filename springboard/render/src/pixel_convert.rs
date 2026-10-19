//! Conversion between `gfx::Image` pixels and the plane bytes of the formats in `pixel_format`: the four 32-bit
//! RGB layouts (ARGB8888, XRGB8888, ABGR8888, XBGR8888), RGB565, and the two YCbCr formats NV12 and NV16. This is
//! what lets a frame in a pixel buffer go through the image filters, and a filtered image go back into a buffer.
//!
//! Conventions: image pixels are premultiplied and packed with red in the low byte, as everywhere in the render
//! crate; the 32-bit layouts store them as they are (so alpha stays premultiplied) and the X formats store 0xff in
//! the unused byte and decode to opaque. YCbCr uses video range (Y 16 to 235, Cb and Cr 16 to 240) and the matrix
//! the caller names; the alpha channel is dropped on encode (the colour is already premultiplied, so this is the
//! image over black) and decode gives opaque pixels. Chroma is averaged over each subsampled block on encode and
//! taken from the covering sample (no interpolation) on decode.
use crate::buffer_attach::YcbcrMatrix;
use crate::gfx::Image;
use crate::pixel_format::PixelFormat;

/// The plane bytes of `img` in format `fmt`, one vector per plane with the sizes `PixelFormat::plane_size`
/// gives. `None` when the image's pixel count does not match its size, the size is empty, or the format has no
/// conversion here.
pub fn encode(img: &Image, fmt: &PixelFormat, matrix: YcbcrMatrix) -> Option<Vec<Vec<u8>>> {
    let (w, h) = (img.w, img.h);
    if w == 0 || h == 0 || img.px.len() != w.checked_mul(h)? {
        return None;
    }
    let code = fmt.code.bytes();
    let rgb = |p: u32| ((p & 255) as u8, (p >> 8 & 255) as u8, (p >> 16 & 255) as u8, (p >> 24) as u8);
    match &code {
        b"AR24" | b"XR24" | b"AB24" | b"XB24" => {
            let swap = code[1] == b'B';
            let keep_alpha = code[0] == b'A';
            let mut out = Vec::with_capacity(w * h * 4);
            for &p in &img.px {
                let (r, g, b, a) = rgb(p);
                let a = if keep_alpha { a } else { 0xff };
                out.extend_from_slice(&if swap { [r, g, b, a] } else { [b, g, r, a] });
            }
            Some(vec![out])
        }
        b"RG16" => {
            let mut out = Vec::with_capacity(w * h * 2);
            for &p in &img.px {
                let (r, g, b, _) = rgb(p);
                let v = (q(r, 31) << 11 | q(g, 63) << 5 | q(b, 31)) as u16;
                out.extend_from_slice(&v.to_le_bytes());
            }
            Some(vec![out])
        }
        b"NV12" | b"NV16" => {
            let kr_kg_kb = matrix.luma_weights();
            let (hs, vs) = (fmt.planes.get(1)?.h_sub, fmt.planes.get(1)?.v_sub);
            let (cw, ch) = ((w + hs - 1) / hs, (h + vs - 1) / vs);
            let mut luma = vec![0u8; w * h];
            let mut sums = vec![[0f32; 3]; cw * ch];
            let mut counts = vec![0f32; cw * ch];
            for y in 0..h {
                for x in 0..w {
                    let (r, g, b, _) = rgb(img.px[y * w + x]);
                    let (y8, cb, cr) = to_ycbcr(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, kr_kg_kb);
                    luma[y * w + x] = y8.round().clamp(0.0, 255.0) as u8;
                    let c = (y / vs) * cw + x / hs;
                    sums[c][1] += cb;
                    sums[c][2] += cr;
                    counts[c] += 1.0;
                }
            }
            let mut chroma = Vec::with_capacity(cw * ch * 2);
            for (s, n) in sums.iter().zip(&counts) {
                chroma.push((s[1] / n).round().clamp(0.0, 255.0) as u8);
                chroma.push((s[2] / n).round().clamp(0.0, 255.0) as u8);
            }
            Some(vec![luma, chroma])
        }
        _ => None,
    }
}

/// The image in `planes` (laid out as `PixelFormat::plane_size` says, tightly packed) of `width` by `height`
/// pixels in format `fmt`. `None` when a plane is shorter than the size needs, the size is empty, or the format has
/// no conversion here.
pub fn decode(fmt: &PixelFormat, planes: &[Vec<u8>], width: usize, height: usize, matrix: YcbcrMatrix) -> Option<Image> {
    if width == 0 || height == 0 {
        return None;
    }
    for (i, plane) in planes.iter().enumerate() {
        let (row, rows) = fmt.plane_size(i, width, height)?;
        if plane.len() < row.checked_mul(rows)? {
            return None;
        }
    }
    if planes.len() < fmt.planes.len() {
        return None;
    }
    let code = fmt.code.bytes();
    let mut img = Image::new(width, height);
    match &code {
        b"AR24" | b"XR24" | b"AB24" | b"XB24" => {
            let swap = code[1] == b'B';
            let keep_alpha = code[0] == b'A';
            for (px, c) in img.px.iter_mut().zip(planes[0].chunks_exact(4)) {
                let (r, g, b) = if swap { (c[0], c[1], c[2]) } else { (c[2], c[1], c[0]) };
                let a = if keep_alpha { c[3] } else { 0xff };
                *px = r as u32 | (g as u32) << 8 | (b as u32) << 16 | (a as u32) << 24;
            }
            Some(img)
        }
        b"RG16" => {
            for (px, c) in img.px.iter_mut().zip(planes[0].chunks_exact(2)) {
                let v = u16::from_le_bytes([c[0], c[1]]) as u32;
                let (r, g, b) = (up(v >> 11 & 31, 31), up(v >> 5 & 63, 63), up(v & 31, 31));
                *px = r | g << 8 | b << 16 | 0xff00_0000;
            }
            Some(img)
        }
        b"NV12" | b"NV16" => {
            let weights = matrix.luma_weights();
            let (hs, vs) = (fmt.planes.get(1)?.h_sub, fmt.planes.get(1)?.v_sub);
            let cw = (width + hs - 1) / hs;
            for y in 0..height {
                for x in 0..width {
                    let c = ((y / vs) * cw + x / hs) * 2;
                    let (r, g, b) = from_ycbcr(planes[0][y * width + x], planes[1][c], planes[1][c + 1], weights);
                    img.px[y * width + x] = r as u32 | (g as u32) << 8 | (b as u32) << 16 | 0xff00_0000;
                }
            }
            Some(img)
        }
        _ => None,
    }
}

/// An 8-bit value scaled to `max` levels, rounded.
fn q(v: u8, max: u32) -> u32 {
    (v as u32 * max + 127) / 255
}

/// A level in 0..=`max` scaled back to 8 bits, rounded.
fn up(v: u32, max: u32) -> u32 {
    (v * 255 + max / 2) / max
}

/// Video-range (Y, Cb, Cr) as floats in 0..=255 for RGB in 0..=1.
fn to_ycbcr(r: f32, g: f32, b: f32, w: [f64; 3]) -> (f32, f32, f32) {
    let (kr, kg, kb) = (w[0] as f32, w[1] as f32, w[2] as f32);
    let y = kr * r + kg * g + kb * b;
    let cb = (b - y) / (2.0 * (1.0 - kb));
    let cr = (r - y) / (2.0 * (1.0 - kr));
    (16.0 + 219.0 * y, 128.0 + 224.0 * cb, 128.0 + 224.0 * cr)
}

/// RGB bytes for a video-range (Y, Cb, Cr) sample.
fn from_ycbcr(y: u8, cb: u8, cr: u8, w: [f64; 3]) -> (u8, u8, u8) {
    let (kr, kg, kb) = (w[0] as f32, w[1] as f32, w[2] as f32);
    let yy = (y as f32 - 16.0) / 219.0;
    let pb = (cb as f32 - 128.0) / 224.0;
    let pr = (cr as f32 - 128.0) / 224.0;
    let r = yy + 2.0 * (1.0 - kr) * pr;
    let b = yy + 2.0 * (1.0 - kb) * pb;
    let g = (yy - kr * r - kb * b) / kg;
    let byte = |v: f32| (v * 255.0).round().clamp(0.0, 255.0) as u8;
    (byte(r), byte(g), byte(b))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pixel_format::{find, FourCc};

    fn fmt(code: &str) -> &'static PixelFormat {
        find(FourCc::parse(code).unwrap()).unwrap()
    }

    fn close(a: u32, b: u32, tol: i32) -> bool {
        (0..4).all(|i| ((a >> (8 * i) & 255) as i32 - (b >> (8 * i) & 255) as i32).abs() <= tol)
    }

    #[test]
    fn the_32_bit_layouts_store_the_channels_in_their_memory_order() {
        // red 0x33, green 0x22, blue 0x11, alpha 0xff
        let img = Image { w: 1, h: 1, px: vec![0xff11_2233] };
        assert_eq!(encode(&img, fmt("AR24"), YcbcrMatrix::Bt709).unwrap(), vec![vec![0x11, 0x22, 0x33, 0xff]]);
        assert_eq!(encode(&img, fmt("AB24"), YcbcrMatrix::Bt709).unwrap(), vec![vec![0x33, 0x22, 0x11, 0xff]]);
        let translucent = Image { w: 1, h: 1, px: vec![0x8011_2233] };
        assert_eq!(encode(&translucent, fmt("XR24"), YcbcrMatrix::Bt709).unwrap()[0][3], 0xff);
        assert_eq!(encode(&translucent, fmt("AR24"), YcbcrMatrix::Bt709).unwrap()[0][3], 0x80);
    }

    #[test]
    fn the_32_bit_layouts_round_trip_exactly() {
        let img = Image { w: 3, h: 1, px: vec![0xff11_2233, 0x8040_2010, 0xffff_ffff] };
        for code in ["AR24", "AB24"] {
            let planes = encode(&img, fmt(code), YcbcrMatrix::Bt709).unwrap();
            assert_eq!(decode(fmt(code), &planes, 3, 1, YcbcrMatrix::Bt709).unwrap(), img, "{code}");
        }
        let planes = encode(&img, fmt("XR24"), YcbcrMatrix::Bt709).unwrap();
        let back = decode(fmt("XR24"), &planes, 3, 1, YcbcrMatrix::Bt709).unwrap();
        assert!(back.px.iter().all(|&p| p >> 24 == 0xff));
        assert_eq!(back.px[0], img.px[0]);
    }

    #[test]
    fn rgb565_keeps_black_and_white_and_quantises_the_rest() {
        let img = Image { w: 3, h: 1, px: vec![0xff00_0000, 0xffff_ffff, 0xff80_8080] };
        let planes = encode(&img, fmt("RG16"), YcbcrMatrix::Bt709).unwrap();
        assert_eq!(planes[0].len(), 6);
        let back = decode(fmt("RG16"), &planes, 3, 1, YcbcrMatrix::Bt709).unwrap();
        assert_eq!(back.px[0], 0xff00_0000);
        assert_eq!(back.px[1], 0xffff_ffff);
        assert!(close(back.px[2], 0xff80_8080, 8));
    }

    #[test]
    fn nv12_round_trips_solid_colours_within_rounding() {
        for px in [0xff00_00ff, 0xff00_ff00, 0xffff_0000, 0xff80_8080, 0xff00_0000, 0xffff_ffff] {
            let img = Image { w: 4, h: 2, px: vec![px; 8] };
            for matrix in [YcbcrMatrix::Bt601, YcbcrMatrix::Bt709, YcbcrMatrix::Bt2020] {
                let planes = encode(&img, fmt("NV12"), matrix).unwrap();
                assert_eq!((planes[0].len(), planes[1].len()), (8, 4));
                let back = decode(fmt("NV12"), &planes, 4, 2, matrix).unwrap();
                assert!(close(back.px[0], px, 4), "{px:#x} -> {:#x} with {matrix:?}", back.px[0]);
            }
        }
    }

    #[test]
    fn nv16_keeps_one_chroma_row_per_picture_row_and_nv12_odd_sizes_round_up() {
        let img = Image { w: 3, h: 3, px: vec![0xff40_80c0; 9] };
        let nv16 = encode(&img, fmt("NV16"), YcbcrMatrix::Bt709).unwrap();
        assert_eq!((nv16[0].len(), nv16[1].len()), (9, 2 * 2 * 3));
        let nv12 = encode(&img, fmt("NV12"), YcbcrMatrix::Bt709).unwrap();
        assert_eq!((nv12[0].len(), nv12[1].len()), (9, 2 * 2 * 2));
        let back = decode(fmt("NV12"), &nv12, 3, 3, YcbcrMatrix::Bt709).unwrap();
        assert!(close(back.px[8], 0xff40_80c0, 4));
    }

    #[test]
    fn chroma_is_averaged_over_each_block() {
        // Left pixel pure red, right pixel pure blue, one 2x1 block of NV12 at height 1 per chroma row.
        let img = Image { w: 2, h: 2, px: vec![0xff00_00ff, 0xffff_0000, 0xff00_00ff, 0xffff_0000] };
        let planes = encode(&img, fmt("NV12"), YcbcrMatrix::Bt709).unwrap();
        let one = encode(&Image { w: 2, h: 2, px: vec![0xff80_0080; 4] }, fmt("NV12"), YcbcrMatrix::Bt709).unwrap();
        // The averaged colour (red plus blue over two) has chroma close to that of the solid mix, not of either end.
        assert!((planes[1][0] as i32 - one[1][0] as i32).abs() <= 12);
        assert!((planes[1][1] as i32 - one[1][1] as i32).abs() <= 12);
    }

    #[test]
    fn bad_inputs_are_refused() {
        let img = Image { w: 2, h: 2, px: vec![0; 3] };
        assert!(encode(&img, fmt("AR24"), YcbcrMatrix::Bt709).is_none());
        assert!(encode(&Image { w: 0, h: 0, px: vec![] }, fmt("AR24"), YcbcrMatrix::Bt709).is_none());
        assert!(decode(fmt("AR24"), &[vec![0u8; 3]], 2, 2, YcbcrMatrix::Bt709).is_none());
        assert!(decode(fmt("NV12"), &[vec![0u8; 4]], 2, 2, YcbcrMatrix::Bt709).is_none());
        assert!(decode(fmt("AR24"), &[vec![0u8; 16]], 0, 2, YcbcrMatrix::Bt709).is_none());
    }
}
