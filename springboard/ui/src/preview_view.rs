//! The layout of an embedded preview (`QLPreviewView`): where the title row, the body and the counter row sit inside a
//! view of a given size. The shell's floating preview panel and an app's own preview view read their title and counter
//! through the same helpers, so both show the same words in the same places.

/// A rectangle in points, origin at the top left of the view.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// The rows of an embedded preview. `footer` is the counter row, and `None` in the compact style.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewFrame {
    pub title: Frame,
    pub body: Frame,
    pub footer: Option<Frame>,
}

/// Height of the title row, in points at scale 1.
pub const TITLE_PT: f32 = 36.0;
/// Height of the counter row, in points at scale 1.
pub const FOOTER_PT: f32 = 22.0;
/// Margin around the body, in points at scale 1.
pub const PAD_PT: f32 = 12.0;

/// The rows for a `w` by `h` view at scale `k` (1.0 is one point per pixel). Negative sizes count as zero, and the
/// rows never run past the view: a view shorter than its rows shows only the title. `compact` drops the counter row.
pub fn view_frame(w: f32, h: f32, k: f32, compact: bool) -> ViewFrame {
    let (w, h) = (w.max(0.0), h.max(0.0));
    let pad = PAD_PT * k;
    let title_h = (TITLE_PT * k).min(h);
    let footer_h = if compact { 0.0 } else { (FOOTER_PT * k).min((h - title_h).max(0.0)) };
    let inner_w = (w - 2.0 * pad).max(0.0);
    let body_h = (h - title_h - footer_h - pad).max(0.0);
    ViewFrame {
        title: Frame { x: 0.0, y: 0.0, w, h: title_h },
        body: Frame { x: pad.min(w), y: title_h, w: inner_w, h: body_h },
        footer: (!compact).then(|| Frame { x: pad.min(w), y: h - footer_h, w: inner_w, h: footer_h }),
    }
}

/// The title a preview shows: the item's own title, or its URL when it has none.
pub fn title_of<'a>(title: &'a str, url: &'a str) -> &'a str {
    if title.is_empty() {
        url
    } else {
        title
    }
}

/// The counter row's text: the 1-based position of the item and the number of items ("3 of 9").
pub fn counter_text(index: usize, count: usize) -> String {
    format!("{} of {}", index + 1, count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_normal_style_has_a_title_a_body_and_a_counter_row_that_stay_inside_the_view() {
        let v = view_frame(400.0, 300.0, 1.0, false);
        assert_eq!(v.title, Frame { x: 0.0, y: 0.0, w: 400.0, h: TITLE_PT });
        let footer = v.footer.expect("the counter row");
        assert_eq!(footer.y + footer.h, 300.0, "the counter row sits at the foot");
        assert!(v.body.y >= v.title.h && v.body.y + v.body.h <= footer.y, "the body is between the rows");
        assert_eq!(v.body.x, PAD_PT);
        assert_eq!(v.body.w, 400.0 - 2.0 * PAD_PT);
    }

    #[test]
    fn the_compact_style_has_no_counter_row_and_a_taller_body() {
        let normal = view_frame(400.0, 300.0, 1.0, false);
        let compact = view_frame(400.0, 300.0, 1.0, true);
        assert!(compact.footer.is_none());
        assert!(compact.body.h > normal.body.h);
    }

    #[test]
    fn scale_grows_the_rows_and_a_tiny_view_keeps_them_inside() {
        let big = view_frame(400.0, 300.0, 2.0, false);
        assert_eq!(big.title.h, 2.0 * TITLE_PT);
        let tiny = view_frame(10.0, 20.0, 1.0, false);
        assert!(tiny.title.h <= 20.0);
        assert!(tiny.footer.map_or(true, |f| f.y >= 0.0 && f.y + f.h <= 20.0 + 1e-3));
        let negative = view_frame(-5.0, -5.0, 1.0, false);
        assert_eq!((negative.title.w, negative.title.h, negative.body.h), (0.0, 0.0, 0.0));
    }

    #[test]
    fn the_title_falls_back_to_the_url_and_the_counter_is_one_based() {
        assert_eq!(title_of("Q1 report", "file:///a.pdf"), "Q1 report");
        assert_eq!(title_of("", "file:///a.pdf"), "file:///a.pdf");
        assert_eq!(counter_text(0, 9), "1 of 9");
        assert_eq!(counter_text(2, 3), "3 of 3");
    }
}
