//! Output guardrail for generated text: a length limit and a list of blocked phrases. A text is either passed through
//! unchanged, refused with the reason, or redacted, with every blocked phrase replaced by a mask.
//!
//! This is a rule-based screen, not a classifier. It stops only the phrases on its list, so it cannot judge the
//! meaning of a text. Phrases match without ASCII case; other characters must match exactly. The length limit counts
//! characters, not bytes. The model reads no text and calls no service, so the caller decides what to do with a
//! refusal.

/// Why a text was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GuardrailError {
    /// The text has more characters than the limit.
    TooLong { limit: usize, length: usize },
    /// The text contains a blocked phrase. The phrase is stored in lower case.
    Blocked { phrase: String },
    /// A blocked phrase is empty or only spaces, which would match everything.
    EmptyPhrase,
}

/// A length limit and a list of blocked phrases.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Guardrail {
    max_chars: usize,
    /// Blocked phrases, lower case, in the order they were added, without duplicates.
    phrases: Vec<String>,
}

impl Guardrail {
    /// A guardrail with no phrases that refuses texts longer than `max_chars` characters.
    pub fn new(max_chars: usize) -> Self {
        Guardrail { max_chars, phrases: Vec::new() }
    }

    /// Adds a blocked phrase. Surrounding spaces are removed. Adding a phrase twice keeps one copy.
    pub fn block(&mut self, phrase: &str) -> Result<(), GuardrailError> {
        let phrase = phrase.trim();
        if phrase.is_empty() {
            return Err(GuardrailError::EmptyPhrase);
        }
        let lower = phrase.to_ascii_lowercase();
        if !self.phrases.contains(&lower) {
            self.phrases.push(lower);
        }
        Ok(())
    }

    /// The blocked phrases, in lower case.
    pub fn phrases(&self) -> &[String] {
        &self.phrases
    }

    /// Passes `text` through unchanged, or says why it was refused. The length is checked first. When several
    /// phrases match, the first one added is reported.
    pub fn check<'a>(&self, text: &'a str) -> Result<&'a str, GuardrailError> {
        let length = text.chars().count();
        if length > self.max_chars {
            return Err(GuardrailError::TooLong { limit: self.max_chars, length });
        }
        let haystack = text.to_ascii_lowercase();
        match self.phrases.iter().find(|p| haystack.contains(p.as_str())) {
            Some(phrase) => Err(GuardrailError::Blocked { phrase: phrase.clone() }),
            None => Ok(text),
        }
    }

    /// The text with every blocked phrase replaced by `mask`. Where phrases overlap at one position, the longest wins.
    /// The length limit does not apply to redaction.
    pub fn redact(&self, text: &str, mask: &str) -> String {
        let haystack = text.to_ascii_lowercase();
        let mut out = String::with_capacity(text.len());
        let mut i = 0;
        while i < text.len() {
            let hit = self
                .phrases
                .iter()
                .filter(|p| haystack.get(i..).map_or(false, |s| s.starts_with(p.as_str())))
                .max_by_key(|p| p.len());
            match hit {
                Some(p) => {
                    out.push_str(mask);
                    i += p.len();
                }
                None => {
                    // Copy one whole character, so the next position is a character boundary.
                    let ch = text[i..].chars().next().unwrap_or('\u{FFFD}');
                    out.push(ch);
                    i += ch.len_utf8();
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_text_without_blocked_phrases_passes_unchanged() {
        let mut g = Guardrail::new(100);
        g.block("forbidden").unwrap();
        assert_eq!(g.check("A harmless sentence."), Ok("A harmless sentence."));
        assert_eq!(Guardrail::new(10).check(""), Ok(""));
    }

    #[test]
    fn a_blocked_phrase_is_found_without_ascii_case() {
        let mut g = Guardrail::new(1000);
        g.block("  Launch Codes ").unwrap();
        assert_eq!(g.phrases(), &["launch codes".to_string()], "trimmed and stored in lower case");
        let err = g.check("Please share the LAUNCH codes now.").unwrap_err();
        assert_eq!(err, GuardrailError::Blocked { phrase: "launch codes".to_string() });
    }

    #[test]
    fn the_length_limit_counts_characters_not_bytes() {
        let g = Guardrail::new(3);
        // Three characters, six bytes: allowed.
        assert_eq!(g.check("\u{e5}\u{e9}\u{ed}"), Ok("\u{e5}\u{e9}\u{ed}"));
        assert_eq!(
            g.check("\u{e5}\u{e9}\u{ed}\u{f3}"),
            Err(GuardrailError::TooLong { limit: 3, length: 4 })
        );
    }

    #[test]
    fn redaction_replaces_every_occurrence_in_the_original_case() {
        let mut g = Guardrail::new(100);
        g.block("secret key").unwrap();
        assert_eq!(g.redact("The SECRET KEY and secret key.", "[removed]"), "The [removed] and [removed].");
        assert_eq!(g.redact("nothing here", "[removed]"), "nothing here");
    }

    #[test]
    fn the_longest_phrase_wins_where_phrases_overlap() {
        let mut g = Guardrail::new(100);
        g.block("code").unwrap();
        g.block("access code").unwrap();
        assert_eq!(g.redact("my access code", "*"), "my *", "one mask for the longer phrase");
        assert_eq!(g.redact("a code", "*"), "a *");
    }

    #[test]
    fn empty_phrases_are_refused_and_duplicates_are_kept_once() {
        let mut g = Guardrail::new(100);
        assert_eq!(g.block("   "), Err(GuardrailError::EmptyPhrase));
        g.block("A").unwrap();
        g.block("a").unwrap();
        assert_eq!(g.phrases().len(), 1);
    }

    #[test]
    fn non_ascii_letters_match_only_exactly() {
        let mut g = Guardrail::new(100);
        g.block("\u{fe}orri").unwrap();
        // The capital thorn is not ASCII, so it does not fold to the lower-case thorn: the limit of the screen.
        assert_eq!(g.check("\u{de}orri"), Ok("\u{de}orri"));
        assert_eq!(g.check("\u{fe}orri"), Err(GuardrailError::Blocked { phrase: "\u{fe}orri".to_string() }));
    }
}
