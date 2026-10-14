//! Collapsible sections under headings: finds the Markdown headings of a note, the lines each heading's section
//! covers, and which lines are hidden when some headings are folded. Fold state is a set of heading line numbers
//! (0-based) kept per note. Pure functions over the note text.
#![allow(dead_code)] // the disclosure triangles and View > Collapse All / Expand All wire these in next

use std::collections::BTreeSet;

/// A heading: its line, level (1 to 6) and title.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Heading {
    pub line: usize,
    pub level: usize,
    pub title: String,
}

/// The headings of `text`, outside fenced code blocks.
pub fn headings(text: &str) -> Vec<Heading> {
    let mut out = vec![];
    let mut fenced = false;
    for (i, line) in text.lines().enumerate() {
        let t = line.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        let level = t.chars().take_while(|c| *c == '#').count();
        if (1..=6).contains(&level) && t[level..].starts_with(' ') && !t[level..].trim().is_empty() {
            out.push(Heading { line: i, level, title: t[level..].trim().trim_end_matches('#').trim().to_string() });
        }
    }
    out
}

/// The lines a heading's section hides when folded: from the line after the heading to the line before the next
/// heading of the same or a higher level (or the end), as `start..end`. Empty when nothing follows.
pub fn section(text: &str, heading_line: usize) -> std::ops::Range<usize> {
    let total = text.lines().count();
    let hs = headings(text);
    let Some(h) = hs.iter().find(|h| h.line == heading_line) else { return 0..0 };
    let end = hs.iter().find(|n| n.line > h.line && n.level <= h.level).map_or(total, |n| n.line);
    (h.line + 1)..end.max(h.line + 1)
}

/// Whether a heading has anything to fold.
pub fn foldable(text: &str, heading_line: usize) -> bool {
    !section(text, heading_line).is_empty()
}

/// Fold state of one note.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Folds {
    folded: BTreeSet<usize>,
}

impl Folds {
    pub fn is_folded(&self, heading_line: usize) -> bool {
        self.folded.contains(&heading_line)
    }

    pub fn toggle(&mut self, text: &str, heading_line: usize) {
        if !foldable(text, heading_line) {
            return;
        }
        if !self.folded.remove(&heading_line) {
            self.folded.insert(heading_line);
        }
    }

    pub fn collapse_all(&mut self, text: &str) {
        self.folded = headings(text).into_iter().filter(|h| foldable(text, h.line)).map(|h| h.line).collect();
    }

    pub fn expand_all(&mut self) {
        self.folded.clear();
    }

    /// The lines hidden by the folded headings (a folded heading inside a folded section is hidden with it).
    pub fn hidden_lines(&self, text: &str) -> BTreeSet<usize> {
        let mut hidden = BTreeSet::new();
        for &h in &self.folded {
            hidden.extend(section(text, h));
        }
        hidden
    }

    /// Drops fold marks whose heading no longer exists (after an edit).
    pub fn prune(&mut self, text: &str) {
        let live: BTreeSet<usize> = headings(text).into_iter().map(|h| h.line).collect();
        self.folded.retain(|l| live.contains(l));
    }

    /// Stored form: line numbers separated by commas.
    pub fn to_text(&self) -> String {
        self.folded.iter().map(usize::to_string).collect::<Vec<_>>().join(",")
    }

    pub fn from_text(text: &str) -> Folds {
        Folds { folded: text.split(',').filter_map(|s| s.trim().parse().ok()).collect() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOTE: &str = "# Title\nintro\n## A\na1\na2\n### A.1\ndeep\n## B\nb1\n```\n# not a heading\n```\n#nospace\n";

    #[test]
    fn finds_headings_outside_code() {
        let h = headings(NOTE);
        let lines: Vec<(usize, usize)> = h.iter().map(|x| (x.line, x.level)).collect();
        assert_eq!(lines, vec![(0, 1), (2, 2), (5, 3), (7, 2)]);
        assert_eq!(h[1].title, "A");
    }

    #[test]
    fn sections_end_at_the_next_equal_or_higher_heading() {
        assert_eq!(section(NOTE, 2), 3..7);
        assert_eq!(section(NOTE, 5), 6..7);
        assert_eq!(section(NOTE, 0), 1..13);
        assert_eq!(section(NOTE, 7), 8..13);
        assert_eq!(section(NOTE, 3), 0..0);
        assert!(!foldable("# Only\n", 0));
    }

    #[test]
    fn folding_hides_lines_and_prunes() {
        let mut f = Folds::default();
        f.toggle(NOTE, 2);
        assert!(f.is_folded(2));
        assert_eq!(f.hidden_lines(NOTE).into_iter().collect::<Vec<_>>(), vec![3, 4, 5, 6]);
        f.toggle(NOTE, 2);
        assert!(!f.is_folded(2));
        f.collapse_all(NOTE);
        assert!(f.is_folded(0) && f.is_folded(7));
        f.prune("# Title\nintro\n");
        assert_eq!(f.to_text(), "0");
        assert_eq!(Folds::from_text("0, 7,x"), { let mut g = Folds::default(); g.folded.insert(0); g.folded.insert(7); g });
        f.expand_all();
        assert!(f.hidden_lines(NOTE).is_empty());
    }
}
