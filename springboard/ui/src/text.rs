//! A text editing model (`NativeTextInputClient`, `NativeTextInput`, `NativeTextView` key bindings): grapheme-aware caret movement by
//! character, word and line, selection, a marked (composing) range for IME, delete and kill commands, and undo that
//! plugs into `undo.rs`.
//!
//! Offsets are UTF-8 byte offsets, the unit of Wayland `text-input-v3`. `NativeRange` counts UTF-16 units;
//! `byte_to_utf16` and `utf16_to_byte` convert. No dependencies: grapheme clusters follow UAX #29 closely enough for
//! combining marks, variation selectors, emoji modifiers, ZWJ sequences, regional-indicator flags, CRLF and Hangul jamo
//! (see the gaps in `docs/bifrost-ui.md`).
//!
//! * `TextBuffer`: text, selection, marked range. Every edit returns an `Edit` that can be reverted and reapplied.
//! * `TextEditor`: a buffer plus an `UndoManager`, typing coalesced into one undo step, a kill buffer, and `perform`
//!   for `EditCommand`s. `binding` maps a key combo to a command (Golden Gate defaults plus the emacs bindings ctrl-A/E/K).
use crate::compose_method::{self, Composer};
use crate::keys::{Key, KeyCombo};
use crate::undo::UndoManager;
use std::ops::Range;

const ZWJ: char = '\u{200d}';

/// How many lines Page Up and Page Down move in a buffer that does not know how tall its view is.
pub const PAGE_LINES: usize = 10;

fn is_regional_indicator(c: char) -> bool {
    ('\u{1f1e6}'..='\u{1f1ff}').contains(&c)
}

/// Grapheme extenders: combining marks, variation selectors, emoji modifiers, tag characters, ZWNJ, common Indic signs.
fn is_extend(c: char) -> bool {
    matches!(c,
        '\u{0300}'..='\u{036f}' | '\u{0483}'..='\u{0489}' | '\u{0591}'..='\u{05bd}' | '\u{064b}'..='\u{065f}'
        | '\u{0900}'..='\u{0903}' | '\u{093a}'..='\u{094f}' | '\u{1ab0}'..='\u{1aff}' | '\u{1dc0}'..='\u{1dff}'
        | '\u{200c}' | '\u{20d0}'..='\u{20ff}' | '\u{3099}'..='\u{309a}' | '\u{fe00}'..='\u{fe0f}'
        | '\u{fe20}'..='\u{fe2f}' | '\u{1f3fb}'..='\u{1f3ff}' | '\u{e0020}'..='\u{e007f}' | '\u{e0100}'..='\u{e01ef}')
}

fn is_pictographic(c: char) -> bool {
    matches!(c, '\u{2190}'..='\u{2bff}' | '\u{1f000}'..='\u{1faff}' | '\u{00a9}' | '\u{00ae}')
}

fn is_jamo(c: char) -> bool {
    ('\u{1100}'..='\u{11ff}').contains(&c)
}

/// Letters, digits, `_` and marks are word characters.
fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || is_extend(c)
}

fn char_at(s: &str, i: usize) -> Option<char> {
    s.get(i..)?.chars().next()
}

fn prev_char_start(s: &str, i: usize) -> usize {
    let mut j = i.saturating_sub(1);
    while j > 0 && !s.is_char_boundary(j) {
        j -= 1;
    }
    j
}

/// The largest char boundary at or below `i` (and at most `s.len()`).
pub fn floor_boundary(s: &str, i: usize) -> usize {
    let mut i = i.min(s.len());
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// The smallest char boundary at or above `i` (and at most `s.len()`).
pub fn ceil_boundary(s: &str, i: usize) -> usize {
    let mut i = i.min(s.len());
    while !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

/// The end of the grapheme cluster that starts at `i`; `s.len()` at the end.
pub fn next_boundary(s: &str, i: usize) -> usize {
    let Some(c) = char_at(s, i) else { return s.len() };
    let mut end = i + c.len_utf8();
    if c == '\r' {
        return if s[end..].starts_with('\n') { end + 1 } else { end };
    }
    if c.is_control() {
        return end;
    }
    if is_regional_indicator(c) {
        if let Some(n) = char_at(s, end).filter(|n| is_regional_indicator(*n)) {
            end += n.len_utf8();
        }
    } else if is_jamo(c) {
        while let Some(n) = char_at(s, end).filter(|n| is_jamo(*n)) {
            end += n.len_utf8();
        }
    }
    while let Some(n) = char_at(s, end) {
        if is_extend(n) {
            end += n.len_utf8();
        } else if n == ZWJ {
            end += n.len_utf8();
            if let Some(m) = char_at(s, end).filter(|m| is_pictographic(*m)) {
                end += m.len_utf8();
            }
        } else {
            break;
        }
    }
    end
}

/// The start of the grapheme cluster that ends at `i`; 0 at the start. `i` between two clusters gives the earlier one's
/// start, `i` inside a cluster gives that cluster's start.
pub fn prev_boundary(s: &str, i: usize) -> usize {
    let i = floor_boundary(s, i);
    if i == 0 {
        return 0;
    }
    // Start a little way back, on a cluster start, and walk forward.
    let mut st = i;
    for _ in 0..16 {
        if st == 0 {
            break;
        }
        st = prev_char_start(s, st);
    }
    while st > 0 {
        let c = char_at(s, st).unwrap_or(' ');
        let p = prev_char_start(s, st);
        let prev = char_at(s, p);
        if is_extend(c) || c == ZWJ || prev.is_some_and(|q| is_regional_indicator(q) || q == ZWJ) {
            st = p;
        } else {
            break;
        }
    }
    let mut b = st;
    loop {
        let n = next_boundary(s, b);
        if n >= i || n == b {
            return b;
        }
        b = n;
    }
}

/// The end of the word at or after `i`: skips non-word characters, then the word.
pub fn next_word_end(s: &str, i: usize) -> usize {
    let mut j = floor_boundary(s, i);
    while let Some(c) = char_at(s, j) {
        if is_word(c) {
            break;
        }
        j += c.len_utf8();
    }
    while let Some(c) = char_at(s, j) {
        if !is_word(c) {
            break;
        }
        j += c.len_utf8();
    }
    j
}

/// The start of the word at or before `i`: skips non-word characters backwards, then the word.
pub fn prev_word_start(s: &str, i: usize) -> usize {
    let mut j = floor_boundary(s, i);
    while j > 0 {
        let p = prev_char_start(s, j);
        if char_at(s, p).is_some_and(is_word) {
            break;
        }
        j = p;
    }
    while j > 0 {
        let p = prev_char_start(s, j);
        if !char_at(s, p).is_some_and(is_word) {
            break;
        }
        j = p;
    }
    j
}

/// Start of the paragraph (line, split at `\n`) containing `i`.
pub fn line_start(s: &str, i: usize) -> usize {
    let i = floor_boundary(s, i);
    s[..i].rfind('\n').map_or(0, |p| p + 1)
}

/// End of the paragraph containing `i`, before its break: a `\n`, or the `\r\n` pair, which is one break.
pub fn line_end(s: &str, i: usize) -> usize {
    let i = floor_boundary(s, i);
    s[i..].find('\n').map_or(s.len(), |p| {
        let at = i + p;
        if at > 0 && s.as_bytes()[at - 1] == b'\r' { at - 1 } else { at }
    })
}

/// How many grapheme clusters `s` has.
pub fn grapheme_count(s: &str) -> usize {
    let (mut n, mut i) = (0, 0);
    while i < s.len() {
        i = next_boundary(s, i);
        n += 1;
    }
    n
}

/// UTF-16 offset (an `NativeRange` location) of byte offset `b`.
pub fn byte_to_utf16(s: &str, b: usize) -> usize {
    s[..floor_boundary(s, b)].chars().map(char::len_utf16).sum()
}

/// Byte offset of UTF-16 offset `u`; an offset inside a surrogate pair gives the start of that character.
pub fn utf16_to_byte(s: &str, u: usize) -> usize {
    let mut acc = 0;
    for (i, c) in s.char_indices() {
        if acc + c.len_utf16() > u {
            return i;
        }
        acc += c.len_utf16();
    }
    s.len()
}

/// One change to the text: `removed` at `at` was replaced by `inserted`, with the selection `(anchor, head)` before and
/// after. Undo reverts it, redo reapplies it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    pub at: usize,
    pub removed: String,
    pub inserted: String,
    pub before: (usize, usize),
    pub after: (usize, usize),
}

/// What the marked range replaced, kept until the composition ends so it can be undone as one edit.
#[derive(Clone, Debug)]
struct Composition {
    at: usize,
    removed: String,
    before: (usize, usize),
}

/// Text around the caret for predictive text and IME (`NativeTextInput` surrounding text). Offsets are relative to `text`.
/// Fits `bifrost_proto::TextState`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Surrounding<'a> {
    pub text: &'a str,
    /// where `text` starts in the buffer
    pub base: usize,
    pub cursor: usize,
    pub anchor: usize,
    pub marked: Option<(usize, usize)>,
}

#[derive(Clone, Debug, Default)]
pub struct TextBuffer {
    text: String,
    anchor: usize,
    head: usize,
    marked: Option<Range<usize>>,
    comp: Option<Composition>,
    /// the column (in graphemes) vertical movement tries to keep
    goal_col: Option<usize>,
    kill: String,
}

impl TextBuffer {
    pub fn new() -> Self {
        Self::default()
    }
    /// A buffer holding `text` with the caret at the end.
    pub fn from_text(text: &str) -> Self {
        TextBuffer { text: text.into(), anchor: text.len(), head: text.len(), ..Self::default() }
    }
    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn len(&self) -> usize {
        self.text.len()
    }
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }
    /// The moving end of the selection (`selectedRange.location` when nothing is selected).
    pub fn caret(&self) -> usize {
        self.head
    }
    pub fn anchor(&self) -> usize {
        self.anchor
    }
    /// `selectedRange`.
    pub fn selection(&self) -> Range<usize> {
        self.anchor.min(self.head)..self.anchor.max(self.head)
    }
    pub fn has_selection(&self) -> bool {
        self.anchor != self.head
    }
    pub fn selected_text(&self) -> &str {
        &self.text[self.selection()]
    }
    /// `markedRange`; `None` is `NativeNotFound`.
    pub fn marked_range(&self) -> Option<Range<usize>> {
        self.marked.clone()
    }
    /// `hasMarkedText`.
    pub fn has_marked_text(&self) -> bool {
        self.marked.is_some()
    }
    /// `substringWithRange:`; the range is clamped to the text and to character boundaries.
    pub fn substring(&self, range: Range<usize>) -> &str {
        let (a, b) = (floor_boundary(&self.text, range.start), floor_boundary(&self.text, range.end));
        &self.text[a..a.max(b)]
    }
    /// The last text removed by a kill command (`ctrl-K`, `ctrl-U`), for `yank`.
    pub fn kill_buffer(&self) -> &str {
        &self.kill
    }
    pub fn set_text(&mut self, text: &str) {
        self.text = text.into();
        self.anchor = self.text.len();
        self.head = self.text.len();
        self.marked = None;
        self.comp = None;
        self.goal_col = None;
    }

    /// Sets the selection; both ends are clamped to character boundaries. Does not end a composition.
    pub fn set_selection(&mut self, anchor: usize, head: usize) {
        self.anchor = floor_boundary(&self.text, anchor);
        self.head = floor_boundary(&self.text, head);
        self.goal_col = None;
    }
    pub fn select_all(&mut self) {
        self.set_selection(0, self.text.len());
    }
    /// Selects the word under `pos` (a double click). On whitespace or punctuation selects that run's neighbour word.
    pub fn select_word_at(&mut self, pos: usize) {
        let pos = floor_boundary(&self.text, pos);
        let on_word = char_at(&self.text, pos).is_some_and(is_word);
        let end = if on_word {
            let mut j = pos;
            while let Some(c) = char_at(&self.text, j).filter(|c| is_word(*c)) {
                j += c.len_utf8();
            }
            j
        } else {
            next_word_end(&self.text, pos)
        };
        let start = prev_word_start(&self.text, end);
        self.set_selection(start, end);
    }
    /// Selects the paragraph under `pos` (a triple click), without its newline.
    pub fn select_line_at(&mut self, pos: usize) {
        self.set_selection(line_start(&self.text, pos), line_end(&self.text, pos));
    }

    fn move_to(&mut self, pos: usize, extend: bool) {
        self.head = pos;
        if !extend {
            self.anchor = pos;
        }
    }

    /// Left or right by one grapheme. Without `extend` a selection collapses to its near edge first (as in the native toolkit).
    pub fn move_char(&mut self, right: bool, extend: bool) {
        self.goal_col = None;
        if !extend && self.has_selection() {
            let r = self.selection();
            let pos = if right { r.end } else { r.start };
            self.move_to(pos, false);
            return;
        }
        let pos = if right { next_boundary(&self.text, self.head) } else { prev_boundary(&self.text, self.head) };
        self.move_to(pos, extend);
    }
    /// To the end of the next word (`right`) or the start of the previous one.
    pub fn move_word(&mut self, right: bool, extend: bool) {
        self.goal_col = None;
        let pos = if right { next_word_end(&self.text, self.head) } else { prev_word_start(&self.text, self.head) };
        self.move_to(pos, extend);
    }
    /// To the start or end of the paragraph (`ctrl-A`, `ctrl-E`, cmd-left, cmd-right).
    pub fn move_line(&mut self, end: bool, extend: bool) {
        self.goal_col = None;
        let pos = if end { line_end(&self.text, self.head) } else { line_start(&self.text, self.head) };
        self.move_to(pos, extend);
    }
    /// To the start or end of the text (cmd-up, cmd-down).
    pub fn move_document(&mut self, end: bool, extend: bool) {
        self.goal_col = None;
        let pos = if end { self.text.len() } else { 0 };
        self.move_to(pos, extend);
    }
    /// Up or down one paragraph line, keeping the column in graphemes. On the first line up goes to the start, on the
    /// last line down goes to the end (as in the native toolkit). Soft-wrapped lines are the view's business.
    pub fn move_vertical(&mut self, down: bool, extend: bool) {
        let ls = line_start(&self.text, self.head);
        let col = match self.goal_col {
            Some(c) => c,
            None => grapheme_count(&self.text[ls..self.head]),
        };
        let pos = if down {
            let le = line_end(&self.text, self.head);
            if le >= self.text.len() {
                self.goal_col = Some(col);
                self.move_to(self.text.len(), extend);
                return;
            }
            // the break may be a CRLF pair: the next line starts after all of it
            self.at_column(next_boundary(&self.text, le), col)
        } else {
            if ls == 0 {
                self.goal_col = Some(col);
                self.move_to(0, extend);
                return;
            }
            self.at_column(line_start(&self.text, ls - 1), col)
        };
        self.move_to(pos, extend);
        self.goal_col = Some(col);
    }
    /// A page up or down: `lines` paragraph lines, keeping the column, stopping at the first or last line (where it goes to
    /// the start or the end of the text, as the arrows do).
    pub fn move_page(&mut self, down: bool, extend: bool, lines: usize) {
        for _ in 0..lines.max(1) {
            let before = self.head;
            self.move_vertical(down, extend);
            if self.head == before {
                break;
            }
        }
    }
    /// The offset `col` graphemes into the line starting at `start`, or the line's end.
    fn at_column(&self, start: usize, col: usize) -> usize {
        let end = line_end(&self.text, start);
        let mut i = start;
        for _ in 0..col {
            if i >= end {
                break;
            }
            i = next_boundary(&self.text, i).min(end);
        }
        i
    }

    // --- editing -----------------------------------------------------------------------------------------------

    /// Replaces `range` with `new` and puts the caret after it. The primitive of every edit.
    pub fn replace_range(&mut self, range: Range<usize>, new: &str) -> Option<Edit> {
        let start = floor_boundary(&self.text, range.start);
        let end = floor_boundary(&self.text, range.end).max(start);
        if start == end && new.is_empty() {
            return None;
        }
        let before = (self.anchor, self.head);
        let removed = self.text[start..end].to_string();
        self.text.replace_range(start..end, new);
        let pos = start + new.len();
        self.anchor = pos;
        self.head = pos;
        self.marked = None;
        self.comp = None;
        self.goal_col = None;
        Some(Edit { at: start, removed, inserted: new.into(), before, after: (pos, pos) })
    }

    /// `insertText:`: replaces the marked text (ending the composition) or else the selection.
    pub fn insert_text(&mut self, text: &str) -> Option<Edit> {
        if let Some(m) = self.marked.take() {
            let comp = self.comp.take().unwrap_or(Composition { at: m.start, removed: String::new(), before: (self.anchor, self.head) });
            self.text.replace_range(m.clone(), text);
            let pos = m.start + text.len();
            self.anchor = pos;
            self.head = pos;
            self.goal_col = None;
            if comp.removed.is_empty() && text.is_empty() {
                return None;
            }
            return Some(Edit { at: comp.at, removed: comp.removed, inserted: text.into(), before: comp.before, after: (pos, pos) });
        }
        let r = self.selection();
        self.replace_range(r, text)
    }

    fn delete_range_or_selection(&mut self, range: Range<usize>) -> Option<Edit> {
        if self.has_selection() {
            let r = self.selection();
            return self.replace_range(r, "");
        }
        self.replace_range(range, "")
    }

    /// `deleteBackward:` (backspace): the selection, else the grapheme before the caret. During a composition it edits
    /// the marked text only.
    pub fn delete_backward(&mut self) -> Option<Edit> {
        if let Some(m) = self.marked.clone() {
            if self.head <= m.start {
                return None;
            }
            let from = prev_boundary(&self.text, self.head).max(m.start);
            self.text.replace_range(from..self.head, "");
            let len = self.head - from;
            self.marked = Some(m.start..m.end - len);
            self.anchor = from;
            self.head = from;
            if self.marked.as_ref().is_some_and(|m| m.is_empty()) {
                self.cancel_marked();
            }
            return None;
        }
        let from = prev_boundary(&self.text, self.head);
        self.delete_range_or_selection(from..self.head)
    }
    /// `deleteForward:` (ctrl-D, the forward delete key).
    pub fn delete_forward(&mut self) -> Option<Edit> {
        let to = next_boundary(&self.text, self.head);
        self.delete_range_or_selection(self.head..to)
    }
    /// `deleteWordBackward:` (option-backspace).
    pub fn delete_word_backward(&mut self) -> Option<Edit> {
        let from = prev_word_start(&self.text, self.head);
        self.delete_range_or_selection(from..self.head)
    }
    /// `deleteWordForward:`.
    pub fn delete_word_forward(&mut self) -> Option<Edit> {
        let to = next_word_end(&self.text, self.head);
        self.delete_range_or_selection(self.head..to)
    }
    /// `deleteToBeginningOfLine:` (cmd-backspace). At a line start deletes the newline before it.
    pub fn delete_to_line_start(&mut self) -> Option<Edit> {
        let ls = line_start(&self.text, self.head);
        let from = if ls == self.head { prev_boundary(&self.text, self.head) } else { ls };
        self.delete_range_or_selection(from..self.head)
    }
    /// `deleteToEndOfParagraph:` (ctrl-K): kills to the end of the line, or the newline at the end of it. The text goes
    /// to the kill buffer.
    pub fn kill_line(&mut self) -> Option<Edit> {
        let le = line_end(&self.text, self.head);
        let to = if le == self.head && le < self.text.len() { next_boundary(&self.text, le) } else { le };
        let range = if self.has_selection() { self.selection() } else { self.head..to };
        if range.is_empty() {
            return None;
        }
        self.kill = self.text[range.clone()].to_string();
        self.replace_range(range, "")
    }
    /// `yank:` (ctrl-Y): inserts the kill buffer.
    pub fn yank(&mut self) -> Option<Edit> {
        let k = self.kill.clone();
        if k.is_empty() {
            return None;
        }
        self.insert_text(&k)
    }
    /// `transpose:` (ctrl-T): swaps the graphemes around the caret; at the end of a line swaps the last two.
    pub fn transpose(&mut self) -> Option<Edit> {
        if self.marked.is_some() {
            return None;
        }
        let le = line_end(&self.text, self.head);
        let mid = if self.head == le { prev_boundary(&self.text, self.head) } else { self.head };
        let a = prev_boundary(&self.text, mid);
        let b = next_boundary(&self.text, mid);
        if a >= mid || b <= mid || self.text[a..b].contains('\n') {
            return None;
        }
        let swapped = format!("{}{}", &self.text[mid..b], &self.text[a..mid]);
        self.replace_range(a..b, &swapped)
    }
    /// `insertNewlineIgnoringFieldEditor:` variant ctrl-O: a newline after the caret, the caret stays.
    pub fn open_line(&mut self) -> Option<Edit> {
        let r = self.selection();
        let mut e = self.replace_range(r.clone(), "\n")?;
        self.anchor = r.start;
        self.head = r.start;
        e.after = (r.start, r.start);
        Some(e)
    }

    /// `text-input-v3` `delete_surrounding_text`: deletes `before` bytes before and `after` bytes after the selection
    /// (rounded out to character boundaries). The selection stays on the same text. One edit per side.
    pub fn delete_surrounding(&mut self, before: usize, after: usize) -> Vec<Edit> {
        let mut edits = Vec::new();
        let sel = self.selection();
        let (a, h) = (self.anchor, self.head);
        if after > 0 {
            let to = ceil_boundary(&self.text, sel.end.saturating_add(after));
            if let Some(mut e) = self.replace_range(sel.end..to, "") {
                self.anchor = a;
                self.head = h;
                e.before = (a, h);
                e.after = (a, h);
                edits.push(e);
            }
        }
        if before > 0 {
            let from = floor_boundary(&self.text, sel.start.saturating_sub(before));
            let cut = sel.start - from;
            if let Some(mut e) = self.replace_range(from..sel.start, "") {
                let (na, nh) = (a - cut, h - cut);
                self.anchor = na;
                self.head = nh;
                e.before = (a, h);
                e.after = (na, nh);
                edits.push(e);
            }
        }
        edits
    }

    // --- marked text (IME) -------------------------------------------------------------------------------------

    /// `setMarkedText:selectedRange:replacementRange:`. Replaces the marked range (if any), else `replacement`, else
    /// the selection, with `text`, marked. `sel_start`/`sel_len` select inside `text` (bytes); text-input-v3 preedit
    /// gives the caret as `cursor_begin..cursor_end`. Empty `text` ends the composition and restores what it replaced.
    /// The replacement range is only honoured when nothing is marked yet. Nothing is recorded for undo until
    /// `unmark_text` or `insert_text` commits.
    pub fn set_marked_text(&mut self, text: &str, sel_start: usize, sel_len: usize, replacement: Option<Range<usize>>) {
        if text.is_empty() {
            self.cancel_marked();
            return;
        }
        let range = match (&self.marked, replacement) {
            (Some(m), _) => m.clone(),
            (None, Some(r)) => {
                let s = floor_boundary(&self.text, r.start);
                s..floor_boundary(&self.text, r.end).max(s)
            }
            (None, None) => self.selection(),
        };
        if self.marked.is_none() {
            self.comp = Some(Composition { at: range.start, removed: self.text[range.clone()].to_string(), before: (self.anchor, self.head) });
        }
        self.text.replace_range(range.clone(), text);
        let start = range.start;
        self.marked = Some(start..start + text.len());
        let s = floor_boundary(text, sel_start);
        let e = floor_boundary(text, s.saturating_add(sel_len));
        self.anchor = start + s;
        self.head = start + e;
        self.goal_col = None;
    }

    /// Abandons the composition: the marked text goes and what it replaced (and the selection) comes back.
    pub fn cancel_marked(&mut self) {
        let Some(m) = self.marked.take() else { return };
        if let Some(c) = self.comp.take() {
            self.text.replace_range(m, &c.removed);
            self.anchor = c.before.0;
            self.head = c.before.1;
        } else {
            self.text.replace_range(m.clone(), "");
            self.anchor = m.start;
            self.head = m.start;
        }
        self.goal_col = None;
    }

    /// `unmarkText`. `commit` keeps the marked text as normal text and returns the edit that the whole composition
    /// amounts to (one undo step); otherwise it is cancelled and nothing is returned.
    pub fn unmark_text(&mut self, commit: bool) -> Option<Edit> {
        if !commit {
            self.cancel_marked();
            return None;
        }
        let m = self.marked.take()?;
        let comp = self.comp.take().unwrap_or(Composition { at: m.start, removed: String::new(), before: (self.anchor, self.head) });
        self.anchor = m.end;
        self.head = m.end;
        let inserted = self.text[m].to_string();
        if comp.removed.is_empty() && inserted.is_empty() {
            return None;
        }
        Some(Edit { at: comp.at, removed: comp.removed, inserted, before: comp.before, after: (self.head, self.head) })
    }

    // --- undo --------------------------------------------------------------------------------------------------

    /// Undoes `e` (the text goes back, the selection is as it was before). Does nothing if the text no longer fits.
    pub fn revert(&mut self, e: &Edit) {
        let end = e.at + e.inserted.len();
        if end > self.text.len() || !self.text.is_char_boundary(e.at) || !self.text.is_char_boundary(end) {
            return;
        }
        self.text.replace_range(e.at..end, &e.removed);
        self.marked = None;
        self.comp = None;
        self.set_selection(e.before.0, e.before.1);
    }
    /// Applies `e` again (redo).
    pub fn reapply(&mut self, e: &Edit) {
        let end = e.at + e.removed.len();
        if end > self.text.len() || !self.text.is_char_boundary(e.at) || !self.text.is_char_boundary(end) {
            return;
        }
        self.text.replace_range(e.at..end, &e.inserted);
        self.marked = None;
        self.comp = None;
        self.set_selection(e.after.0, e.after.1);
    }

    /// The text around the caret for predictive text, at most about `max_bytes` bytes, cut on character boundaries.
    pub fn surrounding(&self, max_bytes: usize) -> Surrounding<'_> {
        let len = self.text.len();
        let (base, end) = if len <= max_bytes {
            (0, len)
        } else {
            let start = ceil_boundary(&self.text, self.head.saturating_sub(max_bytes / 2).min(len - max_bytes));
            (start, floor_boundary(&self.text, (start + max_bytes).min(len)))
        };
        let rel = |v: usize| v.saturating_sub(base).min(end - base);
        Surrounding {
            text: &self.text[base..end],
            base,
            cursor: rel(self.head),
            anchor: rel(self.anchor),
            marked: self.marked.as_ref().map(|m| (rel(m.start), rel(m.end))),
        }
    }
}

/// An editing command (`NativeResponder` selectors).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditCommand {
    MoveLeft,
    MoveRight,
    MoveUp,
    MoveDown,
    MoveWordLeft,
    MoveWordRight,
    MoveLineStart,
    MoveLineEnd,
    MoveDocumentStart,
    MoveDocumentEnd,
    /// Page Up: up by a page of lines (`PAGE_LINES` for a view that does not know its height).
    MovePageUp,
    /// Page Down: down by a page of lines.
    MovePageDown,
    DeleteBackward,
    DeleteForward,
    DeleteWordBackward,
    DeleteWordForward,
    DeleteToLineStart,
    KillLine,
    Yank,
    Transpose,
    OpenLine,
    SelectAll,
    Undo,
    Redo,
}

/// The command a key combo means in a text field, and whether it extends the selection (shift). Golden Gate defaults plus
/// the emacs bindings every Cocoa text field has: ctrl-A/E line start/end, ctrl-F/B/N/P, ctrl-D, ctrl-H, ctrl-K, ctrl-Y,
/// ctrl-T, ctrl-O. `cmd` is the primary modifier (Super).
///
/// The whole table (Shift extends the selection for every movement):
///
/// | Keys | Command |
/// |---|---|
/// | Left, Right, Up, Down | by character, by line (a single line goes to its start or end) |
/// | Home, End, Cmd+Left, Cmd+Right, Ctrl+A, Ctrl+E | line start, line end |
/// | Cmd+Up, Cmd+Down, Cmd+Home, Cmd+End, Ctrl+Home, Ctrl+End | document start, document end |
/// | Option+Left, Option+Right, Ctrl+Left, Ctrl+Right | word back, word forward |
/// | Page Up, Page Down | a page up, a page down |
/// | Backspace, Ctrl+H | delete back |
/// | Delete (Fn+Delete), Ctrl+D | delete forward |
/// | Option+Backspace, Ctrl+Backspace | delete the word back |
/// | Option+Delete, Ctrl+Delete | delete the word forward |
/// | Cmd+Backspace | delete to the line start |
/// | Ctrl+K, Ctrl+Y, Ctrl+T, Ctrl+O | kill to the line end, yank, transpose, open line |
/// | Cmd+A, Cmd+Z, Shift+Cmd+Z | select all, undo, redo |
///
/// Copy, cut and paste are the pasteboard's: see [`key_action`].
pub fn binding(k: KeyCombo) -> Option<(EditCommand, bool)> {
    use EditCommand::*;
    let ext = k.shift;
    let cmd = if k.cmd {
        match k.key {
            Key::Left => MoveLineStart,
            Key::Right => MoveLineEnd,
            Key::Up | Key::Home | Key::PageUp => MoveDocumentStart,
            Key::Down | Key::End | Key::PageDown => MoveDocumentEnd,
            Key::Delete => DeleteToLineStart,
            Key::Char(c) if c.eq_ignore_ascii_case(&'a') && !k.shift => return Some((SelectAll, false)),
            Key::Char(c) if c.eq_ignore_ascii_case(&'z') => return Some(if k.shift { (Redo, false) } else { (Undo, false) }),
            _ => return None,
        }
    } else if k.alt {
        match k.key {
            Key::Left => MoveWordLeft,
            Key::Right => MoveWordRight,
            Key::Delete => DeleteWordBackward,
            Key::ForwardDelete => DeleteWordForward,
            _ => return None,
        }
    } else if k.ctrl {
        match k.key {
            Key::Char(c) => match c.to_ascii_lowercase() {
                'a' => MoveLineStart,
                'e' => MoveLineEnd,
                'f' => MoveRight,
                'b' => MoveLeft,
                'n' => MoveDown,
                'p' => MoveUp,
                'd' => DeleteForward,
                'h' => DeleteBackward,
                'k' => KillLine,
                'y' => Yank,
                't' => Transpose,
                'o' => OpenLine,
                _ => return None,
            },
            Key::Left => MoveWordLeft,
            Key::Right => MoveWordRight,
            Key::Home => MoveDocumentStart,
            Key::End => MoveDocumentEnd,
            Key::Delete => DeleteWordBackward,
            Key::ForwardDelete => DeleteWordForward,
            _ => return None,
        }
    } else {
        match k.key {
            Key::Left => MoveLeft,
            Key::Right => MoveRight,
            Key::Up => MoveUp,
            Key::Down => MoveDown,
            Key::Home => MoveLineStart,
            Key::End => MoveLineEnd,
            Key::PageUp => MovePageUp,
            Key::PageDown => MovePageDown,
            Key::Delete => DeleteBackward,
            Key::ForwardDelete => DeleteForward,
            _ => return None,
        }
    };
    Some((cmd, ext))
}

/// What a key combo does in a text control: an editing command (and whether it extends the selection), or one of the
/// three pasteboard commands, which the control runs against the pasteboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyAction {
    Edit(EditCommand, bool),
    Cut,
    Copy,
    Paste,
}

/// The action a key combo means in a text control. The pasteboard chords are Cmd+C, Cmd+X and Cmd+V, and Ctrl+C, Ctrl+X
/// and Ctrl+V as well (the Ctrl form is the desktop's alias of the Cmd one, and the shell turns both into commands before
/// they reach an app). Ctrl+Z is Undo and Ctrl+Shift+Z Redo for the same reason. Everything else is [`binding`]'s, so Ctrl+A
/// is the line start, Ctrl+Y the yank and Cmd+A select all.
pub fn key_action(k: KeyCombo) -> Option<KeyAction> {
    if (k.cmd || k.ctrl) && !k.alt {
        if let Key::Char(c) = k.key {
            match c.to_ascii_lowercase() {
                'c' if !k.shift => return Some(KeyAction::Copy),
                'x' if !k.shift => return Some(KeyAction::Cut),
                'v' if !k.shift => return Some(KeyAction::Paste),
                'z' if k.ctrl && !k.cmd => return Some(KeyAction::Edit(if k.shift { EditCommand::Redo } else { EditCommand::Undo }, false)),
                _ => {}
            }
        }
    }
    binding(k).map(|(c, extend)| KeyAction::Edit(c, extend))
}

/// The selection (anchor, caret) of a drag that began on `origin` (the word or paragraph a double or triple click selected)
/// and is now over `unit` (the word or paragraph under the pointer): their union, anchored at the far end of `origin`
/// so the selection grows by whole words or paragraphs in either direction.
pub fn extend_unit(origin: Range<usize>, unit: Range<usize>) -> (usize, usize) {
    if unit.start < origin.start {
        (origin.end, unit.start)
    } else {
        (origin.start, unit.end.max(origin.end))
    }
}

/// Counts consecutive presses of a pointer on one spot: one for a single click, two for a double click (select a word),
/// three for a triple click (select the paragraph). A press restarts at one when it comes too late or lands too far from
/// the last. Times are milliseconds on any steady clock; positions are points.
#[derive(Clone, Copy, Debug, Default)]
pub struct ClickCounter {
    last: Option<(f64, f32, f32)>,
    count: u8,
}

impl ClickCounter {
    /// Two presses within this many milliseconds are a multiple click.
    pub const INTERVAL_MS: f64 = 450.0;
    /// ... and within this many points of each other.
    pub const SLOP_PT: f32 = 6.0;

    /// A press at (`x`, `y`) at time `t`: the click count, 1 to 3 (a fourth click starts over at one).
    pub fn press(&mut self, t: f64, x: f32, y: f32) -> u8 {
        let near = self.last.is_some_and(|(lt, lx, ly)| t - lt >= 0.0 && t - lt <= Self::INTERVAL_MS && (x - lx).abs() <= Self::SLOP_PT && (y - ly).abs() <= Self::SLOP_PT);
        self.count = if near && self.count < 3 { self.count + 1 } else { 1 };
        self.last = Some((t, x, y));
        self.count
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Typing,
    Other,
}

/// A `TextBuffer` with undo. Typing (`insert_text` without a newline) coalesces into one step until something else
/// happens; a composition is one step when it commits.
pub struct TextEditor {
    pub buf: TextBuffer,
    undo: UndoManager<TextBuffer>,
    typing: bool,
    /// `allowsUndo`: when off, edits are applied but not recorded.
    allows_undo: bool,
    /// The compose method's state (`compose_method`): the armed apostrophe, the kana being composed and an open list.
    compose: Composer,
}

/// The editor as the compose method's target: its marked text, commits and plain typing are the editor's own edits.
struct Editing<'a>(&'a mut TextEditor);

impl compose_method::ComposeTarget for Editing<'_> {
    fn set_marked(&mut self, text: &str) {
        self.0.set_marked_text(text, text.len(), 0, None);
    }
    fn commit_marked(&mut self) {
        self.0.unmark_text(true);
    }
    fn insert(&mut self, text: &str) {
        self.0.insert_text(text);
    }
    fn discard_marked(&mut self) {
        self.0.unmark_text(false);
    }
    fn delete_back(&mut self) {
        self.0.perform(EditCommand::DeleteBackward, false);
    }
}

impl TextEditor {
    pub fn new(limit: usize) -> Self {
        TextEditor { buf: TextBuffer::new(), undo: UndoManager::new(limit), typing: false, allows_undo: true, compose: Composer::default() }
    }
    pub fn with_text(text: &str, limit: usize) -> Self {
        TextEditor { buf: TextBuffer::from_text(text), undo: UndoManager::new(limit), typing: false, allows_undo: true, compose: Composer::default() }
    }

    /// Types `text` the way a typed key is typed: through the compose method, so an apostrophe and a letter with
    /// alternates open the candidate list (`candidates`) and a digit picks from it. True when the text changed.
    pub fn type_text(&mut self, text: &str) -> bool {
        let before = self.buf.text().to_string();
        let mut composer = std::mem::take(&mut self.compose);
        compose_method::type_text(&mut composer, &mut Editing(&mut *self), text);
        self.compose = composer;
        self.buf.text() != before
    }

    /// The candidate list of an open composition (empty when none is open), for the field or view to draw.
    pub fn candidates(&self) -> &[String] {
        self.compose.candidates()
    }

    /// A letter held down (the key repeat's accent hold): the letter typed at its press is taken back and shown as
    /// marked text over its accents (`candidates`). False, and nothing changes, when the letter has none or a
    /// composition is already open.
    pub fn accent_hold(&mut self, letter: char) -> bool {
        let mut composer = std::mem::take(&mut self.compose);
        let done = compose_method::accent_hold(&mut composer, &mut Editing(&mut *self), letter);
        self.compose = composer;
        done
    }

    /// Ends an open composition the way a control key does: the list closes keeping its letter, and a held apostrophe
    /// is typed. Called before every editing command, so no command runs over a half-finished composition.
    pub fn end_composition(&mut self) {
        if !self.compose.is_composing() {
            return;
        }
        let mut composer = std::mem::take(&mut self.compose);
        compose_method::end_for_control(&mut composer, &mut Editing(&mut *self));
        self.compose = composer;
    }

    /// Backspace over an open kana composition removes its last letter or kana, not the whole composition. True when it did.
    fn compose_backspace(&mut self) -> bool {
        let mut composer = std::mem::take(&mut self.compose);
        let done = compose_method::backspace(&mut composer, &mut Editing(&mut *self), compose_method::japanese_source());
        self.compose = composer;
        done
    }

    /// Forgets the compose state without typing anything: an input method or an edit from outside took over the text.
    fn drop_composition(&mut self) {
        self.compose = Composer::default();
    }
    /// `allowsUndo`.
    pub fn allows_undo(&self) -> bool {
        self.allows_undo
    }
    /// `setAllowsUndo:`. Turning it off drops the history.
    pub fn set_allows_undo(&mut self, on: bool) {
        self.allows_undo = on;
        if !on {
            self.end_typing();
            self.undo.clear();
        }
    }
    /// Replaces `range` with `new` as one undo step named `name` (find-bar replace).
    pub fn replace_range(&mut self, range: Range<usize>, new: &str, name: &str) -> bool {
        self.end_composition();
        let e = self.buf.replace_range(range, new);
        self.record(e, name, Kind::Other)
    }
    /// Replaces every range (sorted, disjoint) with `new` as one undo step. Returns how many were replaced.
    pub fn replace_ranges(&mut self, ranges: &[Range<usize>], new: &str, name: &str) -> usize {
        self.end_composition();
        self.end_typing();
        if self.allows_undo {
            self.undo.begin_group(name);
        }
        let mut n = 0;
        for r in ranges.iter().rev() {
            if let Some(e) = self.buf.replace_range(r.clone(), new) {
                n += 1;
                if self.allows_undo {
                    let (a, b) = (e.clone(), e);
                    self.undo.register(name, move |t| t.revert(&a), move |t| t.reapply(&b));
                }
            }
        }
        if self.allows_undo {
            self.undo.end_group();
        }
        n
    }
    pub fn undo_manager(&self) -> &UndoManager<TextBuffer> {
        &self.undo
    }

    fn end_typing(&mut self) {
        if self.typing {
            self.undo.end_group();
            self.typing = false;
        }
    }

    fn record(&mut self, edit: Option<Edit>, name: &str, kind: Kind) -> bool {
        let Some(e) = edit else { return false };
        if !self.allows_undo {
            return true;
        }
        match kind {
            Kind::Typing => {
                if !self.typing {
                    self.undo.begin_group("Typing");
                    self.typing = true;
                }
            }
            Kind::Other => self.end_typing(),
        }
        let (a, b) = (e.clone(), e.clone());
        self.undo.register(name, move |t| t.revert(&a), move |t| t.reapply(&b));
        if kind == Kind::Typing && e.inserted.contains('\n') {
            self.end_typing();
        }
        true
    }

    /// `insertText:`. True when the text changed.
    pub fn insert_text(&mut self, text: &str) -> bool {
        self.end_composition();
        let committing = self.buf.has_marked_text();
        let e = self.buf.insert_text(text);
        if committing {
            return self.record(e, "Typing", Kind::Other);
        }
        self.record(e, "Typing", Kind::Typing)
    }
    /// Pastes `text` over the selection as an undo step of its own ("Paste"), so one undo takes the whole paste back and
    /// not the typing before it. True when the text changed.
    pub fn paste_text(&mut self, text: &str) -> bool {
        self.end_composition();
        let e = self.buf.insert_text(text);
        self.record(e, "Paste", Kind::Other)
    }
    /// Cuts the selection as an undo step of its own ("Cut") and returns it for the pasteboard; `None`, and nothing
    /// changes, when nothing is selected.
    pub fn cut_selection(&mut self) -> Option<String> {
        self.end_composition();
        if !self.buf.has_selection() || self.buf.has_marked_text() {
            return None;
        }
        let cut = self.buf.selected_text().to_string();
        let e = self.buf.insert_text("");
        self.record(e, "Cut", Kind::Other);
        Some(cut)
    }
    /// `setMarkedText:selectedRange:replacementRange:`.
    pub fn set_marked_text(&mut self, text: &str, sel_start: usize, sel_len: usize, replacement: Option<Range<usize>>) {
        self.drop_composition();
        self.end_typing();
        self.buf.set_marked_text(text, sel_start, sel_len, replacement);
    }
    /// `unmarkText`. True when committed text changed the buffer.
    pub fn unmark_text(&mut self, commit: bool) -> bool {
        self.drop_composition();
        let e = self.buf.unmark_text(commit);
        self.record(e, "Typing", Kind::Other)
    }
    /// `delete_surrounding_text`: one undo step.
    pub fn delete_surrounding(&mut self, before: usize, after: usize) -> bool {
        self.drop_composition();
        let edits = self.buf.delete_surrounding(before, after);
        if edits.is_empty() {
            return false;
        }
        self.end_typing();
        self.undo.begin_group("Delete");
        for e in edits {
            let (a, b) = (e.clone(), e);
            self.undo.register("Delete", move |t| t.revert(&a), move |t| t.reapply(&b));
        }
        self.undo.end_group();
        true
    }

    /// Runs `cmd`; `extend` extends the selection for movements. True when the text or selection changed.
    pub fn perform(&mut self, cmd: EditCommand, extend: bool) -> bool {
        use EditCommand::*;
        if matches!(cmd, DeleteBackward) && self.compose_backspace() {
            return true;
        }
        self.end_composition();
        let before = (self.buf.anchor(), self.buf.caret(), self.buf.len());
        let name = match cmd {
            KillLine => "Cut",
            Yank => "Paste",
            Transpose => "Transpose",
            OpenLine => "Typing",
            _ => "Delete",
        };
        let edit = match cmd {
            MoveLeft | MoveRight | MoveUp | MoveDown | MoveWordLeft | MoveWordRight | MoveLineStart | MoveLineEnd
            | MoveDocumentStart | MoveDocumentEnd | MovePageUp | MovePageDown | SelectAll => {
                self.end_typing();
                match cmd {
                    MoveLeft => self.buf.move_char(false, extend),
                    MoveRight => self.buf.move_char(true, extend),
                    MoveUp => self.buf.move_vertical(false, extend),
                    MoveDown => self.buf.move_vertical(true, extend),
                    MoveWordLeft => self.buf.move_word(false, extend),
                    MoveWordRight => self.buf.move_word(true, extend),
                    MoveLineStart => self.buf.move_line(false, extend),
                    MoveLineEnd => self.buf.move_line(true, extend),
                    MoveDocumentStart => self.buf.move_document(false, extend),
                    MoveDocumentEnd => self.buf.move_document(true, extend),
                    MovePageUp => self.buf.move_page(false, extend, PAGE_LINES),
                    MovePageDown => self.buf.move_page(true, extend, PAGE_LINES),
                    _ => self.buf.select_all(),
                }
                return before != (self.buf.anchor(), self.buf.caret(), self.buf.len());
            }
            Undo => return self.undo(),
            Redo => return self.redo(),
            DeleteBackward => {
                let marked = self.buf.has_marked_text();
                let e = self.buf.delete_backward();
                if marked {
                    return before != (self.buf.anchor(), self.buf.caret(), self.buf.len());
                }
                e
            }
            DeleteForward => self.buf.delete_forward(),
            DeleteWordBackward => self.buf.delete_word_backward(),
            DeleteWordForward => self.buf.delete_word_forward(),
            DeleteToLineStart => self.buf.delete_to_line_start(),
            KillLine => self.buf.kill_line(),
            Yank => self.buf.yank(),
            Transpose => self.buf.transpose(),
            OpenLine => self.buf.open_line(),
        };
        self.record(edit, name, Kind::Other)
    }

    /// Handles a key combo with `binding`. `None` when the combo is not an editing key.
    pub fn handle_key(&mut self, k: KeyCombo) -> Option<bool> {
        binding(k).map(|(c, ext)| self.perform(c, ext))
    }

    /// True while typing is open too: the open group is undone as one step.
    pub fn can_undo(&self) -> bool {
        self.typing || self.undo.can_undo()
    }
    pub fn can_redo(&self) -> bool {
        self.undo.can_redo()
    }
    pub fn undo(&mut self) -> bool {
        self.end_composition();
        self.typing = false;
        self.undo.undo(&mut self.buf).is_some()
    }
    pub fn redo(&mut self) -> bool {
        self.end_composition();
        self.typing = false;
        self.undo.redo(&mut self.buf).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str, caret: usize) -> TextBuffer {
        let mut b = TextBuffer::from_text(text);
        b.set_selection(caret, caret);
        b
    }

    fn combo(key: Key) -> KeyCombo {
        KeyCombo { cmd: false, ctrl: false, alt: false, shift: false, key }
    }

    #[test]
    fn graphemes_keep_combining_marks_together() {
        let s = "e\u{301}x";
        assert_eq!(next_boundary(s, 0), 3);
        assert_eq!(next_boundary(s, 3), 4);
        assert_eq!(prev_boundary(s, 3), 0);
        assert_eq!(prev_boundary(s, 4), 3);
        assert_eq!(grapheme_count(s), 2);
    }

    #[test]
    fn graphemes_keep_crlf_flags_zwj_and_modifiers() {
        assert_eq!(next_boundary("\r\nx", 0), 2);
        assert_eq!(next_boundary("\n\n", 0), 1);
        let flags = "🇮🇸🇺🇸";
        assert_eq!(next_boundary(flags, 0), 8);
        assert_eq!(next_boundary(flags, 8), 16);
        assert_eq!(prev_boundary(flags, 16), 8);
        assert_eq!(prev_boundary(flags, 8), 0);
        let family = "👨\u{200d}👩\u{200d}👧!";
        assert_eq!(next_boundary(family, 0), 18);
        assert_eq!(prev_boundary(family, 18), 0);
        assert_eq!(next_boundary("👍🏽", 0), 8);
        assert_eq!(next_boundary("❤\u{fe0f}", 0), 6);
        assert_eq!(next_boundary("\u{1100}\u{1161}\u{11a8}", 0), 9);
    }

    #[test]
    fn boundaries_clamp_at_the_ends() {
        assert_eq!(next_boundary("ab", 2), 2);
        assert_eq!(next_boundary("ab", 99), 2);
        assert_eq!(prev_boundary("ab", 0), 0);
        assert_eq!(prev_boundary("éa", 1), 0, "inside a character gives its start");
        assert_eq!(grapheme_count(""), 0);
    }

    #[test]
    fn word_motion_skips_punctuation_and_spaces() {
        let s = "hello, big  world";
        assert_eq!(next_word_end(s, 0), 5);
        assert_eq!(next_word_end(s, 5), 10);
        assert_eq!(next_word_end(s, 10), 17);
        assert_eq!(next_word_end(s, 17), 17);
        assert_eq!(prev_word_start(s, 17), 12);
        assert_eq!(prev_word_start(s, 12), 7);
        assert_eq!(prev_word_start(s, 7), 0);
        assert_eq!(prev_word_start(s, 0), 0);
        assert_eq!(next_word_end("snake_case x", 0), 10, "underscore is a word character");
        assert_eq!(next_word_end("héllo wörld", 0), 6);
    }

    #[test]
    fn line_helpers() {
        let s = "ab\ncd\nef";
        assert_eq!((line_start(s, 4), line_end(s, 4)), (3, 5));
        assert_eq!((line_start(s, 0), line_end(s, 0)), (0, 2));
        assert_eq!((line_start(s, 8), line_end(s, 8)), (6, 8));
        assert_eq!(line_end(s, 2), 2, "the caret before the newline is on the first line");
        assert_eq!(line_start(s, 3), 3, "after the newline is the next line");
    }

    #[test]
    fn a_crlf_pair_is_one_line_break() {
        let s = "ab\r\ncd\r\nef";
        assert_eq!(line_end(s, 0), 2, "the line ends before the pair, not between its halves");
        assert_eq!((line_start(s, 5), line_end(s, 5)), (4, 6));
        assert_eq!(line_end(s, 9), 10);
        let mut b = at(s, 0);
        b.move_line(true, false);
        assert_eq!(b.caret(), 2);
        b.insert_text("X");
        assert_eq!(b.text(), "abX\r\ncd\r\nef", "typing at the line end keeps the break whole");
        let mut c = at(s, 4);
        c.select_line_at(4);
        assert_eq!(c.selected_text(), "cd", "a triple click does not take the carriage return");
        let mut d = at(s, 0);
        d.kill_line();
        assert_eq!(d.text(), "\r\ncd\r\nef");
        d.kill_line();
        assert_eq!(d.text(), "cd\r\nef", "at the line end the whole pair goes");
        let mut e = at(s, 1);
        e.move_vertical(true, false);
        assert_eq!(e.caret(), 5, "down lands in the next line, not between the halves");
    }

    #[test]
    fn utf16_conversion_counts_surrogate_pairs() {
        let s = "a😀b";
        assert_eq!(byte_to_utf16(s, 0), 0);
        assert_eq!(byte_to_utf16(s, 1), 1);
        assert_eq!(byte_to_utf16(s, 5), 3);
        assert_eq!(byte_to_utf16(s, 6), 4);
        assert_eq!(utf16_to_byte(s, 3), 5);
        assert_eq!(utf16_to_byte(s, 2), 1, "inside a pair gives the start");
        assert_eq!(utf16_to_byte(s, 99), 6);
    }

    #[test]
    fn char_movement_and_selection_collapse() {
        let mut b = at("aé😀", 0);
        b.move_char(true, false);
        assert_eq!(b.caret(), 1);
        b.move_char(true, false);
        assert_eq!(b.caret(), 3);
        b.move_char(true, false);
        assert_eq!(b.caret(), 7);
        b.move_char(true, false);
        assert_eq!(b.caret(), 7, "stops at the end");
        b.move_char(false, true);
        assert_eq!(b.selection(), 3..7);
        assert_eq!((b.anchor(), b.caret()), (7, 3));
        b.move_char(true, false);
        assert_eq!((b.anchor(), b.caret()), (7, 7), "right collapses to the end of the selection");
        b.set_selection(1, 3);
        b.move_char(false, false);
        assert_eq!(b.caret(), 1, "left collapses to the start");
        b.move_char(false, false);
        assert_eq!(b.caret(), 0);
    }

    #[test]
    fn extending_by_word_and_line() {
        let mut b = at("one two three", 0);
        b.move_word(true, true);
        b.move_word(true, true);
        assert_eq!(b.selected_text(), "one two");
        b.move_line(true, true);
        assert_eq!(b.selected_text(), "one two three");
        b.move_word(false, false);
        assert_eq!((b.anchor(), b.caret()), (8, 8));
        b.move_document(false, true);
        assert_eq!(b.selection(), 0..8);
    }

    #[test]
    fn line_start_and_end_like_ctrl_a_ctrl_e() {
        let mut b = at("ab\ncd\nef", 4);
        b.move_line(false, false);
        assert_eq!(b.caret(), 3);
        b.move_line(true, false);
        assert_eq!(b.caret(), 5);
    }

    #[test]
    fn vertical_movement_keeps_the_column() {
        let mut b = at("abcd\nxy\nlmnop", 3);
        b.move_vertical(true, false);
        assert_eq!(b.caret(), 7, "the short line ends at its end");
        b.move_vertical(true, false);
        assert_eq!(b.caret(), 11, "the goal column 3 comes back");
        b.move_vertical(false, false);
        assert_eq!(b.caret(), 7);
        b.move_vertical(false, false);
        assert_eq!(b.caret(), 3);
        b.move_vertical(false, false);
        assert_eq!(b.caret(), 0, "up from the first line goes to the start");
        let mut c = at("ab\ncd", 4);
        c.move_vertical(true, false);
        assert_eq!(c.caret(), 5, "down from the last line goes to the end");
    }

    #[test]
    fn vertical_movement_counts_graphemes() {
        let mut b = at("e\u{301}e\u{301}x\nabc", 6);
        // two clusters of 3 bytes, caret before x is column 2; down lands at column 2 of "abc"
        b.move_vertical(true, false);
        assert_eq!(b.caret(), 8 + 2);
    }

    #[test]
    fn insert_text_replaces_the_selection() {
        let mut b = at("hello world", 0);
        b.set_selection(6, 11);
        let e = b.insert_text("there").unwrap();
        assert_eq!(b.text(), "hello there");
        assert_eq!(b.caret(), 11);
        assert_eq!((e.at, e.removed.as_str(), e.inserted.as_str()), (6, "world", "there"));
        assert_eq!(e.before, (6, 11));
        assert!(b.insert_text("").is_none(), "nothing to do");
    }

    #[test]
    fn delete_backward_takes_a_whole_grapheme() {
        let mut b = at("ae\u{301}", 4);
        b.delete_backward();
        assert_eq!(b.text(), "a");
        b.delete_backward();
        assert_eq!(b.text(), "");
        assert!(b.delete_backward().is_none());
        let mut c = at("x👨\u{200d}👩", 12);
        c.delete_backward();
        assert_eq!(c.text(), "x");
    }

    #[test]
    fn delete_backward_and_forward_remove_a_selection_first() {
        let mut b = at("abcdef", 0);
        b.set_selection(1, 4);
        b.delete_backward();
        assert_eq!((b.text(), b.caret()), ("aef", 1));
        b.set_selection(0, 1);
        b.delete_forward();
        assert_eq!(b.text(), "ef");
        b.set_selection(0, 0);
        b.delete_forward();
        assert_eq!(b.text(), "f");
        b.delete_forward();
        assert!(b.delete_forward().is_none());
    }

    #[test]
    fn word_deletion() {
        let mut b = at("foo bar", 7);
        b.delete_word_backward();
        assert_eq!(b.text(), "foo ");
        b.delete_word_backward();
        assert_eq!(b.text(), "");
        let mut c = at("foo bar", 0);
        c.delete_word_forward();
        assert_eq!(c.text(), " bar");
        c.delete_word_forward();
        assert_eq!(c.text(), "");
    }

    #[test]
    fn delete_to_line_start_stops_at_the_line() {
        let mut b = at("ab\ncdef", 5);
        b.delete_to_line_start();
        assert_eq!(b.text(), "ab\nef");
        assert_eq!(b.caret(), 3);
        b.delete_to_line_start();
        assert_eq!(b.text(), "abef", "at a line start the newline goes");
    }

    #[test]
    fn kill_line_and_yank_like_ctrl_k_ctrl_y() {
        let mut b = at("ab\ncd", 1);
        b.kill_line();
        assert_eq!(b.text(), "a\ncd");
        assert_eq!(b.kill_buffer(), "b");
        b.kill_line();
        assert_eq!(b.text(), "acd", "at the end of a line ctrl-K takes the newline");
        assert_eq!(b.kill_buffer(), "\n");
        b.set_selection(3, 3);
        assert!(b.kill_line().is_none(), "nothing left at the end of the text");
        b.set_selection(1, 1);
        b.kill_line();
        assert_eq!(b.text(), "a");
        b.yank();
        assert_eq!(b.text(), "acd");
        assert_eq!(b.caret(), 3);
    }

    #[test]
    fn transpose_swaps_around_the_caret() {
        let mut b = at("abc", 1);
        b.transpose();
        assert_eq!((b.text(), b.caret()), ("bac", 2));
        let mut c = at("abc", 3);
        c.transpose();
        assert_eq!((c.text(), c.caret()), ("acb", 3));
        let mut d = at("a", 1);
        assert!(d.transpose().is_none());
        let mut e = at("a\nb", 2);
        assert!(e.transpose().is_none(), "never across a newline");
    }

    #[test]
    fn open_line_keeps_the_caret() {
        let mut b = at("ab", 1);
        b.open_line();
        assert_eq!((b.text(), b.caret(), b.anchor()), ("a\nb", 1, 1));
    }

    #[test]
    fn marked_text_composes_and_commits() {
        let mut b = at("ab", 2);
        b.set_marked_text("にほ", 6, 0, None);
        assert_eq!(b.text(), "abにほ");
        assert!(b.has_marked_text());
        assert_eq!(b.marked_range(), Some(2..8));
        assert_eq!(b.caret(), 8);
        b.set_marked_text("日本", 0, 3, None);
        assert_eq!(b.text(), "ab日本");
        assert_eq!(b.marked_range(), Some(2..8));
        assert_eq!(b.selection(), 2..5, "the selection is inside the marked text");
        let e = b.unmark_text(true).unwrap();
        assert!(!b.has_marked_text());
        assert_eq!((e.at, e.removed.as_str(), e.inserted.as_str()), (2, "", "日本"));
        assert_eq!(b.caret(), 8);
        assert!(b.unmark_text(true).is_none());
    }

    #[test]
    fn cancelling_a_composition_restores_the_selection_and_text() {
        let mut b = at("hello", 0);
        b.set_selection(1, 3);
        b.set_marked_text("X", 1, 0, None);
        assert_eq!(b.text(), "hXlo");
        assert_eq!(b.marked_range(), Some(1..2));
        assert!(b.unmark_text(false).is_none());
        assert_eq!(b.text(), "hello");
        assert_eq!(b.selection(), 1..3);
        assert!(!b.has_marked_text());
        b.set_marked_text("Y", 1, 0, None);
        b.set_marked_text("", 0, 0, None);
        assert_eq!(b.text(), "hello", "an empty preedit ends the composition");
        assert!(!b.has_marked_text());
    }

    #[test]
    fn marked_text_replaces_a_given_range_once() {
        let mut b = at("abcdef", 0);
        b.set_marked_text("XY", 2, 0, Some(1..3));
        assert_eq!(b.text(), "aXYdef");
        assert_eq!(b.marked_range(), Some(1..3));
        b.set_marked_text("Z", 1, 0, Some(5..6));
        assert_eq!(b.text(), "aZdef", "a marked range wins over the replacement range");
        let e = b.unmark_text(true).unwrap();
        assert_eq!((e.at, e.removed.as_str(), e.inserted.as_str()), (1, "bc", "Z"));
        b.revert(&e);
        assert_eq!(b.text(), "abcdef");
    }

    #[test]
    fn insert_text_commits_the_marked_text() {
        let mut b = at("ab", 2);
        b.set_marked_text("に", 3, 0, None);
        let e = b.insert_text("日").unwrap();
        assert_eq!(b.text(), "ab日");
        assert!(!b.has_marked_text());
        assert_eq!((e.at, e.removed.as_str(), e.inserted.as_str()), (2, "", "日"));
        assert_eq!(b.caret(), 5);
    }

    #[test]
    fn deleting_backward_inside_a_composition_edits_the_marked_text() {
        let mut b = at("ab", 2);
        b.set_marked_text("xyz", 3, 0, None);
        b.delete_backward();
        assert_eq!(b.text(), "abxy");
        assert_eq!(b.marked_range(), Some(2..4));
        b.delete_backward();
        b.delete_backward();
        assert_eq!(b.text(), "ab");
        assert!(!b.has_marked_text(), "an emptied composition ends");
        assert_eq!(b.caret(), 2);
    }

    #[test]
    fn delete_surrounding_around_a_caret() {
        let mut b = at("hello world", 5);
        let edits = b.delete_surrounding(2, 3);
        assert_eq!(b.text(), "helrld");
        assert_eq!(b.caret(), 3);
        assert_eq!(edits.len(), 2);
        for e in edits.iter().rev() {
            b.revert(e);
        }
        assert_eq!(b.text(), "hello world");
        assert_eq!(b.caret(), 5);
    }

    #[test]
    fn delete_surrounding_keeps_the_selection_text() {
        let mut b = at("abcdef", 0);
        b.set_selection(2, 4);
        b.delete_surrounding(1, 1);
        assert_eq!(b.text(), "acdf");
        assert_eq!(b.selection(), 1..3);
        assert_eq!(b.selected_text(), "cd");
        assert!(b.delete_surrounding(0, 0).is_empty());
    }

    #[test]
    fn delete_surrounding_rounds_out_to_characters() {
        let mut b = at("éé", 2);
        b.delete_surrounding(1, 1);
        assert_eq!(b.text(), "", "one byte of each side cuts the whole character");
    }

    #[test]
    fn surrounding_text_windows_around_the_caret() {
        let mut b = at("héllo", 3);
        let s = b.surrounding(100);
        assert_eq!((s.text, s.base, s.cursor, s.anchor, s.marked), ("héllo", 0, 3, 3, None));
        b.set_marked_text("xy", 2, 0, None);
        assert_eq!(b.surrounding(100).marked, Some((3, 5)));
        let long = TextBuffer::from_text(&"é".repeat(5000));
        let s = long.surrounding(100);
        assert!(s.text.len() <= 100 && !s.text.is_empty());
        assert!(s.base + s.cursor == long.caret());
        assert!(s.text.is_char_boundary(s.cursor));
        let mut mid = TextBuffer::from_text(&"ab".repeat(100));
        mid.set_selection(100, 100);
        let s = mid.surrounding(40);
        assert_eq!((s.base, s.text.len(), s.cursor), (80, 40, 20));
    }

    #[test]
    fn select_word_and_line_at() {
        let mut b = at("one two  three\nfour", 0);
        b.select_word_at(5);
        assert_eq!(b.selected_text(), "two");
        b.select_word_at(8);
        assert_eq!(b.selected_text(), "three", "on a gap it takes the next word");
        b.select_line_at(16);
        assert_eq!(b.selected_text(), "four");
        b.select_all();
        assert_eq!(b.selection(), 0..b.len());
    }

    #[test]
    fn substring_clamps() {
        let b = at("héllo", 0);
        assert_eq!(b.substring(0..3), "hé");
        assert_eq!(b.substring(0..2), "h", "inside a character rounds down");
        assert_eq!(b.substring(3..99), "llo");
        assert_eq!(b.substring(9..99), "");
    }

    #[test]
    fn revert_and_reapply_are_inverse() {
        let mut b = at("hello world", 0);
        b.set_selection(0, 5);
        let e = b.insert_text("bye").unwrap();
        assert_eq!(b.text(), "bye world");
        b.revert(&e);
        assert_eq!((b.text(), b.selection()), ("hello world", 0..5));
        b.reapply(&e);
        assert_eq!((b.text(), b.caret()), ("bye world", 3));
        let stale = Edit { at: 50, removed: "x".into(), inserted: "y".into(), before: (0, 0), after: (0, 0) };
        b.revert(&stale);
        b.reapply(&stale);
        assert_eq!(b.text(), "bye world", "an edit that no longer fits is ignored");
    }

    /// Runs a key chord against an editor the way a text control does: editing commands on the editor, the pasteboard
    /// commands against `clip`.
    fn press(ed: &mut TextEditor, keys: &str, clip: &mut String) {
        let combo = KeyCombo::parse(keys).unwrap_or_else(|| panic!("{keys}: not a chord"));
        match key_action(combo).unwrap_or_else(|| panic!("{keys}: no action")) {
            KeyAction::Edit(cmd, extend) => {
                ed.perform(cmd, extend);
            }
            KeyAction::Copy => *clip = ed.buf.selected_text().to_string(),
            KeyAction::Cut => {
                if let Some(cut) = ed.cut_selection() {
                    *clip = cut;
                }
            }
            KeyAction::Paste => {
                let text = clip.clone();
                ed.paste_text(&text);
            }
        }
    }

    /// Every standard text key: the text, the selection (anchor, caret) before, the chord, and the text and selection after.
    /// The text is two lines: "one two three" (bytes 0 to 13), a newline at 13, and "four five six" (14 to 27).
    #[test]
    fn the_standard_text_keys_act_on_text_caret_and_selection() {
        const T: &str = "one two three\nfour five six";
        #[rustfmt::skip]
        let table: &[(&str, &str, (usize, usize), &str, &str, (usize, usize))] = &[
            // characters
            ("left", T, (5, 5), "left", T, (4, 4)),
            ("shift+left", T, (5, 5), "shift+left", T, (5, 4)),
            ("right", T, (5, 5), "right", T, (6, 6)),
            ("shift+right", T, (5, 5), "shift+right", T, (5, 6)),
            ("right collapses a selection to its end", T, (2, 6), "right", T, (6, 6)),
            ("left collapses a selection to its start", T, (6, 2), "left", T, (2, 2)),
            // lines
            ("home", T, (18, 18), "home", T, (14, 14)),
            ("shift+home", T, (18, 18), "shift+home", T, (18, 14)),
            ("end", T, (16, 16), "end", T, (27, 27)),
            ("shift+end", T, (16, 16), "shift+end", T, (16, 27)),
            ("end on the first line stops before the newline", T, (4, 4), "end", T, (13, 13)),
            ("cmd+left", T, (18, 18), "cmd+left", T, (14, 14)),
            ("cmd+right", T, (4, 4), "cmd+right", T, (13, 13)),
            ("shift+cmd+left", T, (18, 18), "shift+cmd+left", T, (18, 14)),
            ("shift+cmd+right", T, (4, 4), "shift+cmd+right", T, (4, 13)),
            ("ctrl+a is the line start", T, (16, 16), "ctrl+a", T, (14, 14)),
            ("ctrl+e is the line end", T, (16, 16), "ctrl+e", T, (27, 27)),
            ("shift+ctrl+e extends to the line end", T, (16, 16), "shift+ctrl+e", T, (16, 27)),
            // the document
            ("cmd+up", T, (16, 16), "cmd+up", T, (0, 0)),
            ("cmd+down", T, (16, 16), "cmd+down", T, (27, 27)),
            ("shift+cmd+up", T, (16, 16), "shift+cmd+up", T, (16, 0)),
            ("shift+cmd+down", T, (16, 16), "shift+cmd+down", T, (16, 27)),
            ("cmd+home", T, (16, 16), "cmd+home", T, (0, 0)),
            ("cmd+end", T, (16, 16), "cmd+end", T, (27, 27)),
            ("ctrl+home", T, (16, 16), "ctrl+home", T, (0, 0)),
            // words
            ("alt+left", T, (9, 9), "alt+left", T, (8, 8)),
            ("alt+right", T, (4, 4), "alt+right", T, (7, 7)),
            ("shift+alt+left", T, (13, 13), "shift+alt+left", T, (13, 8)),
            ("shift+alt+right", T, (0, 0), "shift+alt+right", T, (0, 3)),
            ("ctrl+left is the word too", T, (9, 9), "ctrl+left", T, (8, 8)),
            ("ctrl+right is the word too", T, (4, 4), "ctrl+right", T, (7, 7)),
            // lines up and down, pages
            ("up keeps the column", T, (16, 16), "up", T, (2, 2)),
            ("down keeps the column", T, (5, 5), "down", T, (19, 19)),
            ("up on the first line goes to the start", T, (5, 5), "up", T, (0, 0)),
            ("down on the last line goes to the end", T, (16, 16), "down", T, (27, 27)),
            ("shift+up", T, (16, 16), "shift+up", T, (16, 2)),
            ("shift+down", T, (5, 5), "shift+down", T, (5, 19)),
            ("page up", T, (16, 16), "pageup", T, (0, 0)),
            ("page down", T, (5, 5), "pagedown", T, (27, 27)),
            ("shift+page up", T, (16, 16), "shift+pageup", T, (16, 0)),
            ("shift+page down", T, (5, 5), "shift+pagedown", T, (5, 27)),
            // deleting
            ("backspace", T, (5, 5), "backspace", "one wo three\nfour five six", (4, 4)),
            ("backspace over a selection", T, (4, 7), "backspace", "one  three\nfour five six", (4, 4)),
            ("ctrl+h is backspace", T, (5, 5), "ctrl+h", "one wo three\nfour five six", (4, 4)),
            ("forward delete", T, (4, 4), "forwarddelete", "one wo three\nfour five six", (4, 4)),
            ("ctrl+d is forward delete", T, (4, 4), "ctrl+d", "one wo three\nfour five six", (4, 4)),
            ("forward delete at the end does nothing", T, (27, 27), "forwarddelete", T, (27, 27)),
            ("backspace at the start does nothing", T, (0, 0), "backspace", T, (0, 0)),
            ("option+backspace deletes the word back", T, (7, 7), "alt+backspace", "one  three\nfour five six", (4, 4)),
            ("ctrl+backspace deletes the word back", T, (7, 7), "ctrl+backspace", "one  three\nfour five six", (4, 4)),
            ("option+forward delete deletes the word forward", T, (4, 4), "alt+forwarddelete", "one  three\nfour five six", (4, 4)),
            ("ctrl+forward delete deletes the word forward", T, (4, 4), "ctrl+forwarddelete", "one  three\nfour five six", (4, 4)),
            ("cmd+backspace deletes to the line start", T, (20, 20), "cmd+backspace", "one two three\nive six", (14, 14)),
            ("cmd+backspace at a line start joins the lines", T, (14, 14), "cmd+backspace", "one two threefour five six", (13, 13)),
            ("ctrl+k kills to the line end", T, (4, 4), "ctrl+k", "one \nfour five six", (4, 4)),
            ("ctrl+k at the line end kills the newline", T, (13, 13), "ctrl+k", "one two threefour five six", (13, 13)),
            // select all
            ("cmd+a", T, (3, 3), "cmd+a", T, (0, 27)),
            ("cmd+a again", T, (0, 27), "cmd+a", T, (0, 27)),
        ];
        for (label, text, (anchor, head), keys, want_text, (want_anchor, want_head)) in table {
            let mut ed = TextEditor::with_text(text, 20);
            ed.buf.set_selection(*anchor, *head);
            press(&mut ed, keys, &mut String::new());
            assert_eq!(ed.buf.text(), *want_text, "{label} ({keys}): text");
            assert_eq!((ed.buf.anchor(), ed.buf.caret()), (*want_anchor, *want_head), "{label} ({keys}): selection");
        }
    }

    #[test]
    fn copy_cut_and_paste_go_through_the_pasteboard_and_are_undoable() {
        let mut ed = TextEditor::with_text("one two three", 20);
        let mut clip = String::new();
        ed.buf.set_selection(0, 4);
        press(&mut ed, "cmd+c", &mut clip);
        assert_eq!((clip.as_str(), ed.buf.text()), ("one ", "one two three"), "copy leaves the text");
        clip.clear();
        press(&mut ed, "cmd+x", &mut clip);
        assert_eq!((clip.as_str(), ed.buf.text(), ed.buf.caret()), ("one ", "two three", 0), "cut takes the selection");
        press(&mut ed, "cmd+z", &mut clip);
        assert_eq!((ed.buf.text(), ed.buf.selection()), ("one two three", 0..4), "undo puts the cut back and selects it again");
        press(&mut ed, "shift+cmd+z", &mut clip);
        assert_eq!(ed.buf.text(), "two three", "redo cuts again");
        ed.buf.set_selection(3, 3);
        clip = "XY".into();
        press(&mut ed, "cmd+v", &mut clip);
        assert_eq!((ed.buf.text(), ed.buf.caret()), ("twoXY three", 5), "paste at the caret");
        press(&mut ed, "cmd+z", &mut clip);
        assert_eq!((ed.buf.text(), ed.buf.caret()), ("two three", 3), "undo takes the paste back");
        ed.buf.set_selection(0, 3);
        press(&mut ed, "cmd+v", &mut clip);
        assert_eq!(ed.buf.text(), "XY three", "paste replaces the selection");
        press(&mut ed, "ctrl+z", &mut clip);
        assert_eq!(ed.buf.text(), "two three", "ctrl+z undoes too");
        press(&mut ed, "ctrl+shift+z", &mut clip);
        assert_eq!(ed.buf.text(), "XY three", "ctrl+shift+z redoes");
        press(&mut ed, "cmd+c", &mut clip);
        assert_eq!(clip, "", "copy with nothing selected copies the empty selection");
    }

    #[test]
    fn copying_cutting_and_deleting_are_undoable_steps_and_typing_replaces_a_selection() {
        let mut ed = TextEditor::with_text("hello world", 20);
        ed.buf.set_selection(6, 11);
        assert!(ed.type_text("there"));
        assert_eq!(ed.buf.text(), "hello there", "typing replaces the selection");
        assert!(ed.undo());
        assert_eq!(ed.buf.text(), "hello world");
        let mut clip = String::new();
        ed.buf.set_selection(0, 6);
        press(&mut ed, "backspace", &mut clip);
        assert_eq!(ed.buf.text(), "world");
        press(&mut ed, "cmd+z", &mut clip);
        assert_eq!(ed.buf.text(), "hello world", "a delete is one undo step");
        ed.buf.set_selection(6, 6);
        press(&mut ed, "alt+backspace", &mut clip);
        assert_eq!(ed.buf.text(), "world");
        press(&mut ed, "cmd+z", &mut clip);
        assert_eq!(ed.buf.text(), "hello world", "a word delete is one undo step");
    }

    #[test]
    fn a_single_line_goes_to_its_ends_on_up_and_down() {
        let mut ed = TextEditor::with_text("hello", 20);
        ed.buf.set_selection(2, 2);
        press(&mut ed, "up", &mut String::new());
        assert_eq!(ed.buf.caret(), 0);
        ed.buf.set_selection(2, 2);
        press(&mut ed, "down", &mut String::new());
        assert_eq!(ed.buf.caret(), 5);
        ed.buf.set_selection(2, 2);
        press(&mut ed, "shift+up", &mut String::new());
        assert_eq!((ed.buf.anchor(), ed.buf.caret()), (2, 0));
    }

    #[test]
    fn key_actions_name_the_pasteboard_chords_and_leave_the_rest_to_the_binding() {
        let act = |s: &str| key_action(KeyCombo::parse(s).unwrap());
        assert_eq!(act("cmd+c"), Some(KeyAction::Copy));
        assert_eq!(act("cmd+x"), Some(KeyAction::Cut));
        assert_eq!(act("cmd+v"), Some(KeyAction::Paste));
        assert_eq!(act("ctrl+c"), Some(KeyAction::Copy));
        assert_eq!(act("ctrl+x"), Some(KeyAction::Cut));
        assert_eq!(act("ctrl+v"), Some(KeyAction::Paste));
        assert_eq!(act("alt+cmd+c"), None, "Option+Cmd+C is the app's");
        assert_eq!(act("shift+cmd+c"), None, "Shift+Cmd+C is the app's");
        assert_eq!(act("ctrl+z"), Some(KeyAction::Edit(EditCommand::Undo, false)));
        assert_eq!(act("ctrl+shift+z"), Some(KeyAction::Edit(EditCommand::Redo, false)));
        assert_eq!(act("ctrl+y"), Some(KeyAction::Edit(EditCommand::Yank, false)), "Ctrl+Y is the yank");
        assert_eq!(act("cmd+z"), Some(KeyAction::Edit(EditCommand::Undo, false)));
        assert_eq!(act("cmd+a"), Some(KeyAction::Edit(EditCommand::SelectAll, false)));
        assert_eq!(act("ctrl+a"), Some(KeyAction::Edit(EditCommand::MoveLineStart, false)), "Ctrl+A is the line start, not select all");
        assert_eq!(act("a"), None, "a plain letter is text");
    }

    #[test]
    fn dragging_after_a_double_click_grows_by_whole_units() {
        // the press selected "two" (4..7); the pointer moves over "three" (8..13), then back over "one" (0..3)
        assert_eq!(extend_unit(4..7, 8..13), (4, 13));
        assert_eq!(extend_unit(4..7, 0..3), (7, 0));
        assert_eq!(extend_unit(4..7, 4..7), (4, 7));
    }

    #[test]
    fn a_double_click_selects_a_word_and_a_triple_click_the_paragraph() {
        let mut clicks = ClickCounter::default();
        assert_eq!(clicks.press(1000.0, 10.0, 10.0), 1);
        assert_eq!(clicks.press(1200.0, 11.0, 10.0), 2, "close in time and place");
        assert_eq!(clicks.press(1400.0, 11.0, 10.0), 3);
        assert_eq!(clicks.press(1600.0, 11.0, 10.0), 1, "a fourth click starts over");
        assert_eq!(clicks.press(2600.0, 11.0, 10.0), 1, "too late");
        assert_eq!(clicks.press(2700.0, 60.0, 10.0), 1, "too far away");
        let mut b = at("one two three\nfour five six", 0);
        b.select_word_at(5);
        assert_eq!(b.selected_text(), "two", "a double click selects the word");
        b.select_line_at(5);
        assert_eq!(b.selected_text(), "one two three", "a triple click selects the paragraph");
        b.select_line_at(20);
        assert_eq!(b.selected_text(), "four five six");
        // dragging from the word end keeps the anchor: the selection grows from where the press began
        b.set_selection(4, 4);
        b.set_selection(b.anchor(), 13);
        assert_eq!(b.selected_text(), "two three");
    }

    #[test]
    fn bindings_cover_the_emacs_and_mac_keys() {
        use EditCommand::*;
        let ctrl = |c: char| KeyCombo { ctrl: true, ..combo(Key::Char(c)) };
        assert_eq!(binding(ctrl('a')), Some((MoveLineStart, false)));
        assert_eq!(binding(ctrl('E')), Some((MoveLineEnd, false)));
        assert_eq!(binding(KeyCombo { shift: true, ..ctrl('e') }), Some((MoveLineEnd, true)));
        assert_eq!(binding(ctrl('k')), Some((KillLine, false)));
        assert_eq!(binding(ctrl('y')), Some((Yank, false)));
        assert_eq!(binding(ctrl('d')), Some((DeleteForward, false)));
        assert_eq!(binding(ctrl('h')), Some((DeleteBackward, false)));
        assert_eq!(binding(ctrl('f')), Some((MoveRight, false)));
        assert_eq!(binding(ctrl('b')), Some((MoveLeft, false)));
        assert_eq!(binding(ctrl('n')), Some((MoveDown, false)));
        assert_eq!(binding(ctrl('p')), Some((MoveUp, false)));
        assert_eq!(binding(ctrl('t')), Some((Transpose, false)));
        assert_eq!(binding(ctrl('o')), Some((OpenLine, false)));
        assert_eq!(binding(ctrl('q')), None);
        assert_eq!(binding(KeyCombo { alt: true, ..combo(Key::Left) }), Some((MoveWordLeft, false)));
        assert_eq!(binding(KeyCombo { alt: true, shift: true, ..combo(Key::Right) }), Some((MoveWordRight, true)));
        assert_eq!(binding(KeyCombo { alt: true, ..combo(Key::Delete) }), Some((DeleteWordBackward, false)));
        assert_eq!(binding(KeyCombo { cmd: true, ..combo(Key::Left) }), Some((MoveLineStart, false)));
        assert_eq!(binding(KeyCombo { cmd: true, ..combo(Key::Down) }), Some((MoveDocumentEnd, false)));
        assert_eq!(binding(KeyCombo { cmd: true, ..combo(Key::Delete) }), Some((DeleteToLineStart, false)));
        assert_eq!(binding(KeyCombo { cmd: true, ..combo(Key::Char('z')) }), Some((Undo, false)));
        assert_eq!(binding(KeyCombo { cmd: true, shift: true, ..combo(Key::Char('z')) }), Some((Redo, false)));
        assert_eq!(binding(KeyCombo { cmd: true, ..combo(Key::Char('a')) }), Some((SelectAll, false)));
        assert_eq!(binding(combo(Key::Delete)), Some((DeleteBackward, false)));
        assert_eq!(binding(combo(Key::Char('a'))), None, "plain letters are text, not commands");
        assert_eq!(binding(KeyCombo { shift: true, ..combo(Key::Left) }), Some((MoveLeft, true)));
        assert_eq!(binding(combo(Key::Home)), Some((MoveLineStart, false)));
        assert_eq!(binding(KeyCombo { shift: true, ..combo(Key::End) }), Some((MoveLineEnd, true)));
        assert_eq!(binding(combo(Key::PageUp)), Some((MovePageUp, false)));
        assert_eq!(binding(KeyCombo { shift: true, ..combo(Key::PageDown) }), Some((MovePageDown, true)));
        assert_eq!(binding(combo(Key::ForwardDelete)), Some((DeleteForward, false)));
        assert_eq!(binding(KeyCombo { alt: true, ..combo(Key::ForwardDelete) }), Some((DeleteWordForward, false)));
        assert_eq!(binding(KeyCombo { cmd: true, ..combo(Key::Home) }), Some((MoveDocumentStart, false)));
    }

    #[test]
    fn typing_is_one_undo_step() {
        let mut ed = TextEditor::new(50);
        for c in ["a", "b", "c"] {
            assert!(ed.insert_text(c));
        }
        assert_eq!(ed.buf.text(), "abc");
        assert!(ed.can_undo());
        assert!(ed.undo());
        assert_eq!(ed.buf.text(), "");
        assert!(!ed.can_undo());
        assert!(ed.can_redo());
        assert!(ed.redo());
        assert_eq!(ed.buf.text(), "abc");
        assert_eq!(ed.buf.caret(), 3);
    }

    #[test]
    fn moving_the_caret_splits_typing_into_two_steps() {
        let mut ed = TextEditor::new(50);
        ed.insert_text("a");
        ed.perform(EditCommand::MoveLeft, false);
        ed.insert_text("b");
        assert_eq!(ed.buf.text(), "ba");
        ed.undo();
        assert_eq!(ed.buf.text(), "a");
        ed.undo();
        assert_eq!(ed.buf.text(), "");
    }

    #[test]
    fn a_newline_ends_the_typing_step() {
        let mut ed = TextEditor::new(50);
        ed.insert_text("a");
        ed.insert_text("\n");
        ed.insert_text("b");
        ed.undo();
        assert_eq!(ed.buf.text(), "a\n");
        ed.undo();
        assert_eq!(ed.buf.text(), "");
    }

    #[test]
    fn deletes_and_kills_are_their_own_steps() {
        let mut ed = TextEditor::with_text("hello world", 50);
        ed.perform(EditCommand::DeleteWordBackward, false);
        assert_eq!(ed.buf.text(), "hello ");
        ed.insert_text("x");
        assert_eq!(ed.buf.text(), "hello x");
        ed.undo();
        assert_eq!(ed.buf.text(), "hello ");
        ed.undo();
        assert_eq!(ed.buf.text(), "hello world");
        assert_eq!(ed.buf.caret(), 11, "the selection comes back too");
        assert_eq!(ed.undo_manager().undo_title(), None);
    }

    #[test]
    fn a_composition_is_one_undo_step() {
        let mut ed = TextEditor::new(50);
        ed.insert_text("x");
        ed.set_marked_text("に", 3, 0, None);
        ed.set_marked_text("にほ", 6, 0, None);
        assert!(!ed.can_redo());
        assert!(ed.unmark_text(true));
        assert_eq!(ed.buf.text(), "xにほ");
        ed.undo();
        assert_eq!(ed.buf.text(), "x");
        ed.undo();
        assert_eq!(ed.buf.text(), "");
        ed.redo();
        ed.redo();
        assert_eq!(ed.buf.text(), "xにほ");
    }

    #[test]
    fn a_cancelled_composition_leaves_no_undo_step() {
        let mut ed = TextEditor::new(50);
        ed.set_marked_text("に", 3, 0, None);
        assert!(!ed.unmark_text(false));
        assert_eq!(ed.buf.text(), "");
        assert!(!ed.can_undo());
    }

    #[test]
    fn committing_with_insert_text_is_one_step() {
        let mut ed = TextEditor::new(50);
        ed.set_marked_text("に", 3, 0, None);
        assert!(ed.insert_text("日"));
        assert_eq!(ed.buf.text(), "日");
        ed.undo();
        assert_eq!(ed.buf.text(), "");
    }

    #[test]
    fn delete_surrounding_undoes_as_one_step() {
        let mut ed = TextEditor::with_text("hello world", 50);
        ed.buf.set_selection(5, 5);
        assert!(ed.delete_surrounding(2, 3));
        assert_eq!(ed.buf.text(), "helrld");
        ed.undo();
        assert_eq!(ed.buf.text(), "hello world");
        assert!(!ed.can_undo());
        ed.redo();
        assert_eq!(ed.buf.text(), "helrld");
        assert!(!ed.delete_surrounding(0, 0));
    }

    #[test]
    fn handle_key_runs_bindings() {
        let mut ed = TextEditor::with_text("ab\ncd", 50);
        ed.buf.set_selection(1, 1);
        assert_eq!(ed.handle_key(KeyCombo { ctrl: true, ..combo(Key::Char('k')) }), Some(true));
        assert_eq!(ed.buf.text(), "a\ncd");
        assert_eq!(ed.handle_key(KeyCombo { ctrl: true, ..combo(Key::Char('y')) }), Some(true));
        assert_eq!(ed.buf.text(), "ab\ncd");
        assert_eq!(ed.handle_key(KeyCombo { ctrl: true, ..combo(Key::Char('a')) }), Some(true));
        assert_eq!(ed.buf.caret(), 0);
        assert_eq!(ed.handle_key(KeyCombo { ctrl: true, ..combo(Key::Char('a')) }), Some(false), "already there");
        assert_eq!(ed.handle_key(combo(Key::Char('x'))), None);
        assert_eq!(ed.handle_key(KeyCombo { cmd: true, ..combo(Key::Char('z')) }), Some(true));
        assert_eq!(ed.buf.text(), "a\ncd", "undo of the yank");
        ed.perform(EditCommand::SelectAll, false);
        assert_eq!(ed.buf.selection(), 0..4);
    }

    #[test]
    fn undo_history_is_bounded() {
        let mut ed = TextEditor::with_text("abcdef", 2);
        for _ in 0..4 {
            ed.perform(EditCommand::DeleteBackward, false);
        }
        assert_eq!(ed.buf.text(), "ab");
        ed.undo();
        ed.undo();
        assert_eq!(ed.buf.text(), "abcd");
        assert!(!ed.can_undo());
    }
}

#[cfg(test)]
mod compose_tests {
    use super::*;

    #[test]
    fn an_apostrophe_and_a_letter_open_the_list_and_a_digit_types_the_pick() {
        let mut e = TextEditor::new(10);
        e.type_text("'");
        assert!(e.type_text("e"));
        assert_eq!(e.buf.text(), "e");
        assert_eq!(e.buf.marked_range(), Some(0..1));
        assert_eq!(e.candidates().len(), 4);
        e.type_text("2");
        assert_eq!(e.buf.text(), "è");
        assert_eq!(e.buf.marked_range(), None);
        assert!(e.candidates().is_empty());
    }

    #[test]
    fn an_accent_hold_takes_the_letter_back_and_a_digit_picks_its_accent() {
        let mut e = TextEditor::new(10);
        e.type_text("e");
        assert!(e.accent_hold('e'));
        assert_eq!(e.buf.text(), "e");
        assert_eq!(e.buf.marked_range(), Some(0..1));
        assert_eq!(e.candidates().len(), 4);
        e.type_text("2");
        assert_eq!(e.buf.text(), "è");
        assert!(e.candidates().is_empty());
        // a letter without accents, or a list already open, does nothing
        assert!(!e.accent_hold('x'));
        e.type_text("'");
        e.type_text("o");
        assert!(!e.accent_hold('e'));
    }

    #[test]
    fn a_command_ends_the_list_and_keeps_the_letter() {
        let mut e = TextEditor::new(10);
        e.type_text("'");
        e.type_text("o");
        assert!(!e.candidates().is_empty());
        e.perform(EditCommand::MoveLeft, false);
        assert!(e.candidates().is_empty());
        assert_eq!(e.buf.text(), "o");
        assert_eq!(e.buf.marked_range(), None);
    }

    #[test]
    fn a_held_apostrophe_is_typed_before_a_command() {
        let mut e = TextEditor::new(10);
        e.type_text("'");
        e.perform(EditCommand::DeleteBackward, false);
        assert_eq!(e.buf.text(), "", "the held apostrophe was typed and then deleted");
        e.type_text("'");
        e.perform(EditCommand::MoveLeft, false);
        assert_eq!(e.buf.text(), "'", "the apostrophe was typed before the caret moved");
    }

    #[test]
    fn an_input_method_takes_over_and_drops_an_open_list() {
        let mut e = TextEditor::new(10);
        e.type_text("'");
        e.type_text("a");
        e.set_marked_text("ka", 2, 0, None);
        assert!(e.candidates().is_empty());
        assert_eq!(e.buf.text(), "ka");
    }
}
