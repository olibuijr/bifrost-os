//! Font metrics for math layout: per-glyph sizes, stretchy variants and the MATH constants.
//!
//! All values here are in em (1.0 = the text font size). The layout multiplies by the
//! current style size. A font that carries an OpenType `MATH` table supplies its own
//! [`MathConstants`] through [`MathConstants::from_math_table`]; otherwise
//! [`MathConstants::DEFAULT`] (TeX-derived values) is used.

/// Measurements of one glyph, in em.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlyphMetrics {
    pub ch: char,
    pub advance: f32,
    /// Height above the baseline.
    pub ascent: f32,
    /// Depth below the baseline (positive).
    pub descent: f32,
}

impl GlyphMetrics {
    pub fn total(&self) -> f32 {
        self.ascent + self.descent
    }
}

/// Supplies glyph measurements to the layout.
pub trait FontMetrics {
    /// Metrics of `ch`. Fonts without the glyph should return an estimate, never panic.
    fn glyph(&self, ch: char) -> GlyphMetrics;

    /// Pre-built stretchy variants of `ch`, smallest first (the MATH `MathVariants`
    /// table). Empty when the font has none; the layout then scales the base glyph.
    fn variants(&self, _ch: char) -> Vec<GlyphMetrics> {
        Vec::new()
    }

    /// The MATH table constants, when the font has them.
    fn constants(&self) -> Option<MathConstants> {
        None
    }
}

/// The subset of OpenType MATH constants the layout uses, in em.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MathConstants {
    /// Script size as a fraction of the text size (MATH: scriptPercentScaleDown).
    pub script_scale: f32,
    /// Script-script size as a fraction of the text size.
    pub script_script_scale: f32,
    pub axis_height: f32,
    pub fraction_rule: f32,
    pub num_shift_display: f32,
    pub num_shift_text: f32,
    pub den_shift_display: f32,
    pub den_shift_text: f32,
    pub num_gap_display: f32,
    pub num_gap_text: f32,
    pub den_gap_display: f32,
    pub den_gap_text: f32,
    pub sup_shift: f32,
    pub sup_shift_cramped: f32,
    pub sub_shift: f32,
    pub sub_top_max: f32,
    pub sup_bottom_min: f32,
    pub sub_sup_gap_min: f32,
    pub sup_baseline_drop: f32,
    pub sub_baseline_drop: f32,
    pub space_after_script: f32,
    pub upper_limit_gap: f32,
    pub lower_limit_gap: f32,
    pub radical_gap: f32,
    pub radical_display_gap: f32,
    pub radical_rule: f32,
    pub radical_extra_ascender: f32,
    /// Fraction of the radicand height at which the index sits (MATH: radicalDegreeBottomRaisePercent).
    pub radical_degree_raise: f32,
    /// Smallest height of a display-style large operator.
    pub display_op_min_height: f32,
}

impl MathConstants {
    /// TeX-derived defaults for fonts without a MATH table.
    pub const DEFAULT: MathConstants = MathConstants {
        script_scale: 0.7,
        script_script_scale: 0.5,
        axis_height: 0.25,
        fraction_rule: 0.04,
        num_shift_display: 0.677,
        num_shift_text: 0.394,
        den_shift_display: 0.686,
        den_shift_text: 0.345,
        num_gap_display: 0.12,
        num_gap_text: 0.04,
        den_gap_display: 0.12,
        den_gap_text: 0.04,
        sup_shift: 0.413,
        sup_shift_cramped: 0.363,
        sub_shift: 0.15,
        sub_top_max: 0.25,
        sup_bottom_min: 0.1,
        sub_sup_gap_min: 0.16,
        sup_baseline_drop: 0.386,
        sub_baseline_drop: 0.05,
        space_after_script: 0.05,
        upper_limit_gap: 0.111,
        lower_limit_gap: 0.111,
        radical_gap: 0.05,
        radical_display_gap: 0.25,
        radical_rule: 0.04,
        radical_extra_ascender: 0.04,
        radical_degree_raise: 0.6,
        display_op_min_height: 1.3,
    };

    /// Reads the constants from the raw bytes of an OpenType `MATH` table.
    ///
    /// `upem` is the font's units-per-em (from `head`). Returns `None` when the table is
    /// truncated or its MathConstants subtable is missing. Records are read in the order
    /// the OpenType specification gives them, starting at MathConstants record 4.
    pub fn from_math_table(table: &[u8], upem: u16) -> Option<MathConstants> {
        if upem == 0 {
            return None;
        }
        let base = read_u16(table, 4)? as usize;
        if base == 0 {
            return None;
        }
        let c = table.get(base..)?;
        let scale = |v: i16| v as f32 / 100.0;
        // Plain fields come first: two int16 percentages, then two uint16 heights.
        let script_scale = scale(read_i16(c, 0)?);
        let script_script_scale = scale(read_i16(c, 2)?);
        let display_op_min = read_u16(c, 6)? as f32 / upem as f32;
        // MathValueRecord k (k >= 4) is 4 bytes at 8 + 4 * (k - 4): value, then device offset.
        let rec = |k: usize| -> Option<f32> {
            let off = 4 * k - 8;
            Some(read_i16(c, off)? as f32 / upem as f32)
        };
        let d = MathConstants::DEFAULT;
        // Records we do not read keep their defaults only if the table lacks them: a
        // missing record (out of range) is an error, so the whole table is rejected.
        Some(MathConstants {
            script_scale,
            script_script_scale,
            axis_height: rec(5)?,
            fraction_rule: rec(38)?,
            num_shift_display: rec(33)?,
            num_shift_text: rec(32)?,
            den_shift_display: rec(35)?,
            den_shift_text: rec(34)?,
            num_gap_display: rec(37)?,
            num_gap_text: rec(36)?,
            den_gap_display: rec(40)?,
            den_gap_text: rec(39)?,
            sup_shift: rec(11)?,
            sup_shift_cramped: rec(12)?,
            sub_shift: rec(8)?,
            sub_top_max: rec(9)?,
            sup_bottom_min: rec(13)?,
            sub_sup_gap_min: rec(15)?,
            sup_baseline_drop: rec(14)?,
            sub_baseline_drop: rec(10)?,
            space_after_script: rec(17)?,
            upper_limit_gap: rec(18)?,
            lower_limit_gap: rec(20)?,
            radical_gap: rec(49)?,
            radical_display_gap: rec(50)?,
            radical_rule: rec(51)?,
            radical_extra_ascender: rec(52)?,
            radical_degree_raise: read_i16(c, 4 * 55 - 8).map(scale).unwrap_or(d.radical_degree_raise),
            display_op_min_height: if display_op_min > 0.0 { display_op_min } else { d.display_op_min_height },
        })
    }
}

fn read_u16(b: &[u8], off: usize) -> Option<u16> {
    let s = b.get(off..off + 2)?;
    Some(u16::from_be_bytes([s[0], s[1]]))
}

fn read_i16(b: &[u8], off: usize) -> Option<i16> {
    read_u16(b, off).map(|v| v as i16)
}

/// A glyph stretched to cover a target height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stretched {
    pub ch: char,
    /// Multiplier on the glyph's own size (1.0 when a pre-built variant was enough).
    pub scale: f32,
    pub advance: f32,
    pub ascent: f32,
    pub descent: f32,
}

/// Chooses the glyph that best covers `target` em of total height (ascent + descent).
///
/// The first variant at least that tall wins with scale 1. When none is tall enough, the
/// largest variant is scaled up. A glyph never shrinks below its own size.
pub fn stretch(metrics: &dyn FontMetrics, ch: char, target: f32) -> Stretched {
    let base = metrics.glyph(ch);
    let mut candidates = metrics.variants(ch);
    if candidates.is_empty() {
        candidates.push(base);
    }
    for cand in &candidates {
        if cand.total() >= target {
            return Stretched {
                ch: cand.ch,
                scale: 1.0,
                advance: cand.advance,
                ascent: cand.ascent,
                descent: cand.descent,
            };
        }
    }
    let largest = candidates[candidates.len() - 1];
    let total = largest.total().max(f32::EPSILON);
    let scale = target / total;
    Stretched {
        ch: largest.ch,
        scale,
        advance: largest.advance * scale,
        ascent: largest.ascent * scale,
        descent: largest.descent * scale,
    }
}

/// Rough metrics for fonts that have no math data: enough to lay out, not to be exact.
pub struct EstimatedMetrics;

impl FontMetrics for EstimatedMetrics {
    fn glyph(&self, ch: char) -> GlyphMetrics {
        let (advance, ascent, descent) = match ch {
            '0'..='9' => (0.5, 0.7, 0.0),
            'g' | 'j' | 'p' | 'q' | 'y' => (0.5, 0.45, 0.2),
            'a'..='z' => (0.5, 0.45, 0.0),
            'A'..='Z' => (0.68, 0.7, 0.0),
            '(' | ')' | '[' | ']' => (0.39, 0.75, 0.25),
            '{' | '}' | '|' | '‖' | '⟨' | '⟩' => (0.39, 0.75, 0.25),
            '∑' | '∏' => (0.9, 0.75, 0.25),
            '∫' => (0.5, 0.8, 0.3),
            '√' => (0.83, 0.8, 0.2),
            '.' | ',' => (0.28, 0.1, 0.1),
            _ => (0.6, 0.6, 0.1),
        };
        GlyphMetrics { ch, advance, ascent, descent }
    }
}

/// Metrics read from a loaded font: advances from its shaping, heights from its x-height, cap-height and
/// descent. Per-glyph outlines are not read, so the heights are an estimate; the math constants come from the
/// caller (`constants`, from a MATH table when the font has one).
pub struct FontFaceMetrics<'a> {
    pub font: &'a crate::font::Font<'static>,
    pub constants: Option<MathConstants>,
}

impl FontMetrics for FontFaceMetrics<'_> {
    fn glyph(&self, ch: char) -> GlyphMetrics {
        let lm = self.font.line_metrics(1.0);
        let mut buf = [0u8; 4];
        let advance = self.font.measure_var(ch.encode_utf8(&mut buf), 1.0, &[]);
        let (ascent, descent) = match ch {
            '∑' | '∏' => (0.75, 0.25),
            '∫' => (0.8, 0.25),
            _ if ch.is_ascii_lowercase() && !"bdfhklt".contains(ch) => (lm.x_height, 0.0),
            _ if "gjpqy".contains(ch) => (lm.cap_height, lm.descent),
            _ if ch.is_whitespace() => (0.0, 0.0),
            _ => (lm.cap_height, 0.0),
        };
        GlyphMetrics { ch, advance, ascent, descent }
    }

    fn constants(&self) -> Option<MathConstants> {
        self.constants
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fake provider: fixed glyph sizes plus a `(` variant ladder, no MATH table.
    struct Fake;

    impl FontMetrics for Fake {
        fn glyph(&self, ch: char) -> GlyphMetrics {
            GlyphMetrics { ch, advance: 0.5, ascent: 0.5, descent: 0.1 }
        }

        fn variants(&self, ch: char) -> Vec<GlyphMetrics> {
            if ch != '(' {
                return Vec::new();
            }
            vec![
                GlyphMetrics { ch: '(', advance: 0.4, ascent: 0.5, descent: 0.1 },
                GlyphMetrics { ch: '\u{239B}', advance: 0.5, ascent: 0.8, descent: 0.4 },
                GlyphMetrics { ch: '\u{239C}', advance: 0.6, ascent: 1.1, descent: 0.5 },
            ]
        }
    }

    #[test]
    fn stretch_picks_smallest_variant_that_fits() {
        // Totals are 0.6, 1.2 and 1.6.
        let s = stretch(&Fake, '(', 1.0);
        assert_eq!(s.ch, '\u{239B}');
        assert_eq!(s.scale, 1.0);
        let small = stretch(&Fake, '(', 0.5);
        assert_eq!(small.ch, '(');
        assert_eq!(small.scale, 1.0);
    }

    #[test]
    fn stretch_scales_largest_variant_when_none_fits() {
        let s = stretch(&Fake, '(', 3.2);
        assert_eq!(s.ch, '\u{239C}');
        assert!((s.scale - 2.0).abs() < 1e-5);
        assert!((s.ascent - 2.2).abs() < 1e-5);
    }

    #[test]
    fn stretch_without_variants_scales_base_glyph() {
        let s = stretch(&Fake, 'x', 1.2);
        assert_eq!(s.ch, 'x');
        assert!((s.scale - 2.0).abs() < 1e-5);
        assert_eq!(stretch(&Fake, 'x', 0.3).scale, 1.0);
    }

    #[test]
    fn math_table_is_read_from_the_constants_subtable() {
        // Header: version (4), MathConstants offset = 10; then 2 int16 + 2 uint16 + records.
        let upem = 1000u16;
        let mut t = vec![0u8; 10];
        t[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
        t[4..6].copy_from_slice(&10u16.to_be_bytes());
        let mut c = Vec::new();
        c.extend_from_slice(&70i16.to_be_bytes());
        c.extend_from_slice(&50i16.to_be_bytes());
        c.extend_from_slice(&0u16.to_be_bytes());
        c.extend_from_slice(&1400u16.to_be_bytes());
        // Records 4..=55, each 4 bytes; fill all with 0 and set the ones we read.
        let mut recs = vec![0i16; 52];
        recs[5 - 4] = 250; // axis
        recs[38 - 4] = 50; // fraction rule
        recs[55 - 4] = 60; // degree raise, percent
        for v in recs {
            c.extend_from_slice(&v.to_be_bytes());
            c.extend_from_slice(&0u16.to_be_bytes());
        }
        t.extend_from_slice(&c);
        let m = MathConstants::from_math_table(&t, upem).expect("table parses");
        assert!((m.script_scale - 0.7).abs() < 1e-6);
        assert!((m.script_script_scale - 0.5).abs() < 1e-6);
        assert!((m.axis_height - 0.25).abs() < 1e-6);
        assert!((m.fraction_rule - 0.05).abs() < 1e-6);
        assert!((m.display_op_min_height - 1.4).abs() < 1e-6);
        assert!((m.radical_degree_raise - 0.6).abs() < 1e-6);
    }

    #[test]
    fn truncated_math_table_is_rejected() {
        assert!(MathConstants::from_math_table(&[0, 1, 0, 0, 0, 10], 1000).is_none());
        assert!(MathConstants::from_math_table(&[], 1000).is_none());
        assert!(MathConstants::from_math_table(&[0; 16], 0).is_none());
    }

    #[test]
    fn estimated_metrics_give_descenders_depth() {
        let m = EstimatedMetrics;
        assert!(m.glyph('g').descent > 0.0);
        assert_eq!(m.glyph('x').descent, 0.0);
        assert!(m.glyph('∑').total() > 0.0);
    }
}
