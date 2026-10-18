//! Procedural animated wallpaper frames, as pure per-pixel functions (std only). Four looks, each a CPU reference
//! for a fragment shader (`wallpaper_drift.frag` is the GPU version of the first):
//!
//! - `drift_frame`: a slowly flowing, domain-warped fractal noise field coloured through a ramp.
//! - `waves_frame`: layered, travelling sine ridges drawn back to front over a sky ramp (a coastal-layers look).
//! - `dither_frame`: a retro one-bit look, an ordered 4x4 Bayer dither of a vertical ramp that scrolls sideways.
//! - `dynamic_frame`: a time-of-day poster that cross-fades between ramps keyed to hours of the day.
//!
//! Coordinates are normalised by the longer side, so a frame looks the same at any resolution and aspect ratio.
//! Frames are row-major `Rgb` with channels in 0..=1. `wallpaper_frame::render_frame` is the plain two-ramp fade.

use std::f32::consts::TAU;

use crate::colorfx::Rgb;
use crate::gradient::{cross_fade, linear_t, Ramp};

/// A deterministic hash of a lattice point to 0..1.
fn hash(ix: i32, iy: i32, seed: u32) -> f32 {
    let mut h = (ix as u32).wrapping_mul(0x85EB_CA6B) ^ (iy as u32).wrapping_mul(0xC2B2_AE35) ^ seed.wrapping_mul(0x27D4_EB2F);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297A_2D39);
    h ^= h >> 15;
    (h >> 8) as f32 / 16_777_216.0
}

fn smooth(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

/// Smooth value noise in 0..=1: lattice hashes blended with a smoothstep. Equal to the hash at lattice points.
pub fn value_noise(x: f32, y: f32, seed: u32) -> f32 {
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (smooth(x - x0), smooth(y - y0));
    let (ix, iy) = (x0 as i32, y0 as i32);
    let a = hash(ix, iy, seed);
    let b = hash(ix + 1, iy, seed);
    let c = hash(ix, iy + 1, seed);
    let d = hash(ix + 1, iy + 1, seed);
    (a + (b - a) * fx) + ((c + (d - c) * fx) - (a + (b - a) * fx)) * fy
}

/// Fractal noise in 0..=1: `octaves` layers of value noise, each at twice the frequency and half the weight.
pub fn fbm(x: f32, y: f32, octaves: u32, seed: u32) -> f32 {
    let mut sum = 0.0;
    let mut amp = 0.5;
    let mut norm = 0.0;
    let (mut px, mut py) = (x, y);
    for o in 0..octaves.max(1) {
        sum += amp * value_noise(px, py, seed.wrapping_add(o));
        norm += amp;
        amp *= 0.5;
        px *= 2.0;
        py *= 2.0;
    }
    sum / norm
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    smooth(((x - e0) / (e1 - e0)).clamp(0.0, 1.0))
}

/// The drift field value (0..=1) at normalised coordinates `(u, v)` and `time` seconds: noise warped by noise,
/// with the warp and the sample drifting at different slow speeds, stretched to use the full range.
pub fn drift_value(u: f32, v: f32, time: f32, scale: f32, seed: u32) -> f32 {
    let (x, y) = (u * scale, v * scale);
    let qx = fbm(x + time * 0.05, y, 3, seed);
    let qy = fbm(x + 5.2, y + 1.3 - time * 0.04, 3, seed.wrapping_add(7));
    let f = fbm(x + 4.0 * qx + time * 0.03, y + 4.0 * qy, 4, seed.wrapping_add(13));
    smoothstep(0.2, 0.8, f)
}

/// One frame of the drift look. `scale` is the noise cells across the longer side (2 to 4 reads as large soft
/// clouds). Empty when a side is zero.
pub fn drift_frame(width: usize, height: usize, time: f32, ramp: &Ramp, scale: f32, seed: u32) -> Vec<Rgb> {
    let side = width.max(height) as f32;
    let mut out = Vec::with_capacity(width * height);
    for y in 0..height {
        for x in 0..width {
            let u = (x as f32 + 0.5) / side;
            let v = (y as f32 + 0.5) / side;
            out.push(ramp.color_at(drift_value(u, v, time, scale, seed)));
        }
    }
    out
}

/// One travelling ridge of the layered-waves look.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WaveLayer {
    /// Mean height of the ridge as a fraction of the frame height from the top.
    pub base: f32,
    /// Amplitude as a fraction of the frame height.
    pub amplitude: f32,
    /// Wavelength as a fraction of the frame width.
    pub wavelength: f32,
    /// Cycles per second. The frame repeats exactly every `1 / speed` seconds.
    pub speed: f32,
    /// Phase offset in cycles.
    pub phase: f32,
    pub color: Rgb,
}

/// The ridge height of a layer as a fraction of frame height at horizontal fraction `x` and `time` seconds: a
/// travelling fundamental plus a half-weight second harmonic travelling the other way.
pub fn wave_edge(layer: &WaveLayer, x: f32, time: f32) -> f32 {
    let u = x / layer.wavelength.max(1e-4);
    let a = (TAU * (u - layer.speed * time + layer.phase)).sin();
    let b = (TAU * (2.0 * u + layer.speed * time + 3.0 * layer.phase)).sin();
    layer.base + layer.amplitude * (a + 0.5 * b) / 1.5
}

/// One frame of layered waves: the sky ramp from top to bottom, then each layer back to front with a one-pixel
/// anti-aliased edge.
pub fn waves_frame(width: usize, height: usize, time: f32, sky: &Ramp, layers: &[WaveLayer]) -> Vec<Rgb> {
    let mut out = Vec::with_capacity(width * height);
    for y in 0..height {
        let py = y as f32 + 0.5;
        for x in 0..width {
            let fx = (x as f32 + 0.5) / width as f32;
            let mut c = sky.color_at(py / height as f32);
            for layer in layers {
                let edge = wave_edge(layer, fx, time) * height as f32;
                let cover = (py - edge + 0.5).clamp(0.0, 1.0);
                if cover > 0.0 {
                    c = c.mix(layer.color, cover);
                }
            }
            out.push(c);
        }
    }
    out
}

/// The ordered 4x4 Bayer matrix.
const BAYER4: [u8; 16] = [0, 8, 2, 10, 12, 4, 14, 6, 3, 11, 1, 9, 15, 7, 13, 5];

/// One frame of the one-bit dither look: a vertical ramp from `dark` at the top to `light` at the bottom, quantised
/// with the Bayer matrix, in blocks of `cell` pixels, with the pattern scrolled right by `scroll` cells per second.
pub fn dither_frame(width: usize, height: usize, time: f32, dark: Rgb, light: Rgb, cell: usize, scroll: f32) -> Vec<Rgb> {
    let cell = cell.max(1);
    let rows = height.div_ceil(cell).max(1);
    let shift = (time * scroll).floor() as i64;
    let mut out = Vec::with_capacity(width * height);
    for y in 0..height {
        let cy = y / cell;
        let level = if rows > 1 { cy as f32 / (rows - 1) as f32 } else { 0.0 };
        for x in 0..width {
            let cx = (x / cell) as i64 - shift;
            let threshold = (f32::from(BAYER4[(cy % 4) * 4 + cx.rem_euclid(4) as usize]) + 0.5) / 16.0;
            out.push(if level > threshold { light } else { dark });
        }
    }
    out
}

/// The two ramps and the blend position for `hour` (any real, wrapped into 0..24) given ascending key `stops` in
/// hours. Returns `(from, to, t)`: the key before the hour, the key after it (the first key again after the last,
/// across midnight) and the 0..1 position between them. `None` for no stops, stops that are not strictly ascending
/// within 0..24, or a non-finite hour.
pub fn dynamic_phase(hour: f32, stops: &[f32]) -> Option<(usize, usize, f32)> {
    if stops.is_empty() || !hour.is_finite() {
        return None;
    }
    let n = stops.len();
    if stops.windows(2).any(|w| w[1] <= w[0]) || stops[0] < 0.0 || stops[n - 1] >= 24.0 {
        return None;
    }
    if n == 1 {
        return Some((0, 0, 0.0));
    }
    let h = hour.rem_euclid(24.0);
    for i in 0..n - 1 {
        if h >= stops[i] && h < stops[i + 1] {
            return Some((i, i + 1, (h - stops[i]) / (stops[i + 1] - stops[i])));
        }
    }
    let span = stops[0] + 24.0 - stops[n - 1];
    let since = if h >= stops[n - 1] { h - stops[n - 1] } else { h + 24.0 - stops[n - 1] };
    Some((n - 1, 0, since / span))
}

/// One frame of the time-of-day poster: a diagonal gradient whose colours cross-fade between the keyed `ramps`
/// (one per stop) as `hour` advances. `None` when `ramps` and `stops` differ in length or `dynamic_phase` refuses.
pub fn dynamic_frame(width: usize, height: usize, hour: f32, stops: &[f32], ramps: &[Ramp]) -> Option<Vec<Rgb>> {
    if ramps.len() != stops.len() {
        return None;
    }
    let (from, to, t) = dynamic_phase(hour, stops)?;
    let (w, h) = (width as f32, height as f32);
    let mut out = Vec::with_capacity(width * height);
    for y in 0..height {
        for x in 0..width {
            let pos = linear_t((0.0, 0.0), (w, h), x as f32 + 0.5, y as f32 + 0.5);
            out.push(cross_fade(&ramps[from], &ramps[to], t, pos));
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgb(r: f32, g: f32, b: f32) -> Rgb {
        Rgb::new(r, g, b)
    }

    fn near(a: Rgb, b: Rgb) -> bool {
        (a.r - b.r).abs() < 1e-4 && (a.g - b.g).abs() < 1e-4 && (a.b - b.b).abs() < 1e-4
    }

    fn ramp(a: Rgb, b: Rgb) -> Ramp {
        Ramp::new(&[(0.0, a), (1.0, b)]).unwrap()
    }

    #[test]
    fn noise_is_deterministic_bounded_and_continuous() {
        assert_eq!(value_noise(1.3, 2.7, 5), value_noise(1.3, 2.7, 5));
        assert_ne!(value_noise(1.3, 2.7, 5), value_noise(1.3, 2.7, 6));
        assert_eq!(value_noise(3.0, 4.0, 9), hash(3, 4, 9));
        for i in 0..200 {
            let x = i as f32 * 0.173;
            let v = value_noise(x, x * 0.61, 3);
            assert!((0.0..=1.0).contains(&v));
            let f = fbm(x, x * 0.4, 4, 1);
            assert!((0.0..=1.0).contains(&f), "{f}");
            let step = (value_noise(x + 0.001, x * 0.61, 3) - v).abs();
            assert!(step < 0.01);
        }
    }

    #[test]
    fn drift_frames_are_sized_stable_and_animate() {
        let r = ramp(rgb(0.0, 0.0, 0.2), rgb(1.0, 0.8, 0.6));
        let a = drift_frame(24, 16, 0.0, &r, 3.0, 1);
        assert_eq!(a.len(), 24 * 16);
        assert_eq!(a, drift_frame(24, 16, 0.0, &r, 3.0, 1));
        let b = drift_frame(24, 16, 20.0, &r, 3.0, 1);
        assert_ne!(a, b);
        assert!(a.iter().all(|c| c.b >= 0.2 - 1e-5 && c.r <= 1.0 + 1e-5));
        assert!(drift_frame(0, 4, 0.0, &r, 3.0, 1).is_empty());
    }

    #[test]
    fn waves_show_sky_above_and_the_front_layer_below() {
        let sky = ramp(rgb(0.0, 0.0, 1.0), rgb(1.0, 1.0, 1.0));
        let layers = [
            WaveLayer { base: 0.6, amplitude: 0.05, wavelength: 0.5, speed: 0.5, phase: 0.0, color: rgb(0.0, 0.5, 0.5) },
            WaveLayer { base: 0.8, amplitude: 0.05, wavelength: 0.4, speed: 0.25, phase: 0.3, color: rgb(0.0, 0.2, 0.3) },
        ];
        let (w, h) = (32, 100);
        let f = waves_frame(w, h, 1.0, &sky, &layers);
        assert_eq!(f.len(), w * h);
        assert!(near(f[0], sky.color_at(0.005)));
        assert!(near(f[(h - 1) * w + 5], layers[1].color));
        // The back layer shows between the layers.
        assert!(near(f[(h * 7 / 10) * w + 5], layers[0].color));
    }

    #[test]
    fn waves_repeat_every_period() {
        let layer = WaveLayer { base: 0.5, amplitude: 0.1, wavelength: 0.3, speed: 0.5, phase: 0.2, color: rgb(1.0, 0.0, 0.0) };
        for i in 0..20 {
            let x = i as f32 / 20.0;
            assert!((wave_edge(&layer, x, 0.7) - wave_edge(&layer, x, 0.7 + 2.0)).abs() < 1e-4);
        }
        assert!((wave_edge(&layer, 0.123, 0.0) - 0.5).abs() <= 0.1 + 1e-6);
    }

    #[test]
    fn dither_follows_the_ramp_and_scrolls() {
        let dark = rgb(0.0, 0.0, 0.0);
        let light = rgb(1.0, 1.0, 1.0);
        let f = dither_frame(16, 16, 0.0, dark, light, 1, 0.0);
        assert!(f[..16].iter().all(|c| *c == dark));
        assert!(f[15 * 16..].iter().all(|c| *c == light));
        // Mid rows mix both colours.
        let mid = &f[8 * 16..9 * 16];
        assert!(mid.iter().any(|c| *c == dark) && mid.iter().any(|c| *c == light));
        // One cell of scroll moves the pattern one pixel to the right.
        let g = dither_frame(16, 16, 1.0, dark, light, 1, 1.0);
        for x in 1..16 {
            assert_eq!(g[8 * 16 + x], f[8 * 16 + x - 1]);
        }
    }

    #[test]
    fn dynamic_phase_interpolates_and_wraps_midnight() {
        let stops = [6.0, 12.0, 18.0, 23.0];
        assert_eq!(dynamic_phase(9.0, &stops), Some((0, 1, 0.5)));
        let (a, b, t) = dynamic_phase(23.5, &stops).unwrap();
        assert_eq!((a, b), (3, 0));
        assert!((t - 0.5 / 7.0).abs() < 1e-6);
        let (a, b, t) = dynamic_phase(2.0, &stops).unwrap();
        assert_eq!((a, b), (3, 0));
        assert!((t - 3.0 / 7.0).abs() < 1e-6);
        assert_eq!(dynamic_phase(24.0 + 9.0, &stops), Some((0, 1, 0.5)));
        assert_eq!(dynamic_phase(5.0, &[]), None);
        assert_eq!(dynamic_phase(5.0, &[10.0, 9.0]), None);
        assert_eq!(dynamic_phase(5.0, &[30.0]), None);
        assert_eq!(dynamic_phase(5.0, &[8.0]), Some((0, 0, 0.0)));
    }

    #[test]
    fn dynamic_frame_fades_between_keyed_ramps() {
        let night = ramp(rgb(0.0, 0.0, 0.0), rgb(0.0, 0.0, 0.0));
        let day = ramp(rgb(1.0, 1.0, 1.0), rgb(1.0, 1.0, 1.0));
        let stops = [6.0, 12.0];
        let ramps = [night, day];
        let dawn = dynamic_frame(4, 4, 6.0, &stops, &ramps).unwrap();
        assert!(dawn.iter().all(|c| near(*c, rgb(0.0, 0.0, 0.0))));
        let mid = dynamic_frame(4, 4, 9.0, &stops, &ramps).unwrap();
        assert!(mid.iter().all(|c| near(*c, rgb(0.5, 0.5, 0.5))));
        assert!(dynamic_frame(4, 4, 9.0, &stops, &ramps[..1]).is_none());
    }
}
