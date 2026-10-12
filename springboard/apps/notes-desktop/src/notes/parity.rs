//! Text parity for the note editor, built on the shared models: find with whole-word matching (`textfind`), spelling
//! merged with the data detectors (`spellcheck` and `detect`), and the language of a note (`language`).
//! Pure functions over the note text. Every range is in chars, the unit the text area uses.
use bifrost_app::controls::{detect, language::{self, Language}, textfind};
use bifrost_app::spellcheck::{CheckingTypes, SpellChecker, WordList};
use std::ops::Range;

/// Word lists shipped with the keyboard app: one word per line, the count after it.
const EN_WORDS: &str = include_str!("../../../keyboard/src/words/en.txt");
const IS_WORDS: &str = include_str!("../../../keyboard/src/words/is.txt");

/// A byte range of `text` as a char range.
fn to_chars(text: &str, r: Range<usize>) -> Range<usize> {
    let start = text[..r.start].chars().count();
    start..start + text[r].chars().count()
}

/// The name of a language, as the note's date line shows it.
pub fn language_name(l: Language) -> &'static str {
    match l {
        Language::Arabic => "Arabic",
        Language::Chinese => "Chinese",
        Language::Danish => "Danish",
        Language::Dutch => "Dutch",
        Language::English => "English",
        Language::French => "French",
        Language::German => "German",
        Language::Greek => "Greek",
        Language::Hebrew => "Hebrew",
        Language::Icelandic => "Icelandic",
        Language::Italian => "Italian",
        Language::Japanese => "Japanese",
        Language::Korean => "Korean",
        Language::Norwegian => "Norwegian",
        Language::Polish => "Polish",
        Language::Portuguese => "Portuguese",
        Language::Russian => "Russian",
        Language::Spanish => "Spanish",
        Language::Swedish => "Swedish",
        Language::Thai => "Thai",
    }
}

/// The smallest share the top guess needs before the note counts as written in that language.
const LANGUAGE_CONFIDENCE: f64 = 0.6;

/// The language the note is written in, or None when its text has no letters to judge by or the guess is unsure
/// (a short note with a link and a date gives the recognizer little to go on). Spelling then falls back to English,
/// as the reference checker falls back to the user's language.
pub fn note_language(text: &str) -> Option<Language> {
    let mut r = language::LanguageRecognizer::new();
    r.process(text);
    r.hypotheses(1).into_iter().next().filter(|(_, p)| *p >= LANGUAGE_CONFIDENCE).map(|(l, _)| l)
}

/// The note's language for the date line, e.g. "Icelandic".
pub fn language_label(text: &str) -> Option<&'static str> {
    note_language(text).map(language_name)
}

/// Every match of `query` in `text` (case-insensitive; `whole_word` skips matches inside a longer word), as char
/// ranges in order. The query is trimmed, and an empty query matches nothing.
pub fn find_ranges(text: &str, query: &str, whole_word: bool) -> Vec<Range<usize>> {
    let opts = textfind::Options { ignore_case: true, whole_word };
    textfind::find_all(text, query.trim(), opts).into_iter().map(|r| to_chars(text, r)).collect()
}

/// A spelling checker for a word list of `lang`.
fn checker(words: &str, lang: &str) -> SpellChecker {
    let list = words.lines().filter_map(|line| line.split_whitespace().next());
    SpellChecker::new(Box::new(WordList::new(lang, list)))
}

/// The ranges Check Spelling and Links flags, in order: the misspelled words of the note, plus its links, dates and
/// phone numbers. A misspelling inside a detected item (the "www" of a link) is dropped, so each piece of text is
/// flagged once. Spelling uses the word list of the note's language: English and Icelandic have one; a note in any
/// other language gets only the detectors. A note with no letters is checked as English.
pub fn check_ranges(text: &str) -> Vec<Range<usize>> {
    let spelling = match note_language(text) {
        Some(Language::Icelandic) => Some(checker(IS_WORDS, "is")),
        Some(Language::English) | None => Some(checker(EN_WORDS, "en")),
        Some(_) => None,
    };
    let kinds = [detect::Kind::Link, detect::Kind::Date, detect::Kind::Phone];
    let detected: Vec<Range<usize>> = detect::find(text, &kinds).into_iter().map(|m| to_chars(text, m.range)).collect();
    let mut out: Vec<Range<usize>> = match spelling {
        Some(sc) => sc
            .check(text, CheckingTypes::SPELLING)
            .into_iter()
            .map(|r| to_chars(text, r.range))
            .filter(|s| !detected.iter().any(|d| s.start < d.end && d.start < s.end))
            .collect(),
        None => Vec::new(),
    };
    out.extend(detected);
    out.sort_by_key(|r| r.start);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slice(text: &str, r: &Range<usize>) -> String {
        text.chars().skip(r.start).take(r.end - r.start).collect()
    }

    #[test]
    fn find_reports_char_ranges_and_respects_whole_word() {
        let text = "Árni árni ÁRNI árnir";
        let all: Vec<String> = find_ranges(text, "árni", false).iter().map(|r| slice(text, r)).collect();
        assert_eq!(all, ["Árni", "árni", "ÁRNI", "árni"]);
        assert_eq!(find_ranges(text, "árni", false)[1], 5..9);
        let whole: Vec<String> = find_ranges(text, "árni", true).iter().map(|r| slice(text, r)).collect();
        assert_eq!(whole, ["Árni", "árni", "ÁRNI"]);
    }

    #[test]
    fn empty_or_blank_query_matches_nothing() {
        assert!(find_ranges("anything", "  ", false).is_empty());
        assert!(find_ranges("anything", "", true).is_empty());
    }

    #[test]
    fn check_flags_misspellings_and_keeps_detected_items_whole() {
        let text = "hello wrold see https://example.com on 2026-10-09";
        let found: Vec<String> = check_ranges(text).iter().map(|r| slice(text, r)).collect();
        assert!(found.iter().any(|s| s == "wrold"), "misspelling flagged: {found:?}");
        assert!(found.iter().any(|s| s == "https://example.com"), "link flagged: {found:?}");
        assert!(found.iter().any(|s| s == "2026-10-09"), "date flagged: {found:?}");
        // "example" and "com" sit inside the link and must not be flagged on their own.
        assert!(!found.iter().any(|s| s == "example" || s == "com"), "{found:?}");
    }

    #[test]
    fn check_results_come_in_order_without_overlap() {
        let text = "wrold hello wrold https://example.com wrold";
        let r = check_ranges(text);
        assert!(r.windows(2).all(|w| w[0].end <= w[1].start), "{r:?}");
    }

    #[test]
    fn language_is_named_and_empty_text_has_none() {
        assert_eq!(language_name(Language::Icelandic), "Icelandic");
        assert_eq!(language_label(""), None);
        assert_eq!(language_label("123 456 !!!"), None);
    }

    #[test]
    fn english_and_icelandic_notes_are_recognised() {
        assert_eq!(language_label("The quick brown fox jumps over the lazy dog and the cat is here with us today."), Some("English"));
        assert_eq!(language_label("Þetta er texti og það er gott að hafa hann með sér í dag."), Some("Icelandic"));
    }
}
