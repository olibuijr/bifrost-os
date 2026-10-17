//! The suggested-actions panel model: for a piece of text, every detected item (link, date, phone, address and so on)
//! with the actions its kind offers, in the order the text reads.
//!
//! The panel host draws these rows and runs the one tapped. The detector finds the items (`detect::find`) and the
//! action titles come from `detect::menu_actions`, so the panel and the context menu always offer the same actions.

use crate::detect::{self, Kind, Match};
use std::ops::Range;

/// One detected item and the actions it offers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Suggestion {
    pub kind: Kind,
    /// byte range of the item in the source text
    pub range: Range<usize>,
    /// the item as written in the text
    pub text: String,
    /// the action titles for this kind, first action first
    pub actions: Vec<&'static str>,
}

/// Every item of `kinds` found in `text`, in reading order. Items that no kind offers an action for are skipped.
pub fn suggest(text: &str, kinds: &[Kind]) -> Vec<Suggestion> {
    detect::find(text, kinds).into_iter().filter_map(|m| suggestion(&m, text)).collect()
}

fn suggestion(m: &Match, text: &str) -> Option<Suggestion> {
    let actions = detect::menu_actions(m.kind);
    if actions.is_empty() {
        return None;
    }
    Some(Suggestion { kind: m.kind, range: m.range.clone(), text: m.text(text).to_string(), actions: actions.to_vec() })
}

/// The action the panel runs first when an item is tapped: its first title, or `None` when the item offers nothing.
pub fn primary_action(s: &Suggestion) -> Option<&'static str> {
    s.actions.first().copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_a_link_and_offers_the_link_actions() {
        let out = suggest("see https://bifrost.olibuijr.com/docs now", &[Kind::Link]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].kind, Kind::Link);
        assert_eq!(out[0].text, "https://bifrost.olibuijr.com/docs");
        assert!(out[0].actions.contains(&"Copy Link"));
    }

    #[test]
    fn items_come_in_reading_order() {
        let text = "call 555 1234 or mail someone@example.com";
        let out = suggest(text, &[Kind::Phone, Kind::Email]);
        let kinds: Vec<Kind> = out.iter().map(|s| s.kind).collect();
        assert_eq!(kinds, [Kind::Phone, Kind::Email]);
        assert!(out[0].range.start < out[1].range.start);
    }

    #[test]
    fn only_the_requested_kinds_are_suggested() {
        let text = "https://example.com on 2026-10-09";
        let out = suggest(text, &[Kind::Date]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].kind, Kind::Date);
    }

    #[test]
    fn ranges_index_the_source_text() {
        let text = "pay $12.50 today";
        let out = suggest(text, &[Kind::Money]);
        assert_eq!(out.len(), 1);
        assert_eq!(&text[out[0].range.clone()], out[0].text);
    }

    #[test]
    fn primary_action_is_the_first_title_of_the_kind() {
        let out = suggest("call 555 1234", &[Kind::Phone]);
        assert_eq!(primary_action(&out[0]), Some("Call"));
    }

    #[test]
    fn no_text_gives_no_suggestions() {
        assert!(suggest("nothing to see here", &[Kind::Link, Kind::Email]).is_empty());
        assert!(suggest("", &[Kind::Link]).is_empty());
    }
}
