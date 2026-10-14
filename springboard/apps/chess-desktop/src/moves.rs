//! Move generation and the making of a move: every legal move of a position, including castling, en passant and
//! promotion. A move is only its squares and, for a promotion, the piece chosen; the rest follows from the position.

use crate::board::{
    file_of, offset, rank_of, sq, Colour, Kind, Piece, Position, Sq, BLACK_KINGSIDE, BLACK_QUEENSIDE, DIAGONAL, KING,
    KNIGHT, ORTHOGONAL, WHITE_KINGSIDE, WHITE_QUEENSIDE,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Move {
    pub from: Sq,
    pub to: Sq,
    pub promotion: Option<Kind>,
}

impl Move {
    pub fn new(from: Sq, to: Sq) -> Move {
        Move { from, to, promotion: None }
    }

    pub fn promoting(from: Sq, to: Sq, kind: Kind) -> Move {
        Move { from, to, promotion: Some(kind) }
    }
}

/// The rights a square takes away when a piece leaves it or is captured on it.
fn rights_on(s: Sq) -> u8 {
    match s {
        0 => WHITE_QUEENSIDE,
        4 => WHITE_KINGSIDE | WHITE_QUEENSIDE,
        7 => WHITE_KINGSIDE,
        56 => BLACK_QUEENSIDE,
        60 => BLACK_KINGSIDE | BLACK_QUEENSIDE,
        63 => BLACK_KINGSIDE,
        _ => 0,
    }
}

impl Position {
    /// Every legal move for the side to move.
    pub fn legal_moves(&self) -> Vec<Move> {
        let me = self.turn;
        self.pseudo_moves().into_iter().filter(|mv| !self.make(*mv).in_check(me)).collect()
    }

    /// The moves the side to move can play, before the check that its own king is left safe.
    fn pseudo_moves(&self) -> Vec<Move> {
        let me = self.turn;
        let mut out = Vec::with_capacity(48);
        for from in 0..64u8 {
            let Some(piece) = self.at(from) else {
                continue;
            };
            if piece.colour != me {
                continue;
            }
            match piece.kind {
                Kind::Pawn => self.pawn_moves(from, me, &mut out),
                Kind::Knight => self.step_moves(from, me, &KNIGHT, &mut out),
                Kind::King => {
                    self.step_moves(from, me, &KING, &mut out);
                    self.castle_moves(from, me, &mut out);
                }
                Kind::Bishop => self.slide_moves(from, me, &DIAGONAL, &mut out),
                Kind::Rook => self.slide_moves(from, me, &ORTHOGONAL, &mut out),
                Kind::Queen => {
                    self.slide_moves(from, me, &DIAGONAL, &mut out);
                    self.slide_moves(from, me, &ORTHOGONAL, &mut out);
                }
            }
        }
        out
    }

    fn is_own(&self, s: Sq, me: Colour) -> bool {
        matches!(self.at(s), Some(p) if p.colour == me)
    }

    fn step_moves(&self, from: Sq, me: Colour, deltas: &[(i8, i8)], out: &mut Vec<Move>) {
        for &(df, dr) in deltas {
            if let Some(to) = offset(from, df, dr) {
                if !self.is_own(to, me) {
                    out.push(Move::new(from, to));
                }
            }
        }
    }

    fn slide_moves(&self, from: Sq, me: Colour, deltas: &[(i8, i8)], out: &mut Vec<Move>) {
        for &(df, dr) in deltas {
            let mut cur = from;
            while let Some(to) = offset(cur, df, dr) {
                match self.at(to) {
                    None => out.push(Move::new(from, to)),
                    Some(p) => {
                        if p.colour != me {
                            out.push(Move::new(from, to));
                        }
                        break;
                    }
                }
                cur = to;
            }
        }
    }

    fn pawn_moves(&self, from: Sq, me: Colour, out: &mut Vec<Move>) {
        let (dr, start_rank, promo_rank): (i8, u8, u8) = if me == Colour::White { (1, 1, 7) } else { (-1, 6, 0) };
        if let Some(one) = offset(from, 0, dr) {
            if self.at(one).is_none() {
                push_pawn(from, one, promo_rank, out);
                if rank_of(from) == start_rank {
                    if let Some(two) = offset(one, 0, dr) {
                        if self.at(two).is_none() {
                            out.push(Move::new(from, two));
                        }
                    }
                }
            }
        }
        for df in [-1i8, 1] {
            if let Some(to) = offset(from, df, dr) {
                let enemy = matches!(self.at(to), Some(p) if p.colour != me);
                let en_passant = Some(to) == self.en_passant && self.at(to).is_none();
                if enemy || en_passant {
                    push_pawn(from, to, promo_rank, out);
                }
            }
        }
    }

    fn castle_moves(&self, from: Sq, me: Colour, out: &mut Vec<Move>) {
        let (home, kingside, queenside) = match me {
            Colour::White => (4u8, WHITE_KINGSIDE, WHITE_QUEENSIDE),
            Colour::Black => (60u8, BLACK_KINGSIDE, BLACK_QUEENSIDE),
        };
        if from != home || self.in_check(me) {
            return;
        }
        let rook = Some(Piece { colour: me, kind: Kind::Rook });
        let opp = me.other();
        if self.castling & kingside != 0
            && self.at(home + 3) == rook
            && self.at(home + 1).is_none()
            && self.at(home + 2).is_none()
            && !self.attacked(home + 1, opp)
            && !self.attacked(home + 2, opp)
        {
            out.push(Move::new(home, home + 2));
        }
        if self.castling & queenside != 0
            && self.at(home - 4) == rook
            && self.at(home - 1).is_none()
            && self.at(home - 2).is_none()
            && self.at(home - 3).is_none()
            && !self.attacked(home - 1, opp)
            && !self.attacked(home - 2, opp)
        {
            out.push(Move::new(home, home - 2));
        }
    }

    /// The position after `mv`. The move is taken as legal; a move from an empty square leaves the position as it was.
    pub fn make(&self, mv: Move) -> Position {
        let mut p = *self;
        let Some(piece) = self.at(mv.from) else {
            return p;
        };

        // en passant: the captured pawn stands beside the moving pawn's target, on the square the pawn came from
        let en_passant_capture = piece.kind == Kind::Pawn
            && Some(mv.to) == self.en_passant
            && self.at(mv.to).is_none()
            && file_of(mv.from) != file_of(mv.to);
        if en_passant_capture {
            let behind = if piece.colour == Colour::White { mv.to - 8 } else { mv.to + 8 };
            p.squares[behind as usize] = None;
        }

        p.squares[mv.from as usize] = None;
        let placed = match mv.promotion {
            Some(kind) if piece.kind == Kind::Pawn => Piece { colour: piece.colour, kind },
            _ => piece,
        };
        p.squares[mv.to as usize] = Some(placed);

        // castling moves the rook as well as the king
        let castles = piece.kind == Kind::King && (file_of(mv.to) as i8 - file_of(mv.from) as i8).abs() == 2;
        if castles {
            let rank = rank_of(mv.from);
            let (rook_from, rook_to) = if file_of(mv.to) == 6 { (sq(7, rank), sq(5, rank)) } else { (sq(0, rank), sq(3, rank)) };
            p.squares[rook_to as usize] = p.squares[rook_from as usize].take();
        }

        // a king move gives up both rights, and a piece leaving or landing on a corner or the king's square gives up
        // the rights tied to it
        p.castling &= !(rights_on(mv.from) | rights_on(mv.to));

        let double_step = piece.kind == Kind::Pawn && (mv.to as i16 - mv.from as i16).abs() == 16;
        p.en_passant = if double_step { Some((mv.from + mv.to) / 2) } else { None };

        let capture = self.at(mv.to).is_some() || en_passant_capture;
        p.halfmove = if piece.kind == Kind::Pawn || capture { 0 } else { self.halfmove + 1 };
        if self.turn == Colour::Black {
            p.fullmove = self.fullmove + 1;
        }
        p.turn = self.turn.other();
        p
    }
}

/// A pawn move onto `to`; a move onto the last rank is a promotion, with one move for each piece it may become.
fn push_pawn(from: Sq, to: Sq, promo_rank: u8, out: &mut Vec<Move>) {
    if rank_of(to) == promo_rank {
        for kind in [Kind::Queen, Kind::Rook, Kind::Bishop, Kind::Knight] {
            out.push(Move::promoting(from, to, kind));
        }
    } else {
        out.push(Move::new(from, to));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::parse_square;

    fn fen(text: &str) -> Position {
        Position::from_fen(text).expect("test position is valid")
    }

    fn mv(from: &str, to: &str) -> Move {
        Move::new(parse_square(from).expect("square"), parse_square(to).expect("square"))
    }

    fn has(moves: &[Move], from: &str, to: &str) -> bool {
        moves.contains(&mv(from, to))
    }

    #[test]
    fn start_position_has_twenty_moves() {
        let moves = Position::start().legal_moves();
        assert_eq!(moves.len(), 20);
    }

    #[test]
    fn pinned_piece_cannot_leave_the_pin_line() {
        // the bishop on e2 is pinned to the king on e1 by the rook on e8
        let p = fen("4r1k1/8/8/8/8/8/4B3/4K3 w - - 0 1");
        let moves = p.legal_moves();
        assert!(!moves.iter().any(|m| m.from == parse_square("e2").expect("square")));
    }

    #[test]
    fn castling_both_sides_when_clear() {
        let p = fen("r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1");
        let moves = p.legal_moves();
        assert!(has(&moves, "e1", "g1"));
        assert!(has(&moves, "e1", "c1"));
    }

    #[test]
    fn no_castling_through_an_attacked_square() {
        // the black rook on f8 attacks f1, so the king may not pass it to castle short
        let p = fen("4kr2/8/8/8/8/8/8/R3K2R w KQ - 0 1");
        let moves = p.legal_moves();
        assert!(!has(&moves, "e1", "g1"));
        assert!(has(&moves, "e1", "c1"));
    }

    #[test]
    fn no_castling_out_of_check() {
        let p = fen("4r1k1/8/8/8/8/8/8/R3K2R w KQ - 0 1");
        let moves = p.legal_moves();
        assert!(!has(&moves, "e1", "g1"));
        assert!(!has(&moves, "e1", "c1"));
    }

    #[test]
    fn castling_moves_the_rook_and_drops_the_rights() {
        let p = fen("r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1");
        let after = p.make(mv("e1", "g1"));
        assert_eq!(after.at(parse_square("g1").expect("square")), Some(Piece { colour: Colour::White, kind: Kind::King }));
        assert_eq!(after.at(parse_square("f1").expect("square")), Some(Piece { colour: Colour::White, kind: Kind::Rook }));
        assert_eq!(after.at(parse_square("h1").expect("square")), None);
        assert_eq!(after.castling & (WHITE_KINGSIDE | WHITE_QUEENSIDE), 0);
        assert_eq!(after.castling & (BLACK_KINGSIDE | BLACK_QUEENSIDE), BLACK_KINGSIDE | BLACK_QUEENSIDE);
    }

    #[test]
    fn rook_move_gives_up_only_its_side() {
        let p = fen("r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1");
        let after = p.make(mv("h1", "h2"));
        assert_eq!(after.castling & WHITE_KINGSIDE, 0);
        assert_eq!(after.castling & WHITE_QUEENSIDE, WHITE_QUEENSIDE);
    }

    #[test]
    fn en_passant_capture_removes_the_passed_pawn() {
        // white pawn on e5, black plays d7-d5 next to it: exd6 en passant is legal
        let p = fen("4k3/3p4/8/4P3/8/8/8/4K3 b - - 0 1");
        let after_double = p.make(mv("d7", "d5"));
        assert_eq!(after_double.en_passant, parse_square("d6"));
        let target = Position::from_fen("4k3/8/8/3pP3/8/8/8/4K3 w - d6 0 2").expect("valid");
        let moves = target.legal_moves();
        assert!(has(&moves, "e5", "d6"));
        let after = target.make(mv("e5", "d6"));
        assert_eq!(after.at(parse_square("d5").expect("square")), None);
        assert_eq!(after.at(parse_square("d6").expect("square")), Some(Piece { colour: Colour::White, kind: Kind::Pawn }));
    }

    #[test]
    fn promotion_offers_four_pieces() {
        let p = fen("4k3/P7/8/8/8/8/8/4K3 w - - 0 1");
        let promos: Vec<Move> = p.legal_moves().into_iter().filter(|m| m.from == parse_square("a7").expect("square")).collect();
        assert_eq!(promos.len(), 4);
        let queen = promos.iter().find(|m| m.promotion == Some(Kind::Queen)).copied().expect("queen promotion");
        let after = p.make(queen);
        assert_eq!(after.at(parse_square("a8").expect("square")), Some(Piece { colour: Colour::White, kind: Kind::Queen }));
    }

    #[test]
    fn double_step_is_blocked_by_a_piece_in_front() {
        let p = fen("4k3/8/8/8/8/p7/P7/4K3 w - - 0 1");
        let moves = p.legal_moves();
        assert!(!has(&moves, "a2", "a4"));
        assert!(!has(&moves, "a2", "a3"));
    }

    #[test]
    fn fifty_move_clock_resets_on_pawn_moves_and_captures() {
        let p = fen("4k3/8/8/8/8/8/P7/4K3 w - - 40 1");
        assert_eq!(p.make(mv("a2", "a3")).halfmove, 0);
        assert_eq!(p.make(mv("e1", "d1")).halfmove, 41);
    }
}
