//! Phonetic Indic input: Latin letters are romanized sounds, and the composer turns them into the script (the TamilIM
//! and TransliterationIM sources). Two scripts are tabled: Tamil, and Devanagari (Hindi).
//!
//! A consonant carries the inherent vowel `a` unless a vowel key follows it, which replaces the inherent vowel with
//! its vowel sign. A vowel with no consonant before it is written as the independent vowel letter. Two consonants with
//! no vowel between them get a virama between them, so the first loses its vowel. A word that ends on a consonant keeps
//! the inherent vowel (no virama). Keys are case sensitive: the capitals are the retroflex and the Tamil letters.
//! The romanization is a simple phonetic one, not the Tamil99 typewriter layout and not ISO 15919.

use super::Composer;

/// A script's romanization: consonant keys, vowel keys (key, independent letter, vowel sign; a `\0` sign means the
/// inherent vowel) and the virama.
#[derive(Debug)]
pub struct Script {
    pub consonants: &'static [(&'static str, char)],
    pub vowels: &'static [(&'static str, char, char)],
    pub virama: char,
}

/// Tamil: the stops are written with the plain letters (no aspirated or voiced letters of their own).
pub static TAMIL: Script = Script {
    consonants: &[
        ("ng", 'ங'), ("nj", 'ஞ'), ("ch", 'ச'), ("sh", 'ஷ'), ("zh", 'ழ'), ("th", 'த'), ("dh", 'த'),
        ("k", 'க'), ("g", 'க'), ("c", 'ச'), ("j", 'ஜ'), ("s", 'ஸ'), ("h", 'ஹ'), ("t", 'ட'), ("d", 'ட'),
        ("N", 'ண'), ("n", 'ந'), ("p", 'ப'), ("b", 'ப'), ("m", 'ம'), ("y", 'ய'), ("r", 'ர'), ("R", 'ற'),
        ("l", 'ல'), ("L", 'ள'), ("v", 'வ'), ("w", 'வ'),
    ],
    vowels: &[
        ("a", 'அ', '\0'), ("aa", 'ஆ', 'ா'), ("i", 'இ', 'ி'), ("ii", 'ஈ', 'ீ'), ("u", 'உ', 'ு'), ("uu", 'ஊ', 'ூ'),
        ("e", 'எ', 'ெ'), ("ee", 'ஏ', 'ே'), ("ai", 'ஐ', 'ை'), ("o", 'ஒ', 'ொ'), ("oo", 'ஓ', 'ோ'), ("au", 'ஔ', 'ௌ'),
    ],
    virama: '்',
};

/// Devanagari for Hindi: the aspirates are the two-letter keys, and the capitals are the retroflex letters.
pub static DEVANAGARI: Script = Script {
    consonants: &[
        ("chh", 'छ'), ("ch", 'च'), ("kh", 'ख'), ("gh", 'घ'), ("ng", 'ङ'), ("jh", 'झ'), ("j", 'ज'), ("ny", 'ञ'),
        ("Th", 'ठ'), ("Dh", 'ढ'), ("T", 'ट'), ("D", 'ड'), ("N", 'ण'), ("th", 'थ'), ("dh", 'ध'), ("t", 'त'),
        ("d", 'द'), ("n", 'न'), ("ph", 'फ'), ("bh", 'भ'), ("p", 'प'), ("b", 'ब'), ("m", 'म'), ("y", 'य'),
        ("r", 'र'), ("l", 'ल'), ("v", 'व'), ("w", 'व'), ("sh", 'श'), ("s", 'स'), ("h", 'ह'), ("k", 'क'), ("g", 'ग'),
    ],
    vowels: &[
        ("a", 'अ', '\0'), ("aa", 'आ', 'ा'), ("i", 'इ', 'ि'), ("ii", 'ई', 'ी'), ("u", 'उ', 'ु'), ("uu", 'ऊ', 'ू'),
        ("e", 'ए', 'े'), ("ai", 'ऐ', 'ै'), ("o", 'ओ', 'ो'), ("au", 'औ', 'ौ'),
    ],
    virama: '्',
};

/// One step of the romanized word.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Token {
    Consonant(char),
    Vowel(char, char),
    Literal(char),
}

/// The longest key at the start of `rest` (its length in bytes and the token it stands for). A letter that starts no
/// key is a literal of one byte, since the composer only takes ASCII letters.
fn next_token(script: &Script, rest: &str) -> (usize, Token) {
    let mut best: Option<(usize, Token)> = None;
    for &(key, c) in script.consonants {
        if rest.starts_with(key) && best.map_or(true, |(n, _)| key.len() > n) {
            best = Some((key.len(), Token::Consonant(c)));
        }
    }
    for &(key, independent, sign) in script.vowels {
        if rest.starts_with(key) && best.map_or(true, |(n, _)| key.len() > n) {
            best = Some((key.len(), Token::Vowel(independent, sign)));
        }
    }
    best.unwrap_or_else(|| {
        let c = rest.chars().next().unwrap_or('\0');
        (c.len_utf8(), Token::Literal(c))
    })
}

/// Writes a romanized word in a script.
pub fn render(script: &Script, roman: &str) -> String {
    let mut out = String::new();
    let mut open = false; // the last thing written is a consonant that still carries its inherent vowel
    let mut pos = 0;
    while pos < roman.len() {
        let (len, token) = next_token(script, &roman[pos..]);
        pos += len;
        match token {
            Token::Consonant(c) => {
                if open {
                    out.push(script.virama);
                }
                out.push(c);
                open = true;
            }
            Token::Vowel(independent, sign) => {
                if open {
                    if sign != '\0' {
                        out.push(sign);
                    }
                    open = false;
                } else {
                    out.push(independent);
                }
            }
            Token::Literal(c) => {
                out.push(c);
                open = false;
            }
        }
    }
    out
}

/// The composer for one script: letters build the romanized word, which is shown in the script as it grows.
#[derive(Clone, Debug)]
pub struct IndicComposer {
    script: &'static Script,
    word: String,
}

impl IndicComposer {
    pub fn new(script: &'static Script) -> Self {
        Self { script, word: String::new() }
    }
}

impl Composer for IndicComposer {
    fn feed(&mut self, ch: char) -> Option<String> {
        if ch.is_ascii_alphabetic() {
            self.word.push(ch);
            Some(String::new())
        } else {
            None
        }
    }

    fn backspace(&mut self) -> bool {
        self.word.pop().is_some()
    }

    fn composed(&self) -> String {
        render(self.script, &self.word)
    }

    fn reset(&mut self) {
        self.word.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tamil(word: &str) -> String {
        render(&TAMIL, word)
    }

    fn devanagari(word: &str) -> String {
        render(&DEVANAGARI, word)
    }

    #[test]
    fn a_consonant_carries_the_inherent_vowel_until_a_vowel_key_replaces_it() {
        assert_eq!(tamil("ka"), "க");
        assert_eq!(tamil("kaa"), "கா");
        assert_eq!(tamil("ki"), "கி");
        assert_eq!(devanagari("ka"), "क");
        assert_eq!(devanagari("kaa"), "का");
    }

    #[test]
    fn a_vowel_without_a_consonant_is_the_independent_letter() {
        assert_eq!(tamil("a"), "அ");
        assert_eq!(devanagari("ai"), "ऐ");
        assert_eq!(devanagari("aam"), "आम");
    }

    #[test]
    fn consonants_in_a_row_get_a_virama_between_them() {
        // Devanagari namaste: n a m a s t e
        assert_eq!(devanagari("namaste"), "नमस्ते");
    }

    #[test]
    fn a_word_ending_on_a_consonant_keeps_the_inherent_vowel() {
        assert_eq!(tamil("tamil"), "டமில");
        assert_eq!(devanagari("ram"), "रम");
    }

    #[test]
    fn two_letter_keys_win_over_their_single_letters() {
        // ng is one Tamil letter, so "nga" is that letter with its inherent vowel, and "ngaa" adds the long sign.
        assert_eq!(tamil("nga"), "ங");
        assert_eq!(tamil("ngaa"), "ங".to_string() + "ா");
        assert_eq!(devanagari("chh"), "छ", "chh is one key, not ch and h");
    }

    #[test]
    fn case_selects_the_retroflex_key_in_devanagari() {
        assert_eq!(devanagari("T"), "ट");
        assert_eq!(devanagari("t"), "त");
        assert_eq!(devanagari("Th"), "ठ");
    }

    #[test]
    fn unknown_letters_are_kept_as_typed() {
        assert_eq!(tamil("q"), "q");
    }

    #[test]
    fn the_composer_shows_the_word_and_backspace_removes_a_letter() {
        let mut c = IndicComposer::new(&DEVANAGARI);
        for ch in "namas".chars() {
            assert_eq!(c.feed(ch), Some(String::new()));
        }
        assert_eq!(c.composed(), "नमस");
        assert!(c.backspace());
        assert_eq!(c.composed(), "नम");
        assert_eq!(c.feed(' '), None, "a space ends the composition");
        c.reset();
        assert!(!c.backspace());
    }
}
