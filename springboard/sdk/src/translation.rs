//! `TranslationSession` parity: a session between one installed source and target language that translates single
//! strings or batches through an engine the app supplies (`Translator`), and can be cancelled. BifrOSt has no
//! language packs or system translation popover; `TranslationMemory` is the built-in engine, a table of strings
//! translated ahead of time.

use crate::attributed::{AttributedString, Attrs};
use crate::error::ErrorCode;
use bifrost_ui::translation::{language_code, LanguageAvailability};
pub use bifrost_ui::translation::TranslationError;
use std::cell::Cell;
use std::collections::HashMap;
use std::path::PathBuf;

/// The domain of [`TranslationError`] in an [`AppError`](crate::error::AppError).
pub const TRANSLATION_DOMAIN: &str = "bifrost.translation";

/// Codes 1 to 8 in declaration order: the general cases, then the unsupported-language cases.
impl ErrorCode for TranslationError {
    const DOMAIN: &'static str = TRANSLATION_DOMAIN;

    fn code(&self) -> i64 {
        match self {
            TranslationError::NothingToTranslate => 1,
            TranslationError::UnableToIdentifyLanguage => 2,
            TranslationError::NotInstalled => 3,
            TranslationError::UnsupportedSourceLanguage => 4,
            TranslationError::UnsupportedTargetLanguage => 5,
            TranslationError::UnsupportedLanguagePairing => 6,
            TranslationError::InternalError => 7,
            TranslationError::AlreadyCancelled => 8,
        }
    }

    fn name(&self) -> &'static str {
        match self {
            TranslationError::NothingToTranslate => "NothingToTranslate",
            TranslationError::UnableToIdentifyLanguage => "UnableToIdentifyLanguage",
            TranslationError::NotInstalled => "NotInstalled",
            TranslationError::UnsupportedSourceLanguage => "UnsupportedSourceLanguage",
            TranslationError::UnsupportedTargetLanguage => "UnsupportedTargetLanguage",
            TranslationError::UnsupportedLanguagePairing => "UnsupportedLanguagePairing",
            TranslationError::InternalError => "InternalError",
            TranslationError::AlreadyCancelled => "AlreadyCancelled",
        }
    }

    fn description(&self) -> &'static str {
        self.error_description()
    }
}

/// The engine a session calls. It receives normalised language codes and a non-empty text.
pub trait Translator {
    fn translate(&self, source: &str, target: &str, text: &str) -> Result<String, TranslationError>;
}

/// `TranslationSession`: one language pair and one engine. Not shared across threads.
pub struct TranslationSession {
    source: String,
    target: String,
    engine: Box<dyn Translator>,
    cancelled: Cell<bool>,
}

impl TranslationSession {
    /// `init(installedSource:target:)`: the pair must be installed in `availability`, otherwise the error the
    /// availability check reports is returned.
    pub fn new(
        availability: &LanguageAvailability,
        source: &str,
        target: &str,
        engine: Box<dyn Translator>,
    ) -> Result<Self, TranslationError> {
        availability.check_session(source, target)?;
        Ok(Self {
            source: language_code(source),
            target: language_code(target),
            engine,
            cancelled: Cell::new(false),
        })
    }

    /// `sourceLanguage`: the language code the session translates from.
    pub fn source_language(&self) -> &str {
        &self.source
    }

    /// `targetLanguage`: the language code the session translates into.
    pub fn target_language(&self) -> &str {
        &self.target
    }

    /// `translate(_:)`: one string. Empty or whitespace-only text fails before the engine runs.
    pub fn translate(&self, text: &str) -> Result<String, TranslationError> {
        if self.cancelled.get() {
            return Err(TranslationError::AlreadyCancelled);
        }
        if text.trim().is_empty() {
            return Err(TranslationError::NothingToTranslate);
        }
        self.engine.translate(&self.source, &self.target, text)
    }

    /// `translations(from:)`: several strings. The results come back in input order, one per string.
    pub fn translate_batch<S: AsRef<str>>(&self, texts: &[S]) -> Vec<Result<String, TranslationError>> {
        texts.iter().map(|t| self.translate(t.as_ref())).collect()
    }

    /// `translate(_:)` for a formatted string (`AttributedString` input): the text goes to the engine run by run, and
    /// each translated piece keeps the attributes of its run. Blank text fails before the engine runs, as for `translate`.
    pub fn translate_attributed(&self, text: &AttributedString) -> Result<AttributedString, TranslationError> {
        if self.cancelled.get() {
            return Err(TranslationError::AlreadyCancelled);
        }
        if text.string().trim().is_empty() {
            return Err(TranslationError::NothingToTranslate);
        }
        let runs: Vec<(&str, Attrs)> = text.runs().map(|(s, a)| (s, a.clone())).collect();
        let engine = |t: &str| self.engine.translate(&self.source, &self.target, t);
        let pieces = bifrost_ui::translation::translate_runs(&runs, &engine)?;
        let mut out = AttributedString::new();
        for (s, a) in pieces {
            out.append(&s, a);
        }
        Ok(out)
    }

    /// `cancel()`: every later call fails with `AlreadyCancelled`.
    pub fn cancel(&self) {
        self.cancelled.set(true);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.get()
    }
}

/// A phrase table: exact matches for strings translated ahead of time, keyed by language pair. Lookup uses the
/// text byte for byte. A text without an entry fails with `InternalError`, because this engine cannot translate it.
#[derive(Default)]
pub struct TranslationMemory {
    phrases: HashMap<(String, String, String), String>,
}

impl TranslationMemory {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds `translation` for `text` from `source` to `target`. Language identifiers are reduced to codes.
    pub fn insert(&mut self, source: &str, target: &str, text: &str, translation: &str) {
        let key = (language_code(source), language_code(target), text.to_string());
        self.phrases.insert(key, translation.to_string());
    }

    pub fn len(&self) -> usize {
        self.phrases.len()
    }

    pub fn is_empty(&self) -> bool {
        self.phrases.is_empty()
    }
}

impl Translator for TranslationMemory {
    fn translate(&self, source: &str, target: &str, text: &str) -> Result<String, TranslationError> {
        let key = (source.to_string(), target.to_string(), text.to_string());
        self.phrases.get(&key).cloned().ok_or(TranslationError::InternalError)
    }
}

/// File extension of an on-device language pack. A pack holds one pair, named `<source>-<target>.tsv`, with one
/// `text<TAB>translation` line per phrase. Lines that are blank or start with `#` are ignored.
pub const PACK_EXTENSION: &str = "tsv";

/// Why a language pack could not be read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PackError {
    /// The 1-based line that is not `text<TAB>translation`, or has an empty side.
    Malformed(usize),
}

/// Parses the phrases of one pack for `source` to `target` into a phrase table.
pub fn parse_pack(source: &str, target: &str, text: &str) -> Result<TranslationMemory, PackError> {
    let mut memory = TranslationMemory::new();
    for (index, line) in text.lines().enumerate() {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let malformed = PackError::Malformed(index + 1);
        let (from, to) = line.split_once('\t').ok_or(malformed)?;
        if from.is_empty() || to.is_empty() {
            return Err(malformed);
        }
        memory.insert(source, target, from, to);
    }
    Ok(memory)
}

/// The on-device language packs in one folder. Nothing is downloaded: a pair is installed when its pack file is
/// present, and the phrases are read from that file. This is the engine BifrOSt ships offline. A neural engine
/// (Argos or bergamot style) is not in the Arch repositories, so it is not wired.
#[derive(Clone, Debug)]
pub struct LanguagePack {
    root: PathBuf,
}

impl LanguagePack {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The pack file for a pair. Locale identifiers are reduced to language codes first.
    pub fn file_for(&self, source: &str, target: &str) -> PathBuf {
        let name = format!("{}-{}.{}", language_code(source), language_code(target), PACK_EXTENSION);
        self.root.join(name)
    }

    /// The availability for `supported` languages: every ordered pair whose pack file exists is installed.
    pub fn availability<S: AsRef<str>>(&self, supported: &[S]) -> LanguageAvailability {
        let mut availability = LanguageAvailability::new(supported);
        let codes = availability.supported_languages().to_vec();
        for source in &codes {
            for target in &codes {
                if source != target && self.file_for(source, target).is_file() {
                    availability.install(source, target);
                }
            }
        }
        availability
    }

    /// The engine for an installed pair. A missing pack is `NotInstalled`; an unreadable or malformed one is
    /// `InternalError`, because the phrases cannot be trusted.
    pub fn engine(&self, source: &str, target: &str) -> Result<TranslationMemory, TranslationError> {
        let path = self.file_for(source, target);
        if !path.is_file() {
            return Err(TranslationError::NotInstalled);
        }
        let text = std::fs::read_to_string(&path).map_err(|_| TranslationError::InternalError)?;
        parse_pack(&language_code(source), &language_code(target), &text).map_err(|_| TranslationError::InternalError)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_ui::translation::AvailabilityStatus;

    fn installed() -> LanguageAvailability {
        let mut a = LanguageAvailability::new(&["en", "is"]);
        a.install("en", "is");
        a
    }

    fn memory() -> Box<TranslationMemory> {
        let mut m = TranslationMemory::new();
        m.insert("en_US", "is_IS", "Save", "Vista");
        m.insert("en", "is", "Open", "Opna");
        Box::new(m)
    }

    #[test]
    fn a_session_needs_an_installed_pair_and_normalises_its_languages() {
        let none = LanguageAvailability::new(&["en", "is"]);
        assert_eq!(TranslationSession::new(&none, "en", "is", memory()).err(), Some(TranslationError::NotInstalled));

        let s = TranslationSession::new(&installed(), "en_US.UTF-8", "is_IS", memory()).ok().unwrap();
        assert_eq!(s.source_language(), "en");
        assert_eq!(s.target_language(), "is");
        assert!(!s.is_cancelled());
    }

    #[test]
    fn translate_returns_the_engine_result_for_the_session_pair() {
        let s = TranslationSession::new(&installed(), "en", "is", memory()).ok().unwrap();
        assert_eq!(s.translate("Save"), Ok("Vista".to_string()));
        // The memory is directional: the reverse pair has no entry for the same text.
        let mut r = LanguageAvailability::new(&["en", "is"]);
        r.install("is", "en");
        let s = TranslationSession::new(&r, "is", "en", memory()).ok().unwrap();
        assert_eq!(s.translate("Save"), Err(TranslationError::InternalError));
    }

    #[test]
    fn empty_text_fails_before_the_engine_runs() {
        let s = TranslationSession::new(&installed(), "en", "is", memory()).ok().unwrap();
        assert_eq!(s.translate(""), Err(TranslationError::NothingToTranslate));
        assert_eq!(s.translate(" \t\n"), Err(TranslationError::NothingToTranslate));
    }

    #[test]
    fn batch_returns_one_result_per_input_in_order() {
        let s = TranslationSession::new(&installed(), "en", "is", memory()).ok().unwrap();
        let out = s.translate_batch(&["Open", "Unknown", "  ", "Open"]);
        assert_eq!(
            out,
            vec![
                Ok("Opna".to_string()),
                Err(TranslationError::InternalError),
                Err(TranslationError::NothingToTranslate),
                Ok("Opna".to_string()),
            ]
        );
    }

    struct Upper;

    impl Translator for Upper {
        fn translate(&self, _source: &str, _target: &str, text: &str) -> Result<String, TranslationError> {
            Ok(text.to_uppercase())
        }
    }

    #[test]
    fn formatted_text_keeps_the_attributes_of_each_run() {
        let s = TranslationSession::new(&installed(), "en", "is", Box::new(Upper)).ok().unwrap();
        let mut text = AttributedString::with_attrs("Open", Attrs { bold: true, ..Default::default() });
        text.append(" file", Attrs::default());
        let out = s.translate_attributed(&text).unwrap();
        assert_eq!(out.string(), "OPEN FILE");
        assert!(out.attributes_at(0).bold);
        assert!(!out.attributes_at(4).bold);
    }

    #[test]
    fn blank_formatted_text_is_nothing_to_translate() {
        let s = TranslationSession::new(&installed(), "en", "is", Box::new(Upper)).ok().unwrap();
        let text = AttributedString::with_attrs("   ", Attrs::default());
        assert_eq!(s.translate_attributed(&text).err(), Some(TranslationError::NothingToTranslate));
    }

    #[test]
    fn cancel_makes_every_later_call_fail_with_already_cancelled() {
        let s = TranslationSession::new(&installed(), "en", "is", memory()).ok().unwrap();
        assert_eq!(s.translate("Open"), Ok("Opna".to_string()));
        s.cancel();
        assert!(s.is_cancelled());
        assert_eq!(s.translate("Open"), Err(TranslationError::AlreadyCancelled));
        let expected: Vec<Result<String, TranslationError>> = vec![Err(TranslationError::AlreadyCancelled)];
        assert_eq!(s.translate_batch(&["Open"]), expected);
    }

    #[test]
    fn memory_reports_its_size_and_replaces_repeated_entries() {
        let mut m = TranslationMemory::new();
        assert!(m.is_empty());
        m.insert("en", "is", "Save", "Vista");
        m.insert("en_GB", "is", "Save", "Vista!");
        assert_eq!(m.len(), 1);
        assert_eq!(m.translate("en", "is", "Save"), Ok("Vista!".to_string()));
    }

    #[test]
    fn a_pack_line_is_text_then_tab_then_translation() {
        let text = "# phrases for en to is\nSave\tVista\n\nOpen\tOpna\n";
        let m = parse_pack("en_US", "is", text).ok().unwrap();
        assert_eq!(m.len(), 2);
        assert_eq!(m.translate("en", "is", "Open"), Ok("Opna".to_string()));
    }

    #[test]
    fn a_malformed_pack_line_reports_its_line_number() {
        assert_eq!(parse_pack("en", "is", "Save\tVista\nno tab here\n").err(), Some(PackError::Malformed(2)));
        assert_eq!(parse_pack("en", "is", "\tVista\n").err(), Some(PackError::Malformed(1)));
    }

    #[test]
    fn a_pack_file_installs_only_its_own_direction() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("en-is.tsv"), "Save\tVista\n").unwrap();
        let packs = LanguagePack::new(dir.path());
        assert_eq!(packs.file_for("en_US.UTF-8", "is_IS"), dir.path().join("en-is.tsv"));

        let a = packs.availability(&["en", "is"]);
        assert!(matches!(a.status("en", "is"), AvailabilityStatus::Installed));
        assert!(matches!(a.status("is", "en"), AvailabilityStatus::Supported));

        let s = TranslationSession::new(&a, "en", "is", Box::new(packs.engine("en", "is").ok().unwrap())).ok().unwrap();
        assert_eq!(s.translate("Save"), Ok("Vista".to_string()));
    }

    #[test]
    fn a_missing_pack_is_not_installed_and_a_broken_one_is_an_internal_error() {
        let dir = tempfile::tempdir().unwrap();
        let packs = LanguagePack::new(dir.path());
        assert_eq!(packs.engine("en", "is").err(), Some(TranslationError::NotInstalled));
        std::fs::write(dir.path().join("en-is.tsv"), "broken line\n").unwrap();
        assert_eq!(packs.engine("en", "is").err(), Some(TranslationError::InternalError));
    }
}

/// One pair's phrases keyed by their lower-case words, and how many words the longest has.
#[derive(Default)]
struct Glossary {
    phrases: HashMap<String, String>,
    max_words: usize,
}

/// An engine that translates running text from a language pack's phrases, with no network and no model: the text is
/// cut into words and separators, each stretch of words is matched against the longest phrase of the pack that fits
/// (case and punctuation do not matter), the match is replaced and the separators stay where they were. Capital
/// letters carry over (`Hello` gives `Halló`). Words the pack does not know are left as written, so a text the pack
/// barely covers would mislead: when fewer than half the words matched the translation fails with `InternalError`,
/// like a phrase table does for a text it cannot translate.
#[derive(Default)]
pub struct GlossaryTranslator {
    pairs: HashMap<(String, String), Glossary>,
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '\'' || c == '\u{2019}'
}

/// The text as (is a word, slice) pieces in order.
fn pieces(text: &str) -> Vec<(bool, &str)> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut word: Option<bool> = None;
    for (i, c) in text.char_indices() {
        let w = is_word_char(c);
        let prev = word;
        match prev {
            Some(p) if p == w => {}
            Some(p) => {
                out.push((p, &text[start..i]));
                start = i;
                word = Some(w);
            }
            None => word = Some(w),
        }
    }
    if let Some(p) = word {
        out.push((p, &text[start..]));
    }
    out
}

fn phrase_key(text: &str) -> (String, usize) {
    let words: Vec<String> = pieces(text).into_iter().filter(|(w, _)| *w).map(|(_, s)| s.to_lowercase()).collect();
    let n = words.len();
    (words.join(" "), n)
}

/// Gives `translation` the capitals of `source`: all capitals stay all capitals, an initial capital stays one.
fn recase(source: &str, translation: &str) -> String {
    let letters: Vec<char> = source.chars().filter(|c| c.is_alphabetic()).collect();
    if letters.len() > 1 && letters.iter().all(|c| c.is_uppercase()) {
        return translation.to_uppercase();
    }
    if source.chars().next().map_or(false, |c| c.is_uppercase()) {
        let mut chars = translation.chars();
        if let Some(first) = chars.next() {
            return first.to_uppercase().chain(chars).collect();
        }
    }
    translation.to_string()
}

impl GlossaryTranslator {
    /// An engine over the phrases of `memory` (every pair in it).
    pub fn new(memory: &TranslationMemory) -> Self {
        let mut engine = GlossaryTranslator::default();
        for ((source, target, text), translation) in &memory.phrases {
            let (key, words) = phrase_key(text);
            if words == 0 {
                continue;
            }
            let glossary = engine.pairs.entry((source.clone(), target.clone())).or_default();
            glossary.max_words = glossary.max_words.max(words);
            glossary.phrases.insert(key, translation.clone());
        }
        engine
    }

    /// How many language pairs have phrases.
    pub fn pair_count(&self) -> usize {
        self.pairs.len()
    }
}

impl Translator for GlossaryTranslator {
    fn translate(&self, source: &str, target: &str, text: &str) -> Result<String, TranslationError> {
        let glossary = self.pairs.get(&(source.to_string(), target.to_string())).ok_or(TranslationError::InternalError)?;
        let parts = pieces(text);
        let word_at: Vec<usize> = parts.iter().enumerate().filter(|(_, (w, _))| *w).map(|(i, _)| i).collect();
        let mut out = String::with_capacity(text.len());
        let (mut matched, mut next_part, mut w) = (0usize, 0usize, 0usize);
        while w < word_at.len() {
            // the longest run of words from w that is a phrase; only blanks may sit between its words
            let mut hit = None;
            let longest = glossary.max_words.min(word_at.len() - w);
            for n in (1..=longest).rev() {
                let first = word_at[w];
                let last = word_at[w + n - 1];
                let blanks_only = parts[first..=last].iter().all(|(is_word, s)| *is_word || s.chars().all(char::is_whitespace));
                if !blanks_only {
                    continue;
                }
                let words: Vec<String> = word_at[w..w + n].iter().map(|i| parts[*i].1.to_lowercase()).collect();
                if let Some(translation) = glossary.phrases.get(&words.join(" ")) {
                    hit = Some((n, last, translation));
                    break;
                }
            }
            let first = word_at[w];
            for (_, s) in &parts[next_part..first] {
                out.push_str(s);
            }
            match hit {
                Some((n, last, translation)) => {
                    let original: String = parts[first..=last].iter().map(|(_, s)| *s).collect();
                    out.push_str(&recase(&original, translation));
                    matched += n;
                    next_part = last + 1;
                    w += n;
                }
                None => {
                    out.push_str(parts[first].1);
                    next_part = first + 1;
                    w += 1;
                }
            }
        }
        for (_, s) in &parts[next_part..] {
            out.push_str(s);
        }
        if word_at.is_empty() || matched * 2 < word_at.len() {
            return Err(TranslationError::InternalError);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod glossary_tests {
    use super::*;

    fn engine() -> GlossaryTranslator {
        let mut m = TranslationMemory::new();
        m.insert("en", "is", "good morning", "góðan daginn");
        m.insert("en", "is", "hello", "halló");
        m.insert("en", "is", "world", "heimur");
        m.insert("is", "en", "heimur", "world");
        GlossaryTranslator::new(&m)
    }

    #[test]
    fn words_are_replaced_and_punctuation_and_spacing_stay() {
        let e = engine();
        assert_eq!(e.translate("en", "is", "Hello, world!").unwrap(), "Halló, heimur!");
        assert_eq!(e.translate("en", "is", "hello   world").unwrap(), "halló   heimur");
    }

    #[test]
    fn the_longest_phrase_wins_and_capitals_carry_over() {
        let e = engine();
        assert_eq!(e.translate("en", "is", "Good morning, world").unwrap(), "Góðan daginn, heimur");
        assert_eq!(e.translate("en", "is", "GOOD MORNING world").unwrap(), "GÓÐAN DAGINN heimur");
        // a break that is not a blank stops a phrase, so "good" and "morning" are left alone
        assert_eq!(e.translate("en", "is", "good, morning hello world").unwrap(), "good, morning halló heimur");
    }

    #[test]
    fn unknown_words_pass_through_when_half_the_text_matched() {
        let e = engine();
        assert_eq!(e.translate("en", "is", "hello everyone").unwrap(), "halló everyone");
        assert_eq!(e.translate("en", "is", "hello to everyone").err(), Some(TranslationError::InternalError));
    }

    #[test]
    fn a_text_without_matches_or_without_a_pair_fails() {
        let e = engine();
        assert_eq!(e.translate("en", "is", "zzz qqq").err(), Some(TranslationError::InternalError));
        assert_eq!(e.translate("en", "is", "123 !!!").err(), Some(TranslationError::InternalError));
        assert_eq!(e.translate("en", "de", "hello").err(), Some(TranslationError::InternalError));
        assert_eq!(e.pair_count(), 2);
        assert_eq!(e.translate("is", "en", "heimur").unwrap(), "world");
    }

    #[test]
    fn a_session_translates_a_batch_with_it() {
        let mut availability = LanguageAvailability::new(&["en", "is"]);
        availability.install("en", "is");
        let session = TranslationSession::new(&availability, "en", "is", Box::new(engine())).unwrap();
        let out = session.translate_batch(&["Hello world", "world"]);
        assert_eq!(out[0], Ok("Halló heimur".to_string()));
        assert_eq!(out[1], Ok("heimur".to_string()));
    }
}
