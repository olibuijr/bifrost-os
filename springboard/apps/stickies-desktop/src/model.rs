//! The sticky notes board as plain data: the notes, their colour, frame and collapse, and their text. A note's body is
//! text with one style per character, so a selection can be bold, underlined or outlined on its own. The model also
//! holds editing with a caret and selection, find, arrange, import and export, and the JSON file the app keeps in its
//! data directory. No drawing here.

use crate::swatch::{self, Rgb};
use crate::rtf;
use crate::words;
use bifrost_app::attributed::{AttributedString, ColorToken};
use bifrost_app::controls::text::TextBuffer;
use bifrost_core::json::{self, Json};
use std::ops::Range;

/// The title bar height of a note, in points. A collapsed note is only its title bar.
pub const TITLE_H: f32 = 28.0;
/// Width of the disclosure (collapse) control at the left of the title bar, in points.
pub const DISCLOSURE_PT: f32 = 24.0;
/// The resize grip square at the bottom right corner of an expanded note, in points.
pub const GRIP_PT: f32 = 16.0;
/// Inner padding of a note's body, in points.
pub const PAD_PT: f32 = 10.0;
pub const MIN_W: f32 = 180.0;
pub const MIN_H: f32 = 120.0;
pub const DEFAULT_W: f32 = 220.0;
pub const DEFAULT_H: f32 = 200.0;
/// Separates the notes in the exported text file of all notes.
pub const EXPORT_SEPARATOR: &str = "\n\n---\n\n";
/// Largest file the app reads on import (bytes).
pub const IMPORT_MAX: u64 = 1 << 20;
/// How far a duplicated note sits from the note it copies, in points, so the copy shows on top of it.
pub const DUPLICATE_OFFSET_PT: f32 = 24.0;
/// The size a zoomed note takes, in points.
pub const ZOOM_W: f32 = 460.0;
pub const ZOOM_H: f32 = 380.0;
/// Margin and gap of the grid that Arrange By lays the notes out in, in points.
pub const ARRANGE_MARGIN_PT: f32 = 16.0;
/// Height of the bands that Arrange By Location reads as one row of notes, in points.
pub const LOCATION_BAND_PT: f32 = 60.0;

/// The six sticky colours, in menu order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Colour {
    #[default]
    Yellow,
    Orange,
    Green,
    Teal,
    Pink,
    Purple,
}

impl Colour {
    pub const ALL: [Colour; 6] = [Colour::Yellow, Colour::Orange, Colour::Green, Colour::Teal, Colour::Pink, Colour::Purple];

    pub fn index(self) -> usize {
        Colour::ALL.iter().position(|c| *c == self).unwrap_or(0)
    }

    /// The colour at `i`, wrapping, so a stored index past the end still picks a colour.
    pub fn from_index(i: usize) -> Colour {
        Colour::ALL[i % Colour::ALL.len()]
    }

    #[cfg_attr(not(test), allow(dead_code))] // the colour menu cycles through these
    pub fn next(self) -> Colour {
        Colour::from_index(self.index() + 1)
    }

    /// The name stored in the file and shown in the menu.
    pub fn name(self) -> &'static str {
        match self {
            Colour::Yellow => "Yellow",
            Colour::Orange => "Orange",
            Colour::Green => "Green",
            Colour::Teal => "Teal",
            Colour::Pink => "Pink",
            Colour::Purple => "Purple",
        }
    }

    pub fn from_name(name: &str) -> Option<Colour> {
        Colour::ALL.iter().copied().find(|c| c.name() == name)
    }
}

/// Text size of a note's body. The drawing maps each to a system text style token. The size belongs to the whole
/// note: Bigger, Smaller and the size items change every character.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextSize {
    Small,
    Medium,
    Large,
}

impl TextSize {
    pub fn name(self) -> &'static str {
        match self {
            TextSize::Small => "small",
            TextSize::Medium => "medium",
            TextSize::Large => "large",
        }
    }

    pub fn from_name(name: &str) -> TextSize {
        match name {
            "small" => TextSize::Small,
            "large" => TextSize::Large,
            _ => TextSize::Medium,
        }
    }

    /// The next larger size, or the same one at the top (Font > Bigger).
    pub fn bigger(self) -> TextSize {
        match self {
            TextSize::Small => TextSize::Medium,
            _ => TextSize::Large,
        }
    }

    /// The next smaller size, or the same one at the bottom (Font > Smaller).
    pub fn smaller(self) -> TextSize {
        match self {
            TextSize::Large => TextSize::Medium,
            _ => TextSize::Small,
        }
    }
}

/// The colours a note's text can take (Font > Text Colour).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Ink {
    #[default]
    Black,
    Red,
    Blue,
    Green,
    Purple,
}

impl Ink {
    pub const ALL: [Ink; 5] = [Ink::Black, Ink::Red, Ink::Blue, Ink::Green, Ink::Purple];

    pub fn name(self) -> &'static str {
        match self {
            Ink::Black => "Black",
            Ink::Red => "Red",
            Ink::Blue => "Blue",
            Ink::Green => "Green",
            Ink::Purple => "Purple",
        }
    }

    pub fn from_name(name: &str) -> Ink {
        Ink::ALL.iter().copied().find(|i| i.name() == name).unwrap_or_default()
    }
}

/// Where the text sits against the line (Font > Baseline).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Baseline {
    #[default]
    Normal,
    Superscript,
    Subscript,
}

impl Baseline {
    pub const ALL: [Baseline; 3] = [Baseline::Normal, Baseline::Superscript, Baseline::Subscript];

    pub fn title(self) -> &'static str {
        match self {
            Baseline::Normal => "Use Default",
            Baseline::Superscript => "Superscript",
            Baseline::Subscript => "Subscript",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Baseline::Normal => "normal",
            Baseline::Superscript => "super",
            Baseline::Subscript => "sub",
        }
    }

    pub fn from_name(name: &str) -> Baseline {
        match name {
            "super" => Baseline::Superscript,
            "sub" => Baseline::Subscript,
            _ => Baseline::Normal,
        }
    }

    /// Vertical shift as a fraction of the line step: negative is up.
    pub fn shift(self) -> f32 {
        match self {
            Baseline::Normal => 0.0,
            Baseline::Superscript => -0.22,
            Baseline::Subscript => 0.18,
        }
    }
}

/// How a font feature of a run is set: the font's own choice, none of it, or all of it (Font > Kern, Font > Ligature).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Setting {
    #[default]
    Default,
    Off,
    All,
}

impl Setting {
    pub const ALL: [Setting; 3] = [Setting::Default, Setting::Off, Setting::All];

    pub fn title(self) -> &'static str {
        match self {
            Setting::Default => "Use Default",
            Setting::Off => "Use None",
            Setting::All => "Use All",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Setting::Default => "default",
            Setting::Off => "none",
            Setting::All => "all",
        }
    }

    pub fn from_name(name: &str) -> Setting {
        match name {
            "none" => Setting::Off,
            "all" => Setting::All,
            _ => Setting::Default,
        }
    }
}

/// How Window > Arrange By orders the notes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arrange {
    Colour,
    Content,
    Created,
    Modified,
    Location,
}

impl Arrange {
    pub const ALL: [Arrange; 5] = [Arrange::Colour, Arrange::Content, Arrange::Created, Arrange::Modified, Arrange::Location];

    pub fn title(self) -> &'static str {
        match self {
            Arrange::Colour => "Colour",
            Arrange::Content => "Content",
            Arrange::Created => "Date Created",
            Arrange::Modified => "Date Modified",
            Arrange::Location => "Location",
        }
    }
}

/// Letter case changes of the selection (Edit > Transformations).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Case {
    Upper,
    Lower,
    Capitalised,
}

impl Case {
    pub fn apply(self, text: &str) -> String {
        match self {
            Case::Upper => text.to_uppercase(),
            Case::Lower => text.to_lowercase(),
            Case::Capitalised => {
                let mut out = String::with_capacity(text.len());
                let mut word_start = true;
                for c in text.chars() {
                    if c.is_alphanumeric() {
                        if word_start {
                            out.extend(c.to_uppercase());
                        } else {
                            out.extend(c.to_lowercase());
                        }
                        word_start = false;
                    } else {
                        out.push(c);
                        // an apostrophe stays inside a word
                        word_start = !(c == '\'' || c == '\u{2019}');
                    }
                }
                out
            }
        }
    }
}

/// The attributes of the characters of a note. Size and the typing style belong to the note; the rest can differ from
/// one character to the next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    /// Text drawn as an outline: the letters are ringed in the text colour (Font > Outline).
    pub outline: bool,
    pub baseline: Baseline,
    pub kern: Setting,
    pub ligature: Setting,
    pub size: TextSize,
    pub ink: Ink,
}

impl Default for Style {
    fn default() -> Self {
        Style {
            bold: false,
            italic: false,
            underline: false,
            strike: false,
            outline: false,
            baseline: Baseline::Normal,
            kern: Setting::Default,
            ligature: Setting::Default,
            size: TextSize::Medium,
            ink: Ink::Black,
        }
    }
}

impl Style {
    /// The attributes of `from`, with this style's size: Paste Style keeps the size of the note it lands in.
    pub fn take_attributes(self, from: Style) -> Style {
        Style { size: self.size, ..from }
    }
}

/// Applies `f` to one style and reports whether it changed.
fn apply(s: &mut Style, f: &impl Fn(&mut Style)) -> bool {
    let before = *s;
    f(s);
    *s != before
}

/// The hit areas of a note.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Disclosure,
    Title,
    Body,
    Grip,
}

/// Caret movements that need no line layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Move {
    Left,
    Right,
    LineStart,
    LineEnd,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Note {
    pub id: u64,
    pub colour: Colour,
    /// A custom colour (Colour > Custom Colour...), which replaces `colour` while it is set.
    pub rgb: Option<Rgb>,
    /// Frame in points, board space.
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub collapsed: bool,
    /// The note's text size, and the style new text takes where no text precedes the caret.
    pub style: Style,
    pub text: String,
    /// The style of each character of `text`, in order. Its length is the character count of `text`.
    pub styles: Vec<Style>,
    /// The selection in the text, as byte offsets: `anchor` stays, `caret` moves. Equal when nothing is selected.
    pub anchor: usize,
    pub caret: usize,
    /// Whether the note shows the window behind it while it is not the focused note (Window > Translucent).
    pub translucent: bool,
    /// The size before Zoom, while the note is zoomed.
    pub restore: Option<(f32, f32)>,
    /// Unix seconds.
    pub created: u64,
    pub modified: u64,
}

/// Keeps `styles` in step with an edit that turns `old` into `new`. The characters the edit inserts take the style of
/// the character before them (the note's style at the start), and every other character keeps its style.
fn splice_styles(styles: &mut Vec<Style>, base: Style, old: &str, new: &str) {
    let mut start = old.bytes().zip(new.bytes()).take_while(|(a, b)| a == b).count();
    while !old.is_char_boundary(start) || !new.is_char_boundary(start) {
        start -= 1;
    }
    let room = old.len().min(new.len()) - start;
    let mut tail = old.bytes().rev().zip(new.bytes().rev()).take(room).take_while(|(a, b)| a == b).count();
    while !old.is_char_boundary(old.len() - tail) || !new.is_char_boundary(new.len() - tail) {
        tail -= 1;
    }
    let at = old[..start].chars().count();
    let removed = old[start..old.len() - tail].chars().count();
    let inserted = new[start..new.len() - tail].chars().count();
    let fill = if at > 0 { styles.get(at - 1).copied().unwrap_or(base) } else { base };
    let from = at.min(styles.len());
    let to = (at + removed).min(styles.len()).max(from);
    styles.splice(from..to, std::iter::repeat(fill).take(inserted));
    // one style per character, even when the text and the styles disagreed before this edit
    styles.resize(new.chars().count(), base);
}

impl Note {
    pub fn new(id: u64, x: f32, y: f32, now: u64) -> Note {
        Note {
            id,
            colour: Colour::Yellow,
            rgb: None,
            x,
            y,
            w: DEFAULT_W,
            h: DEFAULT_H,
            collapsed: false,
            style: Style::default(),
            text: String::new(),
            styles: Vec::new(),
            anchor: 0,
            caret: 0,
            translucent: true,
            restore: None,
            created: now,
            modified: now,
        }
    }

    /// The height the note occupies on the board: its title bar when collapsed.
    pub fn frame_h(&self) -> f32 {
        if self.collapsed {
            TITLE_H
        } else {
            self.h
        }
    }

    /// The first non-blank line, which names the note in the title bar; empty for a blank note.
    pub fn title(&self) -> String {
        self.text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("").chars().take(60).collect()
    }

    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.frame_h()
    }

    /// Which part of the note a board point (points) lands on.
    pub fn part_at(&self, px: f32, py: f32) -> Option<Part> {
        if !self.contains(px, py) {
            return None;
        }
        if py < self.y + TITLE_H {
            return Some(if px < self.x + DISCLOSURE_PT { Part::Disclosure } else { Part::Title });
        }
        if !self.collapsed && px >= self.x + self.w - GRIP_PT && py >= self.y + self.h - GRIP_PT {
            return Some(Part::Grip);
        }
        Some(Part::Body)
    }

    /// Set the frame, keeping it at least the minimum size.
    pub fn resize(&mut self, w: f32, h: f32) {
        self.w = w.max(MIN_W);
        self.h = h.max(MIN_H);
    }

    /// Takes one of the six colours and drops any custom colour.
    pub fn set_preset(&mut self, colour: Colour) {
        self.colour = colour;
        self.rgb = None;
    }

    /// The character index of the byte offset `at`.
    pub fn char_index(&self, at: usize) -> usize {
        self.text.char_indices().take_while(|(i, _)| *i < at).count()
    }

    /// The character indices covering the byte range `r`.
    fn char_range(&self, r: Range<usize>) -> Range<usize> {
        let start = self.char_index(r.start);
        start..start + self.text.get(r).map_or(0, |s| s.chars().count())
    }

    /// The style of the character at index `i`, or the note's style past the end of the text.
    pub fn style_at_index(&self, i: usize) -> Style {
        self.styles.get(i).copied().unwrap_or(self.style)
    }

    /// The style a menu shows and a format change starts from: the first selected character, else the character before
    /// the caret, else the first character.
    pub fn current_style(&self) -> Style {
        if self.has_selection() {
            return self.style_at_index(self.char_index(self.selection().start));
        }
        let at = self.char_index(self.caret);
        if at > 0 {
            self.style_at_index(at - 1)
        } else {
            self.style_at_index(0)
        }
    }

    /// The runs of the byte range `r`: neighbouring characters with equal styles, as byte ranges.
    pub fn style_runs(&self, r: Range<usize>) -> Vec<(Range<usize>, Style)> {
        let mut out: Vec<(Range<usize>, Style)> = Vec::new();
        let Some(slice) = self.text.get(r.clone()) else { return out };
        let mut i = self.char_index(r.start);
        for (off, c) in slice.char_indices() {
            let style = self.style_at_index(i);
            let start = r.start + off;
            let end = start + c.len_utf8();
            match out.last_mut() {
                Some((range, last)) if *last == style && range.end == start => range.end = end,
                _ => out.push((start..end, style)),
            }
            i += 1;
        }
        out
    }

    /// Runs `f` on a text buffer holding the body and the selection, then takes the result back. The styles follow the
    /// edit.
    fn with_buffer<R>(&mut self, f: impl FnOnce(&mut TextBuffer) -> R) -> R {
        let mut b = TextBuffer::from_text(&self.text);
        b.set_selection(self.anchor, self.caret);
        let r = f(&mut b);
        if b.text() != self.text {
            let text = b.text().to_string();
            splice_styles(&mut self.styles, self.style, &self.text, &text);
            self.text = text;
        }
        self.anchor = b.anchor();
        self.caret = b.caret();
        r
    }

    /// Replaces the whole body; the caret goes to the end. Characters that are still there keep their styles.
    pub fn set_text(&mut self, text: &str) {
        let old = std::mem::replace(&mut self.text, text.to_string());
        splice_styles(&mut self.styles, self.style, &old, &self.text);
        self.anchor = self.text.len();
        self.caret = self.text.len();
    }

    /// Replaces the body with rich text runs, each with its style. The runs take the note's size.
    pub fn set_runs(&mut self, runs: &[(String, Style)]) {
        let size = self.style.size;
        self.text = runs.iter().map(|(t, _)| t.as_str()).collect();
        self.styles = runs.iter().flat_map(|(t, s)| t.chars().map(move |_| Style { size, ..*s })).collect();
        self.anchor = self.text.len();
        self.caret = self.text.len();
    }

    /// The selected byte range (empty at the caret when nothing is selected).
    pub fn selection(&self) -> Range<usize> {
        self.anchor.min(self.caret)..self.anchor.max(self.caret)
    }

    pub fn has_selection(&self) -> bool {
        self.anchor != self.caret
    }

    pub fn selected_text(&self) -> &str {
        self.text.get(self.selection()).unwrap_or("")
    }

    /// Applies `f` to the styles of the selection, or to the whole note (and its style) when nothing is selected.
    /// True when a style changed.
    pub fn restyle(&mut self, f: impl Fn(&mut Style)) -> bool {
        let mut changed = false;
        if self.has_selection() {
            let r = self.char_range(self.selection());
            if let Some(part) = self.styles.get_mut(r) {
                for s in part.iter_mut() {
                    changed |= apply(s, &f);
                }
            }
        } else {
            changed |= self.restyle_whole(f);
        }
        changed
    }

    /// Applies `f` to every character and to the note's style, whatever the selection. True when a style changed.
    pub fn restyle_whole(&mut self, f: impl Fn(&mut Style)) -> bool {
        let mut changed = false;
        for s in self.styles.iter_mut() {
            changed |= apply(s, &f);
        }
        changed |= apply(&mut self.style, &f);
        changed
    }

    /// Typed text replaces the selection, or goes in at the caret.
    pub fn type_text(&mut self, text: &str, now: u64) {
        if self.with_buffer(|b| b.insert_text(text).is_some()) {
            self.modified = now;
        }
    }

    pub fn newline(&mut self, now: u64) {
        self.type_text("\n", now);
    }

    /// Deletes the selection, or the character before the caret. False when there was nothing to delete.
    pub fn backspace(&mut self, now: u64) -> bool {
        let removed = self.with_buffer(|b| b.delete_backward().is_some());
        if removed {
            self.modified = now;
        }
        removed
    }

    /// Deletes the selection, or the character after the caret. False when there was nothing to delete.
    pub fn delete_forward(&mut self, now: u64) -> bool {
        let removed = self.with_buffer(|b| b.delete_forward().is_some());
        if removed {
            self.modified = now;
        }
        removed
    }

    /// Replaces the bytes `range` with `with` and leaves the caret after it. Used by spelling corrections.
    pub fn replace_range(&mut self, range: Range<usize>, with: &str, now: u64) {
        let end = range.start + with.len();
        self.with_buffer(|b| {
            b.replace_range(range, with);
            b.set_selection(end, end);
        });
        self.modified = now;
    }

    /// Removes the selection and returns it (Cut); `None` when nothing is selected.
    pub fn cut(&mut self, now: u64) -> Option<String> {
        if !self.has_selection() {
            return None;
        }
        let text = self.selected_text().to_string();
        self.with_buffer(|b| b.insert_text(""));
        self.modified = now;
        Some(text)
    }

    pub fn select_all(&mut self) {
        self.with_buffer(|b| b.select_all());
    }

    /// Puts the caret at byte `pos`; with `extend` the anchor stays and the selection grows.
    pub fn set_caret(&mut self, pos: usize, extend: bool) {
        let anchor = if extend { self.anchor } else { pos };
        self.with_buffer(|b| b.set_selection(anchor, pos));
    }

    /// Selects the word around byte `pos` (a double click).
    pub fn select_word_at(&mut self, pos: usize) {
        self.with_buffer(|b| b.select_word_at(pos));
    }

    /// Moves the caret one character or to the line or text edge.
    pub fn move_caret(&mut self, dir: Move, extend: bool) {
        self.with_buffer(|b| match dir {
            Move::Left => b.move_char(false, extend),
            Move::Right => b.move_char(true, extend),
            Move::LineStart => b.move_line(false, extend),
            Move::LineEnd => b.move_line(true, extend),
        });
    }

    /// Changes the case of the selection and keeps it selected. False without a selection or when nothing changed.
    pub fn transform(&mut self, case: Case, now: u64) -> bool {
        if !self.has_selection() {
            return false;
        }
        let range = self.selection();
        let new = case.apply(self.selected_text());
        if new == self.selected_text() {
            return false;
        }
        let end = range.start + new.len();
        self.with_buffer(|b| {
            b.replace_range(range.clone(), &new);
            b.set_selection(range.start, end);
        });
        self.modified = now;
        true
    }

    /// The link under byte `pos`, as a URL to open.
    pub fn link_at(&self, pos: usize) -> Option<String> {
        links(&self.text).into_iter().find(|r| r.start <= pos && pos <= r.end).map(|r| {
            let url = &self.text[r];
            if url.to_ascii_lowercase().starts_with("www.") {
                format!("https://{url}")
            } else {
                url.to_string()
            }
        })
    }

    /// Zoom: a note jumps to the large size, and back to its old size on the next Zoom.
    pub fn toggle_zoom(&mut self) {
        match self.restore.take() {
            Some((w, h)) => self.resize(w, h),
            None => {
                self.restore = Some((self.w, self.h));
                self.resize(ZOOM_W, ZOOM_H);
                self.collapsed = false;
            }
        }
    }
}

/// The rich text runs of an RTF document: its text with the bold, italic, underline, strikethrough and text colour of
/// each part. Other formatting is left at the note's defaults.
pub fn runs_from_rtf(rtf: &str) -> Vec<(String, Style)> {
    let attributed = AttributedString::from_rtf(rtf);
    let runs: Vec<(String, Style)> = attributed
        .runs()
        .map(|(text, attrs)| {
            let style = Style {
                bold: attrs.bold,
                italic: attrs.italic,
                underline: attrs.underline,
                strike: attrs.strikethrough,
                ink: ink_of_colour(attrs.color),
                ..Style::default()
            };
            (text.to_string(), style)
        })
        .collect();
    rtf::with_list_markers(runs, &attributed.list_markers())
}

/// The Font > Text Colour an RTF colour becomes: the nearest of the five inks. Yellow and the default label colours
/// are black, since yellow text cannot be read on a sticky note.
fn ink_of_colour(colour: Option<ColorToken>) -> Ink {
    match colour {
        Some(ColorToken::Red | ColorToken::Orange) => Ink::Red,
        Some(ColorToken::Blue) => Ink::Blue,
        Some(ColorToken::Green) => Ink::Green,
        _ => Ink::Black,
    }
}

/// The byte ranges of the web addresses in `text`: words that start with `http://`, `https://` or `www.`, without
/// the punctuation that ends a sentence.
pub fn links(text: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut pos = 0;
    for word in text.split_inclusive(char::is_whitespace) {
        let core = word.trim_end().trim_end_matches(['.', ',', ';', ':', '!', '?', ')', '"', '\'']);
        let lower = core.to_ascii_lowercase();
        let is_link = (lower.starts_with("http://") && lower.len() > 7) || (lower.starts_with("https://") && lower.len() > 8) || (lower.starts_with("www.") && lower.len() > 4);
        if is_link {
            out.push(pos..pos + core.len());
        }
        pos += word.len();
    }
    out
}

/// Whether `text` is a web address that Add Link can insert: the whole of it is one link.
pub fn is_link(text: &str) -> bool {
    let t = text.trim();
    !t.is_empty() && links(t) == vec![0..t.len()]
}

/// A place where the find text occurs: the note and the byte range in its body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Match {
    pub note: u64,
    pub start: usize,
    pub end: usize,
}

/// All notes, in stacking order: the last one is in front.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Board {
    pub notes: Vec<Note>,
    pub next_id: u64,
    /// The note that takes typing and draws active.
    pub focus: Option<u64>,
    /// The colour, custom colour and text style that new notes start with (Use as Default).
    pub default_colour: Colour,
    pub default_rgb: Option<Rgb>,
    pub default_style: Style,
}

impl Board {
    pub fn find(&self, id: u64) -> Option<&Note> {
        self.notes.iter().find(|n| n.id == id)
    }

    pub fn find_mut(&mut self, id: u64) -> Option<&mut Note> {
        self.notes.iter_mut().find(|n| n.id == id)
    }

    pub fn focused(&self) -> Option<&Note> {
        self.focus.and_then(|id| self.find(id))
    }

    pub fn focused_mut(&mut self) -> Option<&mut Note> {
        let id = self.focus?;
        self.find_mut(id)
    }

    /// Adds an empty note at `(x, y)`, in front, and focuses it. It takes the default colour and style.
    pub fn new_note(&mut self, x: f32, y: f32, now: u64) -> u64 {
        self.next_id += 1;
        let id = self.next_id;
        let mut note = Note::new(id, x, y, now);
        note.colour = self.default_colour;
        note.rgb = self.default_rgb;
        note.style = self.default_style;
        self.notes.push(note);
        self.focus = Some(id);
        id
    }

    /// Makes the focused note's colour and the style at its caret the ones new notes start with. False without a
    /// focused note.
    pub fn use_as_default(&mut self) -> bool {
        let Some((colour, rgb, style)) = self.focused().map(|n| (n.colour, n.rgb, n.current_style())) else { return false };
        self.default_colour = colour;
        self.default_rgb = rgb;
        self.default_style = style;
        true
    }

    /// Copies note `id` into a new note in front, offset from the original, and focuses the copy. `None` when
    /// there is no note `id`.
    pub fn duplicate(&mut self, id: u64, now: u64) -> Option<u64> {
        let original = self.find(id)?.clone();
        self.next_id += 1;
        let copy = Note {
            id: self.next_id,
            x: original.x + DUPLICATE_OFFSET_PT,
            y: original.y + DUPLICATE_OFFSET_PT,
            created: now,
            modified: now,
            ..original
        };
        let new_id = copy.id;
        self.notes.push(copy);
        self.focus = Some(new_id);
        Some(new_id)
    }

    /// Adds a note holding `text`, placed at `(x, y)`.
    pub fn add_text(&mut self, text: &str, x: f32, y: f32, now: u64) -> u64 {
        let id = self.new_note(x, y, now);
        if let Some(n) = self.find_mut(id) {
            n.set_text(text);
        }
        id
    }

    /// Adds a note holding rich text runs, placed at `(x, y)`.
    pub fn add_runs(&mut self, runs: &[(String, Style)], x: f32, y: f32, now: u64) -> u64 {
        let id = self.new_note(x, y, now);
        if let Some(n) = self.find_mut(id) {
            n.set_runs(runs);
        }
        id
    }

    pub fn remove(&mut self, id: u64) -> bool {
        let before = self.notes.len();
        self.notes.retain(|n| n.id != id);
        if self.focus == Some(id) {
            self.focus = self.notes.last().map(|n| n.id);
        }
        self.notes.len() != before
    }

    /// The front-most note under the point, if any.
    pub fn hit(&self, px: f32, py: f32) -> Option<u64> {
        self.notes.iter().rev().find(|n| n.contains(px, py)).map(|n| n.id)
    }

    /// Moves the note to the front and focuses it.
    pub fn raise(&mut self, id: u64) {
        if let Some(pos) = self.notes.iter().position(|n| n.id == id) {
            let n = self.notes.remove(pos);
            self.notes.push(n);
            self.focus = Some(id);
        }
    }

    /// Collapses or expands every note (Window > Collapse All, Expand All). False when none changed.
    pub fn set_all_collapsed(&mut self, collapsed: bool) -> bool {
        let mut changed = false;
        for n in &mut self.notes {
            if n.collapsed != collapsed {
                n.collapsed = collapsed;
                changed = true;
            }
        }
        changed
    }

    /// Orders the notes by `by` and lays them out in rows that fit `width` points (Window > Arrange By). The stacking
    /// order follows the sort. False when there are no notes.
    pub fn arrange(&mut self, by: Arrange, width: f32) -> bool {
        if self.notes.is_empty() {
            return false;
        }
        match by {
            Arrange::Colour => self.notes.sort_by_key(|n| (n.rgb.is_some(), n.colour.index(), n.rgb)),
            Arrange::Content => self.notes.sort_by_key(|n| n.title().to_lowercase()),
            Arrange::Created => self.notes.sort_by_key(|n| n.created),
            Arrange::Modified => self.notes.sort_by_key(|n| n.modified),
            // reading order: rows of notes from the top, left to right inside a row
            Arrange::Location => self.notes.sort_by(|a, b| {
                let row = |n: &Note| (n.y / LOCATION_BAND_PT).floor() as i64;
                row(a).cmp(&row(b)).then(a.x.total_cmp(&b.x)).then(a.y.total_cmp(&b.y))
            }),
        }
        let (mut x, mut y, mut row_h) = (ARRANGE_MARGIN_PT, ARRANGE_MARGIN_PT, 0.0f32);
        for n in &mut self.notes {
            if x > ARRANGE_MARGIN_PT && x + n.w > width - ARRANGE_MARGIN_PT {
                x = ARRANGE_MARGIN_PT;
                y += row_h + ARRANGE_MARGIN_PT;
                row_h = 0.0;
            }
            n.x = x;
            n.y = y;
            x += n.w + ARRANGE_MARGIN_PT;
            row_h = row_h.max(n.frame_h());
        }
        true
    }

    /// Plain text of one note, or of all of them, separated by `EXPORT_SEPARATOR`.
    pub fn export_text(&self, id: Option<u64>) -> String {
        match id {
            Some(id) => self.find(id).map(|n| n.text.clone()).unwrap_or_default(),
            None => self.notes.iter().map(|n| n.text.as_str()).collect::<Vec<_>>().join(EXPORT_SEPARATOR),
        }
    }

    /// Imports a text file as one note, or as several when it holds `EXPORT_SEPARATOR` lines. Returns the new ids.
    pub fn import_text(&mut self, text: &str, now: u64) -> Vec<u64> {
        let parts: Vec<&str> = if text.contains("\n---\n") {
            text.split("\n---\n").map(|p| p.trim_matches('\n')).collect()
        } else {
            vec![text]
        };
        let mut ids = Vec::new();
        for (i, part) in parts.iter().filter(|p| !p.trim().is_empty()).enumerate() {
            let offset = (i as f32) * 24.0;
            ids.push(self.add_text(part, 40.0 + offset, 60.0 + offset, now));
        }
        ids
    }

    /// Every place `query` occurs, note by note in stacking order. With `ignore_case` the comparison ignores ASCII case,
    /// and with `whole_word` a match inside a longer word does not count.
    pub fn matches(&self, query: &str, ignore_case: bool, whole_word: bool) -> Vec<Match> {
        let mut out = Vec::new();
        if query.is_empty() {
            return out;
        }
        for note in &self.notes {
            let (hay, needle) = fold(&note.text, query, ignore_case);
            let mut pos = 0;
            while let Some(i) = hay[pos..].find(&needle) {
                let start = pos + i;
                let end = start + needle.len();
                pos = end;
                if !whole_word || words::is_whole_word(&note.text, start, end) {
                    out.push(Match { note: note.id, start, end });
                }
            }
        }
        out
    }

    /// The match after (or before) the focused note's selection, wrapping round the board.
    pub fn find_step(&self, query: &str, forward: bool, ignore_case: bool, whole_word: bool) -> Option<Match> {
        let all = self.matches(query, ignore_case, whole_word);
        let index_of = |id: u64| self.notes.iter().position(|n| n.id == id).unwrap_or(0);
        let here = match self.focused() {
            Some(n) => {
                let sel = n.selection();
                (index_of(n.id), if forward { sel.end } else { sel.start })
            }
            None => (if forward { 0 } else { self.notes.len() }, 0),
        };
        let found = if forward {
            all.iter().find(|m| (index_of(m.note), m.start) >= here)
        } else {
            all.iter().rev().find(|m| (index_of(m.note), m.start) < here)
        };
        found.or(if forward { all.first() } else { all.last() }).cloned()
    }

    /// Brings the match's note to the front and selects the match in it.
    pub fn select_match(&mut self, m: &Match) {
        self.raise(m.note);
        if let Some(n) = self.find_mut(m.note) {
            n.anchor = m.start;
            n.caret = m.end;
        }
    }

    /// Replaces the selection of the focused note with `with` when the selection is a match of `query` (ignoring
    /// ASCII case when `ignore_case` is set, and only a whole word when `whole_word` is set). False when it is not, so
    /// Replace on a stray selection changes nothing.
    pub fn replace_selected(&mut self, query: &str, with: &str, ignore_case: bool, whole_word: bool, now: u64) -> bool {
        let Some(note) = self.focused_mut() else { return false };
        let sel = note.selection();
        let selected = note.selected_text();
        let same = if ignore_case { selected.eq_ignore_ascii_case(query) } else { selected == query };
        let whole = !whole_word || words::is_whole_word(&note.text, sel.start, sel.end);
        if query.is_empty() || !note.has_selection() || !same || !whole {
            return false;
        }
        note.type_text(with, now);
        true
    }

    /// Replaces every match of `query` with `with` (Replace All): in every note, or only in the note `only`. With
    /// `whole_word` a match inside a longer word is left alone. Returns how many were replaced.
    pub fn replace_all(&mut self, query: &str, with: &str, ignore_case: bool, whole_word: bool, only: Option<u64>, now: u64) -> usize {
        if query.is_empty() {
            return 0;
        }
        let mut total = 0;
        for note in &mut self.notes {
            if only.is_some_and(|id| note.id != id) {
                continue;
            }
            // lower-casing ASCII keeps every byte offset, so the positions in `hay` index `note.text` too
            let (hay, needle) = fold(&note.text, query, ignore_case);
            let mut out = String::with_capacity(note.text.len());
            let (mut pos, mut count) = (0, 0);
            while let Some(i) = hay[pos..].find(&needle) {
                let start = pos + i;
                let end = start + needle.len();
                let counts = !whole_word || words::is_whole_word(&note.text, start, end);
                out.push_str(&note.text[pos..start]);
                out.push_str(if counts { with } else { &note.text[start..end] });
                pos = end;
                count += usize::from(counts);
            }
            if count == 0 {
                continue;
            }
            out.push_str(&note.text[pos..]);
            note.set_text(&out);
            note.modified = now;
            total += count;
        }
        total
    }
}

/// The text and the query as a search compares them. Ignoring case lower-cases ASCII only, which keeps every byte
/// offset, so an offset found in the folded text is also an offset in the note's own text.
fn fold(text: &str, query: &str, ignore_case: bool) -> (String, String) {
    if ignore_case {
        (text.to_ascii_lowercase(), query.to_ascii_lowercase())
    } else {
        (text.to_string(), query.to_string())
    }
}

fn style_fields(style: &Style) -> Vec<(String, Json)> {
    vec![
        ("bold".into(), Json::Bool(style.bold)),
        ("italic".into(), Json::Bool(style.italic)),
        ("underline".into(), Json::Bool(style.underline)),
        ("strike".into(), Json::Bool(style.strike)),
        ("outline".into(), Json::Bool(style.outline)),
        ("baseline".into(), Json::Str(style.baseline.name().into())),
        ("kern".into(), Json::Str(style.kern.name().into())),
        ("ligature".into(), Json::Str(style.ligature.name().into())),
        ("size".into(), Json::Str(style.size.name().into())),
        ("ink".into(), Json::Str(style.ink.name().into())),
    ]
}

fn flag(item: &Json, key: &str, default: bool) -> bool {
    match item.get(key) {
        Some(Json::Bool(b)) => *b,
        _ => default,
    }
}

fn style_from(item: &Json) -> Style {
    Style {
        bold: flag(item, "bold", false),
        italic: flag(item, "italic", false),
        underline: flag(item, "underline", false),
        strike: flag(item, "strike", false),
        outline: flag(item, "outline", false),
        baseline: Baseline::from_name(&item.str_field("baseline").unwrap_or_default()),
        kern: Setting::from_name(&item.str_field("kern").unwrap_or_default()),
        ligature: Setting::from_name(&item.str_field("ligature").unwrap_or_default()),
        size: TextSize::from_name(&item.str_field("size").unwrap_or_default()),
        ink: Ink::from_name(&item.str_field("ink").unwrap_or_default()),
    }
}

/// The note's styles as runs: each run has the number of characters it covers (`n`) and the style fields.
fn runs_json(note: &Note) -> Json {
    let runs = note
        .style_runs(0..note.text.len())
        .into_iter()
        .map(|(range, style)| {
            let mut fields: Vec<(String, Json)> = vec![("n".into(), Json::Num(note.text[range].chars().count() as f64))];
            fields.extend(style_fields(&style));
            Json::Obj(fields)
        })
        .collect();
    Json::Arr(runs)
}

/// Encodes the board as the app's JSON file.
pub fn encode(board: &Board) -> String {
    let notes = board
        .notes
        .iter()
        .map(|n| {
            let mut fields: Vec<(String, Json)> = vec![
                ("id".into(), Json::Num(n.id as f64)),
                ("colour".into(), Json::Str(n.colour.name().into())),
                ("x".into(), Json::Num(n.x as f64)),
                ("y".into(), Json::Num(n.y as f64)),
                ("w".into(), Json::Num(n.w as f64)),
                ("h".into(), Json::Num(n.h as f64)),
                ("collapsed".into(), Json::Bool(n.collapsed)),
                ("translucent".into(), Json::Bool(n.translucent)),
            ];
            if let Some(rgb) = n.rgb {
                fields.push(("rgb".into(), Json::Str(swatch::hex(rgb))));
            }
            fields.extend(style_fields(&n.style));
            fields.push(("text".into(), Json::Str(n.text.clone())));
            fields.push(("runs".into(), runs_json(n)));
            if let Some((w, h)) = n.restore {
                fields.push(("restore_w".into(), Json::Num(w as f64)));
                fields.push(("restore_h".into(), Json::Num(h as f64)));
            }
            fields.push(("created".into(), Json::Num(n.created as f64)));
            fields.push(("modified".into(), Json::Num(n.modified as f64)));
            Json::Obj(fields)
        })
        .collect();
    let mut default: Vec<(String, Json)> = vec![("colour".into(), Json::Str(board.default_colour.name().into()))];
    if let Some(rgb) = board.default_rgb {
        default.push(("rgb".into(), Json::Str(swatch::hex(rgb))));
    }
    default.extend(style_fields(&board.default_style));
    Json::Obj(vec![
        ("version".into(), Json::Num(1.0)),
        ("next_id".into(), Json::Num(board.next_id as f64)),
        ("focus".into(), board.focus.map_or(Json::Null, |id| Json::Num(id as f64))),
        ("default".into(), Json::Obj(default)),
        ("notes".into(), Json::Arr(notes)),
    ])
    .encode()
}

/// Decodes the app's JSON file. Unknown or malformed notes are skipped rather than failing the whole board. A note from
/// before the per-character styles gets its one style everywhere.
pub fn decode(text: &str) -> Result<Board, String> {
    let root = json::parse(text)?;
    let mut board = Board::default();
    let notes = root.get("notes").and_then(Json::as_array).ok_or("no notes array")?;
    for item in notes {
        let Some(id) = item.get("id").and_then(Json::number) else { continue };
        let num = |k: &str, d: f32| item.get(k).and_then(Json::number).map_or(d, |v| v as f32);
        let mut note = Note::new(id as u64, num("x", 40.0), num("y", 60.0), 0);
        note.colour = item.get("colour").and_then(Json::as_str).and_then(Colour::from_name).unwrap_or(Colour::Yellow);
        note.rgb = item.get("rgb").and_then(Json::as_str).and_then(swatch::parse);
        note.resize(num("w", DEFAULT_W), num("h", DEFAULT_H));
        note.collapsed = flag(item, "collapsed", false);
        note.translucent = flag(item, "translucent", true);
        note.style = style_from(item);
        note.set_text(&item.str_field("text").unwrap_or_default());
        if let Some(runs) = item.get("runs").and_then(Json::as_array) {
            let counts: Vec<(usize, Style)> = runs.iter().map(|run| (run.get("n").and_then(Json::number).unwrap_or(0.0).max(0.0) as usize, style_from(run))).collect();
            // the runs are kept only when they cover the text exactly; otherwise the note keeps its one style
            if counts.iter().map(|(n, _)| n).sum::<usize>() == note.text.chars().count() {
                note.styles = counts.iter().flat_map(|(n, s)| std::iter::repeat(*s).take(*n)).collect();
            }
        }
        if let (Some(w), Some(h)) = (item.get("restore_w").and_then(Json::number), item.get("restore_h").and_then(Json::number)) {
            note.restore = Some((w as f32, h as f32));
        }
        note.created = item.get("created").and_then(Json::number).unwrap_or(0.0) as u64;
        note.modified = item.get("modified").and_then(Json::number).unwrap_or(0.0) as u64;
        board.notes.push(note);
    }
    if let Some(default) = root.get("default") {
        board.default_colour = default.get("colour").and_then(Json::as_str).and_then(Colour::from_name).unwrap_or_default();
        board.default_rgb = default.get("rgb").and_then(Json::as_str).and_then(swatch::parse);
        board.default_style = style_from(default);
    }
    board.next_id = root.get("next_id").and_then(Json::number).map_or(0, |v| v as u64);
    board.next_id = board.next_id.max(board.notes.iter().map(|n| n.id).max().unwrap_or(0));
    board.focus = root.get("focus").and_then(Json::number).map(|v| v as u64).filter(|id| board.find(*id).is_some());
    Ok(board)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bold() -> Style {
        Style { bold: true, ..Style::default() }
    }

    #[test]
    fn colours_wrap_and_round_trip_by_name() {
        assert_eq!(Colour::from_index(6), Colour::Yellow);
        assert_eq!(Colour::Purple.next(), Colour::Yellow);
        for c in Colour::ALL {
            assert_eq!(Colour::from_name(c.name()), Some(c));
        }
        assert_eq!(Colour::from_name("Lime"), None);
    }

    #[test]
    fn collapsed_note_is_only_its_title_bar() {
        let mut n = Note::new(1, 10.0, 10.0, 0);
        assert!(n.contains(20.0, 150.0));
        n.collapsed = true;
        assert!(!n.contains(20.0, 150.0));
        assert!(n.contains(20.0, 20.0));
    }

    #[test]
    fn parts_of_an_expanded_and_a_collapsed_note() {
        let mut n = Note::new(1, 100.0, 100.0, 0);
        assert_eq!(n.part_at(101.0, 101.0), Some(Part::Disclosure));
        assert_eq!(n.part_at(150.0, 101.0), Some(Part::Title));
        assert_eq!(n.part_at(150.0, 150.0), Some(Part::Body));
        assert_eq!(n.part_at(100.0 + n.w - 1.0, 100.0 + n.h - 1.0), Some(Part::Grip));
        assert_eq!(n.part_at(50.0, 50.0), None);
        n.collapsed = true;
        assert_eq!(n.part_at(150.0, 150.0), None);
        assert_eq!(n.part_at(150.0, 101.0), Some(Part::Title));
    }

    #[test]
    fn resize_keeps_the_minimum_size() {
        let mut n = Note::new(1, 0.0, 0.0, 0);
        n.resize(10.0, 10.0);
        assert_eq!((n.w, n.h), (MIN_W, MIN_H));
    }

    #[test]
    fn typing_and_backspace_update_the_text_and_time() {
        let mut n = Note::new(1, 0.0, 0.0, 5);
        n.type_text("héllo", 9);
        n.newline(10);
        n.type_text("x", 11);
        assert_eq!(n.text, "héllo\nx");
        assert!(n.backspace(12));
        assert!(n.backspace(13));
        assert_eq!(n.text, "héllo", "two backspaces remove the x and the newline");
        assert_eq!(n.modified, 13);
        assert_eq!(n.created, 5);
        assert_eq!(n.styles.len(), n.text.chars().count(), "one style per character");
        let mut empty = Note::new(2, 0.0, 0.0, 0);
        assert!(!empty.backspace(4));
        assert_eq!(empty.modified, 0);
    }

    #[test]
    fn typing_goes_in_at_the_caret_and_replaces_the_selection() {
        let mut n = Note::new(1, 0.0, 0.0, 0);
        n.set_text("hello world");
        n.set_caret(5, false);
        n.type_text(",", 1);
        assert_eq!(n.text, "hello, world");
        assert_eq!(n.caret, 6);
        n.set_caret(7, false);
        n.set_caret(12, true);
        assert_eq!(n.selected_text(), "world");
        n.type_text("there", 2);
        assert_eq!(n.text, "hello, there");
        assert!(!n.has_selection());
    }

    #[test]
    fn backspace_and_delete_forward_act_on_the_caret_or_the_selection() {
        let mut n = Note::new(1, 0.0, 0.0, 0);
        n.set_text("abcd");
        n.set_caret(2, false);
        assert!(n.backspace(1));
        assert_eq!(n.text, "acd");
        assert!(n.delete_forward(2));
        assert_eq!(n.text, "ad");
        n.select_all();
        assert!(n.backspace(3));
        assert_eq!(n.text, "");
        assert!(!n.delete_forward(4));
    }

    #[test]
    fn cut_removes_and_returns_the_selection() {
        let mut n = Note::new(1, 0.0, 0.0, 0);
        n.set_text("one two");
        assert_eq!(n.cut(1), None);
        n.set_caret(0, false);
        n.set_caret(3, true);
        assert_eq!(n.cut(2).as_deref(), Some("one"));
        assert_eq!(n.text, " two");
        assert_eq!(n.modified, 2);
    }

    #[test]
    fn the_caret_moves_by_character_and_line() {
        let mut n = Note::new(1, 0.0, 0.0, 0);
        n.set_text("ab\ncd");
        n.move_caret(Move::Left, false);
        assert_eq!(n.caret, 4);
        n.move_caret(Move::LineStart, false);
        assert_eq!(n.caret, 3);
        n.move_caret(Move::LineEnd, true);
        assert_eq!(n.selection(), 3..5);
        n.move_caret(Move::Right, false);
        assert_eq!(n.caret, 5, "the end stays the end");
    }

    #[test]
    fn select_word_selects_the_word_under_the_caret() {
        let mut n = Note::new(1, 0.0, 0.0, 0);
        n.set_text("alpha beta");
        n.select_word_at(7);
        assert_eq!(n.selected_text(), "beta");
    }

    #[test]
    fn transformations_change_the_case_of_the_selection_only() {
        let mut n = Note::new(1, 0.0, 0.0, 0);
        n.set_text("one two three");
        assert!(!n.transform(Case::Upper, 1), "nothing selected");
        n.set_caret(4, false);
        n.set_caret(7, true);
        assert!(n.transform(Case::Upper, 2));
        assert_eq!(n.text, "one TWO three");
        assert_eq!(n.selected_text(), "TWO");
        assert!(!n.transform(Case::Upper, 3), "already upper case");
        assert!(n.transform(Case::Lower, 4));
        assert_eq!(n.text, "one two three");
        assert_eq!(Case::Capitalised.apply("hELLO wORLD don't"), "Hello World Don't");
    }

    #[test]
    fn links_are_found_without_trailing_punctuation() {
        let text = "see https://example.com/a, and www.example.org. not http:// here";
        let found: Vec<&str> = links(text).into_iter().map(|r| &text[r]).collect();
        assert_eq!(found, vec!["https://example.com/a", "www.example.org"]);
        let mut n = Note::new(1, 0.0, 0.0, 0);
        n.set_text(text);
        assert_eq!(n.link_at(8).as_deref(), Some("https://example.com/a"));
        assert_eq!(n.link_at(35).as_deref(), Some("https://www.example.org"));
        assert_eq!(n.link_at(1), None);
    }

    #[test]
    fn add_link_takes_only_a_whole_web_address() {
        assert!(is_link(" https://example.com "));
        assert!(is_link("www.example.org"));
        assert!(!is_link("example.com"));
        assert!(!is_link("see https://example.com"));
        assert!(!is_link("   "));
    }

    #[test]
    fn zoom_toggles_between_the_large_size_and_the_old_one() {
        let mut n = Note::new(1, 0.0, 0.0, 0);
        n.collapsed = true;
        n.toggle_zoom();
        assert_eq!((n.w, n.h, n.collapsed), (ZOOM_W, ZOOM_H, false));
        assert_eq!(n.restore, Some((DEFAULT_W, DEFAULT_H)));
        n.toggle_zoom();
        assert_eq!((n.w, n.h, n.restore), (DEFAULT_W, DEFAULT_H, None));
    }

    #[test]
    fn text_sizes_step_and_stop_at_the_ends() {
        assert_eq!(TextSize::Small.bigger(), TextSize::Medium);
        assert_eq!(TextSize::Large.bigger(), TextSize::Large);
        assert_eq!(TextSize::Large.smaller(), TextSize::Medium);
        assert_eq!(TextSize::Small.smaller(), TextSize::Small);
    }

    #[test]
    fn title_is_the_first_non_blank_line() {
        let mut n = Note::new(1, 0.0, 0.0, 0);
        n.text = "\n   \n  Groceries  \nmilk".into();
        assert_eq!(n.title(), "Groceries");
    }

    #[test]
    fn hit_returns_the_front_note_and_raise_focuses_it() {
        let mut b = Board::default();
        let a = b.add_text("a", 0.0, 0.0, 0);
        let c = b.add_text("c", 0.0, 0.0, 0);
        assert_eq!(b.hit(10.0, 10.0), Some(c));
        b.raise(a);
        assert_eq!(b.hit(10.0, 10.0), Some(a));
        assert_eq!(b.focus, Some(a));
    }

    #[test]
    fn removing_the_focused_note_moves_focus_to_the_front_one() {
        let mut b = Board::default();
        let a = b.add_text("a", 0.0, 0.0, 0);
        let c = b.add_text("c", 0.0, 0.0, 0);
        assert!(b.remove(c));
        assert_eq!(b.focus, Some(a));
        assert!(!b.remove(c));
        assert!(b.remove(a));
        assert_eq!(b.focus, None);
    }

    #[test]
    fn duplicate_copies_the_note_offset_and_focuses_the_copy() {
        let mut b = Board::default();
        let id = b.add_text("hello", 40.0, 50.0, 1);
        b.find_mut(id).unwrap().colour = Colour::Teal;
        let copy = b.duplicate(id, 9).unwrap();
        assert_ne!(copy, id);
        assert_eq!(b.focus, Some(copy));
        assert_eq!(b.notes.len(), 2);
        let n = b.find(copy).unwrap();
        assert_eq!((n.x, n.y), (40.0 + DUPLICATE_OFFSET_PT, 50.0 + DUPLICATE_OFFSET_PT));
        assert_eq!((n.text.as_str(), n.colour, n.created, n.modified), ("hello", Colour::Teal, 9, 9));
        assert!(b.duplicate(999, 9).is_none());
        assert_eq!(b.notes.len(), 2);
    }

    #[test]
    fn ids_keep_growing_after_removal() {
        let mut b = Board::default();
        let a = b.new_note(0.0, 0.0, 0);
        b.remove(a);
        assert!(b.new_note(0.0, 0.0, 0) > a);
    }

    #[test]
    fn new_notes_take_the_default_colour_and_style() {
        let mut b = Board::default();
        assert!(!b.use_as_default(), "no focused note");
        let a = b.new_note(0.0, 0.0, 0);
        {
            let n = b.find_mut(a).unwrap();
            n.colour = Colour::Pink;
            n.style = Style { bold: true, size: TextSize::Large, ink: Ink::Blue, ..Style::default() };
        }
        assert!(b.use_as_default());
        let c = b.new_note(0.0, 0.0, 1);
        let n = b.find(c).unwrap();
        assert_eq!(n.colour, Colour::Pink);
        assert_eq!((n.style.bold, n.style.size, n.style.ink), (true, TextSize::Large, Ink::Blue));
    }

    #[test]
    fn use_as_default_takes_the_style_at_the_caret() {
        let mut b = Board::default();
        let a = b.add_text("plain bold", 0.0, 0.0, 0);
        let n = b.find_mut(a).unwrap();
        n.restyle_whole(|s| s.bold = true);
        n.set_caret(0, false);
        n.set_caret(5, true);
        n.restyle(|s| s.bold = false);
        n.set_caret(5, false);
        assert!(b.use_as_default());
        assert!(!b.default_style.bold, "the character before the caret is not bold");
        let n = b.find_mut(a).unwrap();
        n.set_caret(10, false);
        assert!(b.use_as_default());
        assert!(b.default_style.bold, "the character before the caret is bold");
    }

    #[test]
    fn collapse_all_and_expand_all_report_whether_anything_changed() {
        let mut b = Board::default();
        b.new_note(0.0, 0.0, 0);
        b.new_note(0.0, 0.0, 0);
        assert!(!b.set_all_collapsed(false));
        assert!(b.set_all_collapsed(true));
        assert!(b.notes.iter().all(|n| n.collapsed));
        assert!(!b.set_all_collapsed(true));
        assert!(b.set_all_collapsed(false));
    }

    #[test]
    fn arrange_sorts_and_lays_the_notes_out_in_rows() {
        let mut b = Board::default();
        let a = b.add_text("banana", 300.0, 300.0, 5);
        let c = b.add_text("avocado", 300.0, 300.0, 3);
        let d = b.add_text("cherry", 300.0, 300.0, 4);
        b.find_mut(a).unwrap().colour = Colour::Teal;
        b.find_mut(c).unwrap().colour = Colour::Pink;
        b.find_mut(d).unwrap().colour = Colour::Orange;
        assert!(b.arrange(Arrange::Content, 1000.0));
        let order: Vec<u64> = b.notes.iter().map(|n| n.id).collect();
        assert_eq!(order, vec![c, a, d]);
        assert_eq!((b.notes[0].x, b.notes[0].y), (ARRANGE_MARGIN_PT, ARRANGE_MARGIN_PT));
        assert_eq!(b.notes[1].x, ARRANGE_MARGIN_PT * 2.0 + DEFAULT_W);
        assert_eq!(b.notes[1].y, ARRANGE_MARGIN_PT);
        b.arrange(Arrange::Colour, 1000.0);
        let order: Vec<u64> = b.notes.iter().map(|n| n.id).collect();
        assert_eq!(order, vec![d, a, c], "orange, teal, pink");
        b.arrange(Arrange::Created, 1000.0);
        assert_eq!(b.notes.iter().map(|n| n.created).collect::<Vec<_>>(), vec![3, 4, 5]);
        // a narrow board wraps to a second row
        b.arrange(Arrange::Modified, 300.0);
        assert!(b.notes.iter().all(|n| n.x == ARRANGE_MARGIN_PT));
        assert!(b.notes[1].y > b.notes[0].y);
        assert!(!Board::default().arrange(Arrange::Colour, 100.0));
    }

    #[test]
    fn arrange_by_location_reads_rows_top_down_and_left_to_right() {
        let mut b = Board::default();
        let low_left = b.add_text("c", 10.0, 300.0, 0);
        let top_right = b.add_text("b", 400.0, 20.0, 0);
        let top_left = b.add_text("a", 20.0, 30.0, 0);
        assert!(b.arrange(Arrange::Location, 2000.0));
        let order: Vec<u64> = b.notes.iter().map(|n| n.id).collect();
        assert_eq!(order, vec![top_left, top_right, low_left], "the two upper notes share a row band");
        assert_eq!(Arrange::ALL.len(), 5);
        assert_eq!(Arrange::Location.title(), "Location");
    }

    #[test]
    fn arrange_by_colour_puts_custom_colours_after_the_six() {
        let mut b = Board::default();
        let custom = b.add_text("custom", 0.0, 0.0, 0);
        let purple = b.add_text("purple", 0.0, 0.0, 0);
        b.find_mut(custom).unwrap().rgb = Some((1, 2, 3));
        b.find_mut(purple).unwrap().colour = Colour::Purple;
        b.arrange(Arrange::Colour, 2000.0);
        assert_eq!(b.notes.iter().map(|n| n.id).collect::<Vec<_>>(), vec![purple, custom]);
    }

    #[test]
    fn a_custom_colour_replaces_the_preset_until_one_is_chosen_again() {
        let mut n = Note::new(1, 0.0, 0.0, 0);
        n.rgb = Some((200, 100, 50));
        n.set_preset(Colour::Teal);
        assert_eq!((n.colour, n.rgb), (Colour::Teal, None));
    }

    #[test]
    fn the_default_colour_carries_a_custom_colour_to_new_notes() {
        let mut b = Board::default();
        let a = b.new_note(0.0, 0.0, 0);
        b.find_mut(a).unwrap().rgb = Some((10, 20, 30));
        assert!(b.use_as_default());
        let c = b.new_note(0.0, 0.0, 1);
        assert_eq!(b.find(c).unwrap().rgb, Some((10, 20, 30)));
        b.find_mut(a).unwrap().set_preset(Colour::Pink);
        assert!(b.use_as_default());
        let d = b.new_note(0.0, 0.0, 2);
        assert_eq!(b.find(d).unwrap().rgb, None);
    }

    #[test]
    fn replace_selected_only_replaces_a_selection_that_matches() {
        let mut b = Board::default();
        let a = b.add_text("Foo and foo", 0.0, 0.0, 0);
        assert!(!b.replace_selected("foo", "bar", true, false, 1), "nothing is selected");
        b.find_mut(a).unwrap().set_caret(0, false);
        b.find_mut(a).unwrap().set_caret(3, true);
        assert!(!b.replace_selected("and", "bar", true, false, 1), "the selection is another word");
        assert!(b.replace_selected("foo", "bar", true, false, 2), "case is ignored");
        assert_eq!(b.find(a).unwrap().text, "bar and foo");
        assert_eq!(b.find(a).unwrap().modified, 2);
        assert!(!b.replace_selected("", "x", true, false, 3));
    }

    #[test]
    fn replace_selected_respects_case_when_it_is_not_ignored() {
        let mut b = Board::default();
        let a = b.add_text("Foo", 0.0, 0.0, 0);
        b.find_mut(a).unwrap().set_caret(0, false);
        b.find_mut(a).unwrap().set_caret(3, true);
        assert!(!b.replace_selected("foo", "bar", false, false, 1), "Foo is not foo when case counts");
        assert!(b.replace_selected("Foo", "bar", false, false, 2));
        assert_eq!(b.find(a).unwrap().text, "bar");
    }

    #[test]
    fn replace_all_replaces_in_every_note_ignoring_case() {
        let mut b = Board::default();
        let a = b.add_text("Foo bar FOO", 0.0, 0.0, 0);
        let c = b.add_text("none here", 0.0, 0.0, 0);
        let d = b.add_text("foo", 0.0, 0.0, 0);
        assert_eq!(b.replace_all("foo", "x", true, false, None, 9), 3);
        assert_eq!(b.find(a).unwrap().text, "x bar x");
        assert_eq!(b.find(c).unwrap().text, "none here");
        assert_eq!(b.find(c).unwrap().modified, 0, "an untouched note keeps its date");
        assert_eq!((b.find(d).unwrap().text.as_str(), b.find(d).unwrap().modified), ("x", 9));
        assert_eq!(b.replace_all("zzz", "y", true, false, None, 9), 0);
        assert_eq!(b.replace_all("", "y", true, false, None, 9), 0);
        // deleting with an empty replacement, and a replacement that contains the search text
        assert_eq!(b.replace_all("x", "", true, false, None, 10), 3);
        assert_eq!(b.find(a).unwrap().text, " bar ");
        b.add_text("a", 0.0, 0.0, 0);
        assert_eq!(b.replace_all("a", "aa", true, false, None, 11), 2);
    }

    #[test]
    fn replace_all_in_one_note_leaves_the_others_alone() {
        let mut b = Board::default();
        let a = b.add_text("foo one", 0.0, 0.0, 0);
        let c = b.add_text("foo two", 0.0, 0.0, 0);
        assert_eq!(b.replace_all("foo", "bar", true, false, Some(a), 4), 1);
        assert_eq!(b.find(a).unwrap().text, "bar one");
        assert_eq!(b.find(c).unwrap().text, "foo two", "the other note keeps its text");
        assert_eq!(b.find(c).unwrap().modified, 0);
    }

    #[test]
    fn replace_all_with_case_counted_matches_only_the_same_case() {
        let mut b = Board::default();
        let a = b.add_text("Foo foo FOO", 0.0, 0.0, 0);
        assert_eq!(b.replace_all("foo", "x", false, false, None, 1), 1);
        assert_eq!(b.find(a).unwrap().text, "Foo x FOO");
    }

    #[test]
    fn find_lists_every_match_ignoring_ascii_case() {
        let mut b = Board::default();
        let a = b.add_text("Foo bar foo", 0.0, 0.0, 0);
        let c = b.add_text("nothing", 0.0, 0.0, 0);
        let d = b.add_text("FOO", 0.0, 0.0, 0);
        let m = b.matches("foo", true, false);
        assert_eq!(m, vec![Match { note: a, start: 0, end: 3 }, Match { note: a, start: 8, end: 11 }, Match { note: d, start: 0, end: 3 }]);
        assert!(b.matches("", true, false).is_empty());
        assert!(b.matches("zzz", true, false).is_empty());
        let _ = c;
    }

    #[test]
    fn find_with_case_counted_skips_other_cases() {
        let mut b = Board::default();
        let a = b.add_text("Foo foo FOO", 0.0, 0.0, 0);
        assert_eq!(b.matches("foo", false, false), vec![Match { note: a, start: 4, end: 7 }]);
        assert_eq!(b.matches("FOO", false, false), vec![Match { note: a, start: 8, end: 11 }]);
    }

    #[test]
    fn whole_word_find_skips_matches_inside_longer_words() {
        let mut b = Board::default();
        let a = b.add_text("cat category cats cat.", 0.0, 0.0, 0);
        assert_eq!(b.matches("cat", true, false).len(), 4, "every occurrence without the option");
        assert_eq!(
            b.matches("cat", true, true),
            vec![Match { note: a, start: 0, end: 3 }, Match { note: a, start: 18, end: 21 }],
            "only the cat on its own and the cat before the full stop"
        );
        b.focus = None;
        let step = b.find_step("cat", true, true, true).unwrap();
        assert_eq!((step.note, step.start), (a, 0), "the first whole word from the top");
    }

    #[test]
    fn whole_word_replace_all_leaves_longer_words_alone() {
        let mut b = Board::default();
        let a = b.add_text("cat category cat", 0.0, 0.0, 0);
        assert_eq!(b.replace_all("cat", "dog", true, true, None, 1), 2);
        assert_eq!(b.find(a).unwrap().text, "dog category dog");
    }

    #[test]
    fn whole_word_replace_selected_needs_a_whole_word_selection() {
        let mut b = Board::default();
        let a = b.add_text("category", 0.0, 0.0, 0);
        b.raise(a);
        let n = b.find_mut(a).unwrap();
        n.anchor = 0;
        n.caret = 3;
        assert!(!b.replace_selected("cat", "dog", true, true, 2), "cat inside category is not a whole word");
        assert!(b.replace_selected("cat", "dog", true, false, 3), "without the option it is a match");
    }

    #[test]
    fn find_next_and_previous_walk_the_matches_and_wrap() {
        let mut b = Board::default();
        let a = b.add_text("foo bar foo", 0.0, 0.0, 0);
        let c = b.add_text("foo", 0.0, 0.0, 0);
        b.focus = None;
        let first = b.find_step("foo", true, true, false).unwrap();
        assert_eq!((first.note, first.start), (a, 0));
        b.select_match(&first);
        assert_eq!(b.focus, Some(a));
        assert_eq!(b.find(a).unwrap().selected_text(), "foo");
        let second = b.find_step("foo", true, true, false).unwrap();
        assert_eq!((second.note, second.start), (a, 8));
        b.select_match(&second);
        let third = b.find_step("foo", true, true, false).unwrap();
        assert_eq!((third.note, third.start), (c, 0));
        // raising moved note c to the front; stepping on from it wraps to the first match
        b.select_match(&third);
        let wrapped = b.find_step("foo", true, true, false).unwrap();
        assert_eq!((wrapped.note, wrapped.start), (a, 0));
        let back = b.find_step("foo", false, true, false).unwrap();
        assert_eq!((back.note, back.start), (a, 8));
        assert!(b.find_step("zzz", true, true, false).is_none());
        assert!(b.find_step("FOO", true, false, false).is_none(), "the case-counted search finds no upper case foo in these notes");
    }

    #[test]
    fn export_all_joins_notes_and_import_splits_them_back() {
        let mut b = Board::default();
        b.add_text("first", 0.0, 0.0, 0);
        b.add_text("second", 0.0, 0.0, 0);
        let all = b.export_text(None);
        assert_eq!(all, format!("first{EXPORT_SEPARATOR}second"));
        let mut other = Board::default();
        let ids = other.import_text(&all, 7);
        assert_eq!(ids.len(), 2);
        assert_eq!(other.find(ids[1]).map(|n| n.text.as_str()), Some("second"));
        assert_eq!(other.find(ids[0]).map(|n| n.created), Some(7));
    }

    #[test]
    fn import_of_blank_text_adds_nothing() {
        let mut b = Board::default();
        assert!(b.import_text("  \n ", 1).is_empty());
        assert!(b.notes.is_empty());
    }

    #[test]
    fn export_of_one_note_and_of_a_missing_note() {
        let mut b = Board::default();
        let a = b.add_text("only", 0.0, 0.0, 0);
        assert_eq!(b.export_text(Some(a)), "only");
        assert_eq!(b.export_text(Some(99)), "");
    }

    #[test]
    fn a_style_stays_with_its_characters_when_text_is_typed_before_it() {
        let mut n = Note::new(1, 0.0, 0.0, 0);
        n.set_text("abc");
        n.set_caret(1, false);
        n.set_caret(2, true);
        n.restyle(|s| s.bold = true);
        assert!(n.styles[1].bold && !n.styles[0].bold && !n.styles[2].bold);
        n.set_caret(0, false);
        n.type_text("XY", 1);
        assert_eq!(n.text, "XYabc");
        assert!(!n.styles[0].bold && !n.styles[1].bold, "typed text takes the style before the caret");
        assert!(n.styles[3].bold, "the bold b moved along with the text");
        assert_eq!(n.styles.len(), 5);
    }

    #[test]
    fn restyle_changes_the_selection_only_and_the_whole_note_without_one() {
        let mut n = Note::new(1, 0.0, 0.0, 0);
        n.set_text("one two");
        n.set_caret(4, false);
        n.set_caret(7, true);
        assert!(n.restyle(|s| s.underline = true));
        assert_eq!(n.style_runs(0..7).len(), 2, "the words before the selection, and the selection");
        assert!(!n.style.underline, "the typing style is not changed by a selection");
        assert!(!n.restyle(|s| s.underline = true), "already underlined");
        n.set_caret(0, false);
        assert!(n.restyle(|s| s.strike = true));
        assert!(n.styles.iter().all(|s| s.strike) && n.style.strike);
    }

    #[test]
    fn style_runs_group_neighbours_with_equal_styles() {
        let mut n = Note::new(1, 0.0, 0.0, 0);
        n.set_text("abcdé");
        n.set_caret(1, false);
        n.set_caret(3, true);
        n.restyle(bold_setter);
        let runs = n.style_runs(0..n.text.len());
        let ranges: Vec<Range<usize>> = runs.iter().map(|(r, _)| r.clone()).collect();
        assert_eq!(ranges, vec![0..1, 1..3, 3..6]);
        assert!(runs[1].1.bold && !runs[0].1.bold);
        assert_eq!(n.style_runs(1..3).len(), 1, "a sub range keeps its own runs");
    }

    fn bold_setter(s: &mut Style) {
        s.bold = true;
    }

    #[test]
    fn current_style_reads_the_selection_then_the_character_before_the_caret() {
        let mut n = Note::new(1, 0.0, 0.0, 0);
        n.set_text("ab");
        n.styles[1] = bold();
        n.set_caret(2, false);
        assert!(n.current_style().bold, "the character before the caret");
        n.set_caret(1, false);
        assert!(!n.current_style().bold);
        n.set_caret(0, false);
        n.set_caret(2, true);
        assert!(!n.current_style().bold, "the first selected character");
    }

    #[test]
    fn set_runs_replaces_the_body_and_takes_the_note_size() {
        let mut n = Note::new(1, 0.0, 0.0, 0);
        n.style.size = TextSize::Large;
        n.set_runs(&[("bold ".to_string(), bold()), ("plain".to_string(), Style::default())]);
        assert_eq!(n.text, "bold plain");
        assert_eq!(n.styles.len(), 10);
        assert!(n.styles[0].bold && !n.styles[9].bold);
        assert!(n.styles.iter().all(|s| s.size == TextSize::Large));
        assert_eq!(n.caret, 10);
    }

    #[test]
    fn rich_text_runs_keep_bold_and_italic() {
        let runs = runs_from_rtf(r"{\rtf1\ansi plain {\b bold} \i it\i0 }");
        let text: String = runs.iter().map(|(t, _)| t.as_str()).collect();
        assert!(text.contains("bold") && text.contains("it"));
        let bold_run = runs.iter().find(|(t, _)| t.contains("bold")).map(|(_, s)| *s);
        assert!(bold_run.is_some_and(|s| s.bold));
        assert!(runs.iter().any(|(t, s)| t.contains("it") && s.italic));
        assert!(runs.iter().any(|(t, s)| t.contains("plain") && !s.bold));
    }

    #[test]
    fn rich_text_colours_become_inks() {
        let rtf = r"{\rtf1\ansi{\colortbl;\red255\green0\blue0;}plain \cf1 red\cf0  plain}";
        let runs = runs_from_rtf(rtf);
        assert!(runs.iter().any(|(t, s)| t.contains("red") && s.ink == Ink::Red));
        assert!(runs.iter().any(|(t, s)| t.contains("plain") && s.ink == Ink::Black));
        assert_eq!(ink_of_colour(Some(ColorToken::Yellow)), Ink::Black, "yellow text is not readable on a note");
        assert_eq!(ink_of_colour(Some(ColorToken::Orange)), Ink::Red);
        assert_eq!(ink_of_colour(None), Ink::Black);
    }

    #[test]
    fn json_round_trip_keeps_every_field() {
        let mut b = Board::default();
        let a = b.add_text("Hello \"world\"\nline two", 12.5, 40.0, 100);
        {
            let n = b.find_mut(a).unwrap();
            n.colour = Colour::Pink;
            n.collapsed = true;
            n.translucent = false;
            n.style = Style { bold: true, italic: true, underline: true, strike: true, size: TextSize::Large, ink: Ink::Green, ..Style::default() };
            n.resize(300.0, 240.0);
            n.restore = Some((220.0, 200.0));
            n.modified = 150;
            n.rgb = Some((255, 128, 0));
            n.set_caret(0, false);
            n.set_caret(5, true);
            n.restyle(|s| {
                s.outline = true;
                s.baseline = Baseline::Subscript;
                s.kern = Setting::All;
                s.ligature = Setting::Off;
            });
            // the caret is not saved; a loaded note opens with it at the end
            let end = n.text.len();
            n.set_caret(end, false);
        }
        b.add_text("second", 0.0, 0.0, 101);
        b.default_colour = Colour::Teal;
        b.default_rgb = Some((1, 2, 3));
        b.default_style = Style { underline: true, baseline: Baseline::Superscript, ..Style::default() };
        let decoded = decode(&encode(&b)).unwrap();
        assert_eq!(decoded, b);
    }

    #[test]
    fn a_note_without_runs_gets_its_one_style_on_every_character() {
        let text = r#"{"version":1,"next_id":1,"notes":[{"id":1,"text":"abc","bold":true}]}"#;
        let b = decode(text).unwrap();
        let n = &b.notes[0];
        assert_eq!(n.styles.len(), 3);
        assert!(n.styles.iter().all(|s| s.bold));
    }

    #[test]
    fn runs_that_do_not_cover_the_text_are_ignored() {
        let text = r#"{"version":1,"next_id":1,"notes":[{"id":1,"text":"abc","runs":[{"n":2,"bold":true}]}]}"#;
        let b = decode(text).unwrap();
        assert!(b.notes[0].styles.iter().all(|s| !s.bold), "the note keeps its one style");
    }

    #[test]
    fn decode_skips_broken_notes_and_drops_a_stale_focus() {
        let text = r#"{"version":1,"next_id":3,"focus":9,"notes":[{"id":2,"colour":"Nope","text":"ok"},{"text":"no id"}]}"#;
        let b = decode(text).unwrap();
        assert_eq!(b.notes.len(), 1);
        assert_eq!(b.notes[0].colour, Colour::Yellow);
        assert_eq!(b.notes[0].text, "ok");
        assert!(b.notes[0].translucent, "a file from before the flag keeps notes translucent");
        assert_eq!(b.focus, None);
        assert_eq!(b.next_id, 3);
        assert_eq!(b.default_colour, Colour::Yellow);
    }

    #[test]
    fn decode_rejects_text_that_is_not_a_board() {
        assert!(decode("[]").is_err());
        assert!(decode("{").is_err());
    }
}
