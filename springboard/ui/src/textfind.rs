//! Find bar model for a text (the find and replace actions of a text view): the query, its match options, every
//! match in the text, the selected match and the "n of m" counter. Pure model: the SDK draws the bar and the text view
//! selects the returned range. Ranges are byte offsets into the text, on character boundaries.
use std::ops::Range;

/// How the query is compared with the text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    /// Letters compare by their lowercase form. The find bar searches case-insensitively by default.
    pub ignore_case: bool,
    /// A match must not touch a letter or digit on either side.
    pub whole_word: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options { ignore_case: true, whole_word: false }
    }
}

/// One character of a text: its byte range and the character it compares as.
type Unit = (Range<usize>, char);

fn units(s: &str, ignore_case: bool) -> Vec<Unit> {
    s.char_indices()
        .map(|(i, c)| {
            let f = if ignore_case { c.to_lowercase().next().unwrap_or(c) } else { c };
            (i..i + c.len_utf8(), f)
        })
        .collect()
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric()
}

/// Every match of `query` in `text`, in order and without overlaps. An empty query matches nothing.
pub fn find_all(text: &str, query: &str, opts: Options) -> Vec<Range<usize>> {
    let q: Vec<char> = units(query, opts.ignore_case).into_iter().map(|u| u.1).collect();
    let t = units(text, opts.ignore_case);
    let mut out = Vec::new();
    if q.is_empty() || q.len() > t.len() {
        return out;
    }
    let mut i = 0;
    while i + q.len() <= t.len() {
        let end = i + q.len();
        let hit = (0..q.len()).all(|k| t[i + k].1 == q[k]);
        let whole = !opts.whole_word
            || ((i == 0 || !is_word(t[i - 1].1)) && (end == t.len() || !is_word(t[end].1)));
        if hit && whole {
            out.push(t[i].0.start..t[end - 1].0.end);
            i = end;
        } else {
            i += 1;
        }
    }
    out
}

/// `text` with each range replaced by `with`. The ranges must be in order and must not overlap.
fn splice(text: &str, ranges: &[Range<usize>], with: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for r in ranges {
        out.push_str(&text[at..r.start]);
        out.push_str(with);
        at = r.end;
    }
    out.push_str(&text[at..]);
    out
}

/// `text` with every match of `query` replaced by `with`, and how many were replaced.
pub fn replace_all(text: &str, query: &str, opts: Options, with: &str) -> (String, usize) {
    let found = find_all(text, query, opts);
    let n = found.len();
    (splice(text, &found, with), n)
}

/// The find bar's state for one text: the query, its matches and the selected match.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TextFinder {
    query: String,
    options: Options,
    matches: Vec<Range<usize>>,
    selected: Option<usize>,
}

impl TextFinder {
    pub fn new(options: Options) -> Self {
        TextFinder { options, ..Self::default() }
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn options(&self) -> Options {
        self.options
    }

    pub fn matches(&self) -> &[Range<usize>] {
        &self.matches
    }

    pub fn count(&self) -> usize {
        self.matches.len()
    }

    /// The selected match, if any.
    pub fn selected(&self) -> Option<Range<usize>> {
        self.selected.map(|i| self.matches[i].clone())
    }

    /// The counter of the bar: the selected match's place and the total, both counted from 1. None without a selection.
    pub fn position(&self) -> Option<(usize, usize)> {
        self.selected.map(|i| (i + 1, self.matches.len()))
    }

    /// Sets the query and searches `text`. Any selection is cleared.
    pub fn set_query(&mut self, text: &str, query: &str) {
        self.query = query.to_string();
        self.search(text);
        self.selected = None;
    }

    /// Changes the match options and searches `text` again. Any selection is cleared.
    pub fn set_options(&mut self, text: &str, options: Options) {
        self.options = options;
        self.search(text);
        self.selected = None;
    }

    /// Searches `text` again after it changed. The selection keeps its match number while that match still exists.
    pub fn refresh(&mut self, text: &str) {
        let keep = self.selected;
        self.search(text);
        self.selected = keep.filter(|&i| i < self.matches.len());
    }

    fn search(&mut self, text: &str) {
        self.matches = find_all(text, &self.query, self.options);
    }

    fn select(&mut self, i: usize) -> Option<Range<usize>> {
        self.selected = Some(i);
        Some(self.matches[i].clone())
    }

    /// Selects the first match that starts at or after `from`, wrapping to the first match when none does.
    pub fn find_next(&mut self, from: usize) -> Option<Range<usize>> {
        if self.matches.is_empty() {
            self.selected = None;
            return None;
        }
        let i = self.matches.iter().position(|m| m.start >= from).unwrap_or(0);
        self.select(i)
    }

    /// Selects the last match that starts before `from`, wrapping to the last match when none does.
    pub fn find_previous(&mut self, from: usize) -> Option<Range<usize>> {
        if self.matches.is_empty() {
            self.selected = None;
            return None;
        }
        let i = self.matches.iter().rposition(|m| m.start < from).unwrap_or(self.matches.len() - 1);
        self.select(i)
    }

    /// `text` with the selected match replaced by `with`, or None without a selection. The matches are those of the
    /// last search, so call `refresh` with the new text before searching again.
    pub fn replace_selected(&self, text: &str, with: &str) -> Option<String> {
        let r = self.selected()?;
        Some(splice(text, &[r], with))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hits<'a>(text: &'a str, q: &str) -> Vec<&'a str> {
        find_all(text, q, Options::default()).into_iter().map(|r| &text[r]).collect()
    }

    #[test]
    fn ignores_case_by_default_and_reports_byte_ranges() {
        let text = "Árni árni ÁRNI";
        assert_eq!(hits(text, "árni"), vec!["Árni", "árni", "ÁRNI"]);
        // "Á" is two bytes, so the first match is 0..5 and the second starts after the space at 5.
        assert_eq!(find_all(text, "árni", Options::default())[0], 0..5);
        assert_eq!(find_all(text, "árni", Options::default())[1], 6..11);
    }

    #[test]
    fn case_sensitive_matching_takes_only_the_exact_case() {
        let opts = Options { ignore_case: false, whole_word: false };
        let found: Vec<&str> = find_all("Árni árni", "Árni", opts).into_iter().map(|r| &"Árni árni"[r]).collect();
        assert_eq!(found, vec!["Árni"]);
    }

    #[test]
    fn whole_word_skips_matches_inside_longer_words() {
        let text = "cat concatenate cat.";
        assert_eq!(hits(text, "cat").len(), 3, "substring matching finds the one inside concatenate");
        let whole = Options { ignore_case: true, whole_word: true };
        assert_eq!(find_all(text, "cat", whole), vec![0..3, 16..19]);
    }

    #[test]
    fn matches_do_not_overlap() {
        assert_eq!(find_all("aaaa", "aa", Options::default()), vec![0..2, 2..4]);
    }

    #[test]
    fn empty_and_oversized_queries_match_nothing() {
        assert!(find_all("abc", "", Options::default()).is_empty());
        assert!(find_all("ab", "abc", Options::default()).is_empty());
    }

    #[test]
    fn next_and_previous_wrap_around_and_report_the_position() {
        let text = "x cat, cat, cat";
        let mut f = TextFinder::new(Options::default());
        f.set_query(text, "cat");
        assert_eq!(f.count(), 3);
        assert_eq!(f.find_next(0), Some(2..5));
        assert_eq!(f.position(), Some((1, 3)));
        assert_eq!(f.find_next(3), Some(7..10));
        assert_eq!(f.position(), Some((2, 3)));
        assert_eq!(f.find_next(13), Some(2..5), "past the last match wraps to the first");
        assert_eq!(f.find_previous(2), Some(12..15), "before the first match wraps to the last");
        assert_eq!(f.position(), Some((3, 3)));
        assert_eq!(f.find_previous(12), Some(7..10));
    }

    #[test]
    fn replacing_one_match_or_all_matches() {
        let text = "one two one";
        let mut f = TextFinder::new(Options::default());
        f.set_query(text, "one");
        assert!(f.replace_selected(text, "1").is_none(), "nothing is selected yet");
        f.find_next(0);
        assert_eq!(f.replace_selected(text, "1").as_deref(), Some("1 two one"));
        assert_eq!(replace_all(text, "one", Options::default(), "1"), ("1 two 1".to_string(), 2));
    }

    #[test]
    fn refresh_keeps_the_selection_and_a_new_query_clears_it() {
        let mut f = TextFinder::new(Options::default());
        f.set_query("one", "one");
        f.find_next(0);
        f.refresh("one one one");
        assert_eq!(f.position(), Some((1, 3)));
        f.set_query("one one one", "two");
        assert_eq!(f.position(), None);
        assert_eq!(f.count(), 0);
    }
}
