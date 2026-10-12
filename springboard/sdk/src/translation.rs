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

#[cfg(test)]
mod tests {
    use super::*;

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
}
