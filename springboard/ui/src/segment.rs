//! Text segmentation for `NativeTokenizer`: `Unit` (`NativeTokenUnit`: word, sentence, paragraph, document) and `tokens`,
//! which returns the byte range of each token. Pure data, no I/O.
//!
//! * Word: runs of letters and digits, with an apostrophe kept between letters ("don't"). Same rule as
//!   `springboard/sdk/src/spellcheck.rs` `words`.
//! * Sentence: ends after `.`, `!`, `?` or `…` when whitespace or the end of the text follows, and at every line break.
//!   Runs such as `?!` stay together. Abbreviations ("e.g. the") end a sentence early: there is no abbreviation list.
//! * Paragraph: one per non-empty line, without the surrounding spaces.
//! * Document: the whole text, when it is not empty.
//!
//! Ranges are UTF-8 byte offsets, so they are always char boundaries. There is no locale data and no UAX #29
//! break table: the rules above are the only ones.

use std::ops::Range;

/// `NativeTokenUnit`: the size of a token.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    Word,
    Sentence,
    Paragraph,
    Document,
}

/// The byte ranges of the `unit` tokens of `text`, in order and without overlaps.
pub fn tokens(text: &str, unit: Unit) -> Vec<Range<usize>> {
    match unit {
        Unit::Word => words(text),
        Unit::Sentence => sentences(text),
        Unit::Paragraph => paragraphs(text),
        Unit::Document => {
            if text.is_empty() {
                Vec::new()
            } else {
                vec![0..text.len()]
            }
        }
    }
}

fn is_terminator(c: char) -> bool {
    matches!(c, '.' | '!' | '?' | '…')
}

fn words(text: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    for (i, &(p, c)) in chars.iter().enumerate() {
        let inner_apos =
            (c == '\'' || c == '’') && start.is_some() && chars.get(i + 1).is_some_and(|n| n.1.is_alphabetic());
        if c.is_alphanumeric() || inner_apos {
            start.get_or_insert(p);
        } else if let Some(s) = start.take() {
            out.push(s..p);
        }
    }
    if let Some(s) = start {
        out.push(s..text.len());
    }
    out
}

fn sentences(text: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    // First non-space byte of the sentence being built, and the byte after its last non-space character.
    let mut start: Option<usize> = None;
    let mut end = 0;
    let mut chars = text.char_indices().peekable();
    while let Some((p, c)) = chars.next() {
        if c == '\n' || c == '\r' {
            if let Some(s) = start.take() {
                out.push(s..end);
            }
            continue;
        }
        if start.is_none() {
            if c.is_whitespace() {
                continue;
            }
            start = Some(p);
        }
        end = p + c.len_utf8();
        let next_is_space = chars.peek().map_or(true, |&(_, n)| n.is_whitespace());
        if is_terminator(c) && next_is_space {
            if let Some(s) = start.take() {
                out.push(s..end);
            }
        }
    }
    if let Some(s) = start {
        out.push(s..end);
    }
    out
}

fn paragraphs(text: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut offset = 0;
    for line in text.split('\n') {
        let trimmed = line.trim_end_matches('\r');
        let body = trimmed.trim();
        if !body.is_empty() {
            let start = offset + (trimmed.len() - trimmed.trim_start().len());
            out.push(start..start + body.len());
        }
        offset += line.len() + 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts<'a>(text: &'a str, unit: Unit) -> Vec<&'a str> {
        tokens(text, unit).into_iter().map(|r| &text[r]).collect()
    }

    #[test]
    fn words_keep_inner_apostrophes_and_split_on_punctuation() {
        assert_eq!(texts("Don't stop, a-b!", Unit::Word), vec!["Don't", "stop", "a", "b"]);
        assert_eq!(tokens("Don't stop", Unit::Word), vec![0..5, 6..10]);
        assert!(tokens("  ... ", Unit::Word).is_empty());
    }

    #[test]
    fn sentences_end_at_a_terminator_followed_by_space() {
        assert_eq!(tokens("Hello world. Bye now", Unit::Sentence), vec![0..12, 13..20]);
        // A decimal point is followed by a digit, so it does not end the sentence.
        assert_eq!(texts("Pi is 3.14. Really?! Yes", Unit::Sentence), vec!["Pi is 3.14.", "Really?!", "Yes"]);
    }

    #[test]
    fn a_line_break_ends_a_sentence() {
        assert_eq!(tokens("one\ntwo", Unit::Sentence), vec![0..3, 4..7]);
        assert!(tokens("", Unit::Sentence).is_empty());
    }

    #[test]
    fn paragraphs_are_non_empty_trimmed_lines() {
        let text = "First line\n\n  Second  \nThird";
        assert_eq!(tokens(text, Unit::Paragraph), vec![0..10, 14..20, 23..28]);
        assert_eq!(texts(text, Unit::Paragraph), vec!["First line", "Second", "Third"]);
    }

    #[test]
    fn document_is_the_whole_text_or_nothing() {
        assert_eq!(tokens("abc", Unit::Document), vec![0..3]);
        assert!(tokens("", Unit::Document).is_empty());
    }

    #[test]
    fn ranges_are_char_boundaries_for_multibyte_text() {
        // "é" and "ð" take two bytes each in UTF-8.
        let text = "Þetta er já. Sjá nánar";
        for r in tokens(text, Unit::Sentence).into_iter().chain(tokens(text, Unit::Word)) {
            assert!(text.is_char_boundary(r.start) && text.is_char_boundary(r.end));
        }
        assert_eq!(texts(text, Unit::Sentence), vec!["Þetta er já.", "Sjá nánar"]);
    }
}
