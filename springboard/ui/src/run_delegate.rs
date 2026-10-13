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

/// A run delegate: the callbacks that give an embedded object its metrics. A line layout asks the delegate for the
/// ascent, descent and width in place of glyph metrics.
pub trait RunDelegate {
    fn ascent(&self) -> f32;
    fn descent(&self) -> f32;
    fn width(&self) -> f32;
}

/// A delegate built from three closures, for apps that compute the metrics on demand.
pub struct CallbackDelegate {
    pub ascent: Box<dyn Fn() -> f32>,
    pub descent: Box<dyn Fn() -> f32>,
    pub width: Box<dyn Fn() -> f32>,
}

impl RunDelegate for CallbackDelegate {
    fn ascent(&self) -> f32 {
        (self.ascent)()
    }
    fn descent(&self) -> f32 {
        (self.descent)()
    }
    fn width(&self) -> f32 {
        (self.width)()
    }
}

impl RunDelegate for RunMetrics {
    fn ascent(&self) -> f32 {
        self.ascent
    }
    fn descent(&self) -> f32 {
        self.descent
    }
    fn width(&self) -> f32 {
        self.width
    }
}

/// The metrics a delegate reports, clamped to non-negative and with NaN counted as zero.
pub fn metrics_of(d: &dyn RunDelegate) -> RunMetrics {
    let c = |v: f32| if v.is_nan() { 0.0 } else { v };
    RunMetrics::new(c(d.ascent()), c(d.descent()), c(d.width()))
}

/// An image placed in a line of text, with the variants an adaptive image offers.
#[derive(Clone, Debug, PartialEq)]
pub struct ImageAttachment {
    pub id: u64,
    pub variants: Vec<ImageVariant>,
    /// Natural size in points.
    pub natural: (f32, f32),
    /// Offset of the image bottom above the baseline (negative to drop below it).
    pub baseline_shift: f32,
}

impl ImageAttachment {
    pub fn new(id: u64, natural: (f32, f32)) -> Self {
        Self { id, variants: Vec::new(), natural, baseline_shift: 0.0 }
    }

    /// The variant to draw for a display scale and appearance.
    pub fn variant(&self, scale: u32, dark: bool) -> Option<&ImageVariant> {
        pick_variant(&self.variants, scale, dark)
    }

    /// The size it takes when the line offers `max` (the width left and the line height). The image keeps its aspect.
    pub fn size(&self, max: (f32, f32)) -> (f32, f32) {
        fit_size(self.natural, max)
    }

    /// The piece for the line layout: the image sits on the baseline shifted by `baseline_shift`.
    pub fn piece(&self, max: (f32, f32)) -> Piece {
        let (w, h) = self.size(max);
        let shift = if self.baseline_shift.is_nan() { 0.0 } else { self.baseline_shift };
        Piece::Attachment { id: self.id, metrics: RunMetrics::new(h + shift, (-shift).max(0.0), w) }
    }
}

/// A placed attachment: its id and its box in line coordinates (`x` from the line start, `y` down from the line top).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub id: u64,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// Where each attachment of a line is drawn. `baseline` is the distance from the line top to the baseline (normally
/// the line's maximum ascent). The box spans from ascent above the baseline to descent below it.
pub fn placements(pieces: &[Piece], baseline: f32) -> Vec<Placement> {
    let mut x = 0.0_f32;
    let mut out = Vec::new();
    for p in pieces {
        let m = p.metrics();
        if let Piece::Attachment { id, .. } = p {
            out.push(Placement { id: *id, x, y: baseline - m.ascent, width: m.width, height: m.ascent + m.descent });
        }
        x += m.width;
    }
    out
}

#[cfg(test)]
mod attachment_tests {
    use super::*;

    #[test]
    fn callback_delegate_reports_clamped_metrics() {
        let d = CallbackDelegate { ascent: Box::new(|| 10.0), descent: Box::new(|| -2.0), width: Box::new(|| f32::NAN) };
        assert_eq!(metrics_of(&d), RunMetrics::new(10.0, 0.0, 0.0));
        assert_eq!(metrics_of(&RunMetrics::new(1.0, 2.0, 3.0)).width, 3.0);
    }

    #[test]
    fn image_attachment_fits_and_sits_on_the_baseline() {
        let mut a = ImageAttachment::new(7, (40.0, 20.0));
        a.variants.push(ImageVariant { scale: 1, dark: false, name: "a".into() });
        a.variants.push(ImageVariant { scale: 2, dark: false, name: "a2".into() });
        assert_eq!(a.variant(2, false).map(|v| v.name.as_str()), Some("a2"));
        let p = a.piece((20.0, 20.0));
        assert_eq!(p.metrics(), RunMetrics::new(10.0, 0.0, 20.0));
        a.baseline_shift = -3.0;
        let m = a.piece((40.0, 20.0)).metrics();
        assert_eq!((m.ascent, m.descent), (17.0, 3.0));
    }

    #[test]
    fn placements_follow_the_pieces() {
        let pieces = [
            Piece::Text(RunMetrics::new(12.0, 4.0, 30.0)),
            Piece::Attachment { id: 3, metrics: RunMetrics::new(8.0, 2.0, 10.0) },
        ];
        let pl = placements(&pieces, 12.0);
        assert_eq!(pl, vec![Placement { id: 3, x: 30.0, y: 4.0, width: 10.0, height: 10.0 }]);
    }
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
