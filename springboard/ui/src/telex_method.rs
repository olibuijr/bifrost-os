//! The built-in Vietnamese method (Telex typing): plain Latin keys become the letters with shape marks and tone marks.
//! `aa` `ee` `oo` give the circumflex vowels, `aw` `ow` `uw` the breve and horn vowels, `dd` gives đ, and the tone keys
//! `s` `f` `r` `x` `j` put the acute, grave, hook, tilde and dot-below tones on the word (`z` removes the tone). The
//! tone mark is placed on the vowel the standard orthography rules choose. Lower-case keys are interpreted; any other
//! character is kept as typed. No dictionary is involved. Pure model: it draws nothing.

/// The tone variants of each vowel: none, acute, grave, hook above, tilde, dot below.
const TONES: &[(char, [char; 6])] = &[
    ('a', ['a', 'á', 'à', 'ả', 'ã', 'ạ']),
    ('ă', ['ă', 'ắ', 'ằ', 'ẳ', 'ẵ', 'ặ']),
    ('â', ['â', 'ấ', 'ầ', 'ẩ', 'ẫ', 'ậ']),
    ('e', ['e', 'é', 'è', 'ẻ', 'ẽ', 'ẹ']),
    ('ê', ['ê', 'ế', 'ề', 'ể', 'ễ', 'ệ']),
    ('i', ['i', 'í', 'ì', 'ỉ', 'ĩ', 'ị']),
    ('o', ['o', 'ó', 'ò', 'ỏ', 'õ', 'ọ']),
    ('ô', ['ô', 'ố', 'ồ', 'ổ', 'ỗ', 'ộ']),
    ('ơ', ['ơ', 'ớ', 'ờ', 'ở', 'ỡ', 'ợ']),
    ('u', ['u', 'ú', 'ù', 'ủ', 'ũ', 'ụ']),
    ('ư', ['ư', 'ứ', 'ừ', 'ử', 'ữ', 'ự']),
    ('y', ['y', 'ý', 'ỳ', 'ỷ', 'ỹ', 'ỵ']),
];

/// The tone a key types (index into a [`TONES`] row), or None.
fn tone_of_key(c: char) -> Option<usize> {
    Some(match c {
        's' => 1,
        'f' => 2,
        'r' => 3,
        'x' => 4,
        'j' => 5,
        _ => return None,
    })
}

fn is_vowel(c: char) -> bool {
    TONES.iter().any(|(b, _)| *b == c)
}

/// Shape marks are those that change the vowel itself (breve, circumflex, horn).
fn is_marked_vowel(c: char) -> bool {
    matches!(c, 'ă' | 'â' | 'ê' | 'ô' | 'ơ' | 'ư')
}

/// The composition: the word's letters (shape applied, tone not) and its tone.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TelexMethod {
    letters: Vec<char>,
    tone: usize,
}

impl TelexMethod {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_composing(&self) -> bool {
        !self.letters.is_empty()
    }

    /// Feeds one typed character. Always taken: a character with no Telex meaning is added as typed.
    pub fn push(&mut self, c: char) -> bool {
        if self.apply_shape(c) || self.apply_tone(c) {
            return true;
        }
        self.letters.push(c);
        true
    }

    /// Doubled or modifier keys that reshape the last matching letter.
    fn apply_shape(&mut self, c: char) -> bool {
        let target = match c {
            'a' | 'e' | 'o' => self.letters.iter().rposition(|l| *l == c).filter(|i| *i + 1 == self.last_vowel_end()),
            'w' => self.letters.iter().rposition(|l| matches!(*l, 'a' | 'o' | 'u')),
            'd' => self.letters.iter().rposition(|l| *l == 'd'),
            _ => None,
        };
        let Some(i) = target else { return false };
        let from = self.letters[i];
        let to = match (c, from) {
            ('a', 'a') => 'â',
            ('e', 'e') => 'ê',
            ('o', 'o') => 'ô',
            ('w', 'a') => 'ă',
            ('w', 'o') => 'ơ',
            ('w', 'u') => 'ư',
            ('d', 'd') => 'đ',
            _ => return false,
        };
        // "uow" is ươ: the horn goes on both vowels
        if c == 'w' && from == 'o' && i > 0 && self.letters[i - 1] == 'u' {
            self.letters[i - 1] = 'ư';
        }
        self.letters[i] = to;
        true
    }

    /// The index just past the last vowel (0 when there is none): a doubled vowel key only reshapes the vowel it follows.
    fn last_vowel_end(&self) -> usize {
        self.letters.iter().rposition(|l| is_vowel(*l)).map_or(0, |i| i + 1)
    }

    fn apply_tone(&mut self, c: char) -> bool {
        if !self.letters.iter().any(|l| is_vowel(*l)) {
            return false;
        }
        if c == 'z' {
            self.tone = 0;
            return true;
        }
        match tone_of_key(c) {
            Some(t) => {
                self.tone = t;
                true
            }
            None => false,
        }
    }

    /// The index of the vowel that carries the tone, by the standard placement rules.
    fn tone_position(&self) -> Option<usize> {
        let l = &self.letters;
        // a "u" after q and an "i" after g (before another vowel) belong to the consonant
        let skip = |i: usize| (l[i] == 'u' && i > 0 && l[i - 1] == 'q') || (l[i] == 'i' && i > 0 && l[i - 1] == 'g' && l.get(i + 1).is_some_and(|n| is_vowel(*n)));
        let end = l.iter().rposition(|c| is_vowel(*c))?;
        let mut start = end;
        while start > 0 && is_vowel(l[start - 1]) {
            start -= 1;
        }
        while start < end && skip(start) {
            start += 1;
        }
        let cluster: Vec<usize> = (start..=end).collect();
        if let Some(i) = cluster.iter().rev().find(|i| is_marked_vowel(l[**i])) {
            return Some(*i);
        }
        let closed = end + 1 < l.len();
        Some(match cluster.len() {
            1 => cluster[0],
            2 => {
                let pair: String = cluster.iter().map(|i| l[*i]).collect();
                if closed || matches!(pair.as_str(), "oa" | "oe" | "uy") {
                    cluster[1]
                } else {
                    cluster[0]
                }
            }
            _ => cluster[1],
        })
    }

    /// Removes the last character. False when nothing is composed. A removed character also drops the tone when no
    /// vowel is left.
    pub fn backspace(&mut self) -> bool {
        if self.letters.pop().is_none() {
            return false;
        }
        if !self.letters.iter().any(|l| is_vowel(*l)) {
            self.tone = 0;
        }
        true
    }

    /// The word as the field shows it.
    pub fn marked(&self) -> String {
        let at = if self.tone > 0 { self.tone_position() } else { None };
        self.letters
            .iter()
            .enumerate()
            .map(|(i, c)| match (at, TONES.iter().find(|(b, _)| b == c)) {
                (Some(p), Some((_, row))) if p == i => row[self.tone],
                _ => *c,
            })
            .collect()
    }

    /// Ends the composition and returns the word, or None when nothing was composed.
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

    fn word(s: &str) -> String {
        let mut m = TelexMethod::new();
        for c in s.chars() {
            assert!(m.push(c));
        }
        m.marked()
    }

    #[test]
    fn shape_keys_make_circumflex_breve_horn_and_stroke() {
        assert_eq!(word("aa"), "â");
        assert_eq!(word("ee"), "ê");
        assert_eq!(word("oo"), "ô");
        assert_eq!(word("aw"), "ă");
        assert_eq!(word("ow"), "ơ");
        assert_eq!(word("uw"), "ư");
        assert_eq!(word("dd"), "đ");
        assert_eq!(word("duwowng"), "dương");
    }

    #[test]
    fn tone_keys_mark_the_word() {
        assert_eq!(word("as"), "á");
        assert_eq!(word("af"), "à");
        assert_eq!(word("ar"), "ả");
        assert_eq!(word("ax"), "ã");
        assert_eq!(word("aj"), "ạ");
        assert_eq!(word("asz"), "a", "z removes the tone");
    }

    #[test]
    fn tone_keys_without_a_vowel_are_plain_letters() {
        assert_eq!(word("s"), "s");
        assert_eq!(word("xr"), "xr");
    }

    #[test]
    fn the_tone_goes_where_the_orthography_puts_it() {
        assert_eq!(word("vieetj"), "việt", "a marked vowel takes the tone");
        assert_eq!(word("tieengs"), "tiếng");
        assert_eq!(word("hoaf"), "hoà", "oa open takes the second vowel");
        assert_eq!(word("toans"), "toán", "a closed pair takes the second vowel");
        assert_eq!(word("maij"), "mại", "other open pairs take the first vowel");
        assert_eq!(word("quas"), "quá", "the u after q is a consonant");
        assert_eq!(word("gias"), "giá", "the i after g is a consonant");
        assert_eq!(word("nguwowif"), "người");
    }

    #[test]
    fn the_tone_can_be_typed_before_more_letters() {
        assert_eq!(word("vieejt"), "việt");
        assert_eq!(word("Vieetj"), "Việt");
    }

    #[test]
    fn backspace_and_take() {
        let mut m = TelexMethod::new();
        for c in "vieetj".chars() {
            m.push(c);
        }
        assert!(m.backspace());
        assert_eq!(m.marked(), "việ", "the tone stays while a vowel is left");
        assert_eq!(m.take(), Some("việ".to_string()));
        assert_eq!(m.take(), None);
        assert!(!m.backspace());
    }
}
