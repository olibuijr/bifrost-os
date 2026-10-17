//! The translation engines the system offers behind [`Translator`]: the rule-based `apertium` program for the language
//! pairs installed on this computer, and a chain that tries the phrase packs first and the program second.
//!
//! Translation runs on this computer. [`ApertiumEngine`] starts `apertium -u <pair>` with the text on its standard
//! input, so nothing leaves the machine and no network is used. The pairs are whatever `apertium -l` lists (the
//! person installs a pair as a package; the pair names use two or three letter language codes, and both spellings
//! are understood). [`ApertiumEngine::availability`] turns the installed pairs into the
//! [`LanguageAvailability`] a [`TranslationSession`](crate::translation::TranslationSession) and the translation
//! popover check before they run.
//!
//! Limits: a rule-based engine is word-for-word in places and marks words it does not know with `*`; the text is passed
//! through as it is, and the marks are removed by [`clean_unknown_marks`]. There is no neural engine in the Arch
//! repositories, so a pair that is not installed is `NotInstalled`, never a guess. The program sits behind
//! [`Runner`](crate::text_recognition::Runner), so the pair choice and the text handling are tested without it.

use crate::text_recognition::{RecognitionError, RunOutput, Runner, SystemRunner};
use crate::translation::{TranslationError, Translator};
use bifrost_ui::translation::{language_code, LanguageAvailability};

/// The engine program.
pub const PROGRAM: &str = "apertium";

/// The longest text handed to the engine in one call, in bytes.
pub const MAX_TEXT_BYTES: usize = 256 * 1024;

/// The three letter code of a two letter language code, for the languages the pair packages commonly cover.
pub fn three_letter(code: &str) -> Option<&'static str> {
    Some(match code {
        "en" => "eng",
        "is" => "isl",
        "de" => "deu",
        "fr" => "fra",
        "es" => "spa",
        "it" => "ita",
        "pt" => "por",
        "ca" => "cat",
        "nl" => "nld",
        "sv" => "swe",
        "da" => "dan",
        "nb" | "no" => "nob",
        "nn" => "nno",
        "fi" => "fin",
        "pl" => "pol",
        "cs" => "ces",
        "tr" => "tur",
        "el" => "ell",
        "ru" => "rus",
        "uk" => "ukr",
        "ar" => "ara",
        "he" => "heb",
        "ro" => "ron",
        "gl" => "glg",
        _ => return None,
    })
}

/// The pair names in the output of `apertium -l`: one mode per line, such as `en-es` or `isl-eng`. Modes that are not
/// a plain `<source>-<target>` pair (`es-en_US`, `en-es-anmor`, `es-en-debug`) are skipped.
pub fn parse_modes(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| {
            let parts: Vec<&str> = l.split('-').collect();
            parts.len() == 2 && parts.iter().all(|p| (2..=3).contains(&p.len()) && p.bytes().all(|b| b.is_ascii_lowercase()))
        })
        .map(str::to_string)
        .collect()
}

/// Removes the `*` the engine puts before a word it does not know (`*word`), leaving the word. A `*` that stands alone,
/// or in the middle of a word, is kept.
pub fn clean_unknown_marks(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut at_word_start = true;
    let chars: Vec<char> = text.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        if c == '*' && at_word_start && chars.get(i + 1).map_or(false, |n| n.is_alphanumeric()) {
            continue;
        }
        at_word_start = c.is_whitespace() || matches!(c, '(' | '"' | '\u{201C}' | '\u{2018}');
        out.push(c);
    }
    out
}

/// The rule-based engine over the pairs found on this computer.
#[derive(Clone, Debug)]
pub struct ApertiumEngine<R: Runner = SystemRunner> {
    runner: R,
    modes: Vec<String>,
}

impl ApertiumEngine<SystemRunner> {
    /// Asks the installed program which pairs it has. When the program is missing there are none.
    pub fn discover() -> ApertiumEngine<SystemRunner> {
        ApertiumEngine::discover_with(SystemRunner)
    }
}

impl<R: Runner> ApertiumEngine<R> {
    pub fn discover_with(runner: R) -> ApertiumEngine<R> {
        let modes = match runner.run(PROGRAM, &["-l".to_string()], &[]) {
            Ok(out) if out.success => parse_modes(&String::from_utf8_lossy(&out.stdout)),
            _ => Vec::new(),
        };
        ApertiumEngine { runner, modes }
    }

    pub fn modes(&self) -> &[String] {
        &self.modes
    }

    /// The installed mode for a pair of language codes (`en_US` and `en` are the same language), in either spelling.
    pub fn mode_for(&self, source: &str, target: &str) -> Option<String> {
        let (s, t) = (language_code(source), language_code(target));
        let mut candidates = vec![format!("{s}-{t}")];
        if let (Some(s3), Some(t3)) = (three_letter(&s), three_letter(&t)) {
            candidates.push(format!("{s3}-{t3}"));
        }
        candidates.into_iter().find(|c| self.modes.contains(c))
    }

    /// The availability for `supported` languages: every ordered pair with an installed mode is installed.
    pub fn availability<S: AsRef<str>>(&self, supported: &[S]) -> LanguageAvailability {
        let mut availability = LanguageAvailability::new(supported);
        let codes = availability.supported_languages().to_vec();
        for source in &codes {
            for target in &codes {
                if source != target && self.mode_for(source, target).is_some() {
                    availability.install(source, target);
                }
            }
        }
        availability
    }
}

impl<R: Runner> Translator for ApertiumEngine<R> {
    fn translate(&self, source: &str, target: &str, text: &str) -> Result<String, TranslationError> {
        if text.trim().is_empty() {
            return Err(TranslationError::NothingToTranslate);
        }
        if text.len() > MAX_TEXT_BYTES {
            return Err(TranslationError::InternalError);
        }
        let mode = self.mode_for(source, target).ok_or(TranslationError::NotInstalled)?;
        let args = ["-u".to_string(), mode];
        let out: RunOutput = self.runner.run(PROGRAM, &args, text.as_bytes()).map_err(|e| match e {
            RecognitionError::EngineMissing => TranslationError::NotInstalled,
            _ => TranslationError::InternalError,
        })?;
        if !out.success {
            return Err(TranslationError::InternalError);
        }
        let translated = String::from_utf8(out.stdout).map_err(|_| TranslationError::InternalError)?;
        let cleaned = clean_unknown_marks(translated.trim_end_matches(['\n', '\r']));
        if cleaned.trim().is_empty() {
            return Err(TranslationError::InternalError);
        }
        Ok(cleaned)
    }
}

/// Tries each translator in turn and returns the first answer. When none answers it returns the error of the last
/// one, except that a `NothingToTranslate` from the first is final (no engine can do more with an empty text).
pub struct Chain {
    links: Vec<Box<dyn Translator>>,
}

impl Chain {
    pub fn new(links: Vec<Box<dyn Translator>>) -> Chain {
        Chain { links }
    }
}

impl Translator for Chain {
    fn translate(&self, source: &str, target: &str, text: &str) -> Result<String, TranslationError> {
        let mut last = TranslationError::NotInstalled;
        for link in &self.links {
            match link.translate(source, target, text) {
                Ok(t) => return Ok(t),
                Err(TranslationError::NothingToTranslate) => return Err(TranslationError::NothingToTranslate),
                Err(e) => last = e,
            }
        }
        Err(last)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::translation::TranslationMemory;
    use std::cell::RefCell;

    struct Fake {
        modes: &'static str,
        answer: &'static str,
        success: bool,
        calls: RefCell<Vec<(Vec<String>, String)>>,
    }

    impl Fake {
        fn new(answer: &'static str) -> Fake {
            Fake { modes: "en-es\nisl-eng\nes-en_US\nen-es-anmor\nfr-es-debug\n", answer, success: true, calls: RefCell::new(Vec::new()) }
        }
    }

    impl Runner for &Fake {
        fn run(&self, program: &str, args: &[String], stdin: &[u8]) -> Result<RunOutput, RecognitionError> {
            (**self).run(program, args, stdin)
        }
    }

    impl Runner for Fake {
        fn run(&self, _program: &str, args: &[String], stdin: &[u8]) -> Result<RunOutput, RecognitionError> {
            if args == ["-l"] {
                return Ok(RunOutput { stdout: self.modes.as_bytes().to_vec(), stderr: Vec::new(), success: true });
            }
            self.calls.borrow_mut().push((args.to_vec(), String::from_utf8_lossy(stdin).into_owned()));
            Ok(RunOutput { stdout: self.answer.as_bytes().to_vec(), stderr: Vec::new(), success: self.success })
        }
    }

    struct Missing;

    impl Runner for Missing {
        fn run(&self, _: &str, _: &[String], _: &[u8]) -> Result<RunOutput, RecognitionError> {
            Err(RecognitionError::EngineMissing)
        }
    }

    #[test]
    fn only_plain_pairs_are_modes() {
        let fake = Fake::new("");
        let engine = ApertiumEngine::discover_with(&fake);
        assert_eq!(engine.modes(), ["en-es".to_string(), "isl-eng".to_string()]);
    }

    #[test]
    fn a_pair_is_found_in_either_spelling_and_for_a_locale() {
        let fake = Fake::new("");
        let engine = ApertiumEngine::discover_with(&fake);
        assert_eq!(engine.mode_for("en_US", "es").as_deref(), Some("en-es"));
        assert_eq!(engine.mode_for("is", "en-GB").as_deref(), Some("isl-eng"));
        assert_eq!(engine.mode_for("es", "en"), None);
        assert_eq!(engine.mode_for("xx", "yy"), None);
    }

    #[test]
    fn the_installed_pairs_become_the_availability() {
        let fake = Fake::new("");
        let engine = ApertiumEngine::discover_with(&fake);
        let a = engine.availability(&["en", "es", "is"]);
        assert!(a.check_session("en", "es").is_ok());
        assert!(a.check_session("is", "en").is_ok());
        assert_eq!(a.check_session("es", "en"), Err(TranslationError::NotInstalled));
    }

    #[test]
    fn a_missing_program_has_no_modes_and_translates_nothing() {
        let engine = ApertiumEngine::discover_with(Missing);
        assert!(engine.modes().is_empty());
        assert_eq!(engine.translate("en", "es", "hello"), Err(TranslationError::NotInstalled));
    }

    #[test]
    fn the_text_goes_in_on_stdin_with_the_mode_and_the_answer_loses_its_marks_and_final_newline() {
        let fake = Fake::new("Hola *mundo, *amigo.\n");
        let engine = ApertiumEngine::discover_with(&fake);
        let out = engine.translate("en", "es", "Hello world, friend.").unwrap();
        assert_eq!(out, "Hola mundo, amigo.");
        let calls = fake.calls.borrow();
        assert_eq!(calls[0].0, ["-u", "en-es"]);
        assert_eq!(calls[0].1, "Hello world, friend.");
    }

    #[test]
    fn an_empty_text_a_huge_text_a_failed_run_and_an_empty_answer_are_errors() {
        let fake = Fake::new("x");
        let engine = ApertiumEngine::discover_with(&fake);
        assert_eq!(engine.translate("en", "es", "  "), Err(TranslationError::NothingToTranslate));
        assert_eq!(engine.translate("en", "es", &"a".repeat(MAX_TEXT_BYTES + 1)), Err(TranslationError::InternalError));
        assert!(fake.calls.borrow().is_empty());
        let mut failing = Fake::new("x");
        failing.success = false;
        assert_eq!(ApertiumEngine::discover_with(&failing).translate("en", "es", "hi"), Err(TranslationError::InternalError));
        let blank = Fake::new("\n");
        assert_eq!(ApertiumEngine::discover_with(&blank).translate("en", "es", "hi"), Err(TranslationError::InternalError));
    }

    #[test]
    fn marks_are_removed_only_at_the_start_of_a_word() {
        assert_eq!(clean_unknown_marks("*uno dos *tres"), "uno dos tres");
        assert_eq!(clean_unknown_marks("a * b 2*3 (*x)"), "a * b 2*3 (x)");
    }

    #[test]
    fn the_chain_prefers_the_phrase_pack_and_falls_back_to_the_engine() {
        let mut memory = TranslationMemory::new();
        memory.insert("en", "es", "hello", "buenos dias");
        let engine = ApertiumEngine::discover_with(Fake::new("hola\n"));
        let chain = Chain::new(vec![Box::new(memory), Box::new(engine)]);
        assert_eq!(chain.translate("en", "es", "hello").unwrap(), "buenos dias");
        assert_eq!(chain.translate("en", "es", "good morning").unwrap(), "hola");
        assert_eq!(Chain::new(vec![]).translate("en", "es", "x"), Err(TranslationError::NotInstalled));
    }

    #[test]
    fn the_chain_reports_the_last_error_and_stops_on_an_empty_text() {
        let chain = Chain::new(vec![Box::new(TranslationMemory::new()), Box::new(ApertiumEngine::discover_with(Missing))]);
        assert_eq!(chain.translate("en", "es", "hello"), Err(TranslationError::NotInstalled));
        assert_eq!(chain.translate("en", "es", " "), Err(TranslationError::NothingToTranslate));
    }
}
