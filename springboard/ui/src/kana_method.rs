//! The built-in kana method: romaji typed on a Latin board becomes hiragana, and the candidate list offers the
//! hiragana and its katakana form. This is the simple Japanese input the shell runs with no engine installed (no
//! dictionary is shipped, so kanji are not offered). Pure model: it draws nothing.
//!
//! Rules: a vowel or a consonant completes its syllable (`ka` gives か), `nn` and an `n` before a consonant give ん,
//! a doubled consonant gives a small tsu (`kk` gives っ), and `-` gives the long-vowel mark. A consonant that does not
//! complete a syllable stays as pending romaji, shown after the kana.

/// Romaji to hiragana. The lookup takes the longest key that the pending romaji starts with.
const SYLLABLES: &[(&str, &str)] = &[
    ("a", "あ"), ("i", "い"), ("u", "う"), ("e", "え"), ("o", "お"),
    ("ka", "か"), ("ki", "き"), ("ku", "く"), ("ke", "け"), ("ko", "こ"),
    ("sa", "さ"), ("shi", "し"), ("si", "し"), ("su", "す"), ("se", "せ"), ("so", "そ"),
    ("ta", "た"), ("chi", "ち"), ("ti", "ち"), ("tsu", "つ"), ("tu", "つ"), ("te", "て"), ("to", "と"),
    ("na", "な"), ("ni", "に"), ("nu", "ぬ"), ("ne", "ね"), ("no", "の"),
    ("ha", "は"), ("hi", "ひ"), ("fu", "ふ"), ("hu", "ふ"), ("he", "へ"), ("ho", "ほ"),
    ("ma", "ま"), ("mi", "み"), ("mu", "む"), ("me", "め"), ("mo", "も"),
    ("ya", "や"), ("yu", "ゆ"), ("yo", "よ"),
    ("ra", "ら"), ("ri", "り"), ("ru", "る"), ("re", "れ"), ("ro", "ろ"),
    ("wa", "わ"), ("wo", "を"),
    ("ga", "が"), ("gi", "ぎ"), ("gu", "ぐ"), ("ge", "げ"), ("go", "ご"),
    ("za", "ざ"), ("ji", "じ"), ("zi", "じ"), ("zu", "ず"), ("ze", "ぜ"), ("zo", "ぞ"),
    ("da", "だ"), ("di", "ぢ"), ("du", "づ"), ("de", "で"), ("do", "ど"),
    ("ba", "ば"), ("bi", "び"), ("bu", "ぶ"), ("be", "べ"), ("bo", "ぼ"),
    ("pa", "ぱ"), ("pi", "ぴ"), ("pu", "ぷ"), ("pe", "ぺ"), ("po", "ぽ"),
    ("kya", "きゃ"), ("kyu", "きゅ"), ("kyo", "きょ"),
    ("sha", "しゃ"), ("shu", "しゅ"), ("sho", "しょ"),
    ("cha", "ちゃ"), ("chu", "ちゅ"), ("cho", "ちょ"),
    ("nya", "にゃ"), ("nyu", "にゅ"), ("nyo", "にょ"),
    ("hya", "ひゃ"), ("hyu", "ひゅ"), ("hyo", "ひょ"),
    ("mya", "みゃ"), ("myu", "みゅ"), ("myo", "みょ"),
    ("rya", "りゃ"), ("ryu", "りゅ"), ("ryo", "りょ"),
    ("gya", "ぎゃ"), ("gyu", "ぎゅ"), ("gyo", "ぎょ"),
    ("ja", "じゃ"), ("ju", "じゅ"), ("jo", "じょ"),
    ("bya", "びゃ"), ("byu", "びゅ"), ("byo", "びょ"),
    ("pya", "ぴゃ"), ("pyu", "ぴゅ"), ("pyo", "ぴょ"),
];

/// The hiragana of one romaji syllable from the table (`ka` gives か), or None when the table has no such syllable.
pub fn syllable_hiragana(romaji: &str) -> Option<&'static str> {
    SYLLABLES.iter().find(|(k, _)| *k == romaji).map(|(_, kana)| *kana)
}

/// True for the Latin vowels.
fn is_vowel(c: u8) -> bool {
    matches!(c, b'a' | b'i' | b'u' | b'e' | b'o')
}

/// True for the consonants that double into a small tsu (`n` does not: `nn` is ん).
fn is_doubling_consonant(c: u8) -> bool {
    c.is_ascii_lowercase() && !is_vowel(c) && c != b'n'
}

/// The katakana form of a hiragana string: the hiragana block maps onto the katakana block, other characters stay.
pub fn katakana_of(hiragana: &str) -> String {
    hiragana
        .chars()
        .map(|c| match c {
            '\u{3041}'..='\u{3096}' => char::from_u32(c as u32 + 0x60).unwrap_or(c),
            _ => c,
        })
        .collect()
}

/// The composition: converted kana and the romaji still waiting for its vowel.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KanaMethod {
    kana: String,
    romaji: String,
}

impl KanaMethod {
    pub fn new() -> Self {
        Self::default()
    }

    /// True while anything is composed (converted or pending).
    pub fn is_composing(&self) -> bool {
        !self.kana.is_empty() || !self.romaji.is_empty()
    }

    /// Feeds one typed character. A Latin letter is converted as far as it can be, and `-` gives the long-vowel mark;
    /// anything else is refused (`false`) and leaves the composition as it was.
    pub fn push(&mut self, c: char) -> bool {
        if c == '-' {
            // a lone pending n is ん before the long vowel mark
            if self.romaji == "n" {
                self.kana.push('ん');
                self.romaji.clear();
            }
            self.kana.push('ー');
            return true;
        }
        if !c.is_ascii_alphabetic() {
            return false;
        }
        self.romaji.push(c.to_ascii_lowercase());
        self.convert();
        true
    }

    /// Converts whatever the pending romaji completes, and stops at the first romaji that is not yet a syllable.
    fn convert(&mut self) {
        loop {
            let bytes = self.romaji.as_bytes();
            if bytes.is_empty() {
                return;
            }
            if bytes.len() >= 2 && bytes[0] == bytes[1] && is_doubling_consonant(bytes[0]) {
                self.kana.push('っ');
                self.romaji.remove(0);
                continue;
            }
            if bytes[0] == b'n' {
                if bytes.len() >= 2 && bytes[1] == b'n' {
                    self.kana.push('ん');
                    self.romaji.drain(..2);
                    continue;
                }
                if bytes.len() >= 2 && !is_vowel(bytes[1]) && bytes[1] != b'y' {
                    self.kana.push('ん');
                    self.romaji.remove(0);
                    continue;
                }
            }
            let longest = SYLLABLES.iter().filter(|(k, _)| self.romaji.starts_with(k)).max_by_key(|(k, _)| k.len());
            match longest {
                Some((k, kana)) => {
                    self.kana.push_str(kana);
                    self.romaji.drain(..k.len());
                }
                None => return,
            }
        }
    }

    /// Removes the last pending romaji letter, or else the last kana. False when nothing is composed.
    pub fn backspace(&mut self) -> bool {
        if self.romaji.pop().is_some() {
            return true;
        }
        self.kana.pop().is_some()
    }

    /// The text as the field shows it: the kana, then the pending romaji.
    pub fn marked(&self) -> String {
        format!("{}{}", self.kana, self.romaji)
    }

    /// The romaji still waiting for its vowel: shown after the kana and kept as typed when a candidate is picked.
    pub fn pending(&self) -> &str {
        &self.romaji
    }

    /// The candidate list for the composition: its hiragana, then its katakana. Empty when no kana is composed.
    pub fn candidates(&self) -> Vec<String> {
        if self.kana.is_empty() {
            return Vec::new();
        }
        vec![self.kana.clone(), katakana_of(&self.kana)]
    }

    /// Ends the composition and returns what it held (kana and any pending romaji), or None when nothing was.
    pub fn take(&mut self) -> Option<String> {
        if !self.is_composing() {
            return None;
        }
        let text = self.marked();
        self.kana.clear();
        self.romaji.clear();
        Some(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(s: &str) -> KanaMethod {
        let mut m = KanaMethod::new();
        for c in s.chars() {
            assert!(m.push(c), "{c} is taken");
        }
        m
    }

    #[test]
    fn a_syllable_completes_on_its_vowel() {
        assert_eq!(typed("ka").marked(), "か");
        assert_eq!(typed("shi").marked(), "し");
        assert_eq!(typed("kya").marked(), "きゃ");
        assert_eq!(typed("a").marked(), "あ");
    }

    #[test]
    fn a_consonant_waits_for_its_vowel_and_shows_as_pending_romaji() {
        let m = typed("k");
        assert_eq!(m.marked(), "k");
        assert!(m.is_composing());
        assert!(m.candidates().is_empty(), "no kana yet, so no list");
    }

    #[test]
    fn a_word_ending_in_n_keeps_the_n_pending() {
        // "nihon": に and ほ are kana, the final n waits for a vowel or a second n
        assert_eq!(typed("nihon").marked(), "にほn");
        assert_eq!(typed("nihonn").marked(), "にほん");
    }

    #[test]
    fn a_doubled_consonant_gives_a_small_tsu() {
        assert_eq!(typed("kitte").marked(), "きって");
        assert_eq!(typed("tt").marked(), "っt");
    }

    #[test]
    fn n_before_a_consonant_or_a_second_n_gives_the_syllabic_n() {
        assert_eq!(typed("nn").marked(), "ん");
        assert_eq!(typed("kanji").marked(), "かんじ");
        assert_eq!(typed("nya").marked(), "にゃ", "ny is a syllable, not ん plus y");
    }

    #[test]
    fn a_dash_gives_the_long_vowel_mark_and_takes_a_pending_n() {
        assert_eq!(typed("ra-men").marked(), "らーめn");
        let mut m = typed("n");
        assert!(m.push('-'));
        assert_eq!(m.marked(), "んー");
    }

    #[test]
    fn candidates_are_hiragana_then_katakana() {
        let m = typed("ka");
        assert_eq!(m.candidates(), vec!["か".to_string(), "カ".to_string()]);
        assert_eq!(katakana_of("ぁあゔ"), "ァアヴ", "the hiragana block maps onto the katakana block");
    }

    #[test]
    fn backspace_removes_romaji_before_kana_and_reports_empty() {
        let mut m = typed("kak");
        assert_eq!(m.marked(), "かk");
        assert!(m.backspace());
        assert_eq!(m.marked(), "か");
        assert!(m.backspace());
        assert_eq!(m.marked(), "");
        assert!(!m.backspace());
        assert!(!m.is_composing());
    }

    #[test]
    fn take_returns_the_composition_and_clears_it() {
        let mut m = typed("sushi");
        assert_eq!(m.take(), Some("すし".to_string()));
        assert_eq!(m.take(), None);
        assert!(!m.is_composing());
    }

    #[test]
    fn characters_that_are_not_letters_are_refused() {
        let mut m = KanaMethod::new();
        assert!(!m.push(' '));
        assert!(!m.push('1'));
        assert!(!m.is_composing());
    }
}
