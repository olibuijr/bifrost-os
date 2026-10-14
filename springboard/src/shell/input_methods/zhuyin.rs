//! Traditional Chinese input by zhuyin (bopomofo, the TCIM source). The standard keyboard layout puts each bopomofo symbol
//! on a key. A syllable is typed as its symbols and then a tone: `3`, `4`, `6` and `7` give the third, fourth, second
//! and neutral tones, and space gives the first tone (which has no mark). The tone ends the syllable: the syllable is
//! looked up in the seed table and the first candidate is inserted, or the bopomofo as typed when there is none.
//!
//! The seed table (`han::Entry`) is short; it is data, and a larger table replaces it without changing the composer.

use super::han::{candidates, Entry};
use super::Composer;

/// The symbol each key types on the standard zhuyin layout. Tone keys are not symbols and are handled separately.
const SYMBOLS: &[(char, char)] = &[
    ('1', 'ㄅ'), ('2', 'ㄉ'), ('5', 'ㄓ'), ('8', 'ㄚ'), ('9', 'ㄞ'), ('0', 'ㄢ'), ('-', 'ㄦ'),
    ('q', 'ㄆ'), ('w', 'ㄊ'), ('e', 'ㄍ'), ('r', 'ㄐ'), ('t', 'ㄔ'), ('y', 'ㄗ'), ('u', 'ㄧ'), ('i', 'ㄛ'), ('o', 'ㄟ'),
    ('p', 'ㄣ'),
    ('a', 'ㄇ'), ('s', 'ㄋ'), ('d', 'ㄎ'), ('f', 'ㄑ'), ('g', 'ㄕ'), ('h', 'ㄘ'), ('j', 'ㄨ'), ('k', 'ㄜ'), ('l', 'ㄠ'),
    (';', 'ㄤ'),
    ('z', 'ㄈ'), ('x', 'ㄌ'), ('c', 'ㄏ'), ('v', 'ㄒ'), ('b', 'ㄖ'), ('n', 'ㄙ'), ('m', 'ㄩ'), (',', 'ㄝ'), ('.', 'ㄡ'),
    ('/', 'ㄥ'),
];

/// Seed syllables: bopomofo with its tone mark (first tone has none), and the characters it can stand for.
pub const ZHUYIN: &[Entry] = &[
    Entry { reading: "ㄋㄧˇ", text: &["你"] },
    Entry { reading: "ㄏㄠˇ", text: &["好"] },
    Entry { reading: "ㄨㄛˇ", text: &["我"] },
    Entry { reading: "ㄉㄜ˙", text: &["的"] },
    Entry { reading: "ㄕˋ", text: &["是", "事"] },
    Entry { reading: "ㄕˊ", text: &["時"] },
    Entry { reading: "ㄓㄨㄥ", text: &["中"] },
    Entry { reading: "ㄍㄨㄛˊ", text: &["國"] },
    Entry { reading: "ㄖㄣˊ", text: &["人"] },
    Entry { reading: "ㄉㄚˋ", text: &["大"] },
    Entry { reading: "ㄧ", text: &["一"] },
    Entry { reading: "ㄅㄨˋ", text: &["不"] },
    Entry { reading: "ㄒㄧㄝˋ", text: &["謝"] },
];

/// The mark a tone key adds, or None when the key is not a tone key. Space (the first tone) adds no mark.
fn tone_mark(key: char) -> Option<&'static str> {
    match key {
        '3' => Some("ˇ"),
        '4' => Some("ˋ"),
        '6' => Some("ˊ"),
        '7' => Some("˙"),
        _ => None,
    }
}

/// The symbol a key types, or None.
fn symbol_of(key: char) -> Option<char> {
    SYMBOLS.iter().find(|(k, _)| *k == key).map(|(_, s)| *s)
}

/// The text of a finished syllable: its first candidate, or the bopomofo as typed.
fn finish(symbols: &str, mark: &str) -> String {
    let reading = format!("{symbols}{mark}");
    candidates(ZHUYIN, &reading).first().map(|s| s.to_string()).unwrap_or(reading)
}

/// The zhuyin composer: symbols build the current syllable, a tone key or space finishes it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ZhuyinComposer {
    symbols: String,
}

impl Composer for ZhuyinComposer {
    fn feed(&mut self, ch: char) -> Option<String> {
        if let Some(symbol) = symbol_of(ch) {
            self.symbols.push(symbol);
            return Some(String::new());
        }
        if self.symbols.is_empty() {
            return None;
        }
        let mark = if ch == ' ' { "" } else { tone_mark(ch)? };
        let out = finish(&self.symbols, mark);
        self.symbols.clear();
        Some(out)
    }

    fn backspace(&mut self) -> bool {
        self.symbols.pop().is_some()
    }

    fn composed(&self) -> String {
        self.symbols.clone()
    }

    fn reset(&mut self) {
        self.symbols.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Types keys into the composer and collects what each finished syllable inserted.
    fn typed(keys: &str) -> (ZhuyinComposer, Vec<String>) {
        let mut c = ZhuyinComposer::default();
        let mut out = Vec::new();
        for ch in keys.chars() {
            if let Some(done) = c.feed(ch) {
                if !done.is_empty() {
                    out.push(done);
                }
            }
        }
        (c, out)
    }

    #[test]
    fn a_tone_key_finishes_the_syllable_with_its_character() {
        // s = ㄋ, u = ㄧ, 3 = third tone: 你
        let (c, out) = typed("su3");
        assert_eq!(out, vec!["你".to_string()]);
        assert_eq!(c.composed(), "");
    }

    #[test]
    fn space_gives_the_first_tone() {
        let (_, out) = typed("u ");
        assert_eq!(out, vec!["一".to_string()]);
    }

    #[test]
    fn a_neutral_tone_and_a_second_syllable_are_separate_words() {
        // 2 = ㄉ, k = ㄜ, 7 = neutral tone: 的
        let (_, out) = typed("2k7");
        assert_eq!(out, vec!["的".to_string()]);
        // c = ㄏ, l = ㄠ, 3: 好
        let (_, out) = typed("cl3");
        assert_eq!(out, vec!["好".to_string()]);
    }

    #[test]
    fn a_syllable_without_a_seed_entry_is_inserted_as_typed() {
        let (_, out) = typed("1q ");
        assert_eq!(out, vec!["ㄅㄆ".to_string()]);
    }

    #[test]
    fn a_tone_key_with_nothing_typed_is_literal() {
        let mut c = ZhuyinComposer::default();
        assert_eq!(c.feed('3'), None);
        assert_eq!(c.feed(' '), None);
    }

    #[test]
    fn partial_symbols_are_marked_and_backspace_removes_one() {
        let (mut c, _) = typed("su");
        assert_eq!(c.composed(), "ㄋㄧ");
        assert!(c.backspace());
        assert_eq!(c.composed(), "ㄋ");
        c.reset();
        assert!(!c.backspace());
    }

    #[test]
    fn the_seed_readings_use_the_tone_marks_of_their_tones() {
        assert!(ZHUYIN.iter().all(|e| e.reading.chars().all(|c| c != '3' && c != '4')));
        assert_eq!(tone_mark('6'), Some("ˊ"));
        assert_eq!(tone_mark('x'), None);
    }
}
