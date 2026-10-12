//! The text model of Text Editor: styled characters, the selection, undo and redo, formatting, counts and the find and
//! replace rules. Pure Rust with no shell types, so the rules are tested without the shell or a window.
//!
//! A paragraph ends with a `'\n'` cell. Alignment is a paragraph property, stored on every cell of the paragraph
//! (including its newline), so the layout reads it from the first cell it meets.

use std::ops::Range;

/// The font families a document can use. Index 0 is the default. The names are the ones RTF files carry.
pub const FAMILIES: [&str; 3] = ["Helvetica", "Times", "Courier"];
/// The point size of new text.
pub const DEFAULT_SIZE: f32 = 12.0;
/// The sizes Bigger and Smaller step through.
pub const SIZES: [f32; 9] = [9.0, 10.0, 11.0, 12.0, 14.0, 18.0, 24.0, 36.0, 48.0];
/// The smallest and largest size a document may hold.
pub const MIN_SIZE: f32 = 4.0;
pub const MAX_SIZE: f32 = 144.0;
/// How many edits Undo remembers.
pub const UNDO_LIMIT: usize = 200;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
    Justified,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub size: f32,
    /// Index into `FAMILIES`.
    pub font: u8,
    pub align: Align,
}

impl Default for Style {
    fn default() -> Self {
        Style { bold: false, italic: false, underline: false, size: DEFAULT_SIZE, font: 0, align: Align::Left }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cell {
    pub ch: char,
    pub style: Style,
}

/// What an undo entry is called in the Edit menu ("Undo Typing").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Typing,
    Deleting,
    Cutting,
    Pasting,
    Formatting,
    Replacing,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Typing => "Typing",
            Kind::Deleting => "Deleting",
            Kind::Cutting => "Cutting",
            Kind::Pasting => "Pasting",
            Kind::Formatting => "Formatting",
            Kind::Replacing => "Replacing",
        }
    }

    /// Consecutive edits of these kinds share one undo entry.
    fn coalesces(self) -> bool {
        matches!(self, Kind::Typing | Kind::Deleting)
    }
}

#[derive(Clone, Debug)]
struct Snapshot {
    cells: Vec<Cell>,
    anchor: usize,
    head: usize,
    kind: Kind,
}

/// Where the document is in its edit history. `position` is the undo depth, `epoch` changes when a new edit
/// replaces undone ones, so a state saved on an abandoned branch is never mistaken for the present one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct History {
    pub position: i64,
    pub epoch: u32,
}

#[derive(Clone, Debug)]
pub struct Text {
    cells: Vec<Cell>,
    anchor: usize,
    head: usize,
    /// The style new typing gets when the selection is empty.
    typing: Style,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    last_kind: Option<Kind>,
    epoch: u32,
}

impl Default for Text {
    fn default() -> Self {
        Self::new()
    }
}

impl Text {
    pub fn new() -> Self {
        Text { cells: Vec::new(), anchor: 0, head: 0, typing: Style::default(), undo: Vec::new(), redo: Vec::new(), last_kind: None, epoch: 0 }
    }

    /// Replaces the content (a file was read). Undo, redo and the selection start over.
    pub fn load(&mut self, cells: Vec<Cell>) {
        self.cells = cells;
        self.anchor = 0;
        self.head = 0;
        self.typing = Style::default();
        self.undo.clear();
        self.redo.clear();
        self.last_kind = None;
        self.epoch = 0;
        self.sync_typing();
    }

    pub fn cells(&self) -> &[Cell] {
        &self.cells
    }

    pub fn len(&self) -> usize {
        self.cells.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    /// The text without styles; paragraphs end with `'\n'`.
    pub fn text(&self) -> String {
        self.cells.iter().map(|c| c.ch).collect()
    }

    pub fn anchor(&self) -> usize {
        self.anchor
    }

    pub fn head(&self) -> usize {
        self.head
    }

    /// The selected range, start first. Empty when the caret is a single point.
    pub fn selection(&self) -> Range<usize> {
        self.anchor.min(self.head)..self.anchor.max(self.head)
    }

    pub fn has_selection(&self) -> bool {
        self.anchor != self.head
    }

    /// Moves the selection, clamped to the text. Any move ends the current typing group, so the next typing is its own
    /// undo entry, and the typing style follows the caret.
    pub fn select(&mut self, anchor: usize, head: usize) {
        self.anchor = anchor.min(self.len());
        self.head = head.min(self.len());
        self.last_kind = None;
        self.sync_typing();
    }

    pub fn select_all(&mut self) {
        self.select(0, self.len());
    }

    /// The style a toolbar shows: the first selected character, or the typing style at the caret.
    pub fn style(&self) -> Style {
        if self.has_selection() {
            self.cells[self.selection().start].style
        } else {
            self.typing
        }
    }

    /// The alignment of the paragraph that holds `pos`.
    pub fn align_at(&self, pos: usize) -> Align {
        let para = self.paragraph(pos);
        if let Some(c) = self.cells.get(para.start) {
            return c.style.align;
        }
        // an empty document, or a caret after the last newline: the paragraph before it still decides
        if para.start > 0 {
            return self.cells[para.start - 1].style.align;
        }
        Align::Left
    }

    /// The range of the paragraph containing `pos`, including its newline cell when it has one.
    pub fn paragraph(&self, pos: usize) -> Range<usize> {
        let pos = pos.min(self.len());
        let start = self.cells[..pos].iter().rposition(|c| c.ch == '\n').map_or(0, |i| i + 1);
        let end = self.cells[pos..].iter().position(|c| c.ch == '\n').map_or(self.len(), |i| pos + i + 1);
        start..end
    }

    /// Typing takes the style of the character before the caret (or after it at the start of the text).
    fn sync_typing(&mut self) {
        if self.has_selection() {
            return;
        }
        let at = self.head;
        let style = if at > 0 {
            Some(self.cells[at - 1].style)
        } else {
            self.cells.first().map(|c| c.style)
        };
        self.typing = style.unwrap_or_default();
        self.typing.align = self.align_at(at);
    }

    fn record(&mut self, kind: Kind) {
        if kind.coalesces() && self.last_kind == Some(kind) {
            return;
        }
        if !self.redo.is_empty() {
            self.epoch += 1;
            self.redo.clear();
        }
        self.undo.push(Snapshot { cells: self.cells.clone(), anchor: self.anchor, head: self.head, kind });
        if self.undo.len() > UNDO_LIMIT {
            self.undo.remove(0);
        }
        self.last_kind = Some(kind);
    }

    /// Replaces the selection with `s` (or inserts it at the caret). The new characters take the typing style.
    pub fn insert(&mut self, s: &str, kind: Kind) {
        if s.is_empty() {
            return;
        }
        self.record(kind);
        let range = self.selection();
        let style = self.typing;
        let new: Vec<Cell> = s.chars().map(|ch| Cell { ch, style }).collect();
        let at = range.start;
        self.cells.splice(range, new.iter().copied());
        self.anchor = at + new.len();
        self.head = self.anchor;
        self.sync_typing();
    }

    /// Typed text or a paste. Typing is one undo entry however many characters it adds.
    pub fn type_text(&mut self, s: &str) {
        self.insert(s, Kind::Typing);
    }

    /// Deletes the selection, or the character before the caret.
    pub fn delete_backward(&mut self) {
        if !self.has_selection() {
            if self.head == 0 {
                return;
            }
            self.anchor = self.head - 1;
        }
        self.delete_selection(Kind::Deleting);
    }

    /// Deletes the selection, or the character after the caret.
    pub fn delete_forward(&mut self) {
        if !self.has_selection() {
            if self.head >= self.len() {
                return;
            }
            self.head += 1;
        }
        self.delete_selection(Kind::Deleting);
    }

    /// Removes the selected text and returns it (Cut, or Replace).
    pub fn cut(&mut self) -> String {
        let text: String = self.cells[self.selection()].iter().map(|c| c.ch).collect();
        if self.has_selection() {
            self.delete_selection(Kind::Cutting);
        }
        text
    }

    fn delete_selection(&mut self, kind: Kind) {
        if !self.has_selection() {
            return;
        }
        self.record(kind);
        let range = self.selection();
        let at = range.start;
        self.cells.drain(range);
        self.anchor = at;
        self.head = at;
        self.sync_typing();
    }

    /// The selected characters' paragraphs, extended to whole paragraphs.
    fn paragraphs_of_selection(&self) -> Range<usize> {
        let sel = self.selection();
        let start = self.paragraph(sel.start).start;
        let end = if sel.end > sel.start {
            self.paragraph(sel.end - 1).end
        } else {
            self.paragraph(sel.start).end
        };
        start..end
    }

    /// Applies a style change to the selected characters, or to the typing style when nothing is selected.
    fn restyle(&mut self, f: impl Fn(&mut Style)) {
        self.record(Kind::Formatting);
        if self.has_selection() {
            let range = self.selection();
            for c in &mut self.cells[range] {
                f(&mut c.style);
            }
        } else {
            f(&mut self.typing);
        }
    }

    /// Bold on when the first selected character is plain, off otherwise.
    pub fn toggle_bold(&mut self) {
        let on = !self.style().bold;
        self.restyle(|s| s.bold = on);
    }

    pub fn toggle_italic(&mut self) {
        let on = !self.style().italic;
        self.restyle(|s| s.italic = on);
    }

    pub fn toggle_underline(&mut self) {
        let on = !self.style().underline;
        self.restyle(|s| s.underline = on);
    }

    pub fn set_size(&mut self, size: f32) {
        let size = size.clamp(MIN_SIZE, MAX_SIZE);
        self.restyle(|s| s.size = size);
    }

    /// The next size in `SIZES` above the current one (Format > Font > Bigger).
    pub fn bigger(&mut self) {
        let current = self.style().size;
        let next = SIZES.iter().copied().find(|&s| s > current).unwrap_or(MAX_SIZE);
        self.set_size(next);
    }

    /// The next size in `SIZES` below the current one (Format > Font > Smaller).
    pub fn smaller(&mut self) {
        let current = self.style().size;
        let next = SIZES.iter().rev().copied().find(|&s| s < current).unwrap_or(MIN_SIZE);
        self.set_size(next);
    }

    pub fn set_font(&mut self, font: u8) {
        let font = (font as usize).min(FAMILIES.len() - 1) as u8;
        self.restyle(|s| s.font = font);
    }

    /// Sets the alignment of every paragraph the selection touches.
    pub fn set_align(&mut self, align: Align) {
        self.record(Kind::Formatting);
        let range = self.paragraphs_of_selection();
        for c in &mut self.cells[range] {
            c.style.align = align;
        }
        self.typing.align = align;
    }

    /// The first match of `needle` at or after `from` (or the last one before it when `forward` is false), wrapping
    /// around the text. Returns the matched range.
    pub fn find(&self, query: &str, from: usize, forward: bool, case_sensitive: bool) -> Option<Range<usize>> {
        let hay: Vec<char> = self.cells.iter().map(|c| c.ch).collect();
        let needle: Vec<char> = query.chars().collect();
        if needle.is_empty() || needle.len() > hay.len() {
            return None;
        }
        let at = find_index(&hay, &needle, from, forward, case_sensitive)?;
        Some(at..at + needle.len())
    }

    /// Replace (the Replace button): when the selection is a match, it becomes `replacement`. Returns whether it did.
    pub fn replace_selection(&mut self, query: &str, replacement: &str, case_sensitive: bool) -> bool {
        let sel = self.selection();
        if !self.has_selection() || !matches_at(&self.cells, sel.start, query, case_sensitive) {
            return false;
        }
        let style = self.cells[sel.start].style;
        self.record(Kind::Replacing);
        let new: Vec<Cell> = replacement.chars().map(|ch| Cell { ch, style }).collect();
        let at = sel.start;
        self.cells.splice(sel, new.iter().copied());
        self.anchor = at + new.len();
        self.head = self.anchor;
        self.sync_typing();
        true
    }

    /// Replace All: every non-overlapping match becomes `replacement`, in one undo entry. Returns the count.
    pub fn replace_all(&mut self, query: &str, replacement: &str, case_sensitive: bool) -> usize {
        let needle: Vec<char> = query.chars().collect();
        if needle.is_empty() {
            return 0;
        }
        let hay: Vec<char> = self.cells.iter().map(|c| c.ch).collect();
        let mut starts = Vec::new();
        let mut i = 0;
        while i + needle.len() <= hay.len() {
            if matches_at(&self.cells, i, query, case_sensitive) {
                starts.push(i);
                i += needle.len();
            } else {
                i += 1;
            }
        }
        if starts.is_empty() {
            return 0;
        }
        self.record(Kind::Replacing);
        // build from the end so the earlier indices stay valid
        for &start in starts.iter().rev() {
            let style = self.cells[start].style;
            let new: Vec<Cell> = replacement.chars().map(|ch| Cell { ch, style }).collect();
            self.cells.splice(start..start + needle.len(), new);
        }
        self.select(0, 0);
        starts.len()
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// The name of the edit Undo would take back ("Typing"), when there is one.
    pub fn undo_name(&self) -> Option<&'static str> {
        self.undo.last().map(|s| s.kind.name())
    }

    pub fn redo_name(&self) -> Option<&'static str> {
        self.redo.last().map(|s| s.kind.name())
    }

    pub fn undo(&mut self) -> bool {
        let Some(snap) = self.undo.pop() else { return false };
        self.redo.push(Snapshot { cells: self.cells.clone(), anchor: self.anchor, head: self.head, kind: snap.kind });
        self.restore(snap);
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(snap) = self.redo.pop() else { return false };
        self.undo.push(Snapshot { cells: self.cells.clone(), anchor: self.anchor, head: self.head, kind: snap.kind });
        self.restore(snap);
        true
    }

    fn restore(&mut self, snap: Snapshot) {
        self.cells = snap.cells;
        self.anchor = snap.anchor.min(self.len());
        self.head = snap.head.min(self.len());
        self.last_kind = None;
        self.sync_typing();
    }

    /// Where the document is in its history, for the document controller's change count.
    pub fn history(&self) -> History {
        History { position: self.undo.len() as i64, epoch: self.epoch }
    }

    /// Characters, not counting paragraph breaks.
    pub fn char_count(&self) -> usize {
        self.cells.iter().filter(|c| c.ch != '\n').count()
    }

    /// Words: runs of non-space characters.
    pub fn word_count(&self) -> usize {
        let mut words = 0;
        let mut inside = false;
        for c in &self.cells {
            if c.ch.is_whitespace() {
                inside = false;
            } else if !inside {
                inside = true;
                words += 1;
            }
        }
        words
    }

    /// Paragraphs: the line breaks plus one, so an empty document has one.
    pub fn paragraph_count(&self) -> usize {
        self.cells.iter().filter(|c| c.ch == '\n').count() + 1
    }
}

/// The char that a cell stands for when searching, folded to lower case unless the search is case sensitive.
fn fold(ch: char, case_sensitive: bool) -> char {
    if case_sensitive {
        ch
    } else {
        ch.to_lowercase().next().unwrap_or(ch)
    }
}

/// Whether `query` matches the cells at `start`.
fn matches_at(cells: &[Cell], start: usize, query: &str, case_sensitive: bool) -> bool {
    let mut q = query.chars();
    let mut i = start;
    loop {
        match q.next() {
            None => return true,
            Some(want) => match cells.get(i) {
                Some(c) if fold(c.ch, case_sensitive) == fold(want, case_sensitive) => i += 1,
                _ => return false,
            },
        }
    }
}

/// The index of the first match at or after `from` (or the last one that starts before `from`), wrapping around.
fn find_index(hay: &[char], needle: &[char], from: usize, forward: bool, case_sensitive: bool) -> Option<usize> {
    let last_start = hay.len() - needle.len();
    let at = |i: usize| (0..needle.len()).all(|k| fold(hay[i + k], case_sensitive) == fold(needle[k], case_sensitive));
    if forward {
        let from = from.min(last_start + 1);
        (from..=last_start).find(|&i| at(i)).or_else(|| (0..from.min(last_start + 1)).find(|&i| at(i)))
    } else {
        let from = from.min(last_start + 1);
        (0..from).rev().find(|&i| at(i)).or_else(|| (from..=last_start).rev().find(|&i| at(i)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(s: &str) -> Text {
        let mut t = Text::new();
        t.load(s.chars().map(|ch| Cell { ch, style: Style::default() }).collect());
        t.select(s.chars().count(), s.chars().count());
        t
    }

    #[test]
    fn typing_inserts_at_the_caret_and_coalesces_into_one_undo() {
        let mut t = plain("");
        t.type_text("he");
        t.type_text("llo");
        assert_eq!(t.text(), "hello");
        assert_eq!(t.undo_name(), Some("Typing"));
        assert!(t.undo());
        assert_eq!(t.text(), "", "one undo takes back the whole typing run");
        assert!(t.redo());
        assert_eq!(t.text(), "hello");
    }

    #[test]
    fn moving_the_caret_starts_a_new_typing_group() {
        let mut t = plain("");
        t.type_text("ab");
        t.select(1, 1);
        t.type_text("X");
        assert_eq!(t.text(), "aXb");
        assert!(t.undo());
        assert_eq!(t.text(), "ab");
    }

    #[test]
    fn backspace_and_delete_respect_the_selection() {
        let mut t = plain("abcd");
        t.select(1, 3);
        t.delete_backward();
        assert_eq!(t.text(), "ad");
        t.select(1, 1);
        t.delete_forward();
        assert_eq!(t.text(), "a");
        t.select(0, 0);
        t.delete_backward();
        assert_eq!(t.text(), "a", "nothing before the first character");
    }

    #[test]
    fn bold_toggles_over_the_selection_and_the_typing_style() {
        let mut t = plain("abc");
        t.select(0, 2);
        t.toggle_bold();
        assert!(t.cells()[0].style.bold && t.cells()[1].style.bold && !t.cells()[2].style.bold);
        assert!(t.style().bold);
        t.toggle_bold();
        assert!(!t.cells()[0].style.bold);
        t.select(3, 3);
        t.toggle_italic();
        t.type_text("i");
        assert!(t.cells()[3].style.italic, "typed text takes the typing style");
    }

    #[test]
    fn alignment_applies_to_whole_paragraphs() {
        let mut t = plain("one\ntwo");
        t.select(1, 1);
        t.set_align(Align::Center);
        assert!(t.cells()[0..4].iter().all(|c| c.style.align == Align::Center));
        assert!(t.cells()[4..].iter().all(|c| c.style.align == Align::Left));
        assert_eq!(t.align_at(6), Align::Left);
        assert_eq!(t.align_at(0), Align::Center);
    }

    #[test]
    fn bigger_and_smaller_step_through_the_size_list() {
        let mut t = plain("x");
        t.select(0, 1);
        t.bigger();
        assert_eq!(t.cells()[0].style.size, 14.0);
        t.smaller();
        assert_eq!(t.cells()[0].style.size, 12.0);
        t.set_size(1000.0);
        assert_eq!(t.cells()[0].style.size, MAX_SIZE);
    }

    #[test]
    fn find_wraps_and_respects_case() {
        let t = plain("Cat and cat");
        assert_eq!(t.find("cat", 0, true, false), Some(0..3));
        assert_eq!(t.find("cat", 1, true, false), Some(8..11));
        assert_eq!(t.find("cat", 9, true, false), Some(0..3), "wraps to the start");
        assert_eq!(t.find("cat", 0, true, true), Some(8..11), "case sensitive skips the capital");
        assert_eq!(t.find("cat", 10, false, false), Some(8..11), "backwards from the end");
        assert_eq!(t.find("dog", 0, true, false), None);
        assert_eq!(t.find("", 0, true, false), None);
    }

    #[test]
    fn replace_all_counts_and_is_one_undo() {
        let mut t = plain("a-a-a");
        assert_eq!(t.replace_all("a", "bb", true), 3);
        assert_eq!(t.text(), "bb-bb-bb");
        assert_eq!(t.undo_name(), Some("Replacing"));
        assert!(t.undo());
        assert_eq!(t.text(), "a-a-a");
    }

    #[test]
    fn replace_only_touches_a_selected_match() {
        let mut t = plain("foo foo");
        t.select(0, 3);
        assert!(t.replace_selection("foo", "bar", true));
        assert_eq!(t.text(), "bar foo");
        assert!(!t.replace_selection("foo", "x", true), "selection is no longer a match");
    }

    #[test]
    fn counts_words_characters_and_paragraphs() {
        let t = plain("two words\nthree more  words");
        assert_eq!(t.word_count(), 5);
        assert_eq!(t.char_count(), 26);
        assert_eq!(t.paragraph_count(), 2);
        assert_eq!(plain("").paragraph_count(), 1);
    }

    #[test]
    fn a_new_edit_after_undo_starts_a_new_epoch() {
        let mut t = plain("");
        t.type_text("a");
        let saved = t.history();
        assert!(t.undo());
        t.type_text("b");
        assert_ne!(t.history(), saved);
        assert_eq!(t.history().epoch, saved.epoch + 1);
    }
}
