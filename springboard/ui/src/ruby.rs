//! Ruby annotations: a short annotation text set beside a run of base text (readings over ideographs). This module is
//! pure geometry. The caller measures the base run and the annotation characters at the annotation size, and gets back
//! where each annotation character goes along the line, how far the annotation may overhang the neighbours of the
//! base, and how much the base run has to widen when the annotation is longer than it.
//!
//! Positions are along the line axis, from the start of the base run: x for horizontal lines, y for vertical ones.
//! The side of the line the annotation sits on is `RubyPosition`: before (above a horizontal line, to the right of a
//! vertical one), after (the opposite side) or inter-character (to the right of each character of a vertical line,
//! used by some East Asian typography).

/// Where the annotation sits relative to the base text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RubyPosition {
    Before,
    After,
    InterCharacter,
}

/// How the annotation is spread along the base text when it is shorter than the base.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RubyAlignment {
    /// Chosen by the layout: space distributed around the characters when there are several, else centred.
    Auto,
    Start,
    Center,
    End,
    /// Extra space goes between the characters, none at the ends.
    DistributeLetter,
    /// Extra space goes between the characters and half a gap at each end.
    DistributeSpace,
    /// Like `DistributeLetter`, and the first and last characters touch the edges of the base.
    LineEdge,
}

/// Whether an annotation longer than its base may extend over the neighbouring text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RubyOverhang {
    /// Both sides, as far as the neighbours allow.
    Auto,
    Start,
    End,
    None,
}

/// One annotation: its text and how it is set.
#[derive(Clone, Debug, PartialEq)]
pub struct RubyAnnotation {
    pub text: String,
    pub position: RubyPosition,
    pub alignment: RubyAlignment,
    pub overhang: RubyOverhang,
    /// The annotation size as a fraction of the base size.
    pub size_factor: f32,
}

impl RubyAnnotation {
    pub fn new(text: &str) -> Self {
        RubyAnnotation {
            text: text.to_string(),
            position: RubyPosition::Before,
            alignment: RubyAlignment::Auto,
            overhang: RubyOverhang::Auto,
            size_factor: 0.5,
        }
    }

    pub fn with_position(mut self, p: RubyPosition) -> Self {
        self.position = p;
        self
    }

    pub fn with_alignment(mut self, a: RubyAlignment) -> Self {
        self.alignment = a;
        self
    }

    pub fn with_overhang(mut self, o: RubyOverhang) -> Self {
        self.overhang = o;
        self
    }

    pub fn with_size_factor(mut self, f: f32) -> Self {
        self.size_factor = f;
        self
    }

    /// The annotation font size for a base of `base_size`. Never below zero; a NaN factor counts as the default 0.5.
    pub fn size(&self, base_size: f32) -> f32 {
        let f = if self.size_factor.is_nan() { 0.5 } else { self.size_factor };
        (base_size * f).max(0.0)
    }
}

/// The result of laying out one annotation over one base run.
#[derive(Clone, Debug, PartialEq)]
pub struct RubyLayout {
    /// Start of each annotation character along the line, from the start of the base run. May be negative when the
    /// annotation overhangs the text before the base.
    pub positions: Vec<f32>,
    /// How far the annotation reaches over the text before the base, and after it.
    pub overhang_start: f32,
    pub overhang_end: f32,
    /// Space added before and after the base run because the annotation did not fit.
    pub pad_start: f32,
    pub pad_end: f32,
    /// The advance of the base run with its padding.
    pub advance: f32,
    /// Total length of the annotation.
    pub annotation_length: f32,
}

/// Lays out an annotation whose characters measure `widths` (at the annotation size) over a base run of
/// `base_width`. `max_overhang_start` and `max_overhang_end` are how much of the neighbouring text on each side the
/// annotation may cover (usually the width of the neighbour, capped at one annotation em by the caller). Negative or
/// NaN inputs count as zero.
pub fn layout(
    base_width: f32,
    widths: &[f32],
    ann: &RubyAnnotation,
    max_overhang_start: f32,
    max_overhang_end: f32,
) -> RubyLayout {
    let clean = |v: f32| if v.is_nan() { 0.0 } else { v.max(0.0) };
    let base = clean(base_width);
    let ws: Vec<f32> = widths.iter().map(|&w| clean(w)).collect();
    let total: f32 = ws.iter().sum();
    let n = ws.len();
    if n == 0 {
        return RubyLayout {
            positions: Vec::new(),
            overhang_start: 0.0,
            overhang_end: 0.0,
            pad_start: 0.0,
            pad_end: 0.0,
            advance: base,
            annotation_length: 0.0,
        };
    }
    if total <= base {
        let slack = base - total;
        // Gap before the first character and between neighbours.
        let (lead, gap) = match ann.alignment {
            RubyAlignment::Start => (0.0, 0.0),
            RubyAlignment::End => (slack, 0.0),
            RubyAlignment::Center => (slack / 2.0, 0.0),
            RubyAlignment::DistributeLetter | RubyAlignment::LineEdge => {
                if n > 1 {
                    (0.0, slack / (n - 1) as f32)
                } else {
                    (slack / 2.0, 0.0)
                }
            }
            RubyAlignment::DistributeSpace | RubyAlignment::Auto => {
                if n > 1 {
                    (slack / (2.0 * n as f32), slack / n as f32)
                } else {
                    (slack / 2.0, 0.0)
                }
            }
        };
        let mut positions = Vec::with_capacity(n);
        let mut x = lead;
        for w in &ws {
            positions.push(x);
            x += w + gap;
        }
        return RubyLayout {
            positions,
            overhang_start: 0.0,
            overhang_end: 0.0,
            pad_start: 0.0,
            pad_end: 0.0,
            advance: base,
            annotation_length: total,
        };
    }
    // The annotation is longer than the base: overhang the neighbours as allowed, widen the base for the rest.
    let excess = total - base;
    let (ms, me) = (clean(max_overhang_start), clean(max_overhang_end));
    let (os, oe) = match ann.overhang {
        RubyOverhang::None => (0.0, 0.0),
        RubyOverhang::Start => (ms.min(excess), 0.0),
        RubyOverhang::End => (0.0, me.min(excess)),
        RubyOverhang::Auto => {
            let half = excess / 2.0;
            let mut os = ms.min(half);
            let mut oe = me.min(half);
            // Whatever one side cannot take, the other side takes if it can.
            if os < half {
                oe = me.min(excess - os);
            } else if oe < half {
                os = ms.min(excess - oe);
            }
            (os, oe)
        }
    };
    let rest = excess - os - oe;
    let (pad_start, pad_end) = match ann.alignment {
        RubyAlignment::Start => (0.0, rest),
        RubyAlignment::End => (rest, 0.0),
        _ => (rest / 2.0, rest / 2.0),
    };
    // The annotation starts that far before the (padded) base run's original start.
    let mut x = -(os + pad_start);
    let mut positions = Vec::with_capacity(n);
    for w in &ws {
        positions.push(x);
        x += w;
    }
    RubyLayout {
        positions,
        overhang_start: os,
        overhang_end: oe,
        pad_start,
        pad_end,
        advance: base + pad_start + pad_end,
        annotation_length: total,
    }
}

/// The baseline offset of a horizontal annotation from the baseline of its base, in px, positive down. `Before` sits
/// above the base ascent, `After` below the base descent. Inter-character annotations are vertical-line only, so the
/// result is 0 (the caller offsets them sideways with [`side_offset`]).
pub fn baseline_offset(position: RubyPosition, base_ascent: f32, base_descent: f32, ann_ascent: f32, ann_descent: f32) -> f32 {
    match position {
        RubyPosition::Before => -(base_ascent.max(0.0) + ann_descent.max(0.0)),
        RubyPosition::After => base_descent.max(0.0) + ann_ascent.max(0.0),
        RubyPosition::InterCharacter => 0.0,
    }
}

/// The sideways offset of an annotation in a vertical line, from the centre line of the base column: positive is
/// to the right. `Before` and `InterCharacter` sit to the right, `After` to the left.
pub fn side_offset(position: RubyPosition, base_em: f32, ann_em: f32) -> f32 {
    let d = (base_em.max(0.0) + ann_em.max(0.0)) / 2.0;
    match position {
        RubyPosition::After => -d,
        _ => d,
    }
}

/// The extra line height an annotation adds on its side of a horizontal line.
pub fn line_extra(ann_height: f32) -> f32 {
    ann_height.max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ann() -> RubyAnnotation {
        RubyAnnotation::new("ab")
    }

    #[test]
    fn defaults_are_before_auto_half_size() {
        let a = ann();
        assert_eq!(a.position, RubyPosition::Before);
        assert_eq!(a.size(20.0), 10.0);
        assert_eq!(ann().with_size_factor(f32::NAN).size(20.0), 10.0);
    }

    #[test]
    fn short_annotation_centres_a_single_character() {
        let l = layout(20.0, &[6.0], &ann(), 5.0, 5.0);
        assert_eq!(l.positions, vec![7.0]);
        assert_eq!(l.advance, 20.0);
    }

    #[test]
    fn auto_distributes_space_around_characters() {
        let l = layout(20.0, &[4.0, 4.0], &ann(), 0.0, 0.0);
        // slack 12, gap 6, lead 3
        assert_eq!(l.positions, vec![3.0, 13.0]);
    }

    #[test]
    fn line_edge_touches_both_edges() {
        let a = ann().with_alignment(RubyAlignment::LineEdge);
        let l = layout(20.0, &[4.0, 4.0], &a, 0.0, 0.0);
        assert_eq!(l.positions, vec![0.0, 16.0]);
    }

    #[test]
    fn start_and_end_alignment() {
        let s = layout(20.0, &[4.0], &ann().with_alignment(RubyAlignment::Start), 0.0, 0.0);
        assert_eq!(s.positions, vec![0.0]);
        let e = layout(20.0, &[4.0], &ann().with_alignment(RubyAlignment::End), 0.0, 0.0);
        assert_eq!(e.positions, vec![16.0]);
    }

    #[test]
    fn long_annotation_overhangs_neighbours_first() {
        let l = layout(10.0, &[8.0, 8.0], &ann(), 2.0, 2.0);
        // excess 6: 2 each side, 2 left for padding (1 each side)
        assert_eq!(l.overhang_start, 2.0);
        assert_eq!(l.overhang_end, 2.0);
        assert_eq!(l.pad_start, 1.0);
        assert_eq!(l.advance, 12.0);
        assert_eq!(l.positions, vec![-3.0, 5.0]);
    }

    #[test]
    fn overhang_none_widens_the_base() {
        let a = ann().with_overhang(RubyOverhang::None);
        let l = layout(10.0, &[8.0, 8.0], &a, 9.0, 9.0);
        assert_eq!(l.overhang_start + l.overhang_end, 0.0);
        assert_eq!(l.advance, 16.0);
        assert_eq!(l.positions[0], -3.0);
    }

    #[test]
    fn one_sided_neighbour_lets_the_other_side_take_more() {
        let l = layout(10.0, &[8.0, 8.0], &ann(), 0.0, 9.0);
        assert_eq!(l.overhang_start, 0.0);
        assert_eq!(l.overhang_end, 6.0);
        assert_eq!(l.advance, 10.0);
    }

    #[test]
    fn empty_annotation_is_transparent() {
        let l = layout(10.0, &[], &ann(), 1.0, 1.0);
        assert!(l.positions.is_empty());
        assert_eq!(l.advance, 10.0);
    }

    #[test]
    fn baseline_and_side_offsets() {
        assert_eq!(baseline_offset(RubyPosition::Before, 10.0, 3.0, 5.0, 1.0), -11.0);
        assert_eq!(baseline_offset(RubyPosition::After, 10.0, 3.0, 5.0, 1.0), 8.0);
        assert_eq!(side_offset(RubyPosition::Before, 20.0, 10.0), 15.0);
        assert_eq!(side_offset(RubyPosition::After, 20.0, 10.0), -15.0);
    }
}
