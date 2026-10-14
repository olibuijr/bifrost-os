//! Text drag and drop customization for a text view (`UITextDragDelegate`, `UITextDropDelegate`): which parts of the
//! view may be dragged, whether it takes drops, whether a drag inside the view moves the text, and what a drop does to the
//! text. Plain functions over the text and byte ranges, so a text view (or a text field) calls them from its drag handler
//! and applies the result with its own edit path.
//!
//! Operations are the `drag_session::op` bits (`COPY`, `LINK`, `MOVE`). The wire of the shell uses another encoding for
//! MOVE (`proto::rich::dragop`); the caller converts at the boundary.

use crate::drag_session::op;
use std::ops::Range;

/// The plain-text UTType the text drag items carry.
pub const TEXT_TYPE: &str = "public.utf8-plain-text";

/// What the text view allows. The defaults allow everything, as a text view does without a delegate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Policy {
    pub draggable: bool,
    pub accepts_drops: bool,
    /// Whether a drag from this view may move (not only copy) its text when it is dropped back on the same view.
    pub allows_move: bool,
}

impl Default for Policy {
    fn default() -> Self {
        Policy { draggable: true, accepts_drops: true, allows_move: true }
    }
}

/// Moves a byte index back to the nearest character boundary at or before it, within `text`.
fn snap(text: &str, i: usize) -> usize {
    let mut i = i.min(text.len());
    while !text.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// A byte range clamped to the text and snapped to character boundaries; None when it is empty.
fn normalized(text: &str, range: Range<usize>) -> Option<Range<usize>> {
    let (a, b) = (snap(text, range.start.min(range.end)), snap(text, range.start.max(range.end)));
    (a < b).then_some(a..b)
}

/// The drag item of a selection: the selected text. None when the view does not allow dragging or nothing is selected.
pub fn drag_payload(policy: &Policy, text: &str, selection: Range<usize>) -> Option<String> {
    if !policy.draggable {
        return None;
    }
    normalized(text, selection).map(|r| text[r].to_string())
}

/// A drop the view answers with: the operation and the byte index the text goes in at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Proposal {
    pub operation: u8,
    pub insert_at: usize,
}

/// `textDroppableView(_:proposalForDrop:)`. `mask` is the operations the source allows. `source` is the selection being
/// dragged when the drag began in this view. A move onto the dragged selection itself is refused as a move and falls back
/// to a copy, so the text is never dropped into its own place as a removal.
pub fn propose(policy: &Policy, text: &str, source: Option<Range<usize>>, point: usize, mask: u8) -> Option<Proposal> {
    if !policy.accepts_drops {
        return None;
    }
    let at = snap(text, point);
    let inside_source = source.clone().and_then(|s| normalized(text, s)).map_or(false, |s| at >= s.start && at <= s.end);
    let same_view_move = source.is_some() && policy.allows_move && !inside_source && mask & op::MOVE != 0;
    if same_view_move {
        return Some(Proposal { operation: op::MOVE, insert_at: at });
    }
    [op::COPY, op::LINK].into_iter().find(|bit| mask & bit != 0).map(|operation| Proposal { operation, insert_at: at })
}

/// The text after a drop: the new text and the caret after the inserted payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Drop {
    pub text: String,
    pub caret: usize,
}

/// `performDropWith`: inserts `payload` at `insert_at`. A move within the same view removes the dragged selection first,
/// and the insert point moves with the text that was before it. Returns None for an empty payload or a move onto the
/// selection itself.
pub fn apply_drop(text: &str, source: Option<Range<usize>>, insert_at: usize, payload: &str, operation: u8) -> Option<Drop> {
    if payload.is_empty() {
        return None;
    }
    let at = snap(text, insert_at);
    if operation == op::MOVE {
        if let Some(src) = source.and_then(|s| normalized(text, s)) {
            if at >= src.start && at <= src.end {
                return None;
            }
            let removed = src.end - src.start;
            let at = if at > src.end { at - removed } else { at };
            let mut kept = String::with_capacity(text.len());
            kept.push_str(&text[..src.start]);
            kept.push_str(&text[src.end..]);
            kept.insert_str(at, payload);
            return Some(Drop { caret: at + payload.len(), text: kept });
        }
    }
    let mut out = text.to_string();
    out.insert_str(at, payload);
    Some(Drop { caret: at + payload.len(), text: out })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_is_the_selected_text() {
        let p = Policy::default();
        assert_eq!(drag_payload(&p, "hello world", 6..11).as_deref(), Some("world"));
        assert_eq!(drag_payload(&p, "hello", 2..2), None, "an empty selection drags nothing");
    }

    #[test]
    fn a_non_draggable_view_gives_no_payload() {
        let p = Policy { draggable: false, ..Policy::default() };
        assert_eq!(drag_payload(&p, "hello", 0..5), None);
    }

    #[test]
    fn selection_is_clamped_and_snapped_to_characters() {
        // "å" is two bytes; a range ending inside it snaps back to the boundary before it
        let text = "aåb";
        assert_eq!(drag_payload(&Policy::default(), text, 0..2).as_deref(), Some("a"));
        assert_eq!(drag_payload(&Policy::default(), text, 1..99).as_deref(), Some("åb"));
        assert_eq!(drag_payload(&Policy::default(), text, 4..1).as_deref(), Some("åb"), "reversed range is ordered");
    }

    #[test]
    fn a_view_that_refuses_drops_proposes_nothing() {
        let p = Policy { accepts_drops: false, ..Policy::default() };
        assert_eq!(propose(&p, "abc", None, 1, op::COPY), None);
    }

    #[test]
    fn copy_or_link_proposed_from_another_view() {
        let p = Policy::default();
        assert_eq!(propose(&p, "abc", None, 1, op::COPY | op::LINK), Some(Proposal { operation: op::COPY, insert_at: 1 }));
        assert_eq!(propose(&p, "abc", None, 1, op::LINK), Some(Proposal { operation: op::LINK, insert_at: 1 }));
        assert_eq!(propose(&p, "abc", None, 1, op::MOVE), None, "a move from another view is not this view's to propose");
    }

    #[test]
    fn same_view_drag_proposes_move_outside_the_selection() {
        let p = Policy::default();
        let got = propose(&p, "abcdef", Some(1..3), 5, op::COPY | op::MOVE);
        assert_eq!(got, Some(Proposal { operation: op::MOVE, insert_at: 5 }));
    }

    #[test]
    fn dropping_into_the_dragged_selection_falls_back_to_copy() {
        let p = Policy::default();
        let got = propose(&p, "abcdef", Some(1..3), 2, op::COPY | op::MOVE);
        assert_eq!(got, Some(Proposal { operation: op::COPY, insert_at: 2 }));
    }

    #[test]
    fn move_disabled_by_policy_copies_on_the_same_view() {
        let p = Policy { allows_move: false, ..Policy::default() };
        let got = propose(&p, "abcdef", Some(1..3), 5, op::COPY | op::MOVE);
        assert_eq!(got, Some(Proposal { operation: op::COPY, insert_at: 5 }));
    }

    #[test]
    fn copy_inserts_and_keeps_the_source() {
        let d = apply_drop("abcdef", Some(1..3), 5, "bc", op::COPY).unwrap();
        assert_eq!(d.text, "abcdebcf");
        assert_eq!(d.caret, 7);
    }

    #[test]
    fn move_forward_removes_the_source_and_shifts_the_insert_point() {
        // remove "bc" (1..3) -> "adef", then insert "bc" at 5 - 2 = 3 -> "adebcf"
        let d = apply_drop("abcdef", Some(1..3), 5, "bc", op::MOVE).unwrap();
        assert_eq!(d.text, "adebcf");
        assert_eq!(d.caret, 5);
    }

    #[test]
    fn move_backward_inserts_before_the_removed_part() {
        let d = apply_drop("abcdef", Some(3..5), 1, "de", op::MOVE).unwrap();
        assert_eq!(d.text, "adebcf");
        assert_eq!(d.caret, 3);
    }

    #[test]
    fn move_onto_the_selection_is_refused_and_empty_payloads_are_nothing() {
        assert_eq!(apply_drop("abcdef", Some(1..3), 2, "bc", op::MOVE), None);
        assert_eq!(apply_drop("abcdef", None, 2, "", op::COPY), None);
    }

    #[test]
    fn cross_view_drop_inserts_without_removal() {
        let d = apply_drop("xy", None, 1, "QQ", op::MOVE).unwrap();
        assert_eq!(d.text, "xQQy");
        assert_eq!(d.caret, 3);
    }

    #[test]
    fn insert_point_inside_a_character_snaps_back() {
        let d = apply_drop("aåb", None, 2, "Z", op::COPY).unwrap();
        assert_eq!(d.text, "aZåb");
        assert_eq!(d.caret, 2);
    }
}
