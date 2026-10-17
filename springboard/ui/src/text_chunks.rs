//! Text chunking: cuts a long document into passages of a bounded size for indexing, embedding and search.
//!
//! A chunk never starts or ends in the middle of a sentence unless a single sentence is longer than the limit; then it
//! is cut between words, and a single word longer than the limit is cut between characters. Sentences end at `.`,
//! `!`, `?` or `…` followed by white space (or by the end of the text), at the full-width marks used in Chinese and
//! Japanese (`。`, `！`, `？`), and at a blank line. A full stop inside a number (`3.14`) does not end a sentence.
//! Neighbouring sentences are packed into one chunk while they fit. With an overlap, each chunk starts with the last
//! whole sentences of the one before it, up to that many characters, so a passage that straddles a cut is whole in at
//! least one chunk.
//!
//! Sizes count characters, not bytes. Ranges are byte offsets into the source text on character boundaries.

use std::ops::Range;

/// How to cut a text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChunkOptions {
    /// the most characters in one chunk (at least 1)
    pub max_chars: usize,
    /// the most characters repeated from the end of one chunk at the start of the next (less than `max_chars`)
    pub overlap: usize,
}

impl Default for ChunkOptions {
    fn default() -> Self {
        ChunkOptions { max_chars: 800, overlap: 100 }
    }
}

/// Why a text could not be chunked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChunkError {
    /// `max_chars` is zero.
    ZeroSize,
    /// `overlap` is not smaller than `max_chars`, so a chunk could never move forward.
    OverlapTooLarge,
}

fn is_terminator(c: char) -> bool {
    matches!(c, '.' | '!' | '?' | '…' | '。' | '！' | '？')
}

/// Full-width marks end a sentence without a space after them.
fn is_wide_terminator(c: char) -> bool {
    matches!(c, '。' | '！' | '？')
}

fn is_closer(c: char) -> bool {
    matches!(c, ')' | ']' | '}' | '"' | '\'' | '”' | '’' | '」' | '』' | '）')
}

fn char_count(text: &str, r: &Range<usize>) -> usize {
    text[r.clone()].chars().count()
}

/// The sentences and paragraphs of `text`, in order, with no white space at their ends.
fn units(text: &str) -> Vec<Range<usize>> {
    let ch: Vec<(usize, char)> = text.char_indices().collect();
    let n = ch.len();
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    let mut i = 0;
    while i < n {
        let (p, c) = ch[i];
        if c.is_whitespace() {
            let mut j = i;
            let mut newlines = 0;
            while j < n && ch[j].1.is_whitespace() {
                if ch[j].1 == '\n' {
                    newlines += 1;
                }
                j += 1;
            }
            if newlines >= 2 {
                if let Some(s) = start.take() {
                    out.push(s..p);
                }
            }
            i = j;
            continue;
        }
        if start.is_none() {
            start = Some(p);
        }
        if is_terminator(c) {
            let mut j = i + 1;
            while j < n && (is_terminator(ch[j].1) || is_closer(ch[j].1)) {
                j += 1;
            }
            if j >= n || ch[j].1.is_whitespace() || is_wide_terminator(c) {
                let end = ch.get(j).map_or(text.len(), |x| x.0);
                if let Some(s) = start.take() {
                    out.push(s..end);
                }
                i = j;
                continue;
            }
        }
        i += 1;
    }
    if let Some(s) = start {
        out.push(s..text.trim_end().len());
    }
    out
}

/// Pieces of at most `max` characters: the unit itself when it fits, otherwise its words, with any word that is too
/// long cut between characters.
fn fragments(text: &str, unit: Range<usize>, max: usize, out: &mut Vec<Range<usize>>) {
    if char_count(text, &unit) <= max {
        out.push(unit);
        return;
    }
    let slice = &text[unit.clone()];
    let mut word_start: Option<usize> = None;
    let mut words: Vec<Range<usize>> = Vec::new();
    for (i, c) in slice.char_indices() {
        if c.is_whitespace() {
            if let Some(s) = word_start.take() {
                words.push(unit.start + s..unit.start + i);
            }
        } else if word_start.is_none() {
            word_start = Some(i);
        }
    }
    if let Some(s) = word_start {
        words.push(unit.start + s..unit.end);
    }
    for w in words {
        if char_count(text, &w) <= max {
            out.push(w);
            continue;
        }
        let word = &text[w.clone()];
        let mut from = 0;
        let mut count = 0;
        for (i, _) in word.char_indices() {
            if count == max {
                out.push(w.start + from..w.start + i);
                from = i;
                count = 0;
            }
            count += 1;
        }
        out.push(w.start + from..w.end);
    }
}

/// The byte ranges of the chunks of `text`. White space only at the ends of the text is left out; inside a chunk the
/// source text is kept exactly. An empty or blank text has no chunks.
pub fn chunk_ranges(text: &str, options: &ChunkOptions) -> Result<Vec<Range<usize>>, ChunkError> {
    if options.max_chars == 0 {
        return Err(ChunkError::ZeroSize);
    }
    if options.overlap >= options.max_chars {
        return Err(ChunkError::OverlapTooLarge);
    }
    let mut pieces = Vec::new();
    for u in units(text) {
        fragments(text, u, options.max_chars, &mut pieces);
    }
    let mut out = Vec::new();
    let mut i = 0;
    while i < pieces.len() {
        let mut j = i;
        while j + 1 < pieces.len() && char_count(text, &(pieces[i].start..pieces[j + 1].end)) <= options.max_chars {
            j += 1;
        }
        out.push(pieces[i].start..pieces[j].end);
        if j + 1 >= pieces.len() {
            break;
        }
        // the next chunk starts with as many of the last pieces as the overlap allows, but always moves forward
        let mut k = j + 1;
        while k - 1 > i && char_count(text, &(pieces[k - 1].start..pieces[j].end)) <= options.overlap {
            k -= 1;
        }
        i = k;
    }
    Ok(out)
}

/// The chunks of `text` as strings; see [`chunk_ranges`].
pub fn chunks<'a>(text: &'a str, options: &ChunkOptions) -> Result<Vec<&'a str>, ChunkError> {
    Ok(chunk_ranges(text, options)?.into_iter().map(|r| &text[r]).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(max_chars: usize, overlap: usize) -> ChunkOptions {
        ChunkOptions { max_chars, overlap }
    }

    #[test]
    fn empty_and_blank_text_have_no_chunks() {
        assert!(chunks("", &opts(10, 0)).unwrap().is_empty());
        assert!(chunks("  \n\n \t", &opts(10, 0)).unwrap().is_empty());
    }

    #[test]
    fn a_short_text_is_one_chunk_without_its_outer_white_space() {
        assert_eq!(chunks("  Hello there.  \n", &opts(100, 10)).unwrap(), vec!["Hello there."]);
    }

    #[test]
    fn sentences_are_packed_while_they_fit() {
        let got = chunks("One. Two. Three. Four.", &opts(10, 0)).unwrap();
        assert_eq!(got, vec!["One. Two.", "Three.", "Four."]);
    }

    #[test]
    fn the_overlap_repeats_whole_sentences_that_fit_in_it() {
        let got = chunks("One. Two. Three. Four.", &opts(12, 5)).unwrap();
        assert_eq!(got, vec!["One. Two.", "Two. Three.", "Four."]);
    }

    #[test]
    fn a_blank_line_ends_a_sentence_even_without_a_full_stop() {
        let text = "Title line\n\nBody line";
        assert_eq!(units(text).iter().map(|r| &text[r.clone()]).collect::<Vec<_>>(), vec!["Title line", "Body line"]);
        assert_eq!(chunks(text, &opts(12, 0)).unwrap(), vec!["Title line", "Body line"]);
    }

    #[test]
    fn a_full_stop_inside_a_number_or_before_a_letter_does_not_end_a_sentence() {
        let text = "Pi is 3.14 today. Next? Yes!";
        let got: Vec<&str> = units(text).iter().map(|r| &text[r.clone()]).collect();
        assert_eq!(got, vec!["Pi is 3.14 today.", "Next?", "Yes!"]);
    }

    #[test]
    fn closing_quotes_stay_with_their_sentence() {
        let text = "He said \"stop.\" Then left.";
        let got: Vec<&str> = units(text).iter().map(|r| &text[r.clone()]).collect();
        assert_eq!(got, vec!["He said \"stop.\"", "Then left."]);
    }

    #[test]
    fn a_sentence_longer_than_the_limit_is_cut_between_words() {
        assert_eq!(chunks("aaaa bbbb cccc dddd", &opts(9, 0)).unwrap(), vec!["aaaa bbbb", "cccc dddd"]);
    }

    #[test]
    fn a_word_longer_than_the_limit_is_cut_between_characters() {
        assert_eq!(chunks("abcdefghij", &opts(4, 0)).unwrap(), vec!["abcd", "efgh", "ij"]);
    }

    #[test]
    fn chinese_and_japanese_sentences_end_at_the_full_width_mark() {
        let got = chunks("今天天气很好。我们去公园。", &opts(8, 0)).unwrap();
        assert_eq!(got, vec!["今天天气很好。", "我们去公园。"]);
    }

    #[test]
    fn no_chunk_is_over_the_limit_and_none_of_the_text_is_lost() {
        let text = "Alpha beta gamma. Delta epsilon zeta eta theta iota kappa lambda mu nu xi omicron pi rho sigma.\n\n\
                    Supercalifragilisticexpialidocious is long! Short one. 今天天气很好。Done";
        for max in [5usize, 9, 17, 40] {
            let got = chunks(text, &opts(max, 0)).unwrap();
            assert!(got.iter().all(|c| c.chars().count() <= max), "max {max}: {got:?}");
            let joined: String = got.concat().chars().filter(|c| !c.is_whitespace()).collect();
            let wanted: String = text.chars().filter(|c| !c.is_whitespace()).collect();
            assert_eq!(joined, wanted, "max {max}");
        }
    }

    #[test]
    fn with_overlap_every_chunk_still_fits_and_the_text_moves_forward() {
        let text = "a b c d e f g h i j k l m n o p q r s t u v w x y z";
        let got = chunk_ranges(text, &opts(9, 4)).unwrap();
        assert!(got.len() > 1);
        assert!(got.iter().all(|r| char_count(text, r) <= 9));
        assert!(got.windows(2).all(|w| w[1].start > w[0].start && w[1].end > w[0].end));
        assert_eq!(got.last().unwrap().end, text.len());
    }

    #[test]
    fn bad_sizes_are_refused() {
        assert_eq!(chunks("x", &opts(0, 0)).unwrap_err(), ChunkError::ZeroSize);
        assert_eq!(chunks("x", &opts(5, 5)).unwrap_err(), ChunkError::OverlapTooLarge);
        assert!(chunks("x", &ChunkOptions::default()).is_ok());
    }
}
