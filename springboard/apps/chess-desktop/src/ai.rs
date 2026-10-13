//! The computer opponent: a negamax search with alpha-beta pruning, a capture-only quiescence search at the horizon,
//! and a simple static evaluation (material plus a few positional terms). The levels differ in search depth and in
//! how much random noise is added to the scores, so the weaker levels make more mistakes.

use crate::board::{file_of, rank_of, Colour, Kind, Position, Sq};
use crate::moves::Move;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Beginner,
    Easy,
    Medium,
    Hard,
}

impl Level {
    pub const ALL: [Level; 4] = [Level::Beginner, Level::Easy, Level::Medium, Level::Hard];

    pub fn name(self) -> &'static str {
        match self {
            Level::Beginner => "Beginner",
            Level::Easy => "Easy",
            Level::Medium => "Medium",
            Level::Hard => "Hard",
        }
    }

    /// Plies searched before the quiescence search takes over.
    fn depth(self) -> u32 {
        match self {
            Level::Beginner => 1,
            Level::Easy => 2,
            Level::Medium => 3,
            Level::Hard => 4,
        }
    }

    /// Largest random offset, in centipawns, added to each root move's score.
    fn noise(self) -> i32 {
        match self {
            Level::Beginner => 150,
            Level::Easy => 30,
            Level::Medium | Level::Hard => 0,
        }
    }
}

const MATE: i32 = 100_000;
const INFINITY: i32 = 1_000_000;

/// Distance from the centre of the board for a file or rank index: 0 at the edges, 3 in the middle four.
fn centrality(i: i32) -> i32 {
    (7 - (2 * i - 7).abs()) / 2
}

/// The static value of a position for White, in centipawns.
pub fn evaluate(pos: &Position) -> i32 {
    let mut score = 0;
    for (i, slot) in pos.squares.iter().enumerate() {
        if let Some(piece) = slot {
            let square = i as Sq;
            // the positional terms are written from White's side; Black uses the mirrored square
            let view = if piece.colour == Colour::White { square } else { square ^ 56 };
            let value = piece.kind.value() + positional(piece.kind, view);
            score += if piece.colour == Colour::White { value } else { -value };
        }
    }
    score
}

/// A small bonus for a piece on a good square, from White's side of the board.
fn positional(kind: Kind, s: Sq) -> i32 {
    let f = file_of(s) as i32;
    let r = rank_of(s) as i32;
    match kind {
        // advanced pawns, and pawns in the centre files
        Kind::Pawn => r * 4 + centrality(f) * 3,
        Kind::Knight => centrality(f) * 10 + centrality(r) * 8,
        Kind::Bishop => centrality(f) * 5 + centrality(r) * 5,
        Kind::Rook => {
            if r == 6 {
                15
            } else {
                0
            }
        }
        Kind::Queen => centrality(f) * 2 + centrality(r) * 2,
        // a king hides on its back rank and away from the centre
        Kind::King => (if r == 0 { 15 } else { 0 }) - centrality(f) * 6 - centrality(r) * 4,
    }
}

/// The evaluation from the side to move's point of view.
fn side_evaluation(pos: &Position) -> i32 {
    let score = evaluate(pos);
    if pos.turn == Colour::White {
        score
    } else {
        -score
    }
}

/// Move ordering: captures first, the most valuable victim and the least valuable attacker first, then promotions.
fn order(pos: &Position, moves: &mut [Move]) {
    fn key(pos: &Position, mv: &Move) -> i32 {
        let mut score = 0;
        if pos.is_capture(mv.from, mv.to) {
            let victim = pos.at(mv.to).map_or(Kind::Pawn.value(), |p| p.kind.value());
            let attacker = pos.at(mv.from).map_or(0, |p| p.kind.value());
            score += 10 * victim - attacker / 10;
        }
        if mv.promotion.is_some() {
            score += 800;
        }
        score
    }
    moves.sort_by_key(|mv| std::cmp::Reverse(key(pos, mv)));
}

/// Captures only, from a position where the search stopped: stands pat on the static value and trades until quiet.
fn quiesce(pos: &Position, ply: i32, mut alpha: i32, beta: i32) -> i32 {
    let mut moves = pos.legal_moves();
    if moves.is_empty() {
        return if pos.in_check(pos.turn) { -MATE + ply } else { 0 };
    }
    let stand = side_evaluation(pos);
    if stand >= beta {
        return stand;
    }
    if stand > alpha {
        alpha = stand;
    }
    moves.retain(|mv| pos.is_capture(mv.from, mv.to) || mv.promotion.is_some());
    order(pos, &mut moves);
    for mv in moves {
        let score = -quiesce(&pos.make(mv), ply + 1, -beta, -alpha);
        if score >= beta {
            return score;
        }
        if score > alpha {
            alpha = score;
        }
    }
    alpha
}

fn negamax(pos: &Position, depth: u32, ply: i32, mut alpha: i32, beta: i32) -> i32 {
    if depth == 0 {
        return quiesce(pos, ply, alpha, beta);
    }
    let mut moves = pos.legal_moves();
    if moves.is_empty() {
        return if pos.in_check(pos.turn) { -MATE + ply } else { 0 };
    }
    if pos.halfmove >= 100 || pos.insufficient_material() {
        return 0;
    }
    order(pos, &mut moves);
    let mut best = -INFINITY;
    for mv in moves {
        let score = -negamax(&pos.make(mv), depth - 1, ply + 1, -beta, -alpha);
        if score > best {
            best = score;
        }
        if best > alpha {
            alpha = best;
        }
        if alpha >= beta {
            break;
        }
    }
    best
}

/// A small xorshift generator: enough randomness for the weaker levels, and the same moves for the same seed.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Rng {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// A value from `-spread` to `spread`.
    fn jitter(&mut self, spread: i32) -> i32 {
        if spread <= 0 {
            return 0;
        }
        let span = (2 * spread + 1) as u64;
        (self.next() % span) as i32 - spread
    }
}

/// The move the computer plays at `level` in `pos`, or `None` when the side to move has no legal move. `seed` makes
/// the choice repeatable for a given game state.
pub fn choose(pos: &Position, level: Level, seed: u64) -> Option<Move> {
    let mut moves = pos.legal_moves();
    if moves.is_empty() {
        return None;
    }
    order(pos, &mut moves);
    let depth = level.depth();
    let noise = level.noise();
    let mut rng = Rng::new(seed);
    let mut alpha = -INFINITY;
    let mut best: Option<(Move, i32)> = None;
    for mv in moves {
        // with noise every root move is searched with a full window, so each one gets a true score to disturb
        let window_alpha = if noise > 0 { -INFINITY } else { alpha };
        let raw = -negamax(&pos.make(mv), depth - 1, 1, -INFINITY, -window_alpha);
        let score = raw + rng.jitter(noise);
        if best.map_or(true, |(_, s)| score > s) {
            best = Some((mv, score));
        }
        if score > alpha {
            alpha = score;
        }
    }
    best.map(|(mv, _)| mv)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::parse_square;

    fn fen(text: &str) -> Position {
        Position::from_fen(text).expect("test position is valid")
    }

    #[test]
    fn start_position_is_balanced() {
        assert_eq!(evaluate(&Position::start()), 0);
    }

    #[test]
    fn material_decides_the_evaluation() {
        // white is a queen up
        let p = fen("4k3/8/8/8/8/8/8/3QK3 w - - 0 1");
        assert!(evaluate(&p) > 800);
        let mirrored = fen("3qk3/8/8/8/8/8/8/4K3 w - - 0 1");
        assert!(evaluate(&mirrored) < -800);
    }

    #[test]
    fn hard_finds_a_mate_in_one() {
        // Ra8 is mate: the black king on g8 is boxed in by its own pawns
        let p = fen("6k1/5ppp/8/8/8/8/8/R5K1 w - - 0 1");
        let mv = choose(&p, Level::Hard, 1).expect("a move exists");
        assert_eq!(mv, Move::new(parse_square("a1").expect("square"), parse_square("a8").expect("square")));
    }

    #[test]
    fn hard_takes_a_hanging_queen() {
        // the black queen on d5 hangs to the white bishop on b3
        let p = fen("4k3/8/8/3q4/8/1B6/8/4K3 w - - 0 1");
        let mv = choose(&p, Level::Hard, 1).expect("a move exists");
        assert_eq!(mv.to, parse_square("d5").expect("square"));
    }

    #[test]
    fn no_move_when_the_game_is_over() {
        // black is checkmated: the rook on h8 covers the eighth rank and the king is boxed in
        let p = fen("R5k1/5ppp/8/8/8/8/8/6K1 b - - 0 1");
        assert!(choose(&p, Level::Easy, 1).is_none());
    }

    #[test]
    fn the_same_seed_gives_the_same_move() {
        let p = Position::start();
        assert_eq!(choose(&p, Level::Beginner, 9), choose(&p, Level::Beginner, 9));
    }

    #[test]
    fn the_computer_plays_a_legal_move_at_every_level() {
        let p = Position::start();
        let legal = p.legal_moves();
        for level in Level::ALL {
            let mv = choose(&p, level, 3).expect("a move exists");
            assert!(legal.contains(&mv));
        }
    }

    #[test]
    fn centrality_is_symmetric_and_peaks_in_the_middle() {
        assert_eq!(centrality(0), 0);
        assert_eq!(centrality(7), 0);
        assert_eq!(centrality(3), centrality(4));
        assert!(centrality(3) > centrality(1));
    }
}
