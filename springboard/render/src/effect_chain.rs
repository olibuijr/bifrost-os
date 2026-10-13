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
    Gamma,
    HueRotate,
    TemperatureTint,
    Sepia,
    Invert,
    Posterize,
    Threshold,
    Vignette,
}

/// The names a filter can be made from, with their effect.
const NAMES: [(&str, Kind); 11] = [
    ("exposure", Kind::Exposure),
    ("saturation", Kind::Saturation),
    ("contrast", Kind::Contrast),
    ("gamma", Kind::Gamma),
    ("hueRotate", Kind::HueRotate),
    ("temperatureTint", Kind::TemperatureTint),
    ("sepia", Kind::Sepia),
    ("invert", Kind::Invert),
    ("posterize", Kind::Posterize),
    ("threshold", Kind::Threshold),
    ("vignette", Kind::Vignette),
];

/// The inputs of each effect, with their defaults. An effect with no inputs has an empty list.
fn defaults(kind: Kind) -> Vec<(&'static str, f32)> {
    match kind {
        Kind::Exposure => vec![("ev", 0.0)],
        Kind::Saturation => vec![("amount", 1.0)],
        Kind::Contrast => vec![("amount", 1.0)],
        Kind::Gamma => vec![("power", 1.0)],
        Kind::HueRotate => vec![("degrees", 0.0)],
        Kind::TemperatureTint => vec![("temperature", 0.0), ("tint", 0.0)],
        Kind::Sepia => vec![("amount", 1.0)],
        Kind::Invert => Vec::new(),
        Kind::Posterize => vec![("levels", 6.0)],
        Kind::Threshold => vec![("threshold", 0.5)],
        Kind::Vignette => vec![("strength", 0.5)],
    }
}

/// The range of one input: the least and greatest value it accepts, and its default. These follow the
/// minimum, maximum and default attribute keys that a parameter description carries.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InputRange {
    pub key: &'static str,
    pub min: f32,
    pub max: f32,
    pub default: f32,
}

/// The group a filter is listed under in a filter browser.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    /// Changes the tone or colour balance of the whole image: exposure, saturation, contrast, hue, temperature.
    ColorAdjustment,
    /// Maps each colour to another: sepia, invert, posterize.
    ColorEffect,
    /// Adds a visual treatment on top of the image: vignette.
    Stylize,
}

/// The range (min, max) of each input of an effect. The defaults come from `defaults`. Inputs with no range
/// are absent, so `input_range` returns `None` for them.
fn ranges(kind: Kind) -> Vec<(&'static str, f32, f32)> {
    match kind {
        Kind::Exposure => vec![("ev", -10.0, 10.0)],
        Kind::Saturation => vec![("amount", 0.0, 2.0)],
        Kind::Contrast => vec![("amount", 0.0, 4.0)],
        Kind::Gamma => vec![("power", 0.1, 3.0)],
        Kind::HueRotate => vec![("degrees", -180.0, 180.0)],
        Kind::TemperatureTint => vec![("temperature", -100.0, 100.0), ("tint", -100.0, 100.0)],
        Kind::Sepia => vec![("amount", 0.0, 1.0)],
        Kind::Invert => Vec::new(),
        Kind::Posterize => vec![("levels", 2.0, 32.0)],
        Kind::Threshold => vec![("threshold", 0.0, 1.0)],
        Kind::Vignette => vec![("strength", 0.0, 1.0)],
    }
}

fn category_of(kind: Kind) -> Category {
    match kind {
        Kind::Exposure | Kind::Saturation | Kind::Contrast | Kind::Gamma | Kind::HueRotate | Kind::TemperatureTint => {
            Category::ColorAdjustment
        }
        Kind::Sepia | Kind::Invert | Kind::Posterize | Kind::Threshold => Category::ColorEffect,
        Kind::Vignette => Category::Stylize,
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

    /// Sets input `key` to `value`, clamped to the input's range. Returns `false` when the filter has no
    /// such input or `value` is not finite; the input is then left as it was.
    pub fn set_clamped(&mut self, key: &str, value: f32) -> bool {
        if !value.is_finite() {
            return false;
        }
        match self.input_range(key) {
            Some(r) => self.set(key, value.clamp(r.min, r.max)),
            None => false,
        }
    }

    /// The range of input `key`, or `None` when the filter has no such input or the input has no range.
    pub fn input_range(&self, key: &str) -> Option<InputRange> {
        let (k, min, max) = ranges(self.kind).into_iter().find(|e| e.0 == key)?;
        let default = defaults(self.kind).into_iter().find(|e| e.0 == k)?.1;
        Some(InputRange { key: k, min, max, default })
    }

    /// The browser group this filter is listed under.
    pub fn category(&self) -> Category {
        category_of(self.kind)
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
            Kind::Gamma => {
                let power = input("power");
                colorfx::map_pixel(px, |c| colorfx::gamma(c, power))
            }
            Kind::Threshold => {
                let t = input("threshold");
                colorfx::map_pixel(px, |c| colorfx::threshold(c, t))
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

/// The names of every filter, in catalogue order.
pub fn filter_names() -> Vec<&'static str> {
    NAMES.iter().map(|e| e.0).collect()
}

/// The names of the filters in `category`, in catalogue order.
pub fn filters_in(category: Category) -> Vec<&'static str> {
    NAMES.iter().filter(|e| category_of(e.1) == category).map(|e| e.0).collect()
}

/// The names that contain `query`, ignoring case, in catalogue order. A blank query matches every name.
pub fn search(query: &str) -> Vec<&'static str> {
    let q = query.trim().to_lowercase();
    NAMES
        .iter()
        .filter(|e| q.is_empty() || e.0.to_lowercase().contains(q.as_str()))
        .map(|e| e.0)
        .collect()
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

    #[test]
    fn gamma_of_two_squares_mid_grey_and_keeps_black_and_white() {
        // 128 is 0.502 in 0..=1; squared it is about 0.252, which packs to 64. Black and white stay put.
        let mut f = Filter::named("gamma").unwrap();
        f.set("power", 2.0);
        let mut chain = Chain::new();
        chain.push(f);
        let out = chain.apply(&[0xff80_8080, 0xff00_0000, 0xffff_ffff], 3, 1).unwrap();
        assert_eq!(out, vec![0xff40_4040, 0xff00_0000, 0xffff_ffff]);
    }

    #[test]
    fn gamma_default_is_the_identity() {
        let px = vec![0xff32_64c8, 0xff80_8080];
        let mut chain = Chain::new();
        chain.push(Filter::named("gamma").unwrap());
        assert_eq!(chain.apply(&px, 2, 1).unwrap(), px);
    }

    #[test]
    fn threshold_turns_each_pixel_black_or_white_by_luma() {
        // Pure green has luma 0.587, above the default 0.5, so it becomes white. Dark grey 40 stays black.
        let mut chain = Chain::new();
        chain.push(Filter::named("threshold").unwrap());
        let out = chain.apply(&[0xff00_ff00, 0xff28_2828], 2, 1).unwrap();
        assert_eq!(out, vec![0xffff_ffff, 0xff00_0000]);
    }

    #[test]
    fn set_clamped_limits_the_value_to_the_input_range() {
        // Exposure accepts -10 to 10 stops. Values outside are clamped; NaN is refused and leaves the input alone.
        let mut f = Filter::named("exposure").unwrap();
        assert!(f.set_clamped("ev", 99.0));
        assert_eq!(f.get("ev"), Some(10.0));
        assert!(f.set_clamped("ev", -99.0));
        assert_eq!(f.get("ev"), Some(-10.0));
        assert!(!f.set_clamped("gain", 1.0));
        assert!(!f.set_clamped("ev", f32::NAN));
        assert_eq!(f.get("ev"), Some(-10.0));
    }

    #[test]
    fn input_range_reports_min_max_and_default() {
        // Saturation's amount runs from 0 (grey) to 2 (double saturation) and starts at 1 (unchanged).
        let sat = Filter::named("saturation").unwrap();
        assert_eq!(
            sat.input_range("amount"),
            Some(InputRange { key: "amount", min: 0.0, max: 2.0, default: 1.0 })
        );
        assert_eq!(sat.input_range("ev"), None);
        assert_eq!(Filter::named("invert").unwrap().input_range("amount"), None);
    }

    #[test]
    fn every_default_lies_inside_its_range() {
        for name in filter_names() {
            let f = Filter::named(name).unwrap();
            for key in f.input_keys() {
                let r = f.input_range(key).unwrap();
                assert!(r.min <= r.default && r.default <= r.max, "{name}.{key}");
            }
        }
    }

    #[test]
    fn categories_group_the_filters_as_the_browser_lists_them() {
        assert_eq!(Filter::named("exposure").unwrap().category(), Category::ColorAdjustment);
        assert_eq!(Filter::named("invert").unwrap().category(), Category::ColorEffect);
        assert_eq!(Filter::named("vignette").unwrap().category(), Category::Stylize);
        assert_eq!(filters_in(Category::ColorEffect), vec!["sepia", "invert", "posterize", "threshold"]);
        assert_eq!(filters_in(Category::Stylize), vec!["vignette"]);
    }

    #[test]
    fn search_matches_names_ignoring_case_and_blank_matches_all() {
        assert_eq!(search("TINT"), vec!["temperatureTint"]);
        assert_eq!(search("  "), filter_names());
        // Names with an "o": exposure, saturation, contrast, hueRotate, posterize and threshold.
        assert_eq!(search("o"), vec!["exposure", "saturation", "contrast", "hueRotate", "posterize", "threshold"]);
        assert!(search("zzz").is_empty());
    }
}
