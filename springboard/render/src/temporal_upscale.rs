//! Temporal upscaler model for CPU rasters: jittered low-resolution frames accumulate into a full-resolution history.
//!
//! Each frame is rendered at the input size with a sub-pixel jitter taken from a Halton (2, 3) sequence of eight
//! phases. `resolve` takes the colour, depth and motion of that frame and returns the output raster:
//!
//! 1. The current colour is upscaled with the spatial filter (`upscale::upscale_f32`, linear mode).
//! 2. Each output pixel is reprojected into the previous frame with its motion vector. Motion is in input pixels and
//!    points from the current sample back to its position in the previous frame.
//! 3. The history is rejected where the reprojected position leaves the output, or where the depth at the sample
//!    moved by more than `DEPTH_TOLERANCE` since the previous frame (an occlusion or a disocclusion).
//! 4. The accepted history is clamped to the min and max of the 3x3 current neighbourhood, so stale colour cannot
//!    drift away from what the frame now shows.
//! 5. The output is `history * w + current * (1 - w)`, where `w` is the history weight of the accumulation mode.
//!
//! `Accumulation::Denoised` keeps more history (0.95 against 0.9), so noise in the input averages out over more
//! frames. Before the colour is upscaled, it also runs a 3x3 box filter over the input colour (`box_denoise`), which
//! removes single-pixel noise from every frame, not only from the history. Standard accumulation skips that filter.
//! The model runs on the CPU and has no GPU pass, no colour-space argument beyond the spatial filter's linear mode,
//! and no reactive mask.

use crate::upscale::{upscale_f32, ColorProcessing, UpscaleError};

/// Number of jitter phases before the Halton sequence repeats.
pub const JITTER_PHASES: u64 = 8;

/// Largest depth change (in the 0..1 depth range) that still counts as the same surface.
pub const DEPTH_TOLERANCE: f32 = 0.02;

/// How the history is weighted against the current frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Accumulation {
    /// Standard accumulation: 90 % history, 10 % current frame.
    Standard,
    /// Denoised accumulation: 95 % history, 5 % current frame.
    Denoised,
}

impl Accumulation {
    /// The share of the accepted history in the output.
    pub fn history_weight(self) -> f32 {
        match self {
            Accumulation::Standard => 0.9,
            Accumulation::Denoised => 0.95,
        }
    }

    /// Whether the input colour is denoised before it is upscaled.
    pub fn denoises(self) -> bool {
        matches!(self, Accumulation::Denoised)
    }
}

/// Spatial denoise of a `w` x `h` colour raster: each sample becomes the mean of its 3x3 neighbourhood. Edge samples
/// repeat, so the mean of a flat raster is unchanged.
pub fn box_denoise(buf: &[[f32; 3]], w: usize, h: usize) -> Vec<[f32; 3]> {
    let mut out = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            let mut sum = [0.0f32; 3];
            for dy in -1isize..=1 {
                for dx in -1isize..=1 {
                    let nx = (x as isize + dx).clamp(0, w as isize - 1) as usize;
                    let ny = (y as isize + dy).clamp(0, h as isize - 1) as usize;
                    let p = buf[ny * w + nx];
                    for c in 0..3 {
                        sum[c] += p[c];
                    }
                }
            }
            out.push(sum.map(|s| s / 9.0));
        }
    }
    out
}

/// Radical inverse of `index` in `base`: the digits of `index` mirrored around the radix point, in 0..1.
pub fn radical_inverse(mut index: u32, base: u32) -> f32 {
    let inv = 1.0 / base as f32;
    let mut scale = 1.0f32;
    let mut value = 0.0f32;
    while index > 0 {
        scale *= inv;
        value += scale * (index % base) as f32;
        index /= base;
    }
    value
}

/// The sub-pixel jitter (x, y) in -0.5..0.5 input pixels for `frame`. Phase 0 uses Halton index 1, so the
/// sequence never sits on the pixel centre at index 0.
pub fn jitter(frame: u64) -> (f32, f32) {
    let index = (frame % JITTER_PHASES) as u32 + 1;
    (radical_inverse(index, 2) - 0.5, radical_inverse(index, 3) - 0.5)
}

/// Clamps each channel of `v` to the range `lo..=hi` of the same channel.
pub fn clamp_to_range(v: [f32; 3], lo: [f32; 3], hi: [f32; 3]) -> [f32; 3] {
    [0, 1, 2].map(|c| v[c].clamp(lo[c], hi[c]))
}

/// True when the depth at a sample moved by no more than `tolerance` since the previous frame.
pub fn history_valid(depth_now: f32, depth_prev: f32, tolerance: f32) -> bool {
    (depth_now - depth_prev).abs() <= tolerance
}

/// The output-pixel-centre position in the previous frame. `(x, y)` is the output pixel, `motion` is in input
/// pixels, and `sx`, `sy` are the output-to-input scale factors (output size divided by input size).
pub fn reproject(x: usize, y: usize, motion: [f32; 2], sx: f32, sy: f32) -> (f32, f32) {
    let hx = (x as f32 + 0.5) - motion[0] * sx;
    let hy = (y as f32 + 0.5) - motion[1] * sy;
    (hx, hy)
}

/// Blends a history colour with the current colour: `history * w + current * (1 - w)`.
pub fn blend(history: [f32; 3], current: [f32; 3], history_weight: f32) -> [f32; 3] {
    [0, 1, 2].map(|c| history[c] * history_weight + current[c] * (1.0 - history_weight))
}

/// Per-channel min and max of the 3x3 neighbourhood around `(x, y)` in a `w` x `h` raster. Edge samples repeat.
pub fn neighbourhood(buf: &[[f32; 3]], w: usize, h: usize, x: usize, y: usize) -> ([f32; 3], [f32; 3]) {
    let mut lo = [f32::MAX; 3];
    let mut hi = [f32::MIN; 3];
    for dy in -1isize..=1 {
        for dx in -1isize..=1 {
            let nx = (x as isize + dx).clamp(0, w as isize - 1) as usize;
            let ny = (y as isize + dy).clamp(0, h as isize - 1) as usize;
            let p = buf[ny * w + nx];
            for c in 0..3 {
                lo[c] = lo[c].min(p[c]);
                hi[c] = hi[c].max(p[c]);
            }
        }
    }
    (lo, hi)
}

/// Bilinear sample of a `w` x `h` raster at the fractional position (`fx`, `fy`) in sample units. Positions outside
/// the raster clamp to its edge.
pub fn sample_bilinear(buf: &[[f32; 3]], w: usize, h: usize, fx: f32, fy: f32) -> [f32; 3] {
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

/// Accumulates jittered frames into a full-resolution history.
pub struct TemporalScaler {
    in_w: usize,
    in_h: usize,
    out_w: usize,
    out_h: usize,
    mode: Accumulation,
    frame: u64,
    history: Option<Vec<[f32; 3]>>,
    prev_depth: Option<Vec<f32>>,
}

impl TemporalScaler {
    /// A scaler from `in_w` x `in_h` frames to `out_w` x `out_h` output. Any zero dimension is refused.
    pub fn new(
        in_w: usize,
        in_h: usize,
        out_w: usize,
        out_h: usize,
        mode: Accumulation,
    ) -> Result<Self, UpscaleError> {
        if in_w == 0 || in_h == 0 || out_w == 0 || out_h == 0 {
            return Err(UpscaleError::ZeroSize);
        }
        Ok(TemporalScaler { in_w, in_h, out_w, out_h, mode, frame: 0, history: None, prev_depth: None })
    }

    /// The jitter to apply when rendering the next frame.
    pub fn next_jitter(&self) -> (f32, f32) {
        jitter(self.frame)
    }

    /// Drops the history. The next frame is output as the spatial upscale of its colour.
    pub fn reset(&mut self) {
        self.history = None;
        self.prev_depth = None;
    }

    /// Resolves one frame. `color` and `depth` hold one entry per input pixel. `motion` holds one input-pixel offset
    /// per input pixel. Returns the output raster, `out_w * out_h` samples.
    pub fn resolve(
        &mut self,
        color: &[[f32; 3]],
        depth: &[f32],
        motion: &[[f32; 2]],
    ) -> Result<Vec<[f32; 3]>, UpscaleError> {
        let n = self.in_w * self.in_h;
        if color.len() != n || depth.len() != n || motion.len() != n {
            return Err(UpscaleError::LengthMismatch);
        }
        let denoised;
        let source: &[[f32; 3]] = if self.mode.denoises() {
            denoised = box_denoise(color, self.in_w, self.in_h);
            &denoised
        } else {
            color
        };
        let current = upscale_f32(source, self.in_w, self.in_h, self.out_w, self.out_h, ColorProcessing::Linear)?;
        let out = match (&self.history, &self.prev_depth) {
            (Some(history), Some(prev_depth)) => self.accumulate(&current, history, prev_depth, depth, motion),
            _ => current,
        };
        self.history = Some(out.clone());
        self.prev_depth = Some(depth.to_vec());
        self.frame += 1;
        Ok(out)
    }

    fn accumulate(
        &self,
        current: &[[f32; 3]],
        history: &[[f32; 3]],
        prev_depth: &[f32],
        depth: &[f32],
        motion: &[[f32; 2]],
    ) -> Vec<[f32; 3]> {
        let (in_w, in_h, out_w, out_h) = (self.in_w, self.in_h, self.out_w, self.out_h);
        let sx = out_w as f32 / in_w as f32;
        let sy = out_h as f32 / in_h as f32;
        let weight = self.mode.history_weight();
        let mut out = Vec::with_capacity(out_w * out_h);
        for y in 0..out_h {
            for x in 0..out_w {
                let cur = current[y * out_w + x];
                // The input pixel under this output pixel.
                let ix = ((x as f32 + 0.5) / sx).floor().clamp(0.0, (in_w - 1) as f32) as usize;
                let iy = ((y as f32 + 0.5) / sy).floor().clamp(0.0, (in_h - 1) as f32) as usize;
                let m = motion[iy * in_w + ix];
                let (hx, hy) = reproject(x, y, m, sx, sy);
                let inside = hx >= 0.0 && hy >= 0.0 && hx < out_w as f32 && hy < out_h as f32;
                let px = ((hx / sx).floor().max(0.0) as usize).min(in_w - 1);
                let py = ((hy / sy).floor().max(0.0) as usize).min(in_h - 1);
                let same_surface = history_valid(depth[iy * in_w + ix], prev_depth[py * in_w + px], DEPTH_TOLERANCE);
                if inside && same_surface {
                    let (lo, hi) = neighbourhood(current, out_w, out_h, x, y);
                    let h = clamp_to_range(
                        sample_bilinear(history, out_w, out_h, hx - 0.5, hy - 0.5),
                        lo,
                        hi,
                    );
                    out.push(blend(h, cur, weight));
                } else {
                    out.push(cur);
                }
            }
        }
        out
    }
}

/// Virtual shutter: averages the newest sub-frames that fall inside a shutter of `angle_deg` degrees. `frames` are the
/// sub-frames of one output interval, oldest first, spread evenly over 360 degrees, so a 180 degree shutter over four
/// frames averages the last two. At least one frame is always used. Returns `None` for no frames, a shutter outside
/// (0, 360], or frames of unequal length.
pub fn shutter_blend(frames: &[&[[f32; 3]]], angle_deg: f32) -> Option<Vec<[f32; 3]>> {
    if frames.is_empty() || !(angle_deg > 0.0 && angle_deg <= 360.0) {
        return None;
    }
    let n = frames[0].len();
    if frames.iter().any(|f| f.len() != n) {
        return None;
    }
    let k = ((frames.len() as f32 * angle_deg / 360.0).round() as usize).clamp(1, frames.len());
    let used = &frames[frames.len() - k..];
    let mut out = vec![[0.0f32; 3]; n];
    for frame in used {
        for (o, c) in out.iter_mut().zip(frame.iter()) {
            for ch in 0..3 {
                o[ch] += c[ch];
            }
        }
    }
    for o in out.iter_mut() {
        for ch in 0..3 {
            o[ch] /= k as f32;
        }
    }
    Some(out)
}

/// Temporal deghosting: the per-channel median of the sub-frames of one interval. A moving object
/// is in each frame at a different place, so the median keeps the static background and drops the
/// ghost of the object, which a plain average would smear. `frames` are oldest first, and all have
/// the same length. With an even count the median is the mean of the two middle values. Returns
/// `None` for no frames or frames of unequal length.
pub fn median_deghost(frames: &[&[[f32; 3]]]) -> Option<Vec<[f32; 3]>> {
    if frames.is_empty() {
        return None;
    }
    let n = frames[0].len();
    if frames.iter().any(|f| f.len() != n) {
        return None;
    }
    let m = frames.len();
    let mut out = vec![[0.0f32; 3]; n];
    let mut column = vec![0.0f32; m];
    for (i, o) in out.iter_mut().enumerate() {
        for ch in 0..3 {
            for (c, f) in column.iter_mut().zip(frames.iter()) {
                *c = f[i][ch];
            }
            column.sort_by(|a, b| a.total_cmp(b));
            o[ch] = if m % 2 == 1 {
                column[m / 2]
            } else {
                (column[m / 2 - 1] + column[m / 2]) * 0.5
            };
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn radical_inverse_mirrors_the_digits() {
        // 3 is 11 in base 2, so its mirror is 0.11 = 0.75. 1 in base 3 is 0.1 = 1/3, 2 in base 3 is 0.2 = 2/3.
        assert!(near(radical_inverse(1, 2), 0.5));
        assert!(near(radical_inverse(2, 2), 0.25));
        assert!(near(radical_inverse(3, 2), 0.75));
        assert!(near(radical_inverse(1, 3), 1.0 / 3.0));
        assert!(near(radical_inverse(2, 3), 2.0 / 3.0));
    }

    #[test]
    fn jitter_follows_the_halton_phases_and_repeats() {
        // Frame 0 is Halton index 1: x = 0.5 - 0.5 = 0, y = 1/3 - 0.5 = -1/6.
        let (x0, y0) = jitter(0);
        assert!(near(x0, 0.0) && near(y0, -1.0 / 6.0));
        // Frame 1 is index 2: x = 0.25 - 0.5 = -0.25, y = 2/3 - 0.5 = 1/6.
        let (x1, y1) = jitter(1);
        assert!(near(x1, -0.25) && near(y1, 1.0 / 6.0));
        assert_eq!(jitter(JITTER_PHASES), jitter(0));
    }

    #[test]
    fn history_outside_the_neighbourhood_is_clamped_before_blending() {
        // History 0.9 with a neighbourhood of 0.2..0.5 clamps to 0.5. Blending 0.5 with a current 0.1 at weight 0.9
        // gives 0.5 * 0.9 + 0.1 * 0.1 = 0.46.
        let h = clamp_to_range([0.9, 0.1, 0.35], [0.2, 0.2, 0.2], [0.5, 0.5, 0.5]);
        assert_eq!(h, [0.5, 0.2, 0.35]);
        let out = blend([0.5, 0.5, 0.5], [0.1, 0.1, 0.1], Accumulation::Standard.history_weight());
        assert!(near(out[0], 0.46));
    }

    #[test]
    fn denoised_accumulation_keeps_more_history() {
        assert!(near(Accumulation::Standard.history_weight(), 0.9));
        assert!(near(Accumulation::Denoised.history_weight(), 0.95));
        // Standard history 0 against current 1 gives 0.1; denoised gives 0.05.
        assert!(near(blend([0.0; 3], [1.0; 3], Accumulation::Standard.history_weight())[0], 0.1));
        assert!(near(blend([0.0; 3], [1.0; 3], Accumulation::Denoised.history_weight())[0], 0.05));
    }

    #[test]
    fn reprojection_moves_the_sample_back_by_its_motion_in_output_pixels() {
        // Output pixel (10, 5) with motion (1, 0) input pixels at a 2x scale: the centre (10.5, 5.5) moves back by 2.
        let (hx, hy) = reproject(10, 5, [1.0, 0.0], 2.0, 2.0);
        assert!(near(hx, 8.5) && near(hy, 5.5));
        assert!(history_valid(0.5, 0.505, DEPTH_TOLERANCE));
        assert!(!history_valid(0.5, 0.9, DEPTH_TOLERANCE));
    }

    #[test]
    fn neighbourhood_min_max_repeats_the_edge_samples() {
        // A 3x1 row of grey values 0.0, 0.3, 1.0. The centre sees 0.0..1.0; the left edge sees 0.0..0.3.
        let row = [[0.0; 3], [0.3; 3], [1.0; 3]];
        let (lo, hi) = neighbourhood(&row, 3, 1, 1, 0);
        assert!(near(lo[0], 0.0) && near(hi[0], 1.0));
        let (lo, hi) = neighbourhood(&row, 3, 1, 0, 0);
        assert!(near(lo[0], 0.0) && near(hi[0], 0.3));
    }

    #[test]
    fn bilinear_sample_interpolates_and_clamps_at_the_edges() {
        let row = [[0.0; 3], [1.0; 3]];
        assert!(near(sample_bilinear(&row, 2, 1, 0.5, 0.0)[0], 0.5));
        assert!(near(sample_bilinear(&row, 2, 1, -3.0, 0.0)[0], 0.0));
        assert!(near(sample_bilinear(&row, 2, 1, 9.0, 0.0)[0], 1.0));
    }

    #[test]
    fn flat_frames_stay_flat_through_accumulation() {
        let mut s = TemporalScaler::new(2, 2, 4, 4, Accumulation::Standard).unwrap();
        let color = vec![[0.5; 3]; 4];
        let depth = vec![0.3; 4];
        let motion = vec![[0.0, 0.0]; 4];
        let first = s.resolve(&color, &depth, &motion).unwrap();
        let second = s.resolve(&color, &depth, &motion).unwrap();
        assert_eq!(first.len(), 16);
        assert!(first.iter().chain(second.iter()).all(|p| near(p[0], 0.5)));
        // Two frames rendered, so the next frame uses phase 2.
        assert_eq!(s.next_jitter(), jitter(2));
    }

    #[test]
    fn refused_inputs_are_reported() {
        assert_eq!(TemporalScaler::new(0, 2, 4, 4, Accumulation::Standard).err(), Some(UpscaleError::ZeroSize));
        let mut s = TemporalScaler::new(2, 2, 4, 4, Accumulation::Standard).unwrap();
        let short = vec![[0.5; 3]; 3];
        let depth = vec![0.3; 4];
        let motion = vec![[0.0, 0.0]; 4];
        assert_eq!(s.resolve(&short, &depth, &motion).err(), Some(UpscaleError::LengthMismatch));
    }

    #[test]
    fn shutter_blend_averages_the_frames_inside_the_shutter() {
        let a = [[0.0f32; 3]];
        let b = [[0.0f32; 3]];
        let c = [[1.0f32; 3]];
        let d = [[3.0f32; 3]];
        let frames: [&[[f32; 3]]; 4] = [&a, &b, &c, &d];
        // A full shutter averages all four frames: (0 + 0 + 1 + 3) / 4.
        assert!(near(shutter_blend(&frames, 360.0).unwrap()[0][0], 1.0));
        // A 180 degree shutter keeps the newest two: (1 + 3) / 2.
        assert!(near(shutter_blend(&frames, 180.0).unwrap()[0][0], 2.0));
        // A tiny shutter still uses one frame: the newest.
        assert!(near(shutter_blend(&frames, 1.0).unwrap()[0][0], 3.0));
    }

    #[test]
    fn shutter_blend_refuses_bad_input() {
        let one = [[0.5f32; 3]];
        let two = [[0.5f32; 3], [0.5f32; 3]];
        assert!(shutter_blend(&[], 180.0).is_none());
        assert!(shutter_blend(&[&one[..]], 0.0).is_none());
        assert!(shutter_blend(&[&one[..]], 400.0).is_none());
        assert!(shutter_blend(&[&one[..], &two[..]], 180.0).is_none());
        assert_eq!(shutter_blend(&[&one[..]], 90.0).unwrap(), vec![[0.5; 3]]);
    }

    #[test]
    fn median_deghost_drops_a_moving_ghost() {
        // Background 0.2 in every frame; a bright object (0.9) passes through pixel 0 in one frame only.
        let bg = [[0.2f32; 3]];
        let ghost = [[0.9f32; 3]];
        let frames: [&[[f32; 3]]; 3] = [&bg, &ghost, &bg];
        let out = median_deghost(&frames).unwrap();
        assert!(near(out[0][0], 0.2));
        // The plain average would keep the ghost: (0.2 + 0.9 + 0.2) / 3 is about 0.433.
        assert!(shutter_blend(&frames, 360.0).unwrap()[0][0] > 0.4);
    }

    #[test]
    fn median_deghost_even_count_and_bad_input() {
        let a = [[0.0f32; 3]];
        let b = [[1.0f32; 3]];
        assert!(near(median_deghost(&[&a[..], &b[..]]).unwrap()[0][1], 0.5));
        assert!(median_deghost(&[]).is_none());
        let two = [[0.5f32; 3], [0.5f32; 3]];
        assert!(median_deghost(&[&a[..], &two[..]]).is_none());
    }
}
