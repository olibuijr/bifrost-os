//! The list of boards Freeform keeps. Board 1 lives in the app's data folder itself (so a board saved before there
//! were several is still board 1); every later board has a folder `boards/<id>` below it. The list is saved as
//! `boards.txt`, one `id<TAB>title` per line, the current board first on a line of its own (`current <id>`).
use std::path::{Path, PathBuf};

/// One board in the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub id: u32,
    pub title: String,
}

/// The boards, in the order they were made, and which one is open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoardList {
    entries: Vec<Entry>,
    current: u32,
}

impl Default for BoardList {
    /// One board, "Board 1".
    fn default() -> Self {
        BoardList { entries: vec![Entry { id: 1, title: "Board 1".into() }], current: 1 }
    }
}

impl BoardList {
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn current(&self) -> u32 {
        self.current
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// The open board's title.
    pub fn current_title(&self) -> &str {
        self.entries.iter().find(|e| e.id == self.current).map(|e| e.title.as_str()).unwrap_or("Board")
    }

    /// Adds a board ("Board N" with the next number) and returns its id. It is not opened.
    pub fn add(&mut self) -> u32 {
        let id = self.entries.iter().map(|e| e.id).max().unwrap_or(0) + 1;
        self.entries.push(Entry { id, title: format!("Board {id}") });
        id
    }

    /// Adds a copy of board `id` named "<title> copy" and returns its id, or `None` when there is no such board.
    pub fn duplicate(&mut self, id: u32) -> Option<u32> {
        let title = format!("{} copy", self.entries.iter().find(|e| e.id == id)?.title);
        let new = self.entries.iter().map(|e| e.id).max().unwrap_or(0) + 1;
        self.entries.push(Entry { id: new, title });
        Some(new)
    }

    /// Opens board `id`; false (and no change) when there is no such board.
    pub fn open(&mut self, id: u32) -> bool {
        let known = self.entries.iter().any(|e| e.id == id);
        if known {
            self.current = id;
        }
        known
    }

    /// Removes board `id`. The last board cannot be removed. When the open board goes, the one before it (or the first)
    /// opens. Returns whether a board was removed.
    pub fn remove(&mut self, id: u32) -> bool {
        if self.entries.len() < 2 {
            return false;
        }
        let Some(at) = self.entries.iter().position(|e| e.id == id) else { return false };
        self.entries.remove(at);
        if self.current == id {
            self.current = self.entries[at.saturating_sub(1)].id;
        }
        true
    }

    /// Renames board `id`; a blank title is refused.
    pub fn rename(&mut self, id: u32, title: &str) -> bool {
        let title = title.trim();
        match self.entries.iter_mut().find(|e| e.id == id) {
            Some(e) if !title.is_empty() => {
                e.title = title.replace(['\t', '\n'], " ");
                true
            }
            _ => false,
        }
    }

    /// The id of the board after (or before) the open one, wrapping round.
    pub fn neighbour(&self, forward: bool) -> u32 {
        let n = self.entries.len();
        let at = self.entries.iter().position(|e| e.id == self.current).unwrap_or(0);
        let to = if forward { (at + 1) % n } else { (at + n - 1) % n };
        self.entries[to].id
    }

    /// The boards whose title contains `query` (any case); every board for a blank query.
    pub fn matching(&self, query: &str) -> Vec<&Entry> {
        let q = query.trim().to_lowercase();
        self.entries.iter().filter(|e| q.is_empty() || e.title.to_lowercase().contains(&q)).collect()
    }

    pub fn to_text(&self) -> String {
        let mut out = format!("current {}\n", self.current);
        for e in &self.entries {
            out.push_str(&format!("{}\t{}\n", e.id, e.title));
        }
        out
    }

    /// Reads `to_text` back. Lines that do not parse are skipped; with no board left, or an unknown current board, the
    /// first board is open (and a list with no board is the default list).
    pub fn from_text(text: &str) -> BoardList {
        let mut entries: Vec<Entry> = Vec::new();
        let mut current = 0;
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix("current ") {
                current = rest.trim().parse().unwrap_or(0);
            } else if let Some((id, title)) = line.split_once('\t') {
                if let Ok(id) = id.trim().parse::<u32>() {
                    if id > 0 && !entries.iter().any(|e| e.id == id) {
                        entries.push(Entry { id, title: title.to_string() });
                    }
                }
            }
        }
        if entries.is_empty() {
            return BoardList::default();
        }
        if !entries.iter().any(|e| e.id == current) {
            current = entries[0].id;
        }
        BoardList { entries, current }
    }
}

/// The folder board `id` is kept in below `root`: `root` itself for board 1, `root/boards/<id>` for the rest.
pub fn dir_for(root: &Path, id: u32) -> PathBuf {
    if id == 1 {
        root.to_path_buf()
    } else {
        root.join("boards").join(id.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_list_has_one_open_board_and_adding_numbers_the_next() {
        let mut l = BoardList::default();
        assert_eq!((l.len(), l.current(), l.current_title()), (1, 1, "Board 1"));
        assert_eq!(l.add(), 2);
        assert_eq!(l.add(), 3);
        assert_eq!(l.entries()[2].title, "Board 3");
        assert_eq!(l.current(), 1, "adding does not open");
    }

    #[test]
    fn opening_needs_a_known_board() {
        let mut l = BoardList::default();
        let b = l.add();
        assert!(l.open(b));
        assert_eq!(l.current(), b);
        assert!(!l.open(99));
        assert_eq!(l.current(), b);
    }

    #[test]
    fn duplicating_names_the_copy_and_never_reuses_an_id() {
        let mut l = BoardList::default();
        let b = l.add();
        l.remove(b);
        let c = l.duplicate(1).unwrap();
        assert_eq!(c, 2, "the id of a removed last board is free again");
        assert_eq!(l.entries()[1].title, "Board 1 copy");
        assert!(l.duplicate(42).is_none());
    }

    #[test]
    fn removing_the_open_board_opens_its_neighbour_and_the_last_board_stays() {
        let mut l = BoardList::default();
        let (b, c) = (l.add(), l.add());
        l.open(c);
        assert!(l.remove(c));
        assert_eq!(l.current(), b);
        assert!(l.remove(1));
        assert_eq!(l.current(), b);
        assert!(!l.remove(b), "the last board is kept");
        assert!(!l.remove(77));
    }

    #[test]
    fn renaming_trims_and_refuses_a_blank_title() {
        let mut l = BoardList::default();
        assert!(l.rename(1, "  Plans\tfor May "));
        assert_eq!(l.current_title(), "Plans for May");
        assert!(!l.rename(1, "   "));
        assert!(!l.rename(9, "x"));
        assert_eq!(l.current_title(), "Plans for May");
    }

    #[test]
    fn neighbours_wrap_round_the_list() {
        let mut l = BoardList::default();
        let (b, c) = (l.add(), l.add());
        assert_eq!(l.neighbour(true), b);
        assert_eq!(l.neighbour(false), c);
        l.open(c);
        assert_eq!(l.neighbour(true), 1);
    }

    #[test]
    fn matching_filters_by_title_in_any_case() {
        let mut l = BoardList::default();
        let b = l.add();
        l.rename(b, "Trip PLAN");
        assert_eq!(l.matching("plan").len(), 1);
        assert_eq!(l.matching("plan")[0].id, b);
        assert_eq!(l.matching("  ").len(), 2, "a blank query lists every board");
        assert!(l.matching("zzz").is_empty());
    }

    #[test]
    fn the_list_round_trips_through_text_and_survives_bad_lines() {
        let mut l = BoardList::default();
        let b = l.add();
        l.rename(b, "Trip");
        l.open(b);
        assert_eq!(BoardList::from_text(&l.to_text()), l);
        let odd = BoardList::from_text("junk\ncurrent 9\n3\tThree\nx\ty\n3\tDup\n");
        assert_eq!(odd.len(), 1);
        assert_eq!(odd.current(), 3, "an unknown current board falls back to the first");
        assert_eq!(BoardList::from_text(""), BoardList::default());
    }

    #[test]
    fn board_one_keeps_the_data_folder_and_the_rest_get_their_own() {
        let root = Path::new("/d");
        assert_eq!(dir_for(root, 1), PathBuf::from("/d"));
        assert_eq!(dir_for(root, 4), PathBuf::from("/d/boards/4"));
    }
}
