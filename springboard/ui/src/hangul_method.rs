//! The built-in hangul method: keys on a Latin board (the two-set layout) become jamo, and jamo are joined into
//! syllable blocks as they arrive. The composition is the finished syllables plus the block still being built. No
//! dictionary is involved, so hanja conversion is not offered. Pure model: it draws nothing.
//!
//! Composition follows the Unicode syllable block formula: `0xAC00 + (lead * 21 + vowel) * 28 + tail`. Two vowels
//! join into a compound vowel (`ㅗ` + `ㅏ` gives `ㅘ`), two consonants into a compound final (`ㄱ` + `ㅅ` gives `ㄳ`),
//! and a vowel typed after a final moves that final (or the second half of a compound final) to start a new block.

/// The 19 lead consonants, in syllable block order.
const LEADS: [char; 19] = [
    'ㄱ', 'ㄲ', 'ㄴ', 'ㄷ', 'ㄸ', 'ㄹ', 'ㅁ', 'ㅂ', 'ㅃ', 'ㅅ', 'ㅆ', 'ㅇ', 'ㅈ', 'ㅉ', 'ㅊ', 'ㅋ', 'ㅌ', 'ㅍ', 'ㅎ',
];
/// The 21 vowels, in syllable block order.
const VOWELS: [char; 21] = [
    'ㅏ', 'ㅐ', 'ㅑ', 'ㅒ', 'ㅓ', 'ㅔ', 'ㅕ', 'ㅖ', 'ㅗ', 'ㅘ', 'ㅙ', 'ㅚ', 'ㅛ', 'ㅜ', 'ㅝ', 'ㅞ', 'ㅟ', 'ㅠ', 'ㅡ', 'ㅢ',
    'ㅣ',
];
/// The 27 final consonants, in syllable block order (a block's final index 0 means "no final").
const TAILS: [char; 27] = [
    'ㄱ', 'ㄲ', 'ㄳ', 'ㄴ', 'ㄵ', 'ㄶ', 'ㄷ', 'ㄹ', 'ㄺ', 'ㄻ', 'ㄼ', 'ㄽ', 'ㄾ', 'ㄿ', 'ㅀ', 'ㅁ', 'ㅂ', 'ㅄ', 'ㅅ', 'ㅆ',
    'ㅇ', 'ㅈ', 'ㅊ', 'ㅋ', 'ㅌ', 'ㅍ', 'ㅎ',
];

/// Compound vowels: (first, second, joined).
const VOWEL_PAIRS: &[(char, char, char)] = &[
    ('ㅗ', 'ㅏ', 'ㅘ'), ('ㅗ', 'ㅐ', 'ㅙ'), ('ㅗ', 'ㅣ', 'ㅚ'), ('ㅜ', 'ㅓ', 'ㅝ'), ('ㅜ', 'ㅔ', 'ㅞ'),
    ('ㅜ', 'ㅣ', 'ㅟ'), ('ㅡ', 'ㅣ', 'ㅢ'),
];
/// Compound finals: (first, second, joined).
const TAIL_PAIRS: &[(char, char, char)] = &[
    ('ㄱ', 'ㅅ', 'ㄳ'), ('ㄴ', 'ㅈ', 'ㄵ'), ('ㄴ', 'ㅎ', 'ㄶ'), ('ㄹ', 'ㄱ', 'ㄺ'), ('ㄹ', 'ㅁ', 'ㄻ'),
    ('ㄹ', 'ㅂ', 'ㄼ'), ('ㄹ', 'ㅅ', 'ㄽ'), ('ㄹ', 'ㅌ', 'ㄾ'), ('ㄹ', 'ㅍ', 'ㄿ'), ('ㄹ', 'ㅎ', 'ㅀ'),
    ('ㅂ', 'ㅅ', 'ㅄ'),
];

/// The jamo a Latin key types on the two-set layout, or None for a key with no jamo.
pub fn jamo_for_key(c: char) -> Option<char> {
    Some(match c {
        'r' => 'ㄱ', 'R' => 'ㄲ', 's' | 'S' => 'ㄴ', 'e' => 'ㄷ', 'E' => 'ㄸ', 'f' | 'F' => 'ㄹ', 'a' | 'A' => 'ㅁ',
        'q' => 'ㅂ', 'Q' => 'ㅃ', 't' => 'ㅅ', 'T' => 'ㅆ', 'd' | 'D' => 'ㅇ', 'w' => 'ㅈ', 'W' => 'ㅉ',
        'c' | 'C' => 'ㅊ', 'z' | 'Z' => 'ㅋ', 'x' | 'X' => 'ㅌ', 'v' | 'V' => 'ㅍ', 'g' | 'G' => 'ㅎ',
        'k' | 'K' => 'ㅏ', 'o' => 'ㅐ', 'i' | 'I' => 'ㅑ', 'O' => 'ㅒ', 'j' | 'J' => 'ㅓ', 'p' => 'ㅔ',
        'u' | 'U' => 'ㅕ', 'P' => 'ㅖ', 'h' | 'H' => 'ㅗ', 'y' | 'Y' => 'ㅛ', 'n' | 'N' => 'ㅜ', 'b' | 'B' => 'ㅠ',
        'm' | 'M' => 'ㅡ', 'l' | 'L' => 'ㅣ',
        _ => return None,
    })
}

fn is_vowel(j: char) -> bool {
    VOWELS.contains(&j)
}

fn pair(table: &[(char, char, char)], a: char, b: char) -> Option<char> {
    table.iter().find(|(x, y, _)| *x == a && *y == b).map(|(_, _, j)| *j)
}

fn split(table: &[(char, char, char)], joined: char) -> Option<(char, char)> {
    table.iter().find(|(_, _, j)| *j == joined).map(|(a, b, _)| (*a, *b))
}

/// The syllable block (or the lone jamo) the parts make.
fn block(lead: Option<char>, vowel: Option<char>, tail: Option<char>) -> Option<char> {
    match (lead, vowel) {
        (Some(l), Some(v)) => {
            let li = LEADS.iter().position(|c| *c == l)?;
            let vi = VOWELS.iter().position(|c| *c == v)?;
            let ti = match tail {
                Some(t) => TAILS.iter().position(|c| *c == t)? + 1,
                None => 0,
            };
            char::from_u32(0xAC00 + ((li * 21 + vi) * 28 + ti) as u32)
        }
        (Some(l), None) => Some(l),
        (None, Some(v)) => Some(v),
        (None, None) => None,
    }
}

/// The composition: finished syllables and the block under construction.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HangulMethod {
    done: String,
    lead: Option<char>,
    vowel: Option<char>,
    tail: Option<char>,
}

impl HangulMethod {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_composing(&self) -> bool {
        !self.done.is_empty() || self.lead.is_some() || self.vowel.is_some()
    }

    /// Moves the block under construction to the finished text.
    fn flush(&mut self) {
        if let Some(c) = block(self.lead, self.vowel, self.tail) {
            self.done.push(c);
        }
        self.lead = None;
        self.vowel = None;
        self.tail = None;
    }

    /// Feeds one typed key. A key with a jamo is joined into the composition; any other key is refused (`false`) and
    /// leaves the composition as it was.
    pub fn push(&mut self, key: char) -> bool {
        match jamo_for_key(key) {
            Some(j) if is_vowel(j) => self.push_vowel(j),
            Some(j) => self.push_consonant(j),
            None => return false,
        }
        true
    }

    fn push_consonant(&mut self, j: char) {
        match (self.lead, self.vowel, self.tail) {
            (Some(_), Some(_), None) if TAILS.contains(&j) => self.tail = Some(j),
            (Some(_), Some(_), Some(t)) => match pair(TAIL_PAIRS, t, j) {
                Some(joined) => self.tail = Some(joined),
                None => self.start_block(j),
            },
            _ => self.start_block(j),
        }
    }

    /// Finishes the current block and starts a new one with the consonant `j` as its lead.
    fn start_block(&mut self, j: char) {
        self.flush();
        if LEADS.contains(&j) {
            self.lead = Some(j);
        } else {
            self.done.push(j);
        }
    }

    fn push_vowel(&mut self, j: char) {
        match (self.lead, self.vowel, self.tail) {
            (Some(_), None, _) => self.vowel = Some(j),
            (_, Some(v), None) => match pair(VOWEL_PAIRS, v, j) {
                Some(joined) => self.vowel = Some(joined),
                None => {
                    self.flush();
                    self.vowel = Some(j);
                }
            },
            (Some(_), Some(_), Some(t)) => {
                // the last consonant of the final starts the next block
                let (keep, moved) = match split(TAIL_PAIRS, t) {
                    Some((first, second)) => (Some(first), second),
                    None => (None, t),
                };
                self.tail = keep;
                self.flush();
                self.lead = Some(moved);
                self.vowel = Some(j);
            }
            _ => {
                self.flush();
                self.vowel = Some(j);
            }
        }
    }

    /// Undoes the last jamo. False when nothing is composed.
    pub fn backspace(&mut self) -> bool {
        if let Some(t) = self.tail {
            self.tail = split(TAIL_PAIRS, t).map(|(first, _)| first);
        } else if let Some(v) = self.vowel {
            self.vowel = split(VOWEL_PAIRS, v).map(|(first, _)| first);
        } else if self.lead.is_some() {
            self.lead = None;
        } else {
            return self.done.pop().is_some();
        }
        true
    }

    /// The text as the field shows it: the finished syllables, then the block under construction.
    pub fn marked(&self) -> String {
        let mut s = self.done.clone();
        if let Some(c) = block(self.lead, self.vowel, self.tail) {
            s.push(c);
        }
        s
    }

    /// Ends the composition and returns what it held, or None when nothing was composed.
    pub fn take(&mut self) -> Option<String> {
        if !self.is_composing() {
            return None;
        }
        let text = self.marked();
        *self = Self::default();
        Some(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(s: &str) -> HangulMethod {
        let mut m = HangulMethod::new();
        for c in s.chars() {
            assert!(m.push(c), "{c} is taken");
        }
        m
    }

    #[test]
    fn keys_map_to_two_set_jamo() {
        assert_eq!(jamo_for_key('r'), Some('ㄱ'));
        assert_eq!(jamo_for_key('R'), Some('ㄲ'));
        assert_eq!(jamo_for_key('k'), Some('ㅏ'));
        assert_eq!(jamo_for_key('1'), None);
    }

    #[test]
    fn syllables_follow_the_block_formula() {
        assert_eq!(typed("rk").marked(), "가");
        assert_eq!(typed("gks").marked(), "한");
        assert_eq!(typed("rkr").marked(), "각");
        assert_eq!(typed("Rk").marked(), "까");
    }

    #[test]
    fn a_word_moves_the_final_to_the_next_block_on_a_vowel() {
        assert_eq!(typed("gksrmf").marked(), "한글");
        assert_eq!(typed("dkssk").marked(), "안나");
        assert_eq!(typed("rkt").marked(), "갓");
        assert_eq!(typed("rktk").marked(), "가사");
    }

    #[test]
    fn compound_vowels_join() {
        assert_eq!(typed("hk").marked(), "ㅘ");
        assert_eq!(typed("rhk").marked(), "과");
        assert_eq!(typed("rnl").marked(), "귀");
        assert_eq!(typed("dml").marked(), "의");
    }

    #[test]
    fn compound_finals_join_and_split_on_a_vowel() {
        assert_eq!(typed("dkqt").marked(), "없");
        assert_eq!(typed("dkqtk").marked(), "압사", "the second half of a compound final moves to the next block");
        assert_eq!(typed("rkfr").marked(), "갉");
    }

    #[test]
    fn a_consonant_that_cannot_end_a_block_starts_one() {
        assert_eq!(typed("rkQ").marked(), "가ㅃ", "ㅃ has no final form");
        assert_eq!(typed("rr").marked(), "ㄱㄱ");
    }

    #[test]
    fn backspace_undoes_one_jamo_at_a_time() {
        let mut m = typed("dkqt");
        assert_eq!(m.marked(), "없");
        assert!(m.backspace());
        assert_eq!(m.marked(), "압");
        assert!(m.backspace());
        assert_eq!(m.marked(), "아");
        assert!(m.backspace());
        assert_eq!(m.marked(), "ㅇ");
        assert!(m.backspace());
        assert!(!m.backspace());
        assert!(!m.is_composing());
    }

    #[test]
    fn take_returns_the_text_and_clears() {
        let mut m = typed("gks");
        assert_eq!(m.take(), Some("한".to_string()));
        assert_eq!(m.take(), None);
    }

    #[test]
    fn keys_without_jamo_are_refused() {
        let mut m = typed("rk");
        assert!(!m.push(' '));
        assert_eq!(m.marked(), "가");
    }
}
