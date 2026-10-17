//! Ainu katakana composer: romaji keys typed as Ainu syllables in katakana.
//!
//! The key letters are the Ainu inventory `a i u e o k s t c p h m n r w y` (case does not matter). Any other letter,
//! digit, space or mark is not a composing key: it ends the composition and passes through to the client.
//! A consonant before a vowel makes an open syllable (`ka` gives カ). A consonant before another consonant, or at the
//! end of the word, is a closed syllable and takes the small final kana (`kam` gives カㇺ). `sh` and `ch` are digraphs,
//! and `tu` and `pu` use the Ainu forms ト゚ and ㇷ゚. A `y` or `w` directly after a vowel and before a consonant or the
//! end is the vowel i or u, so `kamuy` gives カムイ and `aynu` gives アイヌ.
//!
//! The marked text is rebuilt from the typed keys on every change, so backspace removes the last key. A consonant with
//! no small kana (c, w and y as finals) stays as Latin letters. The small kana for h and the use of ㇵ, and the
//! handling of the other small kana (ㇶ ㇸ ㇹ ㇼ ㇽ ㇾ ㇿ), need a check by a native Ainu speaker.

use super::Composer;

const KEYS: &str = "aiueokstcphmnrwy";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tok {
    V(char),
    C(char),
}

/// The Ainu forms that differ from the built-in kana method's romaji table (`bifrost_ui::kana_method`).
const AINU_FORMS: &[(&str, &str)] = &[
    ("ti", "ティ"),
    ("tu", "\u{30C8}\u{309A}"),
    ("pu", "\u{31F7}\u{309A}"),
    ("wu", "ウ"),
    ("wi", "ウィ"),
    ("we", "ウェ"),
    ("wo", "ウォ"),
    ("yi", "イ"),
    ("ye", "イェ"),
    ("che", "チェ"),
    ("she", "シェ"),
];

/// One romaji syllable as katakana: an Ainu form when one is listed, else the kana method's syllable read as katakana.
fn syllable(romaji: &str) -> String {
    if let Some((_, form)) = AINU_FORMS.iter().find(|(k, _)| *k == romaji) {
        return form.to_string();
    }
    match bifrost_ui::kana_method::syllable_hiragana(romaji) {
        Some(hiragana) => bifrost_ui::kana_method::katakana_of(hiragana),
        None => romaji.to_string(),
    }
}

/// A consonant key with a vowel. `c` is written `ch` in the romaji table (`ca` is `cha`).
fn syllable_of(key: &str, v: char) -> String {
    let base = if key == "c" { "ch" } else { key };
    syllable(&format!("{base}{v}"))
}

/// The small kana a closed syllable takes when the key ends the word or comes before a consonant.
fn final_kana(key: &str) -> Option<&'static str> {
    Some(match key {
        "k" => "\u{31F0}",
        "sh" => "\u{31F1}",
        "s" => "\u{31F2}",
        "t" => "\u{31F3}",
        "n" => "\u{31F4}",
        "h" => "\u{31F5}",
        "p" => "\u{31F7}",
        "m" => "\u{31FA}",
        "r" => "\u{31FB}",
        _ => return None,
    })
}

/// The katakana for a typed key sequence (lower-case keys from the inventory).
pub fn render(keys: &[char]) -> String {
    let mut toks: Vec<Tok> = keys.iter().map(|&c| if "aiueo".contains(c) { Tok::V(c) } else { Tok::C(c) }).collect();
    for j in 1..toks.len() {
        if let Tok::C(c) = toks[j] {
            let vowel_like = c == 'y' || c == 'w';
            let after_vowel = matches!(toks[j - 1], Tok::V(_));
            let closes = j + 1 == toks.len() || matches!(toks[j + 1], Tok::C(_));
            if vowel_like && after_vowel && closes {
                toks[j] = Tok::V(if c == 'y' { 'i' } else { 'u' });
            }
        }
    }
    let mut out = String::new();
    let mut i = 0;
    while i < toks.len() {
        match toks[i] {
            Tok::V(v) => {
                out.push_str(&syllable(&v.to_string()));
                i += 1;
            }
            Tok::C(c) => {
                let digraph = matches!(c, 'c' | 's') && toks.get(i + 1) == Some(&Tok::C('h'));
                let (key, next) = if digraph { (format!("{c}h"), i + 2) } else { (c.to_string(), i + 1) };
                match toks.get(next) {
                    Some(&Tok::V(v)) => {
                        out.push_str(&syllable_of(&key, v));
                        i = next + 1;
                    }
                    _ => {
                        match final_kana(&key) {
                            Some(f) => out.push_str(f),
                            None => out.push_str(&key),
                        }
                        i = next;
                    }
                }
            }
        }
    }
    out
}

#[derive(Default)]
pub struct AinuComposer {
    keys: Vec<char>,
}

impl Composer for AinuComposer {
    fn feed(&mut self, ch: char) -> Option<String> {
        let c = ch.to_ascii_lowercase();
        if ch.is_ascii_alphabetic() && KEYS.contains(c) {
            self.keys.push(c);
            Some(String::new())
        } else {
            None
        }
    }

    fn backspace(&mut self) -> bool {
        self.keys.pop().is_some()
    }

    fn composed(&self) -> String {
        render(&self.keys)
    }

    fn reset(&mut self) {
        self.keys.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::super::testing::*;
    use super::super::*;

    fn typed(keys: &str) -> String {
        type_keys(AINU_KATAKANA, keys).1.text()
    }

    #[test]
    fn open_syllables() {
        assert_eq!(typed("aiueo"), "アイウエオ");
        assert_eq!(typed("kakisukeko"), "カキスケコ");
        assert_eq!(typed("pe"), "ペ");
        assert_eq!(typed("ti"), "ティ");
        assert_eq!(typed("wi"), "ウィ");
        assert_eq!(typed("yu"), "ユ");
    }

    #[test]
    fn digraphs_and_ainu_forms() {
        assert_eq!(typed("shi"), "シ");
        assert_eq!(typed("sha"), "シャ");
        assert_eq!(typed("chi"), "チ");
        assert_eq!(typed("ca"), "チャ");
        assert_eq!(typed("tu"), "\u{30C8}\u{309A}");
        assert_eq!(typed("pu"), "\u{31F7}\u{309A}");
    }

    #[test]
    fn closed_syllables_take_small_kana() {
        assert_eq!(typed("kam"), "カ\u{31FA}");
        assert_eq!(typed("ask"), "ア\u{31F2}\u{31F0}");
        assert_eq!(typed("ash"), "ア\u{31F1}");
        assert_eq!(typed("kamuy"), "カムイ");
        assert_eq!(typed("aynu"), "アイヌ");
        assert_eq!(typed("ainu"), "アイヌ");
    }

    #[test]
    fn upper_case_keys_compose_the_same() {
        assert_eq!(typed("KAMUY"), "カムイ");
    }

    #[test]
    fn latin_and_space_pass_through_and_commit() {
        let (ctl, c) = type_keys(AINU_KATAKANA, "kamuy ");
        assert_eq!(c.committed, "カムイ ");
        assert_eq!(c.marked, "");
        assert_eq!(ctl.composed_string(), "");
        assert_eq!(typed("ka1"), "カ1");
        assert_eq!(typed("kaq"), "カq");
    }

    #[test]
    fn idle_non_key_is_not_handled() {
        let mut ctl = new_controller(AINU_KATAKANA).unwrap();
        let mut c = RecordingClient::default();
        assert!(!ctl.input_text("1", &mut c));
        assert!(!ctl.handle(Key::Backspace, &mut c));
        assert_eq!(c.text(), "");
    }

    #[test]
    fn backspace_removes_one_key_at_a_time() {
        let (mut ctl, mut c) = type_keys(AINU_KATAKANA, "kamuy");
        let mut seen = vec![ctl.composed_string()];
        for _ in 0..5 {
            assert!(ctl.handle(Key::Backspace, &mut c));
            seen.push(ctl.composed_string());
        }
        assert_eq!(seen, ["カムイ", "カム", "カ\u{31FA}", "カ", "\u{31F0}", ""]);
        assert!(!ctl.handle(Key::Backspace, &mut c));
        assert_eq!(c.text(), "");
    }

    #[test]
    fn commit_inserts_marked_text_and_clears_it() {
        let (mut ctl, mut c) = type_keys(AINU_KATAKANA, "kamuy");
        ctl.commit_composition(&mut c);
        assert_eq!((c.committed.as_str(), c.marked.as_str()), ("カムイ", ""));
        assert_eq!(ctl.composed_string(), "");
    }

    #[test]
    fn return_commits_and_escape_cancels() {
        let (mut ctl, mut c) = type_keys(AINU_KATAKANA, "ka");
        assert!(ctl.handle(Key::Return, &mut c));
        assert_eq!(c.committed, "カ");
        let (mut ctl, mut c) = type_keys(AINU_KATAKANA, "kamuy");
        assert!(ctl.handle(Key::Escape, &mut c));
        assert_eq!(c.text(), "");
        assert_eq!(ctl.composed_string(), "");
    }
}
