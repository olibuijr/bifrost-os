//! Substitutions for typed text (Edit > Substitutions): Smart Quotes turns straight quotes into curly ones, and Smart
//! Dashes turns a second hyphen typed after a hyphen into an em dash. The rules look only at the character before the
//! typed one.

/// What typing a character becomes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Typed {
    /// The text to insert in place of the character.
    Insert(String),
    /// Delete the character before the caret, then insert this one (the second hyphen of an em dash).
    Replace(char),
}

/// Whether a quote typed after `before` opens a quotation: at the start, after a space, or after an opening bracket.
fn opens(before: Option<char>) -> bool {
    match before {
        None => true,
        Some(c) => c.is_whitespace() || matches!(c, '(' | '[' | '{' | '\u{2014}' | '\u{2013}' | '\u{201c}' | '\u{2018}'),
    }
}

/// The substitution for typing `c` after the character `before`, or `None` when the character is typed as it is.
pub fn typed(before: Option<char>, c: char, quotes: bool, dashes: bool) -> Option<Typed> {
    match c {
        '"' if quotes => Some(Typed::Insert(if opens(before) { "\u{201c}" } else { "\u{201d}" }.to_string())),
        // an apostrophe after a letter closes a word, as in "don't"
        '\'' if quotes => Some(Typed::Insert(if opens(before) { "\u{2018}" } else { "\u{2019}" }.to_string())),
        '-' if dashes && before == Some('-') => Some(Typed::Replace('\u{2014}')),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn insert(s: &str) -> Option<Typed> {
        Some(Typed::Insert(s.to_string()))
    }

    #[test]
    fn quotes_open_after_a_space_and_close_after_a_word() {
        assert_eq!(typed(None, '"', true, false), insert("\u{201c}"));
        assert_eq!(typed(Some(' '), '"', true, false), insert("\u{201c}"));
        assert_eq!(typed(Some('n'), '"', true, false), insert("\u{201d}"));
        assert_eq!(typed(Some('('), '\'', true, false), insert("\u{2018}"));
        assert_eq!(typed(Some('n'), '\'', true, false), insert("\u{2019}"), "don't");
    }

    #[test]
    fn two_hyphens_make_an_em_dash_and_one_hyphen_does_not() {
        assert_eq!(typed(Some('-'), '-', false, true), Some(Typed::Replace('\u{2014}')));
        assert_eq!(typed(Some('a'), '-', false, true), None);
        assert_eq!(typed(Some('-'), '-', false, false), None, "the setting is off");
    }

    #[test]
    fn the_settings_switch_each_rule_off_separately() {
        assert_eq!(typed(None, '"', false, true), None);
        assert_eq!(typed(Some('-'), '-', true, false), None);
        assert_eq!(typed(None, 'a', true, true), None, "letters are never changed");
    }
}
