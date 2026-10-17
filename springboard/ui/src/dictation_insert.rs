//! Dictation insertion: puts a recognized reading into a text buffer at the caret and keeps it in step as the reading
//! changes. Each reading replaces the one before it (partial results are revised), so the buffer always holds the
//! latest reading for this insertion. The reading is spaced and capitalized for the text around it (`format_reading`).
//!
//! Pure model over `text::TextBuffer::replace_range`: each change is an ordinary edit, so undo and the caret work as
//! they do for typing. The recognizer that produces readings is separate (`recognition_task`).

use crate::text::{Edit, TextBuffer};
use std::ops::Range;

/// How the reading reads when it follows `before` (the text before the insertion point): a space when the words would
/// otherwise run together, and a capital at the start of a sentence or line.
pub fn format_reading(before: &str, reading: &str) -> String {
    let reading = reading.trim_start();
    if reading.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    if let Some(last) = before.chars().last() {
        if !last.is_whitespace() && !"([{\"'/-".contains(last) {
            out.push(' ');
        }
    }
    let last_word_char = before.chars().rev().find(|c| !c.is_whitespace());
    let sentence_start = before.is_empty() || before.ends_with('\n') || last_word_char.map_or(true, |c| matches!(c, '.' | '!' | '?'));
    let mut chars = reading.chars();
    if let Some(first) = chars.next() {
        if sentence_start {
            out.extend(first.to_uppercase());
        } else {
            out.push(first);
        }
    }
    out.push_str(chars.as_str());
    if before.is_empty() || before.ends_with('\n') {
        out = out.trim_start().to_string();
    }
    out
}

/// One dictation insertion: the byte range of the text it currently holds in the buffer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Insertion {
    range: Range<usize>,
}

impl Insertion {
    /// Starts an insertion at byte `at` (the caret).
    pub fn at(at: usize) -> Self {
        Insertion { range: at..at }
    }

    /// The bytes this insertion holds in the buffer now.
    pub fn range(&self) -> Range<usize> {
        self.range.clone()
    }

    /// Shows `reading` in place of the text the previous call put in. Returns the edit, or `None` when nothing changed.
    /// The text before the insertion point must not change between calls (the spacing and capital depend on it).
    pub fn show(&mut self, buf: &mut TextBuffer, reading: &str) -> Option<Edit> {
        let start = self.range.start;
        let text = format_reading(buf.text().get(..start).unwrap_or(""), reading);
        let edit = buf.replace_range(self.range.clone(), &text);
        self.range = start..start + text.len();
        edit
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reading_at_the_start_of_the_text_is_capitalized() {
        assert_eq!(format_reading("", "hello world"), "Hello world");
        assert_eq!(format_reading("", "   "), "");
        assert_eq!(format_reading("", ""), "");
    }

    #[test]
    fn a_reading_after_words_gets_a_space_and_no_capital() {
        assert_eq!(format_reading("Good morning", "and welcome"), " and welcome");
        assert_eq!(format_reading("the list,", "milk"), " milk");
    }

    #[test]
    fn a_reading_after_a_sentence_end_is_capitalized() {
        assert_eq!(format_reading("Done.", "next line"), " Next line");
        assert_eq!(format_reading("Done. ", "next line"), "Next line", "the space is already there");
        assert_eq!(format_reading("Really?", "yes"), " Yes");
    }

    #[test]
    fn a_reading_after_a_line_break_or_an_opening_bracket_is_placed_without_a_space() {
        assert_eq!(format_reading("Line one\n", "two"), "Two");
        assert_eq!(format_reading("see (", "note"), "note");
        assert_eq!(format_reading("a ", "b"), "b");
    }

    #[test]
    fn partial_readings_replace_the_previous_one_in_place() {
        let mut buf = TextBuffer::from_text("Hi. ");
        let mut ins = Insertion::at(buf.text().len());
        assert!(ins.show(&mut buf, "hel").is_some());
        assert_eq!(buf.text(), "Hi. Hel");
        assert_eq!(ins.range(), 4..7);
        ins.show(&mut buf, "hello there");
        assert_eq!(buf.text(), "Hi. Hello there");
        assert_eq!(ins.range(), 4..15);
        ins.show(&mut buf, "");
        assert_eq!(buf.text(), "Hi. ");
        assert_eq!(ins.range(), 4..4);
    }

    #[test]
    fn an_insertion_in_the_middle_keeps_the_text_after_the_caret() {
        let mut buf = TextBuffer::from_text("ab");
        let mut ins = Insertion::at(1);
        ins.show(&mut buf, "xy");
        assert_eq!(buf.text(), "a xyb");
        assert_eq!(ins.range(), 1..4);
    }
}
