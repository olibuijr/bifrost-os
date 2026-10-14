//! Find in PDF. The PDF worker reads each page's text lines (the same extraction the reader uses) and lists every line
//! that holds the search text, ignoring case. The window shows the hits in the sidebar and jumps between them. The
//! extraction gives a line's text and baseline but not where along the line a word sits, so a hit is a whole line and
//! the window marks the page, not the word.

use crate::pdfjob::{self, Job, Pdf, Reply};
use springboard::cg::pdf::extract::{Extractor, Piece};

/// The characters of the line a hit shows in the sidebar.
const SNIPPET_CHARS: usize = 60;
/// The most hits one search lists; a search that runs into the limit stops there.
pub const MAX_HITS: usize = 500;

/// One line of a page that holds the search text.
#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    /// The source page.
    pub page: usize,
    /// The part of the line around the match.
    pub snippet: String,
}

/// The line shortened to about `width` characters around the first match of `query`, with an ellipsis where it was cut.
/// Runs of white space become one space.
pub fn snippet(line: &str, query: &str, width: usize) -> String {
    let text: Vec<char> = line.split_whitespace().collect::<Vec<_>>().join(" ").chars().collect();
    if text.len() <= width {
        return text.into_iter().collect();
    }
    let lower: String = text.iter().collect::<String>().to_lowercase();
    // lower casing can change the number of characters (a few letters become two); then the position is not trusted
    let at = if lower.chars().count() == text.len() {
        lower.find(&query.to_lowercase()).map(|byte| lower[..byte].chars().count()).unwrap_or(0)
    } else {
        0
    };
    let start = at.saturating_sub(width.saturating_sub(query.chars().count()) / 3).min(text.len() - width);
    let body: String = text[start..start + width].iter().collect();
    let head = if start > 0 { "\u{2026}" } else { "" };
    let tail = if start + width < text.len() { "\u{2026}" } else { "" };
    format!("{head}{body}{tail}")
}

/// The hits on source page `page`, whose text lines are `lines`: one for each line that holds `query`.
pub fn page_hits(page: usize, lines: &[String], query: &str) -> Vec<Hit> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }
    lines
        .iter()
        .filter(|line| line.to_lowercase().contains(&needle))
        .map(|line| Hit { page, snippet: snippet(line, query.trim(), SNIPPET_CHARS) })
        .collect()
}

/// The hits for `query` through the whole file, in page order.
pub fn search(pdf: &Pdf, query: &str) -> Vec<Hit> {
    let mut hits = Vec::new();
    for (page, source) in pdf.pages.iter().enumerate() {
        let mut extractor = Extractor::new(&pdf.doc);
        extractor.page(source);
        let lines: Vec<String> = extractor
            .pieces
            .into_iter()
            .filter_map(|piece| match piece {
                Piece::Line { text, .. } => Some(text),
                _ => None,
            })
            .collect();
        hits.extend(page_hits(page, &lines, query));
        if hits.len() >= MAX_HITS {
            hits.truncate(MAX_HITS);
            break;
        }
    }
    hits
}

/// A job that searches the file for `query` and replies with the hits.
pub fn find_job(query: String) -> Job {
    pdfjob::run(move |pdf: &Pdf| {
        let hits = search(pdf, &query);
        Reply::Found(query, hits)
    })
}

/// The shown page of the next (or previous) hit after `current`, from the shown pages of the hits (in any order). It
/// wraps round to the other end. None with no hits.
pub fn next_page(places: &[usize], current: usize, forward: bool) -> Option<usize> {
    if forward {
        places.iter().copied().filter(|&p| p > current).min().or_else(|| places.iter().copied().min())
    } else {
        places.iter().copied().filter(|&p| p < current).max().or_else(|| places.iter().copied().max())
    }
}

/// The shown page to start at after a search: the first hit on or after `current`, or the first hit when none is.
pub fn first_from(places: &[usize], current: usize) -> Option<usize> {
    places.iter().copied().filter(|&p| p >= current).min().or_else(|| places.iter().copied().min())
}

/// The status line after a search.
pub fn summary(query: &str, hits: usize) -> String {
    match hits {
        0 => format!("Nothing found for \u{201c}{query}\u{201d}."),
        1 => format!("1 match for \u{201c}{query}\u{201d}"),
        n if n >= MAX_HITS => format!("{MAX_HITS} or more matches for \u{201c}{query}\u{201d}"),
        n => format!("{n} matches for \u{201c}{query}\u{201d}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use springboard::cg::pdf::doc::tests::tiny_pdf;
    use springboard::cg::pdf::doc::Doc;
    use springboard::cg::pdf::page;

    fn open(content: &str) -> Pdf {
        let doc = Doc::open(tiny_pdf(content)).expect("a document");
        let pages = page::pages(&doc);
        Pdf { doc, pages }
    }

    #[test]
    fn a_search_lists_the_lines_that_hold_the_text_ignoring_case() {
        let pdf = open("BT /F1 12 Tf 72 700 Td (Hello World) Tj 0 -14 Td (Another line) Tj ET");
        let hits = search(&pdf, "world");
        assert_eq!(hits, vec![Hit { page: 0, snippet: "Hello World".into() }]);
        assert_eq!(search(&pdf, "LINE").len(), 1);
        assert_eq!(search(&pdf, "o").len(), 2);
        assert!(search(&pdf, "zebra").is_empty());
        assert!(search(&pdf, "   ").is_empty());
    }

    #[test]
    fn a_snippet_is_cut_around_the_match() {
        assert_eq!(snippet("short  line", "line", 60), "short line");
        let long = format!("{} needle {}", "a".repeat(80), "b".repeat(80));
        let cut = snippet(&long, "NEEDLE", 40);
        assert!(cut.contains("needle"), "{cut}");
        assert!(cut.starts_with('\u{2026}') && cut.ends_with('\u{2026}'));
        assert_eq!(cut.chars().count(), 42);
        let early = snippet(&format!("needle {}", "b".repeat(80)), "needle", 40);
        assert!(early.starts_with("needle") && early.ends_with('\u{2026}'));
        let late = snippet(&format!("{} needle", "a".repeat(80)), "needle", 40);
        assert!(late.ends_with("needle") && late.starts_with('\u{2026}'));
    }

    #[test]
    fn next_and_previous_hits_wrap_round() {
        let places = [2, 5, 9];
        assert_eq!(next_page(&places, 5, true), Some(9));
        assert_eq!(next_page(&places, 9, true), Some(2));
        assert_eq!(next_page(&places, 5, false), Some(2));
        assert_eq!(next_page(&places, 2, false), Some(9));
        assert_eq!(next_page(&[], 0, true), None);
    }

    #[test]
    fn a_search_starts_at_the_first_hit_on_or_after_the_shown_page() {
        let places = [2, 5, 9];
        assert_eq!(first_from(&places, 5), Some(5));
        assert_eq!(first_from(&places, 6), Some(9));
        assert_eq!(first_from(&places, 10), Some(2));
        assert_eq!(first_from(&[], 0), None);
    }

    #[test]
    fn the_status_line_counts_the_matches() {
        assert_eq!(summary("x", 0), "Nothing found for \u{201c}x\u{201d}.");
        assert_eq!(summary("x", 1), "1 match for \u{201c}x\u{201d}");
        assert_eq!(summary("x", 4), "4 matches for \u{201c}x\u{201d}");
        assert!(summary("x", MAX_HITS).starts_with("500 or more"));
    }
}
