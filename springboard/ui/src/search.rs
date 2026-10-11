//! Search field geometry (`NativeSearchField`, `NativeSearchTextField`): a capsule with a leading magnifier, the text, and a
//! clear button at the trailing edge that shows only while there is text. Pure model: the SDK draws it.
use crate::Rect;

/// Side of the magnifier and clear glyphs, in points.
pub const GLYPH_PT: f32 = 14.0;
/// Space from the capsule's edge to a glyph, and from the magnifier to the text.
pub const INSET_PT: f32 = 8.0;
pub const GAP_PT: f32 = 6.0;

/// Where the parts of a search field of `bounds` go.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SearchLayout {
    pub magnifier: Rect,
    pub text: Rect,
    /// Present only while the field has text.
    pub clear: Option<Rect>,
}

pub fn layout(bounds: Rect, has_text: bool) -> SearchLayout {
    let side = GLYPH_PT.min(bounds.h);
    let cy = bounds.y + (bounds.h - side) / 2.0;
    let magnifier = Rect { x: bounds.x + INSET_PT, y: cy, w: side, h: side };
    let clear = has_text.then(|| Rect { x: bounds.right() - INSET_PT - side, y: cy, w: side, h: side });
    let left = magnifier.right() + GAP_PT;
    let right = clear.map_or(bounds.right() - INSET_PT, |c| c.x - GAP_PT);
    SearchLayout { magnifier, text: Rect { x: left, y: bounds.y, w: (right - left).max(0.0), h: bounds.h }, clear }
}

/// True when a press at `(x, y)` lands on the clear button. The target is padded to the capsule's height so it is
/// easy to hit, as the native toolkit's cancel button cell is.
pub fn hits_clear(bounds: Rect, has_text: bool, x: f32, y: f32) -> bool {
    let Some(c) = layout(bounds, has_text).clear else { return false };
    let pad = ((bounds.h - c.h) / 2.0).max(0.0);
    Rect { x: c.x - pad, y: bounds.y, w: bounds.right() - (c.x - pad), h: bounds.h }.contains(x, y)
}

#[cfg(test)]
mod tests {
    use super::*;
    const B: Rect = Rect { x: 100.0, y: 10.0, w: 200.0, h: 28.0 };

    #[test]
    fn the_clear_button_shows_only_with_text_and_narrows_the_text() {
        let empty = layout(B, false);
        assert!(empty.clear.is_none());
        let full = layout(B, true);
        let c = full.clear.unwrap();
        assert!((c.right() - (B.right() - INSET_PT)).abs() < 0.01);
        assert!(full.text.right() <= c.x && full.text.w < empty.text.w);
        assert!(full.text.x >= full.magnifier.right(), "text starts after the magnifier");
        assert_eq!(full.magnifier.y + full.magnifier.h / 2.0, B.y + B.h / 2.0, "glyphs are centred");
    }

    #[test]
    fn hitting_the_clear_button() {
        assert!(!hits_clear(B, false, 290.0, 24.0), "nothing to clear");
        assert!(hits_clear(B, true, 290.0, 24.0));
        assert!(hits_clear(B, true, 290.0, 11.0), "padded to the full height");
        assert!(!hits_clear(B, true, 150.0, 24.0), "the text area is not the button");
    }
}
