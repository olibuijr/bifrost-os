//! Vietnamese composers: Telex (letter keys) and VNI (digit keys).
//!
//! The word is a list of letters with marks plus one tone; the display is rebuilt on every key, so the tone
//! moves to the right vowel as the word grows. Backspace removes the last typed key and replays the rest.
//! Tone placement follows the modern rule (second vowel of oa, oe, uy when the syllable is open).

use super::Composer;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scheme {
    Telex,
    Vni,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mark {
    None,
    Circ,
    Horn,
    Breve,
    Bar,
}

#[derive(Clone, Copy, Debug)]
struct Letter {
    base: char,
    mark: Mark,
    upper: bool,
}

/// Tones in table order: sac, huyen, hoi, nga, nang.
const TONES: [(char, &str); 12] = [
    ('a', "áàảãạ"),
    ('ă', "ắằẳẵặ"),
    ('â', "ấầẩẫậ"),
    ('e', "éèẻẽẹ"),
    ('ê', "ếềểễệ"),
    ('i', "íìỉĩị"),
    ('o', "óòỏõọ"),
    ('ô', "ốồổỗộ"),
    ('ơ', "ớờởỡợ"),
    ('u', "úùủũụ"),
    ('ư', "ứừửữự"),
    ('y', "ýỳỷỹỵ"),
];

fn is_vowel(c: char) -> bool {
    "aeiouy".contains(c)
}

fn glyph(l: Letter) -> char {
    match (l.base, l.mark) {
        ('a', Mark::Breve) => 'ă',
        ('a', Mark::Circ) => 'â',
        ('e', Mark::Circ) => 'ê',
        ('o', Mark::Circ) => 'ô',
        ('o', Mark::Horn) => 'ơ',
        ('u', Mark::Horn) => 'ư',
        ('d', Mark::Bar) => 'đ',
        (b, _) => b,
    }
}

fn telex_tone(k: char) -> Option<usize> {
    "sfrxj".find(k)
}

/// Which mark a mark key puts on a vowel, if any.
fn circ(b: char) -> Option<Mark> {
    "aeo".contains(b).then_some(Mark::Circ)
}
fn horn(b: char) -> Option<Mark> {
    "ou".contains(b).then_some(Mark::Horn)
}
fn breve(b: char) -> Option<Mark> {
    (b == 'a').then_some(Mark::Breve)
}
fn horn_or_breve(b: char) -> Option<Mark> {
    horn(b).or_else(|| breve(b))
}

enum MarkResult {
    Applied,
    Reverted,
    NoTarget,
}

pub struct VietnameseComposer {
    scheme: Scheme,
    letters: Vec<Letter>,
    tone: Option<usize>,
    raw: Vec<char>,
}

impl VietnameseComposer {
    pub fn new(scheme: Scheme) -> Self {
        Self { scheme, letters: Vec::new(), tone: None, raw: Vec::new() }
    }

    fn push(&mut self, ch: char) {
        self.letters.push(Letter { base: ch.to_ascii_lowercase(), mark: Mark::None, upper: ch.is_uppercase() });
    }

    /// Indices of the vowels that carry the tone (last vowel run, minus the u of qu and the i of gi).
    fn nucleus(&self) -> Vec<usize> {
        let Some(end) = self.letters.iter().rposition(|l| is_vowel(l.base)) else { return Vec::new() };
        let mut start = end;
        while start > 0 && is_vowel(self.letters[start - 1].base) {
            start -= 1;
        }
        let prev = if start > 0 { Some(self.letters[start - 1].base) } else { None };
        let first = self.letters[start];
        if (first.base == 'u' && prev == Some('q') && first.mark == Mark::None)
            || (first.base == 'i' && prev == Some('g') && end > start)
        {
            start += 1;
        }
        (start..=end).collect()
    }

    fn tone_pos(&self) -> Option<usize> {
        let nuc = self.nucleus();
        if let Some(&i) = nuc.iter().rev().find(|&&i| self.letters[i].mark != Mark::None) {
            return Some(i);
        }
        match nuc.len() {
            0 => None,
            1 => Some(nuc[0]),
            2 => {
                let closed = nuc[1] + 1 < self.letters.len();
                let pair = (self.letters[nuc[0]].base, self.letters[nuc[1]].base);
                if closed || matches!(pair, ('o', 'a') | ('o', 'e') | ('u', 'y')) {
                    Some(nuc[1])
                } else {
                    Some(nuc[0])
                }
            }
            _ => Some(nuc[1]),
        }
    }

    /// Put `f`'s mark on the last vowel (final consonants are skipped); the same mark again takes it off.
    fn apply_mark(&mut self, f: fn(char) -> Option<Mark>) -> MarkResult {
        let Some(i) = self.letters.iter().rposition(|l| is_vowel(l.base)) else { return MarkResult::NoTarget };
        let l = self.letters[i];
        let Some(m) = f(l.base) else { return MarkResult::NoTarget };
        let pair_u = i > 0 && l.base == 'o' && m == Mark::Horn && self.letters[i - 1].base == 'u';
        if l.mark == Mark::None {
            self.letters[i].mark = m;
            if pair_u && self.letters[i - 1].mark == Mark::None {
                self.letters[i - 1].mark = Mark::Horn;
            }
            MarkResult::Applied
        } else if l.mark == m {
            self.letters[i].mark = Mark::None;
            if pair_u && self.letters[i - 1].mark == Mark::Horn {
                self.letters[i - 1].mark = Mark::None;
            }
            MarkResult::Reverted
        } else {
            MarkResult::NoTarget
        }
    }

    /// Mark key outcome: applied is consumed, reverted leaves the key as a literal.
    fn mark_key(&mut self, f: fn(char) -> Option<Mark>, ch: char) -> bool {
        match self.apply_mark(f) {
            MarkResult::Applied => true,
            MarkResult::Reverted => {
                self.push(ch);
                true
            }
            MarkResult::NoTarget => false,
        }
    }

    /// `d` + `d` / `9`: bar through the d (the last letter for Telex, any for VNI); again takes it off.
    fn bar_key(&mut self, ch: char, adjacent: bool) -> bool {
        let found = if adjacent { self.letters.len().checked_sub(1) } else { self.letters.iter().rposition(|l| l.base == 'd') };
        let Some(i) = found.filter(|&i| self.letters[i].base == 'd') else { return false };
        if self.letters[i].mark == Mark::None {
            self.letters[i].mark = Mark::Bar;
        } else {
            self.letters[i].mark = Mark::None;
            self.push(ch);
        }
        true
    }

    fn tone_key(&mut self, t: usize, cancel_on_repeat: bool, ch: char) -> bool {
        if self.nucleus().is_empty() {
            return false;
        }
        if cancel_on_repeat && self.tone == Some(t) {
            self.tone = None;
            self.push(ch);
        } else {
            self.tone = Some(t);
        }
        true
    }

    fn feed_telex(&mut self, ch: char) -> bool {
        let k = ch.to_ascii_lowercase();
        if let Some(t) = telex_tone(k) {
            if self.tone_key(t, true, ch) {
                return true;
            }
        }
        let last = self.letters.last().map(|l| l.base);
        let handled = match k {
            'z' if self.tone.is_some() => {
                self.tone = None;
                true
            }
            'd' if last == Some('d') => self.bar_key(ch, true),
            'a' | 'e' | 'o' if last == Some(k) => self.mark_key(circ, ch),
            'w' => {
                if !self.mark_key(horn_or_breve, ch) {
                    self.letters.push(Letter { base: 'u', mark: Mark::Horn, upper: ch.is_uppercase() });
                }
                true
            }
            _ => false,
        };
        if handled {
            return true;
        }
        if ch.is_ascii_alphabetic() {
            self.push(ch);
            return true;
        }
        false
    }

    fn feed_vni(&mut self, ch: char) -> bool {
        match ch {
            '1'..='5' => self.tone_key(ch as usize - '1' as usize, false, ch),
            '0' if self.tone.is_some() => {
                self.tone = None;
                true
            }
            '6' => self.mark_key(circ, ch),
            '7' => self.mark_key(horn, ch),
            '8' => self.mark_key(breve, ch),
            '9' => self.bar_key(ch, false),
            c if c.is_ascii_alphabetic() => {
                self.push(c);
                true
            }
            _ => false,
        }
    }

    fn feed_inner(&mut self, ch: char) -> bool {
        match self.scheme {
            Scheme::Telex => self.feed_telex(ch),
            Scheme::Vni => self.feed_vni(ch),
        }
    }
}

impl Composer for VietnameseComposer {
    fn feed(&mut self, ch: char) -> Option<String> {
        if self.feed_inner(ch) {
            self.raw.push(ch);
            Some(String::new())
        } else {
            None
        }
    }

    fn backspace(&mut self) -> bool {
        if self.raw.pop().is_none() {
            return false;
        }
        let raw = std::mem::take(&mut self.raw);
        self.letters.clear();
        self.tone = None;
        for ch in raw {
            if self.feed_inner(ch) {
                self.raw.push(ch);
            }
        }
        true
    }

    fn composed(&self) -> String {
        let at = self.tone_pos();
        let mut out = String::new();
        for (i, l) in self.letters.iter().enumerate() {
            let mut c = glyph(*l);
            if Some(i) == at {
                if let (Some(t), Some((_, row))) = (self.tone, TONES.iter().find(|(p, _)| *p == c)) {
                    c = row.chars().nth(t).unwrap_or(c);
                }
            }
            if l.upper {
                out.extend(c.to_uppercase());
            } else {
                out.push(c);
            }
        }
        out
    }

    fn reset(&mut self) {
        self.letters.clear();
        self.tone = None;
        self.raw.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::super::testing::*;
    use super::super::*;

    fn telex(keys: &str) -> String {
        type_keys(VIETNAMESE_TELEX, keys).1.text()
    }
    fn vni(keys: &str) -> String {
        type_keys(VIETNAMESE_VNI, keys).1.text()
    }

    #[test]
    fn telex_viet() {
        assert_eq!(telex("vieetj"), "việt");
        assert_eq!(telex("Vieetj"), "Việt");
    }

    #[test]
    fn telex_marks() {
        assert_eq!(telex("dd"), "đ");
        assert_eq!(telex("aw"), "ă");
        assert_eq!(telex("ow"), "ơ");
        assert_eq!(telex("uow"), "ươ");
        assert_eq!(telex("w"), "ư");
        assert_eq!(telex("aa"), "â");
        assert_eq!(telex("oo"), "ô");
        assert_eq!(telex("DDi"), "Đi");
    }

    #[test]
    fn telex_tone_placement() {
        assert_eq!(telex("hoaf"), "hoà");
        assert_eq!(telex("toans"), "toán");
        assert_eq!(telex("muaf"), "mùa");
        assert_eq!(telex("miaf"), "mìa");
        assert_eq!(telex("thuyr"), "thuỷ");
        assert_eq!(telex("quas"), "quá");
        assert_eq!(telex("gias"), "giá");
        assert_eq!(telex("gif"), "gì");
        assert_eq!(telex("ngoaif"), "ngoài");
    }

    #[test]
    fn telex_tone_moves_as_the_word_grows() {
        let (ctl, _) = type_keys(VIETNAMESE_TELEX, "toasn");
        assert_eq!(ctl.composed_string(), "toán");
    }

    #[test]
    fn telex_z_removes_tone() {
        assert_eq!(telex("vieetjz"), "viêt");
        assert_eq!(telex("az"), "az");
    }

    #[test]
    fn telex_repeat_reverts() {
        assert_eq!(telex("ass"), "as");
        assert_eq!(telex("aaa"), "aa");
        assert_eq!(telex("ddd"), "dd");
        assert_eq!(telex("aww"), "aw");
    }

    #[test]
    fn telex_tone_key_without_vowel_is_a_letter() {
        assert_eq!(telex("sj"), "sj");
    }

    #[test]
    fn space_commits_word() {
        let (ctl, c) = type_keys(VIETNAMESE_TELEX, "vieetj ");
        assert_eq!(c.committed, "việt ");
        assert_eq!(ctl.composed_string(), "");
    }

    #[test]
    fn backspace_removes_the_last_key() {
        let (mut ctl, mut c) = type_keys(VIETNAMESE_TELEX, "vieetj");
        ctl.handle(Key::Backspace, &mut c);
        assert_eq!(c.marked, "viêt");
        ctl.handle(Key::Backspace, &mut c);
        assert_eq!(c.marked, "viê");
        ctl.handle(Key::Backspace, &mut c);
        assert_eq!(c.marked, "vie");
    }

    #[test]
    fn vni_digits() {
        assert_eq!(vni("viet65"), "việt");
        assert_eq!(vni("d9"), "đ");
        assert_eq!(vni("a8"), "ă");
        assert_eq!(vni("o7"), "ơ");
        assert_eq!(vni("u7o7"), "ươ");
        assert_eq!(vni("hoa2"), "hoà");
        assert_eq!(vni("a1"), "á");
        assert_eq!(vni("a10"), "a");
    }

    #[test]
    fn vni_digits_without_target_pass_through() {
        let mut ctl = new_controller(VIETNAMESE_VNI).unwrap();
        let mut c = RecordingClient::default();
        assert!(!ctl.input_text("1", &mut c));
        assert_eq!(vni("b1"), "b1");
    }
}
