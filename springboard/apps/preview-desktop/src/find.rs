//! Find in a PDF and Copy: the text of a page is read on the PDF worker, searched line by line, and the hits go back to
//! the window for the Search list of the sidebar. The matching is plain, so it runs in tests without a PDF.

use crate::pdfjob::{self, Job, Pdf, Reply};
use springboard::cg::pdf::extract::{Extractor, Piece};

/// The most hits a search keeps, so a common word in a long book stays a short list.
pub const MAX_HITS: usize = 500;
/// The longest snippet shown for a hit, in characters.
const SNIPPET_CHARS: usize = 80;

/// One line of a page that holds the searched text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    /// The source page.
    pub page: usize,
    /// The line, shortened.
    pub snippet: String,
}

/// The lines of source page `src`, in reading order. Empty when the page has no text.
pub fn page_lines(pdf: &Pdf, src: usize) -> Vec<String> {
    let Some(page) = pdf.pages.get(src) else { return Vec::new() };
    let mut extractor = Extractor::new(&pdf.doc);
    extractor.page(page);
    extractor
        .pieces
        .iter()
        .filter_map(|piece| match piece {
            Piece::Line { text, .. } => Some(text.trim().to_string()),
            _ => None,
        })
        .filter(|line| !line.is_empty())
        .collect()
}

/// The hits for `query` in `pages`, each page given as its source index and lines. The match ignores case. A blank
/// query has no hits.
pub fn search(pages: &[(usize, Vec<String>)], query: &str) -> Vec<Hit> {
    let needle = query.trim().to_lowercase();
    let mut hits = Vec::new();
    if needle.is_empty() {
        return hits;
    }
    for (page, lines) in pages {
        for line in lines.iter().filter(|l| l.to_lowercase().contains(&needle)) {
            if hits.len() >= MAX_HITS {
                return hits;
            }
            hits.push(Hit { page: *page, snippet: shorten(line) });
        }
    }
    hits
}

/// `line` cut to the snippet length with an ellipsis.
fn shorten(line: &str) -> String {
    if line.chars().count() <= SNIPPET_CHARS {
        return line.to_string();
    }
    let cut: String = line.chars().take(SNIPPET_CHARS).collect();
    format!("{}\u{2026}", cut.trim_end())
}

/// The index in `hits` to go to from `current` (None before the first search move), forward or back. It wraps.
pub fn step(count: usize, current: Option<usize>, forward: bool) -> Option<usize> {
    if count == 0 {
        return None;
    }
    Some(match (current, forward) {
        (None, true) => 0,
        (None, false) => count - 1,
        (Some(i), true) => (i + 1) % count,
        (Some(i), false) => (i + count - 1) % count,
    })
}

/// The first hit on or after shown position `from`, given each hit's shown position (None for a deleted page). It
/// wraps to the first hit when none is later.
pub fn first_from(places: &[Option<usize>], from: usize) -> Option<usize> {
    places
        .iter()
        .enumerate()
        .filter_map(|(i, p)| p.map(|p| (i, p)))
        .filter(|&(_, p)| p >= from)
        .min_by_key(|&(i, p)| (p, i))
        .map(|(i, _)| i)
        .or_else(|| places.iter().position(Option::is_some))
}

/// A job that searches every page for `query` and replies with the hits.
pub fn search_job(query: String) -> Job {
    pdfjob::run(move |pdf: &Pdf| {
        let pages: Vec<(usize, Vec<String>)> = (0..pdf.pages.len()).map(|src| (src, page_lines(pdf, src))).collect();
        Reply::Found(query.clone(), search(&pages, &query))
    })
}

/// A job that reads the text of source pages `pages` for Copy, one line each, pages separated by a blank line.
pub fn text_job(pages: Vec<usize>) -> Job {
    pdfjob::run(move |pdf: &Pdf| {
        let texts: Vec<String> = pages.iter().map(|&src| page_lines(pdf, src).join("\n")).collect();
        Reply::Text(texts.join("\n\n"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pages() -> Vec<(usize, Vec<String>)> {
        vec![
            (0, vec!["The quick brown fox".into(), "jumps over".into()]),
            (3, vec!["the lazy dog".into(), "THE END".into()]),
        ]
    }

    #[test]
    fn search_matches_lines_ignoring_case() {
        let hits = search(&pages(), "the");
        let found: Vec<(usize, &str)> = hits.iter().map(|h| (h.page, h.snippet.as_str())).collect();
        assert_eq!(found, vec![(0, "The quick brown fox"), (3, "the lazy dog"), (3, "THE END")]);
        assert!(search(&pages(), "  ").is_empty());
        assert!(search(&pages(), "cat").is_empty());
    }

    #[test]
    fn long_lines_are_shortened_and_hits_are_capped() {
        let long = "word ".repeat(40);
        let hits = search(&[(0, vec![long])], "word");
        assert!(hits[0].snippet.ends_with('\u{2026}'));
        assert!(hits[0].snippet.chars().count() <= SNIPPET_CHARS + 1);
        let many: Vec<String> = (0..MAX_HITS + 50).map(|_| "needle".to_string()).collect();
        assert_eq!(search(&[(0, many)], "needle").len(), MAX_HITS);
    }

    #[test]
    fn stepping_wraps_in_both_directions() {
        assert_eq!(step(0, None, true), None);
        assert_eq!(step(3, None, true), Some(0));
        assert_eq!(step(3, None, false), Some(2));
        assert_eq!(step(3, Some(2), true), Some(0));
        assert_eq!(step(3, Some(0), false), Some(2));
        assert_eq!(step(3, Some(1), true), Some(2));
    }

    #[test]
    fn the_first_hit_from_a_page_skips_deleted_pages_and_wraps() {
        let places = [Some(1), None, Some(4), Some(4)];
        assert_eq!(first_from(&places, 0), Some(0));
        assert_eq!(first_from(&places, 2), Some(2));
        assert_eq!(first_from(&places, 5), Some(0));
        assert_eq!(first_from(&[None, None], 0), None);
    }
}
