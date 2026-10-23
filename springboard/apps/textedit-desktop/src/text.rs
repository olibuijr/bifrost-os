//! The text model of Text Editor: styled characters, the selection, undo and redo, formatting, counts and the find and
//! replace rules. Pure Rust with no shell types, so the rules are tested without the shell or a window.
//!
//! A paragraph ends with a `'\n'` cell. Alignment is a paragraph property, stored on every cell of the paragraph
//! (including its newline), so the layout reads it from the first cell it meets.

use std::ops::Range;

/// The font families a document can use. Index 0 is the default. The names are the ones RTF files carry.
pub const FAMILIES: [&str; 3] = ["Helvetica", "Times", "Courier"];
/// The names other programs know the families by (a document written for them names these), in `FAMILIES` order.
pub const FAMILY_NAMES: [&str; 3] = ["Arial", "Times New Roman", "Courier New"];

/// The family a font name from another program is closest to: serif faces map to Times, fixed-width faces to Courier,
/// everything else to the default.
pub fn family_for_name(name: &str) -> u8 {
    let lower = name.to_ascii_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| lower.contains(w));
    if has(&["courier", "mono", "consolas", "menlo", "typewriter"]) {
        2
    } else if has(&["times", "serif", "georgia", "garamond", "cambria", "palatino", "book antiqua"]) && !has(&["sans"]) {
        1
    } else {
        0
    }
}

/// Edit > Insert: a line break starts a new line inside the paragraph, and a page break starts a new page when printing.
/// Both take no width and are drawn as nothing (a page break shows a rule across the rest of its line).
pub const LINE_BREAK: char = '\u{2028}';
pub const PAGE_BREAK: char = '\u{c}';

/// Whether a character is a line or page break.
pub fn is_break(ch: char) -> bool {
    ch == LINE_BREAK || ch == PAGE_BREAK
}

/// The point size of new text.
pub const DEFAULT_SIZE: f32 = 12.0;
/// The sizes Bigger and Smaller step through.
pub const SIZES: [f32; 9] = [9.0, 10.0, 11.0, 12.0, 14.0, 18.0, 24.0, 36.0, 48.0];
/// The smallest and largest size a document may hold.
pub const MIN_SIZE: f32 = 4.0;
pub const MAX_SIZE: f32 = 144.0;
/// The text colours a document can use: a name and red, green, blue. Index 0 is the default text colour (drawn with
/// the window's label colour), so its rgb is only what a file that needs a literal colour gets.
pub const COLORS: [(&str, [u8; 3]); 8] = [
    ("Default", [0, 0, 0]),
    ("Red", [255, 59, 48]),
    ("Orange", [255, 149, 0]),
    ("Yellow", [255, 204, 0]),
    ("Green", [40, 205, 65]),
    ("Blue", [0, 122, 255]),
    ("Purple", [175, 82, 222]),
    ("Gray", [142, 142, 147]),
];
/// The line spacings a paragraph can have, as multiples of the natural line height. Index 0 is single spacing.
pub const SPACINGS: [f32; 5] = [1.0, 1.15, 1.5, 2.0, 2.5];
/// The distance between default tab stops, in points, and the smallest and largest a document may set.
pub const DEFAULT_TAB: f32 = 36.0;
pub const MIN_TAB: f32 = 12.0;
pub const MAX_TAB: f32 = 144.0;
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

/// A paragraph's list marker.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ListKind {
    #[default]
    None,
    Bullet,
    Numbered,
}

/// Edit > Transformations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transform {
    Upper,
    Lower,
    Capitalize,
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
    /// Index into `COLORS`.
    pub color: u8,
    /// Paragraph properties, stored on every cell of the paragraph like `align`: index into `SPACINGS`, and the list.
    pub spacing: u8,
    pub list: ListKind,
    /// Strikethrough, the baseline shift (1 superscript, -1 subscript, 0 none) and the highlight (index into
    /// `COLORS`, 0 for none).
    pub strike: bool,
    pub script: i8,
    pub highlight: u8,
    /// The link the character belongs to: a number from `links::intern`, 0 for none.
    pub link: u16,
    /// Paragraph spacing in points, before and after the paragraph (Format > Text > Spacing). Paragraph properties, stored
    /// on every cell of the paragraph like `align`.
    pub space_before: u8,
    pub space_after: u8,
    /// Writing direction of the paragraph (Format > Text > Writing Direction): right to left when true. A paragraph
    /// property, stored on every cell of the paragraph like `align`.
    pub rtl: bool,
    /// The attachment an `OBJECT` cell shows: its number in the document's attachment list plus one, 0 for none. Not
    /// character formatting: typing never inherits it.
    pub attachment: u16,
}

/// The largest space a paragraph takes before or after, in points.
pub const MAX_PARAGRAPH_SPACE: u8 = 200;

/// The character an attachment (an image or other file inside the document) occupies: one cell, selected, deleted and
/// undone like a letter, drawn as the attachment instead of a glyph.
pub const OBJECT: char = '\u{fffc}';

/// The attachment a style points at, out of an attachment list.
pub fn attachment_in<'a>(list: &'a [Attachment], style: &Style) -> Option<&'a Attachment> {
    (style.attachment as usize).checked_sub(1).and_then(|i| list.get(i))
}

/// The point size an attachment's cell is given: its height over the line factor, so the line is as tall as the
/// picture, kept within the sizes a character can have.
pub fn attachment_size(att: &Attachment) -> f32 {
    (att.height / crate::layout::LINE_FACTOR).clamp(MIN_SIZE, MAX_SIZE)
}

/// A file embedded in the document (Insert > Image, a paste or a drop of an image). `width` and `height` are its size in
/// points as the line draws it; `data` is the file's bytes, saved with the document's package.
#[derive(Clone, Debug, PartialEq)]
pub struct Attachment {
    pub file: String,
    pub width: f32,
    pub height: f32,
    pub data: Vec<u8>,
}

impl Default for Style {
    fn default() -> Self {
        Style {
            bold: false,
            italic: false,
            underline: false,
            size: DEFAULT_SIZE,
            font: 0,
            align: Align::Left,
            color: 0,
            spacing: 0,
            list: ListKind::None,
            strike: false,
            script: 0,
            highlight: 0,
            link: 0,
            space_before: 0,
            space_after: 0,
            rtl: false,
            attachment: 0,
        }
    }
}

impl Style {
    /// Whether two characters draw alike, so they can share one run of text (and one run in a file).
    pub fn same_run(&self, other: &Style) -> bool {
        self.bold == other.bold
            && self.italic == other.italic
            && self.underline == other.underline
            && self.size == other.size
            && self.font == other.font
            && self.color == other.color
            && self.strike == other.strike
            && self.script == other.script
            && self.highlight == other.highlight
            && self.link == other.link
            && self.attachment == other.attachment
    }
}

/// How many cells either side of a caret the cluster rules look at; a cluster longer than this is cut.
const CLUSTER_WINDOW: usize = 64;

/// The cell index of the grapheme cluster boundary before `at`: where Left and Backspace land. A flag, a ZWJ sequence, a
/// skin-tone emoji or a letter with its combining marks is one step.
pub fn prev_cluster(cells: &[Cell], at: usize) -> usize {
    let at = at.min(cells.len());
    let from = at.saturating_sub(CLUSTER_WINDOW);
    let window: String = cells[from..at].iter().map(|c| c.ch).collect();
    let b = bifrost_app::controls::text::prev_boundary(&window, window.len());
    from + window[..b].chars().count()
}

/// The cell index of the grapheme cluster boundary after `at`: where Right and Delete land.
pub fn next_cluster(cells: &[Cell], at: usize) -> usize {
    let at = at.min(cells.len());
    let to = (at + CLUSTER_WINDOW).min(cells.len());
    let window: String = cells[at..to].iter().map(|c| c.ch).collect();
    let b = bifrost_app::controls::text::next_boundary(&window, 0);
    at + window[..b].chars().count()
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
    Inserting,
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
            Kind::Inserting => "Inserting",
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
    /// The distance between tab stops, in points. A document property, kept in the file.
    tab: f32,
    /// The embedded files, numbered from 1 by `Style::attachment`. Only grows while the document is open, so an undone
    /// insert leaves its entry behind (harmless: saving writes only the attachments still in the text).
    attachments: Vec<Attachment>,
    /// The family typing takes when the caret is where no character gives one (an empty document): the preference
    /// for new documents. Not part of the text.
    default_font: u8,
}

impl Default for Text {
    fn default() -> Self {
        Self::new()
    }
}

impl Text {
    pub fn new() -> Self {
        Text { cells: Vec::new(), anchor: 0, head: 0, typing: Style::default(), undo: Vec::new(), redo: Vec::new(), last_kind: None, epoch: 0, tab: DEFAULT_TAB, attachments: Vec::new(), default_font: 0 }
    }

    /// The family new text takes where no character sets one (the Preferences' default font). Setting it changes no
    /// text and makes no undo entry.
    pub fn set_default_font(&mut self, font: u8) {
        self.default_font = (font as usize).min(FAMILIES.len() - 1) as u8;
        if self.cells.is_empty() {
            self.typing.font = self.default_font;
        }
    }

    /// Replaces the content (a file was read). Undo, redo and the selection start over.
    pub fn load(&mut self, cells: Vec<Cell>) {
        self.load_with_attachments(cells, Vec::new());
    }

    /// Replaces the content with cells and the attachments their `OBJECT` cells number (a file with pictures was read).
    pub fn load_with_attachments(&mut self, cells: Vec<Cell>, attachments: Vec<Attachment>) {
        self.cells = cells;
        self.attachments = attachments;
        self.size_attachment_cells();
        self.anchor = 0;
        self.head = 0;
        self.typing = Style::default();
        self.undo.clear();
        self.redo.clear();
        self.last_kind = None;
        self.epoch = 0;
        self.tab = DEFAULT_TAB;
        self.sync_typing();
    }

    pub fn cells(&self) -> &[Cell] {
        &self.cells
    }

    /// The embedded files, in the order their attachment numbers give them.
    pub fn attachments(&self) -> &[Attachment] {
        &self.attachments
    }

    /// The attachment a character's style points at, if it is an attachment.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn attachment_of(&self, style: &Style) -> Option<&Attachment> {
        attachment_in(&self.attachments, style)
    }

    /// Whether the text holds any attachment (Format > Make Plain Text asks before dropping them).
    pub fn has_attachments(&self) -> bool {
        self.cells.iter().any(|c| c.ch == OBJECT)
    }

    /// Sets the size of every attachment cell from its picture: the line it sits on is as tall as the picture (see
    /// `attachment_size`), so the picture fills its line. A picture taller than the largest size is drawn at that size.
    fn size_attachment_cells(&mut self) {
        for c in self.cells.iter_mut().filter(|c| c.ch == OBJECT) {
            if let Some(att) = attachment_in(&self.attachments, &c.style) {
                c.style.size = attachment_size(att);
            }
        }
    }

    /// Inserts an attachment at the caret (replacing the selection) as one `OBJECT` cell. One undo entry.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn insert_attachment(&mut self, attachment: Attachment, kind: Kind) {
        self.record(kind);
        self.attachments.push(attachment);
        let style = Style { attachment: self.attachments.len() as u16, ..self.typing };
        let range = self.selection();
        let at = range.start;
        self.cells.splice(range, [Cell { ch: OBJECT, style }]);
        self.size_attachment_cells();
        self.anchor = at + 1;
        self.head = self.anchor;
        self.sync_typing();
    }

    pub fn len(&self) -> usize {
        self.cells.len()
    }

    /// The text without styles; paragraphs end with `'\n'`.
    #[cfg_attr(not(test), allow(dead_code))] // word count and find use the cells directly
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

    /// The selected text without styles, or `None` when the caret is a single point.
    pub fn selected_text(&self) -> Option<String> {
        if !self.has_selection() {
            return None;
        }
        Some(self.cells[self.selection()].iter().map(|c| c.ch).collect())
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
        self.typing = style.unwrap_or(Style { font: self.default_font, ..Style::default() });
        // typing after an attachment is text, not another attachment
        self.typing.attachment = 0;
        // typing continues a link only from inside it, not from its end
        let before = if at > 0 { self.cells[at - 1].style.link } else { 0 };
        let after = self.cells.get(at).map_or(0, |c| c.style.link);
        if before != after {
            self.typing.link = 0;
        }
        let para = self.paragraph_style_at(at);
        self.typing.align = para.align;
        self.typing.spacing = para.spacing;
        self.typing.list = para.list;
        self.typing.space_before = para.space_before;
        self.typing.space_after = para.space_after;
    }

    /// The paragraph properties (alignment, spacing, list) of the paragraph that holds `pos`, in a style.
    pub fn paragraph_style_at(&self, pos: usize) -> Style {
        let para = self.paragraph(pos);
        let cell = self.cells.get(para.start).or_else(|| if para.start > 0 { self.cells.get(para.start - 1) } else { None });
        cell.map(|c| c.style).unwrap_or_default()
    }

    /// The distance between tab stops in points.
    pub fn tab_width(&self) -> f32 {
        self.tab
    }

    pub fn set_tab_width(&mut self, width: f32) {
        self.tab = width.clamp(MIN_TAB, MAX_TAB);
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

    /// Pastes cells that keep their own styles (a paste from this document's own copy). The selection is replaced, the
    /// pasted cells keep their formatting, and the text after them takes the style of the caret again.
    pub fn insert_cells(&mut self, cells: &[Cell], kind: Kind) {
        if cells.is_empty() {
            return;
        }
        self.record(kind);
        let range = self.selection();
        let at = range.start;
        self.cells.splice(range, cells.iter().copied());
        self.anchor = at + cells.len();
        self.head = self.anchor;
        self.sync_typing();
    }

    /// Deletes the selection, or the grapheme cluster before the caret.
    pub fn delete_backward(&mut self) {
        if !self.has_selection() {
            if self.head == 0 {
                return;
            }
            self.anchor = prev_cluster(&self.cells, self.head);
        }
        self.delete_selection(Kind::Deleting);
    }

    /// Deletes the selection, or the grapheme cluster after the caret.
    pub fn delete_forward(&mut self) {
        if !self.has_selection() {
            if self.head >= self.len() {
                return;
            }
            self.head = next_cluster(&self.cells, self.head);
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

    /// Bold and italic set together, as the format bar's style pop-up does (Regular, Bold, Italic, Bold Italic).
    pub fn set_face(&mut self, bold: bool, italic: bool) {
        self.restyle(|s| {
            s.bold = bold;
            s.italic = italic;
        });
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

    /// Text colour: an index into `COLORS`.
    pub fn set_color(&mut self, color: u8) {
        let color = color.min(COLORS.len() as u8 - 1);
        self.restyle(|s| s.color = color);
    }

    /// Sets the line spacing (an index into `SPACINGS`) of every paragraph the selection touches.
    pub fn set_spacing(&mut self, spacing: u8) {
        let spacing = spacing.min(SPACINGS.len() as u8 - 1);
        self.restyle_paragraphs(|s| s.spacing = spacing);
    }

    /// Spacing panel: the line spacing (an index into `SPACINGS`) and the space before and after, on every paragraph the
    /// selection touches, as one undo entry.
    pub fn set_paragraph_format(&mut self, spacing: u8, before: u8, after: u8) {
        let spacing = spacing.min(SPACINGS.len() as u8 - 1);
        let before = before.min(MAX_PARAGRAPH_SPACE);
        let after = after.min(MAX_PARAGRAPH_SPACE);
        self.restyle_paragraphs(move |s| {
            s.spacing = spacing;
            s.space_before = before;
            s.space_after = after;
        });
    }

    /// Format > Text > Writing Direction: right to left or left to right, on every paragraph the selection touches.
    pub fn set_writing_direction(&mut self, rtl: bool) {
        self.restyle_paragraphs(move |s| s.rtl = rtl);
    }

    /// The paragraph spacing (before, after) of the paragraph that holds the selection start.
    pub fn paragraph_space_at(&self, pos: usize) -> (u8, u8) {
        let para = self.paragraph_style_at(pos);
        (para.space_before, para.space_after)
    }

    /// Sets the list of every paragraph the selection touches; the same list again takes it off.
    pub fn toggle_list(&mut self, list: ListKind) {
        let now = self.paragraph_style_at(self.selection().start).list;
        let list = if now == list { ListKind::None } else { list };
        self.restyle_paragraphs(|s| s.list = list);
    }

    /// Applies a change to every cell of the paragraphs the selection touches, and to the typing style.
    fn restyle_paragraphs(&mut self, f: impl Fn(&mut Style)) {
        self.record(Kind::Formatting);
        let range = self.paragraphs_of_selection();
        for c in &mut self.cells[range] {
            f(&mut c.style);
        }
        f(&mut self.typing);
    }

    /// Makes the document plain text: every character and paragraph loses its formatting. One undo entry.
    /// Attachments are dropped (the caller asks first when `has_attachments`).
    pub fn make_plain(&mut self) {
        self.record(Kind::Formatting);
        self.cells.retain(|c| c.ch != OBJECT);
        for c in &mut self.cells {
            c.style = Style::default();
        }
        self.typing = Style::default();
        self.anchor = self.anchor.min(self.cells.len());
        self.head = self.head.min(self.cells.len());
    }

    /// Strikethrough on when the first selected character has none, off otherwise.
    pub fn toggle_strike(&mut self) {
        let on = !self.style().strike;
        self.restyle(|s| s.strike = on);
    }

    /// Superscript (1) or subscript (-1); the same shift again puts the text back on the baseline.
    pub fn toggle_script(&mut self, script: i8) {
        let script = if self.style().script == script { 0 } else { script.clamp(-1, 1) };
        self.restyle(|s| s.script = script);
    }

    /// Highlight (background) colour: an index into `COLORS`, 0 for none.
    pub fn set_highlight(&mut self, color: u8) {
        let color = color.min(COLORS.len() as u8 - 1);
        self.restyle(|s| s.highlight = color);
    }

    /// The link number of the character at `pos` (0 for none, or past the end).
    pub fn link_at(&self, pos: usize) -> u16 {
        self.cells.get(pos).map_or(0, |c| c.style.link)
    }

    /// The link number at a caret or click: the character after `pos`, else the one before it.
    pub fn link_near(&self, pos: usize) -> u16 {
        match self.link_at(pos) {
            0 => pos.checked_sub(1).map_or(0, |p| self.link_at(p)),
            id => id,
        }
    }

    /// The whole link at `pos` (see `link_near`): the neighbouring characters with the same number.
    pub fn link_range(&self, pos: usize) -> Option<Range<usize>> {
        let at = if self.link_at(pos) != 0 { pos } else { pos.checked_sub(1).filter(|&p| self.link_at(p) != 0)? };
        let id = self.link_at(at);
        let mut start = at;
        while start > 0 && self.link_at(start - 1) == id {
            start -= 1;
        }
        let mut end = at + 1;
        while end < self.len() && self.link_at(end) == id {
            end += 1;
        }
        Some(start..end)
    }

    /// Points the characters of `range` at link `id` (0 removes the link) as one undo entry. Paragraph breaks are left
    /// out, so a link never spans two paragraphs. Returns whether anything changed.
    pub fn set_link(&mut self, range: Range<usize>, id: u16) -> bool {
        let range = range.start.min(self.len())..range.end.min(self.len());
        if self.cells[range.clone()].iter().all(|c| c.ch == '\n' || c.style.link == id) {
            return false;
        }
        self.record(Kind::Formatting);
        for c in &mut self.cells[range] {
            if c.ch != '\n' {
                c.style.link = id;
            }
        }
        self.sync_typing();
        true
    }

    /// Copy Style: the character formatting at the selection or caret.
    pub fn char_style(&self) -> Style {
        self.style()
    }

    /// Paste Style: gives the selection (or the typing style) the character formatting of `from`. Paragraph
    /// properties are left alone.
    #[cfg_attr(not(test), allow(dead_code))] // Paste Style applies `apply_style`, which takes the paragraph too
    pub fn apply_char_style(&mut self, from: &Style) {
        let f = *from;
        self.restyle(move |s| copy_char(s, &f));
    }

    /// Paste Style as one undo entry: the character formatting on the selected characters (or the typing style) and
    /// the paragraph properties on the paragraphs the selection touches.
    pub fn apply_style(&mut self, from: &Style) {
        self.record(Kind::Formatting);
        if self.has_selection() {
            let range = self.selection();
            for c in &mut self.cells[range] {
                copy_char(&mut c.style, from);
            }
        } else {
            copy_char(&mut self.typing, from);
        }
        let paragraphs = self.paragraphs_of_selection();
        for c in &mut self.cells[paragraphs] {
            copy_para(&mut c.style, from);
        }
        copy_para(&mut self.typing, from);
    }

    /// Edit > Transformations: the selected text in upper case, lower case or capitalised words, in one undo entry.
    /// Returns whether anything changed.
    pub fn transform(&mut self, how: Transform) -> bool {
        let range = self.selection();
        if range.is_empty() {
            return false;
        }
        let mut word_start = range.start == 0 || !self.cells[range.start - 1].ch.is_alphanumeric();
        let mapped: Vec<char> = self.cells[range.clone()]
            .iter()
            .map(|c| {
                let out = match how {
                    Transform::Upper => single(c.ch.to_uppercase(), c.ch),
                    Transform::Lower => single(c.ch.to_lowercase(), c.ch),
                    Transform::Capitalize => {
                        if word_start {
                            single(c.ch.to_uppercase(), c.ch)
                        } else {
                            single(c.ch.to_lowercase(), c.ch)
                        }
                    }
                };
                word_start = !c.ch.is_alphanumeric() && c.ch != '\'';
                out
            })
            .collect();
        if mapped.iter().zip(&self.cells[range.clone()]).all(|(m, c)| *m == c.ch) {
            return false;
        }
        self.record(Kind::Replacing);
        for (cell, ch) in self.cells[range].iter_mut().zip(mapped) {
            cell.ch = ch;
        }
        true
    }

    /// Lines of the page (paragraphs, ending at a newline), counting an empty last one.
    pub fn line_count(&self) -> usize {
        1 + self.cells.iter().filter(|c| c.ch == '\n').count()
    }

    /// The 1-based line number of the paragraph that holds `pos`.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn line_at(&self, pos: usize) -> usize {
        1 + self.cells[..pos.min(self.len())].iter().filter(|c| c.ch == '\n').count()
    }

    /// Selects lines `first..=last` (1-based, `last` clamped to the end), without their last newline. False when
    /// `first` is not a line of the document.
    pub fn select_lines(&mut self, first: usize, last: usize) -> bool {
        let mut starts = vec![0];
        for (i, c) in self.cells.iter().enumerate() {
            if c.ch == '\n' {
                starts.push(i + 1);
            }
        }
        if first == 0 || first > starts.len() {
            return false;
        }
        let last = last.clamp(first, starts.len());
        let start = starts[first - 1];
        let end = if last < starts.len() { starts[last] - 1 } else { self.len() };
        self.select(start, end);
        true
    }

    /// Typed text with the substitutions on: straight quotes become curly ones, and a second hyphen turns `--` into an
    /// em dash. Typing stays one undo entry.
    pub fn type_smart(&mut self, s: &str, quotes: bool, dashes: bool) {
        for ch in s.chars() {
            let at = self.selection().start;
            let prev = at.checked_sub(1).map(|i| self.cells[i].ch);
            if dashes && ch == '-' && !self.has_selection() && prev == Some('-') {
                self.record(Kind::Typing);
                self.cells[at - 1].ch = '\u{2014}';
                continue;
            }
            let out = if quotes && (ch == '"' || ch == '\'') { smart_quote(prev, ch) } else { ch };
            let mut buf = [0u8; 4];
            self.type_text(out.encode_utf8(&mut buf));
        }
    }

    /// Replaces `range` with `s` as one undo entry and selects the new text (word completion).
    pub fn replace_range(&mut self, range: Range<usize>, s: &str) {
        self.select(range.start, range.end);
        self.insert(s, Kind::Replacing);
        self.select(range.start, range.start + s.chars().count());
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

    /// Every match of `query`, in order and not overlapping, at most `limit` (the find bar's highlight).
    pub fn find_all(&self, query: &str, case_sensitive: bool, limit: usize) -> Vec<Range<usize>> {
        let n = query.chars().count();
        let mut out = Vec::new();
        if n == 0 {
            return out;
        }
        let mut i = 0;
        while i + n <= self.cells.len() && out.len() < limit {
            if matches_at(&self.cells, i, query, case_sensitive) {
                out.push(i..i + n);
                i += n;
            } else {
                i += 1;
            }
        }
        out
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

}

/// Copies the character formatting of `from` (what Copy Style keeps) onto `to`.
fn copy_char(to: &mut Style, from: &Style) {
    to.bold = from.bold;
    to.italic = from.italic;
    to.underline = from.underline;
    to.size = from.size;
    to.font = from.font;
    to.color = from.color;
    to.strike = from.strike;
    to.script = from.script;
    to.highlight = from.highlight;
}

/// Copies the paragraph properties of `from` (alignment, line spacing and list) onto `to`.
fn copy_para(to: &mut Style, from: &Style) {
    to.align = from.align;
    to.spacing = from.spacing;
    to.list = from.list;
    to.space_before = from.space_before;
    to.space_after = from.space_after;
}

/// The one character a case mapping gives, or `ch` when it maps to several (a German sharp s becomes two letters).
fn single(mut mapped: impl Iterator<Item = char>, ch: char) -> char {
    match (mapped.next(), mapped.next()) {
        (Some(c), None) => c,
        _ => ch,
    }
}

/// The curly quote for a straight one typed after `prev`: opening at the start, after a space or after an opening
/// bracket or dash, closing otherwise.
pub fn smart_quote(prev: Option<char>, quote: char) -> char {
    let opening = prev.map_or(true, |p| p.is_whitespace() || matches!(p, '(' | '[' | '{' | '\u{2014}' | '\u{2013}' | '\u{201c}' | '\u{2018}'));
    match (quote, opening) {
        ('"', true) => '\u{201c}',
        ('"', false) => '\u{201d}',
        (_, true) => '\u{2018}',
        (_, false) => '\u{2019}',
    }
}

/// Words of the text that start with `prefix` (ignoring case) and are longer than it, each once, most common first
/// and then alphabetical: Edit > Complete.
pub fn completions(cells: &[Cell], prefix: &str) -> Vec<String> {
    if prefix.is_empty() {
        return Vec::new();
    }
    let lower = prefix.to_lowercase();
    let mut counts: Vec<(String, usize)> = Vec::new();
    let mut word = String::new();
    let flush = |word: &mut String, counts: &mut Vec<(String, usize)>| {
        if word.chars().count() > prefix.chars().count() && word.to_lowercase().starts_with(&lower) {
            match counts.iter_mut().find(|(w, _)| w == word) {
                Some((_, n)) => *n += 1,
                None => counts.push((word.clone(), 1)),
            }
        }
        word.clear();
    };
    for c in cells {
        if c.ch.is_alphanumeric() || c.ch == '\'' {
            word.push(c.ch);
        } else {
            flush(&mut word, &mut counts);
        }
    }
    flush(&mut word, &mut counts);
    counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    counts.into_iter().map(|(w, _)| w).collect()
}

/// A line range typed in Select Line: "12" or "12-20" (1-based).
pub fn parse_line_spec(spec: &str) -> Option<(usize, usize)> {
    let spec = spec.trim();
    let (a, b) = match spec.split_once('-') {
        Some((a, b)) => (a.trim(), b.trim()),
        None => (spec, spec),
    };
    let first: usize = a.parse().ok()?;
    let last: usize = b.parse().ok()?;
    (first > 0 && last >= first).then_some((first, last))
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
    fn colour_spacing_and_lists_apply_to_the_right_things() {
        let mut t = plain("ab\ncd");
        t.select(0, 2);
        t.set_color(1);
        assert_eq!(t.cells()[0].style.color, 1);
        assert_eq!(t.cells()[3].style.color, 0);
        t.select(4, 4);
        t.set_spacing(2);
        assert_eq!(t.cells()[3].style.spacing, 2);
        assert_eq!(t.cells()[0].style.spacing, 0);
        t.toggle_list(ListKind::Bullet);
        assert_eq!(t.paragraph_style_at(4).list, ListKind::Bullet);
        assert_eq!(t.paragraph_style_at(0).list, ListKind::None);
        t.toggle_list(ListKind::Bullet);
        assert_eq!(t.paragraph_style_at(4).list, ListKind::None);
    }

    #[test]
    fn plain_text_drops_formatting_in_one_undo_and_tabs_are_bounded() {
        let mut t = plain("ab");
        t.select_all();
        t.toggle_bold();
        t.make_plain();
        assert!(!t.cells()[0].style.bold);
        assert!(t.undo());
        assert!(t.cells()[0].style.bold);
        t.set_tab_width(1.0);
        assert_eq!(t.tab_width(), MIN_TAB);
        t.set_tab_width(1000.0);
        assert_eq!(t.tab_width(), MAX_TAB);
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
    fn backspace_and_delete_remove_a_whole_grapheme_cluster() {
        // a flag (two regional indicators), a family (ZWJ sequence), a thumbs-up with a skin tone, an e with an accent
        let text = "a\u{1F1EE}\u{1F1F8}\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{1F44D}\u{1F3FD}e\u{301}z";
        let mut t = plain(text);
        t.select(t.len(), t.len());
        t.delete_backward();
        assert_eq!(t.text(), text.strip_suffix('z').unwrap(), "z is a single character");
        t.delete_backward();
        assert!(t.text().ends_with('\u{1F3FD}'), "e with its accent went in one step: {:?}", t.text());
        t.delete_backward();
        assert!(t.text().ends_with('\u{1F467}'), "the skin tone went with its emoji");
        t.delete_backward();
        assert_eq!(t.text(), "a\u{1F1EE}\u{1F1F8}", "the whole family went in one step");
        t.delete_backward();
        assert_eq!(t.text(), "a", "the flag went in one step");
        let mut f = plain(text);
        f.select(1, 1);
        f.delete_forward();
        assert!(f.text().starts_with("a\u{1F468}"), "the flag went in one step");
        assert_eq!(next_cluster(f.cells(), 1), 6, "the family is five cells");
        assert_eq!(prev_cluster(f.cells(), 6), 1);
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
    fn the_face_sets_bold_and_italic_together_on_the_selection() {
        let mut t = plain("abc");
        t.select(0, 2);
        t.set_face(true, true);
        assert!(t.cells()[0].style.bold && t.cells()[0].style.italic && t.cells()[1].style.bold);
        assert!(!t.cells()[2].style.bold, "the character outside the selection is untouched");
        t.set_face(false, false);
        assert!(!t.cells()[0].style.bold && !t.cells()[0].style.italic);
        // with no selection the typing style takes it
        t.select(3, 3);
        t.set_face(true, false);
        t.type_text("d");
        assert!(t.cells()[3].style.bold && !t.cells()[3].style.italic);
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
    }

    #[test]
    fn writing_direction_applies_to_every_paragraph_the_selection_touches() {
        let mut t = plain("one\ntwo\nthree");
        t.select(0, 7);
        t.set_writing_direction(true);
        assert!(t.cells()[0].style.rtl && t.cells()[4].style.rtl, "both selected paragraphs turn right to left");
        assert!(!t.cells()[9].style.rtl, "the third paragraph is outside the selection");
        assert_eq!(t.undo_name(), Some("Formatting"));
        t.select(9, 9);
        t.set_writing_direction(true);
        t.select(0, 0);
        t.set_writing_direction(false);
        assert!(!t.cells()[0].style.rtl, "a caret in the first paragraph clears that paragraph");
        assert!(t.cells()[4].style.rtl && t.cells()[9].style.rtl, "the other paragraphs keep their direction");
    }

    #[test]
    fn transformations_change_case_in_one_undo() {
        let mut t = plain("hello wORLD it's");
        t.select_all();
        assert!(t.transform(Transform::Capitalize));
        assert_eq!(t.text(), "Hello World It's");
        assert!(t.transform(Transform::Upper));
        assert_eq!(t.text(), "HELLO WORLD IT'S");
        assert!(!t.transform(Transform::Upper), "nothing changed");
        assert!(t.transform(Transform::Lower));
        assert_eq!(t.text(), "hello world it's");
        assert_eq!(t.undo_name(), Some("Replacing"));
        t.select(2, 2);
        assert!(!t.transform(Transform::Upper), "needs a selection");
    }

    #[test]
    fn strike_script_highlight_and_copied_style_apply_to_characters() {
        let mut t = plain("abc");
        t.select(0, 2);
        t.toggle_strike();
        t.toggle_script(1);
        t.set_highlight(3);
        assert!(t.cells()[1].style.strike && t.cells()[1].style.script == 1 && t.cells()[1].style.highlight == 3);
        assert!(!t.cells()[2].style.strike);
        t.toggle_script(1);
        assert_eq!(t.cells()[0].style.script, 0, "the same shift again clears it");
        t.toggle_script(-1);
        assert_eq!(t.cells()[0].style.script, -1);
        let copied = t.char_style();
        t.select(2, 3);
        t.apply_char_style(&copied);
        assert!(t.cells()[2].style.strike && t.cells()[2].style.highlight == 3);
    }

    #[test]
    fn lines_are_selected_by_number_and_range() {
        let mut t = plain("one\ntwo\nthree");
        assert_eq!(t.line_count(), 3);
        assert_eq!(t.line_at(5), 2);
        assert!(t.select_lines(2, 2));
        assert_eq!(t.selection(), 4..7);
        assert!(t.select_lines(2, 9));
        assert_eq!(t.selection(), 4..13);
        assert!(t.select_lines(1, 1));
        assert_eq!(t.selection(), 0..3);
        assert!(!t.select_lines(4, 4));
        assert!(!t.select_lines(0, 1));
        assert_eq!(parse_line_spec(" 3 "), Some((3, 3)));
        assert_eq!(parse_line_spec("2-5"), Some((2, 5)));
        assert_eq!(parse_line_spec("5-2"), None);
        assert_eq!(parse_line_spec("x"), None);
    }

    #[test]
    fn smart_quotes_and_dashes_substitute_while_typing() {
        let mut t = plain("");
        t.type_smart("\"hi\" it's a--b", true, true);
        assert_eq!(t.text(), "\u{201c}hi\u{201d} it\u{2019}s a\u{2014}b");
        assert!(t.undo());
        assert_eq!(t.text(), "", "the whole run is one undo entry");
        let mut raw = plain("");
        raw.type_smart("\"x\"--", false, false);
        assert_eq!(raw.text(), "\"x\"--");
        assert_eq!(smart_quote(Some('('), '"'), '\u{201c}');
        assert_eq!(smart_quote(Some('a'), '\''), '\u{2019}');
    }

    #[test]
    fn completion_lists_longer_words_with_the_prefix_by_frequency() {
        let t = plain("cable cabin cable cabinet ca");
        let found = completions(t.cells(), "cab");
        assert_eq!(found, vec!["cable", "cabin", "cabinet"]);
        assert!(completions(t.cells(), "").is_empty());
        assert!(completions(t.cells(), "zzz").is_empty());
        let mut e = plain("hello help");
        e.replace_range(6..10, "helping");
        assert_eq!(e.text(), "hello helping");
        assert_eq!(e.selection(), 6..13);
    }

    #[test]
    fn font_names_from_other_programs_map_to_a_family() {
        assert_eq!(family_for_name("Times New Roman"), 1);
        assert_eq!(family_for_name("Courier New"), 2);
        assert_eq!(family_for_name("Liberation Mono"), 2);
        assert_eq!(family_for_name("DejaVu Sans"), 0);
        assert_eq!(family_for_name("Noto Sans Mono"), 2);
        assert_eq!(family_for_name("Arial"), 0);
        for (i, name) in FAMILY_NAMES.iter().enumerate() {
            assert_eq!(family_for_name(name) as usize, i);
        }
    }

    #[test]
    fn a_range_can_be_linked_unlinked_and_the_change_undone() {
        let mut t = plain("see the docs here");
        assert!(t.set_link(4..12, 7));
        assert_eq!(t.link_at(3), 0);
        assert_eq!(t.link_at(4), 7);
        assert_eq!(t.link_at(11), 7);
        assert_eq!(t.link_at(12), 0);
        assert!(!t.set_link(4..12, 7), "the same link again changes nothing");
        assert!(t.set_link(4..8, 0));
        assert_eq!(t.link_range(9), Some(8..12));
        assert!(t.undo());
        assert_eq!(t.link_range(9), Some(4..12));
        assert!(t.undo());
        assert_eq!(t.link_range(9), None);
        assert!(!t.set_link(20..30, 3), "outside the text there is nothing to link");
    }

    #[test]
    fn a_link_finds_its_whole_run_from_inside_or_just_after_it() {
        let mut t = plain("a link b");
        t.set_link(2..6, 3);
        assert_eq!(t.link_range(2), Some(2..6));
        assert_eq!(t.link_range(5), Some(2..6));
        assert_eq!(t.link_range(6), Some(2..6), "the caret just after the link is still at it");
        assert_eq!(t.link_range(7), None);
        assert_eq!(t.link_near(6), 3);
        assert_eq!(t.link_near(1), 0);
        // two different links side by side are two runs
        t.set_link(6..8, 9);
        assert_eq!(t.link_range(3), Some(2..6));
        assert_eq!(t.link_range(7), Some(6..8));
    }

    #[test]
    fn a_link_never_takes_a_paragraph_break() {
        let mut t = plain("one\ntwo");
        assert!(t.set_link(0..7, 5));
        assert_eq!(t.link_at(3), 0);
        assert_eq!(t.link_range(1), Some(0..3));
        assert_eq!(t.link_range(5), Some(4..7));
    }

    #[test]
    fn typing_continues_a_link_only_from_inside_it() {
        let mut t = plain("a link b");
        t.set_link(2..6, 3);
        t.select(4, 4);
        t.type_text("X");
        assert_eq!(t.link_at(4), 3, "typed inside the link");
        t.select(7, 7);
        t.type_text("Y");
        assert_eq!(t.link_at(7), 0, "typed after the link, outside it");
        t.select(0, 0);
        t.type_text("Z");
        assert_eq!(t.link_at(0), 0);
        let end = t.len();
        t.select(end, end);
        assert_eq!(t.style().link, 0);
    }

    #[test]
    fn formatting_a_link_keeps_it_a_link() {
        let mut t = plain("link");
        t.set_link(0..4, 2);
        t.select_all();
        t.toggle_bold();
        assert!(t.cells().iter().all(|c| c.style.bold && c.style.link == 2));
        assert!(!Style { link: 1, ..Style::default() }.same_run(&Style::default()), "a link breaks a run");
    }

    #[test]
    fn pasted_cells_keep_their_formatting_and_are_one_undo() {
        let mut t = plain("ab");
        let bold = Style { bold: true, ..Style::default() };
        t.insert_cells(&[Cell { ch: 'x', style: bold }, Cell { ch: 'y', style: Style::default() }], Kind::Pasting);
        assert_eq!(t.text(), "abxy");
        assert!(t.cells()[2].style.bold && !t.cells()[3].style.bold);
        assert_eq!(t.selection(), 4..4);
        assert_eq!(t.undo_name(), Some("Pasting"));
        t.select(0, 0);
        t.insert_cells(&[], Kind::Pasting);
        assert_eq!(t.text(), "abxy", "nothing to paste changes nothing");
    }

    #[test]
    fn the_spacing_panel_sets_line_and_paragraph_space_in_one_undo() {
        let mut t = plain("one\ntwo");
        t.select(1, 1);
        t.set_paragraph_format(2, 12, 6);
        assert_eq!(t.paragraph_style_at(0).spacing, 2);
        assert_eq!(t.paragraph_space_at(0), (12, 6));
        assert_eq!(t.paragraph_space_at(5), (0, 0), "the other paragraph is not touched");
        assert!(t.undo());
        assert_eq!(t.paragraph_space_at(0), (0, 0), "one undo takes back the spacing");
        assert_eq!(t.paragraph_style_at(0).spacing, 0);
        t.set_paragraph_format(200, 250, 250);
        assert_eq!(t.paragraph_space_at(0), (MAX_PARAGRAPH_SPACE, MAX_PARAGRAPH_SPACE), "clamped");
    }

    #[test]
    fn paste_style_is_one_undo_entry_for_characters_and_paragraphs() {
        let mut t = plain("one\ntwo");
        let from = Style { italic: true, align: Align::Center, ..Style::default() };
        t.select(0, 3);
        t.apply_style(&from);
        assert!(t.cells()[0].style.italic && t.cells()[0].style.align == Align::Center);
        assert!(t.cells()[5].style.align == Align::Left, "the next paragraph keeps its alignment");
        assert!(t.undo());
        assert!(!t.cells()[0].style.italic && t.cells()[0].style.align == Align::Left, "one undo takes back both");
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

#[cfg(test)]
mod attachment_tests {
    use super::*;

    fn picture(file: &str) -> Attachment {
        Attachment { file: file.into(), width: 40.0, height: 20.0, data: vec![1, 2, 3] }
    }

    fn doc_with(s: &str) -> Text {
        let mut t = Text::new();
        t.load(s.chars().map(|ch| Cell { ch, style: Style::default() }).collect());
        t
    }

    #[test]
    fn an_attachment_is_one_selectable_character_that_types_do_not_inherit() {
        let mut t = doc_with("ab");
        t.select(1, 1);
        t.insert_attachment(picture("a.png"), Kind::Pasting);
        assert_eq!(t.text(), format!("a{OBJECT}b"));
        assert_eq!(t.attachment_of(&t.cells()[1].style).map(|a| a.file.as_str()), Some("a.png"));
        // the caret is after the picture; typing there is text, not another picture
        t.type_text("x");
        assert_eq!(t.cells()[2].style.attachment, 0);
        assert_eq!(t.attachment_of(&t.cells()[2].style), None);
        // selecting the picture alone selects one character
        t.select(1, 2);
        assert_eq!(t.selected_text(), Some(OBJECT.to_string()));
    }

    #[test]
    fn deleting_an_attachment_is_one_undo_entry() {
        let mut t = doc_with("ab");
        t.select(1, 1);
        t.insert_attachment(picture("a.png"), Kind::Pasting);
        assert!(t.has_attachments());
        t.select(1, 2);
        t.delete_backward();
        assert_eq!(t.text(), "ab");
        assert!(!t.has_attachments());
    }

    #[test]
    fn undo_restores_a_deleted_attachment_and_redo_removes_it_again() {
        let mut t = doc_with("ab");
        t.select(1, 1);
        t.insert_attachment(picture("a.png"), Kind::Pasting);
        let with_picture = t.text();
        t.select(1, 2);
        t.delete_backward();
        assert!(!t.has_attachments());
        assert!(t.undo());
        assert_eq!(t.text(), with_picture);
        assert_eq!(t.attachment_of(&t.cells()[1].style).map(|a| a.file.as_str()), Some("a.png"));
        assert!(t.redo());
        assert!(!t.has_attachments());
    }

    #[test]
    fn make_plain_drops_the_attachments_and_keeps_the_text() {
        let mut t = doc_with("ab");
        t.select(1, 1);
        t.insert_attachment(picture("a.png"), Kind::Pasting);
        t.make_plain();
        assert_eq!(t.text(), "ab");
        assert!(!t.has_attachments());
        assert!(t.undo(), "one undo entry brings the picture back");
        assert!(t.has_attachments());
    }

    #[test]
    fn the_attachment_table_survives_a_load_only_when_given() {
        let mut t = doc_with("x");
        t.load_with_attachments(vec![Cell { ch: OBJECT, style: Style { attachment: 1, ..Style::default() } }], vec![picture("p.png")]);
        assert_eq!(t.attachments().len(), 1);
        t.load(vec![]);
        assert!(t.attachments().is_empty());
    }

    #[test]
    fn an_attachment_cell_is_as_tall_as_its_picture() {
        let mut t = doc_with("ab");
        t.select(1, 1);
        t.insert_attachment(Attachment { file: "a.png".into(), width: 100.0, height: 50.0, data: Vec::new() }, Kind::Pasting);
        assert_eq!(t.cells()[1].style.size, 50.0 / crate::layout::LINE_FACTOR);
        // a picture taller than the largest size is clamped to it
        t.select(0, 0);
        t.insert_attachment(Attachment { file: "b.png".into(), width: 1.0, height: 900.0, data: Vec::new() }, Kind::Pasting);
        assert_eq!(t.cells()[0].style.size, MAX_SIZE);
    }
}
