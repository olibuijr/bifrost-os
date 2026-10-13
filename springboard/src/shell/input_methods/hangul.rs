//! Korean 2-set (dubeolsik) composer: typed jamo become syllables.
//!
//! No Hanja conversion and no ancient (archaic) jamo.

use super::Composer;

const LEADS: [char; 19] = ['ㄱ', 'ㄲ', 'ㄴ', 'ㄷ', 'ㄸ', 'ㄹ', 'ㅁ', 'ㅂ', 'ㅃ', 'ㅅ', 'ㅆ', 'ㅇ', 'ㅈ', 'ㅉ', 'ㅊ', 'ㅋ', 'ㅌ', 'ㅍ', 'ㅎ'];
const VOWELS: [char; 21] = [
    'ㅏ', 'ㅐ', 'ㅑ', 'ㅒ', 'ㅓ', 'ㅔ', 'ㅕ', 'ㅖ', 'ㅗ', 'ㅘ', 'ㅙ', 'ㅚ', 'ㅛ', 'ㅜ', 'ㅝ', 'ㅞ', 'ㅟ', 'ㅠ', 'ㅡ', 'ㅢ', 'ㅣ',
];
/// Final consonants; syllable index is position + 1 (0 is no final).
const TAILS: [char; 27] = [
    'ㄱ', 'ㄲ', 'ㄳ', 'ㄴ', 'ㄵ', 'ㄶ', 'ㄷ', 'ㄹ', 'ㄺ', 'ㄻ', 'ㄼ', 'ㄽ', 'ㄾ', 'ㄿ', 'ㅀ', 'ㅁ', 'ㅂ', 'ㅄ', 'ㅅ', 'ㅆ', 'ㅇ', 'ㅈ', 'ㅊ', 'ㅋ', 'ㅌ', 'ㅍ', 'ㅎ',
];
/// (first, second, compound) final consonants.
const TAIL_PAIRS: [(char, char, char); 11] = [
    ('ㄱ', 'ㅅ', 'ㄳ'),
    ('ㄴ', 'ㅈ', 'ㄵ'),
    ('ㄴ', 'ㅎ', 'ㄶ'),
    ('ㄹ', 'ㄱ', 'ㄺ'),
    ('ㄹ', 'ㅁ', 'ㄻ'),
    ('ㄹ', 'ㅂ', 'ㄼ'),
    ('ㄹ', 'ㅅ', 'ㄽ'),
    ('ㄹ', 'ㅌ', 'ㄾ'),
    ('ㄹ', 'ㅍ', 'ㄿ'),
    ('ㄹ', 'ㅎ', 'ㅀ'),
    ('ㅂ', 'ㅅ', 'ㅄ'),
];
const VOWEL_PAIRS: [(char, char, char); 7] =
    [('ㅗ', 'ㅏ', 'ㅘ'), ('ㅗ', 'ㅐ', 'ㅙ'), ('ㅗ', 'ㅣ', 'ㅚ'), ('ㅜ', 'ㅓ', 'ㅝ'), ('ㅜ', 'ㅔ', 'ㅞ'), ('ㅜ', 'ㅣ', 'ㅟ'), ('ㅡ', 'ㅣ', 'ㅢ')];

/// The jamo a key types on a 2-set board (shifted Q W E R T O P give the double or Y-vowel forms).
fn key_jamo(ch: char) -> Option<char> {
    Some(match ch {
        'Q' => 'ㅃ',
        'W' => 'ㅉ',
        'E' => 'ㄸ',
        'R' => 'ㄲ',
        'T' => 'ㅆ',
        'O' => 'ㅒ',
        'P' => 'ㅖ',
        _ => match ch.to_ascii_lowercase() {
            'q' => 'ㅂ',
            'w' => 'ㅈ',
            'e' => 'ㄷ',
            'r' => 'ㄱ',
            't' => 'ㅅ',
            'y' => 'ㅛ',
            'u' => 'ㅕ',
            'i' => 'ㅑ',
            'o' => 'ㅐ',
            'p' => 'ㅔ',
            'a' => 'ㅁ',
            's' => 'ㄴ',
            'd' => 'ㅇ',
            'f' => 'ㄹ',
            'g' => 'ㅎ',
            'h' => 'ㅗ',
            'j' => 'ㅓ',
            'k' => 'ㅏ',
            'l' => 'ㅣ',
            'z' => 'ㅋ',
            'x' => 'ㅌ',
            'c' => 'ㅊ',
            'v' => 'ㅍ',
            'b' => 'ㅠ',
            'n' => 'ㅜ',
            'm' => 'ㅡ',
            _ => return None,
        },
    })
}

fn combine(table: &[(char, char, char)], a: char, b: char) -> Option<char> {
    table.iter().find(|(x, y, _)| *x == a && *y == b).map(|t| t.2)
}

fn split(table: &[(char, char, char)], c: char) -> Option<(char, char)> {
    table.iter().find(|t| t.2 == c).map(|t| (t.0, t.1))
}

/// The syllable being built: initial, vowel and final jamo.
#[derive(Default)]
pub struct HangulComposer {
    lead: Option<char>,
    vowel: Option<char>,
    tail: Option<char>,
}

impl HangulComposer {
    fn is_empty(&self) -> bool {
        self.lead.is_none() && self.vowel.is_none()
    }

    /// Take the finished text out and start empty.
    fn flush(&mut self) -> String {
        let out = self.composed();
        self.reset();
        out
    }

    fn feed_vowel(&mut self, v: char) -> String {
        if let Some(t) = self.tail {
            // The final consonant moves to the next syllable as its initial.
            let (keep, moved) = match split(&TAIL_PAIRS, t) {
                Some((a, b)) => (Some(a), b),
                None => (None, t),
            };
            self.tail = keep;
            let out = self.flush();
            self.lead = Some(moved);
            self.vowel = Some(v);
            return out;
        }
        match self.vowel {
            Some(cur) => {
                if let Some(c) = combine(&VOWEL_PAIRS, cur, v) {
                    self.vowel = Some(c);
                    return String::new();
                }
            }
            None if self.lead.is_some() => {
                self.vowel = Some(v);
                return String::new();
            }
            None => {}
        }
        let out = self.flush();
        self.vowel = Some(v);
        out
    }

    fn feed_consonant(&mut self, c: char) -> String {
        if self.is_empty() {
            self.lead = Some(c);
            return String::new();
        }
        if self.lead.is_some() && self.vowel.is_some() {
            match self.tail {
                None if TAILS.contains(&c) => {
                    self.tail = Some(c);
                    return String::new();
                }
                Some(t) => {
                    if let Some(cc) = combine(&TAIL_PAIRS, t, c) {
                        self.tail = Some(cc);
                        return String::new();
                    }
                }
                None => {}
            }
        }
        let out = self.flush();
        self.lead = Some(c);
        out
    }
}

impl Composer for HangulComposer {
    fn feed(&mut self, ch: char) -> Option<String> {
        let j = key_jamo(ch)?;
        Some(if VOWELS.contains(&j) { self.feed_vowel(j) } else { self.feed_consonant(j) })
    }

    fn backspace(&mut self) -> bool {
        if let Some(t) = self.tail {
            self.tail = split(&TAIL_PAIRS, t).map(|p| p.0);
        } else if let Some(v) = self.vowel {
            self.vowel = split(&VOWEL_PAIRS, v).map(|p| p.0);
        } else if self.lead.is_some() {
            self.lead = None;
        } else {
            return false;
        }
        true
    }

    fn composed(&self) -> String {
        match (self.lead, self.vowel) {
            (Some(l), Some(v)) => {
                let li = LEADS.iter().position(|&x| x == l).unwrap_or(0) as u32;
                let vi = VOWELS.iter().position(|&x| x == v).unwrap_or(0) as u32;
                let ti = self.tail.and_then(|t| TAILS.iter().position(|&x| x == t)).map_or(0, |p| p as u32 + 1);
                char::from_u32(0xAC00 + (li * 21 + vi) * 28 + ti).map(String::from).unwrap_or_default()
            }
            (Some(c), None) | (None, Some(c)) => c.to_string(),
            (None, None) => String::new(),
        }
    }

    fn reset(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::super::testing::*;
    use super::super::*;

    fn typed(keys: &str) -> String {
        type_keys(KOREAN_2SET, keys).1.text()
    }

    #[test]
    fn gksrmf_is_hangul() {
        assert_eq!(typed("gksrmf"), "한글");
    }

    #[test]
    fn final_moves_to_the_next_syllable_on_a_vowel() {
        assert_eq!(typed("rkr"), "각");
        assert_eq!(typed("rkrk"), "가가");
    }

    #[test]
    fn compound_final_splits_on_a_vowel() {
        assert_eq!(typed("dkfr"), "앍");
        assert_eq!(typed("dkfrk"), "알가");
    }

    #[test]
    fn compound_vowel_and_double_consonants() {
        assert_eq!(typed("ghk"), "화");
        assert_eq!(typed("Rk"), "까");
        assert_eq!(typed("dnj"), "워");
    }

    #[test]
    fn lone_jamo_stay_jamo() {
        assert_eq!(typed("g"), "ㅎ");
        assert_eq!(typed("k"), "ㅏ");
        assert_eq!(typed("gg"), "ㅎㅎ");
    }

    #[test]
    fn backspace_decomposes() {
        let (mut ctl, mut c) = type_keys(KOREAN_2SET, "dkfr");
        let mut seen = vec![ctl.composed_string()];
        for _ in 0..4 {
            assert!(ctl.handle(Key::Backspace, &mut c));
            seen.push(ctl.composed_string());
        }
        assert_eq!(seen, ["앍", "알", "아", "ㅇ", ""]);
        assert!(!ctl.handle(Key::Backspace, &mut c));
        assert_eq!(c.text(), "");
    }

    #[test]
    fn backspace_splits_a_compound_vowel() {
        let (mut ctl, mut c) = type_keys(KOREAN_2SET, "ghk");
        ctl.handle(Key::Backspace, &mut c);
        assert_eq!(ctl.composed_string(), "호");
    }

    #[test]
    fn space_commits_and_is_inserted() {
        let (ctl, c) = type_keys(KOREAN_2SET, "gks ");
        assert_eq!(c.committed, "한 ");
        assert_eq!(c.marked, "");
        assert_eq!(ctl.composed_string(), "");
    }

    #[test]
    fn digits_commit_and_pass() {
        assert_eq!(typed("gks1"), "한1");
    }
}
