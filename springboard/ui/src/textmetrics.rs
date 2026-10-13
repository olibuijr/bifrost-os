//! Where text widths come from. Layout never measures text itself: it asks a [`GlyphSource`] for the advance of each
//! character and for the vertical metrics of a font. The simple source in the SDK measures with the UI font; a source
//! backed by a shaping engine (kerning, ligatures, contextual forms) replaces it without touching the layout code.
//! `glyph_advances` is the single function the layout calls for widths.

/// The font a run asks for. `size` is in points and is never zero in a laid-out run (the caller sets it from its text
/// style). `weight` is 100 to 900 (400 regular, 700 bold).
#[derive(Clone, Debug, PartialEq)]
pub struct FontSpec {
    pub family: Option<String>,
    pub size: f32,
    pub weight: u16,
    pub italic: bool,
}

impl Default for FontSpec {
    fn default() -> Self {
        FontSpec { family: None, size: 0.0, weight: 400, italic: false }
    }
}

impl FontSpec {
    pub fn new(size: f32) -> Self {
        FontSpec { size, ..Default::default() }
    }

    pub fn with_weight(mut self, weight: u16) -> Self {
        self.weight = weight;
        self
    }

    pub fn with_family(mut self, family: &str) -> Self {
        self.family = Some(family.to_string());
        self
    }
}

/// Vertical metrics of a font at one size, in points. Ascent, descent, leading and the underline thickness are
/// positive; the underline position is the distance below the baseline.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FontMetrics {
    pub ascent: f32,
    pub descent: f32,
    pub leading: f32,
    pub underline_position: f32,
    pub underline_thickness: f32,
}

/// The provider of glyph advances and font metrics.
pub trait GlyphSource {
    /// The advance, in points, of each character of `text`, in order. A shaping source may fold a cluster into the
    /// advance of its first character and give the rest zero.
    fn advances(&self, font: &FontSpec, text: &str) -> Vec<f32>;
    fn metrics(&self, font: &FontSpec) -> FontMetrics;
}

/// The advance of each character of `text` in `font`. The one call layout makes for widths: the result always has one
/// entry per character, whatever the source returned.
pub fn glyph_advances(source: &dyn GlyphSource, font: &FontSpec, text: &str) -> Vec<f32> {
    let n = text.chars().count();
    let mut v = source.advances(font, text);
    v.resize(n, 0.0);
    v
}

/// A source built from two closures: the width of a string, and the metrics of a font. Advances are the width of each
/// character on its own.
pub struct FnSource {
    pub measure: Box<dyn Fn(&FontSpec, &str) -> f32>,
    pub metrics: Box<dyn Fn(&FontSpec) -> FontMetrics>,
}

impl GlyphSource for FnSource {
    fn advances(&self, font: &FontSpec, text: &str) -> Vec<f32> {
        let mut buf = [0u8; 4];
        text.chars().map(|c| (self.measure)(font, c.encode_utf8(&mut buf))).collect()
    }

    fn metrics(&self, font: &FontSpec) -> FontMetrics {
        (self.metrics)(font)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Short;
    impl GlyphSource for Short {
        fn advances(&self, _: &FontSpec, _: &str) -> Vec<f32> {
            vec![1.0]
        }
        fn metrics(&self, _: &FontSpec) -> FontMetrics {
            FontMetrics::default()
        }
    }

    #[test]
    fn advances_always_match_the_character_count() {
        assert_eq!(glyph_advances(&Short, &FontSpec::new(10.0), "abc"), vec![1.0, 0.0, 0.0]);
        assert_eq!(glyph_advances(&Short, &FontSpec::new(10.0), ""), Vec::<f32>::new());
    }

    #[test]
    fn fn_source_measures_each_character() {
        let s = FnSource {
            measure: Box::new(|f, t| f.size * t.chars().count() as f32),
            metrics: Box::new(|f| FontMetrics { ascent: f.size, ..Default::default() }),
        };
        assert_eq!(s.advances(&FontSpec::new(2.0), "ab"), vec![2.0, 2.0]);
        assert_eq!(s.metrics(&FontSpec::new(3.0)).ascent, 3.0);
    }
}
