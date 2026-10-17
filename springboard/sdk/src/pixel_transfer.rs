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
    fn destination_reports_the_size() {
        assert_eq!(PixelTransfer::new(640, 360).destination(), (640, 360));
    }
}
