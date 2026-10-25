//! Search's matcher: case- and diacritic-insensitive, ranked prefix >
//! word start > subsequence, with the Icelandic letters folded so "thor"
//! and "þór" both find "Þór" and "ae" finds "Ægir".

/// How a query matched a candidate, best first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    /// the candidate starts with the query
    Prefix,
    /// a later word of the candidate starts with the query
    WordStart,
    /// the query's letters appear in order somewhere in the candidate
    Subsequence,
}

/// A match's rank: its kind, then the candidate's length (shorter wins), so
/// ranks compare and sort straight from best to worst.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Rank {
    pub kind: Kind,
    pub len: usize,
}

/// Lowercases and strips accents: "Reykjavík" → "reykjavik", "Þór" → "thor".
pub fn fold(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars().flat_map(char::to_lowercase) {
        match c {
            'á' | 'à' | 'â' | 'ä' | 'ã' | 'å' => out.push('a'),
            'é' | 'è' | 'ê' | 'ë' => out.push('e'),
            'í' | 'ì' | 'î' | 'ï' => out.push('i'),
            'ó' | 'ò' | 'ô' | 'ö' | 'õ' | 'ø' => out.push('o'),
            'ú' | 'ù' | 'û' | 'ü' => out.push('u'),
            'ý' | 'ÿ' => out.push('y'),
            'ñ' => out.push('n'),
            'ç' => out.push('c'),
            'ð' => out.push('d'),
            'æ' => out.push_str("ae"),
            'œ' => out.push_str("oe"),
            'þ' => out.push_str("th"),
            'ß' => out.push_str("ss"),
            c => out.push(c),
        }
    }
    out
}

fn is_subsequence(q: &str, c: &str) -> bool {
    let mut want = q.chars();
    let mut next = want.next();
    for ch in c.chars() {
        if next == Some(ch) {
            next = want.next();
        }
    }
    next.is_none()
}

/// A query folded once, so matching many candidates (every keystroke) does not
/// re-fold it per candidate; candidates can be folded ahead of time too.
pub struct Matcher(String);

impl Matcher {
    pub fn new(query: &str) -> Self {
        Self(fold(query.trim()))
    }

    /// The query, folded and trimmed.
    pub fn query(&self) -> &str {
        &self.0
    }

    /// How the query matches `candidate`; None when it does not (or is blank).
    pub fn rank(&self, candidate: &str) -> Option<Rank> {
        self.rank_folded(&fold(candidate))
    }

    /// Like `rank`, for a candidate already passed through `fold`.
    pub fn rank_folded(&self, c: &str) -> Option<Rank> {
        let q = self.0.as_str();
        if q.is_empty() {
            return None;
        }
        let len = c.chars().count();
        let kind = if c.starts_with(q) {
            Kind::Prefix
        } else if c.split(|ch: char| !ch.is_alphanumeric()).any(|w| !w.is_empty() && w.starts_with(q)) {
            Kind::WordStart
        } else if is_subsequence(q, c) {
            Kind::Subsequence
        } else {
            return None;
        };
        Some(Rank { kind, len })
    }
}

/// How `query` matches `candidate`; None when it does not (or is blank).
pub fn rank(query: &str, candidate: &str) -> Option<Rank> {
    Matcher::new(query).rank(candidate)
}

/// The items `query` matches, best first; ties keep the input order.
pub fn sorted<T>(query: &str, items: impl IntoIterator<Item = (T, String)>) -> Vec<(Rank, T)> {
    let m = Matcher::new(query);
    let mut v: Vec<(Rank, T)> = items.into_iter().filter_map(|(item, text)| m.rank(&text).map(|r| (r, item))).collect();
    v.sort_by_key(|(r, _)| *r);
    v
}

/// `sorted` over candidates that are already folded.
pub fn sorted_folded<'a, T>(m: &Matcher, items: impl IntoIterator<Item = (T, &'a str)>) -> Vec<(Rank, T)> {
    let mut v: Vec<(Rank, T)> = items.into_iter().filter_map(|(item, folded)| m.rank_folded(folded).map(|r| (r, item))).collect();
    v.sort_by_key(|(r, _)| *r);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind(q: &str, c: &str) -> Option<Kind> {
        rank(q, c).map(|r| r.kind)
    }

    #[test]
    fn prefix_beats_word_start_beats_subsequence() {
        assert_eq!(kind("san", "San Francisco"), Some(Kind::Prefix));
        assert_eq!(kind("fran", "San Francisco"), Some(Kind::WordStart));
        assert_eq!(kind("sfo", "San Francisco"), Some(Kind::Subsequence));
        assert_eq!(kind("xyz", "San Francisco"), None);
        assert!(Kind::Prefix < Kind::WordStart && Kind::WordStart < Kind::Subsequence);
        let v = sorted("se", vec![("sub", "Base".to_string()), ("word", "bifrost Settings".to_string()), ("pre", "Settings".to_string())]);
        assert_eq!(v.iter().map(|(_, t)| *t).collect::<Vec<_>>(), ["pre", "word", "sub"]);
    }

    #[test]
    fn shorter_candidate_wins_among_equal_kinds_and_ties_keep_order() {
        let v = sorted("c", vec![("long", "Clock Radio".to_string()), ("short", "Clock".to_string()), ("same", "Clack".to_string())]);
        assert_eq!(v.iter().map(|(_, t)| *t).collect::<Vec<_>>(), ["short", "same", "long"]);
    }

    #[test]
    fn case_and_diacritics_are_ignored() {
        assert_eq!(kind("REYKJAVIK", "Reykjavík"), Some(Kind::Prefix));
        assert_eq!(kind("reykjavík", "REYKJAVIK"), Some(Kind::Prefix));
        assert_eq!(kind("wi-fi", "Wi-Fi"), Some(Kind::Prefix));
        assert_eq!(kind("fi", "Wi-Fi"), Some(Kind::WordStart), "a hyphen separates words");
        assert_eq!(kind("york", "New York"), Some(Kind::WordStart));
    }

    #[test]
    fn icelandic_letters_fold_both_ways() {
        assert_eq!(fold("Þór Ægir Ðóra ö"), "thor aegir dora o");
        assert_eq!(kind("thor", "Þór"), Some(Kind::Prefix));
        assert_eq!(kind("þor", "Thor"), Some(Kind::Prefix));
        assert_eq!(kind("aegir", "Ægir"), Some(Kind::Prefix));
        assert_eq!(kind("æ", "Ægir"), Some(Kind::Prefix));
        assert_eq!(kind("dei", "Ðeildu"), Some(Kind::Prefix));
        assert_eq!(kind("bjork", "Björk"), Some(Kind::Prefix));
    }

    #[test]
    fn prefolded_candidates_rank_like_raw_ones() {
        let m = Matcher::new("BJORK");
        assert_eq!(m.rank_folded(&fold("Björk.mp3")), rank("bjork", "Björk.mp3"));
        assert_eq!(m.rank_folded(&fold("Björk.mp3")).map(|r| r.kind), Some(Kind::Prefix));
        let v = sorted_folded(&m, vec![("x", "bjork"), ("y", "nope")]);
        assert_eq!(v.len(), 1);
    }

    #[test]
    fn blank_queries_match_nothing_and_whitespace_is_trimmed() {
        assert_eq!(rank("", "Clock"), None);
        assert_eq!(rank("   ", "Clock"), None);
        assert_eq!(kind(" clo ", "Clock"), Some(Kind::Prefix));
        assert_eq!(rank("clock", "Clock").map(|r| r.len), Some(5));
    }
}
