//! Undo and redo for the board: a stack of snapshots, each named for the Edit menu ("Undo Arrange"). A run of typing
//! in one note is one step.

use crate::model::Board;

/// Most steps kept; the oldest go first.
pub const LIMIT: usize = 100;

struct Step {
    name: &'static str,
    board: Board,
}

#[derive(Default)]
pub struct History {
    undo: Vec<Step>,
    redo: Vec<Step>,
    /// The note whose typing run is open: more typing in it joins the step already on the stack.
    typing: Option<u64>,
}

impl History {
    /// Records `before`, the board as it was before an edit named `name`. A new edit empties the redo stack.
    pub fn push(&mut self, name: &'static str, before: Board) {
        self.typing = None;
        self.undo.push(Step { name, board: before });
        if self.undo.len() > LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    /// Records `before` for typing in `note`, unless the previous step is typing in the same note.
    pub fn push_typing(&mut self, note: u64, before: Board) {
        if self.typing == Some(note) && !self.undo.is_empty() {
            self.redo.clear();
            return;
        }
        self.push("Typing", before);
        self.typing = Some(note);
    }

    /// Ends the open typing run, so the next typing is a new step (a click elsewhere, an arrow key).
    pub fn end_typing(&mut self) {
        self.typing = None;
    }

    /// Puts the board back as it was before the last step. False when there is none.
    pub fn undo(&mut self, board: &mut Board) -> bool {
        let Some(step) = self.undo.pop() else { return false };
        self.typing = None;
        let name = step.name;
        self.redo.push(Step { name, board: std::mem::replace(board, step.board) });
        true
    }

    /// Redoes the last undone step. False when there is none.
    pub fn redo(&mut self, board: &mut Board) -> bool {
        let Some(step) = self.redo.pop() else { return false };
        self.typing = None;
        let name = step.name;
        self.undo.push(Step { name, board: std::mem::replace(board, step.board) });
        true
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// The Edit menu title of Undo: "Undo Arrange", or "Undo" with nothing to undo.
    pub fn undo_title(&self) -> String {
        self.undo.last().map_or("Undo".to_string(), |s| format!("Undo {}", s.name))
    }

    pub fn redo_title(&self) -> String {
        self.redo.last().map_or("Redo".to_string(), |s| format!("Redo {}", s.name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn board_with(text: &str) -> Board {
        let mut b = Board::default();
        b.add_text(text, 0.0, 0.0, 0);
        b
    }

    #[test]
    fn undo_and_redo_swap_the_board_and_name_the_step() {
        let mut h = History::default();
        let mut board = board_with("one");
        assert!(!h.undo(&mut board) && !h.redo(&mut board));
        assert_eq!((h.undo_title().as_str(), h.redo_title().as_str()), ("Undo", "Redo"));
        h.push("Arrange", board.clone());
        board = board_with("two");
        assert!(h.can_undo() && !h.can_redo());
        assert_eq!(h.undo_title(), "Undo Arrange");
        assert!(h.undo(&mut board));
        assert_eq!(board.notes[0].text, "one");
        assert_eq!(h.redo_title(), "Redo Arrange");
        assert!(h.redo(&mut board));
        assert_eq!(board.notes[0].text, "two");
        assert!(!h.can_redo());
    }

    #[test]
    fn a_new_edit_clears_the_redo_stack() {
        let mut h = History::default();
        let mut board = board_with("a");
        h.push("Delete", board.clone());
        board = board_with("b");
        h.undo(&mut board);
        assert!(h.can_redo());
        h.push("Colour", board.clone());
        assert!(!h.can_redo());
    }

    #[test]
    fn a_run_of_typing_in_one_note_is_one_step() {
        let mut h = History::default();
        let mut board = board_with("");
        h.push_typing(1, board.clone());
        board.notes[0].text = "a".into();
        h.push_typing(1, board.clone());
        board.notes[0].text = "ab".into();
        h.push_typing(1, board.clone());
        assert!(h.undo(&mut board));
        assert_eq!(board.notes[0].text, "", "one undo takes back the whole run");
        assert!(!h.can_undo());
        assert_eq!(h.redo_title(), "Redo Typing");
    }

    #[test]
    fn typing_in_another_note_or_after_a_break_starts_a_new_step() {
        let mut h = History::default();
        let board = board_with("");
        h.push_typing(1, board.clone());
        h.push_typing(2, board.clone());
        h.end_typing();
        h.push_typing(2, board.clone());
        h.push("Move", board.clone());
        h.push_typing(2, board.clone());
        let mut b = board;
        let mut steps = 0;
        while h.undo(&mut b) {
            steps += 1;
        }
        assert_eq!(steps, 5);
    }

    #[test]
    fn the_stack_keeps_only_the_latest_steps() {
        let mut h = History::default();
        for i in 0..(LIMIT + 10) {
            h.push("Colour", board_with(&i.to_string()));
        }
        let mut b = Board::default();
        let mut steps = 0;
        while h.undo(&mut b) {
            steps += 1;
        }
        assert_eq!(steps, LIMIT);
        assert_eq!(b.notes[0].text, "10", "the oldest ten were dropped");
    }
}
