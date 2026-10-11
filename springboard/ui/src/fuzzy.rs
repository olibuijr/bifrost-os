//! Fuzzy matching for Search and command search: subsequence match with a score that prefers word starts,
//! consecutive runs and early matches. Case-insensitive; returns the matched character positions to highlight.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Match {
    pub score: i32,
    pub positions: Vec<usize>,
}

/// Best-effort greedy-with-lookahead match of `query` in `text` (every query character, in order).
pub fn fuzzy_match(query: &str, text: &str) -> Option<Match> {
    let q: Vec<char> = query.chars().flat_map(|c| c.to_lowercase()).filter(|c| !c.is_whitespace()).collect();
    let t: Vec<char> = text.chars().collect();
    if q.is_empty() {
        return Some(Match { score: 0, positions: vec![] });
    }
    let lower: Vec<char> = t.iter().map(|c| c.to_lowercase().next().unwrap_or(*c)).collect();
    // dp over (query index, text index) keeping the best score; small inputs, so O(q*t) is fine.
    let (n, m) = (q.len(), t.len());
    if n > m {
        return None;
    }
    // Cheap reject before the table: the query must be a subsequence at all.
    let mut qi = 0;
    for c in &lower {
        if qi < n && *c == q[qi] {
            qi += 1;
        }
    }
    if qi < n {
        return None;
    }
    const NEG: i32 = i32::MIN / 2;
    let mut best = vec![vec![NEG; m]; n]; // best[i][j]: q[i] matched at t[j]
    let mut from = vec![vec![usize::MAX; m]; n];
    for i in 0..n {
        for j in i..m {
            if lower[j] != q[i] {
                continue;
            }
            let bonus = {
                let start = j == 0 || !t[j - 1].is_alphanumeric();
                let camel = j > 0 && t[j - 1].is_lowercase() && t[j].is_uppercase();
                (if start { 10 } else { 0 }) + (if camel { 8 } else { 0 }) - (j as i32).min(10)
            };
            if i == 0 {
                best[0][j] = 1 + bonus;
            } else {
                for k in (i - 1)..j {
                    if best[i - 1][k] == NEG {
                        continue;
                    }
                    let run = if k + 1 == j { 15 } else { -((j - k - 1) as i32).min(8) };
                    let s = best[i - 1][k] + 1 + bonus + run;
                    if s > best[i][j] {
                        best[i][j] = s;
                        from[i][j] = k;
                    }
                }
            }
        }
    }
    let (mut j, score) = (0..m).filter(|&j| best[n - 1][j] != NEG).map(|j| (j, best[n - 1][j])).max_by_key(|&(_, s)| s)?;
    let mut positions = vec![0; n];
    for i in (0..n).rev() {
        positions[i] = j;
        if i > 0 {
            j = from[i][j];
        }
    }
    Some(Match { score, positions })
}

/// Indices of `items` that match, best first (stable for equal scores).
pub fn rank<'a>(query: &str, items: impl IntoIterator<Item = &'a str>) -> Vec<(usize, Match)> {
    let mut v: Vec<(usize, Match)> = items.into_iter().enumerate().filter_map(|(i, s)| fuzzy_match(query, s).map(|m| (i, m))).collect();
    v.sort_by(|a, b| b.1.score.cmp(&a.1.score).then(a.0.cmp(&b.0)));
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn subsequence_required() {
        assert!(fuzzy_match("stg", "Settings").is_some());
        assert!(fuzzy_match("xyz", "Settings").is_none());
        assert!(fuzzy_match("tt", "Settings").is_some());
    }
    #[test]
    fn word_starts_beat_scattered_letters() {
        let r = rank("vm", ["Savings Mail", "Voice Memos"]);
        assert_eq!(r[0].0, 1);
    }
    #[test]
    fn consecutive_beats_gapped() {
        let r = rank("set", ["s e t", "Settings"]);
        assert_eq!(r[0].0, 1);
    }
    #[test]
    fn positions_are_increasing_and_correct() {
        let m = fuzzy_match("fnd", "Fonder").unwrap();
        assert_eq!(m.positions, vec![0, 2, 3]);
    }
    #[test]
    fn empty_query_matches_all() {
        assert_eq!(rank("", ["a", "b"]).len(), 2);
    }
}
