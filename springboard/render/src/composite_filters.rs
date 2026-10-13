//! Composite, matrix and generator filters for pixel buffers, the Core Image categories that
//! `effect_chain` does not cover: the blend modes as two-input composite filters (source-over and
//! the twelve separable blend modes), a colour matrix (`ColorMatrix`), gamma, and a linear gradient
//! generator. Pixels are packed as the renderer packs them (red in the low byte, alpha in the top
//! byte). Composite filters take premultiplied pixels, as `blend::composite` does. Colour filters
//! read straight RGBA, as `colorfx` does. Everything is pure std code, so it runs without a GPU.

use crate::blend::{self, Mode};
use crate::colorfx::{self, Rgb};

/// A two-input composite filter: the foreground is put over the background under `mode`.
/// The names follow the Core Image composite and blend filters (`CISourceOverCompositing`,
/// `CIMultiplyBlendMode`, and so on) and are spelled in lower camel case here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Blend {
    SourceOver,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    HardLight,
    SoftLight,
    Difference,
    Exclusion,
}

/// The name of each blend filter, in catalogue order.
const BLENDS: [(&str, Blend); 12] = [
    ("sourceOver", Blend::SourceOver),
    ("multiply", Blend::Multiply),
    ("screen", Blend::Screen),
    ("overlay", Blend::Overlay),
    ("darken", Blend::Darken),
    ("lighten", Blend::Lighten),
    ("colorDodge", Blend::ColorDodge),
    ("colorBurn", Blend::ColorBurn),
    ("hardLight", Blend::HardLight),
    ("softLight", Blend::SoftLight),
    ("difference", Blend::Difference),
    ("exclusion", Blend::Exclusion),
];

impl Blend {
    /// The blend filter called `name`, or `None` for an unknown name.
    pub fn named(name: &str) -> Option<Blend> {
        BLENDS.iter().find(|e| e.0 == name).map(|e| e.1)
    }

    /// The name this filter is called by.
    pub fn name(self) -> &'static str {
        BLENDS.iter().find(|e| e.1 == self).map(|e| e.0).unwrap_or("sourceOver")
    }

    /// The separable blend formula this filter uses. `SourceOver` is plain `Mode::Normal`.
    pub fn mode(self) -> Mode {
        match self {
            Blend::SourceOver => Mode::Normal,
            Blend::Multiply => Mode::Multiply,
            Blend::Screen => Mode::Screen,
            Blend::Overlay => Mode::Overlay,
            Blend::Darken => Mode::Darken,
            Blend::Lighten => Mode::Lighten,
            Blend::ColorDodge => Mode::ColorDodge,
            Blend::ColorBurn => Mode::ColorBurn,
            Blend::HardLight => Mode::HardLight,
            Blend::SoftLight => Mode::SoftLight,
            Blend::Difference => Mode::Difference,
            Blend::Exclusion => Mode::Exclusion,
        }
    }
}

/// The names of every blend filter, in catalogue order.
pub fn blend_names() -> Vec<&'static str> {
    BLENDS.iter().map(|e| e.0).collect()
}

/// Puts `foreground` over `background` pixel by pixel under `filter`. Both buffers are premultiplied
/// and `w` by `h`. Returns `None` when either length is not `w * h`.
pub fn composite(foreground: &[u32], background: &[u32], w: usize, h: usize, filter: Blend) -> Option<Vec<u32>> {
    let n = w.checked_mul(h)?;
    if foreground.len() != n || background.len() != n {
        return None;
    }
    let mode = filter.mode();
    Some(foreground.iter().zip(background).map(|(&s, &d)| blend::composite(s, d, mode)).collect())
}

fn unpack(px: u32) -> [f32; 4] {
    let c = |shift: u32| ((px >> shift) & 0xff) as f32 / 255.0;
    [c(0), c(8), c(16), c(24)]
}

fn pack(v: [f32; 4]) -> u32 {
    let q = |x: f32| (x.clamp(0.0, 1.0) * 255.0 + 0.5) as u32;
    q(v[0]) | (q(v[1]) << 8) | (q(v[2]) << 16) | (q(v[3]) << 24)
}

/// A 4 by 5 colour matrix, as `CIColorMatrix` takes it: each output channel is a dot product of the
/// straight input (red, green, blue, alpha) with its row vector, plus its bias. Rows are `r`, `g`,
/// `b` and `a`; `bias` is the added vector in the same order. Results are clamped to 0..=1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColorMatrix {
    pub r: [f32; 4],
    pub g: [f32; 4],
    pub b: [f32; 4],
    pub a: [f32; 4],
    pub bias: [f32; 4],
}

impl ColorMatrix {
    /// The matrix that leaves every pixel unchanged.
    pub const fn identity() -> Self {
        ColorMatrix {
            r: [1.0, 0.0, 0.0, 0.0],
            g: [0.0, 1.0, 0.0, 0.0],
            b: [0.0, 0.0, 1.0, 0.0],
            a: [0.0, 0.0, 0.0, 1.0],
            bias: [0.0; 4],
        }
    }

    /// Applies the matrix to one straight RGBA pixel.
    pub fn apply_pixel(&self, px: u32) -> u32 {
        let v = unpack(px);
        let row = |m: &[f32; 4], bias: f32| m[0] * v[0] + m[1] * v[1] + m[2] * v[2] + m[3] * v[3] + bias;
        pack([
            row(&self.r, self.bias[0]),
            row(&self.g, self.bias[1]),
            row(&self.b, self.bias[2]),
            row(&self.a, self.bias[3]),
        ])
    }

    /// Applies the matrix to every pixel of a buffer.
    pub fn apply(&self, px: &[u32]) -> Vec<u32> {
        px.iter().map(|&p| self.apply_pixel(p)).collect()
    }
}

/// Raises each colour channel to `power` (`CIGammaAdjust`, whose default power is 1). Alpha is kept.
/// Powers at or below zero are treated as 1 so the result stays defined.
pub fn gamma(px: &[u32], power: f32) -> Vec<u32> {
    let p = if power.is_finite() && power > 0.0 { power } else { 1.0 };
    px.iter()
        .map(|&v| colorfx::map_pixel(v, |c| Rgb::new(c.r.powf(p), c.g.powf(p), c.b.powf(p)).clamped()))
        .collect()
}

/// A `w` by `h` linear gradient (`CILinearGradient`). The colour runs from `color0` at `p0` to
/// `color1` at `p1`, both in pixel coordinates, and is held at the end colours outside the segment.
/// Pixel centres are sampled, and the colours are straight packed RGBA. A zero-length segment
/// shows `color0` everywhere.
pub fn linear_gradient(w: usize, h: usize, p0: (f32, f32), p1: (f32, f32), color0: u32, color1: u32) -> Vec<u32> {
    let (dx, dy) = (p1.0 - p0.0, p1.1 - p0.1);
    let len2 = dx * dx + dy * dy;
    let (c0, c1) = (unpack(color0), unpack(color1));
    let mut out = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            let (px, py) = (x as f32 + 0.5 - p0.0, y as f32 + 0.5 - p0.1);
            let t = if len2 > 0.0 { ((px * dx + py * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
            let mut v = [0.0f32; 4];
            for c in 0..4 {
                v[c] = c0[c] + (c1[c] - c0[c]) * t;
            }
            out.push(pack(v));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn red(px: u32) -> i32 {
        (px & 0xff) as i32
    }

    #[test]
    fn blend_names_round_trip_and_unknown_names_are_refused() {
        for name in blend_names() {
            let b = Blend::named(name).unwrap();
            assert_eq!(b.name(), name);
        }
        assert_eq!(Blend::named("multiply"), Some(Blend::Multiply));
        assert_eq!(Blend::named("nonexistent"), None);
        assert_eq!(blend_names().len(), 12);
        assert_eq!(Blend::SourceOver.mode(), Mode::Normal);
    }

    #[test]
    fn multiply_of_mid_grey_over_white_keeps_the_grey() {
        // Premultiplied opaque pixels: 0x80 over 0xff under multiply gives about 0x80 again.
        let out = composite(&[0xff80_8080], &[0xffff_ffff], 1, 1, Blend::Multiply).unwrap();
        assert!((red(out[0]) - 128).abs() <= 1, "red {}", red(out[0]));
        assert_eq!(out[0] >> 24, 0xff);
    }

    #[test]
    fn screen_of_two_half_greys_is_three_quarters() {
        // 0.5 screened with 0.5 is 0.75, about 191 of 255.
        let out = composite(&[0xff80_8080], &[0xff80_8080], 1, 1, Blend::Screen).unwrap();
        assert!((red(out[0]) - 191).abs() <= 1, "red {}", red(out[0]));
    }

    #[test]
    fn transparent_foreground_leaves_the_background_for_every_blend() {
        let bg = [0xff12_3456, 0x80ab_cdef];
        for name in blend_names() {
            let b = Blend::named(name).unwrap();
            let out = composite(&[0, 0], &bg, 2, 1, b).unwrap();
            assert_eq!(out, bg.to_vec(), "{name}");
        }
    }

    #[test]
    fn composite_refuses_buffers_that_do_not_match_the_size() {
        assert_eq!(composite(&[0; 3], &[0; 4], 2, 2, Blend::SourceOver), None);
        assert_eq!(composite(&[0; 4], &[0; 3], 2, 2, Blend::SourceOver), None);
        assert!(composite(&[0; 4], &[0; 4], 2, 2, Blend::SourceOver).is_some());
    }

    #[test]
    fn identity_matrix_leaves_pixels_unchanged() {
        let m = ColorMatrix::identity();
        let px = vec![0xff11_2233, 0x8000_0000, 0x0000_00ff];
        assert_eq!(m.apply(&px), px);
    }

    #[test]
    fn matrix_swaps_red_and_blue() {
        // Input 0xff112233 has red 0x33, green 0x22, blue 0x11. Swapped red and blue give 0xff332211.
        let mut m = ColorMatrix::identity();
        m.r = [0.0, 0.0, 1.0, 0.0];
        m.b = [1.0, 0.0, 0.0, 0.0];
        assert_eq!(m.apply_pixel(0xff11_2233), 0xff33_2211);
    }

    #[test]
    fn matrix_bias_adds_and_results_clamp() {
        // A red bias of 1 on black sets red to full; a red bias of -1 on black clamps to 0.
        let mut m = ColorMatrix::identity();
        m.bias = [1.0, 0.0, 0.0, 0.0];
        assert_eq!(m.apply_pixel(0xff00_0000), 0xff00_00ff);
        m.bias = [-1.0, 0.0, 0.0, 0.0];
        assert_eq!(m.apply_pixel(0xff00_0000), 0xff00_0000);
    }

    #[test]
    fn gamma_of_one_is_identity_and_two_darkens_mid_tones() {
        let px = [0xff80_8080];
        assert_eq!(gamma(&px, 1.0), px.to_vec());
        assert_eq!(gamma(&px, 0.0), px.to_vec());
        let dark = gamma(&px, 2.0)[0];
        assert!(red(dark) < 0x80 && red(dark) > 0x30, "red {}", red(dark));
        assert_eq!(dark >> 24, 0xff);
    }

    #[test]
    fn linear_gradient_runs_between_its_end_colours_and_clamps_outside() {
        // A 2 by 1 image from x = 0 to x = 2. Pixel centres are at 0.5 and 1.5, so t is 0.25 and 0.75.
        let out = linear_gradient(2, 1, (0.0, 0.0), (2.0, 0.0), 0xff00_0000, 0xffff_ffff);
        assert!((red(out[0]) - 64).abs() <= 1);
        assert!((red(out[1]) - 191).abs() <= 1);
        // Outside the segment the end colours hold: t is clamped to 0 before p0 and to 1 after p1.
        let far = linear_gradient(1, 1, (10.0, 0.0), (20.0, 0.0), 0xff00_0000, 0xffff_ffff);
        assert_eq!(far[0], 0xff00_0000);
    }

    #[test]
    fn zero_length_gradient_shows_the_first_colour() {
        let out = linear_gradient(2, 2, (1.0, 1.0), (1.0, 1.0), 0xff10_2030, 0xffff_ffff);
        assert_eq!(out, vec![0xff10_2030; 4]);
    }
}
