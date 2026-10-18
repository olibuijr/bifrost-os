//! Word models for predictive text and word splitting.
//!
//! Two parts, both local and trained only on text the user types or on word lists the system ships:
//!
//! - [`WordModel`]: unigram and bigram counts for languages written with spaces. It ranks completions for the word
//!   being typed and predicts the next word, using the previous word when there is one. Counts live in memory and
//!   round-trip through a plain text form ([`WordModel::to_text`], [`WordModel::from_text`]) so the owner can keep
//!   them in a preference file. Nothing leaves the device.
//! - [`Lexicon`] with [`segment`]: word splitting for Chinese, Japanese and Korean text, which has no spaces between
//!   words. The split is the lowest-cost path through the lexicon (the cost of a word is the negative log of its
//!   relative count), so common words beat rare ones and unknown characters fall back to single characters. Runs of
//!   unknown kana or Hangul stay together as one token. Characters that are not CJK are split into words the way
//!   spell checking does (alphanumeric runs, with an apostrophe kept inside a word).
//!
//! The lexicon holds no words of its own: a system word list is loaded with [`Lexicon::load_text`].
//! Thai, Lao, Khmer and Myanmar text is not split here.

use crate::script::{script_of, Script};
use std::collections::HashMap;
use std::ops::Range;

/// The most distinct words a [`WordModel`] keeps before it drops the words seen only once.
pub const MAX_VOCABULARY: usize = 50_000;

/// How much of a prediction's weight comes from the previous word when the model has seen it.
const BIGRAM_WEIGHT: f64 = 0.7;

/// Unigram and bigram counts over lower-cased words.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WordModel {
    unigrams: HashMap<String, u32>,
    /// previous word -> (next word -> count)
    bigrams: HashMap<String, HashMap<String, u32>>,
    total: u64,
}

/// Lower-cased words of `text` with a flag marking the first word of each sentence (after `.`, `!`, `?` or a line
/// break), where the previous word must not carry over.
fn tokens(text: &str) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    let mut word = String::new();
    let mut fresh = true;
    let chars: Vec<char> = text.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        let inner = (c == '\'' || c == '’')
            && !word.is_empty()
            && chars.get(i + 1).is_some_and(|n| n.is_alphabetic());
        if c.is_alphanumeric() || inner {
            word.extend(c.to_lowercase());
        } else {
            if !word.is_empty() {
                out.push((std::mem::take(&mut word), fresh));
                fresh = false;
            }
            if matches!(c, '.' | '!' | '?' | '\n' | '\r') {
                fresh = true;
            }
        }
    }
    if !word.is_empty() {
        out.push((word, fresh));
    }
    out
}

impl WordModel {
    /// An empty model.
    pub fn new() -> WordModel {
        WordModel::default()
    }

    /// Counts the words and word pairs of `text`.
    pub fn train(&mut self, text: &str) {
        let mut prev: Option<String> = None;
        for (word, fresh) in tokens(text) {
            if fresh {
                prev = None;
            }
            *self.unigrams.entry(word.clone()).or_insert(0) += 1;
            self.total += 1;
            if let Some(p) = prev.take() {
                *self.bigrams.entry(p).or_default().entry(word.clone()).or_insert(0) += 1;
            }
            prev = Some(word);
        }
        if self.unigrams.len() > MAX_VOCABULARY {
            self.prune_singletons();
        }
    }

    /// Drops every word seen once, and the pairs that mention one.
    pub fn prune_singletons(&mut self) {
        self.unigrams.retain(|_, n| *n > 1);
        let known = &self.unigrams;
        self.bigrams.retain(|p, _| known.contains_key(p));
        for next in self.bigrams.values_mut() {
            next.retain(|w, _| known.contains_key(w));
        }
        self.bigrams.retain(|_, next| !next.is_empty());
        self.total = self.unigrams.values().map(|&n| n as u64).sum();
    }

    /// How often `word` was seen.
    pub fn count(&self, word: &str) -> u32 {
        self.unigrams.get(&word.to_lowercase()).copied().unwrap_or(0)
    }

    /// How many distinct words the model knows.
    pub fn vocabulary(&self) -> usize {
        self.unigrams.len()
    }

    /// The probability of `word`, given the word before it when there is one: the previous word's pair weight blended
    /// with the plain word frequency. A word the model never saw has probability 0.
    pub fn probability(&self, prev: Option<&str>, word: &str) -> f64 {
        let word = word.to_lowercase();
        let uni = match self.unigrams.get(&word) {
            Some(&n) if self.total > 0 => n as f64 / self.total as f64,
            _ => return 0.0,
        };
        let pair = prev.and_then(|p| self.bigrams.get(&p.to_lowercase())).and_then(|next| {
            let sum: u64 = next.values().map(|&n| n as u64).sum();
            next.get(&word).map(|&n| n as f64 / sum as f64)
        });
        match pair {
            Some(b) => BIGRAM_WEIGHT * b + (1.0 - BIGRAM_WEIGHT) * uni,
            None => uni,
        }
    }

    /// The words to offer for what is being typed: `prefix` is the word so far (empty to predict the next word) and
    /// `prev` the word before it. Words that follow `prev` in the model come first, most frequent pair first; the
    /// rest follow by word frequency. A word equal to the prefix is not offered. At most `limit` words.
    pub fn predict(&self, prev: Option<&str>, prefix: &str, limit: usize) -> Vec<String> {
        let prefix = prefix.to_lowercase();
        let mut out: Vec<String> = Vec::new();
        if limit == 0 {
            return out;
        }
        let fits = |w: &str| w.starts_with(&prefix) && w != prefix;
        if let Some(next) = prev.and_then(|p| self.bigrams.get(&p.to_lowercase())) {
            let mut pairs: Vec<(&String, u32)> = next.iter().filter(|(w, _)| fits(w)).map(|(w, &n)| (w, n)).collect();
            pairs.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
            out.extend(pairs.into_iter().map(|(w, _)| w.clone()));
        }
        let mut rest: Vec<(&String, u32)> = self
            .unigrams
            .iter()
            .filter(|(w, _)| fits(w) && !out.contains(*w))
            .map(|(w, &n)| (w, n))
            .collect();
        rest.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
        out.extend(rest.into_iter().map(|(w, _)| w.clone()));
        out.truncate(limit);
        out
    }

    /// The model as text, one line per entry: `u<TAB>word<TAB>count` and `b<TAB>previous<TAB>word<TAB>count`,
    /// sorted so the same model always gives the same text.
    pub fn to_text(&self) -> String {
        let mut lines: Vec<String> = self.unigrams.iter().map(|(w, n)| format!("u\t{w}\t{n}")).collect();
        for (p, next) in &self.bigrams {
            for (w, n) in next {
                lines.push(format!("b\t{p}\t{w}\t{n}"));
            }
        }
        lines.sort();
        let mut text = lines.join("\n");
        if !text.is_empty() {
            text.push('\n');
        }
        text
    }

    /// Reads the text form back. Lines that do not parse are skipped, so a damaged file loads what it still has.
    pub fn from_text(text: &str) -> WordModel {
        let mut m = WordModel::new();
        for line in text.lines() {
            let f: Vec<&str> = line.split('\t').collect();
            match f.as_slice() {
                ["u", w, n] if !w.is_empty() => {
                    if let Ok(n) = n.parse::<u32>() {
                        *m.unigrams.entry((*w).to_string()).or_insert(0) += n;
                    }
                }
                ["b", p, w, n] if !p.is_empty() && !w.is_empty() => {
                    if let Ok(n) = n.parse::<u32>() {
                        *m.bigrams.entry((*p).to_string()).or_default().entry((*w).to_string()).or_insert(0) += n;
                    }
                }
                _ => {}
            }
        }
        m.total = m.unigrams.values().map(|&n| n as u64).sum();
        m
    }
}

/// Words with counts, for splitting text that has no spaces between words.
#[derive(Clone, Debug, Default)]
pub struct Lexicon {
    words: HashMap<String, u32>,
    total: u64,
    /// the longest word, in characters
    longest: usize,
}

impl Lexicon {
    /// An empty lexicon.
    pub fn new() -> Lexicon {
        Lexicon::default()
    }

    /// Adds `count` sightings of `word`. An empty word or a zero count is ignored.
    pub fn add(&mut self, word: &str, count: u32) {
        if word.is_empty() || count == 0 {
            return;
        }
        *self.words.entry(word.to_string()).or_insert(0) += count;
        self.total += count as u64;
        self.longest = self.longest.max(word.chars().count());
    }

    /// Loads `word<TAB>count` lines (a line with just a word counts as 1). Blank lines and lines starting with `#`
    /// are skipped. Returns how many words were added.
    pub fn load_text(&mut self, text: &str) -> usize {
        let mut added = 0;
        for line in text.lines() {
            let line = line.trim_end_matches(['\r', '\n']);
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (word, count) = match line.split_once('\t') {
                Some((w, c)) => (w, c.trim().parse::<u32>().unwrap_or(1)),
                None => (line, 1),
            };
            let before = self.words.len();
            self.add(word, count);
            if self.words.len() > before {
                added += 1;
            }
        }
        added
    }

    /// Whether `word` is in the lexicon.
    pub fn contains(&self, word: &str) -> bool {
        self.words.contains_key(word)
    }

    /// How many distinct words the lexicon holds.
    pub fn len(&self) -> usize {
        self.words.len()
    }

    /// Whether the lexicon holds no words.
    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
    }

    /// The cost of a known word: the negative log of its share of all counts.
    fn cost(&self, word: &str) -> Option<f64> {
        self.words.get(word).map(|&n| -((n as f64) / (self.total.max(1) as f64)).ln())
    }

    /// The cost of one character the lexicon does not know. It is higher than any known word's cost, so a known
    /// word always beats the same characters taken one by one.
    fn unknown_cost(&self) -> f64 {
        ((self.total + 1) as f64).ln() + 2.0
    }
}

/// Whether `c` is written in a script that runs words together.
fn is_cjk(c: char) -> bool {
    matches!(script_of(c), Some(Script::Han | Script::Japanese | Script::Korean))
}

/// Whether unknown neighbours of this script should be kept as one token (kana and Hangul come in runs).
fn joins_when_unknown(c: char) -> Option<Script> {
    match script_of(c) {
        Some(s @ (Script::Japanese | Script::Korean)) => Some(s),
        _ => None,
    }
}

/// The lowest-cost split of one run of CJK characters starting at byte `base` of the source text.
fn split_cjk(run: &str, base: usize, lex: &Lexicon, out: &mut Vec<Range<usize>>) {
    let at: Vec<usize> = run.char_indices().map(|(i, _)| i).chain(std::iter::once(run.len())).collect();
    let n = at.len() - 1;
    let unknown = lex.unknown_cost();
    // best[i]: the cheapest cost of splitting the first i characters; back[i]: (start of the last token, known?)
    let mut best = vec![f64::INFINITY; n + 1];
    let mut back = vec![(0usize, false); n + 1];
    best[0] = 0.0;
    for i in 0..n {
        let from = best[i];
        let step = unknown + from;
        if step < best[i + 1] {
            best[i + 1] = step;
            back[i + 1] = (i, false);
        }
        for len in 1..=lex.longest.min(n - i) {
            if let Some(c) = lex.cost(&run[at[i]..at[i + len]]) {
                if from + c < best[i + len] {
                    best[i + len] = from + c;
                    back[i + len] = (i, true);
                }
            }
        }
    }
    let mut tokens: Vec<(usize, usize, bool)> = Vec::new();
    let mut end = n;
    while end > 0 {
        let (start, known) = back[end];
        tokens.push((start, end, known));
        end = start;
    }
    tokens.reverse();
    // join neighbouring unknown kana or Hangul characters of one script into one token
    let mut merged: Vec<(usize, usize, bool)> = Vec::new();
    for t in tokens {
        if let Some(last) = merged.last_mut() {
            let a = run[at[last.0]..].chars().next().and_then(joins_when_unknown);
            let b = run[at[t.0]..].chars().next().and_then(joins_when_unknown);
            if !last.2 && !t.2 && a.is_some() && a == b {
                last.1 = t.1;
                continue;
            }
        }
        merged.push(t);
    }
    out.extend(merged.into_iter().map(|(s, e, _)| base + at[s]..base + at[e]));
}

/// The word ranges (byte offsets into `text`, in order) of `text`. Spaces, punctuation and symbols separate words and
/// belong to none. CJK stretches are split with `lex`; other text is split into alphanumeric words with an apostrophe
/// kept between letters.
pub fn segment(text: &str, lex: &Lexicon) -> Vec<Range<usize>> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let (p, c) = chars[i];
        if is_cjk(c) {
            let mut j = i;
            while j < chars.len() && is_cjk(chars[j].1) {
                j += 1;
            }
            let end = chars.get(j).map_or(text.len(), |x| x.0);
            split_cjk(&text[p..end], p, lex, &mut out);
            i = j;
        } else if c.is_alphanumeric() {
            let mut j = i + 1;
            while j < chars.len() {
                let d = chars[j].1;
                let inner = (d == '\'' || d == '’') && chars.get(j + 1).is_some_and(|n| n.1.is_alphabetic() && !is_cjk(n.1));
                if (d.is_alphanumeric() && !is_cjk(d)) || inner {
                    j += 1;
                } else {
                    break;
                }
            }
            let end = chars.get(j).map_or(text.len(), |x| x.0);
            out.push(p..end);
            i = j;
        } else {
            i += 1;
        }
    }
    out
}

/// The words of `text` as strings; see [`segment`].
pub fn split_words<'a>(text: &'a str, lex: &Lexicon) -> Vec<&'a str> {
    segment(text, lex).into_iter().map(|r| &text[r]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trained() -> WordModel {
        let mut m = WordModel::new();
        m.train("The cat sat on the mat. The cat ran. A dog sat.");
        m
    }

    #[test]
    fn the_model_counts_words_in_lower_case() {
        let m = trained();
        assert_eq!(m.count("the"), 3);
        assert_eq!(m.count("The"), 3);
        assert_eq!(m.count("cat"), 2);
        assert_eq!(m.count("zebra"), 0);
        assert_eq!(m.vocabulary(), 8);
    }

    #[test]
    fn the_word_after_the_previous_word_ranks_first() {
        let m = trained();
        assert_eq!(m.predict(Some("the"), "", 3), vec!["cat", "mat", "the"]);
        // "ran" and "sat" each follow "cat" once, so they tie and go alphabetically
        assert_eq!(m.predict(Some("cat"), "", 2), vec!["ran", "sat"]);
    }

    #[test]
    fn a_prefix_narrows_the_words_and_never_offers_itself() {
        let m = trained();
        assert_eq!(m.predict(None, "ca", 5), vec!["cat"]);
        assert_eq!(m.predict(None, "cat", 5), Vec::<String>::new());
        assert_eq!(m.predict(Some("the"), "m", 5), vec!["mat"]);
        assert_eq!(m.predict(Some("zzz"), "s", 5), vec!["sat"]);
        assert!(m.predict(None, "", 0).is_empty());
    }

    #[test]
    fn a_sentence_end_breaks_the_word_pair() {
        let m = trained();
        // "...the mat. The cat...": nothing follows "mat", and "sat." leaves no pair with "a"
        assert!(m.bigrams.get("mat").is_none());
        assert!(m.bigrams.get("ran").is_none());
        assert!(m.bigrams.get("sat").is_some_and(|n| n.contains_key("on") && !n.contains_key("a")));
    }

    #[test]
    fn a_seen_pair_is_likelier_than_the_word_alone() {
        let m = trained();
        assert!(m.probability(Some("the"), "cat") > m.probability(None, "cat"));
        assert!(m.probability(None, "the") > m.probability(None, "dog"));
        assert_eq!(m.probability(None, "zebra"), 0.0);
        assert_eq!(WordModel::new().probability(None, "the"), 0.0);
    }

    #[test]
    fn apostrophes_stay_inside_words() {
        let mut m = WordModel::new();
        m.train("don't stop, it’s fine");
        assert_eq!(m.count("don't"), 1);
        assert_eq!(m.count("it’s"), 1);
    }

    #[test]
    fn the_text_form_round_trips() {
        let m = trained();
        let back = WordModel::from_text(&m.to_text());
        assert_eq!(back, m);
        assert_eq!(back.to_text(), m.to_text());
        assert_eq!(WordModel::new().to_text(), "");
    }

    #[test]
    fn damaged_lines_are_skipped_when_loading() {
        let m = WordModel::from_text("u\tcat\t3\nnonsense\nu\tdog\tx\nb\tcat\n\nb\tthe\tcat\t2\n");
        assert_eq!(m.count("cat"), 3);
        assert_eq!(m.count("dog"), 0);
        assert_eq!(m.predict(Some("the"), "", 2), vec!["cat"]);
    }

    #[test]
    fn pruning_drops_singletons_and_their_pairs() {
        let mut m = WordModel::new();
        m.train("go home. go home. go away.");
        m.prune_singletons();
        assert_eq!(m.count("away"), 0);
        assert_eq!(m.count("home"), 2);
        assert_eq!(m.predict(Some("go"), "", 5), vec!["home", "go"]);
    }

    fn chinese() -> Lexicon {
        let mut lex = Lexicon::new();
        for (w, n) in [("我", 50), ("爱", 40), ("北京", 30), ("天安门", 20), ("北", 5), ("京", 5), ("天安", 3), ("门", 10)] {
            lex.add(w, n);
        }
        lex
    }

    #[test]
    fn chinese_text_splits_into_the_likeliest_words() {
        let text = "我爱北京天安门";
        assert_eq!(split_words(text, &chinese()), vec!["我", "爱", "北京", "天安门"]);
    }

    #[test]
    fn unknown_han_characters_stand_alone() {
        assert_eq!(split_words("我爱猫", &chinese()), vec!["我", "爱", "猫"]);
    }

    #[test]
    fn japanese_text_splits_and_unknown_kana_stay_together() {
        let mut lex = Lexicon::new();
        lex.load_text("私\t30\nは\t60\n東京\t20\nに\t50\n行く\t15\n");
        assert_eq!(split_words("私は東京に行く", &lex), vec!["私", "は", "東京", "に", "行く"]);
        // ですね is not in the lexicon: one token, not three
        assert_eq!(split_words("私ですね", &lex), vec!["私", "ですね"]);
    }

    #[test]
    fn korean_words_split_at_spaces_and_unknown_hangul_stays_whole() {
        let lex = Lexicon::new();
        assert_eq!(split_words("한국어 사전", &lex), vec!["한국어", "사전"]);
    }

    #[test]
    fn mixed_text_keeps_latin_words_digits_and_apostrophes() {
        let mut lex = Lexicon::new();
        lex.add("世界", 10);
        assert_eq!(split_words("Hello 世界 2024, don't", &lex), vec!["Hello", "世界", "2024", "don't"]);
        let ranges = segment("a 世界", &lex);
        assert_eq!(ranges, vec![0..1, 2..8]);
    }

    #[test]
    fn punctuation_separates_cjk_words() {
        let text = "我爱,北京。";
        assert_eq!(split_words(text, &chinese()), vec!["我", "爱", "北京"]);
    }

    #[test]
    fn an_empty_lexicon_and_empty_text_are_safe() {
        assert!(segment("", &chinese()).is_empty());
        assert_eq!(split_words("北京", &Lexicon::new()), vec!["北", "京"]);
        assert!(Lexicon::new().is_empty());
    }

    #[test]
    fn the_lexicon_loads_counts_and_skips_comments() {
        let mut lex = Lexicon::new();
        assert_eq!(lex.load_text("# words\n北京\t30\n\n天安门\n北京\t2\n"), 2);
        assert!(lex.contains("北京"));
        assert_eq!(lex.len(), 2);
    }
}
