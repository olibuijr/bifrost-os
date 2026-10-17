//! Pixel conversion for the video thread: NV12 (or RGBA) pictures scaled
//! to fit a viewport, as premultiplied RGBA `Image`s the UI blits.
//!
//! Everything is integer arithmetic on rows, in a shape the compiler
//! auto-vectorises for NEON: per-row precomputed source indices, 8-bit
//! interpolation weights and fixed-point BT.601/BT.709 matrices.

use crate::gfx::Image;

/// Target size for a `w x h` picture inside `vw x vh`, keeping its aspect
/// ratio (letterboxing is the caller's: the image is exactly this size).
pub fn fit(w: u32, h: u32, vw: u32, vh: u32) -> (usize, usize) {
    if w == 0 || h == 0 {
        return (0, 0);
    }
    let by_width = (vw as u64 * h as u64 / w as u64) as u32;
    let (ow, oh) = if by_width <= vh { (vw, by_width) } else { ((vh as u64 * w as u64 / h as u64) as u32, vh) };
    (ow.max(1) as usize & !1, oh.max(1) as usize)
}

/// Source positions for an output axis of `out` samples over `src`
/// samples: (index, 8-bit weight towards index + 1), pixel centres aligned.
fn axis(src: usize, out: usize) -> Vec<(u32, u16)> {
    (0..out)
        .map(|o| {
            // fixed point 16.16 of (o + 0.5) * src / out - 0.5
            let pos = (((o as i64 * 2 + 1) * src as i64) << 15) / out as i64 - (1 << 15);
            let pos = pos.max(0).min(((src as i64 - 1) << 16).max(0));
            ((pos >> 16) as u32, ((pos & 0xffff) >> 8) as u16)
        })
        .collect()
}

/// Colour matrix in 8-bit fixed point for limited-range YCbCr.
#[derive(Clone, Copy)]
pub struct Matrix {
    /// 255/219 luma gain
    pub y: i32,
    pub rv: i32,
    pub gu: i32,
    pub gv: i32,
    pub bu: i32,
}

pub const BT601: Matrix = Matrix { y: 298, rv: 409, gu: -100, gv: -208, bu: 516 };
pub const BT709: Matrix = Matrix { y: 298, rv: 459, gu: -55, gv: -136, bu: 541 };

/// HD material is BT.709 unless flagged otherwise; SD is BT.601.
pub fn matrix_for(height: u32) -> Matrix {
    if height >= 720 {
        BT709
    } else {
        BT601
    }
}

#[inline]
fn clamp8(v: i32) -> u32 {
    v.clamp(0, 255) as u32
}

/// One output row from a luma row pair and the matching chroma row.
#[inline]
fn nv12_row(out: &mut [u32], y0: &[u8], y1: &[u8], wy: u32, uv: &[u8], xs: &[(u32, u16)], m: Matrix) {
    for (o, &(x, wx)) in out.iter_mut().zip(xs) {
        let x = x as usize;
        let (x1, wx) = (x + 1, wx as u32);
        let lerp = |row: &[u8]| -> u32 {
            let a = row[x] as u32;
            let b = row.get(x1).copied().unwrap_or(row[x]) as u32;
            (a * (256 - wx) + b * wx) >> 8
        };
        let y = (lerp(y0) * (256 - wy) + lerp(y1) * wy) >> 8;
        let c = x & !1;
        let (u, v) = (uv[c] as i32 - 128, uv[c + 1] as i32 - 128);
        let yy = m.y * (y as i32 - 16) + 128;
        let r = clamp8((yy + m.rv * v) >> 8);
        let g = clamp8((yy + m.gu * u + m.gv * v) >> 8);
        let b = clamp8((yy + m.bu * u) >> 8);
        *o = r | g << 8 | b << 16 | 0xff00_0000;
    }
}

/// A borrowed NV12 picture: `stride` bytes per row for both planes.
#[derive(Clone, Copy)]
pub struct Nv12<'a> {
    pub y: &'a [u8],
    pub uv: &'a [u8],
    pub stride: usize,
    pub w: usize,
    pub h: usize,
}

/// Converts and scales an NV12 picture to `ow x oh` RGBA, splitting rows
/// over `threads` threads.
pub fn nv12_to_rgba(src: Nv12, ow: usize, oh: usize, m: Matrix, threads: usize) -> Image {
    let Nv12 { y, uv, stride, w, h } = src;
    let mut img = Image::new(ow, oh);
    // chroma pairs are read at the rounded-up width, so odd widths need that much stride
    let rw = (w + 1) & !1;
    if w == 0 || h < 2 || ow == 0 || oh == 0 || stride < rw || y.len() < stride * (h - 1) + w || uv.len() < stride * (h / 2 - 1) + rw {
        return img;
    }
    let xs = axis(w, ow);
    let ys = axis(h, oh);
    let uvw = rw;
    let band = oh.div_ceil(threads.max(1));
    std::thread::scope(|s| {
        for (bi, rows) in img.px.chunks_mut(band * ow).enumerate() {
            let (xs, ys) = (&xs, &ys);
            s.spawn(move || {
                crate::gfx::affinity::pin_big();
                for (r, out) in rows.chunks_mut(ow).enumerate() {
                    let (sy, wy) = ys[bi * band + r];
                    let sy = sy as usize;
                    let y0 = &y[sy * stride..sy * stride + w];
                    let y1 = if sy + 1 < h { &y[(sy + 1) * stride..(sy + 1) * stride + w] } else { y0 };
                    let cy = (sy / 2).min(h / 2 - 1);
                    let uvr = &uv[cy * stride..cy * stride + uvw];
                    nv12_row(out, y0, y1, wy as u32, uvr, xs, m);
                }
            });
        }
    });
    img
}

/// A zero-copy decoder picture converted and scaled like `nv12_to_rgba` (an empty image of
/// the requested size when the picture's description does not fit its buffer).
pub fn dma_to_rgba(p: &crate::media::DmaPicture, ow: usize, oh: usize, m: Matrix, threads: usize) -> Image {
    let (w, h) = (p.width as usize, p.height as usize);
    p.with_nv12(|y, uv, stride| nv12_to_rgba(Nv12 { y, uv, stride, w, h }, ow, oh, m, threads)).unwrap_or_else(|| Image::new(ow, oh))
}

/// Scales an RGBA picture to `ow x oh` with bilinear filtering.
pub fn rgba_scale(src: &Image, ow: usize, oh: usize) -> Image {
    if (src.w, src.h) == (ow, oh) {
        return src.clone();
    }
    let mut img = Image::new(ow, oh);
    if src.w == 0 || src.h == 0 {
        return img;
    }
    let xs = axis(src.w, ow);
    let ys = axis(src.h, oh);
    for (r, out) in img.px.chunks_mut(ow).enumerate() {
        let (sy, wy) = ys[r];
        let (sy, wy) = (sy as usize, wy as u32 * 255 / 256);
        let r0 = &src.px[sy * src.w..sy * src.w + src.w];
        let r1 = if sy + 1 < src.h { &src.px[(sy + 1) * src.w..(sy + 2) * src.w] } else { r0 };
        for (o, &(x, wx)) in out.iter_mut().zip(&xs) {
            let x = x as usize;
            let wx = wx as u32 * 255 / 256;
            let x1 = (x + 1).min(src.w - 1);
            let top = crate::gfx::scale(r0[x], 255 - wx) + crate::gfx::scale(r0[x1], wx);
            let bot = crate::gfx::scale(r1[x], 255 - wx) + crate::gfx::scale(r1[x1], wx);
            let mix = crate::gfx::scale(top, 255 - wy) + crate::gfx::scale(bot, wy);
            *o = mix | 0xff00_0000;
        }
    }
    img
}

/// Per-channel gains for white balance (1.0 leaves a channel as it is).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gains {
    pub r: f32,
    pub g: f32,
    pub b: f32,
}

/// The gains at which the average colour of a frame's opaque pixels is grey (the gray-world assumption: a scene
/// averages to neutral). Each gain is limited to 1/4 to 4, so a frame of one colour is not pushed to a wild cast.
/// Transparent pixels are left out of the average. `None` when there is no opaque pixel or the frame is black.
pub fn gray_world_gains(img: &Image) -> Option<Gains> {
    let (mut sum, mut n) = ([0u64; 3], 0u64);
    for &p in &img.px {
        if p >> 24 == 255 {
            sum[0] += (p & 255) as u64;
            sum[1] += ((p >> 8) & 255) as u64;
            sum[2] += ((p >> 16) & 255) as u64;
            n += 1;
        }
    }
    if n == 0 || sum.iter().all(|&s| s == 0) {
        return None;
    }
    let avg = sum.map(|s| s as f32 / n as f32);
    let grey = (avg[0] + avg[1] + avg[2]) / 3.0;
    let gain = |a: f32| if a < 1.0 { 1.0 } else { (grey / a).clamp(0.25, 4.0) };
    Some(Gains { r: gain(avg[0]), g: gain(avg[1]), b: gain(avg[2]) })
}

/// Multiplies the colour of every opaque pixel by `gains`, clamped to 255. Transparent pixels are not touched.
pub fn apply_gains(img: &mut Image, gains: Gains) {
    let scale = |c: u32, k: f32| (c as f32 * k).round().clamp(0.0, 255.0) as u32;
    for p in img.px.iter_mut() {
        if *p >> 24 == 255 {
            let (r, g, b) = (*p & 255, (*p >> 8) & 255, (*p >> 16) & 255);
            *p = scale(r, gains.r) | scale(g, gains.g) << 8 | scale(b, gains.b) << 16 | 255 << 24;
        }
    }
}

/// Gray-world white balance of a frame in place. Returns the gains used, or `None` when the frame was left as it was.
pub fn white_balance(img: &mut Image) -> Option<Gains> {
    let gains = gray_world_gains(img)?;
    apply_gains(img, gains);
    Some(gains)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(px: Vec<u32>) -> Image {
        let n = px.len();
        Image { w: n, h: 1, px }
    }

    fn opaque(r: u32, g: u32, b: u32) -> u32 {
        r | g << 8 | b << 16 | 255 << 24
    }

    #[test]
    fn a_yellow_cast_is_taken_out_of_a_frame() {
        let mut img = frame(vec![opaque(200, 200, 100); 16]);
        let gains = white_balance(&mut img).expect("a cast can be corrected");
        assert!(gains.b > gains.r, "blue is boosted and red reduced");
        let p = img.px[0];
        let (r, g, b) = (p & 255, (p >> 8) & 255, (p >> 16) & 255);
        assert!(r.abs_diff(g) <= 1 && g.abs_diff(b) <= 2, "the average becomes grey: {r} {g} {b}");
    }

    #[test]
    fn a_neutral_frame_gets_unit_gains() {
        let mut img = frame(vec![opaque(90, 90, 90); 4]);
        let gains = white_balance(&mut img).unwrap();
        assert!((gains.r - 1.0).abs() < 1e-6 && (gains.g - 1.0).abs() < 1e-6 && (gains.b - 1.0).abs() < 1e-6);
        assert_eq!(img.px[0], opaque(90, 90, 90));
    }

    #[test]
    fn transparent_pixels_are_left_out_of_the_average_and_untouched() {
        let clear = 0x12;
        let mut img = frame(vec![opaque(200, 200, 100), clear, opaque(200, 200, 100)]);
        white_balance(&mut img).unwrap();
        assert_eq!(img.px[1], clear);
    }

    #[test]
    fn a_black_or_transparent_frame_is_left_alone() {
        let mut black = frame(vec![opaque(0, 0, 0); 4]);
        assert_eq!(white_balance(&mut black), None);
        let mut clear = frame(vec![0; 4]);
        assert_eq!(white_balance(&mut clear), None);
    }

    #[test]
    fn gains_are_limited_so_a_single_colour_is_not_pushed_out() {
        let g = gray_world_gains(&frame(vec![opaque(250, 2, 2)])).unwrap();
        assert!(g.g <= 4.0 && g.b <= 4.0 && g.r >= 0.25, "{g:?}");
    }


    #[test]
    fn fit_keeps_aspect_inside_the_viewport() {
        assert_eq!(fit(1280, 720, 1200, 1920), (1200, 675));
        assert_eq!(fit(1920, 1080, 1200, 1920), (1200, 675));
        assert_eq!(fit(720, 1280, 1200, 1920), (1080, 1920));
        assert_eq!(fit(640, 480, 1200, 1920), (1200, 900));
        assert_eq!(fit(0, 480, 1200, 1920), (0, 0));
    }

    #[test]
    fn axis_maps_centres_and_clamps() {
        let a = axis(4, 4);
        assert!(a.iter().enumerate().all(|(i, &(x, w))| x as usize == i && w == 0), "{a:?}");
        let up = axis(2, 4);
        assert_eq!(up[0], (0, 0));
        assert_eq!(up[3], (1, 0));
        assert!(up[1].1 > 0 && up[2].1 > 0);
        let down = axis(8, 2);
        assert_eq!(down[0].0, 1);
        assert_eq!(down[1].0, 5);
    }

    fn solid_nv12(w: usize, h: usize, y: u8, u: u8, v: u8) -> (Vec<u8>, Vec<u8>) {
        let uv: Vec<u8> = (0..w * h / 2).map(|i| if i % 2 == 0 { u } else { v }).collect();
        (vec![y; w * h], uv)
    }

    fn pic<'a>(y: &'a [u8], uv: &'a [u8], stride: usize, w: usize, h: usize) -> Nv12<'a> {
        Nv12 { y, uv, stride, w, h }
    }

    #[test]
    fn nv12_grey_white_and_red_convert_correctly() {
        let (y, uv) = solid_nv12(16, 8, 235, 128, 128);
        let img = nv12_to_rgba(pic(&y, &uv, 16, 16, 8), 16, 8, BT601, 2);
        assert!(img.px.iter().all(|&p| p == 0xffff_ffff), "white: {:08x}", img.px[0]);
        let (y, uv) = solid_nv12(16, 8, 16, 128, 128);
        let img = nv12_to_rgba(pic(&y, &uv, 16, 16, 8), 8, 4, BT709, 3);
        assert_eq!((img.w, img.h), (8, 4));
        assert!(img.px.iter().all(|&p| p == 0xff00_0000), "black: {:08x}", img.px[0]);
        // pure red in BT.601 limited range: Y=81 Cb=90 Cr=240
        let (y, uv) = solid_nv12(16, 8, 81, 90, 240);
        let p = nv12_to_rgba(pic(&y, &uv, 16, 16, 8), 16, 8, BT601, 1).px[0];
        let (r, g, b) = (p & 255, p >> 8 & 255, p >> 16 & 255);
        assert!(r >= 250 && g < 8 && b < 8, "red: {r} {g} {b}");
    }

    #[test]
    fn nv12_scaling_blends_a_vertical_edge() {
        let w = 16;
        let mut y = vec![16u8; w * 8];
        for row in y.chunks_mut(w) {
            row[8..].fill(235);
        }
        let uv = vec![128u8; w * 4];
        let img = nv12_to_rgba(pic(&y, &uv, w, w, 8), 8, 4, BT601, 4);
        let lum = |x: usize| img.px[x] & 255;
        assert!(lum(0) < 5 && lum(7) > 250);
        assert!(lum(3) < lum(4), "monotonic across the edge");
        assert!(img.px.iter().all(|&p| p >> 24 == 255));
    }

    #[test]
    fn nv12_honours_stride_and_odd_thread_counts() {
        let (w, h, stride) = (6, 4, 8);
        let mut y = vec![0u8; stride * h];
        for r in 0..h {
            y[r * stride..r * stride + w].fill(235);
        }
        let uv = vec![128u8; stride * h / 2];
        let img = nv12_to_rgba(pic(&y, &uv, stride, w, h), 6, 4, BT601, 3);
        assert!(img.px.iter().all(|&p| p == 0xffff_ffff));
        let img = nv12_to_rgba(pic(&y, &uv, stride, w, h), 6, 4, BT601, 7);
        assert!(img.px.iter().all(|&p| p == 0xffff_ffff), "more threads than rows");
        let short = nv12_to_rgba(pic(&y[..10], &uv, stride, w, h), 6, 4, BT601, 1);
        assert!(short.px.iter().all(|&p| p == 0), "truncated planes give a blank frame, not a panic");
    }

    #[test]
    fn rgba_scale_keeps_flat_colour_and_size() {
        let src = Image { w: 4, h: 4, px: vec![0xff40_80c0; 16] };
        let out = rgba_scale(&src, 10, 6);
        assert_eq!((out.w, out.h), (10, 6));
        assert!(out.px.iter().all(|&p| p == 0xff40_80c0), "{:08x}", out.px[0]);
        assert_eq!(rgba_scale(&src, 4, 4), src);
    }
}
