//! Render destinations, accumulators, perspective correction and per-frame video filtering for apps
//! (`CIRenderDestination`, `CIImageAccumulator`, `CIPerspectiveCorrection`, and running a `CIFilter`
//! chain on each frame of a video). The pixel work lives in `bifrost_render::render_destination` and
//! `bifrost_render::perspective`; this module is the app-facing shape.
//!
//! A video app takes a decoded frame from its media player (`take_frame`) and passes it to
//! `VideoFilter::process`, which runs the filter chain and then the optional feedback accumulator.
//! The result is an `Image` the app can draw or render into a `Destination`. Not yet wired into
//! Photo Booth or Photos video; see docs/bifrost-ui.md, "Core Image filters".
use crate::image_filter::FilterChain;
use bifrost_render::gfx::Image;

pub use bifrost_render::perspective::{homography, map_point, Homography, Point};
pub use bifrost_render::render_destination::{
    render_into, Accumulator, ColorSpace, Destination, PixelFormat, Region,
};

/// Corrects a perspective-distorted quadrilateral (`quad`: top-left, top-right, bottom-right,
/// bottom-left in the image) into an upright `out_w` by `out_h` image. `None` for a degenerate quad.
pub fn perspective_correct(img: &Image, quad: [Point; 4], out_w: usize, out_h: usize) -> Option<Image> {
    bifrost_render::perspective::perspective_correct(img, quad, out_w, out_h)
}

/// Renders `img` into a fresh destination of the given size and format, with the whole image at the
/// origin. `None` for an empty size or a mismatched image.
pub fn render_to_destination(img: &Image, format: PixelFormat) -> Option<Destination> {
    let mut dest = Destination::new(img.w, img.h, format)?;
    render_into(img, Region { x: 0, y: 0, w: img.w, h: img.h }, &mut dest, (0, 0))?;
    Some(dest)
}

/// Runs a filter chain over each video frame, then feeds the result to an optional accumulator for
/// feedback (trails, painting). The accumulator restarts when the frame size changes.
#[derive(Debug)]
pub struct VideoFilter {
    chain: FilterChain,
    retain: Option<f32>,
    accumulator: Option<Accumulator>,
}

impl VideoFilter {
    /// A video filter that applies `chain` to each frame and nothing else.
    pub fn new(chain: FilterChain) -> Self {
        VideoFilter { chain, retain: None, accumulator: None }
    }

    /// Adds feedback: each output is composited over the retained previous output with this factor
    /// (0 shows only the current frame, 1 never fades). Clamped to 0..1.
    pub fn with_feedback(mut self, retain: f32) -> Self {
        self.retain = Some(if retain.is_nan() { 0.0 } else { retain.clamp(0.0, 1.0) });
        self
    }

    /// Filters one decoded frame. `None` when the frame's pixel count does not match its size.
    pub fn process(&mut self, frame: &Image) -> Option<Image> {
        let filtered = if self.chain.is_empty() { frame.clone() } else { self.chain.apply(frame)? };
        let Some(retain) = self.retain else {
            return Some(filtered);
        };
        let fits = self
            .accumulator
            .as_ref()
            .is_some_and(|a| a.width() == filtered.w && a.height() == filtered.h);
        if !fits {
            self.accumulator = Some(Accumulator::new(filtered.w, filtered.h)?);
        }
        let acc = self.accumulator.as_mut()?;
        acc.feed(&filtered, retain)?;
        Some(acc.snapshot())
    }

    /// Drops the feedback state, so the next frame starts a new trail.
    pub fn reset(&mut self) {
        if let Some(acc) = self.accumulator.as_mut() {
            acc.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opaque(w: usize, h: usize, px: u32) -> Image {
        Image { w, h, px: vec![px; w * h] }
    }

    #[test]
    fn empty_chain_passes_frames_through() {
        let mut vf = VideoFilter::new(FilterChain::new());
        let f = opaque(2, 2, 0xff00_00ff);
        assert_eq!(vf.process(&f), Some(f));
    }

    #[test]
    fn feedback_with_zero_retain_matches_the_frame() {
        let mut vf = VideoFilter::new(FilterChain::new()).with_feedback(0.0);
        let a = opaque(2, 2, 0xff00_00ff);
        let b = opaque(2, 2, 0xff00_ff00);
        vf.process(&a).unwrap();
        assert_eq!(vf.process(&b).unwrap(), b);
    }

    #[test]
    fn feedback_restarts_on_size_change() {
        let mut vf = VideoFilter::new(FilterChain::new()).with_feedback(1.0);
        vf.process(&opaque(2, 2, 0xff00_00ff)).unwrap();
        let out = vf.process(&opaque(3, 1, 0xff00_ff00)).unwrap();
        assert_eq!((out.w, out.h), (3, 1));
        assert_eq!(out.px, vec![0xff00_ff00; 3]);
    }

    #[test]
    fn process_rejects_mismatched_frame() {
        let mut vf = VideoFilter::new(FilterChain::new());
        assert!(vf.process(&Image { w: 2, h: 2, px: vec![0; 3] }).is_none());
    }

    #[test]
    fn render_to_destination_copies_whole_image() {
        let img = opaque(2, 1, 0xff00_00ff);
        let d = render_to_destination(&img, PixelFormat::Rgba8Premultiplied).unwrap();
        assert_eq!(d.bytes, vec![255, 0, 0, 255, 255, 0, 0, 255]);
    }
}
