//! Named pixel filters, a filter catalogue and a chain that runs filters in order. A `Filter` is made
//! by name with `Filter::named`. Its inputs are read and set by key (`get`, `set`, `set_clamped`),
//! and each input has a range and an identity value (`attribute`, `attributes`). The catalogue groups
//! the names into categories (`filter_names`) and searches them by name or title (`search`), the way
//! a filter browser lists them. A `Chain` applies its filters one after another to a buffer of packed
//! pixels. Each effect is a per-pixel colour function from `colorfx`, so a chain runs without a GPU.
//! Pixels are packed as the renderer packs them (red in the low byte, alpha in the top byte). Colour
//! is read as straight RGB and the alpha of each pixel is kept.

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

/// The names a filter can be made from, with their effect, in catalogue order.
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

/// The groups the catalogue sorts the filters into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    ColorAdjustment,
    ColorEffect,
}

impl Category {
    /// The heading the catalogue shows for the category.
    pub fn title(self) -> &'static str {
        match self {
            Category::ColorAdjustment => "Color Adjustment Filters",
            Category::ColorEffect => "Color Effect Filters",
        }
    }
}

/// One input of a filter: its key, its default, the range a browser slider covers, and the value
/// that leaves pixels unchanged (`None` when no single value does).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InputAttribute {
    pub key: &'static str,
    pub default: f32,
    pub min: f32,
    pub max: f32,
    pub identity: Option<f32>,
}

impl Kind {
    fn category(self) -> Category {
        match self {
            Kind::Exposure | Kind::Saturation | Kind::Contrast | Kind::HueRotate | Kind::TemperatureTint => {
                Category::ColorAdjustment
            }
            Kind::Sepia | Kind::Invert | Kind::Posterize | Kind::Vignette => Category::ColorEffect,
        }
    }

    /// The name shown to people for the effect.
    fn title(self) -> &'static str {
        match self {
            Kind::Exposure => "Exposure Adjust",
            Kind::Saturation => "Saturation",
            Kind::Contrast => "Contrast",
            Kind::HueRotate => "Hue Rotate",
            Kind::TemperatureTint => "Temperature and Tint",
            Kind::Sepia => "Sepia Tone",
            Kind::Invert => "Invert",
            Kind::Posterize => "Posterize",
            Kind::Vignette => "Vignette",
        }
    }

    /// The inputs of the effect with their range and identity. An effect with no inputs has an empty list.
    fn specs(self) -> Vec<InputAttribute> {
        let a = |key: &'static str, default: f32, min: f32, max: f32, identity: Option<f32>| InputAttribute {
            key,
            default,
            min,
            max,
            identity,
        };
        match self {
            Kind::Exposure => vec![a("ev", 0.0, -10.0, 10.0, Some(0.0))],
            Kind::Saturation => vec![a("amount", 1.0, 0.0, 2.0, Some(1.0))],
            Kind::Contrast => vec![a("amount", 1.0, 0.0, 4.0, Some(1.0))],
            Kind::HueRotate => vec![a("degrees", 0.0, -180.0, 180.0, Some(0.0))],
            Kind::TemperatureTint => vec![
                a("temperature", 0.0, -100.0, 100.0, Some(0.0)),
                a("tint", 0.0, -100.0, 100.0, Some(0.0)),
            ],
            Kind::Sepia => vec![a("amount", 1.0, 0.0, 1.0, Some(0.0))],
            Kind::Invert => Vec::new(),
            Kind::Posterize => vec![a("levels", 6.0, 2.0, 256.0, None)],
            Kind::Vignette => vec![a("strength", 0.5, 0.0, 1.0, Some(0.0))],
        }
    }

    fn apply_pixel_with(self, inputs: &[(&'static str, f32)], px: u32, x: usize, y: usize, w: usize, h: usize) -> u32 {
        let input = |key: &str| inputs.iter().find(|e| e.0 == key).map(|e| e.1).unwrap_or(0.0);
        match self {
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

/// The names of the filters in `category`, or of every filter when `category` is `None`, in catalogue order.
pub fn filter_names(category: Option<Category>) -> Vec<&'static str> {
    NAMES
        .iter()
        .filter(|e| category.map_or(true, |c| e.1.category() == c))
        .map(|e| e.0)
        .collect()
}

/// The display name of the filter called `name`, or `None` for an unknown name.
pub fn title_of(name: &str) -> Option<&'static str> {
    NAMES.iter().find(|e| e.0 == name).map(|e| e.1.title())
}

/// The filters whose name or display name contains `query`, ignoring case, in catalogue order.
/// An empty query lists every filter.
pub fn search(query: &str) -> Vec<&'static str> {
    let q = query.to_lowercase();
    NAMES
        .iter()
        .filter_map(|&(name, kind)| {
            let hit = name.to_lowercase().contains(&q) || kind.title().to_lowercase().contains(&q);
            hit.then_some(name)
        })
        .collect()
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
        let inputs = kind.specs().iter().map(|a| (a.key, a.default)).collect();
        Some(Filter { name, kind, inputs })
    }

    /// The name the filter was made from.
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// The display name of the filter.
    pub fn title(&self) -> &'static str {
        self.kind.title()
    }

    /// The category the filter is listed under.
    pub fn category(&self) -> Category {
        self.kind.category()
    }

    /// The input keys this filter takes, in their fixed order.
    pub fn input_keys(&self) -> Vec<&'static str> {
        self.inputs.iter().map(|e| e.0).collect()
    }

    /// The range, default and identity of input `key`, or `None` when the filter has no such input.
    pub fn attribute(&self, key: &str) -> Option<InputAttribute> {
        self.kind.specs().into_iter().find(|a| a.key == key)
    }

    /// The range, default and identity of every input, in the order of `input_keys`.
    pub fn attributes(&self) -> Vec<InputAttribute> {
        self.kind.specs()
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

    /// Sets input `key` to `value` limited to the input's range. Returns the stored value, or `None`
    /// when the filter has no such input.
    pub fn set_clamped(&mut self, key: &str, value: f32) -> Option<f32> {
        let attr = self.attribute(key)?;
        let stored = value.max(attr.min).min(attr.max);
        self.set(key, stored);
        Some(stored)
    }

    /// Puts every input back to its default.
    pub fn reset_inputs(&mut self) {
        self.inputs = self.kind.specs().iter().map(|a| (a.key, a.default)).collect();
    }

    /// Applies the effect to the pixel at (x, y) of a `w` by `h` buffer.
    fn apply_pixel(&self, px: u32, x: usize, y: usize, w: usize, h: usize) -> u32 {
        self.kind.apply_pixel_with(&self.inputs, px, x, y, w, h)
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

    #[test]
    fn catalogue_sorts_filters_into_their_categories_in_order() {
        assert_eq!(
            filter_names(Some(Category::ColorAdjustment)),
            vec!["exposure", "saturation", "contrast", "hueRotate", "temperatureTint"]
        );
        assert_eq!(filter_names(Some(Category::ColorEffect)), vec!["sepia", "invert", "posterize", "vignette"]);
        assert_eq!(filter_names(None).len(), 9);
        assert_eq!(Filter::named("exposure").unwrap().category(), Category::ColorAdjustment);
        assert_eq!(Category::ColorEffect.title(), "Color Effect Filters");
    }

    #[test]
    fn search_matches_name_or_display_name_ignoring_case() {
        assert_eq!(search("tone"), vec!["sepia"]);
        assert_eq!(search("ADJUST"), vec!["exposure"]);
        assert_eq!(search("").len(), 9);
        assert!(search("zzz").is_empty());
        assert_eq!(title_of("hueRotate"), Some("Hue Rotate"));
        assert_eq!(title_of("gain"), None);
    }

    #[test]
    fn attributes_report_range_default_and_identity() {
        let amount = Filter::named("saturation").unwrap().attribute("amount").unwrap();
        assert_eq!(
            amount,
            InputAttribute { key: "amount", default: 1.0, min: 0.0, max: 2.0, identity: Some(1.0) }
        );
        // Posterize has no single value that leaves pixels unchanged.
        assert_eq!(Filter::named("posterize").unwrap().attribute("levels").unwrap().identity, None);
        assert_eq!(Filter::named("saturation").unwrap().attribute("gain"), None);
        assert!(Filter::named("invert").unwrap().attributes().is_empty());
    }

    #[test]
    fn set_clamped_stops_at_the_range_and_refuses_unknown_keys() {
        let mut f = Filter::named("exposure").unwrap();
        assert_eq!(f.set_clamped("ev", 99.0), Some(10.0));
        assert_eq!(f.get("ev"), Some(10.0));
        assert_eq!(f.set_clamped("ev", -20.0), Some(-10.0));
        assert_eq!(f.set_clamped("ev", 3.5), Some(3.5));
        assert_eq!(f.set_clamped("gain", 1.0), None);
    }

    #[test]
    fn reset_inputs_restores_the_defaults() {
        let mut tt = Filter::named("temperatureTint").unwrap();
        tt.set("temperature", 40.0);
        tt.set("tint", -20.0);
        tt.reset_inputs();
        assert_eq!(tt.get("temperature"), Some(0.0));
        assert_eq!(tt.get("tint"), Some(0.0));
        // Sepia defaults to a full amount, which is not its identity of 0.
        let mut sepia = Filter::named("sepia").unwrap();
        sepia.set("amount", 0.0);
        sepia.reset_inputs();
        assert_eq!(sepia.get("amount"), Some(1.0));
    }
}
