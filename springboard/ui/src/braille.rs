//! Braille translation for a Braille display: text to six-dot cells, and a display window that shows a fixed number of
//! cells and pans over a longer line. The translator covers the Grade 1 letters, digits, capital sign, numeric sign and
//! the common punctuation marks, each character on its own (no contractions). Characters outside that set fail with
//! `BrailleError::UnsupportedCharacter`, so nothing is shown wrongly. Pure data: a display driver sends the cells to the
//! hardware; this module does no device I/O.

/// One Braille cell. Bit 0 is dot 1, bit 5 is dot 6, the order of the Unicode Braille Patterns block.
pub type Cell = u8;

/// The Braille cell with the given dot (1 to 6).
const fn dot(n: u8) -> Cell {
    1 << (n - 1)
}

/// Capital sign: dot 6 before a capital letter.
pub const CAPITAL_SIGN: Cell = dot(6);
/// Numeric sign: dots 3456 before a digit that starts a number.
pub const NUMERIC_SIGN: Cell = dot(3) | dot(4) | dot(5) | dot(6);

/// Dot numbers of the letters a to z, in order.
const LETTER_DOTS: [&str; 26] = [
    "1", "12", "14", "145", "15", "124", "1245", "125", "24", "245", "13", "123", "134", "1345", "135", "1234", "12345",
    "1235", "234", "2345", "136", "1236", "2456", "1346", "13456", "1356",
];

/// Punctuation marks with their dot numbers, each a single cell.
const PUNCTUATION: [(char, &str); 9] = [
    (',', "2"),
    (';', "23"),
    (':', "25"),
    ('.', "256"),
    ('?', "236"),
    ('!', "235"),
    ('-', "36"),
    ('\'', "3"),
    ('/', "34"),
];

/// Why a text cannot be translated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrailleError {
    UnsupportedCharacter(char),
}

fn cell_of(dots: &str) -> Cell {
    dots.bytes().fold(0, |m, b| m | dot(b - b'0'))
}

fn letter_cell(lower: char) -> Option<Cell> {
    if lower.is_ascii_lowercase() {
        Some(cell_of(LETTER_DOTS[(lower as u8 - b'a') as usize]))
    } else {
        None
    }
}

/// Digits use the cells of a to j: 1 is a, 2 is b, and 0 is j.
fn digit_cell(d: char) -> Option<Cell> {
    let index = match d {
        '1'..='9' => d as u8 - b'1',
        '0' => 9,
        _ => return None,
    };
    Some(cell_of(LETTER_DOTS[index as usize]))
}

/// Pushes the cells of one character. Some marks take two cells (the parentheses and the double quote): a dot 5
/// prefix cell, then the mark's own cell.
fn char_cells(c: char, out: &mut Vec<Cell>) -> Result<(), BrailleError> {
    if c == ' ' || c == '\n' || c == '\t' {
        out.push(0);
        return Ok(());
    }
    if c.is_ascii_uppercase() {
        out.push(CAPITAL_SIGN);
        if let Some(cell) = letter_cell(c.to_ascii_lowercase()) {
            out.push(cell);
            return Ok(());
        }
    }
    if let Some(cell) = letter_cell(c) {
        out.push(cell);
        return Ok(());
    }
    match c {
        '(' => {
            out.push(dot(5));
            out.push(cell_of("126"));
            return Ok(());
        }
        ')' => {
            out.push(dot(5));
            out.push(cell_of("345"));
            return Ok(());
        }
        '"' => {
            out.push(dot(5));
            out.push(cell_of("236"));
            return Ok(());
        }
        _ => {}
    }
    if let Some((_, dots)) = PUNCTUATION.iter().find(|(p, _)| *p == c) {
        out.push(cell_of(dots));
        return Ok(());
    }
    Err(BrailleError::UnsupportedCharacter(c))
}

/// Translates text to Braille cells. A digit that does not follow another digit starts with the numeric sign.
/// A capital letter gets the capital sign before its own cell, letter by letter.
pub fn translate(text: &str) -> Result<Vec<Cell>, BrailleError> {
    let mut out = Vec::new();
    let mut prev_digit = false;
    for c in text.chars() {
        if let Some(cell) = digit_cell(c) {
            if !prev_digit {
                out.push(NUMERIC_SIGN);
            }
            out.push(cell);
            prev_digit = true;
            continue;
        }
        prev_digit = false;
        char_cells(c, &mut out)?;
    }
    Ok(out)
}

/// The cells as Unicode Braille Patterns characters, one character per cell.
pub fn to_unicode(cells: &[Cell]) -> String {
    cells.iter().filter_map(|&c| char::from_u32(0x2800 + c as u32)).collect()
}

/// A display of `width` cells that shows part of a line and pans over it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Display {
    width: usize,
    offset: usize,
}

impl Display {
    /// A display with `width` cells (at least one), starting at the first cell.
    pub fn new(width: usize) -> Self {
        Self { width: width.max(1), offset: 0 }
    }

    pub fn width(&self) -> usize {
        self.width
    }

    /// The first cell shown, counted from the start of the line.
    pub fn offset(&self) -> usize {
        self.offset
    }

    /// The cells on the display for a line: the `width` cells from the offset, fewer at the end of the line.
    pub fn window<'a>(&self, line: &'a [Cell]) -> &'a [Cell] {
        let start = self.offset.min(line.len());
        let end = (start + self.width).min(line.len());
        &line[start..end]
    }

    /// Pans by `step` cells (negative goes back). The offset stays between the start of the line and the last full
    /// window, so the display never shows empty space past the end of the line.
    pub fn pan(&mut self, step: isize, line_len: usize) {
        let last = line_len.saturating_sub(self.width) as isize;
        self.offset = (self.offset as isize + step).clamp(0, last) as usize;
    }

    /// The number of display pages a line of `line_len` cells needs; an empty line still takes one page.
    pub fn pages(&self, line_len: usize) -> usize {
        if line_len == 0 {
            1
        } else {
            (line_len + self.width - 1) / self.width
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_map_to_the_grade_one_cells() {
        assert_eq!(translate("ab"), Ok(vec![0x01, 0x03]));
        assert_eq!(to_unicode(&[0x01, 0x03]), "\u{2801}\u{2803}");
    }

    #[test]
    fn a_capital_letter_gets_the_capital_sign_first() {
        // H is dots 125 (0x13), the capital sign is dot 6 (0x20), i is dots 24 (0x0A).
        assert_eq!(translate("Hi"), Ok(vec![0x20, 0x13, 0x0A]));
    }

    #[test]
    fn digits_use_the_numeric_sign_once_per_number() {
        // Numeric sign 0x3C, 1 is a (0x01), 2 is b (0x03), 0 is j (dots 245, 0x1A).
        assert_eq!(translate("12"), Ok(vec![0x3C, 0x01, 0x03]));
        assert_eq!(translate("10"), Ok(vec![0x3C, 0x01, 0x1A]));
        // A digit after a letter starts a new number, so it gets its own sign.
        assert_eq!(translate("a1 b"), Ok(vec![0x01, 0x3C, 0x01, 0x00, 0x03]));
    }

    #[test]
    fn punctuation_and_spaces_are_single_cells() {
        // comma dot 2 (0x02), space (0x00), full stop dots 256 (0x32).
        assert_eq!(translate("a, b."), Ok(vec![0x01, 0x02, 0x00, 0x03, 0x32]));
    }

    #[test]
    fn parentheses_take_two_cells() {
        assert_eq!(translate("("), Ok(vec![0x10, 0x23]));
        assert_eq!(translate(")"), Ok(vec![0x10, 0x1C]));
    }

    #[test]
    fn an_unsupported_character_is_reported_not_guessed() {
        assert_eq!(translate("é"), Err(BrailleError::UnsupportedCharacter('é')));
        assert_eq!(translate("a&b"), Err(BrailleError::UnsupportedCharacter('&')));
    }

    #[test]
    fn the_display_shows_a_window_and_pans_within_the_line() {
        let line: Vec<Cell> = vec![1, 2, 3, 4, 5];
        let mut d = Display::new(2);
        assert_eq!(d.window(&line), &[1, 2]);
        d.pan(2, line.len());
        assert_eq!(d.window(&line), &[3, 4]);
        d.pan(10, line.len());
        assert_eq!(d.offset(), 3);
        assert_eq!(d.window(&line), &[4, 5]);
        d.pan(-1, line.len());
        assert_eq!(d.offset(), 2);
        assert_eq!(d.pages(line.len()), 3);
        assert_eq!(d.pages(0), 1);
    }
}
