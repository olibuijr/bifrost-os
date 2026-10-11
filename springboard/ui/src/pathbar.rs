//! A path bar (Files's View > Show Path Bar, `NativePathControl`): the folders from the root to the current item with
//! chevrons between them. Each segment can be clicked. When the bar is too narrow the middle segments shrink to
//! their glyph alone, then the last label is shortened. Pure model: widths come from the caller's text measure.
use crate::Rect;

/// One laid-out segment: its index in the path, its rectangle, and the width left for its label (0 when only the
/// glyph shows).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Segment {
    pub index: usize,
    pub rect: Rect,
    pub label_w: f32,
}

/// `labels` are the label widths in points, from the root. `glyph` is the glyph plus its gap, `chevron` the space
/// a chevron takes between segments.
pub fn layout(labels: &[f32], bounds: Rect, glyph: f32, chevron: f32) -> Vec<Segment> {
    let n = labels.len();
    if n == 0 {
        return Vec::new();
    }
    let mut shown: Vec<f32> = labels.to_vec();
    let total = |s: &[f32]| s.iter().map(|w| glyph + w).sum::<f32>() + chevron * (n - 1) as f32;
    // shrink the middle segments first (nearest the centre first), keeping the first and last labels
    let mut order: Vec<usize> = (1..n.saturating_sub(1)).collect();
    let mid = (n as f32 - 1.0) / 2.0;
    order.sort_by(|a, b| (*a as f32 - mid).abs().total_cmp(&(*b as f32 - mid).abs()));
    for i in order {
        if total(&shown) <= bounds.w {
            break;
        }
        shown[i] = 0.0;
    }
    // then the first label, then the last one is cut to what is left
    if total(&shown) > bounds.w && n > 1 {
        shown[0] = 0.0;
    }
    let over = total(&shown) - bounds.w;
    if over > 0.0 {
        shown[n - 1] = (shown[n - 1] - over).max(0.0);
    }
    let mut x = bounds.x;
    let mut out = Vec::with_capacity(n);
    for (index, w) in shown.into_iter().enumerate() {
        let rect = Rect { x, y: bounds.y, w: glyph + w, h: bounds.h };
        out.push(Segment { index, rect, label_w: w });
        x = rect.right() + chevron;
    }
    out
}

/// The segment under `(x, y)`.
pub fn hit(segments: &[Segment], x: f32, y: f32) -> Option<usize> {
    segments.iter().find(|s| s.rect.contains(x, y)).map(|s| s.index)
}

#[cfg(test)]
mod tests {
    use super::*;
    const B: Rect = Rect { x: 0.0, y: 0.0, w: 400.0, h: 20.0 };

    #[test]
    fn a_wide_bar_shows_every_label_with_chevrons_between() {
        let s = layout(&[40.0, 60.0, 50.0], B, 20.0, 12.0);
        assert_eq!(s.len(), 3);
        assert_eq!(s[1].rect.x, s[0].rect.right() + 12.0);
        assert!(s.iter().all(|g| g.label_w > 0.0));
        assert_eq!(hit(&s, s[1].rect.x + 1.0, 5.0), Some(1));
        assert_eq!(hit(&s, s[0].rect.right() + 6.0, 5.0), None, "the chevron is not a segment");
    }

    #[test]
    fn a_narrow_bar_drops_middle_labels_first_and_fits() {
        let labels = [60.0, 80.0, 80.0, 80.0, 90.0];
        let b = Rect { w: 300.0, ..B };
        let s = layout(&labels, b, 20.0, 12.0);
        assert!(s.last().unwrap().rect.right() <= b.right() + 0.01, "fits: {:?}", s.last());
        assert!(s[4].label_w > 0.0, "the current item keeps its label");
        assert_eq!(s[2].label_w, 0.0, "the centre goes first");
    }

    #[test]
    fn a_tiny_bar_cuts_the_last_label() {
        let s = layout(&[60.0, 300.0], Rect { w: 120.0, ..B }, 20.0, 12.0);
        assert_eq!(s[0].label_w, 0.0);
        assert!(s[1].rect.right() <= 120.01 && s[1].label_w < 300.0);
    }
}
