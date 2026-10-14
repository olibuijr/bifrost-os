//! Simplified Chinese input by pinyin (the SCIM source): the letters typed are a pinyin reading, and the candidates for
//! that reading come from a seed table. Space picks the first candidate, and a digit from 1 to 9 picks that candidate.
//! A reading with no candidate stays as typed, and a space or other key then ends the composition as typed.
//!
//! The seed table is a short list of common readings. No full dictionary is shipped, so most readings have no
//! candidate yet; the table is data, and a larger dictionary replaces it without changing the composer.

use super::Composer;

/// One reading and the characters it can stand for, most likely first.
pub struct Entry {
    pub reading: &'static str,
    pub text: &'static [&'static str],
}

/// Seed readings for pinyin (tones are not typed; a reading is letters only).
pub const PINYIN: &[Entry] = &[
    Entry { reading: "ni", text: &["你", "呢", "尼"] },
    Entry { reading: "nihao", text: &["你好"] },
    Entry { reading: "hao", text: &["好", "号", "毫"] },
    Entry { reading: "wo", text: &["我", "窝"] },
    Entry { reading: "de", text: &["的", "得", "地"] },
    Entry { reading: "shi", text: &["是", "时", "事", "十"] },
    Entry { reading: "zhong", text: &["中", "种", "重"] },
    Entry { reading: "guo", text: &["国", "过"] },
    Entry { reading: "zhongguo", text: &["中国"] },
    Entry { reading: "ren", text: &["人", "认"] },
    Entry { reading: "da", text: &["大", "达"] },
    Entry { reading: "yi", text: &["一", "以", "已"] },
    Entry { reading: "bu", text: &["不", "部"] },
    Entry { reading: "xie", text: &["谢", "些", "写"] },
    Entry { reading: "xiexie", text: &["谢谢"] },
];

/// The candidates for a reading in a table, or an empty list when the table has none.
pub fn candidates(table: &'static [Entry], reading: &str) -> &'static [&'static str] {
    table.iter().find(|e| e.reading == reading).map(|e| e.text).unwrap_or(&[])
}

/// The pinyin composer: letters build the reading, and candidates are picked by space or a digit.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PinyinComposer {
    reading: String,
}

impl Composer for PinyinComposer {
    fn feed(&mut self, ch: char) -> Option<String> {
        if ch.is_ascii_lowercase() {
            self.reading.push(ch);
            return Some(String::new());
        }
        if self.reading.is_empty() {
            return None;
        }
        let list = candidates(PINYIN, &self.reading);
        let pick = match ch {
            ' ' => list.first(),
            '1'..='9' => list.get((ch as u8 - b'1') as usize),
            _ => None,
        }?;
        self.reading.clear();
        Some(pick.to_string())
    }

    fn backspace(&mut self) -> bool {
        self.reading.pop().is_some()
    }

    fn composed(&self) -> String {
        self.reading.clone()
    }

    fn reset(&mut self) {
        self.reading.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(s: &str) -> PinyinComposer {
        let mut c = PinyinComposer::default();
        for ch in s.chars() {
            assert_eq!(c.feed(ch), Some(String::new()), "{ch} is a reading letter");
        }
        c
    }

    #[test]
    fn letters_build_the_reading_and_space_picks_the_first_candidate() {
        let mut c = typed("ni");
        assert_eq!(c.composed(), "ni");
        assert_eq!(c.feed(' '), Some("你".to_string()));
        assert_eq!(c.composed(), "");
    }

    #[test]
    fn a_digit_picks_that_candidate() {
        let mut c = typed("shi");
        assert_eq!(c.feed('2'), Some("时".to_string()));
        let mut d = typed("shi");
        assert_eq!(d.feed('9'), None, "no ninth candidate, so the digit is literal");
        assert_eq!(d.composed(), "shi", "the reading is kept for the controller to commit");
    }

    #[test]
    fn a_longer_reading_has_its_own_candidates() {
        let mut c = typed("xiexie");
        assert_eq!(c.feed(' '), Some("谢谢".to_string()));
    }

    #[test]
    fn a_reading_without_candidates_ends_on_space() {
        let mut c = typed("xyz");
        assert_eq!(c.feed(' '), None, "the controller commits the reading and inserts the space");
        assert_eq!(c.composed(), "xyz");
    }

    #[test]
    fn non_letters_outside_the_pick_keys_end_the_composition() {
        let mut c = typed("wo");
        assert_eq!(c.feed(','), None);
        let mut idle = PinyinComposer::default();
        assert_eq!(idle.feed(' '), None, "nothing typed, so space is literal");
    }

    #[test]
    fn backspace_removes_the_last_letter() {
        let mut c = typed("wo");
        assert!(c.backspace());
        assert_eq!(c.composed(), "w");
        c.reset();
        assert!(!c.backspace());
    }

    #[test]
    fn candidates_are_looked_up_by_reading() {
        assert_eq!(candidates(PINYIN, "hao")[0], "好");
        assert!(candidates(PINYIN, "qqq").is_empty());
    }
}
