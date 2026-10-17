//! Collapsible sections: a Title, Heading or Subheading line can fold the lines under it, up to the next heading of
//! the same or a higher level. The note text is never changed; the collapsed headings of each note are kept in a
//! sidecar file (`.springboard-notes-folds`) keyed by the note id and the heading line. Pure text logic, so it is
//! tested on the host.

use springboard::shell::md::{classify, Block};
use std::collections::{HashMap, HashSet};
use std::io;
use std::ops::Range;
use std::path::Path;

const FOLDS_FILE: &str = ".springboard-notes-folds";

/// The collapsed headings of every note: note id to the heading lines (exact text) folded shut.
pub type Folds = HashMap<String, HashSet<String>>;

/// A heading line of a note.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Heading {
    /// zero-based line index in the note text
    pub line: usize,
    /// 1 for Title (`# `), 2 for Heading (`## `), 3 for Subheading (`### `)
    pub level: u8,
    /// the heading's text as written, trailing spaces removed (the key it is saved under)
    pub key: String,
}

/// The heading level of a paragraph style, or `None` for a line that is not a heading.
pub fn level_of(block: Block) -> Option<u8> {
    match block {
        Block::Title => Some(1),
        Block::Heading => Some(2),
        Block::Subheading => Some(3),
        _ => None,
    }
}

/// Every heading line of `text`, in order.
pub fn headings(text: &str) -> Vec<Heading> {
    text.lines()
        .enumerate()
        .filter_map(|(line, l)| {
            level_of(classify(l).block).map(|level| Heading { line, level, key: l.trim_end().to_string() })
        })
        .collect()
}

/// The lines a heading folds: the lines after `heads[index]` up to (not including) the next heading of its level or
/// a higher one, or to the end of the note. `lines` is the number of lines in the note.
pub fn section(heads: &[Heading], index: usize, lines: usize) -> Range<usize> {
    let head = &heads[index];
    let end = heads[index + 1..].iter().find(|h| h.level <= head.level).map_or(lines, |h| h.line);
    head.line + 1..end.max(head.line + 1)
}

/// The line ranges hidden by the collapsed headings of `text`. Ranges of headings nested in a collapsed one may
/// overlap it; the editor hides a line once however many ranges name it.
pub fn hidden(text: &str, collapsed: &HashSet<String>) -> Vec<Range<usize>> {
    let heads = headings(text);
    let lines = text.lines().count();
    (0..heads.len())
        .filter(|&i| collapsed.contains(&heads[i].key))
        .map(|i| section(&heads, i, lines))
        .filter(|r| !r.is_empty())
        .collect()
}

/// The heading a caret on `line` belongs to: the last heading at or above it.
pub fn heading_above(text: &str, line: usize) -> Option<Heading> {
    headings(text).into_iter().rfind(|h| h.line <= line)
}

/// Folds the heading above `line` (Collapse Section). Returns true when the state changed.
pub fn collapse(collapsed: &mut HashSet<String>, text: &str, line: usize) -> bool {
    heading_above(text, line).is_some_and(|h| collapsed.insert(h.key))
}

/// Unfolds the heading above `line` (Expand Section). Returns true when the state changed.
pub fn expand(collapsed: &mut HashSet<String>, text: &str, line: usize) -> bool {
    heading_above(text, line).is_some_and(|h| collapsed.remove(&h.key))
}

/// Folds every heading of the note (Collapse All Sections). Returns true when anything was added.
pub fn collapse_all(collapsed: &mut HashSet<String>, text: &str) -> bool {
    let before = collapsed.len();
    collapsed.extend(headings(text).into_iter().map(|h| h.key));
    collapsed.len() != before
}

/// Unfolds every heading (Expand All Sections). Returns true when anything was folded.
pub fn expand_all(collapsed: &mut HashSet<String>) -> bool {
    let changed = !collapsed.is_empty();
    collapsed.clear();
    changed
}

fn hex(text: &str) -> String {
    text.as_bytes().iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Option<String> {
    if text.len() % 2 != 0 || !text.is_ascii() {
        return None;
    }
    let bytes: Option<Vec<u8>> = (0..text.len()).step_by(2).map(|i| u8::from_str_radix(&text[i..i + 2], 16).ok()).collect();
    String::from_utf8(bytes?).ok()
}

/// Reads the folds file text; a damaged line is skipped, the rest still loads.
pub fn parse(text: &str) -> Folds {
    let mut folds: Folds = HashMap::new();
    for line in text.lines() {
        if let Some((id, key)) = line.split_once('\t') {
            if let (Some(id), Some(key)) = (unhex(id), unhex(key)) {
                folds.entry(id).or_default().insert(key);
            }
        }
    }
    folds
}

/// The folds file text: one `note id<TAB>heading` pair per line, both hex-encoded, in sorted order. Notes without a
/// folded heading are left out.
pub fn render(folds: &Folds) -> String {
    let mut pairs: Vec<(String, String)> = folds
        .iter()
        .flat_map(|(id, keys)| keys.iter().map(move |key| (hex(id), hex(key))))
        .collect();
    pairs.sort();
    pairs.iter().map(|(id, key)| format!("{id}\t{key}\n")).collect()
}

/// The folds saved in `dir`, or none.
pub fn load(dir: &Path) -> Folds {
    std::fs::read_to_string(dir.join(FOLDS_FILE)).map(|t| parse(&t)).unwrap_or_default()
}

/// Saves `folds` in `dir`, atomically.
pub fn save(dir: &Path, folds: &Folds) -> io::Result<()> {
    springboard::shell::fsutil::write_atomic(&dir.join(FOLDS_FILE), render(folds))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn set(keys: &[&str]) -> HashSet<String> {
        keys.iter().map(|k| k.to_string()).collect()
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-notes")).join(format!("{}-fold-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    // lines: 0 "# Title", 1 "intro", 2 "## One", 3 "a", 4 "b", 5 "### Sub", 6 "c", 7 "## Two", 8 "d", 9 "# Next", 10 "e"
    const NOTE: &str = "# Title\nintro\n## One\na\nb\n### Sub\nc\n## Two\nd\n# Next\ne";

    #[test]
    fn heading_levels_come_from_the_paragraph_styles() {
        let found: Vec<(usize, u8)> = headings(NOTE).iter().map(|h| (h.line, h.level)).collect();
        assert_eq!(found, vec![(0, 1), (2, 2), (5, 3), (7, 2), (9, 1)]);
        assert!(headings("plain\n- item\n> quote").is_empty());
        assert_eq!(headings("## Trailing   ")[0].key, "## Trailing");
    }

    #[test]
    fn a_section_runs_to_the_next_heading_of_the_same_or_a_higher_level() {
        let heads = headings(NOTE);
        let lines = NOTE.lines().count();
        assert_eq!(section(&heads, 0, lines), 1..9, "a Title folds everything up to the next Title");
        assert_eq!(section(&heads, 1, lines), 3..7, "One stops at Two and keeps its Subheading inside");
        assert_eq!(section(&heads, 2, lines), 6..7, "Sub stops at the Heading after it");
        assert_eq!(section(&heads, 4, lines), 10..lines, "the last Title runs to the end");
    }

    #[test]
    fn hidden_ranges_follow_the_collapsed_headings() {
        assert!(hidden(NOTE, &set(&[])).is_empty());
        assert_eq!(hidden(NOTE, &set(&["## One"])), vec![3..7]);
        assert_eq!(hidden(NOTE, &set(&["## One", "### Sub"])), vec![3..7, 6..7]);
        assert!(hidden(NOTE, &set(&["# Missing"])).is_empty());
    }

    #[test]
    fn collapse_and_expand_act_on_the_heading_above_the_caret() {
        let mut folded = set(&[]);
        assert_eq!(heading_above(NOTE, 4).unwrap().key, "## One");
        assert!(heading_above("just text", 0).is_none());
        assert!(collapse(&mut folded, NOTE, 4));
        assert!(!collapse(&mut folded, NOTE, 4), "folding twice changes nothing");
        assert!(folded.contains("## One"));
        assert!(expand(&mut folded, NOTE, 4));
        assert!(!expand(&mut folded, NOTE, 4));
        assert!(!collapse(&mut folded, "no headings", 0));
    }

    #[test]
    fn collapse_all_and_expand_all_cover_every_heading() {
        let mut folded = set(&["# Other"]);
        assert!(collapse_all(&mut folded, NOTE));
        assert_eq!(folded.len(), 6, "the stale key stays until expanded");
        assert!(!collapse_all(&mut folded, NOTE));
        assert!(expand_all(&mut folded));
        assert!(folded.is_empty());
        assert!(!expand_all(&mut folded));
    }

    #[test]
    fn folds_round_trip_through_the_sidecar_text() {
        let mut folds = Folds::new();
        folds.insert("note one".into(), set(&["## One", "# Title with\ttab"]));
        folds.insert("ünïcode".into(), set(&["### Sub"]));
        let text = render(&folds);
        assert_eq!(parse(&text), folds);
        assert_eq!(render(&parse(&text)), text, "the text is stable once saved");
        assert_eq!(parse("zz\tzz\nnot a pair\n6e6f\tzz\n"), Folds::new(), "damaged lines are skipped");
    }

    #[test]
    fn folds_persist_in_the_notes_directory() {
        let dir = scratch("persist");
        assert!(load(&dir).is_empty());
        let mut folds = Folds::new();
        folds.insert("abc".into(), set(&["# Head"]));
        save(&dir, &folds).unwrap();
        assert_eq!(load(&dir), folds);
        folds.get_mut("abc").unwrap().clear();
        save(&dir, &folds).unwrap();
        assert!(load(&dir).get("abc").is_none_or(|k| k.is_empty()), "an expanded note keeps no entry");
    }
}
