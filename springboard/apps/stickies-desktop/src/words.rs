//! Whole-word matching for Find and Replace. A match counts only when no letter, digit or underscore touches either of
//! its ends, so "cat" does not match inside "category".

/// Whether the text `start..end` of `text` stands as a whole word. The offsets are byte offsets on character boundaries.
pub fn is_whole_word(text: &str, start: usize, end: usize) -> bool {
    let before = text[..start].chars().next_back();
    let after = text[end..].chars().next();
    !before.is_some_and(is_word_char) && !after.is_some_and(is_word_char)
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_match_inside_a_longer_word_is_not_a_whole_word() {
        let text = "category cat";
        assert!(!is_whole_word(text, 0, 3), "cat at the start of category");
        assert!(is_whole_word(text, 9, 12), "cat on its own");
    }

    #[test]
    fn digits_and_underscores_join_a_word() {
        assert!(!is_whole_word("x1 y", 1, 2), "a digit before the match");
        assert!(!is_whole_word("a_b", 0, 1), "an underscore after the match");
        assert!(is_whole_word("a-b", 0, 1), "a hyphen separates words");
    }

    #[test]
    fn the_ends_of_the_text_are_word_boundaries() {
        assert!(is_whole_word("foo", 0, 3));
        assert!(is_whole_word("", 0, 0), "an empty match at the end of nothing");
    }

    #[test]
    fn accented_letters_count_as_letters() {
        let text = "éfoo foo";
        let start = "éfoo ".len();
        assert!(!is_whole_word(text, 2, 5), "foo after an accented letter");
        assert!(is_whole_word(text, start, start + 3));
    }
}
