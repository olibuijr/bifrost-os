//! A game in progress: the positions from the start, the moves between them, their notation, undo, and the rules that
//! end a game (checkmate, stalemate, the fifty-move rule, threefold repetition and insufficient material).

use crate::board::{file_of, name, rank_of, Colour, Kind, Position};
use crate::moves::Move;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Draw {
    Stalemate,
    Repetition,
    FiftyMove,
    InsufficientMaterial,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// The game goes on; `check` is true when the side to move is in check.
    Ongoing { turn: Colour, check: bool },
    Checkmate { winner: Colour },
    Draw(Draw),
}

impl Status {
    pub fn is_over(self) -> bool {
        !matches!(self, Status::Ongoing { .. })
    }
}

/// The status as a sentence for the board's header.
pub fn describe(status: Status) -> String {
    match status {
        Status::Ongoing { turn, check: true } => format!("{} to move, in check", turn.name()),
        Status::Ongoing { turn, check: false } => format!("{} to move", turn.name()),
        Status::Checkmate { winner } => format!("Checkmate. {} wins", winner.name()),
        Status::Draw(Draw::Stalemate) => "Stalemate. The game is a draw".to_string(),
        Status::Draw(Draw::Repetition) => "Draw by threefold repetition".to_string(),
        Status::Draw(Draw::FiftyMove) => "Draw by the fifty-move rule".to_string(),
        Status::Draw(Draw::InsufficientMaterial) => "Draw: neither side can mate".to_string(),
    }
}

/// The notation of `mv` in `pos` (standard algebraic: "Nf3", "exd6", "O-O", "e8=Q+", "Qh4#"). `legal` is the list of
/// the position's legal moves, which decides how much of the origin square a piece move needs.
pub fn san(pos: &Position, mv: Move, legal: &[Move]) -> String {
    let Some(piece) = pos.at(mv.from) else {
        return String::new();
    };
    let mut text = String::new();
    let castles = piece.kind == Kind::King && (file_of(mv.to) as i8 - file_of(mv.from) as i8).abs() == 2;
    if castles {
        text.push_str(if file_of(mv.to) == 6 { "O-O" } else { "O-O-O" });
    } else {
        let capture = pos.is_capture(mv.from, mv.to);
        if piece.kind == Kind::Pawn {
            if capture {
                text.push(char::from(b'a' + file_of(mv.from)));
            }
        } else {
            text.push_str(piece.kind.letter());
            text.push_str(&disambiguation(pos, mv, piece.kind, legal));
        }
        if capture {
            text.push('x');
        }
        text.push_str(&name(mv.to));
        if let Some(kind) = mv.promotion {
            text.push('=');
            text.push_str(kind.letter());
        }
    }
    let after = pos.make(mv);
    if after.in_check(after.turn) {
        text.push(if after.legal_moves().is_empty() { '#' } else { '+' });
    }
    text
}

/// The part of the origin square a piece move needs to tell it from another piece of the same kind that can reach the
/// same square: the file, else the rank, else both.
fn disambiguation(pos: &Position, mv: Move, kind: Kind, legal: &[Move]) -> String {
    let rivals: Vec<Move> = legal
        .iter()
        .copied()
        .filter(|m| m.to == mv.to && m.from != mv.from && pos.at(m.from).map(|p| p.kind) == Some(kind))
        .collect();
    if rivals.is_empty() {
        return String::new();
    }
    let same_file = rivals.iter().any(|m| file_of(m.from) == file_of(mv.from));
    let same_rank = rivals.iter().any(|m| rank_of(m.from) == rank_of(mv.from));
    if !same_file {
        char::from(b'a' + file_of(mv.from)).to_string()
    } else if !same_rank {
        (rank_of(mv.from) + 1).to_string()
    } else {
        name(mv.from)
    }
}

pub struct Game {
    /// `positions[0]` is the start; the last entry is the position now. `moves[i]` leads from `positions[i]` to
    /// `positions[i + 1]`.
    positions: Vec<Position>,
    moves: Vec<Move>,
    notation: Vec<String>,
}

impl Game {
    pub fn new() -> Game {
        Game::from_position(Position::start())
    }

    pub fn from_position(start: Position) -> Game {
        Game { positions: vec![start], moves: Vec::new(), notation: Vec::new() }
    }

    pub fn position(&self) -> &Position {
        self.positions.last().expect("a game always has a position")
    }

    pub fn turn(&self) -> Colour {
        self.position().turn
    }

    pub fn legal_moves(&self) -> Vec<Move> {
        self.position().legal_moves()
    }

    /// Plays `mv` if it is legal in the position now. False, and no change, otherwise.
    pub fn play(&mut self, mv: Move) -> bool {
        let pos = *self.position();
        let legal = pos.legal_moves();
        if !legal.contains(&mv) {
            return false;
        }
        self.notation.push(san(&pos, mv, &legal));
        self.positions.push(pos.make(mv));
        self.moves.push(mv);
        true
    }

    /// Takes back the last move. False when no move has been played.
    pub fn undo(&mut self) -> bool {
        if self.moves.pop().is_none() {
            return false;
        }
        self.positions.pop();
        self.notation.pop();
        true
    }

    pub fn moves_played(&self) -> usize {
        self.moves.len()
    }

    /// The legal move sequence from the starting position, for restoring a saved game.
    pub fn moves_played_list(&self) -> &[Move] {
        &self.moves
    }

    pub fn last_move(&self) -> Option<Move> {
        self.moves.last().copied()
    }

    /// The notation of every move so far, in order.
    pub fn notation(&self) -> &[String] {
        &self.notation
    }

    /// How many times the position now has occurred, counting this one.
    pub fn repetitions(&self) -> usize {
        let key = self.position().repetition_key();
        self.positions.iter().filter(|p| p.repetition_key() == key).count()
    }

    pub fn status(&self) -> Status {
        let pos = self.position();
        let turn = pos.turn;
        let check = pos.in_check(turn);
        if pos.legal_moves().is_empty() {
            return if check { Status::Checkmate { winner: turn.other() } } else { Status::Draw(Draw::Stalemate) };
        }
        // a mate on the move that makes the fifty-move count is still a mate, so the checks above come first
        if pos.halfmove >= 100 {
            return Status::Draw(Draw::FiftyMove);
        }
        if self.repetitions() >= 3 {
            return Status::Draw(Draw::Repetition);
        }
        if pos.insufficient_material() {
            return Status::Draw(Draw::InsufficientMaterial);
        }
        Status::Ongoing { turn, check }
    }
}

impl Default for Game {
    fn default() -> Self {
        Game::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::{parse_square, Piece};

    fn sq(text: &str) -> u8 {
        parse_square(text).expect("square")
    }

    fn mv(from: &str, to: &str) -> Move {
        Move::new(sq(from), sq(to))
    }

    fn fen(text: &str) -> Position {
        Position::from_fen(text).expect("test position is valid")
    }

    fn play_all(game: &mut Game, moves: &[(&str, &str)]) {
        for (from, to) in moves {
            assert!(game.play(mv(from, to)), "illegal test move {from}{to}");
        }
    }

    #[test]
    fn fools_mate_ends_in_checkmate_with_notation() {
        let mut game = Game::new();
        play_all(&mut game, &[("f2", "f3"), ("e7", "e5"), ("g2", "g4"), ("d8", "h4")]);
        assert_eq!(game.status(), Status::Checkmate { winner: Colour::Black });
        assert_eq!(game.notation(), ["f3", "e5", "g4", "Qh4#"]);
    }

    #[test]
    fn illegal_moves_are_refused_and_change_nothing() {
        let mut game = Game::new();
        assert!(!game.play(mv("e2", "e5")));
        assert!(!game.play(mv("e7", "e5")), "it is not black's move");
        assert_eq!(game.moves_played(), 0);
        assert_eq!(game.turn(), Colour::White);
    }

    #[test]
    fn stalemate_is_a_draw() {
        // black king on h8 with no move, and not in check
        let game = Game::from_position(fen("7k/5Q2/6K1/8/8/8/8/8 b - - 0 1"));
        assert_eq!(game.status(), Status::Draw(Draw::Stalemate));
    }

    #[test]
    fn threefold_repetition_is_a_draw() {
        let mut game = Game::new();
        let shuffle = [("g1", "f3"), ("g8", "f6"), ("f3", "g1"), ("f6", "g8")];
        play_all(&mut game, &shuffle);
        assert_eq!(game.repetitions(), 2);
        assert_eq!(game.status(), Status::Ongoing { turn: Colour::White, check: false });
        play_all(&mut game, &shuffle);
        assert_eq!(game.repetitions(), 3);
        assert_eq!(game.status(), Status::Draw(Draw::Repetition));
    }

    #[test]
    fn fifty_move_rule_ends_the_game_at_one_hundred_half_moves() {
        let mut game = Game::from_position(fen("4k3/8/8/8/8/8/8/R3K3 w - - 99 60"));
        assert_eq!(game.status(), Status::Ongoing { turn: Colour::White, check: false });
        assert!(game.play(mv("e1", "d1")));
        assert_eq!(game.status(), Status::Draw(Draw::FiftyMove));
    }

    #[test]
    fn undo_takes_back_one_move_at_a_time() {
        let mut game = Game::new();
        play_all(&mut game, &[("e2", "e4"), ("e7", "e5")]);
        assert!(game.undo());
        assert_eq!(game.moves_played(), 1);
        assert_eq!(game.turn(), Colour::Black);
        assert!(game.undo());
        assert_eq!(game.position().at(sq("e2")), Some(Piece { colour: Colour::White, kind: Kind::Pawn }));
        assert!(!game.undo());
        assert!(game.notation().is_empty());
    }

    #[test]
    fn castling_is_written_as_o_o() {
        let mut game = Game::from_position(fen("r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1"));
        assert!(game.play(mv("e1", "g1")));
        assert_eq!(game.notation(), ["O-O"]);
    }

    #[test]
    fn en_passant_is_written_with_the_capture_square() {
        let mut game = Game::from_position(fen("4k3/3p4/8/4P3/8/8/8/4K3 b - - 0 1"));
        play_all(&mut game, &[("d7", "d5")]);
        assert!(game.play(mv("e5", "d6")));
        assert_eq!(game.notation(), ["d5", "exd6"]);
        assert_eq!(game.position().at(sq("d5")), None);
    }

    #[test]
    fn rooks_on_one_file_are_told_apart_by_rank() {
        let game = Game::from_position(fen("4k3/8/8/R7/8/8/8/R3K3 w - - 0 1"));
        let legal = game.legal_moves();
        assert_eq!(san(game.position(), mv("a1", "a3"), &legal), "R1a3");
        assert_eq!(san(game.position(), mv("a5", "a3"), &legal), "R5a3");
    }

    #[test]
    fn check_and_promotion_suffixes() {
        // a7-a8=Q gives check to the king on h8 along the eighth rank
        let game = Game::from_position(fen("7k/P7/8/8/8/8/8/4K3 w - - 0 1"));
        let legal = game.legal_moves();
        let queen = Move::promoting(sq("a7"), sq("a8"), Kind::Queen);
        assert!(legal.contains(&queen));
        assert_eq!(san(game.position(), queen, &legal), "a8=Q+");
    }

    #[test]
    fn describe_reads_the_status() {
        assert_eq!(describe(Status::Ongoing { turn: Colour::Black, check: true }), "Black to move, in check");
        assert_eq!(describe(Status::Checkmate { winner: Colour::White }), "Checkmate. White wins");
    }
}
