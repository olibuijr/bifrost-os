//! System search ranking: the tokenizer, a BM25 index, the hybrid ranker, local usage counts and the merging runtime
//! that the Search panel uses. Pure data and plain threads; the caller stores the usage text.
//!
//! * [`tokenize`] splits text into words close to Unicode word segmentation (UAX #29, simplified): letters and digits
//!   form a word, an apostrophe between two letters joins a word, every other character separates. Words are case and
//!   diacritic folded ("Café" is "cafe", "Straße" is "strasse"). A run of CJK ideographs, kana or Hangul is cut into
//!   overlapping pairs ("東京都" is "東京", "京都"); a lone character stays itself.
//! * [`index_terms`] is what an index stores for a text: the tokens, the single characters inside CJK pairs (so a
//!   one-character query finds them) and the prefixes of each Latin word from two characters up (so "mai" finds
//!   "mail" as you type). Queries use [`tokenize`] only.
//! * [`Bm25`] scores documents over weighted fields (k1 1.2, b 0.75). [`Bm25::scores`] also returns the share of the
//!   distinct query words a document holds, so a document with every word beats one with a single word.
//! * [`rank`] combines the BM25 score with the title match, the kind of item (only for short queries), recency, the
//!   local open count and an optional semantic score from a [`SemanticHook`]. The shell passes [`NoSemantic`]: no local
//!   text model is bundled, so the hook is a seam, not a score.
//! * [`run`] queries several [`Source`]s on their own threads, waits at most a deadline, and merges what arrived into
//!   sections (in source order) plus one top hit. A source that is late is left out and named in `missing`.
//!
//! Limits: BM25 is over the items handed in (a candidate set, not a persistent index); recency is an explicit Unix
//! time per item (0 means unknown and adds nothing); usage counts are kept only on this machine.

use std::collections::BTreeMap;
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

const K1: f32 = 1.2;
const B: f32 = 0.75;
/// Field weight of a title (a title word counts this many times).
pub const TITLE_WEIGHT: f32 = 3.0;
/// Field weight of the keywords and description.
pub const TEXT_WEIGHT: f32 = 1.0;
/// The longest prefix an index stores for a Latin word.
const PREFIX_MAX: usize = 12;
/// Recency halves every `RECENCY_DAYS * ln 2` days.
const RECENCY_DAYS: f32 = 14.0;
const W_BM25: f32 = 0.45;
const W_TITLE: f32 = 0.25;
const W_PRIOR: f32 = 0.10;
const W_RECENCY: f32 = 0.08;
const W_FREQUENCY: f32 = 0.07;
const W_SEMANTIC: f32 = 0.25;

fn is_combining(c: char) -> bool {
    ('\u{300}'..='\u{36f}').contains(&c)
}

fn is_apostrophe(c: char) -> bool {
    c == '\'' || c == '\u{2019}'
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || is_combining(c)
}

/// Han ideographs, hiragana, katakana and Hangul syllables: scripts written without spaces between words.
pub fn is_cjk(c: char) -> bool {
    matches!(c as u32, 0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF | 0x20000..=0x2FA1F)
}

/// The plain letters a lowercase accented letter folds to.
fn base_letters(c: char) -> Option<&'static str> {
    let s = match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' => "a",
        'æ' => "ae",
        'ç' | 'ć' | 'č' | 'ĉ' | 'ċ' => "c",
        'ď' | 'đ' | 'ð' => "d",
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ė' | 'ę' | 'ě' => "e",
        'ğ' | 'ĝ' | 'ġ' | 'ģ' => "g",
        'ì' | 'í' | 'î' | 'ï' | 'ī' | 'į' | 'ı' => "i",
        'ł' | 'ĺ' | 'ľ' => "l",
        'ñ' | 'ń' | 'ň' | 'ņ' => "n",
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ő' => "o",
        'œ' => "oe",
        'ŕ' | 'ř' => "r",
        'ś' | 'š' | 'ş' => "s",
        'ß' => "ss",
        'þ' => "th",
        'ť' | 'ţ' => "t",
        'ù' | 'ú' | 'û' | 'ü' | 'ū' | 'ů' | 'ű' | 'ų' => "u",
        'ý' | 'ÿ' => "y",
        'ź' | 'ż' | 'ž' => "z",
        _ => return None,
    };
    Some(s)
}

/// Lowercases a Latin word, drops combining marks and apostrophes, and folds accented letters to plain ones.
fn fold_word(word: &str) -> String {
    let mut out = String::with_capacity(word.len());
    for c in word.chars().flat_map(char::to_lowercase) {
        if is_combining(c) || is_apostrophe(c) {
            continue;
        }
        match base_letters(c) {
            Some(s) => out.push_str(s),
            None => out.push(c),
        }
    }
    out
}

/// The words of `text`, folded, in order. See the module notes for the rules.
pub fn tokenize(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if !is_word_char(chars[i]) {
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() {
            let c = chars[i];
            let joins = is_apostrophe(c) && i > start && chars.get(i + 1).map_or(false, |&n| is_word_char(n));
            if is_word_char(c) || joins {
                i += 1;
            } else {
                break;
            }
        }
        push_run(&chars[start..i], &mut out);
    }
    out
}

/// Splits one run of word characters: Latin stretches are folded words, CJK stretches become pairs.
fn push_run(run: &[char], out: &mut Vec<String>) {
    let mut latin = String::new();
    let mut k = 0;
    while k < run.len() {
        if is_cjk(run[k]) {
            push_latin(&mut latin, out);
            let mut j = k;
            while j < run.len() && is_cjk(run[j]) {
                j += 1;
            }
            let seg = &run[k..j];
            if seg.len() == 1 {
                out.push(seg[0].to_string());
            } else {
                out.extend(seg.windows(2).map(|w| w.iter().collect::<String>()));
            }
            k = j;
        } else {
            latin.push(run[k]);
            k += 1;
        }
    }
    push_latin(&mut latin, out);
}

fn push_latin(latin: &mut String, out: &mut Vec<String>) {
    if !latin.is_empty() {
        let folded = fold_word(latin);
        if !folded.is_empty() {
            out.push(folded);
        }
        latin.clear();
    }
}

/// The terms an index stores for `text`: [`tokenize`] output, the single characters of CJK pairs, and the prefixes
/// (two characters up to the word less one, at most [`PREFIX_MAX`]) of each Latin word.
pub fn index_terms(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for token in tokenize(text) {
        let chars: Vec<char> = token.chars().collect();
        if chars.iter().all(|&c| is_cjk(c)) {
            out.push(token.clone());
            if chars.len() == 2 {
                out.extend(chars.iter().map(|c| c.to_string()));
            }
            continue;
        }
        let n = chars.len();
        out.push(token.clone());
        let top = (n - 1).min(PREFIX_MAX);
        for len in 2..=top {
            out.push(chars[..len].iter().collect());
        }
    }
    out
}

/// BM25 over weighted fields. Documents are numbered in the order they were added.
#[derive(Debug, Default)]
pub struct Bm25 {
    /// weighted length of each document
    lengths: Vec<f32>,
    /// term -> (document, weighted frequency), by document
    postings: BTreeMap<String, Vec<(usize, f32)>>,
}

impl Bm25 {
    /// Adds a document made of `(text, weight)` fields and returns its index.
    pub fn add(&mut self, fields: &[(&str, f32)]) -> usize {
        let doc = self.lengths.len();
        let mut tf: BTreeMap<String, f32> = BTreeMap::new();
        let mut len = 0.0f32;
        for &(text, weight) in fields {
            for term in index_terms(text) {
                *tf.entry(term).or_insert(0.0) += weight;
                len += weight;
            }
        }
        self.lengths.push(len);
        for (term, f) in tf {
            self.postings.entry(term).or_default().push((doc, f));
        }
        doc
    }

    /// Number of documents.
    pub fn len(&self) -> usize {
        self.lengths.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lengths.is_empty()
    }

    /// Every document holding a query term, as (document, BM25 score, share of the distinct query terms it holds).
    /// Not sorted.
    pub fn scores(&self, terms: &[String]) -> Vec<(usize, f32, f32)> {
        let mut uniq: Vec<&str> = terms.iter().map(String::as_str).collect();
        uniq.sort_unstable();
        uniq.dedup();
        if uniq.is_empty() || self.lengths.is_empty() {
            return vec![];
        }
        let n = self.lengths.len() as f32;
        let avg = (self.lengths.iter().sum::<f32>() / n).max(f32::MIN_POSITIVE);
        let mut acc: BTreeMap<usize, (f32, usize)> = BTreeMap::new();
        for term in &uniq {
            let Some(post) = self.postings.get(*term) else { continue };
            let df = post.len() as f32;
            let idf = ((n - df + 0.5) / (df + 0.5) + 1.0).ln();
            for &(doc, tf) in post {
                let norm = K1 * (1.0 - B + B * self.lengths[doc] / avg);
                let e = acc.entry(doc).or_insert((0.0, 0));
                e.0 += idf * tf * (K1 + 1.0) / (tf + norm);
                e.1 += 1;
            }
        }
        let k = uniq.len() as f32;
        acc.into_iter().map(|(doc, (s, m))| (doc, s, m as f32 / k)).collect()
    }
}

/// What an item is. Short queries prefer apps, then settings pages, contacts, documents and files.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    App,
    Settings,
    Contact,
    Document,
    File,
    Other,
}

impl Kind {
    fn prior(self) -> f32 {
        match self {
            Kind::App => 1.0,
            Kind::Settings => 0.8,
            Kind::Contact => 0.6,
            Kind::Document => 0.5,
            Kind::File => 0.4,
            Kind::Other => 0.2,
        }
    }
}

/// A searchable item from one source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    /// unique within the usage counts (sources prefix their own ids, for example "app/unique")
    pub id: String,
    pub title: String,
    /// keywords and description, searched at a lower weight than the title
    pub text: String,
    pub kind: Kind,
    /// Unix seconds the item was made or indexed; 0 when unknown
    pub modified: u64,
}

impl Item {
    pub fn new(id: &str, title: &str, kind: Kind) -> Item {
        Item { id: id.to_string(), title: title.to_string(), text: String::new(), kind, modified: 0 }
    }
}

/// How often each item was opened on this machine. Kept locally only; it is never sent anywhere.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    counts: BTreeMap<String, u32>,
}

impl Usage {
    pub fn bump(&mut self, id: &str) {
        let c = self.counts.entry(id.to_string()).or_insert(0);
        *c = c.saturating_add(1);
    }

    pub fn count(&self, id: &str) -> u32 {
        self.counts.get(id).copied().unwrap_or(0)
    }

    pub fn len(&self) -> usize {
        self.counts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.counts.is_empty()
    }

    /// One `count<TAB>id` line per item. Ids with a tab or a newline are left out.
    pub fn to_text(&self) -> String {
        self.counts.iter().filter(|(id, _)| !id.contains(['\t', '\n'])).map(|(id, c)| format!("{c}\t{id}\n")).collect()
    }

    /// Reads [`Usage::to_text`] output; malformed lines are skipped.
    pub fn from_text(text: &str) -> Usage {
        let mut u = Usage::default();
        for line in text.lines() {
            if let Some((c, id)) = line.split_once('\t') {
                if let (Ok(c), false) = (c.parse::<u32>(), id.is_empty()) {
                    u.counts.insert(id.to_string(), c);
                }
            }
        }
        u
    }
}

/// A semantic similarity between a query and a text, in 0..=1, from a local model. `None` when there is no model.
pub trait SemanticHook {
    fn similarity(&self, query: &str, text: &str) -> Option<f32>;
}

/// The hook used when no local text model is available: it adds nothing.
pub struct NoSemantic;

impl SemanticHook for NoSemantic {
    fn similarity(&self, _query: &str, _text: &str) -> Option<f32> {
        None
    }
}

/// True for a query that is one or two words and short: the kind prior then applies.
fn is_short(query: &str, terms: &[String]) -> bool {
    terms.len() <= 2 && query.trim().chars().count() <= 12
}

/// 1 for a title equal to the query, 0.6 for a title starting with it, 0.3 for a title word starting with its last
/// word, else 0.
fn title_signal(title: &str, terms: &[String], joined: &str) -> f32 {
    let words = tokenize(title);
    let t = words.join(" ");
    if t.is_empty() {
        0.0
    } else if t == joined {
        1.0
    } else if t.starts_with(joined) {
        0.6
    } else if terms.last().map_or(false, |last| words.iter().any(|w| w.starts_with(last.as_str()))) {
        0.3
    } else {
        0.0
    }
}

/// e^(-age / RECENCY_DAYS) for an item indexed `modified` seconds ago; 0 when the time is unknown.
fn recency(modified: u64, now: u64) -> f32 {
    if modified == 0 {
        return 0.0;
    }
    let age_days = now.saturating_sub(modified) as f32 / 86_400.0;
    (-age_days / RECENCY_DAYS).exp()
}

/// Ranks the items that hold a query word, best first: (item index, score). Ties go to the title, then the id.
/// The score is a weighted sum: BM25 (times the share of query words held), the title match, the kind prior for short
/// queries, recency, the local open count on a log scale, and the semantic similarity when the hook gives one.
pub fn rank(query: &str, items: &[Item], usage: &Usage, now: u64, semantic: &dyn SemanticHook) -> Vec<(usize, f32)> {
    let terms = tokenize(query);
    if terms.is_empty() || items.is_empty() {
        return vec![];
    }
    let mut bm = Bm25::default();
    for it in items {
        bm.add(&[(it.title.as_str(), TITLE_WEIGHT), (it.text.as_str(), TEXT_WEIGHT)]);
    }
    let scored = bm.scores(&terms);
    let max_bm = scored.iter().map(|s| s.1).fold(0.0f32, f32::max).max(f32::MIN_POSITIVE);
    let max_count = items.iter().map(|it| usage.count(&it.id)).max().unwrap_or(0);
    let short = is_short(query, &terms);
    let joined = terms.join(" ");
    let mut out: Vec<(usize, f32)> = scored
        .into_iter()
        .map(|(i, s, share)| {
            let it = &items[i];
            let lexical = s / max_bm * share;
            let title = title_signal(&it.title, &terms, &joined);
            let prior = if short { it.kind.prior() } else { 0.0 };
            let recent = recency(it.modified, now);
            let freq = if max_count == 0 { 0.0 } else { (1.0 + usage.count(&it.id) as f32).ln() / (1.0 + max_count as f32).ln() };
            let sem = semantic.similarity(query, &format!("{} {}", it.title, it.text)).map_or(0.0, |v| v.clamp(0.0, 1.0));
            let total = W_BM25 * lexical + W_TITLE * title + W_PRIOR * prior + W_RECENCY * recent + W_FREQUENCY * freq + W_SEMANTIC * sem;
            (i, total)
        })
        .collect();
    out.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| items[a.0].title.cmp(&items[b.0].title)).then_with(|| items[a.0].id.cmp(&items[b.0].id)));
    out
}

/// Every item index in display order: the ranked ones first (see [`rank`]), then the rest in their given order. Use
/// this to reorder a list that a source already filtered, so nothing is dropped.
pub fn order(query: &str, items: &[Item], usage: &Usage, now: u64, semantic: &dyn SemanticHook) -> Vec<usize> {
    let mut seen = vec![false; items.len()];
    let mut out: Vec<usize> = rank(query, items, usage, now, semantic)
        .into_iter()
        .map(|(i, _)| {
            seen[i] = true;
            i
        })
        .collect();
    out.extend((0..items.len()).filter(|&i| !seen[i]));
    out
}

/// A place that answers a query with items, such as the apps, the settings pages or the contacts. Runs on its own
/// thread, so it must be `Send + Sync`.
pub trait Source: Send + Sync {
    /// The section name, such as "Apps".
    fn name(&self) -> &'static str;
    /// The items matching `query`.
    fn search(&self, query: &str) -> Vec<Item>;
}

/// One ranked item from a source.
#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub source: &'static str,
    pub item: Item,
    pub score: f32,
}

/// The merged answer: the best hit overall, the sections in source order (empty ones left out), and the names of the
/// sources that did not answer by the deadline (or failed).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Merged {
    pub top: Option<Hit>,
    pub sections: Vec<Section>,
    pub missing: Vec<&'static str>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Section {
    pub source: &'static str,
    pub hits: Vec<Hit>,
}

/// Queries every source at once and waits at most `deadline` for them. Items from the sources that answered are ranked
/// together with [`rank`]; each section keeps at most `per_section` hits. An empty query asks nobody.
pub fn run(query: &str, sources: &[Arc<dyn Source>], deadline: Duration, per_section: usize, usage: &Usage, now: u64, semantic: &dyn SemanticHook) -> Merged {
    let query = query.trim();
    if query.is_empty() {
        return Merged::default();
    }
    let (tx, rx) = mpsc::channel::<(usize, Vec<Item>)>();
    for (i, source) in sources.iter().enumerate() {
        let source = Arc::clone(source);
        let tx = tx.clone();
        let q = query.to_string();
        thread::spawn(move || {
            let items = source.search(&q);
            let _ = tx.send((i, items));
        });
    }
    drop(tx);
    let start = Instant::now();
    let mut got: Vec<Option<Vec<Item>>> = (0..sources.len()).map(|_| None).collect();
    let mut waiting = sources.len();
    while waiting > 0 {
        let left = deadline.saturating_sub(start.elapsed());
        match rx.recv_timeout(left) {
            Ok((i, items)) => {
                if got[i].is_none() {
                    got[i] = Some(items);
                    waiting -= 1;
                }
            }
            // the deadline passed, or every source has finished without answering
            Err(_) => break,
        }
    }
    let mut items: Vec<Item> = Vec::new();
    let mut owner: Vec<usize> = Vec::new();
    let mut missing = Vec::new();
    for (i, slot) in got.into_iter().enumerate() {
        match slot {
            Some(list) => {
                for item in list {
                    items.push(item);
                    owner.push(i);
                }
            }
            None => missing.push(sources[i].name()),
        }
    }
    let ranked = rank(query, &items, usage, now, semantic);
    let top = ranked.first().map(|&(i, score)| Hit { source: sources[owner[i]].name(), item: items[i].clone(), score });
    let mut sections = Vec::new();
    for (si, source) in sources.iter().enumerate() {
        let hits: Vec<Hit> = ranked
            .iter()
            .filter(|&&(i, _)| owner[i] == si)
            .take(per_section)
            .map(|&(i, score)| Hit { source: source.name(), item: items[i].clone(), score })
            .collect();
        if !hits.is_empty() {
            sections.push(Section { source: source.name(), hits });
        }
    }
    Merged { top, sections, missing }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn terms(s: &str) -> Vec<String> {
        tokenize(s)
    }

    #[test]
    fn tokens_fold_case_and_diacritics() {
        assert_eq!(tokenize("Café Résumé"), vec!["cafe", "resume"]);
        assert_eq!(tokenize("ÆBLE École"), vec!["aeble", "ecole"]);
        assert_eq!(tokenize("Straße"), vec!["strasse"]);
        // a decomposed accent (e + combining acute) folds the same way
        assert_eq!(tokenize("cafe\u{301}"), vec!["cafe"]);
    }

    #[test]
    fn punctuation_separates_and_apostrophes_join() {
        assert_eq!(tokenize("Don't stop, 2024-10!"), vec!["dont", "stop", "2024", "10"]);
        assert_eq!(tokenize("  ...  "), Vec::<String>::new());
        assert_eq!(tokenize("' leading and trailing '"), vec!["leading", "and", "trailing"]);
    }

    #[test]
    fn cjk_runs_become_pairs_and_single_characters_stay() {
        assert_eq!(tokenize("東京都庁"), vec!["東京", "京都", "都庁"]);
        assert_eq!(tokenize("猫"), vec!["猫"]);
        assert_eq!(tokenize("Tokyo 東京"), vec!["tokyo", "東京"]);
        assert_eq!(tokenize("ひらがなカタカナ"), vec!["ひら", "らが", "がな", "なカ", "カタ", "タカ", "カナ"]);
    }

    #[test]
    fn index_terms_add_prefixes_and_single_characters() {
        let t = index_terms("mail");
        assert!(t.contains(&"ma".to_string()) && t.contains(&"mai".to_string()) && t.contains(&"mail".to_string()));
        assert!(!t.contains(&"mailx".to_string()));
        assert_eq!(index_terms("a"), vec!["a".to_string()]);
        let cjk = index_terms("東京都");
        assert!(cjk.contains(&"京".to_string()) && cjk.contains(&"東京".to_string()));
        // the word itself plus prefixes of two to PREFIX_MAX characters, so a long word does not explode the index
        let long = index_terms("supercalifragilisticexpialidocious");
        assert_eq!(long.len(), 1 + (PREFIX_MAX - 1));
    }

    #[test]
    fn typing_a_prefix_finds_the_word() {
        let mut bm = Bm25::default();
        bm.add(&[("Mailbox", TITLE_WEIGHT)]);
        bm.add(&[("Calendar", TITLE_WEIGHT)]);
        let hits = bm.scores(&terms("mai"));
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].0, 0);
    }

    #[test]
    fn bm25_rewards_frequency_and_rarity() {
        let mut bm = Bm25::default();
        bm.add(&[("apple apple banana", 1.0)]);
        bm.add(&[("apple cherry", 1.0)]);
        let s = bm.scores(&terms("apple"));
        let score = |doc: usize| s.iter().find(|h| h.0 == doc).unwrap().1;
        assert!(score(0) > score(1), "two occurrences beat one");
        assert_eq!(bm.scores(&terms("banana")).len(), 1, "only the document holding the word");
    }

    #[test]
    fn bm25_coverage_prefers_a_document_with_every_word() {
        let mut bm = Bm25::default();
        bm.add(&[("milk bread", 1.0)]);
        bm.add(&[("milk", 1.0)]);
        let s = bm.scores(&terms("milk bread"));
        let share = |doc: usize| s.iter().find(|h| h.0 == doc).unwrap().2;
        assert_eq!(share(0), 1.0);
        assert_eq!(share(1), 0.5);
    }

    #[test]
    fn exact_app_outranks_settings_then_document_for_a_short_query() {
        let now = 1_000_000;
        let items = vec![
            Item { id: "doc".into(), title: "Mail draft".into(), text: String::new(), kind: Kind::Document, modified: now },
            Item::new("app", "Mail", Kind::App),
            Item::new("settings", "Mail", Kind::Settings),
        ];
        let order: Vec<&str> = rank("mail", &items, &Usage::default(), now, &NoSemantic).into_iter().map(|(i, _)| items[i].id.as_str()).collect();
        assert_eq!(order, vec!["app", "settings", "doc"]);
    }

    #[test]
    fn recent_items_beat_old_ones_with_the_same_title() {
        let now = 40_000_000;
        let items = vec![
            Item { id: "a".into(), title: "Notes".into(), text: String::new(), kind: Kind::Document, modified: now - 365 * 86_400 },
            Item { id: "b".into(), title: "Notes".into(), text: String::new(), kind: Kind::Document, modified: now },
        ];
        let ids: Vec<usize> = rank("notes", &items, &Usage::default(), now, &NoSemantic).into_iter().map(|(i, _)| i).collect();
        assert_eq!(ids, vec![1, 0]);
    }

    #[test]
    fn opens_break_ties_between_equal_items() {
        let items = vec![Item::new("a", "Notes", Kind::Document), Item::new("z", "Notes", Kind::Document)];
        let mut usage = Usage::default();
        for _ in 0..5 {
            usage.bump("z");
        }
        let first = rank("notes", &items, &usage, 0, &NoSemantic)[0].0;
        assert_eq!(items[first].id, "z");
    }

    struct Pasta;

    impl SemanticHook for Pasta {
        fn similarity(&self, _query: &str, text: &str) -> Option<f32> {
            Some(if text.contains("pasta") { 1.0 } else { 0.0 })
        }
    }

    #[test]
    fn a_semantic_score_reorders_lexical_matches() {
        let items = vec![
            Item { id: "a".into(), title: "Food notes".into(), text: "rice".into(), kind: Kind::Document, modified: 0 },
            Item { id: "z".into(), title: "Food notes".into(), text: "pasta".into(), kind: Kind::Document, modified: 0 },
        ];
        let first = rank("food", &items, &Usage::default(), 0, &Pasta)[0].0;
        assert_eq!(items[first].id, "z");
        // without a model the same items keep the id order
        let plain = rank("food", &items, &Usage::default(), 0, &NoSemantic)[0].0;
        assert_eq!(items[plain].id, "a");
    }

    #[test]
    fn order_keeps_unranked_items_in_their_place() {
        let items = vec![Item::new("x", "Zebra", Kind::Other), Item::new("y", "Mail", Kind::App)];
        assert_eq!(order("mail", &items, &Usage::default(), 0, &NoSemantic), vec![1, 0]);
    }

    #[test]
    fn usage_round_trips_and_skips_bad_lines() {
        let mut u = Usage::default();
        u.bump("app/one");
        u.bump("app/one");
        u.bump("x\ty");
        assert_eq!(u.to_text(), "2\tapp/one\n");
        let back = Usage::from_text("2\tapp/one\nnot a line\nx\t\n-1\tbad\n");
        assert_eq!(back.count("app/one"), 2);
        assert_eq!(back.len(), 1);
    }

    struct Fake {
        name: &'static str,
        delay: Duration,
        items: Vec<Item>,
    }

    impl Source for Fake {
        fn name(&self) -> &'static str {
            self.name
        }
        fn search(&self, _query: &str) -> Vec<Item> {
            thread::sleep(self.delay);
            self.items.clone()
        }
    }

    fn fake(name: &'static str, delay_ms: u64, items: Vec<Item>) -> Arc<dyn Source> {
        Arc::new(Fake { name, delay: Duration::from_millis(delay_ms), items })
    }

    #[test]
    fn a_source_that_is_late_is_left_out_at_the_deadline() {
        let sources = vec![fake("apps", 0, vec![Item::new("m", "Mail", Kind::App)]), fake("contacts", 2000, vec![Item::new("c", "Mail Bob", Kind::Contact)])];
        let started = Instant::now();
        let merged = run("mail", &sources, Duration::from_millis(150), 5, &Usage::default(), 0, &NoSemantic);
        assert!(started.elapsed() < Duration::from_millis(1500), "did not wait for the late source");
        assert_eq!(merged.missing, vec!["contacts"]);
        assert_eq!(merged.sections.len(), 1);
        assert_eq!(merged.sections[0].source, "apps");
        assert_eq!(merged.top.as_ref().map(|h| h.item.id.as_str()), Some("m"));
    }

    #[test]
    fn sections_follow_source_order_and_each_is_capped() {
        let sources = vec![
            fake("settings", 0, vec![Item::new("s1", "Mail accounts", Kind::Settings), Item::new("s2", "Mail rules", Kind::Settings), Item::new("s3", "Mail sync", Kind::Settings)]),
            fake("apps", 0, vec![Item::new("m", "Mail", Kind::App)]),
        ];
        let merged = run("mail", &sources, Duration::from_secs(5), 2, &Usage::default(), 0, &NoSemantic);
        assert!(merged.missing.is_empty());
        let names: Vec<&str> = merged.sections.iter().map(|s| s.source).collect();
        assert_eq!(names, vec!["settings", "apps"]);
        assert_eq!(merged.sections[0].hits.len(), 2);
        assert_eq!(merged.top.as_ref().map(|h| h.source), Some("apps"));
    }

    #[test]
    fn an_empty_query_asks_no_source() {
        let sources = vec![fake("apps", 0, vec![Item::new("m", "Mail", Kind::App)])];
        let merged = run("   ", &sources, Duration::from_secs(1), 5, &Usage::default(), 0, &NoSemantic);
        assert_eq!(merged, Merged::default());
    }
}
