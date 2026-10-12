//! Text markers and marker ranges over one text element: the opaque positions an assistive client holds
//! (`AXTextMarker`, `AXTextMarkerRange`). A marker is an element and a character offset; a range is two markers in
//! the same element, ordered so `start <= end`. Offsets are Unicode scalar (char) offsets, the same units as
//! [`crate::ax::AxText`]. [`char_to_utf16`] and [`utf16_to_char`] convert to the UTF-16 units that attributed-string
//! ranges use. parity: AXTextMarker, AXTextMarkerRange, AXTextAttributedString (text markers and ranges).
use crate::ax::AxText;

/// A position inside one text element (`AXTextMarker`). Markers order by element, then offset.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Default, Hash)]
pub struct TextMarker {
    /// the `Node::id` of the text element
    pub element: u64,
    /// characters from the start of the element's text
    pub offset: u32,
}

impl TextMarker {
    pub fn new(element: u64, offset: u32) -> Self {
        TextMarker { element, offset }
    }

    /// The marker at `offset`, clamped to `0..=len` (the element's character count).
    pub fn clamped(element: u64, offset: i64, len: u32) -> Self {
        TextMarker { element, offset: offset.clamp(0, len as i64) as u32 }
    }

    /// The insertion point of `ax` (`AXTextMarker` of the caret).
    pub fn caret(ax: &AxText) -> Self {
        TextMarker { element: ax.id, offset: ax.caret }
    }

    /// This marker moved by `delta` characters, clamped to `0..=len`.
    pub fn moved(self, delta: i64, len: u32) -> Self {
        TextMarker::clamped(self.element, self.offset as i64 + delta, len)
    }
}

/// A span of text in one element (`AXTextMarkerRange`). Build it with [`TextMarkerRange::new`] or one of the
/// constructors below; they keep `start <= end`, so `len` never underflows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct TextMarkerRange {
    pub start: TextMarker,
    pub end: TextMarker,
}

impl TextMarkerRange {
    /// The range between two markers, in either order. `None` when they are in different elements.
    pub fn new(a: TextMarker, b: TextMarker) -> Option<Self> {
        if a.element != b.element {
            return None;
        }
        let (start, end) = if a <= b { (a, b) } else { (b, a) };
        Some(TextMarkerRange { start, end })
    }

    /// `AXSelectedTextMarkerRange`: the selection of `ax`, empty when only a caret.
    pub fn from_selection(ax: &AxText) -> Self {
        TextMarkerRange {
            start: TextMarker::new(ax.id, ax.sel_start),
            end: TextMarker::new(ax.id, ax.sel_end.max(ax.sel_start)),
        }
    }

    /// `AXVisibleTextMarkerRange`: the part of `ax` not scrolled out of view.
    pub fn visible(ax: &AxText) -> Self {
        TextMarkerRange {
            start: TextMarker::new(ax.id, ax.vis_start),
            end: TextMarker::new(ax.id, ax.vis_end.max(ax.vis_start)),
        }
    }

    /// The element both markers belong to.
    pub fn element(&self) -> u64 {
        self.start.element
    }

    /// Characters in the range.
    pub fn len(&self) -> u32 {
        self.end.offset - self.start.offset
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Whether `m` is a position of this range. Both ends count, so a caret at `end` is inside.
    pub fn contains(&self, m: TextMarker) -> bool {
        m.element == self.start.element && self.start.offset <= m.offset && m.offset <= self.end.offset
    }

    /// The part of both ranges, or `None` when they are in different elements or do not touch. Touching ranges give an
    /// empty range.
    pub fn intersection(&self, other: &Self) -> Option<Self> {
        if self.element() != other.element() {
            return None;
        }
        let start = self.start.max(other.start);
        let end = self.end.min(other.end);
        if start <= end { Some(TextMarkerRange { start, end }) } else { None }
    }

    /// The span covering both ranges, or `None` when they are in different elements or leave a gap between them.
    pub fn union(&self, other: &Self) -> Option<Self> {
        if self.element() != other.element() {
            return None;
        }
        if self.start <= other.end && other.start <= self.end {
            Some(TextMarkerRange { start: self.start.min(other.start), end: self.end.max(other.end) })
        } else {
            None
        }
    }

    /// The text of the range (`AXStringForTextMarkerRange`). A secure field answers an empty string, so the masked
    /// content never leaves the element. Offsets past the end of `text` are clipped.
    pub fn string_in(&self, text: &str, secure: bool) -> String {
        if secure {
            return String::new();
        }
        text.chars().skip(self.start.offset as usize).take(self.len() as usize).collect()
    }

    /// The range as `(start, length)` in UTF-16 units, the units attributed-string ranges use.
    pub fn utf16_range(&self, text: &str) -> (usize, usize) {
        let start = char_to_utf16(text, self.start.offset);
        let end = char_to_utf16(text, self.end.offset);
        (start, end - start)
    }
}

/// UTF-16 units before char `offset` of `text`. Offsets past the end count the whole string.
pub fn char_to_utf16(text: &str, offset: u32) -> usize {
    text.chars().take(offset as usize).map(char::len_utf16).sum()
}

/// The char offset at UTF-16 unit `units` of `text`. `None` when `units` falls inside a surrogate pair or past the end.
pub fn utf16_to_char(text: &str, units: usize) -> Option<u32> {
    let mut acc = 0usize;
    let mut n = 0u32;
    for c in text.chars() {
        if acc == units {
            return Some(n);
        }
        if acc > units {
            return None;
        }
        acc += c.len_utf16();
        n += 1;
    }
    if acc == units { Some(n) } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sel(id: u64, caret: u32, a: u32, b: u32, len: u32) -> AxText {
        AxText { id, caret, sel_start: a, sel_end: b, vis_start: 0, vis_end: len, secure: false, len }
    }

    #[test]
    fn new_orders_markers_and_refuses_two_elements() {
        let r = TextMarkerRange::new(TextMarker::new(7, 5), TextMarker::new(7, 2)).unwrap();
        assert_eq!((r.start.offset, r.end.offset, r.len()), (2, 5, 3));
        assert!(TextMarkerRange::new(TextMarker::new(7, 0), TextMarker::new(8, 0)).is_none());
    }

    #[test]
    fn selection_range_and_its_text() {
        let ax = sel(3, 4, 1, 4, 10);
        let r = TextMarkerRange::from_selection(&ax);
        assert_eq!((r.start.offset, r.end.offset, r.len()), (1, 4, 3));
        assert_eq!(r.string_in("abcdefghij", false), "bcd");
        assert_eq!(TextMarker::caret(&ax).offset, 4);
    }

    #[test]
    fn secure_field_gives_no_text() {
        let r = TextMarkerRange::new(TextMarker::new(1, 0), TextMarker::new(1, 3)).unwrap();
        assert_eq!(r.string_in("hunter2", true), "");
        assert_eq!(r.string_in("hunter2", false), "hun");
    }

    #[test]
    fn intersection_and_union() {
        let a = TextMarkerRange::new(TextMarker::new(1, 2), TextMarker::new(1, 6)).unwrap();
        let b = TextMarkerRange::new(TextMarker::new(1, 4), TextMarker::new(1, 9)).unwrap();
        let i = a.intersection(&b).unwrap();
        assert_eq!((i.start.offset, i.end.offset), (4, 6));
        let u = a.union(&b).unwrap();
        assert_eq!((u.start.offset, u.end.offset), (2, 9));

        let c = TextMarkerRange::new(TextMarker::new(1, 0), TextMarker::new(1, 2)).unwrap();
        let d = TextMarkerRange::new(TextMarker::new(1, 5), TextMarker::new(1, 7)).unwrap();
        assert!(c.intersection(&d).is_none());
        assert!(c.union(&d).is_none());
        // touching ranges: the intersection is empty, the union is joined
        let e = TextMarkerRange::new(TextMarker::new(1, 2), TextMarker::new(1, 4)).unwrap();
        assert!(c.intersection(&e).unwrap().is_empty());
        assert_eq!(c.union(&e).unwrap().len(), 4);
    }

    #[test]
    fn contains_counts_both_ends_and_only_its_element() {
        let r = TextMarkerRange::new(TextMarker::new(2, 2), TextMarker::new(2, 5)).unwrap();
        assert!(r.contains(TextMarker::new(2, 5)));
        assert!(!r.contains(TextMarker::new(2, 6)));
        assert!(!r.contains(TextMarker::new(3, 3)));
    }

    #[test]
    fn moving_clamps_to_the_element() {
        assert_eq!(TextMarker::new(1, 2).moved(-5, 4).offset, 0);
        assert_eq!(TextMarker::new(1, 2).moved(10, 4).offset, 4);
        assert_eq!(TextMarker::new(1, 2).moved(1, 4).offset, 3);
    }

    #[test]
    fn utf16_units_around_a_surrogate_pair() {
        // "a" is 1 unit, the emoji is 2 units, "b" is 1 unit
        let text = "a\u{1F600}b";
        let r = TextMarkerRange::new(TextMarker::new(1, 1), TextMarker::new(1, 3)).unwrap();
        assert_eq!(r.utf16_range(text), (1, 3));
        assert_eq!(char_to_utf16(text, 3), 4);
        assert_eq!(utf16_to_char(text, 1), Some(1));
        assert_eq!(utf16_to_char(text, 2), None);
        assert_eq!(utf16_to_char(text, 3), Some(2));
        assert_eq!(utf16_to_char(text, 5), None);
    }
}
