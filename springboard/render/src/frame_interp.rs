//! Frame interpolator model for CPU rasters: builds the frame between two rendered frames at time `t` in 0..=1.
//!
//! Without motion the result is the linear blend `(1 - t) * a + t * b`. With a per-pixel motion field (input-pixel
//! offsets from `a` to `b`) the result is a backward warp: output pixel `p` takes `a` sampled at `p - t * m` and `b`
//! sampled at `p + (1 - t) * m`, and the two samples are blended with weights `1 - t` and `t`. At `t = 0` the output
//! is `a` and at `t = 1` it is `b`, sampled at the same positions, so the endpoints are exact.
//!
//! Samples are bilinear and clamp at the raster edges. The model does not detect occlusions, so a pixel that is
//! covered in one frame and revealed in the other is smeared rather than filled. It runs on the CPU and has no GPU
//! pass, no depth input and no UI or cursor layer.

/// Why an interpolation request was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameError {
    /// The width or the height is zero.
    ZeroSize,
    /// A frame or the motion field does not hold one entry per pixel.
    LengthMismatch,
    /// `t` is outside 0..=1 (or is not a number).
    TimeOutOfRange,
}

impl core::fmt::Display for FrameError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            FrameError::ZeroSize => write!(f, "frame interpolation: a dimension is zero"),
            FrameError::LengthMismatch => write!(f, "frame interpolation: a buffer does not match the frame size"),
            FrameError::TimeOutOfRange => write!(f, "frame interpolation: t must be within 0..=1"),
        }
    }
}

/// Bilinear sample of a `w` x `h` raster at (`fx`, `fy`) in pixel units, clamped to the raster edges.
fn sample(buf: &[[f32; 3]], w: usize, h: usize, fx: f32, fy: f32) -> [f32; 3] {
    let fx = fx.clamp(0.0, (w - 1) as f32);
    let fy = fy.clamp(0.0, (h - 1) as f32);
    let x0 = fx.floor() as usize;
    let y0 = fy.floor() as usize;
    let x1 = (x0 + 1).min(w - 1);
    let y1 = (y0 + 1).min(h - 1);
    let tx = fx - x0 as f32;
    let ty = fy - y0 as f32;
    let top = |c: usize| buf[y0 * w + x0][c] * (1.0 - tx) + buf[y0 * w + x1][c] * tx;
    let bottom = |c: usize| buf[y1 * w + x0][c] * (1.0 - tx) + buf[y1 * w + x1][c] * tx;
    [0, 1, 2].map(|c| top(c) * (1.0 - ty) + bottom(c) * ty)
}

/// The frame at time `t` between `a` (t = 0) and `b` (t = 1). `motion` holds one input-pixel offset per pixel from
/// `a` to `b`, or `None` for a plain cross-fade. Returns `w * h` samples.
pub fn interpolate(
    a: &[[f32; 3]],
    b: &[[f32; 3]],
    motion: Option<&[[f32; 2]]>,
    w: usize,
    h: usize,
    t: f32,
) -> Result<Vec<[f32; 3]>, FrameError> {
    if w == 0 || h == 0 {
        return Err(FrameError::ZeroSize);
    }
    let n = w * h;
    if a.len() != n || b.len() != n || motion.is_some_and(|m| m.len() != n) {
        return Err(FrameError::LengthMismatch);
    }
    if !(0.0..=1.0).contains(&t) {
        return Err(FrameError::TimeOutOfRange);
    }
    let mut out = Vec::with_capacity(n);
    for y in 0..h {
        for x in 0..w {
            let p = match motion {
                None => {
                    let (pa, pb) = (a[y * w + x], b[y * w + x]);
                    [0, 1, 2].map(|c| pa[c] * (1.0 - t) + pb[c] * t)
                }
                Some(m) => {
                    let [mx, my] = m[y * w + x];
                    let pa = sample(a, w, h, x as f32 - t * mx, y as f32 - t * my);
                    let pb = sample(b, w, h, x as f32 + (1.0 - t) * mx, y as f32 + (1.0 - t) * my);
                    [0, 1, 2].map(|c| pa[c] * (1.0 - t) + pb[c] * t)
                }
            };
            out.push(p);
        }
    }
    Ok(out)
}

/// The stream side of the interpolator: it holds the last two frames a presenter rendered, so each new frame gives
/// the pair that the frames in between are built from. Scene cuts are the caller's to report with [`reset`].
///
/// [`reset`]: FrameInterpolator::reset
#[derive(Debug)]
pub struct FrameInterpolator {
    width: usize,
    height: usize,
    previous: Option<Vec<[f32; 3]>>,
    current: Option<Vec<[f32; 3]>>,
}

impl FrameInterpolator {
    /// An interpolator for frames of `width` by `height` pixels. A zero dimension is refused.
    pub fn new(width: usize, height: usize) -> Result<Self, FrameError> {
        if width == 0 || height == 0 {
            return Err(FrameError::ZeroSize);
        }
        Ok(FrameInterpolator { width, height, previous: None, current: None })
    }

    /// Takes the next rendered frame. The frame that was current becomes the previous one.
    pub fn push(&mut self, frame: Vec<[f32; 3]>) -> Result<(), FrameError> {
        if frame.len() != self.width * self.height {
            return Err(FrameError::LengthMismatch);
        }
        self.previous = self.current.replace(frame);
        Ok(())
    }

    /// The frame at time `t` between the previous frame (t = 0) and the current one (t = 1). `None` until two frames
    /// have been pushed since the last reset.
    pub fn between(&self, t: f32, motion: Option<&[[f32; 2]]>) -> Result<Option<Vec<[f32; 3]>>, FrameError> {
        match (&self.previous, &self.current) {
            (Some(a), Some(b)) => interpolate(a, b, motion, self.width, self.height, t).map(Some),
            _ => Ok(None),
        }
    }

    /// Forgets both held frames, as at a scene cut: the next frame starts a new pair.
    pub fn reset(&mut self) {
        self.previous = None;
        self.current = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn plain_cross_fade_blends_linearly() {
        // Black to white at t = 0.25 is 0.25 everywhere.
        let a = vec![[0.0; 3]; 4];
        let b = vec![[1.0; 3]; 4];
        let out = interpolate(&a, &b, None, 4, 1, 0.25).unwrap();
        assert!(out.iter().all(|p| near(p[0], 0.25)));
        assert_eq!(interpolate(&a, &b, None, 4, 1, 1.0).unwrap(), b);
    }

    #[test]
    fn zero_motion_matches_the_cross_fade() {
        let a = vec![[0.2; 3]; 4];
        let b = vec![[0.6; 3]; 4];
        let motion = vec![[0.0, 0.0]; 4];
        let warped = interpolate(&a, &b, Some(&motion), 4, 1, 0.5).unwrap();
        let blended = interpolate(&a, &b, None, 4, 1, 0.5).unwrap();
        assert_eq!(warped, blended);
    }

    #[test]
    fn a_one_pixel_shift_halfway_puts_the_spot_between_both_frames() {
        // A bright pixel at x = 1 in a and at x = 2 in b, with motion (1, 0). At t = 0.5 each frame moves half a pixel:
        // output 1 samples a at 0.5 (0.5) and b at 1.5 (0.5); output 2 samples a at 1.5 (0.5) and b at 2.5 (0.5).
        // Outputs 0 and 3 sample only dark pixels. The result is [0, 0.5, 0.5, 0].
        let a = vec![[0.0; 3], [1.0; 3], [0.0; 3], [0.0; 3]];
        let b = vec![[0.0; 3], [0.0; 3], [1.0; 3], [0.0; 3]];
        let motion = vec![[1.0, 0.0]; 4];
        let out = interpolate(&a, &b, Some(&motion), 4, 1, 0.5).unwrap();
        let red: Vec<f32> = out.iter().map(|p| p[0]).collect();
        assert!(near(red[0], 0.0) && near(red[1], 0.5) && near(red[2], 0.5) && near(red[3], 0.0));
    }

    #[test]
    fn the_endpoints_return_the_input_frames_exactly() {
        let a = vec![[0.0; 3], [1.0; 3], [0.0; 3], [0.0; 3]];
        let b = vec![[0.0; 3], [0.0; 3], [1.0; 3], [0.0; 3]];
        let motion = vec![[1.0, 0.0]; 4];
        assert_eq!(interpolate(&a, &b, Some(&motion), 4, 1, 0.0).unwrap(), a);
        assert_eq!(interpolate(&a, &b, Some(&motion), 4, 1, 1.0).unwrap(), b);
    }

    #[test]
    fn refused_inputs_are_reported() {
        let a = vec![[0.0; 3]; 4];
        assert_eq!(interpolate(&a, &a, None, 0, 4, 0.5), Err(FrameError::ZeroSize));
        assert_eq!(interpolate(&a, &a[..3], None, 4, 1, 0.5), Err(FrameError::LengthMismatch));
        assert_eq!(interpolate(&a, &a, None, 4, 1, 1.5), Err(FrameError::TimeOutOfRange));
        assert_eq!(interpolate(&a, &a, None, 4, 1, f32::NAN), Err(FrameError::TimeOutOfRange));
    }
}
