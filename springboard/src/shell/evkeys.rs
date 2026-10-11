//! evdev key code to the character it types on a US keyboard: app menu chords and type-to-select in menus.

const ROWS: [(u16, &str, &str); 4] = [
    (2, "1234567890", "!@#$%^&*()"),
    (16, "qwertyuiop", "QWERTYUIOP"),
    (30, "asdfghjkl", "ASDFGHJKL"),
    (44, "zxcvbnm", "ZXCVBNM"),
];

const PUNCT: [(u16, char, char); 12] = [
    (12, '-', '_'),
    (13, '=', '+'),
    (26, '[', '{'),
    (27, ']', '}'),
    (39, ';', ':'),
    (40, '\'', '"'),
    (41, '`', '~'),
    (43, '\\', '|'),
    (51, ',', '<'),
    (52, '.', '>'),
    (53, '/', '?'),
    (57, ' ', ' '),
];

pub fn char_for(code: u16, shift: bool) -> Option<char> {
    for (base, lower, upper) in ROWS {
        if code >= base && ((code - base) as usize) < lower.len() {
            let row = if shift { upper } else { lower };
            return row.chars().nth((code - base) as usize);
        }
    }
    PUNCT.iter().find(|p| p.0 == code).map(|p| if shift { p.2 } else { p.1 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_digits_and_punctuation() {
        assert_eq!(char_for(30, false), Some('a'));
        assert_eq!(char_for(30, true), Some('A'));
        assert_eq!(char_for(50, false), Some('m'));
        assert_eq!(char_for(2, false), Some('1'));
        assert_eq!(char_for(2, true), Some('!'));
        assert_eq!(char_for(11, false), Some('0'));
        assert_eq!(char_for(52, false), Some('.'));
        assert_eq!(char_for(57, false), Some(' '));
        assert_eq!(char_for(1, false), None);
        assert_eq!(char_for(28, false), None, "Enter types nothing");
    }
}
