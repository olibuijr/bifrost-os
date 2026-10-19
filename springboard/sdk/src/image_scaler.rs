//! Image scalers for apps: a spatial upscaler, a temporal upscaler with motion and depth input, and a frame
//! interpolator, all over app `Image`s (premultiplied RGBA, see `bifrost_render::gfx`). They are the app-facing
//! entry points for the CPU models in `bifrost_render::upscale`, `temporal_upscale` and `frame_interp`; the shell
//! does not call them. The spatial scaler filters colour weighted by alpha and resamples alpha on its own, so a soft
//! edge does not darken; the temporal scaler and the interpolator take opaque frames in the usual case.
//!
//! Colour handling: `SpatialScaler` filters in the mode it is given (`ColorProcessing::Perceptual` by default,
//! because an `Image` holds sRGB codes). The temporal scaler and the interpolator blend frames, so they decode to
//! linear light first and encode again at the end.
//!
//! There is no GPU pass: a GPU scaler would be checked against these. Metal command-buffer encoding has no
//! counterpart on this system. See docs/bifrost-ui.md "GPU kits" for the decisions and the gaps.
use bifrost_render::frame_interp::{interpolate, FrameError};
use bifrost_render::gfx::{rgba, Image};
use bifrost_render::pixel_layout::unpremultiply;
use bifrost_render::temporal_upscale::{Accumulation, TemporalScaler};
use bifrost_render::upscale::{upscale_f32, UpscaleError};

pub use bifrost_render::upscale::ColorProcessing;

/// The largest side a scaler accepts for its input or its output.
pub const MAX_SIDE: usize = 16384;

/// Why a scaler call was refused.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ScalerError {
    /// A width or height is zero.
    ZeroSize,
    /// A side is above `MAX_SIDE`.
    TooLarge { side: usize },
    /// The image is not the size the scaler was made for.
    SizeMismatch { expected: (usize, usize), got: (usize, usize) },
    /// An image or a side buffer (depth, motion) does not hold one entry per pixel.
    LengthMismatch,
    /// The interpolation time is outside 0..=1.
    TimeOutOfRange,
}

impl core::fmt::Display for ScalerError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ScalerError::ZeroSize => write!(f, "scaler: a dimension is zero"),
            ScalerError::TooLarge { side } => write!(f, "scaler: a side of {side} is above {MAX_SIDE}"),
            ScalerError::SizeMismatch { expected, got } => {
                write!(f, "scaler: expected a {}x{} image, got {}x{}", expected.0, expected.1, got.0, got.1)
            }
            ScalerError::LengthMismatch => write!(f, "scaler: a buffer does not hold one entry per pixel"),
            ScalerError::TimeOutOfRange => write!(f, "scaler: t must be within 0..=1"),
        }
    }
}

impl std::error::Error for ScalerError {}

impl From<UpscaleError> for ScalerError {
    fn from(e: UpscaleError) -> Self {
        match e {
            UpscaleError::ZeroSize => ScalerError::ZeroSize,
            // `HdrNeedsFloat` cannot happen on the f32 path; treat it like a bad buffer if it ever does.
            UpscaleError::LengthMismatch | UpscaleError::HdrNeedsFloat => ScalerError::LengthMismatch,
        }
    }
}

impl From<FrameError> for ScalerError {
    fn from(e: FrameError) -> Self {
        match e {
            FrameError::ZeroSize => ScalerError::ZeroSize,
            FrameError::LengthMismatch => ScalerError::LengthMismatch,
            FrameError::TimeOutOfRange => ScalerError::TimeOutOfRange,
        }
    }
}

fn check_sizes(in_w: usize, in_h: usize, out_w: usize, out_h: usize) -> Result<(), ScalerError> {
    if in_w == 0 || in_h == 0 || out_w == 0 || out_h == 0 {
        return Err(ScalerError::ZeroSize);
    }
    for side in [in_w, in_h, out_w, out_h] {
        if side > MAX_SIDE {
            return Err(ScalerError::TooLarge { side });
        }
    }
    Ok(())
}

fn check_image(img: &Image, expected: (usize, usize)) -> Result<(), ScalerError> {
    if (img.w, img.h) != expected {
        return Err(ScalerError::SizeMismatch { expected, got: (img.w, img.h) });
    }
    if img.px.len() != img.w.checked_mul(img.h).ok_or(ScalerError::LengthMismatch)? {
        return Err(ScalerError::LengthMismatch);
    }
    Ok(())
}

fn srgb_to_linear(v: f32) -> f32 {
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(v: f32) -> f32 {
    let v = v.clamp(0.0, 1.0);
    if v <= 0.003_130_8 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    }
}

/// Straight colour (0..1, as the image holds it) and alpha (0..1) of every pixel. `linear` decodes colour from sRGB.
fn split(img: &Image, linear: bool) -> (Vec<[f32; 3]>, Vec<f32>) {
    let mut color = Vec::with_capacity(img.px.len());
    let mut alpha = Vec::with_capacity(img.px.len());
    for &px in &img.px {
        let [r, g, b, a] = unpremultiply(px);
        let c = [r, g, b].map(|v| {
            let v = v as f32 / 255.0;
            if linear {
                srgb_to_linear(v)
            } else {
                v
            }
        });
        color.push(c);
        alpha.push(a as f32 / 255.0);
    }
    (color, alpha)
}

fn byte(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// The reverse of `split`: premultiplies colour by alpha and packs the pixels.
fn join(w: usize, h: usize, color: &[[f32; 3]], alpha: &[f32], linear: bool) -> Image {
    let px = color
        .iter()
        .zip(alpha)
        .map(|(c, &a)| {
            let enc = c.map(|v| if linear { linear_to_srgb(v) } else { v });
            rgba(byte(enc[0]), byte(enc[1]), byte(enc[2]), byte(a))
        })
        .collect();
    Image { w, h, px }
}

fn alpha_triples(alpha: &[f32]) -> Vec<[f32; 3]> {
    alpha.iter().map(|&a| [a, a, a]).collect()
}

fn alpha_of(triples: &[[f32; 3]]) -> Vec<f32> {
    triples.iter().map(|t| t[0]).collect()
}

/// A spatial upscaler: one `Image` in, one larger (or smaller) `Image` out, from a single frame.
#[derive(Clone, Debug, PartialEq)]
pub struct SpatialScaler {
    in_size: (usize, usize),
    out_size: (usize, usize),
    mode: ColorProcessing,
}

impl SpatialScaler {
    /// A scaler from `in_w` x `in_h` to `out_w` x `out_h`. A zero side or a side above `MAX_SIDE` is refused.
    /// The colour mode starts as `ColorProcessing::Perceptual`.
    pub fn new(in_w: usize, in_h: usize, out_w: usize, out_h: usize) -> Result<Self, ScalerError> {
        check_sizes(in_w, in_h, out_w, out_h)?;
        Ok(SpatialScaler { in_size: (in_w, in_h), out_size: (out_w, out_h), mode: ColorProcessing::Perceptual })
    }

    /// The size of the images `scale` takes.
    pub fn input_size(&self) -> (usize, usize) {
        self.in_size
    }

    /// The size of the images `scale` returns.
    pub fn output_size(&self) -> (usize, usize) {
        self.out_size
    }

    /// The colour processing mode.
    pub fn color_processing_mode(&self) -> ColorProcessing {
        self.mode
    }

    /// Sets the colour processing mode. Output is 8-bit, so `Hdr` overshoot is clamped when the pixels are packed.
    pub fn set_color_processing_mode(&mut self, mode: ColorProcessing) {
        self.mode = mode;
    }

    /// Scales `src`, which must be exactly the input size.
    pub fn scale(&self, src: &Image) -> Result<Image, ScalerError> {
        check_image(src, self.in_size)?;
        let (mut color, alpha) = split(src, false);
        let (w, h) = self.in_size;
        let (ow, oh) = self.out_size;
        // Colour is filtered weighted by alpha, so a clear neighbour (black in the buffer) does not darken an edge.
        for (c, &a) in color.iter_mut().zip(&alpha) {
            *c = c.map(|v| v * a);
        }
        let mut out_color = upscale_f32(&color, w, h, ow, oh, self.mode)?;
        let out_alpha = alpha_of(&upscale_f32(&alpha_triples(&alpha), w, h, ow, oh, ColorProcessing::Linear)?);
        for (c, &a) in out_color.iter_mut().zip(&out_alpha) {
            *c = if a > 1e-4 { c.map(|v| (v / a).min(1.0)) } else { [0.0; 3] };
        }
        Ok(join(ow, oh, &out_color, &out_alpha, false))
    }
}

/// A temporal upscaler: each frame is rendered small with the jitter from `next_jitter`, and `resolve` accumulates
/// the frames into a full-size history using depth and motion. Frames are taken as opaque; the output alpha is 255.
pub struct TemporalImageScaler {
    inner: TemporalScaler,
    in_size: (usize, usize),
    out_size: (usize, usize),
    denoised: bool,
}

impl TemporalImageScaler {
    /// A scaler from `in_w` x `in_h` frames to `out_w` x `out_h`. `denoised` keeps more history and softens the input
    /// colour first (a box filter, not a learned denoiser).
    pub fn new(in_w: usize, in_h: usize, out_w: usize, out_h: usize, denoised: bool) -> Result<Self, ScalerError> {
        check_sizes(in_w, in_h, out_w, out_h)?;
        let mode = if denoised { Accumulation::Denoised } else { Accumulation::Standard };
        let inner = TemporalScaler::new(in_w, in_h, out_w, out_h, mode)?;
        Ok(TemporalImageScaler { inner, in_size: (in_w, in_h), out_size: (out_w, out_h), denoised })
    }

    /// Whether the scaler is in the denoised mode.
    pub fn is_denoised(&self) -> bool {
        self.denoised
    }

    /// The size of the frames `resolve` takes.
    pub fn input_size(&self) -> (usize, usize) {
        self.in_size
    }

    /// The size of the images `resolve` returns.
    pub fn output_size(&self) -> (usize, usize) {
        self.out_size
    }

    /// The sub-pixel jitter to render the next frame with.
    pub fn next_jitter(&self) -> (f32, f32) {
        self.inner.next_jitter()
    }

    /// Drops the history, for a camera cut or a changed scene.
    pub fn reset(&mut self) {
        self.inner.reset();
    }

    /// Resolves one frame. `color` is the input size; `depth` (0..1) and `motion` (input-pixel offsets) hold one entry
    /// per input pixel.
    pub fn resolve(&mut self, color: &Image, depth: &[f32], motion: &[[f32; 2]]) -> Result<Image, ScalerError> {
        check_image(color, self.in_size)?;
        let (samples, _) = split(color, true);
        let out = self.inner.resolve(&samples, depth, motion)?;
        let (ow, oh) = self.out_size;
        let opaque = vec![1.0f32; out.len()];
        Ok(join(ow, oh, &out, &opaque, true))
    }
}

/// A frame interpolator: builds the frame between two rendered frames of one size.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameInterpolator {
    size: (usize, usize),
}

impl FrameInterpolator {
    /// An interpolator for `w` x `h` frames.
    pub fn new(w: usize, h: usize) -> Result<Self, ScalerError> {
        check_sizes(w, h, w, h)?;
        Ok(FrameInterpolator { size: (w, h) })
    }

    /// The frame size.
    pub fn size(&self) -> (usize, usize) {
        self.size
    }

    /// The frame at time `t` (0 gives `a`, 1 gives `b`). `motion` holds one pixel offset per pixel from `a` to `b`, or
    /// `None` for a cross-fade. Blending is in linear light.
    pub fn interpolate(&self, a: &Image, b: &Image, motion: Option<&[[f32; 2]]>, t: f32) -> Result<Image, ScalerError> {
        check_image(a, self.size)?;
        check_image(b, self.size)?;
        let (ca, aa) = split(a, true);
        let (cb, ab) = split(b, true);
        let (w, h) = self.size;
        let color = interpolate(&ca, &cb, motion, w, h, t)?;
        let alpha = alpha_of(&interpolate(&alpha_triples(&aa), &alpha_triples(&ab), motion, w, h, t)?);
        Ok(join(w, h, &color, &alpha, true))
    }

    /// `count` frames spread evenly between `a` and `b`, not counting `a` and `b` themselves: with `count` 1 it is the
    /// middle frame, with 3 it is the frames at 1/4, 2/4 and 3/4.
    pub fn intermediate_frames(&self, a: &Image, b: &Image, motion: Option<&[[f32; 2]]>, count: usize) -> Result<Vec<Image>, ScalerError> {
        (1..=count).map(|i| self.interpolate(a, b, motion, i as f32 / (count + 1) as f32)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: usize, h: usize, px: u32) -> Image {
        Image { w, h, px: vec![px; w * h] }
    }

    fn channel(px: u32, c: u32) -> i32 {
        ((px >> (c * 8)) & 255) as i32
    }

    #[test]
    fn a_flat_image_stays_flat_in_every_mode() {
        let src = solid(4, 4, rgba(200, 100, 50, 255));
        for mode in [ColorProcessing::Linear, ColorProcessing::Perceptual, ColorProcessing::Hdr] {
            let mut s = SpatialScaler::new(4, 4, 9, 7).unwrap();
            s.set_color_processing_mode(mode);
            let out = s.scale(&src).unwrap();
            assert_eq!((out.w, out.h), (9, 7));
            for &px in &out.px {
                for c in 0..4 {
                    assert!((channel(px, c) - channel(src.px[0], c)).abs() <= 1, "{mode:?} channel {c}");
                }
            }
        }
    }

    #[test]
    fn a_gradient_stays_monotone_when_enlarged() {
        let px = (0..8).map(|x| rgba(x * 30, x * 30, x * 30, 255)).collect();
        let src = Image { w: 8, h: 1, px };
        let mut s = SpatialScaler::new(8, 1, 32, 2).unwrap();
        s.set_color_processing_mode(ColorProcessing::Linear);
        let out = s.scale(&src).unwrap();
        let row: Vec<i32> = out.px[..32].iter().map(|&p| channel(p, 0)).collect();
        assert!(row.windows(2).all(|w| w[0] <= w[1]), "{row:?}");
        assert_eq!(row[0], 0);
        assert!(*row.last().unwrap() >= 205, "{row:?}");
    }

    #[test]
    fn transparent_pixels_stay_transparent_and_edges_do_not_darken() {
        let clear = solid(2, 2, 0);
        let out = SpatialScaler::new(2, 2, 4, 4).unwrap().scale(&clear).unwrap();
        assert!(out.px.iter().all(|&p| p == 0));
        // Opaque white beside clear: the straight colour of partly covered pixels is still white.
        let src = Image { w: 2, h: 1, px: vec![rgba(255, 255, 255, 255), 0] };
        let out = SpatialScaler::new(2, 1, 8, 1).unwrap().scale(&src).unwrap();
        for &px in &out.px {
            let [r, _, _, a] = unpremultiply(px);
            if a > 0 {
                assert!(r >= 250, "straight colour darkened to {r} at alpha {a}");
            }
        }
    }

    #[test]
    fn bad_requests_are_refused() {
        assert_eq!(SpatialScaler::new(0, 4, 8, 8), Err(ScalerError::ZeroSize));
        assert_eq!(SpatialScaler::new(4, 4, MAX_SIDE + 1, 8), Err(ScalerError::TooLarge { side: MAX_SIDE + 1 }));
        let s = SpatialScaler::new(4, 4, 8, 8).unwrap();
        assert_eq!(s.scale(&solid(3, 4, 0)), Err(ScalerError::SizeMismatch { expected: (4, 4), got: (3, 4) }));
        let broken = Image { w: 4, h: 4, px: vec![0; 5] };
        assert_eq!(s.scale(&broken), Err(ScalerError::LengthMismatch));
    }

    #[test]
    fn the_first_temporal_frame_is_the_spatial_upscale_of_its_colour() {
        let src = solid(4, 4, rgba(90, 90, 90, 255));
        let mut t = TemporalImageScaler::new(4, 4, 8, 8, false).unwrap();
        let out = t.resolve(&src, &[0.5; 16], &[[0.0, 0.0]; 16]).unwrap();
        assert_eq!((out.w, out.h), (8, 8));
        for &px in &out.px {
            assert!((channel(px, 0) - 90).abs() <= 1);
            assert_eq!(channel(px, 3), 255);
        }
    }

    #[test]
    fn a_static_scene_converges_and_a_reset_forgets_the_history() {
        let mut t = TemporalImageScaler::new(4, 4, 8, 8, true).unwrap();
        assert!(t.is_denoised());
        let dark = solid(4, 4, rgba(40, 40, 40, 255));
        let bright = solid(4, 4, rgba(200, 200, 200, 255));
        let depth = [0.5f32; 16];
        let motion = [[0.0f32, 0.0]; 16];
        t.resolve(&dark, &depth, &motion).unwrap();
        let blended = t.resolve(&bright, &depth, &motion).unwrap();
        // The history (95 percent) still pulls the output toward the dark frame, but the clamp to the
        // neighbourhood of the new frame lets at most the bright value through.
        assert!(channel(blended.px[20], 0) <= 200);
        t.reset();
        let fresh = t.resolve(&bright, &depth, &motion).unwrap();
        assert!(fresh.px.iter().all(|&p| (channel(p, 0) - 200).abs() <= 1));
    }

    #[test]
    fn temporal_buffers_must_match_the_input() {
        let mut t = TemporalImageScaler::new(4, 4, 8, 8, false).unwrap();
        let src = solid(4, 4, 0xff00_0000);
        assert_eq!(t.resolve(&src, &[0.5; 15], &[[0.0, 0.0]; 16]), Err(ScalerError::LengthMismatch));
        assert_eq!(t.resolve(&solid(2, 2, 0), &[0.5; 4], &[[0.0, 0.0]; 4]), Err(ScalerError::SizeMismatch { expected: (4, 4), got: (2, 2) }));
        assert_eq!(TemporalImageScaler::new(4, 4, 0, 8, false).err(), Some(ScalerError::ZeroSize));
    }

    #[test]
    fn interpolation_endpoints_are_exact_and_the_middle_blends_in_linear_light() {
        let a = solid(2, 2, rgba(0, 0, 0, 255));
        let b = solid(2, 2, rgba(255, 255, 255, 255));
        let f = FrameInterpolator::new(2, 2).unwrap();
        assert_eq!(f.interpolate(&a, &b, None, 0.0).unwrap(), a);
        assert_eq!(f.interpolate(&a, &b, None, 1.0).unwrap(), b);
        let mid = f.interpolate(&a, &b, None, 0.5).unwrap();
        let v = channel(mid.px[0], 0);
        assert!((186..=189).contains(&v), "half of white in linear light is about 188, got {v}");
    }

    #[test]
    fn interpolation_refuses_bad_time_and_sizes() {
        let a = solid(2, 2, 0xff00_0000);
        let f = FrameInterpolator::new(2, 2).unwrap();
        assert_eq!(f.interpolate(&a, &a, None, 1.5), Err(ScalerError::TimeOutOfRange));
        assert_eq!(f.interpolate(&a, &a, None, f32::NAN), Err(ScalerError::TimeOutOfRange));
        assert_eq!(f.interpolate(&a, &solid(3, 2, 0), None, 0.5), Err(ScalerError::SizeMismatch { expected: (2, 2), got: (3, 2) }));
        assert_eq!(f.interpolate(&a, &a, Some(&[[0.0, 0.0]; 3]), 0.5), Err(ScalerError::LengthMismatch));
        assert_eq!(FrameInterpolator::new(0, 2), Err(ScalerError::ZeroSize));
    }

    #[test]
    fn intermediate_frames_are_spread_evenly() {
        let a = solid(1, 1, rgba(0, 0, 0, 255));
        let b = solid(1, 1, rgba(255, 255, 255, 255));
        let f = FrameInterpolator::new(1, 1).unwrap();
        let frames = f.intermediate_frames(&a, &b, None, 3).unwrap();
        assert_eq!(frames.len(), 3);
        let v: Vec<i32> = frames.iter().map(|i| channel(i.px[0], 0)).collect();
        assert!(v[0] < v[1] && v[1] < v[2], "{v:?}");
        assert!(f.intermediate_frames(&a, &b, None, 0).unwrap().is_empty());
    }

    #[test]
    fn motion_moves_the_content_between_frames() {
        // A white pixel at x=0 in frame a, at x=2 in frame b, motion +2 pixels. At t = 0.5 the content lands at x=1.
        let mut a = solid(3, 1, rgba(0, 0, 0, 255));
        let mut b = a.clone();
        a.px[0] = rgba(255, 255, 255, 255);
        b.px[2] = rgba(255, 255, 255, 255);
        let motion = [[2.0f32, 0.0]; 3];
        let f = FrameInterpolator::new(3, 1).unwrap();
        let out = f.interpolate(&a, &b, Some(&motion), 0.5).unwrap();
        let v: Vec<i32> = out.px.iter().map(|&p| channel(p, 0)).collect();
        assert!(v[1] > v[0] && v[1] > v[2], "the pixel should sit in the middle: {v:?}");
    }
}
