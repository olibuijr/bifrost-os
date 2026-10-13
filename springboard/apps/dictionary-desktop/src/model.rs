//! The Dictionary app's logic without any drawing: lookup history, dictionary scope, the stored preferences (which
//! dictionaries are used, in what order, under what label, and the text size), find-in-definition and URL parsing.

use bifrost_ui::stardict::Hit;

/// Smallest and largest text-size step; each step is a tenth of the base size.
pub const MIN_STEP: i32 = -3;
pub const MAX_STEP: i32 = 6;

/// The words looked up, with a position for Back and Forward.
#[derive(Debug, Default)]
pub struct History {
    entries: Vec<String>,
    /// Index of the word on screen; meaningless while `entries` is empty.
    pos: usize,
}

impl History {
    /// Records a lookup. Looking up the word already on screen changes nothing; a new word drops the Forward entries.
    pub fn visit(&mut self, word: &str) {
        if self.current() == Some(word) {
            return;
        }
        if !self.entries.is_empty() {
            self.entries.truncate(self.pos + 1);
        }
        self.entries.push(word.to_string());
        self.pos = self.entries.len() - 1;
    }

    pub fn current(&self) -> Option<&str> {
        self.entries.get(self.pos).map(String::as_str)
    }

    pub fn can_back(&self) -> bool {
        !self.entries.is_empty() && self.pos > 0
    }

    pub fn can_forward(&self) -> bool {
        self.pos + 1 < self.entries.len()
    }

    /// Steps back and returns the word to show.
    pub fn back(&mut self) -> Option<String> {
        if !self.can_back() {
            return None;
        }
        self.pos -= 1;
        self.current().map(str::to_string)
    }

    /// Steps forward and returns the word to show.
    pub fn forward(&mut self) -> Option<String> {
        if !self.can_forward() {
            return None;
        }
        self.pos += 1;
        self.current().map(str::to_string)
    }

    /// Forgets everything.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.pos = 0;
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hex = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok());
            if let Some(v) = hex {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The word in a `dict:` or `x-dictionary:` URL: `dict://serendipity`, `dict:caf%C3%A9`, `x-dictionary:r:word`.
pub fn word_from_url(url: &str) -> Option<String> {
    let url = url.trim();
    let lower = url.to_ascii_lowercase();
    let rest = if lower.starts_with("x-dictionary:") {
        let r = &url["x-dictionary:".len()..];
        // x-dictionary:r:<word> and x-dictionary:d:<word> carry a one-letter kind
        match r.as_bytes() {
            [k, b':', ..] if k.is_ascii_alphabetic() => &r[2..],
            _ => r,
        }
    } else if lower.starts_with("dict:") {
        &url["dict:".len()..]
    } else {
        return None;
    };
    let rest = rest.trim_start_matches('/');
    let word = percent_decode(rest);
    let word = word.trim();
    (!word.is_empty()).then(|| word.to_string())
}

/// One installed dictionary's stored choices.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DictPref {
    pub name: String,
    pub enabled: bool,
    /// The display label; empty keeps the dictionary's own name.
    pub label: String,
}

impl DictPref {
    pub fn shown_name(&self) -> &str {
        if self.label.is_empty() {
            &self.name
        } else {
            &self.label
        }
    }
}

/// The stored preferences.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Prefs {
    /// Text-size step, `MIN_STEP..=MAX_STEP`.
    pub step: i32,
    /// Installed dictionaries in the order they are shown.
    pub dicts: Vec<DictPref>,
}

impl Prefs {
    /// Parses the stored text; unreadable lines are skipped.
    pub fn parse(text: &str) -> Prefs {
        let mut p = Prefs::default();
        for line in text.lines() {
            if let Some(v) = line.strip_prefix("size=") {
                p.step = v.trim().parse::<i32>().unwrap_or(0).clamp(MIN_STEP, MAX_STEP);
            } else if let Some(rest) = line.strip_prefix("dict\t") {
                let mut f = rest.splitn(3, '\t');
                let (Some(on), Some(name)) = (f.next(), f.next()) else { continue };
                if name.is_empty() {
                    continue;
                }
                p.dicts.push(DictPref { name: name.to_string(), enabled: on != "0", label: f.next().unwrap_or("").to_string() });
            }
        }
        p
    }

    pub fn serialize(&self) -> String {
        let clean = |s: &str| s.replace(['\t', '\n', '\r'], " ");
        let mut out = format!("size={}\n", self.step);
        for d in &self.dicts {
            out.push_str(&format!("dict\t{}\t{}\t{}\n", if d.enabled { 1 } else { 0 }, clean(&d.name), clean(&d.label)));
        }
        out
    }

    /// Makes the list match the installed dictionaries: new ones are appended enabled, missing ones dropped.
    /// Returns true when the list changed.
    pub fn sync(&mut self, installed: &[String]) -> bool {
        let before = self.dicts.clone();
        self.dicts.retain(|d| installed.contains(&d.name));
        for name in installed {
            if !self.dicts.iter().any(|d| &d.name == name) {
                self.dicts.push(DictPref { name: name.clone(), enabled: true, label: String::new() });
            }
        }
        self.dicts != before
    }

    pub fn bigger(&mut self) {
        self.step = (self.step + 1).min(MAX_STEP);
    }

    pub fn smaller(&mut self) {
        self.step = (self.step - 1).max(MIN_STEP);
    }

    pub fn actual_size(&mut self) {
        self.step = 0;
    }

    /// `pt` scaled by the text-size step.
    pub fn scaled(&self, pt: f32) -> f32 {
        pt * (1.0 + 0.1 * self.step as f32)
    }

    /// Moves dictionary `index` by `delta` places (clamped).
    pub fn move_dict(&mut self, index: usize, delta: i32) {
        if index >= self.dicts.len() {
            return;
        }
        let to = (index as i32 + delta).clamp(0, self.dicts.len() as i32 - 1) as usize;
        let d = self.dicts.remove(index);
        self.dicts.insert(to, d);
    }

    pub fn toggle(&mut self, index: usize) {
        if let Some(d) = self.dicts.get_mut(index) {
            d.enabled = !d.enabled;
        }
    }

    pub fn set_label(&mut self, index: usize, label: &str) {
        if let Some(d) = self.dicts.get_mut(index) {
            d.label = label.trim().to_string();
        }
    }

    /// The hits of enabled dictionaries, in the stored order, each under its display label.
    pub fn arrange(&self, hits: Vec<Hit>) -> Vec<Hit> {
        let mut out = Vec::new();
        for d in self.dicts.iter().filter(|d| d.enabled) {
            for h in hits.iter().filter(|h| h.dictionary == d.name) {
                out.push(Hit { dictionary: d.shown_name().to_string(), text: h.text.clone() });
            }
        }
        // a dictionary the preferences do not know yet is shown as installed
        for h in hits.iter().filter(|h| !self.dicts.iter().any(|d| d.name == h.dictionary)) {
            out.push(h.clone());
        }
        out
    }
}

/// The dictionaries a scope bar offers for these hits: the distinct names, in order.
pub fn scope_names(hits: &[Hit]) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for h in hits {
        if !names.contains(&h.dictionary) {
            names.push(h.dictionary.clone());
        }
    }
    names
}

/// Cycles the scope through All (`None`) and each name; `delta` is +1 for Next Dictionary, -1 for Previous.
pub fn cycle_scope(current: &Option<String>, names: &[String], delta: i32) -> Option<String> {
    let n = names.len() as i32 + 1;
    let at = match current {
        None => 0,
        Some(c) => names.iter().position(|x| x == c).map(|i| i as i32 + 1).unwrap_or(0),
    };
    let next = (at + delta).rem_euclid(n);
    if next == 0 {
        None
    } else {
        Some(names[next as usize - 1].clone())
    }
}

/// The hits that belong to `scope` (all of them for `None`).
pub fn in_scope(hits: &[Hit], scope: &Option<String>) -> Vec<Hit> {
    hits.iter().filter(|h| scope.as_ref().map_or(true, |s| &h.dictionary == s)).cloned().collect()
}

/// Indexes of the lines containing `query`, ignoring case. An empty query matches nothing.
pub fn find_lines(lines: &[&str], query: &str) -> Vec<usize> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Vec::new();
    }
    lines.iter().enumerate().filter(|(_, l)| l.to_lowercase().contains(&q)).map(|(i, _)| i).collect()
}

/// The match after (`delta` 1) or before (-1) `current`, wrapping around. `current` is a position in `matches`.
pub fn step_match(matches: &[usize], current: Option<usize>, delta: i32) -> Option<usize> {
    if matches.is_empty() {
        return None;
    }
    let n = matches.len() as i32;
    Some(match current {
        None => {
            if delta >= 0 {
                0
            } else {
                matches.len() - 1
            }
        }
        Some(c) => (c as i32 + delta).rem_euclid(n) as usize,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(d: &str, t: &str) -> Hit {
        Hit { dictionary: d.into(), text: t.into() }
    }

    #[test]
    fn history_goes_back_and_forward_and_a_new_word_drops_forward() {
        let mut h = History::default();
        assert!(!h.can_back() && !h.can_forward());
        h.visit("a");
        h.visit("b");
        h.visit("b");
        h.visit("c");
        assert_eq!(h.back().as_deref(), Some("b"));
        assert_eq!(h.back().as_deref(), Some("a"));
        assert_eq!(h.back(), None);
        assert_eq!(h.forward().as_deref(), Some("b"));
        h.visit("d");
        assert!(!h.can_forward());
        assert_eq!(h.back().as_deref(), Some("b"));
        h.clear();
        assert!(h.is_empty() && h.current().is_none());
    }

    #[test]
    fn urls_give_the_word() {
        assert_eq!(word_from_url("dict://serendipity").as_deref(), Some("serendipity"));
        assert_eq!(word_from_url("dict:caf%C3%A9").as_deref(), Some("caf\u{e9}"));
        assert_eq!(word_from_url("x-dictionary:r:word").as_deref(), Some("word"));
        assert_eq!(word_from_url("DICT:///Hello%20World").as_deref(), Some("Hello World"));
        assert_eq!(word_from_url("https://example.com"), None);
        assert_eq!(word_from_url("dict://"), None);
        assert_eq!(word_from_url("dict:100%").as_deref(), Some("100%"));
    }

    #[test]
    fn prefs_round_trip_and_sync() {
        let mut p = Prefs::default();
        p.sync(&["A".into(), "B".into()]);
        p.set_label(1, "  Bee ");
        p.toggle(0);
        p.bigger();
        let q = Prefs::parse(&p.serialize());
        assert_eq!(p, q);
        assert_eq!(q.dicts[1].shown_name(), "Bee");
        let mut q = q;
        assert!(q.sync(&["B".into(), "C".into()]));
        assert_eq!(q.dicts.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(), ["B", "C"]);
        assert!(!q.sync(&["B".into(), "C".into()]));
        assert_eq!(Prefs::parse("size=99\nbad\ndict\t1\t\n").step, MAX_STEP);
    }

    #[test]
    fn text_size_is_clamped_and_scales() {
        let mut p = Prefs::default();
        for _ in 0..20 {
            p.bigger();
        }
        assert_eq!(p.step, MAX_STEP);
        for _ in 0..20 {
            p.smaller();
        }
        assert_eq!(p.step, MIN_STEP);
        assert!((p.scaled(10.0) - 7.0).abs() < 1e-4);
        p.actual_size();
        assert_eq!(p.scaled(13.0), 13.0);
    }

    #[test]
    fn arrange_orders_filters_and_relabels() {
        let mut p = Prefs::default();
        p.sync(&["A".into(), "B".into(), "C".into()]);
        p.move_dict(2, -2);
        p.toggle(1);
        p.set_label(0, "Cee");
        let out = p.arrange(vec![hit("A", "a"), hit("B", "b"), hit("C", "c"), hit("Z", "z")]);
        let names: Vec<_> = out.iter().map(|h| h.dictionary.as_str()).collect();
        assert_eq!(names, ["Cee", "B", "Z"]);
    }

    #[test]
    fn move_dict_clamps() {
        let mut p = Prefs::default();
        p.sync(&["A".into(), "B".into()]);
        p.move_dict(0, -5);
        p.move_dict(0, 9);
        assert_eq!(p.dicts[1].name, "A");
        p.move_dict(7, 1);
    }

    #[test]
    fn scope_cycles_through_all_and_each_dictionary() {
        let hits = vec![hit("A", "1"), hit("B", "2"), hit("A", "3")];
        let names = scope_names(&hits);
        assert_eq!(names, ["A", "B"]);
        let s = cycle_scope(&None, &names, 1);
        assert_eq!(s.as_deref(), Some("A"));
        let s = cycle_scope(&s, &names, 1);
        assert_eq!(s.as_deref(), Some("B"));
        assert_eq!(cycle_scope(&s, &names, 1), None);
        assert_eq!(cycle_scope(&None, &names, -1).as_deref(), Some("B"));
        assert_eq!(in_scope(&hits, &Some("A".into())).len(), 2);
        assert_eq!(in_scope(&hits, &None).len(), 3);
        assert_eq!(cycle_scope(&None, &[], 1), None);
    }

    #[test]
    fn find_matches_lines_and_wraps() {
        let lines = ["Hello world", "nothing", "WORLD again"];
        let m = find_lines(&lines, "world");
        assert_eq!(m, [0, 2]);
        assert!(find_lines(&lines, "  ").is_empty());
        assert_eq!(step_match(&m, None, 1), Some(0));
        assert_eq!(step_match(&m, None, -1), Some(1));
        assert_eq!(step_match(&m, Some(1), 1), Some(0));
        assert_eq!(step_match(&m, Some(0), -1), Some(1));
        assert_eq!(step_match(&[], None, 1), None);
    }
}
