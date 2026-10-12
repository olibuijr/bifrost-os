//! The `kern` table: horizontal pair kerning from an OpenType font, read into a lookup and applied to a run of
//! glyphs. Only the version 0 table header and format 0 horizontal subtables are read (the layout OpenType and
//! TrueType fonts ship). Other subtable formats, vertical subtables and cross-stream subtables are skipped, so a
//! font with only those gives no adjustment. The GPOS pair adjustment is not read here.
//!
//! Each subtable is a sorted list of (left glyph, right glyph) pairs with a value in font units. Subtables add up
//! in table order, except that an override subtable replaces the value accumulated so far for a pair it contains.

fn u16_at(d: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_be_bytes(d.get(o..o + 2)?.try_into().ok()?))
}

/// Coverage bit 0: the subtable applies to horizontal text.
const COVERAGE_HORIZONTAL: u16 = 0x0001;
/// Coverage bit 3: the subtable's values replace the accumulated value for the pairs it holds.
const COVERAGE_OVERRIDE: u16 = 0x0008;

struct Subtable {
    override_values: bool,
    /// sorted by key, where key = left glyph << 16 | right glyph
    pairs: Vec<(u32, i16)>,
}

/// The kerning pairs of one font's `kern` table.
#[derive(Default)]
pub struct Kern {
    tables: Vec<Subtable>,
}

impl Kern {
    /// Reads the `kern` table. `None` when the header is not version 0 or is cut short. A subtable whose header is
    /// cut short ends the read; a subtable that is not horizontal format 0 is skipped.
    pub fn parse(data: &[u8]) -> Option<Kern> {
        if u16_at(data, 0)? != 0 {
            return None;
        }
        let count = u16_at(data, 2)? as usize;
        let mut kern = Kern::default();
        let mut at = 4;
        for _ in 0..count {
            let length = u16_at(data, at + 2)? as usize;
            let coverage = u16_at(data, at + 4)?;
            if length < 6 {
                return None;
            }
            let format = coverage >> 8;
            if format == 0 && coverage & COVERAGE_HORIZONTAL != 0 {
                if let Some(pairs) = format0_pairs(data, at + 6, at + length) {
                    kern.tables.push(Subtable { override_values: coverage & COVERAGE_OVERRIDE != 0, pairs });
                }
            }
            at += length;
        }
        Some(kern)
    }

    /// True when no subtable applies to horizontal text, so every pair is 0.
    pub fn is_empty(&self) -> bool {
        self.tables.is_empty()
    }

    /// The kerning value for `left` followed by `right`, in font units. Negative values tighten the pair.
    pub fn pair(&self, left: u16, right: u16) -> i32 {
        let key = (left as u32) << 16 | right as u32;
        let mut value = 0i32;
        for t in &self.tables {
            if let Ok(i) = t.pairs.binary_search_by_key(&key, |p| p.0) {
                let v = t.pairs[i].1 as i32;
                value = if t.override_values { v } else { value + v };
            }
        }
        value
    }

    /// The kerning for `left` then `right` at `px` pixels, for a font with `units_per_em` font units per em.
    pub fn adjust_px(&self, left: u16, right: u16, px: f32, units_per_em: f32) -> f32 {
        if units_per_em <= 0.0 {
            return 0.0;
        }
        self.pair(left, right) as f32 * px / units_per_em
    }

    /// Extra advance after each glyph of a run, in pixels: element `i` is the adjustment between glyph `i` and
    /// glyph `i + 1`, and the last element is 0. The run's total width grows by the sum of these.
    pub fn run_adjustments(&self, glyphs: &[u16], px: f32, units_per_em: f32) -> Vec<f32> {
        let mut out = vec![0.0; glyphs.len()];
        for (i, w) in glyphs.windows(2).enumerate() {
            out[i] = self.adjust_px(w[0], w[1], px, units_per_em);
        }
        out
    }
}

/// The sorted pairs of a format 0 subtable whose body starts at `body` (the pair count is at `body`), limited to
/// `end`. `None` when the body is cut short.
fn format0_pairs(data: &[u8], body: usize, end: usize) -> Option<Vec<(u32, i16)>> {
    let n = u16_at(data, body)? as usize;
    let first = body + 8;
    if first + 6 * n > end.min(data.len()) {
        return None;
    }
    (0..n)
        .map(|i| {
            let p = first + 6 * i;
            let left = u16_at(data, p)? as u32;
            let right = u16_at(data, p + 2)? as u32;
            let value = u16_at(data, p + 4)? as i16;
            Some((left << 16 | right, value))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a `kern` table from (coverage, pairs) subtables. Each subtable gets the format 0 layout.
    fn table(subs: &[(u16, Vec<(u16, u16, i16)>)]) -> Vec<u8> {
        let mut out: Vec<u8> = vec![0, 0, 0, subs.len() as u8];
        for (coverage, pairs) in subs {
            let length = 6 + 8 + 6 * pairs.len();
            out.extend([0, 0]);
            out.extend((length as u16).to_be_bytes());
            out.extend(coverage.to_be_bytes());
            out.extend((pairs.len() as u16).to_be_bytes());
            out.extend([0; 6]); // searchRange, entrySelector, rangeShift
            for (l, r, v) in pairs {
                out.extend(l.to_be_bytes());
                out.extend(r.to_be_bytes());
                out.extend(v.to_be_bytes());
            }
        }
        out
    }

    const HORIZONTAL: u16 = 0x0001;
    const OVERRIDE_HORIZONTAL: u16 = 0x0009;

    #[test]
    fn looks_up_a_pair_and_reports_zero_for_others() {
        let kern = Kern::parse(&table(&[(HORIZONTAL, vec![(1, 2, -50), (1, 4, 20), (3, 2, -7)])])).unwrap();
        assert_eq!(kern.pair(1, 2), -50);
        assert_eq!(kern.pair(1, 4), 20);
        assert_eq!(kern.pair(3, 2), -7);
        assert_eq!(kern.pair(2, 1), 0, "the pair is ordered: left then right");
        assert_eq!(kern.pair(1, 3), 0);
        assert!(!kern.is_empty());
    }

    #[test]
    fn horizontal_subtables_add_up() {
        let kern = Kern::parse(&table(&[(HORIZONTAL, vec![(1, 2, -30)]), (HORIZONTAL, vec![(1, 2, -20), (5, 6, 4)])])).unwrap();
        assert_eq!(kern.pair(1, 2), -50);
        assert_eq!(kern.pair(5, 6), 4);
    }

    #[test]
    fn an_override_subtable_replaces_the_value_it_holds() {
        let kern = Kern::parse(&table(&[(HORIZONTAL, vec![(1, 2, -30), (1, 3, -8)]), (OVERRIDE_HORIZONTAL, vec![(1, 2, -10)])])).unwrap();
        assert_eq!(kern.pair(1, 2), -10, "the override value replaces the accumulated -30");
        assert_eq!(kern.pair(1, 3), -8, "a pair the override does not hold keeps its value");
    }

    #[test]
    fn vertical_and_other_format_subtables_are_ignored() {
        let vertical = table(&[(0x0000, vec![(1, 2, -50)])]);
        assert_eq!(Kern::parse(&vertical).unwrap().pair(1, 2), 0);
        let format2 = table(&[(0x0200 | HORIZONTAL, vec![(1, 2, -50)])]);
        let kern = Kern::parse(&format2).unwrap();
        assert!(kern.is_empty());
        assert_eq!(kern.pair(1, 2), 0);
    }

    #[test]
    fn rejects_other_versions_and_truncated_headers() {
        assert!(Kern::parse(&[]).is_none());
        assert!(Kern::parse(&[0, 1, 0, 0]).is_none(), "only version 0 is read");
        assert!(Kern::parse(&[0, 0, 0, 1, 0, 0]).is_none(), "the subtable header is cut short");
    }

    #[test]
    fn scales_units_to_pixels_and_builds_run_adjustments() {
        let kern = Kern::parse(&table(&[(HORIZONTAL, vec![(1, 2, -50), (2, 3, -100)])])).unwrap();
        assert!((kern.adjust_px(1, 2, 13.0, 1000.0) - (-0.65)).abs() < 1e-6);
        assert_eq!(kern.adjust_px(1, 2, 13.0, 0.0), 0.0, "no units per em means no adjustment");
        let adj = kern.run_adjustments(&[1, 2, 3], 10.0, 1000.0);
        assert_eq!(adj.len(), 3);
        assert!((adj[0] - (-0.5)).abs() < 1e-6);
        assert!((adj[1] - (-1.0)).abs() < 1e-6);
        assert_eq!(adj[2], 0.0);
    }

    #[test]
    fn finds_every_pair_in_a_longer_sorted_table() {
        let pairs: Vec<(u16, u16, i16)> = (1..=5u16).map(|i| (i, i + 1, -(i as i16) * 10)).collect();
        let kern = Kern::parse(&table(&[(HORIZONTAL, pairs)])).unwrap();
        for i in 1..=5u16 {
            assert_eq!(kern.pair(i, i + 1), -(i as i32) * 10);
        }
        assert_eq!(kern.pair(2, 4), 0);
    }
}
