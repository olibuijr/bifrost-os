//! Text list markers (the list model of rich text): the marker drawn before each item of a list, from the list's
//! marker format, its starting item number and the marker of the item it is nested under. Pure model: the text layout
//! draws the returned marker before the item.

/// The marker format of a list. Bullet formats draw a glyph; the others draw a number or letters with a full stop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Disc,
    Circle,
    Square,
    Box,
    Diamond,
    Check,
    Hyphen,
    Decimal,
    LowercaseAlpha,
    UppercaseAlpha,
    LowercaseRoman,
    UppercaseRoman,
    LowercaseHexadecimal,
    UppercaseHexadecimal,
    Octal,
}

impl Format {
    /// True for the numbered formats, whose items count up from the list's starting item number.
    pub fn is_ordered(self) -> bool {
        !matches!(
            self,
            Format::Disc | Format::Circle | Format::Square | Format::Box | Format::Diamond | Format::Check | Format::Hyphen
        )
    }

    /// The marker of item `number`. Bullets ignore the number.
    pub fn marker(self, number: usize) -> String {
        match self {
            Format::Disc => "•".to_string(),
            Format::Circle => "◦".to_string(),
            Format::Square => "▪".to_string(),
            Format::Box => "□".to_string(),
            Format::Diamond => "◊".to_string(),
            Format::Check => "✓".to_string(),
            Format::Hyphen => "-".to_string(),
            Format::Decimal => format!("{number}."),
            Format::LowercaseAlpha => letters(number, false),
            Format::UppercaseAlpha => letters(number, true),
            Format::LowercaseRoman => roman(number, false),
            Format::UppercaseRoman => roman(number, true),
            Format::LowercaseHexadecimal => format!("{number:x}."),
            Format::UppercaseHexadecimal => format!("{number:X}."),
            Format::Octal => format!("{number:o}."),
        }
    }
}

/// Bijective base-26 letters, as spreadsheet columns count: 1 is a, 26 is z, 27 is aa. Zero has no letter and falls
/// back to decimal.
fn letters(n: usize, upper: bool) -> String {
    if n == 0 {
        return format!("{n}.");
    }
    let base = if upper { b'A' } else { b'a' };
    let mut rest = n;
    let mut out = Vec::new();
    while rest > 0 {
        out.push(char::from(base + ((rest - 1) % 26) as u8));
        rest = (rest - 1) / 26;
    }
    out.reverse();
    let text: String = out.into_iter().collect();
    format!("{text}.")
}

/// Roman numerals for 1 to 3999. Other numbers fall back to decimal.
fn roman(n: usize, upper: bool) -> String {
    const TABLE: [(usize, &str); 13] = [
        (1000, "m"),
        (900, "cm"),
        (500, "d"),
        (400, "cd"),
        (100, "c"),
        (90, "xc"),
        (50, "l"),
        (40, "xl"),
        (10, "x"),
        (9, "ix"),
        (5, "v"),
        (4, "iv"),
        (1, "i"),
    ];
    if n == 0 || n > 3999 {
        return format!("{n}.");
    }
    let mut rest = n;
    let mut s = String::new();
    for &(value, symbol) in TABLE.iter() {
        while rest >= value {
            s.push_str(symbol);
            rest -= value;
        }
    }
    if upper {
        s = s.to_uppercase();
    }
    format!("{s}.")
}

/// One list: its marker format, the number its first item shows, and whether a nested item shows the marker of the
/// item it sits under before its own marker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct List {
    pub format: Format,
    pub starting_item_number: usize,
    pub prepend_enclosing_marker: bool,
}

impl List {
    pub fn new(format: Format) -> Self {
        List { format, starting_item_number: 1, prepend_enclosing_marker: false }
    }

    /// The marker of the item at `index` (counted from 0). `enclosing` is the marker of the item this list is nested
    /// under, if any.
    pub fn marker(&self, index: usize, enclosing: Option<&str>) -> String {
        let own = self.format.marker(self.starting_item_number + index);
        match enclosing {
            Some(outer) if self.prepend_enclosing_marker => format!("{outer}{own}"),
            _ => own,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bullets_are_glyphs_that_ignore_the_number() {
        assert_eq!(Format::Disc.marker(7), "•");
        assert_eq!(Format::Hyphen.marker(1), "-");
        assert!(!Format::Disc.is_ordered());
        assert!(Format::Decimal.is_ordered());
    }

    #[test]
    fn numbering_starts_at_the_starting_item_number() {
        let mut list = List::new(Format::Decimal);
        list.starting_item_number = 3;
        assert_eq!(list.marker(0, None), "3.");
        assert_eq!(list.marker(2, None), "5.");
    }

    #[test]
    fn letters_count_like_spreadsheet_columns() {
        assert_eq!(Format::LowercaseAlpha.marker(1), "a.");
        assert_eq!(Format::LowercaseAlpha.marker(26), "z.");
        assert_eq!(Format::LowercaseAlpha.marker(27), "aa.");
        assert_eq!(Format::LowercaseAlpha.marker(28), "ab.");
        assert_eq!(Format::UppercaseAlpha.marker(27), "AA.");
        assert_eq!(Format::LowercaseAlpha.marker(0), "0.", "zero has no letter");
    }

    #[test]
    fn roman_numerals_with_a_decimal_fallback_past_3999() {
        assert_eq!(Format::LowercaseRoman.marker(4), "iv.");
        assert_eq!(Format::LowercaseRoman.marker(1994), "mcmxciv.");
        assert_eq!(Format::UppercaseRoman.marker(9), "IX.");
        assert_eq!(Format::UppercaseRoman.marker(4000), "4000.");
    }

    #[test]
    fn hexadecimal_and_octal_numbering() {
        assert_eq!(Format::LowercaseHexadecimal.marker(255), "ff.");
        assert_eq!(Format::UppercaseHexadecimal.marker(255), "FF.");
        assert_eq!(Format::Octal.marker(8), "10.");
    }

    #[test]
    fn a_nested_item_shows_the_enclosing_marker_only_when_asked() {
        let mut list = List::new(Format::LowercaseAlpha);
        assert_eq!(list.marker(0, Some("1.")), "a.", "off by default");
        list.prepend_enclosing_marker = true;
        assert_eq!(list.marker(1, Some("2.")), "2.b.");
        assert_eq!(list.marker(0, None), "a.", "a top-level item has no enclosing marker");
    }
}
