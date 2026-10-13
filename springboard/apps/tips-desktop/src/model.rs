//! The tips model: collections, scopes, search, the New markers, reading order and text wrapping. Pure logic with
//! unit tests; drawing and input live in `app.rs`.

use std::collections::BTreeSet;

/// A group of cards in the sidebar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Collection {
    GettingStarted,
    Desktop,
    Windows,
    Files,
    Apps,
}

impl Collection {
    /// Every collection, in sidebar order.
    pub const ALL: [Collection; 5] = [
        Collection::GettingStarted,
        Collection::Desktop,
        Collection::Windows,
        Collection::Files,
        Collection::Apps,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Collection::GettingStarted => "Getting Started",
            Collection::Desktop => "Desktop",
            Collection::Windows => "Windows",
            Collection::Files => "Files",
            Collection::Apps => "Apps",
        }
    }
}

/// One how-to card. The text is plain prose; `keywords` adds search terms the body does not use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tip {
    pub id: &'static str,
    pub collection: Collection,
    pub title: &'static str,
    pub body: &'static str,
    pub keywords: &'static str,
}

/// What the list shows: every card, the unread cards, or one collection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    All,
    New,
    Collection(Collection),
}

impl Scope {
    pub fn title(self) -> &'static str {
        match self {
            Scope::All => "All Tips",
            Scope::New => "New",
            Scope::Collection(c) => c.title(),
        }
    }

    /// True when `tip` belongs in this scope. `New` means the card has not been opened yet.
    pub fn includes(self, tip: &Tip, seen: &Seen) -> bool {
        match self {
            Scope::All => true,
            Scope::New => !seen.contains(tip.id),
            Scope::Collection(c) => tip.collection == c,
        }
    }
}

/// True when every word of `query` appears in the card's title, text, keywords or collection name. Case does not
/// matter, and an empty query matches every card.
pub fn matches(tip: &Tip, query: &str) -> bool {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    if words.is_empty() {
        return true;
    }
    let haystack = format!("{} {} {} {}", tip.title, tip.body, tip.keywords, tip.collection.title()).to_lowercase();
    words.iter().all(|word| haystack.contains(word.as_str()))
}

/// Indices into `tips` of the cards in `scope` that match `query`, in catalogue order.
pub fn visible(tips: &[Tip], scope: Scope, query: &str, seen: &Seen) -> Vec<usize> {
    tips.iter()
        .enumerate()
        .filter(|(_, tip)| scope.includes(tip, seen) && matches(tip, query))
        .map(|(index, _)| index)
        .collect()
}

/// How many cards have not been opened yet.
pub fn new_count(tips: &[Tip], seen: &Seen) -> usize {
    tips.iter().filter(|tip| !seen.contains(tip.id)).count()
}

/// The card to show after `current` in `order` (the visible list), one step forward or back. The ends do not wrap.
/// When `current` is not in `order` (the search changed under it), the first card of `order` is returned.
pub fn step(order: &[usize], current: usize, forward: bool) -> Option<usize> {
    let position = order.iter().position(|&index| index == current);
    let next = match (position, forward) {
        (None, _) => 0,
        (Some(p), true) => (p + 1).min(order.len().checked_sub(1)?),
        (Some(p), false) => p.saturating_sub(1),
    };
    order.get(next).copied()
}

/// The ids of the cards that have been opened. Stored one id per line.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Seen {
    ids: BTreeSet<String>,
}

impl Seen {
    /// Reads the stored form. Blank lines are ignored.
    pub fn parse(text: &str) -> Self {
        Seen {
            ids: text.lines().map(str::trim).filter(|line| !line.is_empty()).map(String::from).collect(),
        }
    }

    /// The stored form: one id per line, in sorted order.
    pub fn to_text(&self) -> String {
        self.ids.iter().map(|id| format!("{id}\n")).collect()
    }

    pub fn contains(&self, id: &str) -> bool {
        self.ids.contains(id)
    }

    /// Marks one card as read. True when the card was unread until now.
    pub fn mark(&mut self, id: &str) -> bool {
        self.ids.insert(id.to_string())
    }

    /// Marks every card in `tips` as read. True when anything changed.
    pub fn mark_all(&mut self, tips: &[Tip]) -> bool {
        tips.iter().fold(false, |changed, tip| self.mark(tip.id) || changed)
    }
}

/// Splits `text` into lines no wider than `max_width`, breaking between words. A single word wider than the
/// width gets a line of its own. `measure` returns the width of a string in the same unit as `max_width`.
pub fn wrap(text: &str, max_width: f32, measure: impl Fn(&str) -> f32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if line.is_empty() {
            line.push_str(word);
            continue;
        }
        let candidate = format!("{line} {word}");
        if measure(candidate.as_str()) <= max_width {
            line = candidate;
        } else {
            lines.push(std::mem::take(&mut line));
            line.push_str(word);
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// `wrap`, cut to `max_lines`. A cut line ends with an ellipsis and is shortened until it fits.
pub fn lines(text: &str, max_width: f32, max_lines: usize, measure: impl Fn(&str) -> f32) -> Vec<String> {
    let mut wrapped = wrap(text, max_width, &measure);
    if wrapped.len() <= max_lines {
        return wrapped;
    }
    wrapped.truncate(max_lines);
    if let Some(last) = wrapped.last_mut() {
        let mut shown = last.clone();
        while !shown.is_empty() && measure(format!("{shown}\u{2026}").as_str()) > max_width {
            shown.pop();
        }
        *last = format!("{}\u{2026}", shown.trim_end());
    }
    wrapped
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: [Tip; 3] = [
        Tip {
            id: "one",
            collection: Collection::Files,
            title: "Path bar",
            body: "Click a segment to go to that folder.",
            keywords: "breadcrumb navigate",
        },
        Tip {
            id: "two",
            collection: Collection::Apps,
            title: "Copy a result",
            body: "Choose Copy from the Edit menu.",
            keywords: "calculator clipboard",
        },
        Tip {
            id: "three",
            collection: Collection::Desktop,
            title: "The Dock",
            body: "Open apps from the Dock.",
            keywords: "launch",
        },
    ];

    fn chars(s: &str) -> f32 {
        s.chars().count() as f32
    }

    #[test]
    fn empty_query_matches_everything() {
        assert!(SAMPLE.iter().all(|tip| matches(tip, "")));
        assert!(SAMPLE.iter().all(|tip| matches(tip, "   ")));
    }

    #[test]
    fn search_ignores_case_and_checks_every_field() {
        assert!(matches(&SAMPLE[0], "PATH"));
        assert!(matches(&SAMPLE[0], "breadcrumb"));
        assert!(matches(&SAMPLE[0], "files"));
        assert!(!matches(&SAMPLE[1], "files"));
        assert!(matches(&SAMPLE[1], "clipboard"));
    }

    #[test]
    fn every_word_must_match() {
        assert!(matches(&SAMPLE[1], "copy calculator"));
        assert!(!matches(&SAMPLE[1], "copy folder"));
    }

    #[test]
    fn scopes_filter_by_collection_and_read_state() {
        let mut seen = Seen::default();
        assert_eq!(visible(&SAMPLE, Scope::All, "", &seen), vec![0, 1, 2]);
        assert_eq!(visible(&SAMPLE, Scope::Collection(Collection::Apps), "", &seen), vec![1]);
        assert_eq!(visible(&SAMPLE, Scope::New, "", &seen), vec![0, 1, 2]);
        seen.mark("two");
        assert_eq!(visible(&SAMPLE, Scope::New, "", &seen), vec![0, 2]);
        assert_eq!(visible(&SAMPLE, Scope::All, "dock", &seen), vec![2]);
    }

    #[test]
    fn new_count_counts_unread_cards() {
        let mut seen = Seen::default();
        assert_eq!(new_count(&SAMPLE, &seen), 3);
        seen.mark("one");
        assert_eq!(new_count(&SAMPLE, &seen), 2);
    }

    #[test]
    fn marking_reports_whether_anything_changed() {
        let mut seen = Seen::default();
        assert!(seen.mark("one"));
        assert!(!seen.mark("one"));
        assert!(seen.mark_all(&SAMPLE));
        assert!(!seen.mark_all(&SAMPLE));
        assert_eq!(new_count(&SAMPLE, &seen), 0);
    }

    #[test]
    fn seen_round_trips_through_its_text_form() {
        let mut seen = Seen::default();
        seen.mark("b-card");
        seen.mark("a-card");
        let text = seen.to_text();
        assert_eq!(text, "a-card\nb-card\n");
        assert_eq!(Seen::parse(&text), seen);
    }

    #[test]
    fn parse_skips_blank_lines_and_whitespace() {
        let seen = Seen::parse("  one \n\n\ntwo\r\n");
        assert!(seen.contains("one"));
        assert!(seen.contains("two"));
        assert_eq!(seen.to_text(), "one\ntwo\n");
    }

    #[test]
    fn step_moves_within_the_order_and_stops_at_the_ends() {
        let order = [4, 7, 9];
        assert_eq!(step(&order, 4, true), Some(7));
        assert_eq!(step(&order, 7, false), Some(4));
        assert_eq!(step(&order, 9, true), Some(9));
        assert_eq!(step(&order, 4, false), Some(4));
    }

    #[test]
    fn step_falls_back_to_the_first_card_when_the_current_one_is_gone() {
        assert_eq!(step(&[4, 7], 3, true), Some(4));
        assert_eq!(step(&[], 3, true), None);
    }

    #[test]
    fn wrap_breaks_between_words_within_the_width() {
        assert_eq!(wrap("aa bb cc", 5.0, chars), vec!["aa bb", "cc"]);
        assert_eq!(wrap("aa bb cc", 2.0, chars), vec!["aa", "bb", "cc"]);
        assert_eq!(wrap("  spaced   out  ", 100.0, chars), vec!["spaced out"]);
    }

    #[test]
    fn wrap_keeps_an_overlong_word_on_its_own_line() {
        assert_eq!(wrap("abcdefgh x", 3.0, chars), vec!["abcdefgh", "x"]);
        assert!(wrap("", 10.0, chars).is_empty());
    }

    #[test]
    fn lines_cuts_to_the_limit_with_an_ellipsis_that_fits() {
        let cut = lines("aa bb cc dd ee", 5.0, 2, chars);
        assert_eq!(cut.len(), 2);
        assert_eq!(cut[0], "aa bb");
        assert!(cut[1].ends_with('\u{2026}'));
        assert!(chars(&cut[1]) <= 5.0);
        assert_eq!(lines("short", 10.0, 2, chars), vec!["short"]);
    }
}
