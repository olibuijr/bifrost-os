//! Undo and Redo for the changes a person makes in Photos: a favourite mark, a rating, hiding, a saved edit, album
//! membership, details, and deleting. Almost every change is a setting in `photos.ini`, so a change remembers the
//! value a setting had; applying it swaps that value back and hands over the inverse, which is what Redo applies.
//! Deleting moves a file, so it has its own kind. The stack only records and orders; `Photos` applies the changes.

use std::path::PathBuf;

/// How many steps are kept.
pub const LIMIT: usize = 50;

/// One reversible change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    /// the setting `key` had the value `was` (empty = not set)
    Pref { key: String, was: String },
    /// to reverse it, move the photo now at `path` into Recently Deleted (`trashed`) or back into the library
    Trash { path: PathBuf, trashed: bool },
}

impl Change {
    pub fn pref(key: &str, was: &str) -> Change {
        Change::Pref { key: key.to_string(), was: was.to_string() }
    }
}

/// A named step: the changes one action made, undone together.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    pub label: String,
    pub changes: Vec<Change>,
}

/// The steps that can be undone, and the ones that were undone and can be redone.
#[derive(Clone, Debug, Default)]
pub struct Stack {
    undo: Vec<Step>,
    redo: Vec<Step>,
}

impl Stack {
    /// Records a new step; whatever could be redone is gone. A step without changes is not recorded.
    pub fn record(&mut self, label: &str, changes: Vec<Change>) {
        if changes.is_empty() {
            return;
        }
        self.redo.clear();
        self.undo.push(Step { label: label.to_string(), changes });
        if self.undo.len() > LIMIT {
            self.undo.remove(0);
        }
    }

    /// The label of the step Undo would take back.
    pub fn undo_label(&self) -> Option<&str> {
        self.undo.last().map(|s| s.label.as_str())
    }

    /// The label of the step Redo would apply again.
    pub fn redo_label(&self) -> Option<&str> {
        self.redo.last().map(|s| s.label.as_str())
    }

    /// Takes the newest step to undo it.
    pub fn pop_undo(&mut self) -> Option<Step> {
        self.undo.pop()
    }

    /// Takes the newest undone step to redo it.
    pub fn pop_redo(&mut self) -> Option<Step> {
        self.redo.pop()
    }

    /// Keeps `step`, which was just undone, so Redo can apply it again. `step.changes` are the inverses the undo
    /// produced.
    pub fn push_redo(&mut self, step: Step) {
        self.redo.push(step);
    }

    /// Keeps `step`, which was just redone, so Undo can take it back. Unlike `record` it leaves the redo list alone.
    pub fn push_undo(&mut self, step: Step) {
        self.undo.push(step);
        if self.undo.len() > LIMIT {
            self.undo.remove(0);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.undo.is_empty() && self.redo.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(n: usize) -> Vec<Change> {
        vec![Change::pref("fav:/p/a.jpg", &n.to_string())]
    }

    #[test]
    fn steps_come_back_newest_first() {
        let mut s = Stack::default();
        assert!(s.is_empty() && s.undo_label().is_none() && s.redo_label().is_none());
        s.record("Favorite", step(1));
        s.record("Rating", step(2));
        assert_eq!(s.undo_label(), Some("Rating"));
        let top = s.pop_undo().unwrap();
        assert_eq!(top.changes, step(2));
        assert_eq!(s.undo_label(), Some("Favorite"));
    }

    #[test]
    fn an_undone_step_can_be_redone_until_something_new_happens() {
        let mut s = Stack::default();
        s.record("Rating", step(1));
        let undone = s.pop_undo().unwrap();
        s.push_redo(Step { label: undone.label.clone(), changes: step(9) });
        assert_eq!(s.redo_label(), Some("Rating"));
        assert!(s.undo_label().is_none());
        let redone = s.pop_redo().unwrap();
        s.push_undo(redone);
        assert_eq!(s.undo_label(), Some("Rating"));
        assert!(s.redo_label().is_none());
        // a new change drops the redo list
        let undone = s.pop_undo().unwrap();
        s.push_redo(undone);
        s.record("Hide", step(3));
        assert!(s.redo_label().is_none());
    }

    #[test]
    fn empty_steps_are_not_recorded_and_the_oldest_fall_off() {
        let mut s = Stack::default();
        s.record("Nothing", vec![]);
        assert!(s.is_empty());
        for n in 0..LIMIT + 5 {
            s.record(&format!("step {n}"), step(n));
        }
        let mut count = 0;
        let mut last = String::new();
        while let Some(step) = s.pop_undo() {
            count += 1;
            last = step.label;
        }
        assert_eq!(count, LIMIT);
        assert_eq!(last, "step 5", "the five oldest are gone");
    }

    #[test]
    fn a_trash_change_names_the_file_and_where_it_should_end_up() {
        let c = Change::Trash { path: PathBuf::from("/p/.trash/a.jpg"), trashed: false };
        assert_ne!(c, Change::Trash { path: PathBuf::from("/p/.trash/a.jpg"), trashed: true });
    }
}
