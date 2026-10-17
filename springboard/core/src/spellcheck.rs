//! `NativeSpellChecker` parity: word tokenizer, a checker over a pluggable `Dictionary`, and per-user
//! learned/ignored words. The learned words persist through [`KeyValues`]: the SDK stores them in `UserDefaults`,
//! the shell in its settings file. Ranges are byte ranges on char boundaries.

use std::collections::HashSet;
use std::ops::Range;

/// The string store that keeps the learned words. The SDK implements it for `UserDefaults`; the shell for its
/// settings file (values there are one line, so a learned word never holds a space).
pub trait KeyValues {
    fn get_string(&self, key: &str) -> Option<String>;
    fn put_string(&mut self, key: &str, value: &str);
}

/// `NativeTextCheckingResult.CheckingType` subset used by the checker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckingType {
    Spelling,
    Grammar,
    Link,
    Date,
    Address,
    PhoneNumber,
}

/// `NativeTextCheckingTypes` option set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct CheckingTypes(pub u32);

impl CheckingTypes {
    pub const SPELLING: Self = Self(1);
    pub const GRAMMAR: Self = Self(2);
    pub const LINK: Self = Self(4);
    pub const DATE: Self = Self(8);
    pub const ADDRESS: Self = Self(16);
    pub const PHONE_NUMBER: Self = Self(32);
    pub fn contains(self, o: Self) -> bool {
        self.0 & o.0 == o.0
    }
    pub fn union(self, o: Self) -> Self {
        Self(self.0 | o.0)
    }
}

/// `NativeTextCheckingResult`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckingResult {
    pub kind: CheckingType,
    pub range: Range<usize>,
}

/// Word list backend (hunspell or a plain dictionary plugs in here).
pub trait Dictionary {
    fn language(&self) -> &str;
    fn contains(&self, word: &str) -> bool;
    /// Ranked correction guesses.
    fn suggest(&self, word: &str) -> Vec<String>;
    /// Dictionary words that start with `prefix` (case-insensitive, the prefix itself excluded), for predictive
    /// completion. A backend with no word enumeration keeps the empty default.
    fn completions(&self, _prefix: &str) -> Vec<String> {
        Vec::new()
    }
}

/// Sorts words shortest first, then alphabetically, so the likeliest short completions lead.
fn shortest_first(words: &mut [String]) {
    words.sort_by(|a, b| a.len().cmp(&b.len()).then_with(|| a.cmp(b)));
}

/// In-memory word list; lookups are case-insensitive. Suggestions are words at edit distance 1.
pub struct WordList {
    lang: String,
    words: HashSet<String>,
}

impl WordList {
    pub fn new(lang: &str, words: impl IntoIterator<Item = impl AsRef<str>>) -> Self {
        Self { lang: lang.into(), words: words.into_iter().map(|w| w.as_ref().to_lowercase()).collect() }
    }
}

fn edit1(a: &[char], b: &[char]) -> bool {
    if a.len() == b.len() {
        let d = a.iter().zip(b).filter(|(x, y)| x != y).count();
        if d == 1 {
            return true;
        }
        return d == 2
            && (0..a.len() - 1)
                .any(|i| a[i] == b[i + 1] && a[i + 1] == b[i] && a[..i] == b[..i] && a[i + 2..] == b[i + 2..]);
    }
    let (s, l) = if a.len() < b.len() { (a, b) } else { (b, a) };
    if l.len() - s.len() != 1 {
        return false;
    }
    (0..l.len()).any(|i| l[..i] == s[..i] && l[i + 1..] == s[i..])
}

impl Dictionary for WordList {
    fn language(&self) -> &str {
        &self.lang
    }
    fn contains(&self, word: &str) -> bool {
        self.words.contains(&word.to_lowercase())
    }
    fn suggest(&self, word: &str) -> Vec<String> {
        let w: Vec<char> = word.to_lowercase().chars().collect();
        let mut out: Vec<String> =
            self.words.iter().filter(|c| edit1(&w, &c.chars().collect::<Vec<_>>())).cloned().collect();
        out.sort();
        out
    }
    fn completions(&self, prefix: &str) -> Vec<String> {
        let p = prefix.to_lowercase();
        if p.is_empty() {
            return Vec::new();
        }
        let mut out: Vec<String> = self.words.iter().filter(|w| w.starts_with(&p) && w.as_str() != p).cloned().collect();
        shortest_first(&mut out);
        out
    }
}

/// Split text into words: runs of alphanumerics, with `'` or `’` kept between letters ("don't").
pub fn words(text: &str) -> Vec<Range<usize>> {
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

const LEARNED_KEY: &str = "NativeSpellChecker.learnedWords";

pub struct SpellChecker {
    dict: Box<dyn Dictionary + Send>,
    learned: HashSet<String>,
    ignored: HashSet<String>,
    pub automatically_corrects_spelling: bool,
}

impl SpellChecker {
    pub fn new(dict: Box<dyn Dictionary + Send>) -> Self {
        Self { dict, learned: HashSet::new(), ignored: HashSet::new(), automatically_corrects_spelling: false }
    }
    pub fn available_languages(&self) -> Vec<String> {
        vec![self.dict.language().to_string()]
    }
    /// Load learned words from the store.
    pub fn load(&mut self, d: &impl KeyValues) {
        self.learned = d
            .get_string(LEARNED_KEY)
            .unwrap_or_default()
            .lines()
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect();
    }
    pub fn learn_word(&mut self, w: &str, d: &mut impl KeyValues) {
        self.learned.insert(w.to_lowercase());
        self.save(d);
    }
    pub fn unlearn_word(&mut self, w: &str, d: &mut impl KeyValues) {
        self.learned.remove(&w.to_lowercase());
        self.save(d);
    }
    pub fn has_learned_word(&self, w: &str) -> bool {
        self.learned.contains(&w.to_lowercase())
    }
    /// Ignored words last for this checker only (scopes them to a document tag).
    pub fn ignore_word(&mut self, w: &str) {
        self.ignored.insert(w.to_lowercase());
    }
    fn save(&self, d: &mut impl KeyValues) {
        let mut v: Vec<&str> = self.learned.iter().map(String::as_str).collect();
        v.sort();
        d.put_string(LEARNED_KEY, &v.join("\n"));
    }
    fn known(&self, w: &str) -> bool {
        let l = w.to_lowercase();
        w.chars().all(|c| c.is_ascii_digit())
            || self.dict.contains(w)
            || self.learned.contains(&l)
            || self.ignored.contains(&l)
    }
    /// `checkSpelling(of:startingAt:)`: first misspelled word at or after byte `start`.
    pub fn check_spelling(&self, text: &str, start: usize) -> Option<Range<usize>> {
        words(text).into_iter().find(|r| r.start >= start && !self.known(&text[r.clone()]))
    }
    /// `correction(forWordRange:in:)`: best guess, if any.
    pub fn correction(&self, text: &str, range: Range<usize>) -> Option<String> {
        self.dict.suggest(&text[range]).into_iter().next()
    }
    pub fn guesses(&self, text: &str, range: Range<usize>) -> Vec<String> {
        self.dict.suggest(&text[range])
    }
    /// Predictive completions for the word being typed (`prefix`, the text since the last space). Learned words come
    /// first, then dictionary words; each group is shortest first and the list holds at most `limit` words. An empty
    /// prefix gives no completions, since every word would match it.
    pub fn completions(&self, prefix: &str, limit: usize) -> Vec<String> {
        let p = prefix.to_lowercase();
        if p.is_empty() {
            return Vec::new();
        }
        let mut learned: Vec<String> = self.learned.iter().filter(|w| w.starts_with(&p) && w.as_str() != p).cloned().collect();
        shortest_first(&mut learned);
        for w in self.dict.completions(prefix) {
            if !learned.contains(&w) {
                learned.push(w);
            }
        }
        learned.truncate(limit);
        learned
    }
    /// `check(_:types:)`: spelling results only; grammar and detector types are not produced here.
    pub fn check(&self, text: &str, types: CheckingTypes) -> Vec<CheckingResult> {
        let mut out = Vec::new();
        if types.contains(CheckingTypes::SPELLING) {
            for r in words(text) {
                if !self.known(&text[r.clone()]) {
                    out.push(CheckingResult { kind: CheckingType::Spelling, range: r });
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// An in-memory store for the learned words.
    #[derive(Default)]
    struct Mem(BTreeMap<String, String>);

    impl KeyValues for Mem {
        fn get_string(&self, key: &str) -> Option<String> {
            self.0.get(key).cloned()
        }
        fn put_string(&mut self, key: &str, value: &str) {
            self.0.insert(key.to_string(), value.to_string());
        }
    }

    fn sc() -> SpellChecker {
        SpellChecker::new(Box::new(WordList::new("en", ["hello", "world", "don't"])))
    }
    #[test]
    fn tokenizer_offsets() {
        let t = "Hi, don't ég 42!";
        let w: Vec<&str> = words(t).into_iter().map(|r| &t[r]).collect();
        assert_eq!(w, ["Hi", "don't", "ég", "42"]);
    }
    #[test]
    fn check_and_correct() {
        let c = sc();
        let t = "hello wrold";
        let r = c.check_spelling(t, 0).unwrap();
        assert_eq!(&t[r.clone()], "wrold");
        assert_eq!(c.correction(t, r).as_deref(), Some("world"));
        assert!(c.check_spelling(t, 7).is_none());
        assert_eq!(c.check(t, CheckingTypes::SPELLING).len(), 1);
    }
    #[test]
    fn learn_ignore_round_trip() {
        let mut c = sc();
        let mut d = Mem::default();
        let t = "hello wrold";
        c.learn_word("wrold", &mut d);
        assert!(c.check_spelling(t, 0).is_none());
        let mut c2 = sc();
        assert!(c2.check_spelling(t, 0).is_some());
        c2.load(&d);
        assert!(c2.check_spelling(t, 0).is_none());
        c2.unlearn_word("wrold", &mut d);
        assert!(c2.check_spelling(t, 0).is_some());
        c2.ignore_word("WROLD");
        assert!(c2.check_spelling(t, 0).is_none());
    }
}

#[cfg(test)]
mod completion_tests {
    use super::*;

    #[test]
    fn a_prefix_completes_to_dictionary_words_shortest_first() {
        let c = SpellChecker::new(Box::new(WordList::new("en", ["hello", "help", "helper", "world"])));
        assert_eq!(c.completions("hel", 10), ["help", "hello", "helper"]);
        assert_eq!(c.completions("HEL", 2), ["help", "hello"]);
        assert!(c.completions("xyz", 10).is_empty());
    }

    #[test]
    fn an_empty_or_exact_prefix_gives_no_completion_for_an_exact_word() {
        let c = SpellChecker::new(Box::new(WordList::new("en", ["help", "helper"])));
        assert!(c.completions("", 10).is_empty());
        assert_eq!(c.completions("help", 10), ["helper"]);
    }

    #[test]
    fn learned_words_lead_and_are_not_repeated() {
        struct Mem(std::collections::BTreeMap<String, String>);
        impl KeyValues for Mem {
            fn get_string(&self, key: &str) -> Option<String> {
                self.0.get(key).cloned()
            }
            fn put_string(&mut self, key: &str, value: &str) {
                self.0.insert(key.to_string(), value.to_string());
            }
        }
        let mut d = Mem(Default::default());
        let mut c = SpellChecker::new(Box::new(WordList::new("en", ["hello", "help"])));
        c.learn_word("helium", &mut d);
        c.learn_word("help", &mut d);
        assert_eq!(c.completions("hel", 10), ["help", "helium", "hello"]);
    }

    #[test]
    fn a_backend_without_enumeration_offers_no_dictionary_completion() {
        struct Plain;
        impl Dictionary for Plain {
            fn language(&self) -> &str {
                "en"
            }
            fn contains(&self, _word: &str) -> bool {
                false
            }
            fn suggest(&self, _word: &str) -> Vec<String> {
                Vec::new()
            }
        }
        let c = SpellChecker::new(Box::new(Plain));
        assert!(c.completions("hel", 10).is_empty());
    }
}
