//! `Translation` framework parity models: `LanguageAvailability` (which language pairs the framework supports and
//! which are installed), `LanguageAvailability.Status` and the `TranslationError` cases. BifrOSt ships no language
//! packs, so the supported list and the installed pairs come from the caller. Pure data, no I/O.

use std::fmt;

/// `LanguageAvailability.Status`: a pair is installed and ready, supported but not installed, or unsupported.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AvailabilityStatus {
    Installed,
    Supported,
    Unsupported,
}

/// `NativeTranslationError`: why a translation cannot run. Each case keeps the meaning of its framework case.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TranslationError {
    NothingToTranslate,
    UnableToIdentifyLanguage,
    NotInstalled,
    UnsupportedSourceLanguage,
    UnsupportedTargetLanguage,
    UnsupportedLanguagePairing,
    InternalError,
    AlreadyCancelled,
}

impl TranslationError {
    /// `errorDescription`: a short message for the person.
    pub fn error_description(self) -> &'static str {
        match self {
            Self::NothingToTranslate => "There is no text to translate.",
            Self::UnableToIdentifyLanguage => "The source language could not be identified.",
            Self::NotInstalled => "The translation languages are not installed.",
            Self::UnsupportedSourceLanguage => "The source language is not supported.",
            Self::UnsupportedTargetLanguage => "The target language is not supported.",
            Self::UnsupportedLanguagePairing => "This pair of languages is not supported.",
            Self::InternalError => "The translation engine failed.",
            Self::AlreadyCancelled => "The translation session was cancelled.",
        }
    }

    /// `failureReason`: why the translation failed.
    pub fn failure_reason(self) -> &'static str {
        match self {
            Self::NothingToTranslate => "The text is empty or only whitespace.",
            Self::UnableToIdentifyLanguage => "The text does not show which language it is written in.",
            Self::NotInstalled => "The languages are not downloaded on this device.",
            Self::UnsupportedSourceLanguage => "The source language has no translation support.",
            Self::UnsupportedTargetLanguage => "The target language has no translation support.",
            Self::UnsupportedLanguagePairing => "The source and target languages cannot be paired.",
            Self::InternalError => "An error occurred inside the translation engine.",
            Self::AlreadyCancelled => "The session was cancelled before it produced this result.",
        }
    }
}

impl fmt::Display for TranslationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.error_description())
    }
}

impl std::error::Error for TranslationError {}

/// The language code of a locale identifier: `en_US.UTF-8` and ` IS-is ` give `en` and `is`.
pub fn language_code(identifier: &str) -> String {
    identifier.trim().split(['_', '-', '.', '@']).next().unwrap_or("").to_ascii_lowercase()
}

/// `LanguageAvailability`: the supported languages, and the pairs the caller has installed. Pairs are directional,
/// so `en` to `is` and `is` to `en` are separate installs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LanguageAvailability {
    supported: Vec<String>,
    installed: Vec<(String, String)>,
}

impl LanguageAvailability {
    /// `supportedLanguages`: the framework's language list. Identifiers are reduced to codes and duplicates dropped.
    pub fn new<S: AsRef<str>>(supported: &[S]) -> Self {
        let mut codes: Vec<String> = Vec::new();
        for s in supported {
            let code = language_code(s.as_ref());
            if !code.is_empty() && !codes.contains(&code) {
                codes.push(code);
            }
        }
        Self { supported: codes, installed: Vec::new() }
    }

    /// The supported language codes, in the order they were given.
    pub fn supported_languages(&self) -> &[String] {
        &self.supported
    }

    /// `status(from:to:)`: installed, supported, or unsupported. A pair needs two different supported languages.
    pub fn status(&self, source: &str, target: &str) -> AvailabilityStatus {
        let (s, t) = (language_code(source), language_code(target));
        if !self.pair_supported(&s, &t) {
            AvailabilityStatus::Unsupported
        } else if self.is_installed(&s, &t) {
            AvailabilityStatus::Installed
        } else {
            AvailabilityStatus::Supported
        }
    }

    /// Records a pair as downloaded. Pairs the framework does not support are ignored.
    pub fn install(&mut self, source: &str, target: &str) {
        let (s, t) = (language_code(source), language_code(target));
        if self.pair_supported(&s, &t) && !self.is_installed(&s, &t) {
            self.installed.push((s, t));
        }
    }

    /// The check a session runs before it starts: `Ok` only for an installed pair, otherwise the error that
    /// `TranslationSession(installedSource:target:)` reports, checked in the framework's order.
    pub fn check_session(&self, source: &str, target: &str) -> Result<(), TranslationError> {
        let (s, t) = (language_code(source), language_code(target));
        if !self.supports(&s) {
            Err(TranslationError::UnsupportedSourceLanguage)
        } else if !self.supports(&t) {
            Err(TranslationError::UnsupportedTargetLanguage)
        } else if s == t {
            Err(TranslationError::UnsupportedLanguagePairing)
        } else if !self.is_installed(&s, &t) {
            Err(TranslationError::NotInstalled)
        } else {
            Ok(())
        }
    }

    fn supports(&self, code: &str) -> bool {
        self.supported.iter().any(|c| c == code)
    }

    fn pair_supported(&self, source: &str, target: &str) -> bool {
        source != target && self.supports(source) && self.supports(target)
    }

    fn is_installed(&self, source: &str, target: &str) -> bool {
        self.installed.iter().any(|(s, t)| s == source && t == target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn availability() -> LanguageAvailability {
        LanguageAvailability::new(&["en", "is", "de", "en_GB"])
    }

    #[test]
    fn language_codes_drop_region_script_and_charset() {
        assert_eq!(language_code("en_US.UTF-8"), "en");
        assert_eq!(language_code(" IS-is "), "is");
        assert_eq!(language_code("de@euro"), "de");
        assert_eq!(language_code(""), "");
    }

    #[test]
    fn supported_list_is_reduced_to_codes_without_duplicates() {
        let a = availability();
        let codes: Vec<&str> = a.supported_languages().iter().map(String::as_str).collect();
        assert_eq!(codes, vec!["en", "is", "de"]);
    }

    #[test]
    fn unsupported_sides_and_the_same_language_have_no_pair() {
        let a = availability();
        assert_eq!(a.status("en", "fr"), AvailabilityStatus::Unsupported);
        assert_eq!(a.status("fr", "en"), AvailabilityStatus::Unsupported);
        assert_eq!(a.status("en_US", "en_GB"), AvailabilityStatus::Unsupported);
    }

    #[test]
    fn install_marks_one_direction_and_ignores_unsupported_pairs() {
        let mut a = availability();
        a.install("en_US", "is_IS");
        assert_eq!(a.status("en", "is"), AvailabilityStatus::Installed);
        assert_eq!(a.status("is", "en"), AvailabilityStatus::Supported);
        a.install("en", "fr");
        assert_eq!(a.status("en", "fr"), AvailabilityStatus::Unsupported);
    }

    #[test]
    fn session_check_reports_each_failure_as_its_own_error() {
        let mut a = availability();
        assert_eq!(a.check_session("fr", "en"), Err(TranslationError::UnsupportedSourceLanguage));
        assert_eq!(a.check_session("en", "fr"), Err(TranslationError::UnsupportedTargetLanguage));
        assert_eq!(a.check_session("en_US", "en_GB"), Err(TranslationError::UnsupportedLanguagePairing));
        assert_eq!(a.check_session("en", "is"), Err(TranslationError::NotInstalled));
        a.install("en", "is");
        assert_eq!(a.check_session("en", "is"), Ok(()));
    }

    #[test]
    fn every_error_case_has_its_own_description_and_display_shows_it() {
        let all = [
            TranslationError::NothingToTranslate,
            TranslationError::UnableToIdentifyLanguage,
            TranslationError::NotInstalled,
            TranslationError::UnsupportedSourceLanguage,
            TranslationError::UnsupportedTargetLanguage,
            TranslationError::UnsupportedLanguagePairing,
            TranslationError::InternalError,
            TranslationError::AlreadyCancelled,
        ];
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                assert_ne!(a.error_description(), b.error_description());
            }
            assert!(!a.failure_reason().is_empty());
        }
        assert_eq!(TranslationError::NotInstalled.to_string(), TranslationError::NotInstalled.error_description());
    }
}
