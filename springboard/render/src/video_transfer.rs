//! Pixel transfer and pixel rotation for decoded video pictures, as plain std code.
//! A transfer converts an NV12 picture to RGBA and scales it to a destination size
//! (nearest neighbour). A rotation turns a picture in quarter turns and optionally
//! mirrors it first. Frames are `gfx::Image`s, premultiplied RGBA packed with R in
//! the low byte, like the rest of the renderer. No GPU, no device access.

use crate::gfx::rotate::{rotated, Rotation};
use crate::gfx::Image;

/// A borrowed NV12 picture: a luma plane and an interleaved CbCr plane, `stride`
/// bytes per row for both. `w` and `h` are the picture size in luma pixels.
#[derive(Clone, Copy, Debug)]
pub struct Nv12<'a> {
    pub y: &'a [u8],
    pub uv: &'a [u8],
    pub stride: usize,
    pub w: usize,
    pub h: usize,
}

/// The YCbCr to RGB matrix. Coefficients are fixed point with 8 fraction bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Colorimetry {
    pub rv: i32,
    pub gu: i32,
    pub gv: i32,
    pub bu: i32,
}

/// BT.601 (standard definition) coefficients.
pub const BT601: Colorimetry = Colorimetry { rv: 409, gu: -100, gv: -208, bu: 516 };
/// BT.709 (high definition) coefficients.
pub const BT709: Colorimetry = Colorimetry { rv: 459, gu: -55, gv: -136, bu: 541 };

impl Colorimetry {
    /// HD material (at least 720 lines) is BT.709; everything smaller is BT.601.
    pub fn for_height(h: usize) -> Colorimetry {
        if h >= 720 {
            BT709
        } else {
            BT601
        }
    }
}

fn clamp8(v: i32) -> u32 {
    v.clamp(0, 255) as u32
}

/// One pixel from limited-range Y, Cb and Cr, opaque, R in the low byte.
fn yuv_to_rgba(y: u8, cb: u8, cr: u8, m: Colorimetry) -> u32 {
    let yy = 298 * (y as i32 - 16);
    let u = cb as i32 - 128;
    let v = cr as i32 - 128;
    let r = clamp8((yy + m.rv * v + 128) >> 8);
    let g = clamp8((yy + m.gu * u + m.gv * v + 128) >> 8);
    let b = clamp8((yy + m.bu * u + 128) >> 8);
    r | g << 8 | b << 16 | 0xff00_0000
}

/// Converts an NV12 picture to an opaque `ow` x `oh` RGBA image by nearest-neighbour
/// sampling. None when the picture is empty or its planes are too short for its size.
pub fn transfer_nv12(src: Nv12, ow: usize, oh: usize, m: Colorimetry) -> Option<Image> {
    let Nv12 { y, uv, stride, w, h } = src;
    if w == 0 || h == 0 || stride < w {
        return None;
    }
    // chroma pairs are read at the rounded-up width, so odd widths need that much stride
    let rw = (w + 1) & !1;
    if stride < rw || y.len() < stride * h || uv.len() < stride * ((h + 1) / 2) {
        return None;
    }
    let mut img = Image::new(ow, oh);
    for oy in 0..oh {
        let sy = oy * h / oh;
        let yr = &y[sy * stride..sy * stride + w];
        let ur = &uv[(sy / 2) * stride..(sy / 2) * stride + rw];
        for ox in 0..ow {
            let sx = ox * w / ow;
            img.px[oy * ow + ox] = yuv_to_rgba(yr[sx], ur[sx & !1], ur[(sx & !1) + 1], m);
        }
    }
    Some(img)
}

/// Scales an RGBA image to `ow` x `oh` by nearest-neighbour sampling. None for an empty source.
pub fn transfer_rgba(src: &Image, ow: usize, oh: usize) -> Option<Image> {
    if src.w == 0 || src.h == 0 {
        return None;
    }
    let mut img = Image::new(ow, oh);
    for oy in 0..oh {
        let sy = oy * src.h / oh;
        for ox in 0..ow {
            let sx = ox * src.w / ow;
            img.px[oy * ow + ox] = src.px[sy * src.w + sx];
        }
    }
    Some(img)
}

/// Swaps the red and blue channels in place: converts between RGBA and BGRA byte order.
/// Alpha and green stay where they are.
pub fn swap_red_blue(img: &mut Image) {
    for px in img.px.iter_mut() {
        let p = *px;
        *px = (p & 0xff00_ff00) | ((p & 0xff) << 16) | ((p >> 16) & 0xff);
    }
}

/// Mirrors an image left to right and/or top to bottom.
pub fn mirror(img: &Image, horizontal: bool, vertical: bool) -> Image {
    let (w, h) = (img.w, img.h);
    let mut out = Image::new(w, h);
    for y in 0..h {
        let sy = if vertical { h - 1 - y } else { y };
        for x in 0..w {
            let sx = if horizontal { w - 1 - x } else { x };
            out.px[y * w + x] = img.px[sy * w + sx];
        }
    }
    out
}

/// Rotates a picture in quarter turns, after the optional mirrors. The result is
/// `w` x `h` for no turn or a half turn, and `h` x `w` for a quarter turn.
pub fn rotate_picture(img: &Image, turn: Rotation, mirror_h: bool, mirror_v: bool) -> Image {
    let flipped;
    let src = if mirror_h || mirror_v {
        flipped = mirror(img, mirror_h, mirror_v);
        &flipped
    } else {
        img
    };
    match turn {
        Rotation::None => src.clone(),
        _ => rotated(src, turn),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img(w: usize, h: usize, px: &[u32]) -> Image {
        Image { w, h, px: px.to_vec() }
    }

    #[test]
    fn black_and_white_limited_range_map_to_the_ends() {
        let y = [16u8; 4];
        let uv = [128u8; 4];
        let src = Nv12 { y: &y, uv: &uv, stride: 2, w: 2, h: 2 };
        let black = transfer_nv12(src, 2, 2, BT709).unwrap();
        assert_eq!(black.px, vec![0xff00_0000; 4]);
        let y = [235u8; 4];
        let src = Nv12 { y: &y, uv: &uv, stride: 2, w: 2, h: 2 };
        let white = transfer_nv12(src, 2, 2, BT601).unwrap();
        assert_eq!(white.px, vec![0xffff_ffff; 4]);
    }

    #[test]
    fn neutral_chroma_gives_grey_at_luma_126() {
        // (298 * 110 + 128) >> 8 = 128 for every channel when Cb and Cr are 128
        let y = [126u8];
        let uv = [128u8, 128];
        let src = Nv12 { y: &y, uv: &uv, stride: 2, w: 1, h: 1 };
        let out = transfer_nv12(src, 1, 1, BT601).unwrap();
        assert_eq!(out.px, vec![0xff80_8080]);
    }

    #[test]
    fn nv12_rejects_short_planes() {
        let y = [16u8; 3];
        let uv = [128u8; 2];
        let src = Nv12 { y: &y, uv: &uv, stride: 2, w: 2, h: 2 };
        assert!(transfer_nv12(src, 2, 2, BT601).is_none());
    }

    #[test]
    fn rgba_scale_repeats_each_source_pixel_twice() {
        let src = img(2, 1, &[0xff00_00aa, 0xff00_00bb]);
        let out = transfer_rgba(&src, 4, 1).unwrap();
        assert_eq!(out.px, vec![0xff00_00aa, 0xff00_00aa, 0xff00_00bb, 0xff00_00bb]);
        assert!(transfer_rgba(&img(0, 0, &[]), 2, 2).is_none());
    }

    #[test]
    fn swap_moves_red_and_blue_and_keeps_green_and_alpha() {
        let mut i = img(1, 1, &[0xff10_2030]);
        swap_red_blue(&mut i);
        assert_eq!(i.px, vec![0xff30_2010]);
    }

    #[test]
    fn mirrors_flip_rows_and_columns() {
        let src = img(2, 2, &[1, 2, 3, 4]);
        assert_eq!(mirror(&src, true, false).px, vec![2, 1, 4, 3]);
        assert_eq!(mirror(&src, false, true).px, vec![3, 4, 1, 2]);
    }

    #[test]
    fn half_turn_reverses_and_clockwise_quarter_turn_moves_top_left_to_top_right() {
        let src = img(2, 2, &[1, 2, 3, 4]);
        assert_eq!(rotate_picture(&src, Rotation::Half, false, false).px, vec![4, 3, 2, 1]);
        // a 2x1 row [a b] turned clockwise is a 1x2 column with a on top
        let row = img(2, 1, &[7, 8]);
        let cw = rotate_picture(&row, Rotation::Cw, false, false);
        assert_eq!((cw.w, cw.h, cw.px), (1, 2, vec![7, 8]));
    }
}
