//! Text editing shared by every text control: word boundaries, the
//! `EditTarget` view of a control's text and selection, and the editing-menu
//! actions (Cut, Copy, Paste, Select, Select All) applied through it with the
//! shell clipboard.

use super::clipboard;
use super::ui::Ui;

/// A control the editing menu can act on. Positions are char indices into
/// `text`; a selection is `start..end`, and `start == end` is the caret.
pub trait EditTarget {
    fn text(&self) -> &str;
    fn selection(&self) -> (usize, usize);
    /// Selects `start..end` (clamped, ordered); an empty range places the caret.
    fn select(&mut self, ui: &Ui, range: (usize, usize));
    /// Replaces the chars `start..end` with `with`, leaving the caret after it.
    fn replace(&mut self, ui: &Ui, range: (usize, usize), with: &str);
}

/// An entry of the editing menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Cut,
    Copy,
    Paste,
    Select,
    SelectAll,
}

impl Action {
    pub fn label(self) -> &'static str {
        match self {
            Action::Cut => "Cut",
            Action::Copy => "Copy",
            Action::Paste => "Paste",
            Action::Select => "Select",
            Action::SelectAll => "Select All",
        }
    }
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '\''
}

/// The word holding char `i` of `text`: the run of letters, digits, `_`
/// and apostrophes around it; on any other char just that char, and the
/// caret itself past the end.
pub fn word_at(text: &str, i: usize) -> (usize, usize) {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let i = i.min(n);
    let word = |k: usize| chars.get(k).is_some_and(|&c| is_word(c));
    if !word(i) {
        return (i, (i + 1).min(n));
    }
    let mut start = i;
    while start > 0 && word(start - 1) {
        start -= 1;
    }
    let mut end = i + 1;
    while word(end) {
        end += 1;
    }
    (start, end)
}

/// The word to select for a caret at `i`: the one under it, or the one it
/// sits right after (as Select does at the end of a word).
pub fn word_near_caret(text: &str, i: usize) -> (usize, usize) {
    let on_word = |k: usize| text.chars().nth(k).is_some_and(is_word);
    if !on_word(i) && i > 0 && on_word(i - 1) {
        word_at(text, i - 1)
    } else {
        word_at(text, i)
    }
}

const ZWJ: char = '\u{200D}';

fn is_regional(c: char) -> bool {
    ('\u{1F1E6}'..='\u{1F1FF}').contains(&c)
}

/// Chars that attach to the one before them: combining marks, variation
/// selectors, emoji skin tones, the keycap mark, tag chars and the ZWJ.
fn is_extend(c: char) -> bool {
    matches!(c as u32,
        0x0300..=0x036F | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF | 0x20D0..=0x20FF
        | 0xFE00..=0xFE0F | 0xFE20..=0xFE2F | 0x1F3FB..=0x1F3FF | 0xE0020..=0xE007F)
        || c == ZWJ
}

/// True when a grapheme cluster boundary lies between `chars[k - 1]` and `chars[k]`.
fn is_boundary(chars: &[char], k: usize) -> bool {
    let (a, b) = (chars[k - 1], chars[k]);
    if is_extend(b) || a == ZWJ && (b as u32) > 0x2000 {
        return false;
    }
    if is_regional(a) && is_regional(b) {
        // flags pair up from the start of a run of regional indicators
        let run = chars[..k].iter().rev().take_while(|&&c| is_regional(c)).count();
        return run % 2 == 0;
    }
    a != '\r' || b != '\n'
}

/// The char index of the grapheme boundary before `i`: where a caret
/// moving left or a backspace lands. Flags, ZWJ sequences, skin tones,
/// keycaps and combining marks count as one.
pub fn prev_boundary(text: &str, i: usize) -> usize {
    let chars: Vec<char> = text.chars().collect();
    let mut k = i.min(chars.len());
    if k > 0 {
        k -= 1;
        while k > 0 && !is_boundary(&chars, k) {
            k -= 1;
        }
    }
    k
}

/// The char index of the grapheme boundary after `i`.
pub fn next_boundary(text: &str, i: usize) -> usize {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let mut k = i.min(n);
    if k < n {
        k += 1;
        while k < n && !is_boundary(&chars, k) {
            k += 1;
        }
    }
    k
}

/// The menu for the target's state: Cut/Copy/Paste over a selection, Select,
/// Select All and Paste at a caret. Paste needs clipboard text; Select
/// needs text; `secure` fields (passwords) offer Paste only.
pub fn actions(target: &dyn EditTarget, secure: bool) -> Vec<Action> {
    let (start, end) = target.selection();
    let has_text = !target.text().is_empty();
    let paste = clipboard::global().as_ref().is_some_and(|c| c.has_text());
    let mut out = Vec::new();
    if secure {
        if paste {
            out.push(Action::Paste);
        }
        return out;
    }
    if start < end {
        out.extend([Action::Cut, Action::Copy]);
    } else if has_text {
        out.extend([Action::Select, Action::SelectAll]);
    }
    if paste {
        out.push(Action::Paste);
    }
    out
}

fn selected(target: &dyn EditTarget) -> String {
    let (start, end) = target.selection();
    target.text().chars().skip(start).take(end - start).collect()
}

/// Performs `action` on `target` with the shell clipboard.
pub fn apply(action: Action, target: &mut dyn EditTarget, ui: &Ui) {
    let (start, end) = target.selection();
    match action {
        Action::Cut => {
            clipboard::set_text(&selected(target));
            target.replace(ui, (start, end), "");
        }
        Action::Copy => clipboard::set_text(&selected(target)),
        Action::Paste => {
            if let Some(text) = clipboard::text() {
                target.replace(ui, (start, end), &text);
            }
        }
        Action::Select => target.select(ui, word_near_caret(target.text(), start)),
        Action::SelectAll => target.select(ui, (0, target.text().chars().count())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn word_at_expands_letters_digits_and_icelandic() {
        let s = "Þórður á 3 ísbíla, ok?";
        assert_eq!(word_at(s, 0), (0, 6));
        assert_eq!(word_at(s, 6), (6, 7), "the space after the word");
        assert_eq!(word_near_caret(s, 6), (0, 6), "a caret after the word takes it");
        assert_eq!(word_near_caret(s, 2), (0, 6));
        assert_eq!(word_near_caret(s, 7), (7, 8));
        assert_eq!(word_near_caret(s, 21), (19, 21), "before the question mark: the word it ends");
        assert_eq!(word_near_caret(s, 22), (22, 22), "after punctuation at the end: nothing");
        assert_eq!(word_at(s, 7), (7, 8));
        assert_eq!(word_at(s, 9), (9, 10));
        assert_eq!(word_at(s, 12), (11, 17));
        assert_eq!(word_at(s, 17), (17, 18), "the comma");
        assert_eq!(word_at(s, 18), (18, 19), "the space after the comma");
        assert_eq!(word_at(s, 100), (22, 22), "past the end is the caret");
        assert_eq!(word_at("", 0), (0, 0));
        assert_eq!(word_at("it's_ok", 2), (0, 7));
    }

    #[test]
    fn boundaries_keep_emoji_sequences_whole() {
        let flag = "\u{1F1EE}\u{1F1F8}"; // IS
        let s = format!("a{flag}{flag}b");
        assert_eq!(next_boundary(&s, 1), 3, "a flag is one step");
        assert_eq!(next_boundary(&s, 3), 5, "and so is the next");
        assert_eq!(prev_boundary(&s, 5), 3);
        assert_eq!(prev_boundary(&s, 3), 1);
        let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
        assert_eq!(next_boundary(family, 0), 5);
        assert_eq!(prev_boundary(family, 5), 0);
        let thumb = "\u{1F44D}\u{1F3FD}";
        assert_eq!(next_boundary(thumb, 0), 2, "skin tone");
        assert_eq!(prev_boundary(thumb, 2), 0);
        assert_eq!(next_boundary("\u{2764}\u{FE0F}x", 0), 2, "variation selector");
        assert_eq!(next_boundary("1\u{FE0F}\u{20E3}x", 0), 3, "keycap");
        assert_eq!(prev_boundary("e\u{301}", 2), 0, "combining acute");
        assert_eq!(prev_boundary("ab", 2), 1);
        assert_eq!(prev_boundary("ab", 0), 0);
        assert_eq!(next_boundary("ab", 2), 2);
        assert_eq!(next_boundary("\r\n", 0), 2);
    }

    struct Buf {
        text: String,
        sel: (usize, usize),
    }

    impl EditTarget for Buf {
        fn text(&self) -> &str {
            &self.text
        }
        fn selection(&self) -> (usize, usize) {
            self.sel
        }
        fn select(&mut self, _: &Ui, (a, b): (usize, usize)) {
            let n = self.text.chars().count();
            self.sel = (a.min(b).min(n), a.max(b).min(n));
        }
        fn replace(&mut self, _: &Ui, (a, b): (usize, usize), with: &str) {
            let byte = |i: usize| self.text.char_indices().nth(i).map_or(self.text.len(), |(k, _)| k);
            let (ba, bb) = (byte(a), byte(b));
            self.text.replace_range(ba..bb, with);
            let at = a + with.chars().count();
            self.sel = (at, at);
        }
    }

    fn ui() -> Ui {
        crate::shell::tests::ui()
    }

    #[test]
    fn actions_depend_on_selection_text_and_clipboard() {
        let _g = clipboard::tests::exclusive();
        let mut b = Buf { text: String::new(), sel: (0, 0) };
        assert!(actions(&b, false).is_empty(), "nothing to do in an empty field with an empty clipboard");
        b.text = "hæ".into();
        assert_eq!(actions(&b, false), [Action::Select, Action::SelectAll]);
        clipboard::set_text("x");
        assert_eq!(actions(&b, false), [Action::Select, Action::SelectAll, Action::Paste]);
        b.sel = (0, 2);
        assert_eq!(actions(&b, false), [Action::Cut, Action::Copy, Action::Paste]);
        assert_eq!(actions(&b, true), [Action::Paste], "a password field never reveals its text");
        clipboard::set_text("");
        assert!(actions(&b, true).is_empty());
    }

    #[test]
    fn cut_copy_paste_and_select_go_through_the_clipboard() {
        let _g = clipboard::tests::exclusive();
        let ui = ui();
        let mut b = Buf { text: "hello wörld".into(), sel: (3, 3) };
        apply(Action::Select, &mut b, &ui);
        assert_eq!(b.sel, (0, 5));
        apply(Action::Copy, &mut b, &ui);
        assert_eq!(clipboard::text().as_deref(), Some("hello"));
        b.sel = (6, 11);
        apply(Action::Cut, &mut b, &ui);
        assert_eq!((b.text.as_str(), b.sel), ("hello ", (6, 6)));
        assert_eq!(clipboard::text().as_deref(), Some("wörld"));
        apply(Action::Paste, &mut b, &ui);
        assert_eq!((b.text.as_str(), b.sel), ("hello wörld", (11, 11)));
        b.sel = (0, 5);
        apply(Action::Paste, &mut b, &ui);
        assert_eq!(b.text, "wörld wörld", "pasting replaces the selection");
        apply(Action::SelectAll, &mut b, &ui);
        assert_eq!(b.sel, (0, 11));
        clipboard::set_text("");
        apply(Action::Paste, &mut b, &ui);
        assert_eq!(b.text, "wörld wörld", "an empty clipboard pastes nothing");
    }
}
