//! Run delegates and adaptive images for styled text. A run delegate gives an embedded attachment (an image or a view
//! placed in a line of text) fixed ascent, descent and width, which the line layout uses in place of glyph metrics.
//! This module holds the pure rules: the metrics of one run, the metrics of a line that mixes text runs and
//! attachments, the attachment under a horizontal position, the image variant to show for a scale and appearance,
//! and the aspect-fit size for a proposed size. The renderer does the drawing; nothing here touches the screen.

/// The typographic metrics of one run, in points. Negative values are clamped to zero.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RunMetrics {
    pub ascent: f32,
    pub descent: f32,
    pub width: f32,
}

impl RunMetrics {
    pub fn new(ascent: f32, descent: f32, width: f32) -> Self {
        Self { ascent: ascent.max(0.0), descent: descent.max(0.0), width: width.max(0.0) }
    }

    /// The line height this run needs above and below the baseline.
    pub fn height(&self) -> f32 {
        self.ascent + self.descent
    }
}

/// One piece of a line: a text run with its metrics, or an attachment with the id its owner uses to find it again.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Piece {
    Text(RunMetrics),
    Attachment { id: u64, metrics: RunMetrics },
}

impl Piece {
    pub fn metrics(&self) -> RunMetrics {
        match *self {
            Piece::Text(m) => m,
            Piece::Attachment { metrics, .. } => metrics,
        }
    }
}

/// The metrics of a whole line: the largest ascent and descent of its pieces, and the sum of their widths.
/// An empty line has zero metrics.
pub fn line_metrics(pieces: &[Piece]) -> RunMetrics {
    pieces.iter().map(Piece::metrics).fold(RunMetrics::new(0.0, 0.0, 0.0), |acc, m| RunMetrics {
        ascent: acc.ascent.max(m.ascent),
        descent: acc.descent.max(m.descent),
        width: acc.width + m.width,
    })
}

/// The id of the attachment under the horizontal position `x` (points from the start of the line), if any. A position
/// is inside a piece when `start <= x < start + width`, so the end edge belongs to the next piece.
pub fn attachment_at(pieces: &[Piece], x: f32) -> Option<u64> {
    let mut start = 0.0_f32;
    for p in pieces {
        let m = p.metrics();
        if x >= start && x < start + m.width {
            return match p {
                Piece::Attachment { id, .. } => Some(*id),
                Piece::Text(_) => None,
            };
        }
        start += m.width;
    }
    None
}

/// One image variant of an adaptive image: the display scale it is drawn for and whether it is the dark appearance.
#[derive(Clone, Debug, PartialEq)]
pub struct ImageVariant {
    pub scale: u32,
    pub dark: bool,
    pub name: String,
}

/// Picks the variant to draw for a display `scale` and appearance. When some variants match the appearance, only
/// those are considered; otherwise all are. Among them the smallest scale at or above the requested one wins, so the
/// image is never drawn from a lower resolution than the display needs. If every variant is below the scale, the
/// largest one wins. Returns `None` only when there are no variants.
pub fn pick_variant(variants: &[ImageVariant], scale: u32, dark: bool) -> Option<&ImageVariant> {
    let matching: Vec<&ImageVariant> = variants.iter().filter(|v| v.dark == dark).collect();
    let candidates: Vec<&ImageVariant> = if matching.is_empty() { variants.iter().collect() } else { matching };
    let at_or_above = candidates.iter().copied().filter(|v| v.scale >= scale).min_by_key(|v| v.scale);
    at_or_above.or_else(|| candidates.iter().copied().max_by_key(|v| v.scale))
}

/// The size an image of `natural` size takes when a proposed box of `max` size is offered: it keeps its aspect ratio
/// and fits inside the box. A zero or negative natural or proposed side gives zero.
pub fn fit_size(natural: (f32, f32), max: (f32, f32)) -> (f32, f32) {
    let (nw, nh) = natural;
    let (mw, mh) = max;
    if nw <= 0.0 || nh <= 0.0 || mw <= 0.0 || mh <= 0.0 {
        return (0.0, 0.0);
    }
    let s = (mw / nw).min(mh / nh);
    (nw * s, nh * s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(a: f32, d: f32, w: f32) -> Piece {
        Piece::Text(RunMetrics::new(a, d, w))
    }

    fn attachment(id: u64, a: f32, d: f32, w: f32) -> Piece {
        Piece::Attachment { id, metrics: RunMetrics::new(a, d, w) }
    }

    #[test]
    fn a_line_takes_the_largest_ascent_and_descent_and_the_sum_of_widths() {
        // Text 10/3 width 40, attachment 14/2 width 20: ascent 14, descent 3, width 60, height 17.
        let line = [text(10.0, 3.0, 40.0), attachment(7, 14.0, 2.0, 20.0)];
        let m = line_metrics(&line);
        assert_eq!(m, RunMetrics::new(14.0, 3.0, 60.0));
        assert_eq!(m.height(), 17.0);
    }

    #[test]
    fn an_empty_line_has_zero_metrics() {
        assert_eq!(line_metrics(&[]), RunMetrics::new(0.0, 0.0, 0.0));
    }

    #[test]
    fn negative_metrics_are_clamped_to_zero() {
        assert_eq!(RunMetrics::new(-1.0, 2.0, -3.0), RunMetrics::new(0.0, 2.0, 0.0));
    }

    #[test]
    fn the_attachment_under_a_position_is_found_and_the_end_edge_belongs_to_the_next_piece() {
        let line = [text(10.0, 3.0, 40.0), attachment(7, 14.0, 2.0, 20.0), text(10.0, 3.0, 5.0)];
        assert_eq!(attachment_at(&line, 10.0), None);
        assert_eq!(attachment_at(&line, 40.0), Some(7));
        assert_eq!(attachment_at(&line, 59.5), Some(7));
        assert_eq!(attachment_at(&line, 60.0), None);
        assert_eq!(attachment_at(&line, 100.0), None);
    }

    fn variant(scale: u32, dark: bool, name: &str) -> ImageVariant {
        ImageVariant { scale, dark, name: name.to_string() }
    }

    #[test]
    fn the_variant_is_the_smallest_scale_that_covers_the_display() {
        let v = [variant(1, false, "a1"), variant(2, false, "a2"), variant(3, false, "a3")];
        assert_eq!(pick_variant(&v, 2, false).map(|x| x.name.as_str()), Some("a2"));
        assert_eq!(pick_variant(&v, 1, false).map(|x| x.name.as_str()), Some("a1"));
        // Above every variant: the largest is the best available.
        assert_eq!(pick_variant(&v, 4, false).map(|x| x.name.as_str()), Some("a3"));
    }

    #[test]
    fn the_appearance_narrows_the_choice_and_falls_back_when_none_matches() {
        let v = [variant(2, false, "a2"), variant(2, true, "d2"), variant(3, true, "d3")];
        assert_eq!(pick_variant(&v, 2, true).map(|x| x.name.as_str()), Some("d2"));
        assert_eq!(pick_variant(&v, 3, true).map(|x| x.name.as_str()), Some("d3"));
        let light_only = [variant(2, false, "a2"), variant(3, false, "a3")];
        assert_eq!(pick_variant(&light_only, 2, true).map(|x| x.name.as_str()), Some("a2"));
        assert!(pick_variant(&[], 2, false).is_none());
    }

    #[test]
    fn an_image_fits_the_proposed_box_without_changing_its_aspect_ratio() {
        // 200 by 100 in a 100 by 100 box: the width limits, the scale is 0.5.
        assert_eq!(fit_size((200.0, 100.0), (100.0, 100.0)), (100.0, 50.0));
        // 100 by 100 in a 300 by 50 box: the height limits, the scale is 0.5.
        assert_eq!(fit_size((100.0, 100.0), (300.0, 50.0)), (50.0, 50.0));
        assert_eq!(fit_size((0.0, 100.0), (300.0, 50.0)), (0.0, 0.0));
    }
}
