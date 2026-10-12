//! Named pixel filters and a chain that runs them in order. A `Filter` is made by name with
//! `Filter::named`. Its inputs are read and set by key (`get`, `set`), and the names and defaults
//! are fixed here. A `Chain` applies its filters one after another to a buffer of packed pixels.
//! Each effect is a per-pixel colour function from `colorfx`, so a chain runs without a GPU. Pixels
//! are packed as the renderer packs them (red in the low byte, alpha in the top byte). Colour is
//! read as straight RGB and the alpha of each pixel is kept.

use crate::colorfx;

/// The effect a named filter applies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Exposure,
    Saturation,
    Contrast,
    HueRotate,
    TemperatureTint,
    Sepia,
    Invert,
    Posterize,
    Vignette,
}

/// The names a filter can be made from, with their effect.
const NAMES: [(&str, Kind); 9] = [
    ("exposure", Kind::Exposure),
    ("saturation", Kind::Saturation),
    ("contrast", Kind::Contrast),
    ("hueRotate", Kind::HueRotate),
    ("temperatureTint", Kind::TemperatureTint),
    ("sepia", Kind::Sepia),
    ("invert", Kind::Invert),
    ("posterize", Kind::Posterize),
    ("vignette", Kind::Vignette),
];

/// The inputs of each effect, with their defaults. An effect with no inputs has an empty list.
fn defaults(kind: Kind) -> Vec<(&'static str, f32)> {
    match kind {
        Kind::Exposure => vec![("ev", 0.0)],
        Kind::Saturation => vec![("amount", 1.0)],
        Kind::Contrast => vec![("amount", 1.0)],
        Kind::HueRotate => vec![("degrees", 0.0)],
        Kind::TemperatureTint => vec![("temperature", 0.0), ("tint", 0.0)],
        Kind::Sepia => vec![("amount", 1.0)],
        Kind::Invert => Vec::new(),
        Kind::Posterize => vec![("levels", 6.0)],
        Kind::Vignette => vec![("strength", 0.5)],
    }
}

/// A named filter with its inputs. Build one with `Filter::named`, then set inputs by key.
#[derive(Clone, Debug, PartialEq)]
pub struct Filter {
    name: &'static str,
    kind: Kind,
    inputs: Vec<(&'static str, f32)>,
}

impl Filter {
    /// The filter called `name`, with default inputs. Returns `None` for an unknown name.
    pub fn named(name: &str) -> Option<Filter> {
        let (name, kind) = NAMES.iter().find(|e| e.0 == name).copied()?;
        Some(Filter { name, kind, inputs: defaults(kind) })
    }

    /// The name the filter was made from.
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// The input keys this filter takes, in their fixed order.
    pub fn input_keys(&self) -> Vec<&'static str> {
        self.inputs.iter().map(|e| e.0).collect()
    }

    /// The value of input `key`, or `None` when the filter has no such input.
    pub fn get(&self, key: &str) -> Option<f32> {
        self.inputs.iter().find(|e| e.0 == key).map(|e| e.1)
    }

    /// Sets input `key`. Returns `false` when the filter has no such input.
    pub fn set(&mut self, key: &str, value: f32) -> bool {
        match self.inputs.iter_mut().find(|e| e.0 == key) {
            Some(slot) => {
                slot.1 = value;
                true
            }
            None => false,
        }
    }

    /// Applies the effect to the pixel at (x, y) of a `w` by `h` buffer.
    fn apply_pixel(&self, px: u32, x: usize, y: usize, w: usize, h: usize) -> u32 {
        let input = |key: &str| self.get(key).unwrap_or(0.0);
        match self.kind {
            Kind::Exposure => {
                let ev = input("ev");
                colorfx::map_pixel(px, |c| colorfx::exposure(c, ev))
            }
            Kind::Saturation => {
                let amount = input("amount");
                colorfx::map_pixel(px, |c| colorfx::saturation(c, amount))
            }
            Kind::Contrast => {
                let amount = input("amount");
                colorfx::map_pixel(px, |c| colorfx::contrast(c, amount))
            }
            Kind::HueRotate => {
                let degrees = input("degrees");
                colorfx::map_pixel(px, |c| colorfx::hue_rotate(c, degrees))
            }
            Kind::TemperatureTint => {
                let (temperature, tint) = (input("temperature"), input("tint"));
                colorfx::map_pixel(px, |c| colorfx::temperature_tint(c, temperature, tint))
            }
            Kind::Sepia => {
                let amount = input("amount");
                colorfx::map_pixel(px, |c| colorfx::sepia(c, amount))
            }
            Kind::Invert => colorfx::map_pixel(px, colorfx::invert),
            Kind::Posterize => {
                let levels = input("levels").round().max(0.0) as u32;
                colorfx::map_pixel(px, |c| colorfx::posterize(c, levels))
            }
            Kind::Vignette => {
                let strength = input("strength");
                let (fx, fy) = (x as f32, y as f32);
                let (fw, fh) = (w as f32, h as f32);
                colorfx::map_pixel(px, |c| colorfx::vignette(c, fx, fy, fw, fh, strength))
            }
        }
    }
}

/// An ordered list of filters. `apply` runs each one over the whole buffer in turn.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Chain {
    filters: Vec<Filter>,
}

impl Chain {
    /// An empty chain, which returns its input unchanged.
    pub fn new() -> Self {
        Chain::default()
    }

    /// Adds `filter` to the end of the chain.
    pub fn push(&mut self, filter: Filter) {
        self.filters.push(filter);
    }

    /// The number of filters in the chain.
    pub fn len(&self) -> usize {
        self.filters.len()
    }

    /// Whether the chain has no filters.
    pub fn is_empty(&self) -> bool {
        self.filters.is_empty()
    }

    /// Runs the chain over `px`, a `w` by `h` buffer of packed pixels. Returns `None` when the
    /// length of `px` is not `w * h`.
    pub fn apply(&self, px: &[u32], w: usize, h: usize) -> Option<Vec<u32>> {
        if w.checked_mul(h)? != px.len() {
            return None;
        }
        let mut out = px.to_vec();
        for filter in &self.filters {
            for (i, v) in out.iter_mut().enumerate() {
                *v = filter.apply_pixel(*v, i % w, i / w, w, h);
            }
        }
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_filter_has_defaults_and_rejects_unknown_names_and_keys() {
        let mut f = Filter::named("exposure").unwrap();
        assert_eq!(f.name(), "exposure");
        assert_eq!(f.get("ev"), Some(0.0));
        assert!(f.set("ev", 1.0));
        assert_eq!(f.get("ev"), Some(1.0));
        assert!(!f.set("gain", 1.0));
        assert_eq!(f.get("gain"), None);
        assert!(Filter::named("nonexistent").is_none());
        assert!(Filter::named("invert").unwrap().input_keys().is_empty());
        assert_eq!(Filter::named("temperatureTint").unwrap().input_keys(), vec!["temperature", "tint"]);
    }

    #[test]
    fn exposure_of_minus_one_stop_halves_each_channel_and_keeps_alpha() {
        // 200, 100 and 50 halve to 100, 50 and 25; alpha 255 is untouched.
        let mut f = Filter::named("exposure").unwrap();
        f.set("ev", -1.0);
        let mut chain = Chain::new();
        chain.push(f);
        let out = chain.apply(&[0xff32_64c8], 1, 1).unwrap();
        assert_eq!(out, vec![0xff19_3264]);
    }

    #[test]
    fn invert_reverses_each_channel_around_one() {
        // (0, 255, 10, 128) becomes (255, 0, 245, 128): 10 maps to 255 - 10 = 245.
        let mut chain = Chain::new();
        chain.push(Filter::named("invert").unwrap());
        let out = chain.apply(&[0x800a_ff00], 1, 1).unwrap();
        assert_eq!(out, vec![0x80f5_00ff]);
    }

    #[test]
    fn posterize_with_two_levels_snaps_each_channel_to_black_or_white() {
        // 63 is below half and goes to 0; 128 and 200 are above half and go to 255.
        let mut f = Filter::named("posterize").unwrap();
        f.set("levels", 2.0);
        let mut chain = Chain::new();
        chain.push(f);
        let out = chain.apply(&[0xffc8_803f], 1, 1).unwrap();
        assert_eq!(out, vec![0xffff_ff00]);
    }

    #[test]
    fn chain_runs_in_order_and_refuses_a_size_mismatch() {
        // Two inverts cancel out, so the buffer comes back as it went in.
        let px = vec![0xff0a_141e, 0xff32_3c46];
        let mut chain = Chain::new();
        chain.push(Filter::named("invert").unwrap());
        chain.push(Filter::named("invert").unwrap());
        assert_eq!(chain.len(), 2);
        assert_eq!(chain.apply(&px, 2, 1).unwrap(), px);
        assert_eq!(chain.apply(&[0; 3], 2, 2), None);
        assert_eq!(Chain::new().apply(&px, 2, 1).unwrap(), px);
    }

    #[test]
    fn vignette_darkens_the_corner_of_a_two_by_two_image_to_black() {
        // Strength 1 at the corner (0, 0) of a 2 by 2 image gives a factor of 0. The centre pixel
        // (1, 1) sits at the middle of the image and keeps its white.
        let mut f = Filter::named("vignette").unwrap();
        f.set("strength", 1.0);
        let mut chain = Chain::new();
        chain.push(f);
        let white = 0xffff_ffff;
        let out = chain.apply(&[white; 4], 2, 2).unwrap();
        assert_eq!(out[0], 0xff00_0000);
        assert_eq!(out[3], white);
    }
}
