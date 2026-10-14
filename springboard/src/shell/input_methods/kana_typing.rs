//! Japanese kana input on the kana board (JIS kana layout): each key types one kana, as the kana keys of a Japanese
//! keyboard do. The kana just typed stays marked until the next kana arrives, so a voicing key can still change it:
//! `@` adds the voiced mark and `[` the semi-voiced mark, which are the keys that carry those marks on the kana board.
//! Shifted keys (the small kana and the shifted punctuation) are not modelled.

use super::Composer;

/// Unshifted kana board, keyed by the US key that carries the kana.
const KEYS: &[(char, char)] = &[
    ('1', 'ぬ'), ('2', 'ふ'), ('3', 'あ'), ('4', 'う'), ('5', 'え'), ('6', 'お'), ('7', 'や'), ('8', 'ゆ'), ('9', 'よ'),
    ('0', 'わ'), ('-', 'ほ'), ('^', 'へ'), ('\\', 'ー'),
    ('q', 'た'), ('w', 'て'), ('e', 'い'), ('r', 'す'), ('t', 'か'), ('y', 'ん'), ('u', 'な'), ('i', 'に'), ('o', 'ら'),
    ('p', 'せ'),
    ('a', 'ち'), ('s', 'と'), ('d', 'し'), ('f', 'は'), ('g', 'き'), ('h', 'く'), ('j', 'ま'), ('k', 'の'), ('l', 'り'),
    (';', 'れ'), (':', 'け'), (']', 'む'),
    ('z', 'つ'), ('x', 'さ'), ('c', 'そ'), ('v', 'ひ'), ('b', 'こ'), ('n', 'み'), ('m', 'も'), (',', 'ね'), ('.', 'る'),
    ('/', 'め'), ('_', 'ろ'),
];

/// Voiced forms (゛, the `@` key).
const VOICED: &[(char, char)] = &[
    ('か', 'が'), ('き', 'ぎ'), ('く', 'ぐ'), ('け', 'げ'), ('こ', 'ご'), ('さ', 'ざ'), ('し', 'じ'), ('す', 'ず'),
    ('せ', 'ぜ'), ('そ', 'ぞ'), ('た', 'だ'), ('ち', 'ぢ'), ('つ', 'づ'), ('て', 'で'), ('と', 'ど'), ('は', 'ば'),
    ('ひ', 'び'), ('ふ', 'ぶ'), ('へ', 'べ'), ('ほ', 'ぼ'), ('う', 'ゔ'),
];

/// Semi-voiced forms (゜, the `[` key).
const SEMI_VOICED: &[(char, char)] = &[('は', 'ぱ'), ('ひ', 'ぴ'), ('ふ', 'ぷ'), ('へ', 'ぺ'), ('ほ', 'ぽ')];

/// The kana a key types on the board.
pub fn kana_of(key: char) -> Option<char> {
    KEYS.iter().find(|(k, _)| *k == key).map(|(_, kana)| *kana)
}

/// The kana that a voicing key turns `kana` into, or None when that kana takes no such mark.
pub fn mark_of(kana: char, key: char) -> Option<char> {
    let table = match key {
        '@' => VOICED,
        '[' => SEMI_VOICED,
        _ => return None,
    };
    table.iter().find(|(base, _)| *base == kana).map(|(_, marked)| *marked)
}

/// The composer: one kana held back so that a voicing key can still change it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KanaTypingComposer {
    held: Option<char>,
}

impl Composer for KanaTypingComposer {
    fn feed(&mut self, ch: char) -> Option<String> {
        if ch == '@' || ch == '[' {
            let held = self.held?;
            self.held = Some(mark_of(held, ch)?);
            return Some(String::new());
        }
        let kana = kana_of(ch)?;
        let done = self.held.replace(kana);
        Some(done.map(|c| c.to_string()).unwrap_or_default())
    }

    fn backspace(&mut self) -> bool {
        self.held.take().is_some()
    }

    fn composed(&self) -> String {
        self.held.map(|c| c.to_string()).unwrap_or_default()
    }

    fn reset(&mut self) {
        self.held = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_key_types_its_kana_and_the_previous_one_is_released() {
        let mut c = KanaTypingComposer::default();
        assert_eq!(c.feed('t'), Some(String::new()));
        assert_eq!(c.composed(), "か");
        assert_eq!(c.feed('e'), Some("か".to_string()));
        assert_eq!(c.composed(), "い");
    }

    #[test]
    fn the_voicing_key_changes_the_held_kana() {
        let mut c = KanaTypingComposer::default();
        c.feed('t');
        assert_eq!(c.feed('@'), Some(String::new()));
        assert_eq!(c.composed(), "が");
        c.feed('f');
        assert_eq!(c.feed('['), Some(String::new()));
        assert_eq!(c.composed(), "ぱ", "は takes the semi-voiced mark");
    }

    #[test]
    fn a_voicing_key_on_a_kana_without_that_mark_ends_the_composition() {
        let mut c = KanaTypingComposer::default();
        c.feed('e'); // い takes no voiced mark
        assert_eq!(c.feed('@'), None);
        assert_eq!(c.composed(), "い", "the held kana is not changed");
        let mut idle = KanaTypingComposer::default();
        assert_eq!(idle.feed('@'), None, "nothing held, so the key is literal");
    }

    #[test]
    fn non_kana_keys_end_the_composition() {
        let mut c = KanaTypingComposer::default();
        c.feed('t');
        assert_eq!(c.feed(' '), None);
        assert_eq!(c.composed(), "か", "the controller commits the held kana");
    }

    #[test]
    fn backspace_drops_the_held_kana_and_reports_when_empty() {
        let mut c = KanaTypingComposer::default();
        c.feed('t');
        assert!(c.backspace());
        assert_eq!(c.composed(), "");
        assert!(!c.backspace());
    }

    #[test]
    fn every_key_maps_to_one_kana_and_no_key_repeats() {
        let mut keys: Vec<char> = KEYS.iter().map(|(k, _)| *k).collect();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), KEYS.len());
        assert_eq!(kana_of('a'), Some('ち'));
        assert_eq!(kana_of('@'), None, "the voicing keys are not kana keys");
    }

    #[test]
    fn voiced_forms_are_the_dakuten_of_their_base_kana() {
        for (base, marked) in VOICED.iter().chain(SEMI_VOICED.iter()) {
            assert!(KEYS.iter().any(|(_, k)| k == base), "{base} is a kana on the board");
            assert_ne!(base, marked);
        }
        assert_eq!(mark_of('ち', '@'), Some('ぢ'));
        assert_eq!(mark_of('ほ', '['), Some('ぽ'));
        assert_eq!(mark_of('ほ', 'x'), None);
    }
}
