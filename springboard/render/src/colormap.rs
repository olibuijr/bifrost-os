//! Colour maps and palettes for packed pixels. A `ColorMap` is a one-dimensional gradient: a colour's
//! luma picks a position along it, and the gradient's entries are interpolated there, so a grey scale
//! can be tinted or re-coloured. A `Palette` snaps each colour to its nearest entry, either in RGB or,
//! for `perceptual` palettes, in CIE L*a*b* where equal steps look equally large. Both work on the
//! 0xAARRGGBB pixels the renderer uses and keep the alpha byte unchanged.

use crate::colorfx::{map_pixel, Rgb};
use crate::colorlab::{delta_e, Lab};

/// A one-dimensional colour gradient. Position 0 is the first entry, position 1 the last.
#[derive(Clone, Debug, PartialEq)]
pub struct ColorMap {
    entries: Vec<Rgb>,
}

impl ColorMap {
    /// A gradient from its entries, in order. `None` when there are none.
    pub fn new(entries: Vec<Rgb>) -> Option<Self> {
        if entries.is_empty() {
            return None;
        }
        Some(ColorMap { entries })
    }

    /// Number of gradient entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// The gradient colour at `t` in 0..=1, linearly interpolated between the two nearest entries.
    /// `t` is clamped.
    pub fn at(&self, t: f32) -> Rgb {
        let n = self.entries.len();
        if n == 1 {
            return self.entries[0];
        }
        let f = t.clamp(0.0, 1.0) * (n - 1) as f32;
        let i = (f.floor() as usize).min(n - 2);
        self.entries[i].mix(self.entries[i + 1], f - i as f32)
    }

    /// The colour that `c` maps to: its Rec. 601 luma indexes the gradient.
    pub fn map(&self, c: Rgb) -> Rgb {
        self.at(c.luma())
    }

    /// `map` applied to a packed 0xAARRGGBB pixel, alpha kept.
    pub fn map_pixel(&self, px: u32) -> u32 {
        map_pixel(px, |c| self.map(c))
    }
}

/// A fixed set of colours that every input colour is snapped to.
#[derive(Clone, Debug, PartialEq)]
pub struct Palette {
    colours: Vec<Rgb>,
    labs: Vec<Lab>,
    perceptual: bool,
}

impl Palette {
    /// A palette from its colours. With `perceptual` the nearest entry is judged in L*a*b*, otherwise
    /// in RGB. `None` when there are no colours.
    pub fn new(colours: Vec<Rgb>, perceptual: bool) -> Option<Self> {
        if colours.is_empty() {
            return None;
        }
        let labs = colours.iter().map(|&c| Lab::from_rgb(c)).collect();
        Some(Palette { colours, labs, perceptual })
    }

    /// Number of palette colours.
    pub fn len(&self) -> usize {
        self.colours.len()
    }

    /// The palette colour nearest to `c`. A tie keeps the earlier entry.
    pub fn nearest(&self, c: Rgb) -> Rgb {
        let mut best = 0;
        let mut best_d = f64::INFINITY;
        if self.perceptual {
            let lab = Lab::from_rgb(c);
            for (i, p) in self.labs.iter().enumerate() {
                let d = delta_e(lab, *p);
                if d < best_d {
                    best = i;
                    best_d = d;
                }
            }
        } else {
            for (i, p) in self.colours.iter().enumerate() {
                let d = [(c.r - p.r) as f64, (c.g - p.g) as f64, (c.b - p.b) as f64].iter().map(|v| v * v).sum::<f64>();
                if d < best_d {
                    best = i;
                    best_d = d;
                }
            }
        }
        self.colours[best]
    }

    /// `nearest` applied to a packed 0xAARRGGBB pixel, alpha kept.
    pub fn palettize_pixel(&self, px: u32) -> u32 {
        map_pixel(px, |c| self.nearest(c))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn empty_gradient_and_palette_are_rejected() {
        assert!(ColorMap::new(Vec::new()).is_none());
        assert!(Palette::new(Vec::new(), false).is_none());
        assert!(ColorMap::new(vec![Rgb::new(0.0, 0.0, 0.0)]).is_some());
    }

    #[test]
    fn gradient_endpoints_and_midpoint_interpolate() {
        let map = ColorMap::new(vec![Rgb::new(0.0, 0.0, 0.0), Rgb::new(1.0, 0.5, 0.0), Rgb::new(1.0, 1.0, 1.0)]).unwrap();
        assert_eq!(map.at(0.0), Rgb::new(0.0, 0.0, 0.0));
        assert_eq!(map.at(1.0), Rgb::new(1.0, 1.0, 1.0));
        let mid = map.at(0.25);
        assert!(close(mid.r, 0.5) && close(mid.g, 0.25) && close(mid.b, 0.0), "{mid:?}");
        // Positions outside 0..=1 clamp to the ends.
        assert_eq!(map.at(-3.0), map.at(0.0));
        assert_eq!(map.at(9.0), map.at(1.0));
    }

    #[test]
    fn map_reads_luma_as_the_gradient_position() {
        // A black-to-white gradient maps any colour to its own luma, so grey stays grey.
        let ramp = ColorMap::new(vec![Rgb::new(0.0, 0.0, 0.0), Rgb::new(1.0, 1.0, 1.0)]).unwrap();
        let out = ramp.map(Rgb::new(0.5, 0.5, 0.5));
        assert!(close(out.r, 0.5) && close(out.g, 0.5) && close(out.b, 0.5));
    }

    #[test]
    fn map_pixel_recolours_a_grey_scale_and_keeps_alpha() {
        // Dark maps to blue, light to yellow.
        let tint = ColorMap::new(vec![Rgb::new(0.0, 0.0, 1.0), Rgb::new(1.0, 1.0, 0.0)]).unwrap();
        let px = 0x80_80_80_80u32;
        let out = tint.map_pixel(px);
        assert_eq!(out >> 24, 0x80, "alpha byte must not change");
        let r = (out & 0xff) as i32;
        let b = ((out >> 16) & 0xff) as i32;
        assert!((r - 128).abs() <= 2 && (b - 128).abs() <= 2, "{out:#010x}");
    }

    #[test]
    fn rgb_palette_snaps_to_the_nearest_entry() {
        let pal = Palette::new(vec![Rgb::new(0.0, 0.0, 0.0), Rgb::new(1.0, 0.0, 0.0), Rgb::new(1.0, 1.0, 1.0)], false).unwrap();
        assert_eq!(pal.len(), 3);
        assert_eq!(pal.nearest(Rgb::new(0.9, 0.1, 0.0)), Rgb::new(1.0, 0.0, 0.0));
        assert_eq!(pal.nearest(Rgb::new(0.1, 0.1, 0.05)), Rgb::new(0.0, 0.0, 0.0));
        assert_eq!(pal.nearest(Rgb::new(0.8, 0.9, 0.95)), Rgb::new(1.0, 1.0, 1.0));
    }

    #[test]
    fn perceptual_palette_picks_a_colour_it_is_given_exactly() {
        let entries = vec![Rgb::new(1.0, 0.0, 0.0), Rgb::new(0.0, 0.0, 1.0), Rgb::new(0.0, 0.6, 0.0)];
        let pal = Palette::new(entries.clone(), true).unwrap();
        for c in entries {
            assert_eq!(pal.nearest(c), c);
        }
        // A dark red is closer to pure red than to pure blue in Lab as well.
        assert_eq!(pal.nearest(Rgb::new(0.6, 0.05, 0.05)), Rgb::new(1.0, 0.0, 0.0));
    }

    #[test]
    fn palettize_pixel_keeps_alpha_and_snaps_colour() {
        let pal = Palette::new(vec![Rgb::new(0.0, 0.0, 0.0), Rgb::new(1.0, 1.0, 1.0)], false).unwrap();
        let out = pal.palettize_pixel(0xc0_f0_f0_f0);
        assert_eq!(out, 0xc0_ff_ff_ff);
    }
}
