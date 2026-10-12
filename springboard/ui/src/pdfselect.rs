//! Text selection and find over the text of a PDF document, as a pure model.
//!
//! A document is a list of pages, each holding its text in reading order with lines separated by `\n`. A selection is
//! a list of ranges over those pages, kept in page order and merged where they touch, so a noncontiguous selection is
//! several ranges. Positions are character indices (Unicode scalar values) into a page's text, the way a page reports
//! its number of characters. Find reuses the find rules of the text find model, so case and whole-word matching agree
//! with the text fields. Nothing here draws: the view draws the bounds of the characters a selection returns.

use crate::textfind::{self, Options};
use std::ops::Range;

/// The text of one page: its characters in reading order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PageText {
    chars: Vec<char>,
}

impl PageText {
    pub fn new(text: &str) -> PageText {
        PageText { chars: text.chars().collect() }
    }

    /// The number of characters on the page.
    pub fn len(&self) -> usize {
        self.chars.len()
    }

    pub fn is_empty(&self) -> bool {
        self.chars.is_empty()
    }

    /// The text of the page as one string.
    pub fn string(&self) -> String {
        self.chars.iter().collect()
    }

    /// The characters in `range`, or `None` when the range runs past the end of the page.
    fn slice(&self, range: Range<usize>) -> Option<String> {
        self.chars.get(range).map(|c| c.iter().collect())
    }
}

/// The text of a whole document, page by page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocumentText {
    pages: Vec<PageText>,
}

impl DocumentText {
    /// A document whose pages hold `pages` in order.
    pub fn new(pages: &[&str]) -> DocumentText {
        DocumentText { pages: pages.iter().map(|p| PageText::new(p)).collect() }
    }

    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    pub fn page(&self, index: usize) -> Option<&PageText> {
        self.pages.get(index)
    }

    /// The number of characters on page `index`, or `None` for a page that does not exist.
    pub fn number_of_characters(&self, index: usize) -> Option<usize> {
        self.page(index).map(PageText::len)
    }
}

/// A selection: non-empty ranges of characters, as `(page, range)`, in page order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    ranges: Vec<(usize, Range<usize>)>,
}

impl Selection {
    /// An empty selection.
    pub fn new() -> Selection {
        Selection { ranges: Vec::new() }
    }

    /// The selection from `from` to `to`, both given as `(page, character index)`, `to` excluded. It spans every page
    /// between the two positions. Returns `None` when `from` comes after `to`, or when a position is past the end of
    /// its page or the document has no such page.
    pub fn between(doc: &DocumentText, from: (usize, usize), to: (usize, usize)) -> Option<Selection> {
        if from > to {
            return None;
        }
        let (first, last) = (from.0, to.0);
        if from.1 > doc.number_of_characters(first)? || to.1 > doc.number_of_characters(last)? {
            return None;
        }
        let mut ranges = Vec::new();
        for page in first..=last {
            let len = doc.number_of_characters(page)?;
            let start = if page == first { from.1 } else { 0 };
            let end = if page == last { to.1 } else { len };
            if start < end {
                ranges.push((page, start..end));
            }
        }
        Some(Selection { ranges })
    }

    /// Adds `other` to this selection. Ranges that overlap or touch on the same page are merged into one.
    pub fn add(&mut self, other: &Selection) {
        let mut all: Vec<(usize, Range<usize>)> =
            self.ranges.iter().chain(other.ranges.iter()).cloned().collect();
        all.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.start.cmp(&b.1.start)));
        let mut merged: Vec<(usize, Range<usize>)> = Vec::new();
        for (page, range) in all {
            if let Some(last) = merged.last_mut() {
                if last.0 == page && range.start <= last.1.end {
                    if range.end > last.1.end {
                        last.1.end = range.end;
                    }
                    continue;
                }
            }
            merged.push((page, range));
        }
        self.ranges = merged;
    }

    /// The ranges of the selection, as `(page, character range)`.
    pub fn ranges(&self) -> &[(usize, Range<usize>)] {
        &self.ranges
    }

    pub fn is_empty(&self) -> bool {
        self.ranges.is_empty()
    }

    /// The pages the selection touches, in order.
    pub fn pages(&self) -> Vec<usize> {
        let mut pages: Vec<usize> = self.ranges.iter().map(|r| r.0).collect();
        pages.dedup();
        pages
    }

    /// The text of the selection. Each range is one piece; pieces are joined with `\n`. A piece that no longer fits its
    /// page is left out.
    pub fn string(&self, doc: &DocumentText) -> String {
        self.ranges
            .iter()
            .filter_map(|(page, range)| doc.page(*page)?.slice(range.clone()))
            .collect::<Vec<String>>()
            .join("\n")
    }

    /// The selection split into one selection per line. Line breaks are not part of any line.
    pub fn selections_by_line(&self, doc: &DocumentText) -> Vec<Selection> {
        let mut lines = Vec::new();
        for (page, range) in &self.ranges {
            let page_text = match doc.page(*page) {
                Some(p) => p,
                None => continue,
            };
            let end = range.end.min(page_text.len());
            let mut start = range.start;
            for i in range.start..end {
                if page_text.chars[i] == '\n' {
                    if start < i {
                        lines.push(Selection { ranges: vec![(*page, start..i)] });
                    }
                    start = i + 1;
                }
            }
            if start < end {
                lines.push(Selection { ranges: vec![(*page, start..end)] });
            }
        }
        lines
    }
}

/// Every match of `query` in the document, one selection per match, in document order. Matches do not span pages.
/// An empty query matches nothing. Case and whole-word handling come from `opts`, as in the text find model.
pub fn find(doc: &DocumentText, query: &str, opts: Options) -> Vec<Selection> {
    let mut found = Vec::new();
    for (page, text) in doc.pages.iter().enumerate() {
        let s = text.string();
        for bytes in textfind::find_all(&s, query, opts) {
            let start = s[..bytes.start].chars().count();
            let end = start + s[bytes.clone()].chars().count();
            found.push(Selection { ranges: vec![(page, start..end)] });
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc() -> DocumentText {
        DocumentText::new(&["Hello world", "world peace\nworld"])
    }

    #[test]
    fn selection_inside_one_page_has_its_text() {
        let s = Selection::between(&doc(), (0, 6), (0, 11)).unwrap();
        assert_eq!(s.string(&doc()), "world");
        assert_eq!(s.pages(), vec![0]);
    }

    #[test]
    fn selection_across_pages_joins_the_pieces_with_a_line_break() {
        let s = Selection::between(&doc(), (0, 0), (1, 5)).unwrap();
        assert_eq!(s.ranges(), &[(0, 0..11), (1, 0..5)]);
        assert_eq!(s.string(&doc()), "Hello world\nworld");
        assert_eq!(s.pages(), vec![0, 1]);
    }

    #[test]
    fn positions_outside_the_document_are_refused() {
        assert!(Selection::between(&doc(), (0, 0), (0, 99)).is_none());
        assert!(Selection::between(&doc(), (0, 4), (0, 2)).is_none());
        assert!(Selection::between(&doc(), (7, 0), (7, 0)).is_none());
        assert!(Selection::between(&doc(), (0, 2), (0, 2)).unwrap().is_empty());
    }

    #[test]
    fn find_is_case_insensitive_and_in_document_order() {
        let found = find(&doc(), "world", Options::default());
        let ranges: Vec<_> = found.iter().map(|s| s.ranges()[0].clone()).collect();
        assert_eq!(ranges, vec![(0, 6..11), (1, 0..5), (1, 12..17)]);
        assert_eq!(found[2].string(&doc()), "world");
    }

    #[test]
    fn whole_word_find_skips_matches_inside_longer_words() {
        let d = DocumentText::new(&["worldly world"]);
        let opts = Options { ignore_case: true, whole_word: true };
        let found = find(&d, "world", opts);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].ranges(), &[(0, 8..13)]);
    }

    #[test]
    fn find_positions_count_characters_not_bytes() {
        let d = DocumentText::new(&["Árni og árni"]);
        let found = find(&d, "árni", Options::default());
        let ranges: Vec<_> = found.iter().map(|s| s.ranges()[0].clone()).collect();
        assert_eq!(ranges, vec![(0, 0..4), (0, 8..12)]);
        assert_eq!(found[0].string(&d), "Árni");
        assert_eq!(found[1].string(&d), "árni");
    }

    #[test]
    fn selections_split_at_line_breaks() {
        let d = DocumentText::new(&["ab\ncd"]);
        let s = Selection::between(&d, (0, 0), (0, 5)).unwrap();
        let lines = s.selections_by_line(&d);
        let texts: Vec<String> = lines.iter().map(|l| l.string(&d)).collect();
        assert_eq!(texts, vec!["ab".to_string(), "cd".to_string()]);
    }

    #[test]
    fn adding_touching_ranges_merges_them_and_keeps_gaps_apart() {
        let d = DocumentText::new(&["abcde"]);
        let mut a = Selection::between(&d, (0, 0), (0, 3)).unwrap();
        a.add(&Selection::between(&d, (0, 3), (0, 5)).unwrap());
        assert_eq!(a.ranges(), &[(0, 0..5)]);

        let mut b = Selection::between(&d, (0, 0), (0, 2)).unwrap();
        b.add(&Selection::between(&d, (0, 4), (0, 5)).unwrap());
        assert_eq!(b.ranges().len(), 2);
        assert_eq!(b.string(&d), "ab\ne");
    }
}
