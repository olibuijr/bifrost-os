//! Undo and Redo for library changes. One entry is one user action (favourite, hide, rate, delete, paste edits,
//! ...). It records the settings it wrote with their old and new values and the files it moved, so the action
//! can be taken back and done again. The stack is bounded and a new action clears what could be redone.

use std::path::PathBuf;

/// The most actions kept.
pub const DEPTH: usize = 50;

/// One setting an action wrote. An empty value means the key was unset or cleared.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrefChange {
    pub key: String,
    pub before: String,
    pub after: String,
}

/// A file an action moved into `to_dir`; `from` is the path before the move.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Move {
    pub from: PathBuf,
    pub to_dir: PathBuf,
}

/// One undoable action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub label: String,
    pub prefs: Vec<PrefChange>,
    pub moves: Vec<Move>,
}

impl Entry {
    pub fn is_empty(&self) -> bool {
        self.prefs.is_empty() && self.moves.is_empty()
    }

    /// Records a setting write. A key written twice inside one action keeps its first old value and its last new one.
    pub fn note(&mut self, key: &str, before: &str, after: &str) {
        match self.prefs.iter_mut().find(|c| c.key == key) {
            Some(c) => c.after = after.to_string(),
            None => self.prefs.push(PrefChange { key: key.to_string(), before: before.to_string(), after: after.to_string() }),
        }
    }

    /// The entry that takes this one back: settings swapped, the moves left for the caller to reverse (it knows
    /// where each file ended up).
    pub fn reversed_prefs(&self) -> Vec<PrefChange> {
        self.prefs.iter().rev().map(|c| PrefChange { key: c.key.clone(), before: c.after.clone(), after: c.before.clone() }).collect()
    }
}

/// The undo and redo stacks.
#[derive(Clone, Debug, Default)]
pub struct History {
    undo: Vec<Entry>,
    redo: Vec<Entry>,
}

impl History {
    /// Adds a finished action; an empty one is dropped. Anything that could be redone is forgotten.
    pub fn push(&mut self, entry: Entry) {
        if entry.is_empty() {
            return;
        }
        self.redo.clear();
        self.undo.push(entry);
        if self.undo.len() > DEPTH {
            self.undo.remove(0);
        }
    }

    /// Takes the newest action to undo.
    pub fn pop_undo(&mut self) -> Option<Entry> {
        self.undo.pop()
    }

    /// Takes the newest action to redo.
    pub fn pop_redo(&mut self) -> Option<Entry> {
        self.redo.pop()
    }

    /// Keeps an entry that was just taken back, so Redo can do it again.
    pub fn keep_for_redo(&mut self, entry: Entry) {
        if !entry.is_empty() {
            self.redo.push(entry);
        }
    }

    /// Keeps an entry that was just done again, so Undo can take it back; the redo stack stays.
    pub fn keep_for_undo(&mut self, entry: Entry) {
        if !entry.is_empty() {
            self.undo.push(entry);
        }
    }

    /// The menu title of Undo: "Undo Favorite", or "Undo" when there is nothing.
    pub fn undo_title(&self) -> String {
        self.undo.last().map_or_else(|| "Undo".to_string(), |e| format!("Undo {}", e.label))
    }

    pub fn redo_title(&self) -> String {
        self.redo.last().map_or_else(|| "Redo".to_string(), |e| format!("Redo {}", e.label))
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(label: &str, key: &str) -> Entry {
        let mut e = Entry { label: label.into(), prefs: vec![], moves: vec![] };
        e.note(key, "", "1");
        e
    }

    #[test]
    fn a_key_written_twice_keeps_the_first_old_value() {
        let mut e = Entry { label: "Rate".into(), prefs: vec![], moves: vec![] };
        e.note("rate:a", "", "2");
        e.note("rate:a", "2", "4");
        assert_eq!(e.prefs, vec![PrefChange { key: "rate:a".into(), before: "".into(), after: "4".into() }]);
    }

    #[test]
    fn reversing_swaps_old_and_new_values_in_the_opposite_order() {
        let mut e = Entry { label: "x".into(), prefs: vec![], moves: vec![] };
        e.note("a", "0", "1");
        e.note("b", "5", "6");
        let r = e.reversed_prefs();
        assert_eq!((r[0].key.as_str(), r[0].after.as_str()), ("b", "5"));
        assert_eq!((r[1].key.as_str(), r[1].after.as_str()), ("a", "0"));
    }

    #[test]
    fn undo_then_redo_walk_the_stacks_and_a_new_action_clears_redo() {
        let mut h = History::default();
        assert_eq!(h.undo_title(), "Undo");
        h.push(entry("Favorite", "fav:a"));
        h.push(entry("Hide", "hide:a"));
        assert_eq!(h.undo_title(), "Undo Hide");
        let e = h.pop_undo().unwrap();
        assert_eq!(e.label, "Hide");
        h.keep_for_redo(e);
        assert!(h.can_redo());
        assert_eq!(h.redo_title(), "Redo Hide");
        let e = h.pop_redo().unwrap();
        h.keep_for_undo(e);
        assert_eq!(h.undo_title(), "Undo Hide");
        let e = h.pop_undo().unwrap();
        h.keep_for_redo(e);
        h.push(entry("Rate", "rate:a"));
        assert!(!h.can_redo(), "a new action forgets the redo stack");
    }

    #[test]
    fn empty_actions_are_not_recorded_and_the_stack_is_bounded() {
        let mut h = History::default();
        h.push(Entry { label: "nothing".into(), prefs: vec![], moves: vec![] });
        assert!(!h.can_undo());
        for i in 0..DEPTH + 5 {
            h.push(entry(&format!("a{i}"), &format!("k{i}")));
        }
        let mut n = 0;
        while h.pop_undo().is_some() {
            n += 1;
        }
        assert_eq!(n, DEPTH);
    }
}
