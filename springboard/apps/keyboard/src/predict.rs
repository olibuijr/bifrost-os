//! Predictive text for the on-screen keyboard: a prefix trie over bundled
//! English and Icelandic word lists with frequencies, a previous-word bigram
//! bonus (seeded, then learned from what is typed), and an edit-distance-1
//! autocorrect that keeps the first letter. Pure logic, no drawing.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
#[cfg(not(test))]
use std::sync::OnceLock;

/// Word lists, one `word count` pair per line, most frequent first. The
/// counts come from film-subtitle corpora (FrequencyWords, OpenSubtitles).
const EN: &str = include_str!("words/en.txt");
const IS: &str = include_str!("words/is.txt");
/// Word lists shipped by the keyboard system package (`is.olibuijr.keyboard`), `<lang>.txt`:
/// each one found there replaces the compiled-in list of that language.
pub fn package_words() -> std::path::PathBuf {
    package_dir().join("assets/words")
}

/// The installed package directory: where this executable lives (the sandbox exposes only that and the app's data).
pub fn package_dir() -> std::path::PathBuf {
    std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.to_path_buf())).unwrap_or_else(|| std::path::PathBuf::from("."))
}
/// The Icelandic counts come from a much smaller corpus; this scales them so
/// common Icelandic words compete with common English ones.
const IS_WEIGHT: u32 = 30;
/// Frequency given to a word the user typed that is not in the lists.
const LEARNED_FREQ: u32 = 20_000;
/// Added to a candidate's score when it followed the previous word before.
const BIGRAM_BONUS: f64 = 3.0;
/// Letters tried when inserting or substituting during autocorrect.
const ALPHABET: &str = "abcdefghijklmnopqrstuvwxyzáéíóúýþæöð";

/// Common word pairs, so next-word prediction works before anything is learned.
const SEED_BIGRAMS: &[(&str, &str)] = &[
    ("i", "am"), ("i", "will"), ("i", "have"), ("i", "think"), ("i", "love"), ("i", "was"),
    ("you", "are"), ("you", "can"), ("you", "have"), ("you", "know"),
    ("thank", "you"), ("see", "you"), ("love", "you"), ("how", "are"), ("what", "is"),
    ("it", "is"), ("it", "was"), ("do", "you"), ("do", "not"), ("can", "you"), ("want", "to"),
    ("going", "to"), ("have", "a"), ("have", "to"), ("in", "the"), ("of", "the"), ("on", "the"),
    ("to", "the"), ("to", "be"), ("good", "morning"), ("good", "night"), ("let", "me"),
    ("ég", "er"), ("ég", "get"), ("ég", "vil"), ("ég", "veit"), ("þú", "ert"), ("þú", "getur"),
    ("takk", "fyrir"), ("góðan", "daginn"), ("gott", "kvöld"), ("hvað", "er"), ("hvernig", "hefur"),
    ("það", "er"), ("það", "var"), ("er", "ekki"), ("við", "erum"), ("til", "að"), ("í", "dag"),
    ("á", "morgun"), ("ég", "elska"), ("sjáumst", "á"),
];

#[derive(Default)]
struct Node {
    kids: Vec<(char, u32)>,
    /// 0 when no word ends here
    freq: u32,
}

/// A prefix trie of words and their frequencies.
pub struct Trie {
    nodes: Vec<Node>,
}

impl Default for Trie {
    fn default() -> Self {
        Trie { nodes: vec![Node::default()] }
    }
}

impl Trie {
    /// Adds `word` (kept lower-case by callers) or raises its frequency.
    pub fn insert(&mut self, word: &str, freq: u32) {
        let mut at = 0usize;
        for c in word.chars() {
            at = match self.nodes[at].kids.iter().find(|(k, _)| *k == c) {
                Some(&(_, i)) => i as usize,
                None => {
                    self.nodes.push(Node::default());
                    let i = (self.nodes.len() - 1) as u32;
                    self.nodes[at].kids.push((c, i));
                    i as usize
                }
            };
        }
        self.nodes[at].freq = self.nodes[at].freq.max(freq.max(1));
    }

    fn find(&self, prefix: &str) -> Option<usize> {
        let mut at = 0usize;
        for c in prefix.chars() {
            at = self.nodes[at].kids.iter().find(|(k, _)| *k == c)?.1 as usize;
        }
        Some(at)
    }

    /// The frequency of `word`, when it is in the trie.
    pub fn freq(&self, word: &str) -> Option<u32> {
        let f = self.nodes[self.find(word)?].freq;
        (f > 0).then_some(f)
    }

    /// Up to `n` words starting with `prefix` (the prefix itself included
    /// when it is a word), most frequent first, ties alphabetical.
    pub fn complete(&self, prefix: &str, n: usize) -> Vec<(String, u32)> {
        let Some(root) = self.find(prefix) else { return vec![] };
        let mut found = vec![];
        let mut stack = vec![(root, prefix.to_string())];
        while let Some((i, word)) = stack.pop() {
            let node = &self.nodes[i];
            if node.freq > 0 {
                found.push((word.clone(), node.freq));
            }
            for &(c, k) in &node.kids {
                let mut w = word.clone();
                w.push(c);
                stack.push((k as usize, w));
            }
        }
        found.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        found.truncate(n);
        found
    }
}

pub struct Predictor {
    trie: Trie,
    bigrams: HashMap<(String, String), u32>,
    /// words taught on purpose, kept so a list reload does not forget them
    learned: Vec<String>,
}

/// The English and Icelandic lists: `<dir>/en.txt` and `<dir>/is.txt` when they hold at
/// least one `word count` line, else the compiled-in ones.
pub fn lists_from(dir: &Path) -> (String, String) {
    let pick = |lang: &str, bundled: &str| match std::fs::read_to_string(dir.join(format!("{lang}.txt"))) {
        Ok(t) if t.lines().any(|l| l.split_whitespace().count() >= 2) => t,
        _ => bundled.to_string(),
    };
    (pick("en", EN), pick("is", IS))
}

/// Re-reads the package word lists into the shared predictor (learned words and pairs stay).
pub fn reload_words(dir: &Path) {
    let (en, is) = lists_from(dir);
    if let Ok(mut p) = shared().lock() {
        p.reload(&en, &is);
    }
}

impl Predictor {
    /// The bundled English and Icelandic lists with the seed bigrams.
    pub fn new() -> Self {
        Predictor::with_lists(EN, IS)
    }

    /// Like `new` with the given English and Icelandic lists.
    pub fn with_lists(en: &str, is: &str) -> Self {
        let mut p = Predictor::from_lists(&[(en, 1), (is, IS_WEIGHT)]);
        for (a, b) in SEED_BIGRAMS {
            p.learn(a, b);
        }
        p
    }

    /// Swaps in new lists; learned bigrams and taught words are kept.
    pub fn reload(&mut self, en: &str, is: &str) {
        let mut fresh = Predictor::with_lists(en, is);
        fresh.bigrams.extend(self.bigrams.drain());
        for w in std::mem::take(&mut self.learned) {
            fresh.add_word(&w);
        }
        *self = fresh;
    }

    /// Builds from `(list, weight)` pairs of `word count` lines.
    pub fn from_lists(lists: &[(&str, u32)]) -> Self {
        let mut trie = Trie::default();
        for (list, weight) in lists {
            for line in list.lines() {
                let mut parts = line.split_whitespace();
                let (Some(word), Some(count)) = (parts.next(), parts.next()) else { continue };
                let count: u32 = count.parse().unwrap_or(1);
                trie.insert(word, count.saturating_mul(*weight));
            }
        }
        Predictor { trie, bigrams: HashMap::new(), learned: vec![] }
    }

    pub fn is_word(&self, word: &str) -> bool {
        self.trie.freq(&word.to_lowercase()).is_some()
    }

    /// Remembers that `word` followed `prev`. Unknown words are skipped (a
    /// typo must not become a word); `add_word` teaches those on purpose.
    pub fn learn(&mut self, prev: &str, word: &str) {
        let (prev, word) = (prev.to_lowercase(), word.to_lowercase());
        if word.is_empty() || self.trie.freq(&word).is_none() {
            return;
        }
        if !prev.is_empty() {
            *self.bigrams.entry((prev, word)).or_insert(0) += 1;
        }
    }

    /// Teaches a word the user typed on purpose (after undoing a correction).
    pub fn add_word(&mut self, word: &str) {
        let w = word.to_lowercase();
        self.trie.insert(&w, LEARNED_FREQ);
        if !self.learned.contains(&w) {
            self.learned.push(w);
        }
    }

    fn score(&self, prev: Option<&str>, word: &str, freq: u32) -> f64 {
        let mut s = (1.0 + freq as f64).ln();
        if let Some(p) = prev {
            if let Some(&n) = self.bigrams.get(&(p.to_string(), word.to_string())) {
                s += BIGRAM_BONUS + (1.0 + n as f64).ln();
            }
        }
        s
    }

    /// Up to `n` suggestions for the word being typed (`prefix`, lower-case),
    /// given the word before it. An empty prefix predicts the next word from
    /// the bigrams alone.
    pub fn suggest(&self, prev: Option<&str>, prefix: &str, n: usize) -> Vec<String> {
        let prev = prev.map(str::to_lowercase);
        let prev = prev.as_deref();
        let prefix = prefix.to_lowercase();
        let mut scored: Vec<(f64, String)> = if prefix.is_empty() {
            let Some(p) = prev else { return vec![] };
            self.bigrams
                .iter()
                .filter(|((a, _), _)| a == p)
                .map(|((_, b), _)| (self.score(prev, b, self.trie.freq(b).unwrap_or(1)), b.clone()))
                .collect()
        } else {
            self.trie.complete(&prefix, 60).into_iter().map(|(w, f)| (self.score(prev, &w, f), w)).collect()
        };
        scored.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        scored.into_iter().map(|(_, w)| w).take(n).collect()
    }

    /// The correction for a mistyped `word` (not a word, at least 3 letters):
    /// the best-scoring word one edit away that starts with the same letter.
    pub fn correct(&self, prev: Option<&str>, word: &str) -> Option<String> {
        let lower = word.to_lowercase();
        if lower.chars().count() < 3 || self.trie.freq(&lower).is_some() {
            return None;
        }
        let prev = prev.map(str::to_lowercase);
        let mut best: Option<(f64, String)> = None;
        for cand in edits1(&lower) {
            let Some(f) = self.trie.freq(&cand) else { continue };
            let s = self.score(prev.as_deref(), &cand, f);
            if best.as_ref().map_or(true, |(b, w)| s > *b || (s == *b && cand < *w)) {
                best = Some((s, cand));
            }
        }
        best.map(|(_, w)| match_case(word, &w))
    }
}

impl Default for Predictor {
    fn default() -> Self {
        Predictor::new()
    }
}

/// The process-wide predictor, built on first use and shared by every
/// keyboard so learned words and pairs carry across apps.
#[cfg(not(test))]
pub fn shared() -> &'static Mutex<Predictor> {
    static P: OnceLock<Mutex<Predictor>> = OnceLock::new();
    P.get_or_init(|| {
        let (en, is) = lists_from(&package_words());
        Mutex::new(Predictor::with_lists(&en, &is))
    })
}

/// Tests run on their own threads; a predictor per thread keeps words one test learns (or has
/// corrected back) from changing what another test sees.
#[cfg(test)]
pub fn shared() -> &'static Mutex<Predictor> {
    thread_local! {
        static P: &'static Mutex<Predictor> = Box::leak(Box::new(Mutex::new({
            let (en, is) = lists_from(&package_words());
            Predictor::with_lists(&en, &is)
        })));
    }
    P.with(|p| *p)
}

/// Every string one edit from `word` that keeps its first letter: deletion,
/// transposition, substitution and insertion after the first letter.
pub fn edits1(word: &str) -> Vec<String> {
    let c: Vec<char> = word.chars().collect();
    let mut out = vec![];
    for i in 1..c.len() {
        let mut d = c.clone();
        d.remove(i);
        out.push(d.into_iter().collect());
    }
    for i in 1..c.len().saturating_sub(1) {
        let mut t = c.clone();
        t.swap(i, i + 1);
        out.push(t.into_iter().collect());
    }
    for i in 1..c.len() {
        for a in ALPHABET.chars().filter(|a| *a != c[i]) {
            let mut s = c.clone();
            s[i] = a;
            out.push(s.into_iter().collect());
        }
    }
    for i in 1..=c.len() {
        for a in ALPHABET.chars() {
            let mut s = c.clone();
            s.insert(i, a);
            out.push(s.into_iter().collect());
        }
    }
    out
}

/// `word` in the capitalisation `typed` used: Capitalised or ALL CAPS follow.
pub fn match_case(typed: &str, word: &str) -> String {
    let mut chars = typed.chars();
    let first_upper = chars.next().is_some_and(char::is_uppercase);
    let rest: Vec<char> = chars.collect();
    if first_upper && !rest.is_empty() && rest.iter().all(|c| c.is_uppercase()) {
        return word.to_uppercase();
    }
    if first_upper {
        let mut w = word.chars();
        return w.next().map(|f| f.to_uppercase().chain(w).collect()).unwrap_or_default();
    }
    word.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST: &str = "the 1000\nthen 300\nthere 400\nthey 500\nhello 200\nhelp 250\nworld 150\nhvað 90\nhver 80\n";

    fn small() -> Predictor {
        let mut p = Predictor::from_lists(&[(LIST, 1)]);
        p.learn("hello", "world");
        p
    }

    #[test]
    fn trie_finds_words_and_completes_by_frequency() {
        let mut t = Trie::default();
        for (w, f) in [("the", 10), ("then", 3), ("there", 4), ("they", 5), ("a", 7)] {
            t.insert(w, f);
        }
        assert_eq!(t.freq("then"), Some(3));
        assert_eq!(t.freq("th"), None, "a prefix is not a word");
        assert_eq!(t.freq("zebra"), None);
        let top: Vec<_> = t.complete("th", 3).into_iter().map(|(w, _)| w).collect();
        assert_eq!(top, ["the", "they", "there"]);
        assert_eq!(t.complete("the", 10).len(), 4, "the prefix itself is included");
        assert!(t.complete("x", 3).is_empty());
        t.insert("then", 1);
        assert_eq!(t.freq("then"), Some(3), "a lower count never lowers it");
        t.insert("hvað", 2);
        assert_eq!(t.complete("hv", 1), [("hvað".to_string(), 2)], "non-ASCII letters work");
    }

    #[test]
    fn suggestions_rank_by_frequency_with_a_bigram_bonus() {
        let p = small();
        assert_eq!(p.suggest(None, "the", 3), ["the", "they", "there"]);
        assert_eq!(p.suggest(None, "he", 3), ["help", "hello"], "only words starting with he");
        assert_eq!(p.suggest(None, "w", 3), ["world"]);
        let mut p = small();
        for _ in 0..3 {
            p.learn("say", "then");
        }
        assert_eq!(p.suggest(Some("say"), "the", 3)[0], "then", "the pair outranks plain frequency");
        assert_eq!(p.suggest(Some("Say"), "T", 1)[0], "then", "case is ignored");
        assert_eq!(p.suggest(Some("hello"), "", 3), ["world"], "next word from the pairs");
        assert!(p.suggest(None, "", 3).is_empty());
    }

    #[test]
    fn autocorrect_is_one_edit_with_the_same_first_letter() {
        let p = small();
        assert_eq!(p.correct(None, "hte"), None, "a transposition at the first letter changes it");
        assert_eq!(p.correct(None, "teh"), Some("the".to_string()), "a later transposition is one edit");
        assert_eq!(p.correct(None, "thr"), Some("the".to_string()), "insertion keeps the first letter");
        assert_eq!(p.correct(None, "thney"), Some("they".to_string()), "deletion");
        assert_eq!(p.correct(None, "helo"), Some("help".to_string()), "the more frequent of hello and help");
        assert_eq!(p.correct(None, "wordl"), Some("world".to_string()), "transposition");
        assert_eq!(p.correct(None, "xorld"), None, "never changes the first letter");
        assert_eq!(p.correct(None, "the"), None, "real words are left alone");
        assert_eq!(p.correct(None, "zz"), None, "short words are left alone");
        assert_eq!(p.correct(None, "hvad"), Some("hvað".to_string()), "Icelandic letters");
    }

    #[test]
    fn autocorrect_keeps_capitals_and_uses_the_previous_word() {
        let p = small();
        assert_eq!(p.correct(None, "Thr"), Some("The".to_string()));
        assert_eq!(p.correct(None, "THR"), Some("THE".to_string()));
        assert_eq!(match_case("hello", "world"), "world");
        let mut p = small();
        p.learn("see", "then");
        assert_eq!(p.correct(Some("see"), "thex"), Some("then".to_string()), "the bigram breaks the tie");
    }

    #[test]
    fn typos_are_not_learned_but_added_words_are() {
        let mut p = small();
        p.learn("the", "thx");
        assert!(!p.is_word("thx"), "learning skips unknown words");
        let mut p = small();
        assert_eq!(p.correct(None, "helq"), Some("help".to_string()));
        p.add_word("helq");
        assert!(p.is_word("Helq"));
        assert_eq!(p.correct(None, "helq"), None);
    }

    #[test]
    fn match_case_handles_unicode_and_single_letters() {
        assert_eq!(match_case("Ég", "ég"), "Ég");
        assert_eq!(match_case("I", "it"), "It");
        assert_eq!(match_case("", "x"), "x");
    }

    #[test]
    fn the_bundled_lists_load_and_cover_both_languages() {
        let p = Predictor::new();
        for w in ["the", "hello", "keyboard", "að", "takk", "hvernig"] {
            assert!(p.is_word(w), "{w}");
        }
        assert!(p.suggest(None, "thi", 3).contains(&"this".to_string()));
        assert_eq!(p.suggest(Some("thank"), "", 1), ["you"]);
    }

    #[test]
    fn package_lists_override_per_language_and_reload_keeps_learning() {
        let dir = std::env::temp_dir().join(format!("system-pkgs-words-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("en.txt"), "zebrafish 900\nthe 5\n").unwrap();
        std::fs::write(dir.join("is.txt"), "garbage\n").unwrap();
        let (en, is) = lists_from(&dir);
        assert!(en.starts_with("zebrafish") && is == IS, "a list without words falls back to the compiled-in one");
        let mut p = Predictor::new();
        assert!(!p.is_word("zebrafish"));
        p.add_word("Snorkelwort");
        p.learn("thank", "you");
        p.reload(&en, &is);
        assert!(p.is_word("zebrafish") && p.is_word("snorkelwort") && p.is_word("takk"));
        assert!(!p.is_word("hello"), "the package English list replaced the bundled one");
        assert!(lists_from(&dir.join("missing")).0 == EN);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
