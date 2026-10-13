//! Word wrapping for the episode notes and the transcript: lines that fit a width, by a measuring function.
//! Pure: the caller supplies how wide a text is.

/// Wraps `text` into lines no wider than `max` by `measure`. Line breaks in `text` are kept, blank lines stay as
/// empty lines, and a word wider than a line is cut where it overflows.
pub fn wrap(text: &str, max: f32, measure: &dyn Fn(&str) -> f32) -> Vec<String> {
    let mut out = Vec::new();
    for paragraph in text.split('\n') {
        let mut line = String::new();
        for word in paragraph.split_whitespace() {
            let candidate = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
            if measure(&candidate) <= max {
                line = candidate;
                continue;
            }
            if !line.is_empty() {
                out.push(std::mem::take(&mut line));
            }
            let mut rest = word;
            while measure(rest) > max {
                // the longest prefix that fits (at least one character, so the loop ends)
                let n = rest
                    .char_indices()
                    .map(|(i, c)| i + c.len_utf8())
                    .filter(|&n| measure(&rest[..n]) <= max)
                    .last()
                    .unwrap_or_else(|| rest.chars().next().map_or(rest.len(), char::len_utf8));
                out.push(rest[..n].to_string());
                rest = &rest[n..];
            }
            line = rest.to_string();
        }
        if !line.is_empty() || paragraph.trim().is_empty() {
            out.push(line);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ten units per character.
    fn ten(s: &str) -> f32 {
        s.chars().count() as f32 * 10.0
    }

    #[test]
    fn words_fill_lines_up_to_the_width() {
        assert_eq!(wrap("aa bb cc dd", 50.0, &ten), ["aa bb", "cc dd"]);
        assert_eq!(wrap("aa bb cc", 80.0, &ten), ["aa bb cc"]);
    }

    #[test]
    fn line_breaks_and_blank_lines_stay() {
        assert_eq!(wrap("one\n\ntwo", 100.0, &ten), ["one", "", "two"]);
    }

    #[test]
    fn a_long_word_is_cut() {
        assert_eq!(wrap("abcdefgh", 30.0, &ten), ["abc", "def", "gh"]);
        assert_eq!(wrap("x abcdefg", 30.0, &ten), ["x", "abc", "def", "g"]);
    }

    #[test]
    fn nothing_fits_still_ends() {
        assert_eq!(wrap("ab", 5.0, &ten), ["a", "b"]);
    }
}
