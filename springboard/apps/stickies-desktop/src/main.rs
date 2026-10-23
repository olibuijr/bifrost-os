//! Sticky Notes: small floating notes on one board. Each note has a colour, a collapse control, a caret and
//! selection, styles that apply to a selection, and a translucent look when it is not the focused note. The board saves
//! on every change, and edits can be undone. Find searches all notes; spelling, substitutions and data detectors mark
//! and change text as it is typed; a note prints, exports and imports as text, and imports rich text too.

mod board;
mod dates;
mod history;
mod layout;
mod model;
mod page;
mod prefs;
mod rtf;
mod speech;
mod spell;
mod store;
mod substitute;
mod swatch;
mod words;

use bifrost_app::control::RpcError;
use bifrost_app::controls::sheet::{ButtonRole, Hit, Sheet, SheetButton};
use bifrost_app::controls::Rect;
use bifrost_app::intents::{self, Intent, IntentResult, Param, ParamType};
use bifrost_app::kit::canvas::Canvas;
use bifrost_app::kit::{Damage, Surface};
use bifrost_app::layout::UNIFIED_BAR_PT;
use bifrost_app::print::{Orientation, PrintError, PrintInfo, PrintOperation};
use bifrost_app::proto::{DocErrorCode, DocMode, CHORD_META, CHORD_SHIFT};
use bifrost_app::widgets;
use bifrost_app::windows::{self, Level};
use bifrost_core::json::Json;
use bifrost_app::{App, AppMenu, AppMenuItem, Command, Commands, Ctx, DocumentEvent, Input, KeyInput, TouchKind};
use board::Marks;
use history::History;
use model::{Arrange, Baseline, Board, Case, Colour, Ink, Move, Note, Part, Setting, Style, TextSize};
use page::PAPERS;
use prefs::Prefs;
use spell::Spelling;
use std::collections::HashMap;
use std::io::{Read, Write};
use substitute::Typed;

const MENU_NEW: u32 = 1;
const MENU_DELETE: u32 = 2;
const MENU_DUPLICATE: u32 = 23;
const MENU_COLLAPSE: u32 = 3;
const MENU_BOLD: u32 = 4;
const MENU_ITALIC: u32 = 5;
const MENU_SIZE_BASE: u32 = 6; // 6, 7, 8: small, medium, large
const MENU_COLOUR_BASE: u32 = 10; // 10..=15, one per colour
const MENU_COLOUR_CUSTOM: u32 = 16;
const MENU_IMPORT: u32 = 20;
const MENU_EXPORT: u32 = 21;
const MENU_EXPORT_ALL: u32 = 22;
const MENU_PRINT: u32 = 24;
const MENU_UNDO: u32 = 25;
const MENU_REDO: u32 = 26;
const MENU_CUT: u32 = 27;
const MENU_COPY: u32 = 28;
const MENU_PASTE: u32 = 29;
const MENU_PASTE_PLAIN: u32 = 30;
const MENU_SELECT_ALL: u32 = 31;
const MENU_FIND: u32 = 32;
const MENU_FIND_NEXT: u32 = 33;
const MENU_FIND_PREVIOUS: u32 = 34;
const MENU_USE_SELECTION: u32 = 35;
const MENU_OPEN_LINK: u32 = 36;
const MENU_UPPER: u32 = 37;
const MENU_LOWER: u32 = 38;
const MENU_CAPITALISE: u32 = 39;
const MENU_UNDERLINE: u32 = 40;
const MENU_STRIKE: u32 = 41;
const MENU_BIGGER: u32 = 42;
const MENU_SMALLER: u32 = 43;
const MENU_COPY_STYLE: u32 = 44;
const MENU_PASTE_STYLE: u32 = 45;
const MENU_USE_DEFAULT: u32 = 46;
const MENU_COLLAPSE_ALL: u32 = 47;
const MENU_EXPAND_ALL: u32 = 48;
const MENU_TRANSLUCENT: u32 = 49;
const MENU_ZOOM: u32 = 50;
const MENU_ARRANGE_BASE: u32 = 52; // 52..=56, one per Arrange
const MENU_INK_BASE: u32 = 60; // 60..=64, one per Ink
const MENU_INFO_CREATED: u32 = 70;
const MENU_INFO_MODIFIED: u32 = 71;
const MENU_SPEAK_START: u32 = 72;
const MENU_SPEAK_STOP: u32 = 73;
const MENU_REPLACE: u32 = 74;
const MENU_REPLACE_ALL: u32 = 75;
const MENU_OUTLINE: u32 = 79;
const MENU_BASELINE_BASE: u32 = 80; // 80..=82, one per Baseline
const MENU_KERN_BASE: u32 = 83; // 83..=85, one per Setting
const MENU_LIGATURE_BASE: u32 = 86; // 86..=88, one per Setting
const MENU_ADD_LINK: u32 = 89;
const MENU_SPELL_WHILE_TYPING: u32 = 90;
const MENU_SPELL_CHECK_NOW: u32 = 91;
const MENU_SPELL_AUTO: u32 = 92;
const MENU_SPELL_CORRECT: u32 = 93;
const MENU_SPELL_LEARN: u32 = 94;
const MENU_SPELL_IGNORE: u32 = 95;
const MENU_SMART_QUOTES: u32 = 96;
const MENU_SMART_DASHES: u32 = 97;
const MENU_DETECT_DATES: u32 = 98;
const MENU_DETECT_ADDRESSES: u32 = 99;
const MENU_DETECT_PHONES: u32 = 100;
const MENU_IGNORE_CASE: u32 = 101;
const MENU_REPLACE_IN_NOTE: u32 = 102;
const MENU_PORTRAIT: u32 = 103;
const MENU_LANDSCAPE: u32 = 104;
const MENU_PAPER_BASE: u32 = 105; // 105..=109, one per entry of PAPERS
const MENU_WHOLE_WORD: u32 = 110;
/// Window > Float on Top: the window stays above the other windows (`windows::set_level`).
const MENU_FLOAT_ON_TOP: u32 = 111;
/// File > Close (Cmd+W): closes the front note, as a note's close box does: one with text asks first, then it is
/// deleted (W2-22: Cmd+W closed the whole board window).
const MENU_CLOSE: u32 = 112;

/// What a replace changes: the selected match, every match in the focused note, or every match in every note.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scope {
    Selection,
    FocusedNote,
    AllNotes,
}

/// The buttons of the delete confirmation.
const SHEET_CANCEL: u32 = 1;
const SHEET_DELETE: u32 = 2;

/// The text of the note the first launch opens.
const WELCOME_TEXT: &str = "Make a note of it!\n\nSticky Notes keeps small notes on your desktop: reminders, lists, or text you use often.\n\n\u{2022} To add another note, choose File, New Sticky Note.\n\n\u{2022} To collapse a note, double-click its title bar.\n\nYour notes are here again when you open Sticky Notes.";
/// Stacking offset of each new note, so new notes do not land on each other.
const CASCADE_PT: f32 = 24.0;
const CASCADE_STEPS: usize = 8;
/// Two presses on the same note within this many seconds are a double press.
const DOUBLE_PRESS_S: f64 = 0.4;

/// A pointer gesture in progress.
#[derive(Clone, Copy, Debug)]
enum Drag {
    None,
    /// Down on a note's disclosure: a release on the disclosure toggles the note.
    Press { id: u64, part: Part },
    /// Down on a title bar: the note follows the pointer, keeping the grab offset.
    Move { id: u64, dx: f32, dy: f32 },
    /// Down on the grip: the note resizes from its size at the start.
    Resize { id: u64, x: f32, y: f32, w: f32, h: f32 },
    /// Down in a body: the selection grows from the caret the press placed.
    Select { id: u64 },
}

/// The question "discard this note?" while its sheet is open.
struct Confirm {
    sheet: Sheet,
    note: u64,
}

impl Confirm {
    fn delete(note: u64, title: &str) -> Confirm {
        let name = if title.is_empty() { "this note".to_string() } else { format!("\"{title}\"") };
        let buttons = vec![SheetButton::new(SHEET_CANCEL, "Cancel", ButtonRole::Cancel), SheetButton::new(SHEET_DELETE, "Delete", ButtonRole::Destructive)];
        let sheet = Sheet::alert("Delete this note?", &format!("{name} will be deleted. You can undo this with Edit, Undo."), buttons);
        Confirm { sheet, note }
    }
}

/// The frames of all notes, to tell whether a drag moved or resized anything.
fn frames(board: &Board) -> Vec<(u64, f32, f32, f32, f32)> {
    board.notes.iter().map(|n| (n.id, n.x, n.y, n.w, n.h)).collect()
}

/// The Edit menu name of the change a menu action makes, or `None` for an action that leaves the board alone.
fn undo_name(id: u32) -> Option<&'static str> {
    Some(match id {
        MENU_NEW => "New Note",
        MENU_DELETE | MENU_CLOSE => "Delete",
        MENU_DUPLICATE => "Duplicate",
        MENU_COLLAPSE | MENU_COLLAPSE_ALL | MENU_EXPAND_ALL => "Collapse",
        MENU_BOLD | MENU_ITALIC | MENU_UNDERLINE | MENU_STRIKE | MENU_OUTLINE | MENU_BIGGER | MENU_SMALLER | MENU_PASTE_STYLE | MENU_USE_DEFAULT => "Style",
        MENU_BASELINE_BASE..=82 | MENU_KERN_BASE..=85 | MENU_LIGATURE_BASE..=88 => "Style",
        MENU_SIZE_BASE..=8 => "Text Size",
        MENU_INK_BASE..=64 => "Text Colour",
        MENU_COLOUR_BASE..=15 => "Colour",
        MENU_CUT => "Cut",
        MENU_PASTE | MENU_PASTE_PLAIN => "Paste",
        MENU_UPPER | MENU_LOWER | MENU_CAPITALISE => "Transformation",
        MENU_TRANSLUCENT => "Translucent",
        MENU_ZOOM => "Zoom",
        MENU_ARRANGE_BASE..=56 => "Arrange",
        MENU_SPELL_CORRECT => "Correct Spelling",
        _ => return None,
    })
}

/// Whether typed text is a word boundary (one space or punctuation mark), after which Correct Spelling Automatically
/// checks the word before it. An apostrophe is part of a word.
fn is_boundary(text: &str) -> bool {
    let mut chars = text.chars();
    matches!((chars.next(), chars.next()), (Some(c), None) if !c.is_alphanumeric() && c != '\'' && c != '\u{2019}' && !c.is_control())
}

/// Types `text` into the note. A single typed character goes through the Substitutions rules first: Smart Quotes and
/// Smart Dashes.
fn type_with_substitutions(note: &mut Note, text: &str, prefs: Prefs, now: u64) {
    let mut chars = text.chars();
    let single = match (chars.next(), chars.next()) {
        (Some(c), None) if prefs.smart_quotes || prefs.smart_dashes => Some(c),
        _ => None,
    };
    let Some(c) = single else {
        note.type_text(text, now);
        return;
    };
    let before = note.text[..note.selection().start].chars().next_back();
    match substitute::typed(before, c, prefs.smart_quotes, prefs.smart_dashes) {
        Some(Typed::Replace(dash)) if !note.has_selection() => {
            note.backspace(now);
            note.type_text(&dash.to_string(), now);
        }
        Some(Typed::Insert(s)) => note.type_text(&s, now),
        _ => note.type_text(text, now),
    }
}

/// Correct Spelling Automatically: replaces the misspelled word that ends right before the boundary the caret just
/// passed. The caret stays after the boundary.
fn autocorrect_before_boundary(note: &mut Note, spelling: &Spelling, typed_len: usize, now: u64) {
    let Some(boundary) = note.caret.checked_sub(typed_len) else { return };
    let Some((range, guess)) = spelling.autocorrect(&note.text, boundary) else { return };
    let caret = note.caret;
    let shifted = caret + guess.len() - range.len();
    note.replace_range(range, &guess, now);
    note.set_caret(shifted, false);
}

struct StickiesApp {
    board: Option<Board>,
    /// Window > Float on Top: the window is kept above the other windows.
    floating: bool,
    drag: Drag,
    dirty: bool,
    /// Export waiting for the save panel's file: the request and the bytes to write.
    export: Option<(u32, Vec<u8>)>,
    /// The request of an import panel, when one is open.
    import: Option<u32>,
    history: History,
    /// The find bar's text while the bar is open.
    find: Option<String>,
    /// The last find text, which Find Next uses once the bar is closed.
    last_find: String,
    /// The text Replace puts in, and whether the find bar's typing goes to it (Tab switches fields).
    replace: String,
    replacing: bool,
    /// The custom colour entry while its bar is open.
    colour_entry: Option<String>,
    /// The web address entry of Add Link while its bar is open.
    link_entry: Option<String>,
    speaker: speech::Speaker,
    confirm: Option<Confirm>,
    /// The style Copy Style took, for Paste Style.
    style_clip: Option<Style>,
    /// The note and time of the last press, for double presses.
    last_press: Option<(u64, f64)>,
    /// The board as it was when a press began, for the undo step of a move or resize.
    before_drag: Option<Board>,
    /// The page Print lays the note out for: paper and orientation, set by File > Page Setup for this session.
    page: PrintInfo,
    /// The spelling checker, made the first time it is needed.
    spelling: Option<Spelling>,
    /// The switches of the Spelling, Substitutions and Data Detectors menus.
    prefs: Prefs,
    /// The rules under the text of each note, and the text they were made for.
    marks: Marks,
    marked: HashMap<u64, String>,
}

impl StickiesApp {
    fn new() -> StickiesApp {
        StickiesApp {
            board: None,
            floating: false,
            drag: Drag::None,
            dirty: false,
            export: None,
            import: None,
            history: History::default(),
            find: None,
            last_find: String::new(),
            replace: String::new(),
            replacing: false,
            colour_entry: None,
            link_entry: None,
            speaker: speech::Speaker::default(),
            confirm: None,
            style_clip: None,
            last_press: None,
            before_drag: None,
            page: PrintInfo::default(),
            spelling: None,
            prefs: Prefs::default(),
            marks: Marks::new(),
            marked: HashMap::new(),
        }
    }

    /// Saves the board when something changed since the last save.
    fn flush(&mut self, ctx: &Ctx) {
        if !self.dirty {
            return;
        }
        let Some(board) = &self.board else { return };
        match store::save(&ctx.data_dir, board) {
            Ok(()) => self.dirty = false,
            Err(e) => ctx.log(&format!("save: {e}")),
        }
    }

    fn changed(&mut self) -> bool {
        self.dirty = true;
        true
    }

    /// Makes the spelling checker, loading the words the user taught it, the first time it is needed.
    fn ensure_spelling(&mut self, ctx: &Ctx) {
        if self.spelling.is_none() {
            let mut spelling = Spelling::new();
            spelling.load(&ctx.defaults());
            self.spelling = Some(spelling);
        }
    }

    /// Recomputes the marks of the notes whose text changed, with the checker when spelling is checked as typing.
    fn refresh_marks(&mut self, ctx: &Ctx) {
        if self.board.is_none() {
            return;
        }
        if self.prefs.check_while_typing {
            self.ensure_spelling(ctx);
        }
        let spelling = if self.prefs.check_while_typing { self.spelling.as_ref() } else { None };
        let kinds = self.prefs.detector_kinds();
        let Some(board) = &self.board else { return };
        self.marks.retain(|id, _| board.find(*id).is_some());
        self.marked.retain(|id, _| board.find(*id).is_some());
        for note in &board.notes {
            if self.marked.get(&note.id) != Some(&note.text) {
                self.marks.insert(note.id, spell::marks(&note.text, spelling, &kinds));
                self.marked.insert(note.id, note.text.clone());
            }
        }
    }

    /// Drops the marks, so the next draw computes them again (after a switch or the learned words change).
    fn forget_marks(&mut self) {
        self.marks.clear();
        self.marked.clear();
    }

    /// Pointer position in points: input is in pixels, and the board is laid out in points.
    fn to_points(ctx: &Ctx, x: f32, y: f32) -> (f32, f32) {
        let scale = ctx.ui.pt(1.0).max(0.001);
        (x / scale, y / scale)
    }

    /// The window in points, the parent of the confirmation sheet.
    fn window_rect(ctx: &Ctx) -> Rect {
        let (w, h) = Self::to_points(ctx, ctx.cfg.w as f32, ctx.cfg.h as f32);
        Rect { x: 0.0, y: 0.0, w, h }
    }

    fn cascade_position(board: &Board) -> (f32, f32) {
        let step = board.notes.len() % CASCADE_STEPS;
        (40.0 + CASCADE_PT * step as f32, 60.0 + CASCADE_PT * step as f32)
    }

    fn now() -> u64 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
    }

    fn new_note(&mut self) -> bool {
        let Some(board) = &mut self.board else { return false };
        let (x, y) = Self::cascade_position(board);
        board.new_note(x, y, Self::now());
        self.changed()
    }

    /// Adds a note holding `text` in front, as one undo step, and returns its id. Before the app has loaded its board
    /// the note goes into the saved file instead, which the app reads on launch.
    fn create_sticky(&mut self, data_dir: &std::path::Path, text: &str) -> Result<u64, String> {
        if text.trim().is_empty() {
            return Err("There is no text for the sticky note".to_string());
        }
        let now = Self::now();
        match &mut self.board {
            Some(board) => {
                let before = board.clone();
                let (x, y) = Self::cascade_position(board);
                let id = board.add_text(text, x, y, now);
                self.history.push("New Note", before);
                self.dirty = true;
                Ok(id)
            }
            None => {
                let mut board = store::load(data_dir);
                let (x, y) = Self::cascade_position(&board);
                let id = board.add_text(text, x, y, now);
                store::save(data_dir, &board).map_err(|e| e.to_string())?;
                Ok(id)
            }
        }
    }

    fn touch(&mut self, ctx: &Ctx, kind: TouchKind, x: f32, y: f32, t: f64) -> bool {
        let (px, py) = Self::to_points(ctx, x, y);
        if let Some(confirm) = &self.confirm {
            // an open sheet blocks the board: only its buttons answer
            if kind == TouchKind::Down {
                if let Hit::Button(id) = confirm.sheet.hit(Self::window_rect(ctx), UNIFIED_BAR_PT, 1.0, px, py) {
                    return self.answer(id);
                }
            }
            return true;
        }
        match kind {
            TouchKind::Down => self.press(ctx, x, y, px, py, t),
            TouchKind::Move => {
                let Some(board) = &mut self.board else { return false };
                match self.drag {
                    Drag::Move { id, dx, dy } => {
                        if let Some(n) = board.find_mut(id) {
                            n.x = (px - dx).max(0.0);
                            n.y = (py - dy).max(0.0);
                        }
                        self.changed()
                    }
                    Drag::Resize { id, x: x0, y: y0, w, h } => {
                        if let Some(n) = board.find_mut(id) {
                            n.resize(w + px - x0, h + py - y0);
                        }
                        self.changed()
                    }
                    Drag::Select { id } => {
                        if let Some(n) = board.find_mut(id) {
                            let pos = board::offset_at(&ctx.ui, n, x, y);
                            n.set_caret(pos, true);
                        }
                        true
                    }
                    _ => false,
                }
            }
            TouchKind::Up | TouchKind::Cancel => {
                let drag = std::mem::replace(&mut self.drag, Drag::None);
                let before = self.before_drag.take();
                let Some(board) = &mut self.board else { return false };
                match (drag, kind) {
                    (Drag::Press { id, part: Part::Disclosure }, TouchKind::Up) => {
                        let on_disclosure = board.find(id).is_some_and(|n| n.part_at(px, py) == Some(Part::Disclosure));
                        if on_disclosure {
                            let snapshot = board.clone();
                            if let Some(n) = board.find_mut(id) {
                                n.collapsed = !n.collapsed;
                            }
                            self.history.push("Collapse", snapshot);
                            return self.changed();
                        }
                        true
                    }
                    (Drag::Move { .. } | Drag::Resize { .. }, _) => {
                        if let Some(before) = before {
                            if frames(&before) != frames(board) {
                                let name = if matches!(drag, Drag::Move { .. }) { "Move" } else { "Resize" };
                                self.history.push(name, before);
                            }
                        }
                        self.changed()
                    }
                    _ => true,
                }
            }
        }
    }

    /// A press on the board: it focuses the note under it and starts the gesture its part asks for.
    fn press(&mut self, ctx: &Ctx, x: f32, y: f32, px: f32, py: f32, t: f64) -> bool {
        self.history.end_typing();
        let Some(board) = &mut self.board else { return false };
        let Some(id) = board.hit(px, py) else {
            // a press on the bare board takes focus away from every note
            self.last_press = None;
            let had = board.focus.is_some();
            board.focus = None;
            return had;
        };
        let double = self.last_press.is_some_and(|(note, at)| note == id && t - at < DOUBLE_PRESS_S);
        self.last_press = if double { None } else { Some((id, t)) };
        let snapshot = board.clone();
        board.raise(id);
        let Some(note) = board.find_mut(id) else { return true };
        let part = note.part_at(px, py).unwrap_or(Part::Body);
        match part {
            Part::Title if double => {
                // a double press on the title bar collapses or expands the note
                note.collapsed = !note.collapsed;
                self.history.push("Collapse", snapshot);
                return self.changed();
            }
            Part::Title => {
                self.drag = Drag::Move { id, dx: px - note.x, dy: py - note.y };
                self.before_drag = Some(snapshot);
            }
            Part::Grip => {
                self.drag = Drag::Resize { id, x: px, y: py, w: note.w, h: note.h };
                self.before_drag = Some(snapshot);
            }
            Part::Body => {
                let pos = board::offset_at(&ctx.ui, note, x, y);
                if double {
                    match note.link_at(pos) {
                        Some(url) => ctx.open_url(&url),
                        None => note.select_word_at(pos),
                    }
                } else {
                    note.set_caret(pos, false);
                    self.drag = Drag::Select { id };
                }
            }
            part => self.drag = Drag::Press { id, part },
        }
        true
    }

    fn key(&mut self, ctx: &Ctx, key: &KeyInput) -> bool {
        if let Some(confirm) = &self.confirm {
            // Escape answers Cancel; the sheet takes every other key so nothing reaches the notes under it
            if let (KeyInput::Code(1 | 27), Some(id)) = (key, confirm.sheet.cancel_button()) {
                return self.answer(id);
            }
            return false;
        }
        if self.colour_entry.is_some() {
            return self.colour_key(key);
        }
        if self.link_entry.is_some() {
            return self.link_key(key);
        }
        if self.find.is_some() {
            return self.find_key(key);
        }
        // a command arrow (Cmd with an arrow key) moves the caret to the edge of the line or the text
        if let Some((code, mods)) = key.as_chord() {
            return self.chord(code, mods);
        }
        // Escape leaves the note: no note takes typing until one is pressed again
        if let KeyInput::Code(1 | 27) = key {
            let Some(board) = &mut self.board else { return false };
            if board.focus.take().is_none() {
                return false;
            }
            return self.changed();
        }
        if let KeyInput::Text(text) = key {
            if self.prefs.correct_automatically && is_boundary(text) {
                self.ensure_spelling(ctx);
            }
            return self.typed(text);
        }
        let now = Self::now();
        let Some(board) = &mut self.board else { return false };
        let before = board.clone();
        let Some(note) = board.focused_mut() else { return false };
        let id = note.id;
        match key {
            KeyInput::Code(8) => {
                if !note.backspace(now) {
                    return false;
                }
            }
            KeyInput::Code(46) => {
                if !note.delete_forward(now) {
                    return false;
                }
            }
            KeyInput::Code(13) => note.newline(now),
            KeyInput::Code(code @ (35 | 36 | 37 | 39)) => {
                let dir = match *code {
                    35 => Move::LineEnd,
                    36 => Move::LineStart,
                    37 => Move::Left,
                    _ => Move::Right,
                };
                note.move_caret(dir, false);
                self.history.end_typing();
                return true;
            }
            KeyInput::Code(code @ (38 | 40)) => {
                let pos = board::caret_vertical(&ctx.ui, note, *code == 40);
                note.set_caret(pos, false);
                self.history.end_typing();
                return true;
            }
            _ => return false,
        }
        if *board != before {
            self.history.push_typing(id, before);
        }
        self.changed()
    }

    /// Types `text` into the focused note: the Substitutions rules first, then Correct Spelling Automatically when a
    /// boundary was typed. The typing is one undo step, shared with the typing before it in the same note.
    fn typed(&mut self, text: &str) -> bool {
        if text.chars().all(char::is_control) {
            return false;
        }
        let prefs = self.prefs;
        let spelling = if prefs.correct_automatically && is_boundary(text) { self.spelling.as_ref() } else { None };
        let now = Self::now();
        let Some(board) = &mut self.board else { return false };
        let before = board.clone();
        let Some(note) = board.focused_mut() else { return false };
        let id = note.id;
        type_with_substitutions(note, text, prefs, now);
        if let Some(sp) = spelling {
            autocorrect_before_boundary(note, sp, text.len(), now);
        }
        if *board != before {
            self.history.push_typing(id, before);
        }
        self.changed()
    }

    /// Command arrows: Cmd-Left and Cmd-Right go to the ends of the line, Cmd-Up and Cmd-Down to the ends of the text,
    /// and Shift with them extends the selection.
    fn chord(&mut self, code: u32, mods: u32) -> bool {
        if mods & CHORD_META == 0 {
            return false;
        }
        let extend = mods & CHORD_SHIFT != 0;
        let Some(board) = &mut self.board else { return false };
        let Some(note) = board.focused_mut() else { return false };
        let end = note.text.len();
        match code {
            37 => note.move_caret(Move::LineStart, extend),
            39 => note.move_caret(Move::LineEnd, extend),
            38 => note.set_caret(0, extend),
            40 => note.set_caret(end, extend),
            _ => return false,
        }
        self.history.end_typing();
        true
    }

    /// Keys while the find bar is open: they edit the find text instead of a note.
    fn find_key(&mut self, key: &KeyInput) -> bool {
        if self.find.is_none() {
            return false;
        }
        // Tab moves between the find and the replace field
        if matches!(key, KeyInput::Code(9)) || matches!(key, KeyInput::Text(t) if t.contains('\t')) {
            self.replacing = !self.replacing;
            return true;
        }
        if self.replacing {
            return self.replace_key(key);
        }
        let Some(query) = &mut self.find else { return false };
        match key {
            KeyInput::Text(text) => query.extend(text.chars().filter(|c| !c.is_control())),
            KeyInput::Code(8) => {
                if query.pop().is_none() {
                    return false;
                }
            }
            KeyInput::Code(13) => {
                self.step_find(true);
                return true;
            }
            KeyInput::Code(1 | 27) => {
                self.find = None;
                self.replacing = false;
                return true;
            }
            _ => return false,
        }
        self.last_find = query.clone();
        // the match under the caret stays while it still matches: search from the start of the selection
        if let Some(note) = self.board.as_mut().and_then(|b| b.focused_mut()) {
            let start = note.selection().start;
            note.set_caret(start, false);
        }
        self.step_find(true);
        true
    }

    /// Keys in the replace field of the find bar. Return replaces the selected match and moves to the next one.
    fn replace_key(&mut self, key: &KeyInput) -> bool {
        match key {
            KeyInput::Text(text) => self.replace.extend(text.chars().filter(|c| !c.is_control())),
            KeyInput::Code(8) => {
                if self.replace.pop().is_none() {
                    return false;
                }
            }
            KeyInput::Code(13) => return self.replace(Scope::Selection),
            KeyInput::Code(1 | 27) => {
                self.find = None;
                self.replacing = false;
            }
            _ => return false,
        }
        true
    }

    /// Replaces the matches in `scope` and records one undo step when something was replaced. With no find text the
    /// find bar opens instead.
    fn replace(&mut self, scope: Scope) -> bool {
        let before = self.board.clone();
        if self.replace_step(scope, Self::now()) > 0 {
            if let Some(before) = before {
                self.history.push("Replace", before);
            }
        }
        true
    }

    /// The replacement itself, with no undo step. Returns how many matches were replaced.
    fn replace_step(&mut self, scope: Scope, now: u64) -> usize {
        let query = self.find.clone().unwrap_or_else(|| self.last_find.clone());
        if query.is_empty() {
            self.find = Some(String::new());
            return 0;
        }
        let with = self.replace.clone();
        let ignore_case = self.prefs.find_ignore_case;
        let whole_word = self.prefs.find_whole_word;
        let Some(board) = &mut self.board else { return 0 };
        let done = match scope {
            Scope::Selection => usize::from(board.replace_selected(&query, &with, ignore_case, whole_word, now)),
            Scope::FocusedNote => match board.focus {
                Some(id) => board.replace_all(&query, &with, ignore_case, whole_word, Some(id), now),
                None => 0,
            },
            Scope::AllNotes => board.replace_all(&query, &with, ignore_case, whole_word, None, now),
        };
        if done > 0 {
            self.dirty = true;
        }
        if scope == Scope::Selection {
            self.step_find(true);
        }
        done
    }

    /// Keys in the custom colour bar: hex digits build the colour, Return applies it, Escape closes the bar.
    fn colour_key(&mut self, key: &KeyInput) -> bool {
        let Some(entry) = &mut self.colour_entry else { return false };
        match key {
            KeyInput::Text(text) => {
                let mut taken = false;
                for c in text.chars() {
                    taken |= swatch::push(entry, c);
                }
                taken
            }
            KeyInput::Code(8) => entry.pop().is_some(),
            KeyInput::Code(13) => match swatch::parse(entry) {
                Some(rgb) => {
                    self.colour_entry = None;
                    self.apply_custom(rgb);
                    true
                }
                None => false,
            },
            KeyInput::Code(1 | 27) => {
                self.colour_entry = None;
                true
            }
            _ => false,
        }
    }

    /// Keys in the web address bar of Add Link: typing builds the address, Return adds it as a link, Escape closes it.
    fn link_key(&mut self, key: &KeyInput) -> bool {
        let Some(entry) = &mut self.link_entry else { return false };
        match key {
            KeyInput::Text(text) => {
                let before = entry.len();
                entry.extend(text.chars().filter(|c| !c.is_control()));
                entry.len() != before
            }
            KeyInput::Code(8) => entry.pop().is_some(),
            KeyInput::Code(13) => {
                if !model::is_link(entry) {
                    return false;
                }
                let url = entry.trim().to_string();
                self.link_entry = None;
                self.add_link(&url)
            }
            KeyInput::Code(1 | 27) => {
                self.link_entry = None;
                true
            }
            _ => false,
        }
    }

    /// Adds a web address to the focused note as text in place of the selection, as one undo step. The note underlines
    /// it and a double press opens it.
    fn add_link(&mut self, url: &str) -> bool {
        let before = self.board.clone();
        let now = Self::now();
        let Some(note) = self.board.as_mut().and_then(|b| b.focused_mut()) else { return false };
        note.type_text(url, now);
        if let Some(before) = before {
            self.history.push("Add Link", before);
        }
        self.changed()
    }

    /// Gives the focused note a custom colour, as one undo step. False without a focused note or a change.
    fn apply_custom(&mut self, rgb: swatch::Rgb) -> bool {
        let Some(board) = &mut self.board else { return false };
        let before = board.clone();
        let Some(note) = board.focused_mut() else { return false };
        if note.rgb == Some(rgb) {
            return false;
        }
        note.rgb = Some(rgb);
        self.history.push("Colour", before);
        self.changed()
    }

    /// Selects the next or previous match of the find text. With no text the find bar opens.
    fn step_find(&mut self, forward: bool) {
        let query = self.find.clone().unwrap_or_else(|| self.last_find.clone());
        if query.is_empty() {
            self.find = Some(String::new());
            return;
        }
        let ignore_case = self.prefs.find_ignore_case;
        let whole_word = self.prefs.find_whole_word;
        let Some(board) = &mut self.board else { return };
        if let Some(found) = board.find_step(&query, forward, ignore_case, whole_word) {
            board.select_match(&found);
        }
    }

    fn import_panel(&mut self, ctx: &Ctx) -> bool {
        self.import = Some(ctx.open_document(DocMode::Read, "Import Sticky Note", &["public.text", "public.rtf"]));
        false
    }

    fn export_panel(&mut self, ctx: &Ctx, id: Option<u64>) -> bool {
        let Some(board) = &self.board else { return false };
        let text = board.export_text(id);
        let name = match id.and_then(|i| board.find(i)) {
            Some(n) if !n.title().is_empty() => format!("{}.txt", n.title()),
            _ => "Sticky Notes.txt".to_string(),
        };
        let request = ctx.save_document(&name, &["public.text"]);
        self.export = Some((request, text.into_bytes()));
        false
    }

    /// Sends the focused note to the default printer and tells the user how that went.
    fn print(&self, ctx: &Ctx) {
        let Some(note) = self.board.as_ref().and_then(|b| b.focused()) else { return };
        let title = if note.title().is_empty() { "Sticky Note".to_string() } else { note.title() };
        let message = match PrintOperation::text(title, note.text.clone()).with_info(self.page.clone()).run() {
            Ok(_) => "Sent to the printer".to_string(),
            Err(PrintError::NoCups) => "The printing service is not running".to_string(),
            Err(PrintError::NoPrinter) => "No printer is set up".to_string(),
            Err(PrintError::Failed(why)) => format!("Printing failed: {why}"),
        };
        ctx.notify("Sticky Notes", &message);
    }

    /// The answer of a button in the delete confirmation.
    fn answer(&mut self, id: u32) -> bool {
        let Some(confirm) = self.confirm.take() else { return false };
        if id == SHEET_DELETE {
            if let Some(board) = &mut self.board {
                let before = board.clone();
                if board.remove(confirm.note) {
                    self.history.push("Delete", before);
                    self.dirty = true;
                }
            }
        }
        true
    }

    /// Copy Style: remembers the text style at the focused note's caret.
    fn copy_style(&mut self) {
        self.style_clip = self.board.as_ref().and_then(|b| b.focused()).map(|n| n.current_style());
    }

    /// Paste Style: gives the selection (or the whole note) the remembered style, and keeps the note's text size. False
    /// when none was copied or nothing changed.
    fn paste_style(&mut self) -> bool {
        let Some(clip) = self.style_clip else { return false };
        let Some(note) = self.board.as_mut().and_then(|b| b.focused_mut()) else { return false };
        note.restyle(move |s| *s = (*s).take_attributes(clip))
    }

    /// Applies a style change to the focused note: its selection, or the whole note without one. True when a style changed.
    fn restyle_focused(&mut self, f: impl Fn(&mut Style)) -> bool {
        let Some(note) = self.board.as_mut().and_then(|b| b.focused_mut()) else { return false };
        note.restyle(f) && self.changed()
    }

    /// Toggles one attribute of the style at the caret: on when it is off at the caret, off otherwise.
    fn toggle_style(&mut self, on: fn(&Style) -> bool, set: fn(&mut Style, bool)) -> bool {
        let Some(note) = self.board.as_mut().and_then(|b| b.focused_mut()) else { return false };
        let value = !on(&note.current_style());
        note.restyle(move |s| set(s, value)) && self.changed()
    }

    /// Flips one of the Spelling, Substitutions or Data Detectors switches, saves it and drops the marks.
    fn toggle_pref(&mut self, ctx: &Ctx, id: u32) -> bool {
        let p = &mut self.prefs;
        match id {
            MENU_SPELL_WHILE_TYPING => p.check_while_typing = !p.check_while_typing,
            MENU_SPELL_AUTO => p.correct_automatically = !p.correct_automatically,
            MENU_SMART_QUOTES => p.smart_quotes = !p.smart_quotes,
            MENU_SMART_DASHES => p.smart_dashes = !p.smart_dashes,
            MENU_DETECT_DATES => p.detect_dates = !p.detect_dates,
            MENU_DETECT_ADDRESSES => p.detect_addresses = !p.detect_addresses,
            MENU_DETECT_PHONES => p.detect_phone_numbers = !p.detect_phone_numbers,
            _ => return false,
        }
        let prefs = self.prefs;
        prefs.save(&mut ctx.defaults_mut());
        self.forget_marks();
        true
    }

    /// Check Document Now: selects the next misspelled word after the selection, wrapping round the note.
    fn check_document_now(&mut self, ctx: &Ctx) -> bool {
        self.ensure_spelling(ctx);
        let (Some(board), Some(spelling)) = (self.board.as_mut(), self.spelling.as_ref()) else { return false };
        let Some(note) = board.focused_mut() else { return false };
        let ranges = spelling.misspelled(&note.text);
        match spell::next_misspelled(&ranges, note.selection().end) {
            Some(r) => {
                note.set_caret(r.start, false);
                note.set_caret(r.end, true);
                true
            }
            None => {
                ctx.notify("Sticky Notes", "No misspelled words");
                false
            }
        }
    }

    /// Correct Spelling: replaces the word at the caret with the checker's best guess.
    fn correct_spelling(&mut self, ctx: &Ctx) -> bool {
        self.ensure_spelling(ctx);
        let now = Self::now();
        let (Some(board), Some(spelling)) = (self.board.as_mut(), self.spelling.as_ref()) else { return false };
        let Some(note) = board.focused_mut() else { return false };
        let Some(range) = spell::word_at(&note.text, note.caret) else { return false };
        let Some(guess) = spelling.correction(&note.text, range.clone()) else { return false };
        if guess == note.text[range.clone()] {
            return false;
        }
        let caret = note.caret;
        let shifted = caret + guess.len() - range.len();
        note.replace_range(range, &guess, now);
        note.set_caret(shifted, false);
        self.changed()
    }

    /// Learn Spelling and Ignore Spelling for the word at the caret.
    fn learn_or_ignore(&mut self, ctx: &Ctx, id: u32) -> bool {
        self.ensure_spelling(ctx);
        let (Some(board), Some(spelling)) = (self.board.as_ref(), self.spelling.as_mut()) else { return false };
        let Some(note) = board.focused() else { return false };
        let Some(range) = spell::word_at(&note.text, note.caret) else { return false };
        let word = note.text[range].to_string();
        if id == MENU_SPELL_LEARN {
            spelling.learn(&mut ctx.defaults_mut(), &word);
        } else {
            spelling.ignore(&word);
        }
        self.forget_marks();
        true
    }

    fn undo(&mut self) -> bool {
        let Some(board) = &mut self.board else { return false };
        let done = self.history.undo(board);
        self.dirty |= done;
        done
    }

    fn redo(&mut self) -> bool {
        let Some(board) = &mut self.board else { return false };
        let done = self.history.redo(board);
        self.dirty |= done;
        done
    }

    /// Runs a menu action and records the undo step of the change it makes. True when a frame is needed.
    fn run(&mut self, ctx: &Ctx, id: u32) -> bool {
        if self.confirm.is_some() {
            return false;
        }
        let changed = match id {
            MENU_UNDO => self.undo(),
            MENU_REDO => self.redo(),
            _ => {
                self.history.end_typing();
                let name = undo_name(id);
                let before = name.and_then(|_| self.board.clone());
                let changed = self.action(ctx, id);
                if let (Some(name), Some(before)) = (name, before) {
                    if self.board.as_ref() != Some(&before) {
                        self.history.push(name, before);
                    }
                }
                changed
            }
        };
        self.flush(ctx);
        changed
    }

    /// The main-menu action with `id`. Returns true when the board changed.
    fn action(&mut self, ctx: &Ctx, id: u32) -> bool {
        let now = Self::now();
        match id {
            MENU_NEW => self.new_note(),
            MENU_DELETE | MENU_CLOSE => {
                let Some(board) = &mut self.board else { return false };
                let Some(focus) = board.focus else { return false };
                let title = board.find(focus).map(|n| n.title()).unwrap_or_default();
                let has_text = board.find(focus).is_some_and(|n| !n.text.trim().is_empty());
                if has_text {
                    // a note with something in it asks before it goes
                    self.confirm = Some(Confirm::delete(focus, &title));
                    return true;
                }
                board.remove(focus);
                self.changed()
            }
            MENU_DUPLICATE => {
                let Some(board) = &mut self.board else { return false };
                let Some(focus) = board.focus else { return false };
                board.duplicate(focus, now).is_some() && self.changed()
            }
            MENU_COLLAPSE => {
                let Some(note) = self.board.as_mut().and_then(|b| b.focused_mut()) else { return false };
                note.collapsed = !note.collapsed;
                self.changed()
            }
            MENU_COLLAPSE_ALL | MENU_EXPAND_ALL => {
                let collapse = id == MENU_COLLAPSE_ALL;
                self.board.as_mut().is_some_and(|b| b.set_all_collapsed(collapse)) && self.changed()
            }
            MENU_BOLD => self.toggle_style(|s| s.bold, |s, v| s.bold = v),
            MENU_ITALIC => self.toggle_style(|s| s.italic, |s, v| s.italic = v),
            MENU_UNDERLINE => self.toggle_style(|s| s.underline, |s, v| s.underline = v),
            MENU_STRIKE => self.toggle_style(|s| s.strike, |s, v| s.strike = v),
            MENU_OUTLINE => self.toggle_style(|s| s.outline, |s, v| s.outline = v),
            MENU_BIGGER | MENU_SMALLER => {
                let Some(note) = self.board.as_mut().and_then(|b| b.focused_mut()) else { return false };
                let size = if id == MENU_BIGGER { note.style.size.bigger() } else { note.style.size.smaller() };
                if size == note.style.size {
                    return false;
                }
                note.restyle_whole(|s| s.size = size) && self.changed()
            }
            MENU_SIZE_BASE..=8 => {
                let size = [TextSize::Small, TextSize::Medium, TextSize::Large][(id - MENU_SIZE_BASE) as usize];
                let Some(note) = self.board.as_mut().and_then(|b| b.focused_mut()) else { return false };
                note.restyle_whole(|s| s.size = size) && self.changed()
            }
            MENU_INK_BASE..=64 => {
                let ink = Ink::ALL[(id - MENU_INK_BASE) as usize];
                self.restyle_focused(|s| s.ink = ink)
            }
            MENU_BASELINE_BASE..=82 => {
                let baseline = Baseline::ALL[(id - MENU_BASELINE_BASE) as usize];
                self.restyle_focused(|s| s.baseline = baseline)
            }
            MENU_KERN_BASE..=85 => {
                let kern = Setting::ALL[(id - MENU_KERN_BASE) as usize];
                self.restyle_focused(|s| s.kern = kern)
            }
            MENU_LIGATURE_BASE..=88 => {
                let ligature = Setting::ALL[(id - MENU_LIGATURE_BASE) as usize];
                self.restyle_focused(|s| s.ligature = ligature)
            }
            MENU_COPY_STYLE => {
                self.copy_style();
                false
            }
            MENU_PASTE_STYLE => self.paste_style() && self.changed(),
            MENU_USE_DEFAULT => self.board.as_mut().is_some_and(|b| b.use_as_default()) && self.changed(),
            MENU_COLOUR_BASE..=15 => {
                let colour = Colour::from_index((id - MENU_COLOUR_BASE) as usize);
                let Some(note) = self.board.as_mut().and_then(|b| b.focused_mut()) else { return false };
                note.set_preset(colour);
                self.changed()
            }
            MENU_COLOUR_CUSTOM => {
                let Some(note) = self.board.as_ref().and_then(|b| b.focused()) else { return false };
                let start = note.rgb.map_or_else(|| "#".to_string(), swatch::hex);
                self.find = None;
                self.replacing = false;
                self.link_entry = None;
                self.colour_entry = Some(start);
                true
            }
            MENU_TRANSLUCENT => {
                let Some(note) = self.board.as_mut().and_then(|b| b.focused_mut()) else { return false };
                note.translucent = !note.translucent;
                self.changed()
            }
            MENU_ZOOM => {
                let Some(note) = self.board.as_mut().and_then(|b| b.focused_mut()) else { return false };
                note.toggle_zoom();
                self.changed()
            }
            MENU_FLOAT_ON_TOP => {
                self.floating = !self.floating;
                windows::set_level(ctx, if self.floating { Level::Floating } else { Level::Normal });
                return true;
            }
            MENU_ARRANGE_BASE..=56 => {
                let by = Arrange::ALL[(id - MENU_ARRANGE_BASE) as usize];
                let width = Self::window_rect(ctx).w;
                self.board.as_mut().is_some_and(|b| b.arrange(by, width)) && self.changed()
            }
            MENU_CUT | MENU_COPY => {
                let Some(note) = self.board.as_mut().and_then(|b| b.focused_mut()) else { return false };
                if id == MENU_COPY {
                    // Copy takes the selection, or the whole note when nothing is selected
                    let text = if note.has_selection() { note.selected_text() } else { note.text.as_str() };
                    ctx.set_pasteboard_string(text);
                    return false;
                }
                match note.cut(now) {
                    Some(text) => {
                        ctx.set_pasteboard_string(&text);
                        self.changed()
                    }
                    None => false,
                }
            }
            MENU_PASTE | MENU_PASTE_PLAIN => {
                let Some(text) = ctx.pasteboard_string() else { return false };
                let Some(note) = self.board.as_mut().and_then(|b| b.focused_mut()) else { return false };
                note.type_text(&text, now);
                self.changed()
            }
            MENU_SELECT_ALL => {
                let Some(note) = self.board.as_mut().and_then(|b| b.focused_mut()) else { return false };
                note.select_all();
                true
            }
            MENU_UPPER | MENU_LOWER | MENU_CAPITALISE => {
                let case = match id {
                    MENU_UPPER => Case::Upper,
                    MENU_LOWER => Case::Lower,
                    _ => Case::Capitalised,
                };
                let Some(note) = self.board.as_mut().and_then(|b| b.focused_mut()) else { return false };
                note.transform(case, now) && self.changed()
            }
            MENU_FIND => {
                self.colour_entry = None;
                self.link_entry = None;
                if self.find.is_none() {
                    self.find = Some(self.last_find.clone());
                }
                true
            }
            MENU_FIND_NEXT | MENU_FIND_PREVIOUS => {
                self.step_find(id == MENU_FIND_NEXT);
                true
            }
            MENU_USE_SELECTION => {
                let selected = self.board.as_ref().and_then(|b| b.focused()).map(|n| n.selected_text().lines().next().unwrap_or("").to_string());
                match selected {
                    Some(text) if !text.is_empty() => {
                        self.last_find = text.clone();
                        if self.find.is_some() {
                            self.find = Some(text);
                        }
                        true
                    }
                    _ => false,
                }
            }
            MENU_REPLACE | MENU_REPLACE_ALL | MENU_REPLACE_IN_NOTE => {
                self.colour_entry = None;
                self.link_entry = None;
                let scope = match id {
                    MENU_REPLACE_ALL => Scope::AllNotes,
                    MENU_REPLACE_IN_NOTE => Scope::FocusedNote,
                    _ => Scope::Selection,
                };
                self.replace(scope)
            }
            MENU_IGNORE_CASE => {
                self.prefs.find_ignore_case = !self.prefs.find_ignore_case;
                let prefs = self.prefs;
                prefs.save(&mut ctx.defaults_mut());
                true
            }
            MENU_WHOLE_WORD => {
                self.prefs.find_whole_word = !self.prefs.find_whole_word;
                let prefs = self.prefs;
                prefs.save(&mut ctx.defaults_mut());
                true
            }
            MENU_ADD_LINK => {
                let open = self.board.as_ref().and_then(|b| b.focused()).is_some();
                if open {
                    self.find = None;
                    self.replacing = false;
                    self.colour_entry = None;
                    self.link_entry = Some(String::new());
                }
                open
            }
            MENU_SPEAK_START => {
                let Some(text) = self.board.as_ref().and_then(|b| b.focused()).and_then(speech::text_to_speak) else { return false };
                if let Err(speech::SpeakError::NoEngine) = self.speaker.start(&text) {
                    ctx.notify("Sticky Notes", "No speech engine is installed. Install espeak-ng to have notes read aloud.");
                }
                false
            }
            MENU_SPEAK_STOP => {
                self.speaker.stop();
                false
            }
            MENU_OPEN_LINK => {
                let url = self.board.as_ref().and_then(|b| b.focused()).and_then(|n| n.link_at(n.caret));
                if let Some(url) = url {
                    ctx.open_url(&url);
                }
                false
            }
            MENU_SPELL_WHILE_TYPING | MENU_SPELL_AUTO | MENU_SMART_QUOTES | MENU_SMART_DASHES | MENU_DETECT_DATES | MENU_DETECT_ADDRESSES | MENU_DETECT_PHONES => {
                self.toggle_pref(ctx, id)
            }
            MENU_SPELL_CHECK_NOW => self.check_document_now(ctx),
            MENU_SPELL_CORRECT => self.correct_spelling(ctx),
            MENU_SPELL_LEARN | MENU_SPELL_IGNORE => self.learn_or_ignore(ctx, id),
            MENU_PRINT => {
                self.print(ctx);
                false
            }
            MENU_PORTRAIT | MENU_LANDSCAPE => {
                self.page.orientation = if id == MENU_LANDSCAPE { Orientation::Landscape } else { Orientation::Portrait };
                let page = self.page.clone();
                page::save(&mut ctx.defaults_mut(), &page);
                false
            }
            MENU_PAPER_BASE..=109 => {
                if let Some(paper) = PAPERS.get((id - MENU_PAPER_BASE) as usize) {
                    self.page.paper = *paper;
                }
                let page = self.page.clone();
                page::save(&mut ctx.defaults_mut(), &page);
                false
            }
            MENU_IMPORT => self.import_panel(ctx),
            MENU_EXPORT => {
                let focus = self.board.as_ref().and_then(|b| b.focus);
                self.export_panel(ctx, focus)
            }
            MENU_EXPORT_ALL => self.export_panel(ctx, None),
            _ => false,
        }
    }

    fn file_menu(&self, has_focus: bool) -> AppMenu {
        let any = self.board.as_ref().is_some_and(|b| !b.notes.is_empty());
        let mut items = vec![
            AppMenuItem::new(MENU_NEW, "New Sticky Note").key("cmd+n"),
            AppMenuItem::new(MENU_DUPLICATE, "Duplicate Sticky Note").key("cmd+d").enabled(has_focus),
            AppMenuItem::new(MENU_DELETE, "Delete Sticky Note").key("cmd+shift+backspace").enabled(has_focus),
            // Cmd+W is the front note's: the shell sends the chord here while a note is focused (`app_owns_close`)
            AppMenuItem::new(MENU_CLOSE, "Close").key("cmd+w").enabled(has_focus),
            AppMenuItem::separator(),
            AppMenuItem::new(MENU_IMPORT, "Import Text...").key("cmd+o"),
            AppMenuItem::new(MENU_EXPORT, "Export Sticky Note...").enabled(has_focus),
            AppMenuItem::new(MENU_EXPORT_ALL, "Export All Sticky Notes...").enabled(any),
            AppMenuItem::separator(),
            AppMenuItem::new(MENU_PRINT, "Print...").key("cmd+p").enabled(has_focus),
            AppMenuItem::separator(),
            AppMenuItem::submenu("Paper Size"),
        ];
        for (i, paper) in PAPERS.iter().enumerate() {
            let id = MENU_PAPER_BASE + i as u32;
            items.push(AppMenuItem::new(id, paper.media()).checked(self.page.paper == *paper).depth(1));
        }
        items.push(AppMenuItem::submenu("Orientation"));
        let portrait = self.page.orientation == Orientation::Portrait;
        items.push(AppMenuItem::new(MENU_PORTRAIT, "Portrait").checked(portrait).depth(1));
        items.push(AppMenuItem::new(MENU_LANDSCAPE, "Landscape").checked(!portrait).depth(1));
        AppMenu { title: "File".into(), items }
    }

    fn edit_menu(&self, ctx: &Ctx) -> AppMenu {
        let note = self.board.as_ref().and_then(|b| b.focused());
        let has_focus = note.is_some();
        let has_selection = note.is_some_and(|n| n.has_selection());
        let has_link = note.is_some_and(|n| n.link_at(n.caret).is_some());
        let can_paste = has_focus && ctx.pasteboard_string().is_some();
        let has_find = !self.last_find.is_empty() || self.find.as_ref().is_some_and(|q| !q.is_empty());
        let any = self.board.as_ref().is_some_and(|b| !b.notes.is_empty());
        let can_speak = note.and_then(speech::text_to_speak).is_some();
        let can_replace = any && has_find;
        let has_word = note.is_some_and(|n| spell::word_at(&n.text, n.caret).is_some());
        let p = self.prefs;
        AppMenu {
            title: "Text".into(),
            items: vec![
                AppMenuItem::new(MENU_PASTE_PLAIN, "Paste and Match Style").key("cmd+alt+shift+v").enabled(can_paste),
                AppMenuItem::separator(),
                AppMenuItem::new(MENU_FIND, "Find...").key("cmd+f").enabled(any),
                AppMenuItem::new(MENU_FIND_NEXT, "Find Next").key("cmd+g").enabled(any && has_find),
                AppMenuItem::new(MENU_FIND_PREVIOUS, "Find Previous").key("cmd+shift+g").enabled(any && has_find),
                AppMenuItem::new(MENU_USE_SELECTION, "Use Selection for Find").key("cmd+e").enabled(has_selection),
                AppMenuItem::new(MENU_REPLACE, "Replace").enabled(can_replace),
                AppMenuItem::new(MENU_REPLACE_ALL, "Replace All").enabled(can_replace),
                AppMenuItem::new(MENU_REPLACE_IN_NOTE, "Replace All in Current Note").enabled(can_replace && has_focus),
                AppMenuItem::new(MENU_IGNORE_CASE, "Ignore Case").checked(p.find_ignore_case),
                AppMenuItem::new(MENU_WHOLE_WORD, "Whole Word").checked(p.find_whole_word),
                AppMenuItem::separator(),
                AppMenuItem::new(MENU_ADD_LINK, "Add Link...").key("cmd+k").enabled(has_focus),
                AppMenuItem::new(MENU_OPEN_LINK, "Open Link").enabled(has_link),
                AppMenuItem::separator(),
                AppMenuItem::submenu("Spelling and Grammar"),
                AppMenuItem::new(MENU_SPELL_WHILE_TYPING, "Check Spelling While Typing").checked(p.check_while_typing).depth(1),
                AppMenuItem::new(MENU_SPELL_CHECK_NOW, "Check Document Now").key("cmd+;").enabled(any).depth(1),
                AppMenuItem::new(MENU_SPELL_AUTO, "Correct Spelling Automatically").checked(p.correct_automatically).depth(1),
                AppMenuItem::separator().depth(1),
                AppMenuItem::new(MENU_SPELL_CORRECT, "Correct Spelling").enabled(has_word).depth(1),
                AppMenuItem::new(MENU_SPELL_LEARN, "Learn Spelling").enabled(has_word).depth(1),
                AppMenuItem::new(MENU_SPELL_IGNORE, "Ignore Spelling").enabled(has_word).depth(1),
                AppMenuItem::submenu("Substitutions"),
                AppMenuItem::new(MENU_SMART_QUOTES, "Smart Quotes").checked(p.smart_quotes).depth(1),
                AppMenuItem::new(MENU_SMART_DASHES, "Smart Dashes").checked(p.smart_dashes).depth(1),
                AppMenuItem::submenu("Data Detectors"),
                AppMenuItem::new(MENU_DETECT_DATES, "Dates").checked(p.detect_dates).depth(1),
                AppMenuItem::new(MENU_DETECT_ADDRESSES, "Addresses").checked(p.detect_addresses).depth(1),
                AppMenuItem::new(MENU_DETECT_PHONES, "Phone Numbers").checked(p.detect_phone_numbers).depth(1),
                AppMenuItem::separator(),
                AppMenuItem::submenu("Transformations"),
                AppMenuItem::new(MENU_UPPER, "Make Upper Case").enabled(has_selection).depth(1),
                AppMenuItem::new(MENU_LOWER, "Make Lower Case").enabled(has_selection).depth(1),
                AppMenuItem::new(MENU_CAPITALISE, "Capitalise").enabled(has_selection).depth(1),
                AppMenuItem::submenu("Speech"),
                AppMenuItem::new(MENU_SPEAK_START, "Start Speaking").enabled(can_speak).depth(1),
                AppMenuItem::new(MENU_SPEAK_STOP, "Stop Speaking").enabled(self.speaker.is_speaking()).depth(1),
            ],
        }
    }

    fn font_menu(&self, has_focus: bool) -> AppMenu {
        // the checks show the style at the caret, which is what the next typing takes
        let style = self.board.as_ref().and_then(|b| b.focused()).map(|n| n.current_style());
        let on = |f: fn(&Style) -> bool| style.as_ref().is_some_and(f);
        let mut items = vec![
            AppMenuItem::new(MENU_BOLD, "Bold").key("cmd+b").enabled(has_focus).checked(on(|s| s.bold)),
            AppMenuItem::new(MENU_ITALIC, "Italic").key("cmd+i").enabled(has_focus).checked(on(|s| s.italic)),
            AppMenuItem::new(MENU_UNDERLINE, "Underline").key("cmd+u").enabled(has_focus).checked(on(|s| s.underline)),
            AppMenuItem::new(MENU_STRIKE, "Strikethrough").enabled(has_focus).checked(on(|s| s.strike)),
            AppMenuItem::new(MENU_OUTLINE, "Outline").enabled(has_focus).checked(on(|s| s.outline)),
            AppMenuItem::separator(),
            AppMenuItem::submenu("Kern"),
        ];
        for (i, kern) in Setting::ALL.iter().enumerate() {
            items.push(AppMenuItem::new(MENU_KERN_BASE + i as u32, kern.title()).enabled(has_focus).checked(style.is_some_and(|s| s.kern == *kern)).depth(1));
        }
        items.push(AppMenuItem::submenu("Ligature"));
        for (i, ligature) in Setting::ALL.iter().enumerate() {
            items.push(AppMenuItem::new(MENU_LIGATURE_BASE + i as u32, ligature.title()).enabled(has_focus).checked(style.is_some_and(|s| s.ligature == *ligature)).depth(1));
        }
        items.push(AppMenuItem::submenu("Baseline"));
        for (i, baseline) in Baseline::ALL.iter().enumerate() {
            items.push(AppMenuItem::new(MENU_BASELINE_BASE + i as u32, baseline.title()).enabled(has_focus).checked(style.is_some_and(|s| s.baseline == *baseline)).depth(1));
        }
        items.push(AppMenuItem::separator());
        items.push(AppMenuItem::new(MENU_BIGGER, "Bigger").key("cmd+=").enabled(style.is_some_and(|s| s.size != TextSize::Large)));
        items.push(AppMenuItem::new(MENU_SMALLER, "Smaller").key("cmd+-").enabled(style.is_some_and(|s| s.size != TextSize::Small)));
        for (i, (size, title)) in [(TextSize::Small, "Small Text"), (TextSize::Medium, "Medium Text"), (TextSize::Large, "Large Text")].into_iter().enumerate() {
            items.push(AppMenuItem::new(MENU_SIZE_BASE + i as u32, title).enabled(has_focus).checked(style.is_some_and(|s| s.size == size)));
        }
        items.push(AppMenuItem::separator());
        items.push(AppMenuItem::submenu("Text Colour"));
        for (i, ink) in Ink::ALL.iter().enumerate() {
            items.push(AppMenuItem::new(MENU_INK_BASE + i as u32, ink.name()).enabled(has_focus).checked(style.is_some_and(|s| s.ink == *ink)).depth(1));
        }
        items.push(AppMenuItem::separator());
        items.push(AppMenuItem::new(MENU_COPY_STYLE, "Copy Style").key("cmd+alt+c").enabled(has_focus));
        items.push(AppMenuItem::new(MENU_PASTE_STYLE, "Paste Style").key("cmd+alt+v").enabled(has_focus && self.style_clip.is_some()));
        items.push(AppMenuItem::separator());
        items.push(AppMenuItem::new(MENU_USE_DEFAULT, "Use as Default").enabled(has_focus));
        AppMenu { title: "Font".into(), items }
    }

    fn colour_menu(&self, has_focus: bool) -> AppMenu {
        let note = self.board.as_ref().and_then(|b| b.focused());
        // a custom colour replaces the preset, so no preset is checked then
        let colour = note.filter(|n| n.rgb.is_none()).map(|n| n.colour);
        let mut items: Vec<AppMenuItem> = Colour::ALL
            .iter()
            .enumerate()
            .map(|(i, c)| AppMenuItem::new(MENU_COLOUR_BASE + i as u32, c.name()).enabled(has_focus).checked(colour == Some(*c)))
            .collect();
        items.push(AppMenuItem::separator());
        items.push(AppMenuItem::new(MENU_COLOUR_CUSTOM, "Custom Colour...").enabled(has_focus).checked(note.is_some_and(|n| n.rgb.is_some())));
        AppMenu { title: "Colour".into(), items }
    }

    fn window_menu(&self) -> AppMenu {
        let note = self.board.as_ref().and_then(|b| b.focused());
        let has_focus = note.is_some();
        let any = self.board.as_ref().is_some_and(|b| !b.notes.is_empty());
        let mut items = vec![
            AppMenuItem::new(MENU_COLLAPSE, if note.is_some_and(|n| n.collapsed) { "Expand Sticky Note" } else { "Collapse Sticky Note" }).enabled(has_focus),
            AppMenuItem::new(MENU_COLLAPSE_ALL, "Collapse All").enabled(any),
            AppMenuItem::new(MENU_EXPAND_ALL, "Expand All").enabled(any),
            AppMenuItem::new(MENU_TRANSLUCENT, "Translucent").enabled(has_focus).checked(note.is_some_and(|n| n.translucent)),
            AppMenuItem::new(MENU_ZOOM, "Zoom Sticky Note").enabled(has_focus).checked(note.is_some_and(|n| n.restore.is_some())),
            AppMenuItem::new(MENU_FLOAT_ON_TOP, "Float on Top").checked(self.floating),
            AppMenuItem::separator(),
            AppMenuItem::submenu("Arrange By"),
        ];
        for (i, by) in Arrange::ALL.iter().enumerate() {
            items.push(AppMenuItem::new(MENU_ARRANGE_BASE + i as u32, by.title()).enabled(any).depth(1));
        }
        if let Some(n) = note {
            // the note's dates, as disabled rows (the reference shows them as a tooltip)
            items.push(AppMenuItem::separator());
            items.push(AppMenuItem::new(MENU_INFO_CREATED, &format!("Created {}", dates::format(n.created))).enabled(false));
            items.push(AppMenuItem::new(MENU_INFO_MODIFIED, &format!("Modified {}", dates::format(n.modified))).enabled(false));
        }
        AppMenu { title: "Window".into(), items }
    }
}

impl App for StickiesApp {
    fn id(&self) -> &str {
        "is.olibuijr.stickies.desktop"
    }

    fn configure(&mut self, ctx: &Ctx) {
        if self.board.is_none() {
            let first = store::first_launch(&ctx.data_dir);
            let mut board = store::load(&ctx.data_dir);
            if first && board.notes.is_empty() {
                let (x, y) = Self::cascade_position(&board);
                board.add_text(WELCOME_TEXT, x, y, Self::now());
                self.dirty = true;
            }
            self.board = Some(board);
        }
        self.prefs = Prefs::load(&ctx.defaults());
        self.page = page::load(&ctx.defaults());
    }

    /// The Make New Sticky Note service of the Services menu sends its selection here.
    fn intents(&self) -> Vec<Intent> {
        vec![Intent::new("create-sticky", "Make New Sticky Note", "Creates a sticky note holding the given text.").param(Param::new("text", "Text", ParamType::String))]
    }

    fn perform_intent(&mut self, ctx: Option<&Ctx>, data_dir: &std::path::Path, id: &str, params: &Json) -> Result<IntentResult, RpcError> {
        match id {
            "create-sticky" => {
                let text = intents::str_param(params, "text")?;
                self.create_sticky(data_dir, text).map_err(intents::failed)?;
                if let Some(ctx) = ctx {
                    self.flush(ctx);
                }
                Ok(IntentResult::dialog("Created a sticky note"))
            }
            _ => Err(RpcError::new(bifrost_app::control::METHOD_NOT_FOUND, format!("no intent {id}"))),
        }
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        let changed = match ev {
            Input::Touch { kind, x, y, t, .. } => self.touch(ctx, kind, x, y, t),
            Input::Key(key) => self.key(ctx, &key),
            Input::Command(command) => {
                let id = match command {
                    Command::Cut => MENU_CUT,
                    Command::Copy => MENU_COPY,
                    Command::Paste => MENU_PASTE,
                    Command::SelectAll => MENU_SELECT_ALL,
                    Command::Undo => MENU_UNDO,
                    Command::Redo => MENU_REDO,
                };
                self.run(ctx, id)
            }
            _ => false,
        };
        self.flush(ctx);
        changed
    }

    /// Cut needs a selection; Copy, Paste and Select All need a focused note.
    fn commands(&self, ctx: &Ctx) -> Commands {
        let note = self.board.as_ref().and_then(|b| b.focused());
        Commands::default()
            .with(Command::Cut, note.is_some_and(|n| n.has_selection()))
            .with(Command::Copy, note.is_some())
            .with(Command::Paste, note.is_some() && ctx.pasteboard_string().is_some())
            .with(Command::SelectAll, note.is_some())
            .with(Command::Undo, self.history.can_undo())
            .with(Command::Redo, self.history.can_redo())
    }

    fn undo_titles(&self, _ctx: &Ctx) -> (String, String) {
        (self.history.undo_title(), self.history.redo_title())
    }

    fn main_menu(&self, ctx: &Ctx) -> Vec<AppMenu> {
        let has_focus = self.board.as_ref().and_then(|b| b.focused()).is_some();
        vec![self.file_menu(has_focus), self.edit_menu(ctx), self.font_menu(has_focus), self.colour_menu(has_focus), self.window_menu()]
    }

    fn menu_action(&mut self, ctx: &Ctx, id: u32) -> bool {
        self.run(ctx, id)
    }

    fn document(&mut self, ctx: &Ctx, ev: DocumentEvent) -> bool {
        match ev {
            DocumentEvent::Opened(mut doc) if self.import == Some(doc.request) => {
                self.import = None;
                let mut bytes = Vec::new();
                if let Err(e) = (&mut doc.file).take(model::IMPORT_MAX).read_to_end(&mut bytes) {
                    ctx.log(&format!("import: {e}"));
                    return false;
                }
                let text = String::from_utf8_lossy(&bytes).into_owned();
                let before = self.board.clone();
                let rich = bytes.starts_with(b"{\\rtf");
                let ids = match &mut self.board {
                    // a rich text file becomes one note that keeps its bold, italic, underline and strikethrough
                    Some(board) if rich => {
                        let runs = model::runs_from_rtf(&text);
                        if runs.iter().all(|(t, _)| t.trim().is_empty()) {
                            Vec::new()
                        } else {
                            vec![board.add_runs(&runs, 40.0, 60.0, Self::now())]
                        }
                    }
                    Some(board) => board.import_text(&text, Self::now()),
                    None => Vec::new(),
                };
                // one import is one use of the file: the app keeps no grant to it
                ctx.release_document(doc.grant);
                if ids.is_empty() {
                    return false;
                }
                if let Some(before) = before {
                    self.history.push("Import", before);
                }
                self.changed();
                self.flush(ctx);
                return true;
            }
            DocumentEvent::SaveTo(mut doc) => match self.export.take() {
                Some((request, bytes)) if request == doc.request => match doc.file.write_all(&bytes) {
                    Ok(()) => ctx.commit_document(request),
                    Err(e) => {
                        ctx.log(&format!("export: {e}"));
                        ctx.abort_document(request);
                    }
                },
                other => self.export = other,
            },
            DocumentEvent::Failed { request, code } => {
                if self.export.as_ref().is_some_and(|(r, _)| *r == request) {
                    self.export = None;
                }
                if self.import == Some(request) {
                    self.import = None;
                }
                if code != DocErrorCode::Cancelled {
                    ctx.log(&format!("document: {code:?}"));
                }
            }
            _ => {}
        }
        false
    }

    fn suspend(&mut self, ctx: &Ctx) {
        self.flush(ctx);
    }

    fn draws_canvas(&self) -> bool {
        true
    }

    fn draw_canvas(&mut self, ctx: &Ctx, canvas: &mut dyn Canvas, _t: f64) -> Damage {
        let (w, h) = canvas.size();
        self.refresh_marks(ctx);
        if let Some(board) = &self.board {
            board::draw(canvas, &ctx.ui, board, &self.marks);
            if let Some(query) = &self.find {
                let found = board.matches(query, self.prefs.find_ignore_case, self.prefs.find_whole_word).len();
                board::draw_find(canvas, &ctx.ui, query, &self.replace, self.replacing, found);
            }
            if let Some(entry) = &self.colour_entry {
                board::draw_colour_entry(canvas, &ctx.ui, entry);
            }
            if let Some(entry) = &self.link_entry {
                board::draw_link_entry(canvas, &ctx.ui, entry);
            }
        }
        if let Some(confirm) = &self.confirm {
            let scale = ctx.ui.pt(1.0).max(0.001);
            let parent = Rect { x: 0.0, y: 0.0, w: w as f32 / scale, h: h as f32 / scale };
            widgets::sheet(canvas, &ctx.ui, parent, UNIFIED_BAR_PT, &confirm.sheet, 1.0, None);
        }
        let mut d = Damage::new(w as usize, h as usize);
        d.all();
        d
    }

    /// The shared-memory path: the same routine on the CPU `Surface`, which implements `Canvas`.
    fn draw(&mut self, ctx: &Ctx, s: &mut Surface, t: f64) -> Damage {
        self.draw_canvas(ctx, s, t)
    }
}

fn main() {
    bifrost_app::run(StickiesApp::new());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app_with(texts: &[&str]) -> StickiesApp {
        let mut app = StickiesApp::new();
        let mut board = Board::default();
        for (i, t) in texts.iter().enumerate() {
            board.add_text(t, 40.0 + 30.0 * i as f32, 60.0, 1);
        }
        app.board = Some(board);
        app
    }

    fn ctx_in(dir: &std::path::Path) -> Ctx {
        use bifrost_app::kit::{Font, Ui};
        use bifrost_app::proto::Configure;
        let assets = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets");
        let data: &'static [u8] = Box::leak(std::fs::read(format!("{assets}/SFUI.ttf")).unwrap().into_boxed_slice());
        let cfg = Configure { w: 800, h: 600, scale: 1.0, safe_top: 0.0, safe_bottom: 0.0, dark: true, text_scale: 1.0, reduce_motion: false };
        let mut ctx = Ctx::testing(Ui::new(Font::parse(data).unwrap(), 1.0), cfg);
        ctx.data_dir = dir.to_path_buf();
        ctx
    }

    #[test]
    fn the_first_launch_opens_a_welcome_note_and_a_later_empty_board_stays_empty() {
        let dir = std::env::temp_dir().join(format!("stickies-first-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut app = StickiesApp::new();
        app.configure(&ctx_in(&dir));
        let board = app.board.as_ref().unwrap();
        assert_eq!(board.notes.len(), 1, "a first note is open");
        assert!(board.notes[0].text.starts_with("Make a note of it!"));
        assert!(app.dirty, "and it is saved");
        store::save(&dir, &Board::default()).unwrap();
        let mut again = StickiesApp::new();
        again.configure(&ctx_in(&dir));
        assert!(again.board.as_ref().unwrap().notes.is_empty(), "a board the person emptied is not refilled");
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn focused(app: &StickiesApp) -> &Note {
        app.board.as_ref().unwrap().focused().unwrap()
    }

    fn type_chars(app: &mut StickiesApp, text: &str, quotes: bool, dashes: bool) {
        app.prefs = Prefs { smart_quotes: quotes, smart_dashes: dashes, ..Prefs::default() };
        for c in text.chars() {
            app.typed(&c.to_string());
        }
    }

    #[test]
    fn every_mutating_menu_id_has_an_undo_name_and_the_others_do_not() {
        for id in [MENU_NEW, MENU_DELETE, MENU_CLOSE, MENU_DUPLICATE, MENU_BOLD, MENU_CUT, MENU_PASTE, MENU_ZOOM, MENU_UPPER, 7, 12, 62, 54, MENU_OUTLINE, 80, 85, 88, MENU_SPELL_CORRECT] {
            assert!(undo_name(id).is_some(), "id {id}");
        }
        for id in [MENU_COPY, MENU_SELECT_ALL, MENU_FIND, MENU_PRINT, MENU_IMPORT, MENU_OPEN_LINK, MENU_UNDO, MENU_COPY_STYLE, MENU_SPELL_CHECK_NOW, MENU_SMART_QUOTES, MENU_ADD_LINK, MENU_FLOAT_ON_TOP] {
            assert!(undo_name(id).is_none(), "id {id}");
        }
        assert_eq!(undo_name(MENU_ARRANGE_BASE), Some("Arrange"));
    }

    #[test]
    fn the_new_menu_ids_do_not_clash_with_the_old_ones() {
        let new = [MENU_OUTLINE, MENU_BASELINE_BASE, MENU_KERN_BASE, MENU_LIGATURE_BASE, MENU_ADD_LINK, MENU_SPELL_WHILE_TYPING, MENU_SPELL_CHECK_NOW, MENU_SPELL_AUTO, MENU_SPELL_CORRECT, MENU_SPELL_LEARN, MENU_SPELL_IGNORE, MENU_SMART_QUOTES, MENU_SMART_DASHES, MENU_DETECT_DATES, MENU_DETECT_ADDRESSES, MENU_DETECT_PHONES];
        for (i, a) in new.iter().enumerate() {
            assert!(!new[i + 1..].contains(a));
            assert!(!(MENU_COLOUR_BASE..=15).contains(a) && !(MENU_ARRANGE_BASE..=56).contains(a) && !(MENU_INK_BASE..=64).contains(a));
            assert!(!(MENU_SIZE_BASE..=8).contains(a) && *a != MENU_REPLACE_ALL && *a != MENU_UNDO && *a != MENU_TRANSLUCENT);
        }
        assert_eq!(MENU_BASELINE_BASE + 2, MENU_KERN_BASE - 1, "three baselines fit before the kern items");
        assert_eq!(MENU_KERN_BASE + 2, MENU_LIGATURE_BASE - 1, "three kern settings fit before the ligature items");
        assert_eq!(MENU_LIGATURE_BASE + 2, MENU_ADD_LINK - 1, "three ligature settings fit before Add Link");
    }

    /// W2-22: Cmd+W (File > Close) closes the front note, not the board: an empty note goes at once, one with text asks
    /// first; the item is enabled (so the shell hands it the chord) only while a note is focused.
    #[test]
    fn cmd_w_closes_the_front_note_and_asks_first_when_it_has_text() {
        let dir = std::env::temp_dir().join(format!("stickies-close-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let ctx = ctx_in(&dir);
        let mut app = app_with(&["keep me", "", "other"]);
        let file = app.file_menu(true);
        let close = file.items.iter().find(|i| i.id == MENU_CLOSE).expect("File has Close");
        assert_eq!((close.key.as_str(), close.enabled), ("cmd+w", true));
        assert!(!app.file_menu(false).items.iter().any(|i| i.id == MENU_CLOSE && i.enabled), "no focused note: Cmd+W is the shell's");
        let empty = app.board.as_ref().unwrap().notes[1].id;
        app.board.as_mut().unwrap().focus = Some(empty);
        assert!(app.run(&ctx, MENU_CLOSE));
        assert!(app.board.as_ref().unwrap().find(empty).is_none(), "the empty front note is gone");
        assert_eq!(app.board.as_ref().unwrap().notes.len(), 2, "the other notes stay");
        assert!(app.undo(), "and it can be undone");
        let kept = app.board.as_ref().unwrap().notes[0].id;
        app.board.as_mut().unwrap().focus = Some(kept);
        assert!(app.run(&ctx, MENU_CLOSE));
        assert_eq!(app.confirm.as_ref().map(|c| c.note), Some(kept), "a note with text asks first");
        assert_eq!(app.board.as_ref().unwrap().notes.len(), 3);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_confirmation_names_the_note_and_cancel_is_the_escape_button() {
        let c = Confirm::delete(3, "Groceries");
        assert!(c.sheet.message.contains("\"Groceries\""));
        assert_eq!(c.sheet.cancel_button(), Some(SHEET_CANCEL));
        assert!(Confirm::delete(3, "").sheet.message.contains("this note"));
    }

    #[test]
    fn answering_delete_removes_the_note_and_can_be_undone() {
        let mut app = app_with(&["keep", "drop"]);
        let drop = app.board.as_ref().unwrap().notes[1].id;
        app.confirm = Some(Confirm::delete(drop, "drop"));
        assert!(app.answer(SHEET_DELETE));
        assert_eq!(app.board.as_ref().unwrap().notes.len(), 1);
        assert!(app.confirm.is_none() && app.dirty);
        assert!(app.undo());
        assert_eq!(app.board.as_ref().unwrap().notes.len(), 2);
        assert!(app.redo());
        assert_eq!(app.board.as_ref().unwrap().notes.len(), 1);
    }

    #[test]
    fn answering_cancel_keeps_the_note() {
        let mut app = app_with(&["keep"]);
        let id = app.board.as_ref().unwrap().notes[0].id;
        app.confirm = Some(Confirm::delete(id, "keep"));
        assert!(app.answer(SHEET_CANCEL));
        assert_eq!(app.board.as_ref().unwrap().notes.len(), 1);
        assert!(!app.history.can_undo());
        assert!(!app.answer(SHEET_CANCEL), "nothing is open any more");
    }

    #[test]
    fn find_keys_edit_the_find_text_and_select_the_first_match() {
        let mut app = app_with(&["alpha beta", "gamma beta"]);
        app.find = Some(String::new());
        for c in "bet".chars() {
            assert!(app.find_key(&KeyInput::Text(c.to_string())));
        }
        assert_eq!(app.find.as_deref(), Some("bet"));
        assert_eq!(app.last_find, "bet");
        let board = app.board.as_ref().unwrap();
        let note = board.focused().unwrap();
        assert_eq!(note.selected_text(), "bet");
        // Return steps to the next match, which is in the other note
        let first = note.id;
        assert!(app.find_key(&KeyInput::Code(13)));
        assert_ne!(app.board.as_ref().unwrap().focus, Some(first));
        // Backspace shortens the text, Escape closes the bar but keeps the text for Find Next
        assert!(app.find_key(&KeyInput::Code(8)));
        assert_eq!(app.find.as_deref(), Some("be"));
        assert!(app.find_key(&KeyInput::Code(27)));
        assert!(app.find.is_none());
        assert_eq!(app.last_find, "be");
    }

    #[test]
    fn find_next_without_text_opens_the_bar() {
        let mut app = app_with(&["x"]);
        app.step_find(true);
        assert_eq!(app.find.as_deref(), Some(""));
    }

    #[test]
    fn copy_style_and_paste_style_move_a_style_between_notes_and_keep_the_size() {
        let mut app = app_with(&["one", "two"]);
        let (first, second) = {
            let b = app.board.as_ref().unwrap();
            (b.notes[0].id, b.notes[1].id)
        };
        assert!(!app.paste_style(), "nothing copied yet");
        {
            let b = app.board.as_mut().unwrap();
            b.find_mut(first).unwrap().restyle_whole(|s| {
                s.bold = true;
                s.underline = true;
                s.size = TextSize::Large;
            });
            b.raise(first);
        }
        app.copy_style();
        assert_eq!(app.style_clip.map(|s| s.bold), Some(true));
        app.board.as_mut().unwrap().raise(second);
        assert!(app.paste_style());
        let pasted = app.board.as_ref().unwrap().find(second).unwrap();
        assert!(pasted.current_style().underline);
        assert_eq!(pasted.style.size, TextSize::Medium, "the size is the note's own");
        assert!(!app.paste_style(), "the style is already there");
        assert_eq!(undo_name(MENU_PASTE_STYLE), Some("Style"));
    }

    #[test]
    fn the_colour_bar_applies_a_typed_colour_as_one_undo_step() {
        let mut app = app_with(&["note"]);
        app.colour_entry = Some("#".into());
        for c in "0a84ff".chars() {
            assert!(app.colour_key(&KeyInput::Text(c.to_string())));
        }
        assert_eq!(app.colour_entry.as_deref(), Some("#0A84FF"));
        assert!(!app.colour_key(&KeyInput::Text("a".into())), "a seventh digit");
        assert!(app.colour_key(&KeyInput::Code(13)));
        assert!(app.colour_entry.is_none());
        assert_eq!(app.board.as_ref().unwrap().focused().unwrap().rgb, Some((10, 132, 255)));
        assert_eq!(app.history.undo_title(), "Undo Colour");
        assert!(app.undo());
        assert_eq!(app.board.as_ref().unwrap().focused().unwrap().rgb, None);
    }

    #[test]
    fn the_colour_bar_refuses_an_unfinished_colour_and_escape_closes_it() {
        let mut app = app_with(&["note"]);
        app.colour_entry = Some("#12".into());
        assert!(!app.colour_key(&KeyInput::Code(13)));
        assert!(app.colour_entry.is_some());
        assert!(app.colour_key(&KeyInput::Code(8)));
        assert_eq!(app.colour_entry.as_deref(), Some("#1"));
        assert!(app.colour_key(&KeyInput::Code(27)));
        assert!(app.colour_entry.is_none());
        assert_eq!(app.board.as_ref().unwrap().focused().unwrap().rgb, None);
        assert!(!app.history.can_undo());
    }

    #[test]
    fn choosing_a_preset_colour_drops_the_custom_one() {
        let mut app = app_with(&["note"]);
        assert!(app.apply_custom((1, 2, 3)));
        assert!(!app.apply_custom((1, 2, 3)), "the same colour again");
        let note = app.board.as_mut().unwrap().focused_mut().unwrap();
        note.set_preset(Colour::Green);
        assert_eq!((note.colour, note.rgb), (Colour::Green, None));
    }

    #[test]
    fn tab_moves_to_the_replace_field_and_return_replaces_the_match_and_steps_on() {
        let mut app = app_with(&["foo one", "foo two"]);
        let first = app.board.as_ref().unwrap().notes[0].id;
        app.find = Some(String::new());
        for c in "foo".chars() {
            assert!(app.find_key(&KeyInput::Text(c.to_string())));
        }
        assert!(app.find_key(&KeyInput::Code(9)));
        assert!(app.replacing);
        for c in "bar".chars() {
            assert!(app.find_key(&KeyInput::Text(c.to_string())));
        }
        assert_eq!(app.replace, "bar");
        assert_eq!(app.find.as_deref(), Some("foo"), "typing went to the replace field");
        assert!(app.find_key(&KeyInput::Code(13)));
        let board = app.board.as_ref().unwrap();
        assert_eq!(board.find(first).unwrap().text, "bar one");
        assert_eq!(board.focused().unwrap().text, "foo two", "the next match is selected");
        assert_eq!(board.focused().unwrap().selected_text(), "foo");
        assert!(app.dirty);
        assert_eq!(app.history.undo_title(), "Undo Replace");
        assert!(app.undo());
        assert_eq!(app.board.as_ref().unwrap().find(first).unwrap().text, "foo one");
        // Tab goes back to the find field, and Escape from either closes the bar
        assert!(app.find_key(&KeyInput::Code(9)));
        assert!(!app.replacing);
        assert!(app.find_key(&KeyInput::Code(9)));
        assert!(app.find_key(&KeyInput::Code(27)));
        assert!(app.find.is_none() && !app.replacing);
    }

    #[test]
    fn replace_all_changes_every_note_in_one_undo_step() {
        let mut app = app_with(&["foo one", "Foo two"]);
        app.find = Some("foo".into());
        app.replace = "baz".into();
        assert!(app.replace(Scope::AllNotes));
        let texts: Vec<&str> = app.board.as_ref().unwrap().notes.iter().map(|n| n.text.as_str()).collect();
        assert_eq!(texts, vec!["baz one", "baz two"]);
        assert!(app.undo());
        let texts: Vec<&str> = app.board.as_ref().unwrap().notes.iter().map(|n| n.text.as_str()).collect();
        assert_eq!(texts, vec!["foo one", "Foo two"]);
        assert!(!app.history.can_undo(), "one step");
    }

    #[test]
    fn replace_all_in_the_focused_note_leaves_the_other_notes_alone() {
        let mut app = app_with(&["foo one", "foo two"]);
        let first = app.board.as_ref().unwrap().notes[0].id;
        app.board.as_mut().unwrap().focus = Some(first);
        app.find = Some("foo".into());
        app.replace = "baz".into();
        assert!(app.replace(Scope::FocusedNote));
        let texts: Vec<&str> = app.board.as_ref().unwrap().notes.iter().map(|n| n.text.as_str()).collect();
        assert_eq!(texts, vec!["baz one", "foo two"]);
        assert!(app.undo(), "one undo step");
        let texts: Vec<&str> = app.board.as_ref().unwrap().notes.iter().map(|n| n.text.as_str()).collect();
        assert_eq!(texts, vec!["foo one", "foo two"]);
    }

    #[test]
    fn page_setup_lists_each_paper_size_once() {
        let names: Vec<&str> = PAPERS.iter().map(|p| p.media()).collect();
        assert_eq!(names.len(), 5);
        for (i, a) in names.iter().enumerate() {
            assert!(!names[i + 1..].contains(a), "{a} is listed twice");
        }
        assert_eq!(PrintInfo::default().paper, PAPERS[0], "the first paper in the menu is the default");
    }

    #[test]
    fn replace_in_the_focused_note_without_focus_changes_nothing() {
        let mut app = app_with(&["foo"]);
        app.board.as_mut().unwrap().focus = None;
        app.find = Some("foo".into());
        app.replace = "baz".into();
        assert!(app.replace(Scope::FocusedNote));
        assert_eq!(app.board.as_ref().unwrap().notes[0].text, "foo");
        assert!(!app.history.can_undo());
    }

    #[test]
    fn replace_with_nothing_found_makes_no_undo_step_and_without_find_text_opens_the_bar() {
        let mut app = app_with(&["alpha"]);
        app.find = Some("zzz".into());
        assert!(app.replace(Scope::AllNotes));
        assert!(!app.history.can_undo());
        app.find = None;
        assert!(app.replace(Scope::Selection));
        assert_eq!(app.find.as_deref(), Some(""), "no find text: the bar opens");
        assert!(!app.history.can_undo());
    }

    #[test]
    fn the_make_sticky_service_adds_a_note_and_can_be_undone() {
        let mut app = app_with(&["old"]);
        let declared = app.intents();
        assert_eq!(declared.len(), 1);
        assert_eq!(declared[0].id, "create-sticky");
        let dir = std::env::temp_dir();
        let params = Json::Obj(vec![("text".into(), Json::Str("from a selection".into()))]);
        assert!(app.perform_intent(None, &dir, "create-sticky", &params).is_ok());
        let board = app.board.as_ref().unwrap();
        assert_eq!(board.notes.len(), 2);
        assert_eq!(board.focused().unwrap().text, "from a selection");
        assert!(app.dirty);
        assert!(app.undo());
        assert_eq!(app.board.as_ref().unwrap().notes.len(), 1);
        let blank = Json::Obj(vec![("text".into(), Json::Str("  \n".into()))]);
        assert!(app.perform_intent(None, &dir, "create-sticky", &blank).is_err());
        assert!(app.perform_intent(None, &dir, "no-such-intent", &params).is_err());
        assert!(app.perform_intent(None, &dir, "create-sticky", &Json::Obj(Vec::new())).is_err(), "the text is required");
    }

    #[test]
    fn the_make_sticky_service_before_the_board_is_loaded_writes_the_saved_file() {
        let dir = std::env::temp_dir().join(format!("stickies-intent-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut app = StickiesApp::new();
        app.create_sticky(&dir, "first").unwrap();
        app.create_sticky(&dir, "second").unwrap();
        let saved = store::load(&dir);
        assert_eq!(saved.notes.iter().map(|n| n.text.as_str()).collect::<Vec<_>>(), vec!["first", "second"]);
        assert!(saved.notes[1].x > saved.notes[0].x, "the second note cascades");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn replace_and_custom_colour_ids_do_not_clash_with_the_other_menu_ids() {
        let ids = [MENU_COLOUR_CUSTOM, MENU_SPEAK_START, MENU_SPEAK_STOP, MENU_REPLACE, MENU_REPLACE_ALL, MENU_REPLACE_IN_NOTE, MENU_IGNORE_CASE, MENU_WHOLE_WORD, MENU_FLOAT_ON_TOP, MENU_CLOSE, MENU_PORTRAIT, MENU_LANDSCAPE, MENU_PAPER_BASE, MENU_PAPER_BASE + 4];
        for (i, a) in ids.iter().enumerate() {
            assert!(!(MENU_COLOUR_BASE..=15).contains(a) && !(MENU_ARRANGE_BASE..=56).contains(a) && !(MENU_INK_BASE..=64).contains(a));
            assert!(![MENU_INFO_CREATED, MENU_INFO_MODIFIED, MENU_ZOOM, MENU_TRANSLUCENT].contains(a));
            assert!(!ids[i + 1..].contains(a));
        }
        assert_eq!(undo_name(MENU_ARRANGE_BASE + 4), Some("Arrange"), "Location");
    }

    #[test]
    fn frames_change_when_a_note_moves() {
        let mut b = Board::default();
        let id = b.new_note(10.0, 10.0, 0);
        let before = frames(&b);
        b.find_mut(id).unwrap().x = 50.0;
        assert_ne!(before, frames(&b));
        b.find_mut(id).unwrap().x = 10.0;
        assert_eq!(before, frames(&b));
    }

    #[test]
    fn a_format_change_applies_to_the_selection_only() {
        let mut app = app_with(&["one two"]);
        {
            let note = app.board.as_mut().unwrap().focused_mut().unwrap();
            note.set_caret(4, false);
            note.set_caret(7, true);
        }
        assert!(app.restyle_focused(|s| s.outline = true));
        let note = focused(&app);
        assert!(note.styles[4..7].iter().all(|s| s.outline));
        assert!(!note.styles[0].outline && !note.style.outline, "the rest, and the typing style, keep theirs");
        assert!(app.toggle_style(|s| s.outline, |s, v| s.outline = v), "the selection is outlined, so this turns it off");
        assert!(!focused(&app).styles[5].outline);
    }

    #[test]
    fn kern_baseline_and_ligature_items_set_their_own_attribute() {
        let mut app = app_with(&["x y"]);
        app.board.as_mut().unwrap().focused_mut().unwrap().set_caret(0, false);
        assert!(app.restyle_focused(|s| s.baseline = Baseline::Superscript));
        assert!(app.restyle_focused(|s| s.kern = Setting::Off));
        assert!(app.restyle_focused(|s| s.ligature = Setting::All));
        let style = focused(&app).style_at_index(0);
        assert_eq!((style.baseline, style.kern, style.ligature), (Baseline::Superscript, Setting::Off, Setting::All));
        assert!(!app.restyle_focused(|s| s.kern = Setting::Off), "already set");
    }

    #[test]
    fn smart_quotes_and_dashes_change_what_is_typed() {
        let mut app = app_with(&[""]);
        type_chars(&mut app, "\"hi\" a--b", true, true);
        assert_eq!(focused(&app).text, "\u{201c}hi\u{201d} a\u{2014}b");
    }

    #[test]
    fn with_substitutions_off_the_text_is_typed_as_it_is() {
        let mut app = app_with(&[""]);
        type_chars(&mut app, "\"x\"--", false, false);
        assert_eq!(focused(&app).text, "\"x\"--");
    }

    #[test]
    fn command_arrows_go_to_the_edges_and_shift_extends_the_selection() {
        let mut app = app_with(&["hello world"]);
        // the caret starts at the end of the text
        assert!(app.chord(37, CHORD_META), "Cmd-Left goes to the start of the line");
        assert_eq!(focused(&app).caret, 0);
        assert!(app.chord(39, CHORD_META));
        assert_eq!(focused(&app).caret, 11);
        assert!(app.chord(38, CHORD_META));
        assert_eq!(focused(&app).caret, 0);
        assert!(app.chord(40, CHORD_META | CHORD_SHIFT));
        assert_eq!(focused(&app).selection(), 0..11);
        assert!(!app.chord(40, 0), "no Cmd, no move");
        assert!(!app.chord(33, CHORD_META), "other keys are not handled here");
        // the key input form of the same chord reaches the note through key()
        let shifted = KeyInput::chord(37, CHORD_META | CHORD_SHIFT);
        assert_eq!(shifted.as_chord(), Some((37, CHORD_META | CHORD_SHIFT)));
    }

    #[test]
    fn add_link_inserts_a_web_address_and_refuses_anything_else() {
        let mut app = app_with(&["see "]);
        app.link_entry = Some(String::new());
        for c in "example.com".chars() {
            app.link_key(&KeyInput::Text(c.to_string()));
        }
        assert!(!app.link_key(&KeyInput::Code(13)), "not an address yet");
        assert!(app.link_entry.is_some());
        app.link_entry = Some(String::new());
        for c in "https://example.com".chars() {
            app.link_key(&KeyInput::Text(c.to_string()));
        }
        assert!(app.link_key(&KeyInput::Code(13)));
        assert!(app.link_entry.is_none());
        assert_eq!(focused(&app).text, "see https://example.com");
        assert_eq!(app.history.undo_title(), "Undo Add Link");
        assert!(app.undo());
        assert_eq!(focused(&app).text, "see ");
    }

    #[test]
    fn escape_closes_the_link_bar_without_a_change() {
        let mut app = app_with(&["note"]);
        app.link_entry = Some("https://".into());
        assert!(app.link_key(&KeyInput::Code(27)));
        assert!(app.link_entry.is_none());
        assert!(!app.history.can_undo());
    }

    #[test]
    fn a_boundary_is_a_space_or_punctuation_and_not_a_letter() {
        assert!(is_boundary(" "));
        assert!(is_boundary("."));
        assert!(!is_boundary("a"));
        assert!(!is_boundary("'"), "an apostrophe is part of a word");
        assert!(!is_boundary("ab"));
        assert!(!is_boundary(""));
    }

    #[test]
    fn a_single_hyphen_after_text_is_typed_as_it_is_with_smart_dashes_on() {
        let mut app = app_with(&["ab"]);
        app.prefs = Prefs { smart_dashes: true, ..Prefs::default() };
        app.board.as_mut().unwrap().focused_mut().unwrap().set_caret(2, false);
        assert!(app.typed("-"));
        assert_eq!(focused(&app).text, "ab-");
        assert!(app.typed("-"));
        assert_eq!(focused(&app).text, "ab\u{2014}", "the second hyphen makes the dash");
    }

    #[test]
    fn toggling_a_switch_saves_it_and_drops_the_marks() {
        let mut app = app_with(&["note"]);
        app.marks.insert(1, Vec::new());
        app.marked.insert(1, "note".into());
        assert!(app.prefs.check_while_typing);
        app.prefs.check_while_typing = false;
        app.forget_marks();
        assert!(app.marks.is_empty() && app.marked.is_empty());
        assert!(!app.prefs.correct_automatically);
    }
}
