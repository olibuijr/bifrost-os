//! Pixel transfer and rotation sessions for app pixel buffers: convert a picture to RGBA at a fixed destination size,
//! and turn it by a quarter-turn angle with optional mirrors.
//!
//! A session is created for one destination size. NV12 pictures are converted with the colour matrix that matches
//! their height (BT.709 at 720 lines and up, BT.601 below) unless the session is given one. Every call returns `None`
//! when the input is empty or its planes are too short, and a rotation angle that is not a multiple of 90 degrees is
//! refused rather than rounded. Work runs on the CPU, with nearest-neighbour scaling.
//!
//! ```ignore
//! let session = PixelTransfer::new(640, 360);
//! let frame = session.nv12(Nv12 { y: &luma, uv: &chroma, stride: 1280, w: 1280, h: 720 });
//! let turned = session.rotated(&frame?, 90, false, false);   // 360 x 640
//! ```
//!
//! Not built: GPU transfer, 10-bit and HDR input, and a shared buffer type; the input and output are the renderer's
//! `Image`, premultiplied RGBA with R in the low byte.

use bifrost_render::gfx::Image;
use bifrost_render::video_transfer::{self, Colorimetry, Nv12};

/// A pixel transfer and rotation session for one destination size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PixelTransfer {
    width: usize,
    height: usize,
    colorimetry: Option<Colorimetry>,
}

impl PixelTransfer {
    /// A session whose conversions produce `width` x `height` pixels.
    pub fn new(width: usize, height: usize) -> PixelTransfer {
        PixelTransfer { width, height, colorimetry: None }
    }

    /// Uses one colour matrix for every NV12 picture, instead of choosing it by the picture's height.
    pub fn with_colorimetry(mut self, colorimetry: Colorimetry) -> PixelTransfer {
        self.colorimetry = Some(colorimetry);
        self
    }

    /// The destination size as `(width, height)`.
    pub fn destination(&self) -> (usize, usize) {
        (self.width, self.height)
    }

    /// Converts an NV12 picture to RGBA at the destination size.
    pub fn nv12(&self, src: Nv12<'_>) -> Option<Image> {
        let m = self.colorimetry.unwrap_or_else(|| Colorimetry::for_height(src.h));
        video_transfer::transfer_nv12(src, self.width, self.height, m)
    }

    /// Scales an RGBA picture to the destination size.
    pub fn rgba(&self, src: &Image) -> Option<Image> {
        video_transfer::transfer_rgba(src, self.width, self.height)
    }

    /// Mirrors a picture (horizontally and/or vertically) and turns it clockwise by `degrees`. The angle must be a
    /// multiple of 90; negative angles and angles past a full turn wrap. A quarter turn swaps the width and height.
    pub fn rotated(&self, img: &Image, degrees: i32, mirror_h: bool, mirror_v: bool) -> Option<Image> {
        let turn = video_transfer::turn_for_degrees(degrees)?;
        Some(video_transfer::rotate_picture(img, turn, mirror_h, mirror_v))
    }

    /// Turns a picture clockwise by any angle in degrees, about its centre. The output is the bounding box of the
    /// turned picture, and pixels that fall outside the source are transparent. Sampling is nearest-neighbour. Returns
    /// `None` for an empty picture or a non-finite angle.
    pub fn rotated_by(&self, img: &Image, degrees: f64) -> Option<Image> {
        if img.w == 0 || img.h == 0 || !degrees.is_finite() {
            return None;
        }
        let (s, c) = degrees.to_radians().sin_cos();
        let (w, h) = (img.w as f64, img.h as f64);
        // The epsilon keeps a quarter turn, where cos is 6e-17 rather than 0, from gaining a row or column.
        let out_w = ceil_eps(w * c.abs() + h * s.abs());
        let out_h = ceil_eps(w * s.abs() + h * c.abs());
        let (ocx, ocy) = (out_w as f64 / 2.0, out_h as f64 / 2.0);
        let (icx, icy) = (w / 2.0, h / 2.0);
        let mut px = vec![0u32; out_w * out_h];
        for y in 0..out_h {
            for x in 0..out_w {
                // Map the output pixel centre back into the source (inverse of a clockwise turn on a y-down screen).
                let dx = x as f64 + 0.5 - ocx;
                let dy = y as f64 + 0.5 - ocy;
                let sx = (dx * c + dy * s + icx).floor();
                let sy = (-dx * s + dy * c + icy).floor();
                if sx >= 0.0 && sy >= 0.0 && (sx as usize) < img.w && (sy as usize) < img.h {
                    px[y * out_w + x] = img.px[sy as usize * img.w + sx as usize];
                }
            }
        }
        Some(Image { w: out_w, h: out_h, px })
    }
}

fn ceil_eps(v: f64) -> usize {
    (v - 1e-9).ceil().max(0.0) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: usize, h: usize, px: u32) -> Image {
        Image { w, h, px: vec![px; w * h] }
    }

    #[test]
    fn rgba_is_scaled_to_the_destination_size() {
        let out = PixelTransfer::new(4, 3).rgba(&solid(2, 2, 0xff00_00ff)).expect("scaled");
        assert_eq!((out.w, out.h), (4, 3));
        assert!(out.px.iter().all(|p| *p == 0xff00_00ff));
    }

    #[test]
    fn an_empty_rgba_picture_is_refused() {
        assert!(PixelTransfer::new(4, 4).rgba(&solid(0, 0, 0)).is_none());
    }

    #[test]
    fn nv12_black_converts_to_opaque_black_at_the_destination_size() {
        let y = [16u8; 4];
        let uv = [128u8; 2];
        let frame = Nv12 { y: &y, uv: &uv, stride: 2, w: 2, h: 2 };
        let out = PixelTransfer::new(4, 4).nv12(frame).expect("converted");
        assert_eq!((out.w, out.h), (4, 4));
        assert!(out.px.iter().all(|p| *p == 0xff00_0000));
    }

    #[test]
    fn a_quarter_turn_swaps_width_and_height() {
        let session = PixelTransfer::new(1, 1);
        let turned = session.rotated(&solid(3, 2, 0xff00_0000), 90, false, false).expect("turned");
        assert_eq!((turned.w, turned.h), (2, 3));
        let half = session.rotated(&solid(3, 2, 0xff00_0000), 180, false, false).expect("turned");
        assert_eq!((half.w, half.h), (3, 2));
    }

    #[test]
    fn negative_and_wrapped_angles_are_the_same_turn() {
        let session = PixelTransfer::new(1, 1);
        let a = session.rotated(&solid(3, 2, 0xff00_0000), -90, false, false).expect("turned");
        let b = session.rotated(&solid(3, 2, 0xff00_0000), 270, false, false).expect("turned");
        assert_eq!((a.w, a.h), (b.w, b.h));
        assert_eq!((a.w, a.h), (2, 3));
    }

    #[test]
    fn an_angle_that_is_not_a_quarter_turn_is_refused() {
        assert!(PixelTransfer::new(1, 1).rotated(&solid(3, 2, 0), 45, false, false).is_none());
    }

    #[test]
    fn an_arbitrary_angle_grows_the_bounding_box() {
        let session = PixelTransfer::new(1, 1);
        let square = solid(2, 2, 0xff00_00ff);
        let turned = session.rotated_by(&square, 45.0).expect("turned");
        assert_eq!((turned.w, turned.h), (3, 3));
        // The centre stays inside the turned picture and the corner falls outside it.
        assert_eq!(turned.px[1 * 3 + 1], 0xff00_00ff);
        assert_eq!(turned.px[0], 0);
    }

    #[test]
    fn a_zero_angle_is_the_identity() {
        let session = PixelTransfer::new(1, 1);
        let src = Image { w: 3, h: 2, px: vec![1, 2, 3, 4, 5, 6] };
        let out = session.rotated_by(&src, 0.0).expect("turned");
        assert_eq!((out.w, out.h), (3, 2));
        assert_eq!(out.px, src.px);
    }

    #[test]
    fn a_quarter_turn_by_any_path_gives_the_same_size() {
        let session = PixelTransfer::new(1, 1);
        let src = solid(3, 2, 0xff00_0000);
        let by_angle = session.rotated_by(&src, 90.0).expect("turned");
        assert_eq!((by_angle.w, by_angle.h), (2, 3));
    }

    #[test]
    fn an_empty_picture_or_a_non_finite_angle_is_refused() {
        let session = PixelTransfer::new(1, 1);
        assert!(session.rotated_by(&solid(0, 0, 0), 10.0).is_none());
        assert!(session.rotated_by(&solid(2, 2, 0), f64::NAN).is_none());
    }

    #[test]
    fn destination_reports_the_size() {
        assert_eq!(PixelTransfer::new(640, 360).destination(), (640, 360));
    }
}
