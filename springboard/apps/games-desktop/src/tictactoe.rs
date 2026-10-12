//! Tic-Tac-Toe against the computer. The board is a `GameModel` (the strategist model in `bifrost_app::controls`), so the
//! computer's move is `Minimax::best_move`: an exhaustive search with alpha-beta pruning, deterministic, so the same
//! position always gets the same answer. The player is X and moves first; the computer is O.
//!
//! The game's phase (playing, won, lost) is the shared `Flow` state machine from `phase.rs`: X completing a line wins,
//! O completing a line loses for X. A full board with no line is a draw, which the phase does not track; `over` and
//! `result` read it from the board.

use crate::phase::{Flow, Phase};
use bifrost_app::controls::strategist::{GameModel, Minimax};

/// The two marks. X is the player, O the computer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mark {
    X,
    O,
}

impl Mark {
    fn other(self) -> Mark {
        match self {
            Mark::X => Mark::O,
            Mark::O => Mark::X,
        }
    }
}

/// The eight lines of three cells, by cell index (cells run left to right, top to bottom).
const LINES: [[usize; 3]; 8] = [[0, 1, 2], [3, 4, 5], [6, 7, 8], [0, 3, 6], [1, 4, 7], [2, 5, 8], [0, 4, 8], [2, 4, 6]];

/// Search depth that reaches the end of any Tic-Tac-Toe game (nine moves at most).
const DEPTH: usize = 9;

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Board {
    cells: [Option<Mark>; 9],
}

impl Board {
    /// The mark that owns a full line, if any.
    fn winner(&self) -> Option<Mark> {
        LINES.iter().find_map(|l| {
            let m = self.cells[l[0]]?;
            (self.cells[l[1]] == Some(m) && self.cells[l[2]] == Some(m)).then_some(m)
        })
    }

    fn full(&self) -> bool {
        self.cells.iter().all(|c| c.is_some())
    }

    /// Whose turn it is by count: X when an even number of cells are filled, O otherwise.
    fn turn(&self) -> Mark {
        let filled = self.cells.iter().filter(|c| c.is_some()).count();
        if filled % 2 == 0 {
            Mark::X
        } else {
            Mark::O
        }
    }
}

impl GameModel for Board {
    type Player = Mark;
    type Move = usize;

    fn players(&self) -> Vec<Mark> {
        vec![Mark::X, Mark::O]
    }

    fn active_player(&self) -> Option<Mark> {
        if self.winner().is_some() || self.full() {
            None
        } else {
            Some(self.turn())
        }
    }

    fn moves_for(&self, player: Mark) -> Vec<usize> {
        if self.active_player() != Some(player) {
            return vec![];
        }
        (0..9).filter(|&i| self.cells[i].is_none()).collect()
    }

    fn apply(&mut self, mv: &usize) {
        let m = self.turn();
        self.cells[*mv] = Some(m);
    }

    fn score(&self, player: Mark) -> f64 {
        if self.is_win(player) {
            1.0
        } else if self.is_loss(player) {
            -1.0
        } else {
            0.0
        }
    }

    fn is_win(&self, player: Mark) -> bool {
        self.winner() == Some(player)
    }

    fn is_loss(&self, player: Mark) -> bool {
        self.winner() == Some(player.other())
    }
}

/// One game: the board, its phase, and the square the keyboard or a controller's pad is on.
pub struct TicTacToe {
    board: Board,
    flow: Flow,
    cursor: usize,
}

impl Default for TicTacToe {
    fn default() -> Self {
        Self::new()
    }
}

impl TicTacToe {
    pub fn new() -> Self {
        Self { board: Board::default(), flow: Flow::new(), cursor: 4 }
    }

    /// The mark on square `i`, if any.
    pub fn cell(&self, i: usize) -> Option<Mark> {
        self.board.cells.get(i).copied().flatten()
    }

    /// The square the cursor is on (0 to 8).
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn set_cursor(&mut self, i: usize) {
        if i < 9 {
            self.cursor = i;
        }
    }

    /// Moves the cursor one square: 0 left, 1 right, 2 up, 3 down. It stops at the edge of the board.
    pub fn move_cursor(&mut self, dir: u8) {
        let (r, c) = (self.cursor / 3, self.cursor % 3);
        let (r, c) = match dir {
            0 => (r, c.saturating_sub(1)),
            1 => (r, (c + 1).min(2)),
            2 => (r.saturating_sub(1), c),
            _ => ((r + 1).min(2), c),
        };
        self.cursor = r * 3 + c;
    }

    /// The phase of the game (see `phase.rs`).
    pub fn phase(&self) -> Phase {
        self.flow.phase()
    }

    /// Whether the game has ended: a line was made, or the board is full.
    pub fn over(&self) -> bool {
        self.flow.phase() != Phase::Playing || self.board.full()
    }

    /// The line to show in the message row once the game has ended.
    pub fn result(&self) -> Option<&'static str> {
        match self.flow.phase() {
            Phase::Won => Some("You won!"),
            Phase::Lost => Some("The computer won"),
            Phase::Playing if self.board.full() => Some("Draw"),
            Phase::Playing => None,
        }
    }

    /// The number of marks on the board.
    pub fn marks(&self) -> usize {
        self.board.cells.iter().filter(|c| c.is_some()).count()
    }

    /// The player plays X on `cell`. Returns false, and changes nothing, when the game is over, the square is taken or
    /// it is not X's turn.
    pub fn play(&mut self, cell: usize) -> bool {
        if self.over() || cell >= 9 || self.board.active_player() != Some(Mark::X) || self.board.cells[cell].is_some() {
            return false;
        }
        self.board.apply(&cell);
        self.update_phase();
        true
    }

    /// The computer plays O with the strategist's best move. Returns the square, or None when it is not O's turn.
    pub fn computer_move(&mut self) -> Option<usize> {
        if self.over() || self.board.active_player() != Some(Mark::O) {
            return None;
        }
        let mv = Minimax::new(DEPTH).best_move(&self.board, Mark::O)?;
        self.board.apply(&mv);
        self.update_phase();
        Some(mv)
    }

    /// Moves the phase on when a line has been made: X's line wins, O's line loses for X.
    fn update_phase(&mut self) {
        match self.board.winner() {
            Some(Mark::X) => {
                self.flow.go(Phase::Won);
            }
            Some(Mark::O) => {
                self.flow.go(Phase::Lost);
            }
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A position with the given marks; the phase is worked out from the board as the game would.
    fn with_marks(xs: &[usize], os: &[usize]) -> TicTacToe {
        let mut g = TicTacToe::new();
        for &i in xs {
            g.board.cells[i] = Some(Mark::X);
        }
        for &i in os {
            g.board.cells[i] = Some(Mark::O);
        }
        g.update_phase();
        g
    }

    #[test]
    fn x_moves_first_and_the_turns_alternate() {
        let mut b = Board::default();
        assert_eq!(b.active_player(), Some(Mark::X));
        assert_eq!(b.moves_for(Mark::X).len(), 9);
        assert!(b.moves_for(Mark::O).is_empty());
        b.apply(&4);
        assert_eq!(b.active_player(), Some(Mark::O));
        assert_eq!(b.moves_for(Mark::O), vec![0, 1, 2, 3, 5, 6, 7, 8]);
        b.apply(&0);
        assert_eq!(b.active_player(), Some(Mark::X));
    }

    #[test]
    fn a_line_in_any_direction_is_a_win() {
        for line in LINES {
            let mut b = Board::default();
            for i in line {
                b.cells[i] = Some(Mark::X);
            }
            assert!(b.is_win(Mark::X), "line {line:?}");
            assert!(b.is_loss(Mark::O), "line {line:?}");
            assert_eq!(b.active_player(), None);
        }
    }

    #[test]
    fn a_full_board_without_a_line_is_a_draw() {
        // X O X / X O O / O X X: no line for either mark
        let b = Board { cells: [Some(Mark::X), Some(Mark::O), Some(Mark::X), Some(Mark::X), Some(Mark::O), Some(Mark::O), Some(Mark::O), Some(Mark::X), Some(Mark::X)] };
        assert!(!b.is_win(Mark::X) && !b.is_loss(Mark::X));
        assert_eq!(b.active_player(), None);
        assert_eq!(b.score(Mark::X), 0.0);
    }

    #[test]
    fn the_computer_blocks_a_threat() {
        // X has 0 and 1 and threatens 2; O has 4, so it is O's turn (three marks)
        let mut g = with_marks(&[0, 1], &[4]);
        assert_eq!(g.computer_move(), Some(2));
        assert_eq!(g.cell(2), Some(Mark::O));
        assert_eq!(g.phase(), Phase::Playing);
    }

    #[test]
    fn after_a_corner_the_computer_takes_the_centre() {
        let mut g = TicTacToe::new();
        assert!(g.play(0));
        assert_eq!(g.computer_move(), Some(4));
    }

    #[test]
    fn against_the_centre_the_computer_takes_the_first_corner() {
        let mut g = TicTacToe::new();
        assert!(g.play(4));
        assert_eq!(g.computer_move(), Some(0));
    }

    #[test]
    fn a_taken_square_or_the_wrong_turn_is_refused() {
        let mut g = TicTacToe::new();
        assert!(g.play(0));
        assert!(!g.play(0), "square 0 is taken");
        assert!(!g.play(1), "it is the computer's turn");
        assert_eq!(g.computer_move(), Some(4));
        assert!(g.computer_move().is_none(), "it is X's turn again");
        assert_eq!(g.marks(), 2);
    }

    #[test]
    fn an_x_line_wins_and_ends_the_game() {
        let g = with_marks(&[0, 1, 2], &[3, 4]);
        assert_eq!(g.phase(), Phase::Won);
        assert!(g.over());
        assert_eq!(g.result(), Some("You won!"));
        let mut g = g;
        assert!(!g.play(8), "a finished game takes no more moves");
        assert_eq!(g.computer_move(), None);
    }

    #[test]
    fn an_o_line_loses_for_x() {
        let g = with_marks(&[0, 1], &[3, 4, 5]);
        assert_eq!(g.phase(), Phase::Lost);
        assert_eq!(g.result(), Some("The computer won"));
    }

    #[test]
    fn a_full_board_without_a_line_is_a_draw_in_the_message_row() {
        let g = with_marks(&[0, 2, 3, 7, 8], &[1, 4, 5, 6]);
        assert_eq!(g.phase(), Phase::Playing);
        assert!(g.over());
        assert_eq!(g.result(), Some("Draw"));
    }

    #[test]
    fn a_game_in_progress_has_no_result() {
        let g = TicTacToe::new();
        assert!(!g.over());
        assert_eq!(g.result(), None);
    }

    #[test]
    fn the_cursor_stops_at_the_edges() {
        let mut g = TicTacToe::new();
        assert_eq!(g.cursor(), 4);
        g.move_cursor(0);
        assert_eq!(g.cursor(), 3);
        g.move_cursor(2);
        assert_eq!(g.cursor(), 0);
        g.move_cursor(2);
        g.move_cursor(0);
        assert_eq!(g.cursor(), 0, "left and up stop at the corner");
        g.set_cursor(8);
        g.move_cursor(1);
        g.move_cursor(3);
        assert_eq!(g.cursor(), 8, "right and down stop at the corner");
        g.set_cursor(9);
        assert_eq!(g.cursor(), 8, "a square off the board is ignored");
    }
}
