//! Colour adjustments and effects on single pixels: exposure, saturation, contrast, hue rotation,
//! temperature and tint, gamma, sepia, invert, posterize, threshold and vignette. Colours are unpremultiplied RGB with
//! each channel in 0..=1. Every function is pure; the Photos edit and app filters map their pixel
//! buffers through them with `map_pixel`.

/// An unpremultiplied colour, each channel in 0..=1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgb {
    pub r: f32,
    pub g: f32,
    pub b: f32,
}

/// Rec. 601 luma weights, the same ones the Photos filters use.
const LUMA: (f32, f32, f32) = (0.299, 0.587, 0.114);

impl Rgb {
    pub const fn new(r: f32, g: f32, b: f32) -> Self {
        Rgb { r, g, b }
    }

    /// Each channel clamped to 0..=1.
    pub fn clamped(self) -> Self {
        Rgb::new(self.r.clamp(0.0, 1.0), self.g.clamp(0.0, 1.0), self.b.clamp(0.0, 1.0))
    }

    /// Linear blend: `self` at `t = 0`, `other` at `t = 1`. `t` is not clamped.
    pub fn mix(self, other: Rgb, t: f32) -> Rgb {
        Rgb::new(self.r + (other.r - self.r) * t, self.g + (other.g - self.g) * t, self.b + (other.b - self.b) * t)
    }

    /// Rec. 601 luma of the colour.
    pub fn luma(self) -> f32 {
        LUMA.0 * self.r + LUMA.1 * self.g + LUMA.2 * self.b
    }

    fn map(self, f: impl Fn(f32) -> f32) -> Rgb {
        Rgb::new(f(self.r), f(self.g), f(self.b))
    }
}

/// Multiplies the light by `2^ev`, so each stop doubles (positive) or halves (negative) it. Clamped to 0..=1.
pub fn exposure(c: Rgb, ev: f32) -> Rgb {
    c.map(|v| v * ev.exp2()).clamped()
}

/// Moves each channel away from (above 1) or towards (below 1) the pixel's own luma. 0 gives grey,
/// 1 leaves the colour unchanged.
pub fn saturation(c: Rgb, amount: f32) -> Rgb {
    let l = c.luma();
    Rgb::new(l + (c.r - l) * amount, l + (c.g - l) * amount, l + (c.b - l) * amount).clamped()
}

/// Scales each channel around mid grey. 1 leaves the colour unchanged, 0 gives flat grey.
pub fn contrast(c: Rgb, amount: f32) -> Rgb {
    c.map(|v| (v - 0.5) * amount + 0.5).clamped()
}

/// Rotates the hue by `degrees` with a matrix whose rows sum to 1, so grey stays grey.
pub fn hue_rotate(c: Rgb, degrees: f32) -> Rgb {
    let (s, k) = degrees.to_radians().sin_cos();
    let m = [
        [0.213 + k * 0.787 - s * 0.213, 0.715 - k * 0.715 - s * 0.715, 0.072 - k * 0.072 + s * 0.928],
        [0.213 - k * 0.213 + s * 0.143, 0.715 + k * 0.285 + s * 0.140, 0.072 - k * 0.072 - s * 0.283],
        [0.213 - k * 0.213 - s * 0.787, 0.715 - k * 0.715 + s * 0.715, 0.072 + k * 0.928 + s * 0.072],
    ];
    let row = |r: [f32; 3]| r[0] * c.r + r[1] * c.g + r[2] * c.b;
    Rgb::new(row(m[0]), row(m[1]), row(m[2])).clamped()
}

/// A gain model of white balance, not a white-point conversion. `temperature` runs from -1 (cool)
/// to 1 (warm): red up and blue down. `tint` runs from -1 (green) to 1 (magenta): green down and
/// red and blue up.
pub fn temperature_tint(c: Rgb, temperature: f32, tint: f32) -> Rgb {
    Rgb::new(
        c.r * (1.0 + 0.25 * temperature + 0.1 * tint),
        c.g * (1.0 - 0.2 * tint),
        c.b * (1.0 - 0.25 * temperature + 0.1 * tint),
    )
    .clamped()
}

/// Blends towards the sepia tone matrix. 0 leaves the colour unchanged, 1 is full sepia.
pub fn sepia(c: Rgb, amount: f32) -> Rgb {
    let tone = Rgb::new(
        0.393 * c.r + 0.769 * c.g + 0.189 * c.b,
        0.349 * c.r + 0.686 * c.g + 0.168 * c.b,
        0.272 * c.r + 0.534 * c.g + 0.131 * c.b,
    )
    .clamped();
    c.mix(tone, amount.clamp(0.0, 1.0))
}

/// Each channel reversed around 1.
pub fn invert(c: Rgb) -> Rgb {
    c.map(|v| 1.0 - v)
}

/// Raises each channel to `power` (a gamma curve). 1 leaves the colour unchanged; above 1 darkens the mid tones, below 1
/// lifts them. A non-positive power is treated as 1 so the curve stays defined.
pub fn gamma(c: Rgb, power: f32) -> Rgb {
    let p = if power > 0.0 { power } else { 1.0 };
    c.map(|v| v.clamp(0.0, 1.0).powf(p)).clamped()
}

/// Black or white by luma: white where the Rec. 601 luma reaches `threshold`, black below it.
pub fn threshold(c: Rgb, threshold: f32) -> Rgb {
    let v = if c.luma() >= threshold { 1.0 } else { 0.0 };
    Rgb::new(v, v, v)
}

/// Quantises each channel to `levels` evenly spaced steps. Fewer than 2 levels is treated as 2.
pub fn posterize(c: Rgb, levels: u32) -> Rgb {
    let n = levels.max(2) as f32 - 1.0;
    c.map(|v| (v.clamp(0.0, 1.0) * n + 0.5).floor() / n)
}

/// The darkening factor for the point (x, y) in a `w` by `h` area: 1 at the centre, `1 - strength`
/// at the corners, falling off with the squared distance from the centre.
pub fn vignette_factor(x: f32, y: f32, w: f32, h: f32, strength: f32) -> f32 {
    if w <= 0.0 || h <= 0.0 {
        return 1.0;
    }
    let dx = (x - w * 0.5) / (w * 0.5);
    let dy = (y - h * 0.5) / (h * 0.5);
    let d2 = ((dx * dx + dy * dy) / 2.0).clamp(0.0, 1.0);
    (1.0 - strength.clamp(0.0, 1.0) * d2).clamp(0.0, 1.0)
}

/// `c` darkened by `vignette_factor` at (x, y).
pub fn vignette(c: Rgb, x: f32, y: f32, w: f32, h: f32, strength: f32) -> Rgb {
    let f = vignette_factor(x, y, w, h, strength);
    c.map(|v| v * f).clamped()
}

/// Applies `f` to a packed pixel (red in the low byte, then green, blue and alpha) and keeps its alpha.
pub fn map_pixel(px: u32, f: impl Fn(Rgb) -> Rgb) -> u32 {
    let ch = |shift: u32| ((px >> shift) & 0xff) as f32 / 255.0;
    let out = f(Rgb::new(ch(0), ch(8), ch(16))).clamped();
    let q = |v: f32| (v * 255.0 + 0.5) as u32;
    q(out.r) | (q(out.g) << 8) | (q(out.b) << 16) | (px & 0xff00_0000)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    fn same(a: Rgb, b: Rgb) -> bool {
        close(a.r, b.r) && close(a.g, b.g) && close(a.b, b.b)
    }

    #[test]
    fn exposure_doubles_per_stop_and_clamps() {
        assert!(close(exposure(Rgb::new(0.25, 0.25, 0.25), 1.0).r, 0.5));
        assert!(close(exposure(Rgb::new(0.25, 0.25, 0.25), -2.0).g, 0.0625));
        assert!(close(exposure(Rgb::new(0.8, 0.8, 0.8), 1.0).b, 1.0));
    }

    #[test]
    fn saturation_zero_is_luma_grey_and_one_is_identity() {
        let c = Rgb::new(0.9, 0.2, 0.1);
        let grey = saturation(c, 0.0);
        assert!(close(grey.r, c.luma()) && close(grey.g, c.luma()) && close(grey.b, c.luma()));
        assert!(same(saturation(c, 1.0), c));
    }

    #[test]
    fn contrast_zero_is_mid_grey() {
        let g = contrast(Rgb::new(0.1, 0.7, 1.0), 0.0);
        assert!(close(g.r, 0.5) && close(g.g, 0.5) && close(g.b, 0.5));
        let c = Rgb::new(0.1, 0.7, 1.0);
        assert!(same(contrast(c, 1.0), c));
    }

    #[test]
    fn hue_rotation_keeps_grey_and_moves_red_to_green() {
        let grey = Rgb::new(0.5, 0.5, 0.5);
        assert!(same(hue_rotate(grey, 77.0), grey));
        assert!(same(hue_rotate(Rgb::new(0.2, 0.4, 0.6), 360.0), Rgb::new(0.2, 0.4, 0.6)));
        let green = hue_rotate(Rgb::new(1.0, 0.0, 0.0), 120.0);
        assert!(green.g > green.r && green.g > green.b);
    }

    #[test]
    fn warm_temperature_raises_red_and_lowers_blue() {
        let neutral = Rgb::new(0.5, 0.5, 0.5);
        assert!(same(temperature_tint(neutral, 0.0, 0.0), neutral));
        let warm = temperature_tint(neutral, 1.0, 0.0);
        assert!(warm.r > 0.5 && warm.b < 0.5 && close(warm.g, 0.5));
    }

    #[test]
    fn sepia_blends_from_identity_to_brown_tone() {
        let c = Rgb::new(0.5, 0.5, 0.5);
        assert!(same(sepia(c, 0.0), c));
        let full = sepia(c, 1.0);
        assert!(full.r > full.g && full.g > full.b);
    }

    #[test]
    fn invert_and_posterize_quantise_as_expected() {
        let inv = invert(Rgb::new(0.2, 0.0, 1.0));
        assert!(close(inv.r, 0.8) && close(inv.g, 1.0) && close(inv.b, 0.0));
        let two = posterize(Rgb::new(0.4, 0.6, 0.9), 2);
        assert!(close(two.r, 0.0) && close(two.g, 1.0) && close(two.b, 1.0));
        let four = posterize(Rgb::new(0.5, 0.0, 1.0), 4);
        assert!(close(four.r, 2.0 / 3.0) && close(four.g, 0.0) && close(four.b, 1.0));
    }

    #[test]
    fn vignette_is_full_at_centre_and_strength_at_corners() {
        assert!(close(vignette_factor(50.0, 50.0, 100.0, 100.0, 0.6), 1.0));
        assert!(close(vignette_factor(0.0, 0.0, 100.0, 100.0, 0.6), 0.4));
        let edge = vignette_factor(100.0, 50.0, 100.0, 100.0, 0.6);
        assert!(edge < 1.0 && edge > 0.4);
        assert!(close(vignette_factor(3.0, 3.0, 0.0, 0.0, 0.6), 1.0));
    }

    #[test]
    fn map_pixel_keeps_alpha_and_packs_channels() {
        // alpha 0x80, red 255, green 0, blue 0
        let px = (0x80u32 << 24) | 0xff;
        let out = map_pixel(px, invert);
        assert_eq!(out, (0x80u32 << 24) | (0xff << 16) | (0xff << 8));
    }
}
