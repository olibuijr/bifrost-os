//! Find Next, Find Previous and Use Selection for Find: stepping the selection through the rows that match a find
//! text, without filtering the table. Pure: the window hands in the shown rows and the match test.

use crate::model::LogEntry;

/// The row to select after `current` (or before it, going backwards) among `rows` for which `matches` is true,
/// wrapping past either end. With no current row (or one no longer shown) a forward search starts at the first row
/// and a backward one at the last. The current row itself is returned only when it is the one match.
pub fn step(rows: &[u64], current: Option<u64>, forward: bool, matches: impl Fn(u64) -> bool) -> Option<u64> {
    let n = rows.len();
    if n == 0 {
        return None;
    }
    let at = current.and_then(|c| rows.iter().position(|r| *r == c));
    (1..=n)
        .map(|k| match (at, forward) {
            (Some(i), true) => (i + k) % n,
            (Some(i), false) => (i + n - k % n) % n,
            (None, true) => k - 1,
            (None, false) => n - k,
        })
        .map(|i| rows[i])
        .find(|r| matches(*r))
}

/// The find text that stands for a selected message: its first non-blank line, cut to 80 characters, as one quoted
/// phrase so the whole line must appear. Empty when the message has no text.
pub fn text_for(entry: &LogEntry) -> String {
    const MAX_CHARS: usize = 80;
    let line = entry.message.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    let phrase: String = line.chars().filter(|c| *c != '"').take(MAX_CHARS).collect();
    let phrase = phrase.trim();
    if phrase.is_empty() {
        String::new()
    } else {
        format!("\"{phrase}\"")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn even(r: u64) -> bool {
        r % 2 == 0
    }

    #[test]
    fn forward_goes_to_the_next_match_and_wraps() {
        let rows = [10, 11, 12, 13, 14];
        assert_eq!(step(&rows, Some(10), true, even), Some(12));
        assert_eq!(step(&rows, Some(12), true, even), Some(14));
        assert_eq!(step(&rows, Some(14), true, even), Some(10), "wraps to the first match");
    }

    #[test]
    fn backward_goes_to_the_previous_match_and_wraps() {
        let rows = [10, 11, 12, 13, 14];
        assert_eq!(step(&rows, Some(14), false, even), Some(12));
        assert_eq!(step(&rows, Some(10), false, even), Some(14));
        assert_eq!(step(&rows, Some(13), false, even), Some(12), "from a row that does not match");
    }

    #[test]
    fn with_nothing_selected_the_search_starts_at_an_end() {
        let rows = [10, 11, 12, 13, 14, 15];
        assert_eq!(step(&rows, None, true, |r| r >= 11), Some(11));
        assert_eq!(step(&rows, None, false, |r| r <= 14), Some(14));
        assert_eq!(step(&rows, Some(99), true, even), Some(10), "a selection that left the list counts as none");
    }

    #[test]
    fn the_only_match_is_found_again_and_no_match_is_none() {
        let rows = [1, 2, 3];
        assert_eq!(step(&rows, Some(2), true, even), Some(2));
        assert_eq!(step(&rows, Some(2), false, even), Some(2));
        assert_eq!(step(&rows, Some(1), true, |_| false), None);
        assert_eq!(step(&[], None, true, |_| true), None);
    }

    #[test]
    fn selection_text_is_one_quoted_phrase() {
        let e = |m: &str| LogEntry { message: m.into(), ..LogEntry::default() };
        assert_eq!(text_for(&e("disk full on sda1")), "\"disk full on sda1\"");
        assert_eq!(text_for(&e("\n  first line  \nsecond")), "\"first line\"");
        assert_eq!(text_for(&e("say \"hi\"")), "\"say hi\"");
        assert_eq!(text_for(&e("   ")), "");
        assert_eq!(text_for(&e(&"x".repeat(200))).chars().count(), 82, "80 characters and two quotes");
    }
}
