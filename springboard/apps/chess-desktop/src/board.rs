//! A chess position: piece placement, the side to move, castling rights, the en passant square and the two clocks.
//! Squares are numbered 0 to 63 with a1 = 0, h1 = 7, a8 = 56 and h8 = 63.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Colour {
    White,
    Black,
}

impl Colour {
    pub fn other(self) -> Colour {
        match self {
            Colour::White => Colour::Black,
            Colour::Black => Colour::White,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Colour::White => "White",
            Colour::Black => "Black",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Pawn,
    Knight,
    Bishop,
    Rook,
    Queen,
    King,
}

impl Kind {
    /// Material value in centipawns. The king is not counted in material.
    pub fn value(self) -> i32 {
        match self {
            Kind::Pawn => 100,
            Kind::Knight => 320,
            Kind::Bishop => 330,
            Kind::Rook => 500,
            Kind::Queen => 900,
            Kind::King => 0,
        }
    }

    /// The letter of the piece in standard algebraic notation (pawns have none).
    pub fn letter(self) -> &'static str {
        match self {
            Kind::Pawn => "",
            Kind::Knight => "N",
            Kind::Bishop => "B",
            Kind::Rook => "R",
            Kind::Queen => "Q",
            Kind::King => "K",
        }
    }

    /// The label of the piece in the promotion choice.
    pub fn name(self) -> &'static str {
        match self {
            Kind::Pawn => "Pawn",
            Kind::Knight => "Knight",
            Kind::Bishop => "Bishop",
            Kind::Rook => "Rook",
            Kind::Queen => "Queen",
            Kind::King => "King",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Piece {
    pub colour: Colour,
    pub kind: Kind,
}

pub type Sq = u8;

pub const WHITE_KINGSIDE: u8 = 1;
pub const WHITE_QUEENSIDE: u8 = 2;
pub const BLACK_KINGSIDE: u8 = 4;
pub const BLACK_QUEENSIDE: u8 = 8;

/// Offsets of a knight's jumps.
pub(crate) const KNIGHT: [(i8, i8); 8] = [(1, 2), (2, 1), (2, -1), (1, -2), (-1, -2), (-2, -1), (-2, 1), (-1, 2)];
/// Offsets of the eight squares around a king.
pub(crate) const KING: [(i8, i8); 8] = [(1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0), (-1, -1), (0, -1), (1, -1)];
/// Diagonal ray directions (bishops and queens).
pub(crate) const DIAGONAL: [(i8, i8); 4] = [(1, 1), (1, -1), (-1, 1), (-1, -1)];
/// Orthogonal ray directions (rooks and queens).
pub(crate) const ORTHOGONAL: [(i8, i8); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];

pub fn sq(file: u8, rank: u8) -> Sq {
    rank * 8 + file
}

pub fn file_of(s: Sq) -> u8 {
    s % 8
}

pub fn rank_of(s: Sq) -> u8 {
    s / 8
}

/// The square name, such as "e4".
pub fn name(s: Sq) -> String {
    format!("{}{}", (b'a' + file_of(s)) as char, rank_of(s) + 1)
}

/// Reads a square name such as "e4".
pub fn parse_square(text: &str) -> Option<Sq> {
    let b = text.as_bytes();
    if b.len() != 2 {
        return None;
    }
    let file = b[0].checked_sub(b'a')?;
    let rank = b[1].checked_sub(b'1')?;
    if file < 8 && rank < 8 {
        Some(sq(file, rank))
    } else {
        None
    }
}

/// The square `df` files and `dr` ranks from `s`, or `None` when that is off the board.
pub fn offset(s: Sq, df: i8, dr: i8) -> Option<Sq> {
    let f = file_of(s) as i8 + df;
    let r = rank_of(s) as i8 + dr;
    if (0..8).contains(&f) && (0..8).contains(&r) {
        Some((r * 8 + f) as u8)
    } else {
        None
    }
}

/// What decides whether two positions are the same for the threefold repetition rule: the placement, the side to
/// move, the castling rights, and the en passant square when a capture is really possible.
pub type RepetitionKey = ([Option<Piece>; 64], Colour, u8, Option<Sq>);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Position {
    pub squares: [Option<Piece>; 64],
    pub turn: Colour,
    pub castling: u8,
    pub en_passant: Option<Sq>,
    /// Half-moves since the last capture or pawn move (the fifty-move clock).
    pub halfmove: u32,
    pub fullmove: u32,
}

impl Position {
    /// The standard starting position.
    pub fn start() -> Position {
        Position::from_fen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1").expect("the start position is valid")
    }

    /// Reads a position in Forsyth-Edwards notation. The clock fields may be left out.
    pub fn from_fen(fen: &str) -> Option<Position> {
        let mut parts = fen.split_whitespace();
        let placement = parts.next()?;
        let turn = match parts.next()? {
            "w" => Colour::White,
            "b" => Colour::Black,
            _ => return None,
        };
        let castling_text = parts.next().unwrap_or("-");
        let en_passant_text = parts.next().unwrap_or("-");
        let halfmove = parts.next().unwrap_or("0").parse::<u32>().ok()?;
        let fullmove = parts.next().unwrap_or("1").parse::<u32>().ok()?;

        let mut squares = [None; 64];
        let rows: Vec<&str> = placement.split('/').collect();
        if rows.len() != 8 {
            return None;
        }
        for (i, row) in rows.iter().enumerate() {
            let rank = 7 - i as u8;
            let mut file: u8 = 0;
            for ch in row.chars() {
                if let Some(empty) = ch.to_digit(10) {
                    file += empty as u8;
                    continue;
                }
                if file >= 8 {
                    return None;
                }
                squares[sq(file, rank) as usize] = Some(piece_from_char(ch)?);
                file += 1;
            }
            if file != 8 {
                return None;
            }
        }

        let mut castling = 0u8;
        for ch in castling_text.chars() {
            castling |= match ch {
                'K' => WHITE_KINGSIDE,
                'Q' => WHITE_QUEENSIDE,
                'k' => BLACK_KINGSIDE,
                'q' => BLACK_QUEENSIDE,
                '-' => 0,
                _ => return None,
            };
        }
        let en_passant = if en_passant_text == "-" { None } else { Some(parse_square(en_passant_text)?) };
        Some(Position { squares, turn, castling, en_passant, halfmove, fullmove })
    }

    pub fn at(&self, s: Sq) -> Option<Piece> {
        self.squares[s as usize]
    }

    pub fn king_square(&self, colour: Colour) -> Option<Sq> {
        (0..64u8).find(|&s| self.at(s) == Some(Piece { colour, kind: Kind::King }))
    }

    /// True when some piece of `by` attacks square `s`.
    pub fn attacked(&self, s: Sq, by: Colour) -> bool {
        // a pawn attacks diagonally forward, so the attacking pawns stand one rank behind the target
        let back: i8 = if by == Colour::White { -1 } else { 1 };
        for df in [-1i8, 1] {
            if let Some(from) = offset(s, df, back) {
                if self.at(from) == Some(Piece { colour: by, kind: Kind::Pawn }) {
                    return true;
                }
            }
        }
        for (df, dr) in KNIGHT {
            if let Some(from) = offset(s, df, dr) {
                if self.at(from) == Some(Piece { colour: by, kind: Kind::Knight }) {
                    return true;
                }
            }
        }
        for (df, dr) in KING {
            if let Some(from) = offset(s, df, dr) {
                if self.at(from) == Some(Piece { colour: by, kind: Kind::King }) {
                    return true;
                }
            }
        }
        for (df, dr) in DIAGONAL {
            if self.ray_hits(s, df, dr, by, Kind::Bishop) {
                return true;
            }
        }
        for (df, dr) in ORTHOGONAL {
            if self.ray_hits(s, df, dr, by, Kind::Rook) {
                return true;
            }
        }
        false
    }

    /// Walks a ray from `s` and reports whether the first piece on it is `by`'s `slider` or queen.
    fn ray_hits(&self, s: Sq, df: i8, dr: i8, by: Colour, slider: Kind) -> bool {
        let mut cur = s;
        while let Some(next) = offset(cur, df, dr) {
            match self.at(next) {
                None => cur = next,
                Some(p) => return p.colour == by && (p.kind == slider || p.kind == Kind::Queen),
            }
        }
        false
    }

    pub fn in_check(&self, colour: Colour) -> bool {
        match self.king_square(colour) {
            Some(k) => self.attacked(k, colour.other()),
            None => false,
        }
    }

    /// True when the move `from` to `to` by a pawn is an en passant capture, or any other move onto an occupied square.
    pub fn is_capture(&self, from: Sq, to: Sq) -> bool {
        if self.at(to).is_some() {
            return true;
        }
        matches!(self.at(from), Some(Piece { kind: Kind::Pawn, .. })) && Some(to) == self.en_passant && file_of(from) != file_of(to)
    }

    /// True when neither side can mate with the pieces left: king against king, or king and one minor piece against a
    /// king. Other material, including pawns, rooks and queens, always leaves a mate possible.
    pub fn insufficient_material(&self) -> bool {
        let mut minors = 0;
        for p in self.squares.iter().flatten() {
            match p.kind {
                Kind::Pawn | Kind::Rook | Kind::Queen => return false,
                Kind::Knight | Kind::Bishop => minors += 1,
                Kind::King => {}
            }
        }
        minors <= 1
    }

    pub fn repetition_key(&self) -> RepetitionKey {
        let ep = self.en_passant.filter(|_| self.en_passant_possible());
        (self.squares, self.turn, self.castling, ep)
    }

    /// True when a pawn of the side to move stands beside the en passant square and so can take it.
    fn en_passant_possible(&self) -> bool {
        let Some(target) = self.en_passant else {
            return false;
        };
        // the capturing pawn stands one rank behind the target, from the side to move
        let back: i8 = if self.turn == Colour::White { -1 } else { 1 };
        let pawn = Some(Piece { colour: self.turn, kind: Kind::Pawn });
        [-1i8, 1].iter().any(|&df| offset(target, df, back).is_some_and(|s| self.at(s) == pawn))
    }
}

fn piece_from_char(ch: char) -> Option<Piece> {
    let colour = if ch.is_ascii_uppercase() { Colour::White } else { Colour::Black };
    let kind = match ch.to_ascii_lowercase() {
        'p' => Kind::Pawn,
        'n' => Kind::Knight,
        'b' => Kind::Bishop,
        'r' => Kind::Rook,
        'q' => Kind::Queen,
        'k' => Kind::King,
        _ => return None,
    };
    Some(Piece { colour, kind })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn square_names_round_trip() {
        assert_eq!(name(sq(4, 0)), "e1");
        assert_eq!(name(sq(0, 7)), "a8");
        assert_eq!(parse_square("h8"), Some(63));
        assert_eq!(parse_square("i1"), None);
        assert_eq!(parse_square("a0"), None);
        assert_eq!(parse_square("e44"), None);
    }

    #[test]
    fn offsets_stay_on_the_board() {
        assert_eq!(offset(0, -1, 0), None);
        assert_eq!(offset(7, 1, 0), None);
        assert_eq!(offset(0, 1, 1), Some(9));
        assert_eq!(offset(63, 0, 1), None);
    }

    #[test]
    fn start_position_has_the_standard_setup() {
        let p = Position::start();
        assert_eq!(p.at(4), Some(Piece { colour: Colour::White, kind: Kind::King }));
        assert_eq!(p.at(60), Some(Piece { colour: Colour::Black, kind: Kind::King }));
        assert_eq!(p.at(12), Some(Piece { colour: Colour::White, kind: Kind::Pawn }));
        assert_eq!(p.castling, 15);
        assert_eq!(p.turn, Colour::White);
        assert!(!p.in_check(Colour::White));
    }

    #[test]
    fn fen_rejects_malformed_text() {
        assert!(Position::from_fen("8/8/8/8/8/8/8 w - - 0 1").is_none());
        assert!(Position::from_fen("9/8/8/8/8/8/8/8 w - - 0 1").is_none());
        assert!(Position::from_fen("8/8/8/8/8/8/8/8 x - - 0 1").is_none());
    }

    #[test]
    fn attacks_by_each_piece_kind() {
        // a white rook on d4 attacks along its file, a black knight on f6 attacks g4
        let p = Position::from_fen("4k3/8/5n2/8/3R4/8/8/4K3 w - - 0 1").expect("valid fen");
        assert!(p.attacked(sq(3, 7), Colour::White));
        assert!(p.attacked(sq(6, 3), Colour::Black));
        assert!(!p.attacked(sq(0, 0), Colour::Black));
        assert!(!p.in_check(Colour::White));
    }

    #[test]
    fn insufficient_material_cases() {
        let bare = Position::from_fen("4k3/8/8/8/8/8/8/4K3 w - - 0 1").expect("valid fen");
        assert!(bare.insufficient_material());
        let minor = Position::from_fen("4k3/8/8/8/8/8/8/3BK3 w - - 0 1").expect("valid fen");
        assert!(minor.insufficient_material());
        let pawn = Position::from_fen("4k3/8/8/8/8/8/P7/4K3 w - - 0 1").expect("valid fen");
        assert!(!pawn.insufficient_material());
    }
}
