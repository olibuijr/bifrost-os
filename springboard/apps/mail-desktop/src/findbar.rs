//! Find in the open message (Edit > Find in Message): a bar over the reader takes the text, every match in the drawn lines
//! is highlighted, and Return and Shift+Return step through them. This module holds the search text, the match positions
//! and the stepping; drawing is in [`crate::draw`].

/// One match: the line it is in and its byte range there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub line: usize,
    pub start: usize,
    pub end: usize,
}

/// The bar's state: the text typed and which match is the current one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FindBar {
    pub text: String,
    pub current: usize,
}

impl FindBar {
    pub fn type_text(&mut self, text: &str) {
        self.text.push_str(text);
        self.current = 0;
    }

    pub fn backspace(&mut self) {
        self.text.pop();
        self.current = 0;
    }

    /// Steps to the next match of `total`, wrapping at the end.
    pub fn next(&mut self, total: usize) {
        self.current = if total == 0 { 0 } else { (self.current + 1) % total };
    }

    /// Steps to the previous match of `total`, wrapping at the start.
    pub fn previous(&mut self, total: usize) {
        self.current = if total == 0 { 0 } else { (self.current + total - 1) % total };
    }
}

/// Lower case for matching: a character whose lower case is one character maps to it, any other stays as it is, so a
/// match never changes the length of the text it is found in.
fn fold(c: char) -> char {
    let mut lower = c.to_lowercase();
    match (lower.next(), lower.next()) {
        (Some(one), None) => one,
        _ => c,
    }
}

/// Every match of `needle` in the lines, without case, in reading order. Matches do not overlap. An empty needle has none.
pub fn find_all(lines: &[String], needle: &str) -> Vec<Span> {
    let wanted: Vec<char> = needle.chars().map(fold).collect();
    let mut found = Vec::new();
    if wanted.is_empty() {
        return found;
    }
    for (line_number, line) in lines.iter().enumerate() {
        let chars: Vec<(usize, char)> = line.char_indices().map(|(at, c)| (at, fold(c))).collect();
        let mut i = 0;
        while i + wanted.len() <= chars.len() {
            if chars[i..i + wanted.len()].iter().map(|(_, c)| *c).eq(wanted.iter().copied()) {
                let end = chars.get(i + wanted.len()).map_or(line.len(), |(at, _)| *at);
                found.push(Span { line: line_number, start: chars[i].0, end });
                i += wanted.len();
            } else {
                i += 1;
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &[&str]) -> Vec<String> {
        text.iter().map(|l| l.to_string()).collect()
    }

    #[test]
    fn matches_are_found_without_case_in_reading_order() {
        let found = find_all(&lines(&["The plan", "", "A PLAN and another plan"]), "plan");
        assert_eq!(found, vec![Span { line: 0, start: 4, end: 8 }, Span { line: 2, start: 2, end: 6 }, Span { line: 2, start: 19, end: 23 }]);
    }

    #[test]
    fn matches_do_not_overlap_and_an_empty_needle_has_none() {
        assert_eq!(find_all(&lines(&["aaaa"]), "aa").len(), 2);
        assert!(find_all(&lines(&["aaaa"]), "").is_empty());
        assert!(find_all(&lines(&["abc"]), "abcd").is_empty());
    }

    #[test]
    fn byte_ranges_stay_correct_around_multibyte_characters() {
        let text = lines(&["Ísland á morgun"]);
        let found = find_all(&text, "ÁM");
        assert!(found.is_empty(), "the letters are never next to each other");
        let found = find_all(&text, "á m");
        assert_eq!(found.len(), 1);
        assert_eq!(&text[0][found[0].start..found[0].end], "á m");
        let found = find_all(&text, "ÍSLAND");
        assert_eq!(&text[0][found[0].start..found[0].end], "Ísland");
    }

    #[test]
    fn stepping_wraps_in_both_directions() {
        let mut bar = FindBar::default();
        bar.type_text("pl");
        bar.type_text("an");
        assert_eq!(bar.text, "plan");
        bar.next(3);
        bar.next(3);
        bar.next(3);
        assert_eq!(bar.current, 0);
        bar.previous(3);
        assert_eq!(bar.current, 2);
        bar.previous(0);
        assert_eq!(bar.current, 0);
        bar.next(3);
        bar.backspace();
        assert_eq!((bar.text.as_str(), bar.current), ("pla", 0), "a change of text starts again at the first match");
    }
}
