//! Pixel-format conversion for video capture and decode: packed RGBA to the two-plane 4:2:0 NV12 layout
//! a capture-to-encoder path hands over, 10-bit P010 (the layout a 10-bit software decoder outputs) back
//! to packed RGBA, and un-premultiplying captured frames. The 8-bit NV12 to RGBA direction and
//! rotation/mirroring are in `video_transfer`; the HDR transfer curves are in `hdr_frame`. The GLSL twin
//! of the RGBA to NV12 luma step is `springboard/gpu/shaders/rgba_to_nv12.comp`. Matrix coefficients are
//! the published BT.601 and BT.709 luma weights (Kr, Kb); ranges follow ITU-R limited (16..235 luma,
//! 16..240 chroma at 8 bit) and full. Pixels are packed RGBA with red in the low byte.

/// The luma weights of a colour matrix.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Matrix {
    Bt601,
    Bt709,
}

impl Matrix {
    /// `(Kr, Kb)`; Kg is `1 - Kr - Kb`.
    pub fn weights(self) -> (f32, f32) {
        match self {
            Matrix::Bt601 => (0.299, 0.114),
            Matrix::Bt709 => (0.2126, 0.0722),
        }
    }

    /// HD material (720 lines or more) uses BT.709, smaller uses BT.601.
    pub fn for_height(h: usize) -> Matrix {
        if h >= 720 {
            Matrix::Bt709
        } else {
            Matrix::Bt601
        }
    }
}

/// The code range of the encoded signal.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Range {
    Limited,
    Full,
}

/// The gamma-encoded `(Y, Cb, Cr)` of an RGB triple in 0..=1: Y in 0..=1, chroma in -0.5..=0.5.
pub fn rgb_to_ycbcr(rgb: [f32; 3], m: Matrix) -> [f32; 3] {
    let (kr, kb) = m.weights();
    let kg = 1.0 - kr - kb;
    let y = kr * rgb[0] + kg * rgb[1] + kb * rgb[2];
    let cb = (rgb[2] - y) / (2.0 * (1.0 - kb));
    let cr = (rgb[0] - y) / (2.0 * (1.0 - kr));
    [y, cb, cr]
}

/// The inverse of [`rgb_to_ycbcr`], unclamped.
pub fn ycbcr_to_rgb(ycc: [f32; 3], m: Matrix) -> [f32; 3] {
    let (kr, kb) = m.weights();
    let kg = 1.0 - kr - kb;
    let r = ycc[0] + 2.0 * (1.0 - kr) * ycc[2];
    let b = ycc[0] + 2.0 * (1.0 - kb) * ycc[1];
    let g = (ycc[0] - kr * r - kb * b) / kg;
    [r, g, b]
}

fn quantise(v: f32, scale: f32, offset: f32) -> u8 {
    (v * scale + offset).round().clamp(0.0, 255.0) as u8
}

/// Converts packed RGBA to NV12: a full-size luma plane and one interleaved Cb/Cr plane at half width
/// and height (each chroma pair averages its 2x2 block of RGB). Width and height must be even and
/// non-zero. Returns `(luma, chroma)` or `None`.
pub fn rgba_to_nv12(rgba: &[u32], w: usize, h: usize, m: Matrix, range: Range) -> Option<(Vec<u8>, Vec<u8>)> {
    if w == 0 || h == 0 || w % 2 != 0 || h % 2 != 0 || rgba.len() != w * h {
        return None;
    }
    let (y_scale, y_off, c_scale, c_off) = match range {
        Range::Limited => (219.0, 16.0, 224.0, 128.0),
        Range::Full => (255.0, 0.0, 255.0, 128.0),
    };
    let rgb_of = |p: u32| -> [f32; 3] {
        [(p & 0xff) as f32 / 255.0, ((p >> 8) & 0xff) as f32 / 255.0, ((p >> 16) & 0xff) as f32 / 255.0]
    };
    let mut luma = Vec::with_capacity(w * h);
    for p in rgba {
        luma.push(quantise(rgb_to_ycbcr(rgb_of(*p), m)[0], y_scale, y_off));
    }
    let mut chroma = Vec::with_capacity(w * h / 2);
    for by in 0..h / 2 {
        for bx in 0..w / 2 {
            let mut avg = [0.0f32; 3];
            for (dx, dy) in [(0usize, 0usize), (1, 0), (0, 1), (1, 1)] {
                let c = rgb_of(rgba[(2 * by + dy) * w + 2 * bx + dx]);
                for i in 0..3 {
                    avg[i] += c[i] * 0.25;
                }
            }
            let ycc = rgb_to_ycbcr(avg, m);
            chroma.push(quantise(ycc[1], c_scale, c_off));
            chroma.push(quantise(ycc[2], c_scale, c_off));
        }
    }
    Some((luma, chroma))
}

/// Converts 10-bit P010 (each sample is the 10-bit value in the high bits of a `u16`; luma plane then an
/// interleaved Cb/Cr plane at half size) to opaque packed RGBA. Chroma is nearest (co-sited). `None` on
/// odd or zero dimensions or short planes.
pub fn p010_to_rgba(luma: &[u16], chroma: &[u16], w: usize, h: usize, m: Matrix, range: Range) -> Option<Vec<u32>> {
    if w == 0 || h == 0 || w % 2 != 0 || h % 2 != 0 || luma.len() != w * h || chroma.len() != w * h / 2 {
        return None;
    }
    let mut out = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            let y10 = (luma[y * w + x] >> 6) as f32;
            let ci = ((y / 2) * (w / 2) + x / 2) * 2;
            let cb10 = (chroma[ci] >> 6) as f32;
            let cr10 = (chroma[ci + 1] >> 6) as f32;
            let ycc = match range {
                Range::Limited => [(y10 - 64.0) / 876.0, (cb10 - 512.0) / 896.0, (cr10 - 512.0) / 896.0],
                Range::Full => [y10 / 1023.0, (cb10 - 512.0) / 1023.0, (cr10 - 512.0) / 1023.0],
            };
            let rgb = ycbcr_to_rgb(ycc, m);
            let q = |v: f32| -> u32 { (v * 255.0).round().clamp(0.0, 255.0) as u32 };
            out.push(q(rgb[0]) | q(rgb[1]) << 8 | q(rgb[2]) << 16 | 0xff00_0000);
        }
    }
    Some(out)
}

/// Un-premultiplies packed RGBA in place (a capture surface is premultiplied; an encoder wants straight
/// colour). Alpha 0 becomes transparent black; alpha 255 is untouched.
pub fn unpremultiply_rgba(px: &mut [u32]) {
    for p in px.iter_mut() {
        let a = *p >> 24;
        if a == 0 {
            *p = 0;
            continue;
        }
        if a == 255 {
            continue;
        }
        let mut out = a << 24;
        for shift in [0u32, 8, 16] {
            let c = (*p >> shift) & 0xff;
            let v = ((c * 255 + a / 2) / a).min(255);
            out |= v << shift;
        }
        *p = out;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(p: u32, n: usize) -> Vec<u32> {
        vec![p; n]
    }

    #[test]
    fn limited_range_white_black_and_red() {
        let white = solid(0xffff_ffff, 4);
        let (y, c) = rgba_to_nv12(&white, 2, 2, Matrix::Bt601, Range::Limited).unwrap();
        assert_eq!(y, vec![235; 4]);
        assert_eq!(c, vec![128, 128]);
        let black = solid(0xff00_0000, 4);
        let (y, _) = rgba_to_nv12(&black, 2, 2, Matrix::Bt601, Range::Limited).unwrap();
        assert_eq!(y, vec![16; 4]);
        let red = solid(0xff00_00ff, 4);
        let (y, c) = rgba_to_nv12(&red, 2, 2, Matrix::Bt601, Range::Limited).unwrap();
        assert!((y[0] as i32 - 82).abs() <= 1);
        assert_eq!(c[1], 240, "red is the Cr maximum");
        assert!((c[0] as i32 - 90).abs() <= 1);
    }

    #[test]
    fn full_range_uses_the_whole_byte() {
        let white = solid(0xffff_ffff, 4);
        let (y, _) = rgba_to_nv12(&white, 2, 2, Matrix::Bt709, Range::Full).unwrap();
        assert_eq!(y, vec![255; 4]);
    }

    #[test]
    fn odd_or_mismatched_sizes_are_refused() {
        assert!(rgba_to_nv12(&[0; 6], 3, 2, Matrix::Bt709, Range::Limited).is_none());
        assert!(rgba_to_nv12(&[0; 3], 2, 2, Matrix::Bt709, Range::Limited).is_none());
        assert!(p010_to_rgba(&[0; 4], &[0; 1], 2, 2, Matrix::Bt709, Range::Limited).is_none());
    }

    #[test]
    fn matrix_round_trips_an_arbitrary_colour() {
        for m in [Matrix::Bt601, Matrix::Bt709] {
            let rgb = [0.2, 0.6, 0.9];
            let back = ycbcr_to_rgb(rgb_to_ycbcr(rgb, m), m);
            for i in 0..3 {
                assert!((back[i] - rgb[i]).abs() < 1e-5);
            }
        }
        assert_eq!(Matrix::for_height(1080), Matrix::Bt709);
        assert_eq!(Matrix::for_height(480), Matrix::Bt601);
    }

    #[test]
    fn nv12_to_p010_to_rgba_round_trips_a_flat_colour() {
        let src = solid(0xff80_4020, 16); // R 0x20, G 0x40, B 0x80
        let (y8, c8) = rgba_to_nv12(&src, 4, 4, Matrix::Bt709, Range::Limited).unwrap();
        let y10: Vec<u16> = y8.iter().map(|v| (*v as u16) << 8).collect();
        let c10: Vec<u16> = c8.iter().map(|v| (*v as u16) << 8).collect();
        let back = p010_to_rgba(&y10, &c10, 4, 4, Matrix::Bt709, Range::Limited).unwrap();
        for p in back {
            assert_eq!(p >> 24, 0xff);
            assert!(((p & 0xff) as i32 - 0x20).abs() <= 3);
            assert!((((p >> 8) & 0xff) as i32 - 0x40).abs() <= 3);
            assert!((((p >> 16) & 0xff) as i32 - 0x80).abs() <= 3);
        }
    }

    #[test]
    fn unpremultiply_restores_straight_colour() {
        // 50% alpha, premultiplied red 0x40 over alpha 0x80 -> straight red close to 0x7f.
        let mut px = [0x8000_0040u32, 0u32, 0xff12_3456u32];
        unpremultiply_rgba(&mut px);
        assert!(((px[0] & 0xff) as i32 - 0x7f).abs() <= 1);
        assert_eq!(px[0] >> 24, 0x80);
        assert_eq!(px[1], 0);
        assert_eq!(px[2], 0xff12_3456);
    }
}
