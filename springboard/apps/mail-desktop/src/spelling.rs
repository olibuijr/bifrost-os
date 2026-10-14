//! Spelling in the message being written (Edit > Check Spelling). The words are checked by the system's text service, which
//! answers with byte ranges of the text it was given; the compose sheet keeps them as ranges of characters, which is what the
//! drawing and the styled text use. This module reads the service's answers and converts the ranges.

use bifrost_core::json::Json;
use std::ops::Range;

/// The ranges in the answer to `check`: a list of `[start, end]` pairs of bytes. Pairs of another shape are skipped.
pub fn byte_ranges(reply: &Json) -> Vec<Range<usize>> {
    let Some(items) = reply.as_array() else { return Vec::new() };
    items
        .iter()
        .filter_map(|item| {
            let pair = item.as_array()?;
            let (start, end) = (pair.first()?.as_u64()? as usize, pair.get(1)?.as_u64()? as usize);
            (start < end).then_some(start..end)
        })
        .collect()
}

/// The same ranges as ranges of characters of `text`. A range that is outside the text or cuts a character is dropped.
pub fn char_ranges(text: &str, bytes: &[Range<usize>]) -> Vec<Range<usize>> {
    bytes
        .iter()
        .filter(|r| r.end <= text.len() && text.is_char_boundary(r.start) && text.is_char_boundary(r.end))
        .map(|r| text[..r.start].chars().count()..text[..r.end].chars().count())
        .collect()
}

/// The byte range of a range of characters of `text`, or None when it is past the end.
pub fn byte_range(text: &str, chars: &Range<usize>) -> Option<Range<usize>> {
    let at = |n: usize| text.char_indices().map(|(i, _)| i).chain(std::iter::once(text.len())).nth(n);
    Some(at(chars.start)?..at(chars.end)?)
}

/// The words in the answer to `guesses` (a list of strings), best first.
pub fn words(reply: &Json) -> Vec<String> {
    reply.as_array().unwrap_or(&[]).iter().filter_map(Json::as_str).map(str::to_string).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_core::json::parse;

    #[test]
    fn the_checker_answers_with_pairs_of_bytes() {
        let reply = parse("[[6,11],[20,23],[3],[9,9],\"x\",[4,2]]").unwrap();
        assert_eq!(byte_ranges(&reply), vec![6..11, 20..23], "pairs that are short, empty or backwards are skipped");
        assert!(byte_ranges(&Json::Null).is_empty(), "no answer is no misspellings");
        assert!(byte_ranges(&parse("{}").unwrap()).is_empty());
    }

    #[test]
    fn byte_ranges_become_character_ranges_and_back() {
        let text = "Ég sá wrold í dag";
        // "wrold" starts after "Ég sá " which is 8 bytes (É and á take two) and 6 characters
        let start = text.find("wrold").unwrap();
        let bytes = vec![start..start + 5];
        let chars = char_ranges(text, &bytes);
        assert_eq!(chars, vec![6..11]);
        assert_eq!(byte_range(text, &chars[0]), Some(bytes[0].clone()));
        assert_eq!(byte_range(text, &(0..text.chars().count())), Some(0..text.len()));
        assert_eq!(byte_range(text, &(100..101)), None);
    }

    #[test]
    fn a_range_that_cuts_a_character_or_leaves_the_text_is_dropped() {
        let text = "Ég";
        assert!(char_ranges(text, &[1..2]).is_empty(), "byte 1 is in the middle of É");
        assert!(char_ranges(text, &[0..10]).is_empty());
        assert_eq!(char_ranges(text, &[0..2, 2..3]), vec![0..1, 1..2]);
    }

    #[test]
    fn guesses_are_the_strings_of_the_answer() {
        assert_eq!(words(&parse("[\"world\",\"would\",3]").unwrap()), vec!["world", "would"]);
        assert!(words(&Json::Null).is_empty());
    }
}
