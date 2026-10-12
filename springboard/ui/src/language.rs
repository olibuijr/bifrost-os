//! Language and script identification for text: `NativeLanguageRecognizer` (the dominant language and the
//! language hypotheses of a text, with hints and constraints), `NativeLanguage` (the language codes) and `NativeScript`
//! (the writing system of a letter or of a text). Pure data, no I/O.
//!
//! The recognizer scores each letter and word and turns the scores into probabilities:
//! * Latin-script text counts common function words per language and letters that point at one language
//!   (the Icelandic eth and thorn, the Polish stroke l, the Spanish tilde n).
//! * Text in a script that one language uses (Greek, Hebrew, Arabic, Thai, Hangul, Cyrillic for Russian) is
//!   recognised from its letters. Kana means Japanese; Han characters alone mean Chinese.
//!
//! Limits: no n-gram model is bundled, so short texts with one or two function words are weak evidence, and the
//! Scandinavian languages (Danish, Norwegian, Swedish) share most of their words and are not reliably told apart.
//! See the gaps in `docs/parity/natural-language.md`.

/// `NativeScript`: the writing system of a letter, the subset the recognizer distinguishes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Script {
    Latin,
    Greek,
    Cyrillic,
    Armenian,
    Hebrew,
    Arabic,
    Devanagari,
    Thai,
    Georgian,
    Hangul,
    Kana,
    Han,
}

impl Script {
    pub const ALL: [Script; 12] = [
        Script::Latin,
        Script::Greek,
        Script::Cyrillic,
        Script::Armenian,
        Script::Hebrew,
        Script::Arabic,
        Script::Devanagari,
        Script::Thai,
        Script::Georgian,
        Script::Hangul,
        Script::Kana,
        Script::Han,
    ];

    /// The script of a letter, or `None` for digits, punctuation, spaces and code points outside these blocks.
    pub fn of(c: char) -> Option<Script> {
        if !c.is_alphabetic() {
            return None;
        }
        let script = match c as u32 {
            0x41..=0x5A | 0x61..=0x7A | 0xC0..=0x24F | 0x1E00..=0x1EFF => Script::Latin,
            0x370..=0x3FF | 0x1F00..=0x1FFF => Script::Greek,
            0x400..=0x52F => Script::Cyrillic,
            0x530..=0x58F => Script::Armenian,
            0x590..=0x5FF => Script::Hebrew,
            0x600..=0x6FF | 0x750..=0x77F => Script::Arabic,
            0x900..=0x97F => Script::Devanagari,
            0xE00..=0xE7F => Script::Thai,
            0x10A0..=0x10FF => Script::Georgian,
            0x1100..=0x11FF | 0x3130..=0x318F | 0xAC00..=0xD7AF => Script::Hangul,
            0x3040..=0x30FF | 0x31F0..=0x31FF => Script::Kana,
            0x3400..=0x4DBF | 0x4E00..=0x9FFF => Script::Han,
            _ => return None,
        };
        Some(script)
    }

    /// The script with the most letters in `text`, or `None` when the text has no letters.
    /// A tie goes to the script that comes first in `ALL`.
    pub fn dominant(text: &str) -> Option<Script> {
        let mut counts = [0usize; 12];
        for c in text.chars() {
            if let Some(script) = Script::of(c) {
                counts[script as usize] += 1;
            }
        }
        let mut best: Option<(Script, usize)> = None;
        for (script, n) in Script::ALL.iter().zip(counts.iter().copied()) {
            if n > 0 && best.map_or(true, |(_, b)| n > b) {
                best = Some((*script, n));
            }
        }
        best.map(|(script, _)| script)
    }
}

/// `NativeLanguage`: the languages the recognizer can return. `code` gives the BCP 47 tag.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Language {
    Arabic,
    Chinese,
    Danish,
    Dutch,
    English,
    French,
    German,
    Greek,
    Hebrew,
    Icelandic,
    Italian,
    Japanese,
    Korean,
    Norwegian,
    Polish,
    Portuguese,
    Russian,
    Spanish,
    Swedish,
    Thai,
}

/// The number of languages. `Language::ALL` lists them in declaration order, so `language as usize` is its index.
pub const LANGUAGE_COUNT: usize = 20;

impl Language {
    pub const ALL: [Language; LANGUAGE_COUNT] = [
        Language::Arabic,
        Language::Chinese,
        Language::Danish,
        Language::Dutch,
        Language::English,
        Language::French,
        Language::German,
        Language::Greek,
        Language::Hebrew,
        Language::Icelandic,
        Language::Italian,
        Language::Japanese,
        Language::Korean,
        Language::Norwegian,
        Language::Polish,
        Language::Portuguese,
        Language::Russian,
        Language::Spanish,
        Language::Swedish,
        Language::Thai,
    ];

    /// The BCP 47 tag: "en", "is", "zh-Hans" and so on.
    pub fn code(self) -> &'static str {
        match self {
            Language::Arabic => "ar",
            Language::Chinese => "zh-Hans",
            Language::Danish => "da",
            Language::Dutch => "nl",
            Language::English => "en",
            Language::French => "fr",
            Language::German => "de",
            Language::Greek => "el",
            Language::Hebrew => "he",
            Language::Icelandic => "is",
            Language::Italian => "it",
            Language::Japanese => "ja",
            Language::Korean => "ko",
            Language::Norwegian => "no",
            Language::Polish => "pl",
            Language::Portuguese => "pt",
            Language::Russian => "ru",
            Language::Spanish => "es",
            Language::Swedish => "sv",
            Language::Thai => "th",
        }
    }

    /// The language with this tag (case-insensitive), or `None` when the recognizer does not know it.
    pub fn from_code(code: &str) -> Option<Language> {
        Language::ALL.iter().copied().find(|l| l.code().eq_ignore_ascii_case(code))
    }
}

/// Common function words of each Latin-script language, lowercase. Words shared by several languages count for each.
fn stopwords(language: Language) -> &'static [&'static str] {
    match language {
        Language::Danish => &["og", "er", "det", "at", "en", "til", "på", "med", "jeg", "ikke", "som", "har", "var", "for", "men", "vi", "du", "hvad", "mig"],
        Language::Dutch => &["de", "het", "een", "en", "van", "is", "dat", "niet", "ik", "je", "met", "voor", "zijn", "ook", "maar", "op", "te"],
        Language::English => &["the", "and", "is", "of", "to", "in", "that", "it", "with", "for", "are", "this", "you", "not", "was", "have", "they", "what"],
        Language::French => &["le", "la", "les", "et", "est", "un", "une", "de", "des", "du", "que", "qui", "pas", "je", "vous", "nous", "dans", "pour", "avec", "sur", "ce"],
        Language::German => &["und", "der", "die", "das", "ist", "nicht", "ich", "zu", "mit", "ein", "eine", "sich", "auch", "auf", "für", "es", "dem", "den", "wir"],
        Language::Icelandic => &["og", "er", "að", "ekki", "það", "við", "sem", "til", "með", "ég", "þú", "hann", "hún", "var", "eru", "þetta", "fyrir", "frá", "um"],
        Language::Italian => &["il", "di", "che", "è", "e", "un", "una", "per", "con", "non", "sono", "della", "anche", "ma", "lo", "gli", "questo"],
        Language::Norwegian => &["og", "er", "det", "at", "en", "til", "på", "med", "jeg", "ikke", "som", "har", "var", "av", "for", "men", "vi", "du", "hva", "å", "også"],
        Language::Polish => &["i", "w", "nie", "na", "się", "to", "że", "jest", "z", "do", "jak", "po", "tak", "ale", "co", "o"],
        Language::Portuguese => &["o", "a", "os", "as", "e", "é", "de", "que", "do", "da", "em", "um", "uma", "para", "com", "não", "por", "se", "mais", "mas"],
        Language::Spanish => &["el", "la", "los", "las", "y", "es", "de", "que", "en", "un", "una", "por", "para", "con", "no", "se", "lo", "pero", "como", "más"],
        Language::Swedish => &["och", "är", "det", "att", "en", "till", "på", "med", "jag", "inte", "som", "har", "var", "av", "för", "men", "vi", "du", "vad", "över"],
        _ => &[],
    }
}

/// Letters that point at one language, with the weight each adds to its score.
const LETTER_CUES: &[(char, Language, f64)] = &[
    ('ð', Language::Icelandic, 3.0),
    ('þ', Language::Icelandic, 3.0),
    ('æ', Language::Icelandic, 1.0),
    ('ö', Language::Icelandic, 0.5),
    ('ß', Language::German, 2.0),
    ('ä', Language::German, 1.0),
    ('ö', Language::German, 1.0),
    ('ñ', Language::Spanish, 2.0),
    ('ã', Language::Portuguese, 2.0),
    ('õ', Language::Portuguese, 2.0),
    ('ç', Language::French, 1.0),
    ('ç', Language::Portuguese, 1.0),
    ('è', Language::French, 1.0),
    ('ê', Language::French, 1.0),
    ('é', Language::French, 0.5),
    ('ł', Language::Polish, 3.0),
    ('ą', Language::Polish, 2.0),
    ('ę', Language::Polish, 2.0),
    ('ś', Language::Polish, 2.0),
    ('ż', Language::Polish, 2.0),
    ('ź', Language::Polish, 2.0),
    ('ø', Language::Danish, 1.0),
    ('ø', Language::Norwegian, 1.0),
    ('å', Language::Danish, 1.0),
    ('å', Language::Norwegian, 1.0),
    ('å', Language::Swedish, 1.0),
    ('ä', Language::Swedish, 1.0),
    ('ö', Language::Swedish, 1.0),
];

/// Raw evidence per language, indexed by `Language as usize`. Text is lowercased first.
/// True for a whitespace-separated token that is an address rather than a word: a link, an email address or a
/// dotted host name. Its parts ("com", "www") say nothing about the language of the text around it.
fn is_address(token: &str) -> bool {
    token.contains("://") || token.contains('@') || token.starts_with("www.") || {
        let t = token.trim_matches(|c: char| !c.is_alphanumeric());
        t.contains('.') && t.split('.').filter(|p| !p.is_empty()).count() >= 2 && !t.ends_with('.')
    }
}

fn evidence(text: &str) -> [f64; LANGUAGE_COUNT] {
    let mut s = [0.0f64; LANGUAGE_COUNT];
    let lower: String = text.to_lowercase().split_whitespace().filter(|t| !is_address(t)).collect::<Vec<_>>().join(" ");
    for word in lower.split(|c: char| !c.is_alphabetic()).filter(|w| !w.is_empty()) {
        for language in Language::ALL {
            if stopwords(language).contains(&word) {
                s[language as usize] += 1.0;
            }
        }
    }
    let mut han = 0.0;
    let mut kana = 0.0;
    for c in lower.chars() {
        for &(letter, language, weight) in LETTER_CUES {
            if letter == c {
                s[language as usize] += weight;
            }
        }
        match Script::of(c) {
            Some(Script::Greek) => s[Language::Greek as usize] += 1.0,
            Some(Script::Cyrillic) => s[Language::Russian as usize] += 1.0,
            Some(Script::Hebrew) => s[Language::Hebrew as usize] += 1.0,
            Some(Script::Arabic) => s[Language::Arabic as usize] += 1.0,
            Some(Script::Thai) => s[Language::Thai as usize] += 1.0,
            Some(Script::Hangul) => s[Language::Korean as usize] += 1.0,
            Some(Script::Kana) => {
                kana += 1.0;
                s[Language::Japanese as usize] += 1.0;
            }
            Some(Script::Han) => han += 1.0,
            _ => {}
        }
    }
    // Han characters are Chinese, unless kana in the same text makes it Japanese.
    if kana > 0.0 {
        s[Language::Japanese as usize] += han;
    } else {
        s[Language::Chinese as usize] += han;
    }
    s
}

/// `NativeLanguageRecognizer`: analyses text and reports its dominant language and the probability of each candidate.
/// `process` adds text to the analysis, as `processString(_:)` does, and `reset` clears it.
#[derive(Clone, Debug, Default)]
pub struct LanguageRecognizer {
    text: String,
    /// `languageHints`: per-language weight in 0...1. A hinted language's evidence is multiplied by `1 + hint`.
    hints: [f64; LANGUAGE_COUNT],
    /// `languageConstraints`: when set, only these languages can be returned.
    constraints: Option<Vec<Language>>,
}

impl LanguageRecognizer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds `text` to the analysis. Several calls accumulate, as in the framework.
    pub fn process(&mut self, text: &str) {
        self.text.push_str(text);
        self.text.push(' ');
    }

    /// Clears the analysed text. Hints and constraints stay.
    pub fn reset(&mut self) {
        self.text.clear();
    }

    /// Sets the hint for `language`, clamped to 0...1. A hint of 0 removes it.
    pub fn set_hint(&mut self, language: Language, weight: f64) {
        self.hints[language as usize] = weight.clamp(0.0, 1.0);
    }

    /// Limits the answer to `languages`. `None` removes the limit.
    pub fn set_constraints(&mut self, languages: Option<Vec<Language>>) {
        self.constraints = languages;
    }

    /// Up to `maximum` candidate languages with their probabilities, most likely first. The probabilities are over
    /// all candidates, so the list may be shorter than the total. Empty when no candidate has any evidence.
    pub fn hypotheses(&self, maximum: usize) -> Vec<(Language, f64)> {
        if maximum == 0 {
            return Vec::new();
        }
        let scores = evidence(&self.text);
        let mut weighted = [0.0f64; LANGUAGE_COUNT];
        for (i, language) in Language::ALL.iter().enumerate() {
            let allowed = self.constraints.as_ref().map_or(true, |c| c.contains(language));
            if allowed {
                weighted[i] = scores[i] * (1.0 + self.hints[i]);
            }
        }
        let total: f64 = weighted.iter().sum();
        if total <= 0.0 {
            return Vec::new();
        }
        let mut out: Vec<(Language, f64)> = Language::ALL
            .iter()
            .copied()
            .zip(weighted.iter().copied())
            .filter(|(_, w)| *w > 0.0)
            .map(|(l, w)| (l, w / total))
            .collect();
        out.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        out.truncate(maximum);
        out
    }

    /// The most likely language of the analysed text, or `None` when there is no evidence.
    pub fn dominant(&self) -> Option<Language> {
        self.hypotheses(1).first().map(|(l, _)| *l)
    }
}

/// The dominant language of `text` with no hints or constraints, or `None` when it has no recognisable letters.
pub fn dominant_language(text: &str) -> Option<Language> {
    let mut recognizer = LanguageRecognizer::new();
    recognizer.process(text);
    recognizer.dominant()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prob(h: &[(Language, f64)], l: Language) -> f64 {
        h.iter().find(|(x, _)| *x == l).map_or(0.0, |(_, p)| *p)
    }

    #[test]
    fn links_and_addresses_are_not_evidence() {
        // "com" is a Portuguese function word; inside a link or an address it must not count
        assert_eq!(dominant_language("https://example.com"), None);
        assert_eq!(dominant_language("mail anna@example.com or www.example.com"), None);
        assert_eq!(dominant_language("see the notes on example.com"), Some(Language::English));
        assert!(is_address("example.com,") && !is_address("end.") && !is_address("com"));
    }

    #[test]
    fn english_sentence_is_identified_by_function_words() {
        let text = "The quick brown fox jumps over the lazy dog and it is not sad";
        assert_eq!(dominant_language(text), Some(Language::English));
        let mut r = LanguageRecognizer::new();
        r.process(text);
        assert!(prob(&r.hypotheses(3), Language::English) > 0.8);
    }

    #[test]
    fn icelandic_letters_and_words_beat_danish_and_norwegian() {
        assert_eq!(dominant_language("Þetta er mjög góður dagur og ég er glöð"), Some(Language::Icelandic));
    }

    #[test]
    fn german_and_french_sentences_are_told_apart() {
        assert_eq!(
            dominant_language("Ich bin nicht sicher, ob das der richtige Weg ist und wir gehen auch zu der Schule"),
            Some(Language::German)
        );
        assert_eq!(dominant_language("Je ne sais pas si vous êtes dans le bon endroit pour le café"), Some(Language::French));
    }

    #[test]
    fn single_script_languages_are_recognised_from_their_letters() {
        assert_eq!(dominant_language("Привет, как дела? Это тест"), Some(Language::Russian));
        assert_eq!(dominant_language("Γειά σου κόσμε"), Some(Language::Greek));
        assert_eq!(dominant_language("こんにちは世界"), Some(Language::Japanese));
        assert_eq!(dominant_language("你好世界"), Some(Language::Chinese));
    }

    #[test]
    fn text_without_letters_has_no_language() {
        assert_eq!(dominant_language(""), None);
        assert_eq!(dominant_language("12345 !!! --"), None);
        assert!(LanguageRecognizer::new().hypotheses(5).is_empty());
    }

    #[test]
    fn hints_break_a_tie_and_constraints_remove_candidates() {
        // "de" is a function word of Spanish, Portuguese, Dutch and French: equal candidates before any hint.
        let mut r = LanguageRecognizer::new();
        r.process("de");
        assert!(r.hypotheses(9).len() >= 3);
        r.set_hint(Language::Spanish, 0.5);
        assert_eq!(r.dominant(), Some(Language::Spanish));
        r.set_constraints(Some(vec![Language::Portuguese, Language::Dutch]));
        let picked = r.dominant();
        assert!(picked == Some(Language::Portuguese) || picked == Some(Language::Dutch));
    }

    #[test]
    fn process_accumulates_until_reset() {
        let mut r = LanguageRecognizer::new();
        r.process("the");
        r.process("and is");
        assert_eq!(r.dominant(), Some(Language::English));
        r.reset();
        assert_eq!(r.dominant(), None);
    }

    #[test]
    fn language_codes_round_trip() {
        assert_eq!(Language::Icelandic.code(), "is");
        assert_eq!(Language::Chinese.code(), "zh-Hans");
        assert_eq!(Language::from_code("IS"), Some(Language::Icelandic));
        assert_eq!(Language::from_code("zz"), None);
        for (i, l) in Language::ALL.iter().enumerate() {
            assert_eq!(*l as usize, i);
        }
    }

    #[test]
    fn script_of_letters_and_dominant_script_of_text() {
        assert_eq!(Script::of('a'), Some(Script::Latin));
        assert_eq!(Script::of('Ж'), Some(Script::Cyrillic));
        assert_eq!(Script::of('漢'), Some(Script::Han));
        assert_eq!(Script::of('ა'), Some(Script::Georgian));
        assert_eq!(Script::of('7'), None);
        assert_eq!(Script::dominant("Hello Привет мир"), Some(Script::Cyrillic));
        assert_eq!(Script::dominant("123"), None);
    }
}
