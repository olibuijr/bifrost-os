//! Directional and disc blurs on packed pixels: a motion blur along a line, a zoom blur towards the
//! centre and a uniform disc blur. Each output pixel is the rounded average of the source pixels it
//! samples. Samples outside the image clamp to the nearest edge pixel. The buffer is `w * h` packed
//! pixels, red in the low byte, and every function returns `None` when the length does not match.
//! The gaussian and box blurs of the renderer stay in `gfx`; these are the shapes it lacks.

fn fits(px: &[u32], w: usize, h: usize) -> bool {
    w.checked_mul(h) == Some(px.len())
}

/// The source pixel at (x, y), with the coordinates clamped to the image.
fn at(px: &[u32], w: usize, h: usize, x: i64, y: i64) -> u32 {
    let x = x.clamp(0, w as i64 - 1) as usize;
    let y = y.clamp(0, h as i64 - 1) as usize;
    px[y * w + x]
}

/// Averages `n` packed pixels per channel, rounding to the nearest integer.
fn average(samples: impl Iterator<Item = u32>, n: u32) -> u32 {
    let mut sum = [0u32; 4];
    for p in samples {
        for (c, s) in sum.iter_mut().enumerate() {
            *s += (p >> (c * 8)) & 0xff;
        }
    }
    (0..4).fold(0u32, |out, c| out | (((sum[c] + n / 2) / n) << (c * 8)))
}

/// Blurs along a line of `length` samples centred on each pixel, at `angle_deg` from the x axis.
/// A length of 0 or 1 returns the pixels unchanged.
pub fn motion_blur(px: &[u32], w: usize, h: usize, length: usize, angle_deg: f32) -> Option<Vec<u32>> {
    if !fits(px, w, h) {
        return None;
    }
    if length <= 1 {
        return Some(px.to_vec());
    }
    let (s, c) = angle_deg.to_radians().sin_cos();
    let n = length;
    let half = (n as f32 - 1.0) / 2.0;
    let mut out = vec![0u32; px.len()];
    for y in 0..h {
        for x in 0..w {
            let samples = (0..n).map(|i| {
                let t = i as f32 - half;
                let sx = (x as f32 + t * c + 0.5).floor() as i64;
                let sy = (y as f32 + t * s + 0.5).floor() as i64;
                at(px, w, h, sx, sy)
            });
            out[y * w + x] = average(samples, n as u32);
        }
    }
    Some(out)
}

/// Blurs towards the image centre. `amount` is the fraction of the distance to the centre that the
/// samples span (0 to 1), and `samples` is how many points are taken along each ray.
pub fn zoom_blur(px: &[u32], w: usize, h: usize, amount: f32, samples: usize) -> Option<Vec<u32>> {
    if !fits(px, w, h) {
        return None;
    }
    if amount <= 0.0 || samples <= 1 {
        return Some(px.to_vec());
    }
    let amount = amount.min(1.0);
    let (cx, cy) = ((w as f32 - 1.0) * 0.5, (h as f32 - 1.0) * 0.5);
    let n = samples;
    let mut out = vec![0u32; px.len()];
    for y in 0..h {
        for x in 0..w {
            let (dx, dy) = (x as f32 - cx, y as f32 - cy);
            let samples = (0..n).map(|i| {
                let t = i as f32 / (n as f32 - 1.0);
                let scale = 1.0 - amount * t;
                let sx = (cx + dx * scale + 0.5).floor() as i64;
                let sy = (cy + dy * scale + 0.5).floor() as i64;
                at(px, w, h, sx, sy)
            });
            out[y * w + x] = average(samples, n as u32);
        }
    }
    Some(out)
}

/// Averages every pixel within `radius` of each pixel (a disc of whole-pixel offsets). A radius of 0
/// returns the pixels unchanged.
pub fn disc_blur(px: &[u32], w: usize, h: usize, radius: usize) -> Option<Vec<u32>> {
    if !fits(px, w, h) {
        return None;
    }
    if radius == 0 {
        return Some(px.to_vec());
    }
    let r = radius as i64;
    let offsets: Vec<(i64, i64)> = (-r..=r)
        .flat_map(|dy| (-r..=r).map(move |dx| (dx, dy)))
        .filter(|(dx, dy)| dx * dx + dy * dy <= r * r)
        .collect();
    let n = offsets.len() as u32;
    let mut out = vec![0u32; px.len()];
    for y in 0..h {
        for x in 0..w {
            let (ix, iy) = (x as i64, y as i64);
            let samples = offsets.iter().map(|&(dx, dy)| at(px, w, h, ix + dx, iy + dy));
            out[y * w + x] = average(samples, n);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPIKE: u32 = 0xffff_ffff;

    /// A 5 by 5 image of black, transparent, with one opaque white pixel at the centre (2, 2).
    fn spike_5x5() -> Vec<u32> {
        let mut px = vec![0u32; 25];
        px[2 * 5 + 2] = SPIKE;
        px
    }

    #[test]
    fn motion_blur_spreads_a_spike_along_the_row() {
        // One row of 5 pixels, spike at x = 2, a 3-sample window. Each neighbour of the spike
        // averages 255 once over 3 samples: (255 + 1) / 3 = 85. The ends clamp to black.
        let px = vec![0, 0, SPIKE, 0, 0];
        let out = motion_blur(&px, 5, 1, 3, 0.0).unwrap();
        let red: Vec<u32> = out.iter().map(|p| p & 0xff).collect();
        assert_eq!(red, vec![0, 85, 85, 85, 0]);
    }

    #[test]
    fn motion_blur_rejects_a_wrong_length() {
        assert_eq!(motion_blur(&[0; 3], 2, 2, 3, 0.0), None);
    }

    #[test]
    fn flat_image_is_unchanged_by_every_blur() {
        let flat = vec![0xff1e_3c5a; 16];
        assert_eq!(motion_blur(&flat, 4, 4, 5, 33.0).unwrap(), flat);
        assert_eq!(zoom_blur(&flat, 4, 4, 0.8, 6).unwrap(), flat);
        assert_eq!(disc_blur(&flat, 4, 4, 2).unwrap(), flat);
    }

    #[test]
    fn disc_blur_of_a_spike_spreads_over_the_five_pixel_cross() {
        // Radius 1 keeps the centre and its four orthogonal neighbours; the diagonals sit at
        // distance sqrt(2) and drop out. One opaque pixel over 5 samples: (255 + 2) / 5 = 51.
        let out = disc_blur(&spike_5x5(), 5, 5, 1).unwrap();
        let at_xy = |x: usize, y: usize| out[y * 5 + x];
        assert_eq!(at_xy(2, 2) & 0xff, 51);
        assert_eq!(at_xy(1, 2) & 0xff, 51);
        assert_eq!(at_xy(2, 1) & 0xff, 51);
        assert_eq!(at_xy(1, 1) & 0xff, 0);
        assert_eq!(at_xy(3, 3) & 0xff, 0);
    }

    #[test]
    fn zoom_blur_keeps_the_centre_and_smears_its_neighbour() {
        // The centre maps to itself at every scale, so it keeps the spike's 255.
        // The pixel left of centre samples scales 1, 0.75 and 0.5; only the last lands on the spike
        // (x = 2 - 0.5 * 1 = 1.5, which rounds to 2): (255 + 1) / 3 = 85.
        let out = zoom_blur(&spike_5x5(), 5, 5, 0.5, 3).unwrap();
        assert_eq!(out[2 * 5 + 2] & 0xff, 255);
        assert_eq!(out[2 * 5 + 1] & 0xff, 85);
    }

    #[test]
    fn zero_amount_or_radius_returns_the_input() {
        let px = spike_5x5();
        assert_eq!(zoom_blur(&px, 5, 5, 0.0, 4).unwrap(), px);
        assert_eq!(disc_blur(&px, 5, 5, 0).unwrap(), px);
    }
}
