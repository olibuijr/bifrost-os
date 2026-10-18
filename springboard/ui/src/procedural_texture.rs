//! Procedural textures for material inputs: a checkerboard and a two-colour gradient generator, and a sampler with
//! nearest or bilinear filtering and repeat or clamp addressing. Pixels are RGBA8, row by row from the top. The
//! generators fill plain vectors and own no GPU object; a renderer uploads the result.
//!
//! Texture space: `u` and `v` run from 0 to 1 across the whole image, with `v` increasing down the rows. A texel's
//! centre is at `(x + 0.5) / width`. Repeat wraps the coordinate into 0..1; clamp holds it at the nearest edge.

/// One pixel: red, green, blue and alpha, each 0 to 255.
pub type Texel = [u8; 4];

/// A width-by-height image. `texels` holds `width * height` entries, row by row from the top.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Texture {
    pub width: usize,
    pub height: usize,
    pub texels: Vec<Texel>,
}

impl Texture {
    /// The pixel at column `x` and row `y`. Panics when either is out of range, like any slice index.
    pub fn texel(&self, x: usize, y: usize) -> Texel {
        self.texels[y * self.width + x]
    }
}

/// How a coordinate outside 0..1 is handled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wrap {
    Repeat,
    Clamp,
}

/// Nearest texel, or the weighted average of the four texels around the sample point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter {
    Nearest,
    Linear,
}

/// A checkerboard with `cells` squares along each side. The cell index of a column is `x * cells / width` (integer
/// division), so each square covers an equal share of the image when `width` and `height` divide by `cells`. The
/// cell at the top left is `a`, and colours alternate across both axes. Returns `None` for an empty size or zero cells.
pub fn checkerboard(width: usize, height: usize, cells: usize, a: Texel, b: Texel) -> Option<Texture> {
    if width == 0 || height == 0 || cells == 0 {
        return None;
    }
    let mut texels = Vec::with_capacity(width * height);
    for y in 0..height {
        let cy = y * cells / height;
        for x in 0..width {
            let cx = x * cells / width;
            texels.push(if (cx + cy) % 2 == 0 { a } else { b });
        }
    }
    Some(Texture { width, height, texels })
}

/// A horizontal gradient from `a` at the left edge to `b` at the right edge. Each channel is `a + (b - a) * t`, where
/// `t` is `x / (width - 1)`, rounded to the nearest value. A one-pixel-wide image is all `a`. Returns `None` for an
/// empty size.
pub fn gradient(width: usize, height: usize, a: Texel, b: Texel) -> Option<Texture> {
    if width == 0 || height == 0 {
        return None;
    }
    let mut texels = Vec::with_capacity(width * height);
    for _ in 0..height {
        for x in 0..width {
            let t = if width > 1 { x as f32 / (width - 1) as f32 } else { 0.0 };
            let mut px = [0u8; 4];
            for c in 0..4 {
                let (lo, hi) = (f32::from(a[c]), f32::from(b[c]));
                px[c] = (lo + (hi - lo) * t).round() as u8;
            }
            texels.push(px);
        }
    }
    Some(Texture { width, height, texels })
}

/// A named gradient for a gradient wallpaper: the colours at the left and right edges.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GradientPreset {
    pub name: &'static str,
    pub from: Texel,
    pub to: Texel,
}

/// The gradient wallpapers the shell offers, in picker order.
pub const GRADIENT_PRESETS: [GradientPreset; 5] = [
    GradientPreset { name: "Sunrise", from: [255, 183, 77, 255], to: [236, 98, 129, 255] },
    GradientPreset { name: "Ocean", from: [79, 172, 254, 255], to: [0, 82, 212, 255] },
    GradientPreset { name: "Meadow", from: [168, 230, 163, 255], to: [46, 139, 87, 255] },
    GradientPreset { name: "Dusk", from: [142, 84, 233, 255], to: [40, 30, 90, 255] },
    GradientPreset { name: "Graphite", from: [120, 124, 130, 255], to: [28, 29, 33, 255] },
];

/// The preset with this name (case-sensitive, as the picker lists it), if there is one.
pub fn gradient_preset(name: &str) -> Option<&'static GradientPreset> {
    GRADIENT_PRESETS.iter().find(|p| p.name == name)
}

/// The wallpaper texture for a preset at `width` by `height`. Returns `None` for an empty size or an unknown name.
pub fn gradient_poster(name: &str, width: usize, height: usize) -> Option<Texture> {
    let preset = gradient_preset(name)?;
    gradient(width, height, preset.from, preset.to)
}

/// Maps a coordinate into the sampling range. Repeat takes the fractional part, so 1.25 becomes 0.25; clamp limits it
/// to 0..1. A NaN coordinate stays NaN and samples the first texel.
fn address(coord: f32, wrap: Wrap) -> f32 {
    match wrap {
        Wrap::Repeat => coord.rem_euclid(1.0),
        Wrap::Clamp => coord.clamp(0.0, 1.0),
    }
}

/// A texel index for a possibly out-of-range column or row: repeat wraps it, clamp holds it at the edge.
fn texel_index(i: i64, size: usize, wrap: Wrap) -> usize {
    let n = size as i64;
    match wrap {
        Wrap::Repeat => i.rem_euclid(n) as usize,
        Wrap::Clamp => i.clamp(0, n - 1) as usize,
    }
}

/// The colour at `(u, v)`. Nearest returns the texel that contains the point. Linear blends the four texels whose
/// centres surround it, weighting each by its distance, and rounds each channel. Addressing applies to the texel
/// indices, so a linear sample at the seam of a repeating texture blends with the far edge.
pub fn sample(texture: &Texture, u: f32, v: f32, wrap: Wrap, filter: Filter) -> Texel {
    let (w, h) = (texture.width, texture.height);
    match filter {
        Filter::Nearest => {
            let x = ((address(u, wrap) * w as f32).floor() as usize).min(w - 1);
            let y = ((address(v, wrap) * h as f32).floor() as usize).min(h - 1);
            texture.texel(x, y)
        }
        Filter::Linear => {
            let px = address(u, wrap) * w as f32 - 0.5;
            let py = address(v, wrap) * h as f32 - 0.5;
            let (fx, fy) = (px - px.floor(), py - py.floor());
            let (x0, y0) = (px.floor() as i64, py.floor() as i64);
            let xs = [texel_index(x0, w, wrap), texel_index(x0 + 1, w, wrap)];
            let ys = [texel_index(y0, h, wrap), texel_index(y0 + 1, h, wrap)];
            let mut out = [0u8; 4];
            for (c, channel) in out.iter_mut().enumerate() {
                let at = |xi: usize, yi: usize| f32::from(texture.texels[yi * w + xi][c]);
                let top = at(xs[0], ys[0]) + (at(xs[1], ys[0]) - at(xs[0], ys[0])) * fx;
                let bottom = at(xs[0], ys[1]) + (at(xs[1], ys[1]) - at(xs[0], ys[1])) * fx;
                *channel = (top + (bottom - top) * fy).round() as u8;
            }
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: Texel = [255, 0, 0, 255];
    const BLUE: Texel = [0, 0, 255, 255];
    const BLACK: Texel = [0, 0, 0, 255];
    const WHITE: Texel = [255, 255, 255, 255];

    #[test]
    fn checkerboard_alternates_on_both_axes() {
        let t = checkerboard(4, 4, 2, RED, BLUE).unwrap();
        assert_eq!(t.texel(0, 0), RED);
        assert_eq!(t.texel(2, 0), BLUE);
        assert_eq!(t.texel(0, 2), BLUE);
        assert_eq!(t.texel(2, 2), RED);
        // Two pixels per cell: pixel (1, 1) is still in the first cell.
        assert_eq!(t.texel(1, 1), RED);
        assert_eq!(t.texels.len(), 16);
    }

    #[test]
    fn checkerboard_rejects_empty_sizes_and_zero_cells() {
        assert!(checkerboard(0, 4, 2, RED, BLUE).is_none());
        assert!(checkerboard(4, 4, 0, RED, BLUE).is_none());
        assert!(gradient(4, 0, BLACK, WHITE).is_none());
    }

    #[test]
    fn gradient_rounds_each_channel_to_the_nearest_value() {
        let t = gradient(3, 1, BLACK, [255, 128, 0, 255]).unwrap();
        assert_eq!(t.texel(0, 0), BLACK);
        assert_eq!(t.texel(2, 0), [255, 128, 0, 255]);
        // Middle pixel: red 127.5 rounds to 128, green 64 is exact, blue stays 0.
        assert_eq!(t.texel(1, 0), [128, 64, 0, 255]);
    }

    #[test]
    fn nearest_repeat_wraps_and_clamp_holds_the_edge() {
        // Two-texel row: red then blue.
        let t = Texture { width: 2, height: 1, texels: vec![RED, BLUE] };
        assert_eq!(sample(&t, 0.75, 0.5, Wrap::Repeat, Filter::Nearest), BLUE);
        // 1.25 repeats to 0.25, which is the red texel.
        assert_eq!(sample(&t, 1.25, 0.5, Wrap::Repeat, Filter::Nearest), RED);
        // Clamp holds 1.25 at 1.0, which is the last texel.
        assert_eq!(sample(&t, 1.25, 0.5, Wrap::Clamp, Filter::Nearest), BLUE);
        assert_eq!(sample(&t, -0.25, 0.5, Wrap::Clamp, Filter::Nearest), RED);
    }

    #[test]
    fn linear_blends_between_texel_centres() {
        // Red channel 0 and 200. Centres sit at 0.25 and 0.75, so u = 0.5 is halfway: 100.
        let t = Texture { width: 2, height: 1, texels: vec![[0, 0, 0, 255], [200, 0, 0, 255]] };
        assert_eq!(sample(&t, 0.5, 0.5, Wrap::Clamp, Filter::Linear)[0], 100);
        // Left of the first centre, clamp holds the sample at the first texel: 0.
        assert_eq!(sample(&t, 0.0, 0.5, Wrap::Clamp, Filter::Linear)[0], 0);
    }

    #[test]
    fn linear_repeat_blends_across_the_seam() {
        // At u = 0 the sample sits before the first centre. Repeat takes the last texel (200) on the left, so the
        // blend with the first texel (0) is halfway: 100. Clamp would give 0 here.
        let t = Texture { width: 2, height: 1, texels: vec![[0, 0, 0, 255], [200, 0, 0, 255]] };
        assert_eq!(sample(&t, 0.0, 0.5, Wrap::Repeat, Filter::Linear)[0], 100);
        assert_eq!(sample(&t, 0.0, 0.5, Wrap::Clamp, Filter::Linear)[0], 0);
    }

    #[test]
    fn linear_blends_vertically_as_well() {
        // A 1x2 column with channel 0 at 0 (top) and 100 (bottom). v = 0.5 sits halfway between the two centres: 50.
        let t = Texture { width: 1, height: 2, texels: vec![[0, 0, 0, 255], [100, 0, 0, 255]] };
        assert_eq!(sample(&t, 0.5, 0.5, Wrap::Clamp, Filter::Linear)[0], 50);
        assert_eq!(sample(&t, 0.5, 0.25, Wrap::Clamp, Filter::Nearest), [0, 0, 0, 255]);
    }

    #[test]
    fn presets_have_unique_names_and_build_a_poster() {
        for (i, p) in GRADIENT_PRESETS.iter().enumerate() {
            assert!(GRADIENT_PRESETS[..i].iter().all(|q| q.name != p.name), "{}", p.name);
            let poster = gradient_poster(p.name, 8, 4).unwrap();
            assert_eq!(poster.texel(0, 0), p.from, "{}", p.name);
            assert_eq!(poster.texel(7, 3), p.to, "{}", p.name);
        }
    }

    #[test]
    fn a_preset_is_found_by_its_exact_name() {
        assert_eq!(gradient_preset("Ocean").map(|p| p.name), Some("Ocean"));
        assert!(gradient_preset("ocean").is_none(), "names are case-sensitive");
        assert!(gradient_poster("Nope", 8, 4).is_none());
        assert!(gradient_poster("Dusk", 0, 4).is_none(), "an empty size has no poster");
    }
}
