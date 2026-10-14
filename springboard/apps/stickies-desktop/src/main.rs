//! Sticky Notes: small floating notes on one board. Each note has a colour, a collapse control, a caret and
//! selection, text styles, and a translucent look when it is not the focused note. The board saves on every
//! change, and edits can be undone. Find searches all notes; a note prints, exports and imports as text.

mod board;
mod dates;
mod history;
mod layout;
mod model;
mod rgb;
mod store;

use bifrost_app::controls::sheet::{ButtonRole, Hit, Sheet, SheetButton};
use bifrost_app::controls::Rect;
use bifrost_app::kit::canvas::Canvas;
use bifrost_app::kit::{Damage, Surface};
use bifrost_app::layout::UNIFIED_BAR_PT;
use bifrost_app::print::{PrintError, PrintOperation};
use bifrost_app::proto::{DocErrorCode, DocMode};
use bifrost_app::widgets;
use bifrost_app::{App, AppMenu, AppMenuItem, Command, Commands, Ctx, DocumentEvent, Input, KeyInput, TouchKind};
use history::History;
use model::{Arrange, Baseline, Board, Case, Colour, Ink, Move, Part, Style, TextSize};
use std::io::{Read, Write};

const MENU_NEW: u32 = 1;
const MENU_DELETE: u32 = 2;
const MENU_DUPLICATE: u32 = 23;
const MENU_COLLAPSE: u32 = 3;
const MENU_BOLD: u32 = 4;
const MENU_ITALIC: u32 = 5;
const MENU_SIZE_BASE: u32 = 6; // 6, 7, 8: small, medium, large
const MENU_COLOUR_BASE: u32 = 10; // 10..=15, one per colour
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
const MENU_OUTLINE: u32 = 72;
const MENU_BASELINE_BASE: u32 = 73; // 73, 74, 75: normal, superscript, subscript
const MENU_REPLACE: u32 = 80;
const MENU_REPLACE_ALL: u32 = 81;

/// The buttons of the delete confirmation.
const SHEET_CANCEL: u32 = 1;
const SHEET_DELETE: u32 = 2;

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
        MENU_DELETE => "Delete",
        MENU_DUPLICATE => "Duplicate",
        MENU_COLLAPSE | MENU_COLLAPSE_ALL | MENU_EXPAND_ALL => "Collapse",
        MENU_BOLD | MENU_ITALIC | MENU_UNDERLINE | MENU_STRIKE | MENU_OUTLINE | MENU_BASELINE_BASE..=75 | MENU_BIGGER | MENU_SMALLER | MENU_PASTE_STYLE | MENU_USE_DEFAULT => "Style",
        MENU_SIZE_BASE..=8 => "Text Size",
        MENU_INK_BASE..=64 => "Text Colour",
        MENU_COLOUR_BASE..=15 => "Colour",
        MENU_CUT => "Cut",
        MENU_PASTE | MENU_PASTE_PLAIN => "Paste",
        MENU_UPPER | MENU_LOWER | MENU_CAPITALISE => "Transformation",
        MENU_TRANSLUCENT => "Translucent",
        MENU_REPLACE | MENU_REPLACE_ALL => "Replace",
        MENU_ZOOM => "Zoom",
        MENU_ARRANGE_BASE..=56 => "Arrange",
        _ => return None,
    })
}

struct StickiesApp {
    board: Option<Board>,
    drag: Drag,
    dirty: bool,
    /// Export waiting for the save panel's file: the request and the bytes to write.
    export: Option<(u32, Vec<u8>)>,
    /// The request of an import panel, when one is open.
    import: Option<u32>,
    history: History,
    /// The find bar's text while the bar is open.
    find: Option<String>,
    /// The replacement text while the find bar shows its replace field.
    replace: Option<String>,
    /// Whether typing goes to the replace field instead of the find field (Tab switches).
    replace_active: bool,
    /// The last find text, which Find Next uses once the bar is closed.
    last_find: String,
    confirm: Option<Confirm>,
    /// The style Copy Style took, for Paste Style.
    style_clip: Option<Style>,
    /// The note and time of the last press, for double presses.
    last_press: Option<(u64, f64)>,
    /// The board as it was when a press began, for the undo step of a move or resize.
    before_drag: Option<Board>,
}

impl StickiesApp {
    fn new() -> StickiesApp {
        StickiesApp {
            board: None,
            drag: Drag::None,
            dirty: false,
            export: None,
            import: None,
            history: History::default(),
            find: None,
            replace: None,
            replace_active: false,
            last_find: String::new(),
            confirm: None,
            style_clip: None,
            last_press: None,
            before_drag: None,
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
            if let (KeyInput::Code(27), Some(id)) = (key, confirm.sheet.cancel_button()) {
                return self.answer(id);
            }
            return false;
        }
        if self.find.is_some() {
            return self.find_key(key);
        }
        // Escape leaves the note: no note takes typing until one is pressed again
        if let KeyInput::Code(27) = key {
            let Some(board) = &mut self.board else { return false };
            if board.focus.take().is_none() {
                return false;
            }
            return self.changed();
        }
        let Some(board) = &mut self.board else { return false };
        let now = Self::now();
        let before = board.clone();
        let Some(note) = board.focused_mut() else { return false };
        let id = note.id;
        match key {
            KeyInput::Text(text) => {
                if text.chars().all(char::is_control) {
                    return false;
                }
                note.type_text(text, now)
            }
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

    /// Keys while the find bar is open: they edit the find text instead of a note.
    fn find_key(&mut self, key: &KeyInput) -> bool {
        if self.replace_active {
            return self.replace_key(key);
        }
        let Some(query) = &mut self.find else { return false };
        match key {
            KeyInput::Code(9) if self.replace.is_some() => {
                self.replace_active = true;
                return true;
            }
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
            KeyInput::Code(27) => {
                self.find = None;
                self.replace = None;
                self.replace_active = false;
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

    /// Keys while the replace field has the typing: Enter replaces the current match and steps to the next.
    fn replace_key(&mut self, key: &KeyInput) -> bool {
        let Some(with) = &mut self.replace else {
            self.replace_active = false;
            return false;
        };
        match key {
            KeyInput::Text(text) => with.extend(text.chars().filter(|c| !c.is_control())),
            KeyInput::Code(8) => {
                if with.pop().is_none() {
                    return false;
                }
            }
            KeyInput::Code(9) => self.replace_active = false,
            KeyInput::Code(13) => {
                let before = self.board.clone();
                let (query, with) = (self.find.clone().unwrap_or_default(), with.clone());
                let now = Self::now();
                let replaced = self.board.as_mut().is_some_and(|b| b.replace_selected(&query, &with, now));
                if replaced {
                    if let Some(before) = before {
                        self.history.push("Replace", before);
                    }
                    self.dirty = true;
                }
                self.step_find(true);
            }
            KeyInput::Code(27) => {
                self.find = None;
                self.replace = None;
                self.replace_active = false;
            }
            _ => return false,
        }
        true
    }

    /// Selects the next or previous match of the find text. With no text the find bar opens.
    fn step_find(&mut self, forward: bool) {
        let query = self.find.clone().unwrap_or_else(|| self.last_find.clone());
        if query.is_empty() {
            self.find = Some(String::new());
            return;
        }
        let Some(board) = &mut self.board else { return };
        if let Some(found) = board.find_step(&query, forward) {
            board.select_match(&found);
        }
    }

    fn import_panel(&mut self, ctx: &Ctx) -> bool {
        self.import = Some(ctx.open_document(DocMode::Read, "Import Sticky Note", &["public.text"]));
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
        let message = match PrintOperation::text(title, note.text.clone()).run() {
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

    /// Copy Style: remembers the focused note's text style.
    fn copy_style(&mut self) {
        self.style_clip = self.board.as_ref().and_then(|b| b.focused()).map(|n| n.style);
    }

    /// Paste Style: gives the focused note the remembered style. False when none was copied or no note has focus.
    fn paste_style(&mut self) -> bool {
        let Some(style) = self.style_clip else { return false };
        let Some(note) = self.board.as_mut().and_then(|b| b.focused_mut()) else { return false };
        if note.style == style {
            return false;
        }
        note.style = style;
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
            MENU_DELETE => {
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
            MENU_BOLD | MENU_ITALIC | MENU_UNDERLINE | MENU_STRIKE | MENU_OUTLINE => {
                let Some(note) = self.board.as_mut().and_then(|b| b.focused_mut()) else { return false };
                match id {
                    MENU_BOLD => note.style.bold = !note.style.bold,
                    MENU_ITALIC => note.style.italic = !note.style.italic,
                    MENU_UNDERLINE => note.style.underline = !note.style.underline,
                    MENU_OUTLINE => note.style.outline = !note.style.outline,
                    _ => note.style.strike = !note.style.strike,
                }
                self.changed()
            }
            MENU_BASELINE_BASE..=75 => {
                let baseline = [Baseline::Normal, Baseline::Superscript, Baseline::Subscript][(id - MENU_BASELINE_BASE) as usize];
                let Some(note) = self.board.as_mut().and_then(|b| b.focused_mut()) else { return false };
                note.style.baseline = baseline;
                self.changed()
            }
            MENU_BIGGER | MENU_SMALLER => {
                let Some(note) = self.board.as_mut().and_then(|b| b.focused_mut()) else { return false };
                let size = if id == MENU_BIGGER { note.style.size.bigger() } else { note.style.size.smaller() };
                if size == note.style.size {
                    return false;
                }
                note.style.size = size;
                self.changed()
            }
            MENU_SIZE_BASE..=8 => {
                let size = [TextSize::Small, TextSize::Medium, TextSize::Large][(id - MENU_SIZE_BASE) as usize];
                let Some(note) = self.board.as_mut().and_then(|b| b.focused_mut()) else { return false };
                note.style.size = size;
                self.changed()
            }
            MENU_INK_BASE..=64 => {
                let ink = Ink::ALL[(id - MENU_INK_BASE) as usize];
                let Some(note) = self.board.as_mut().and_then(|b| b.focused_mut()) else { return false };
                note.style.ink = ink;
                self.changed()
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
                note.colour = colour;
                self.changed()
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
                if self.find.is_none() {
                    self.find = Some(self.last_find.clone());
                }
                true
            }
            MENU_FIND_NEXT | MENU_FIND_PREVIOUS => {
                self.step_find(id == MENU_FIND_NEXT);
                true
            }
            MENU_REPLACE => {
                // opens the find bar with its replace field, which takes the typing
                if self.find.is_none() {
                    self.find = Some(self.last_find.clone());
                }
                self.replace.get_or_insert_with(String::new);
                self.replace_active = true;
                true
            }
            MENU_REPLACE_ALL => {
                let (Some(query), Some(with)) = (self.find.clone(), self.replace.clone()) else { return false };
                let count = self.board.as_mut().map_or(0, |b| b.replace_all(&query, &with, now));
                if count > 0 {
                    ctx.notify("Sticky Notes", &format!("Replaced {count} {}", if count == 1 { "match" } else { "matches" }));
                }
                count > 0 && self.changed()
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
            MENU_OPEN_LINK => {
                let url = self.board.as_ref().and_then(|b| b.focused()).and_then(|n| n.link_at(n.caret));
                if let Some(url) = url {
                    ctx.open_url(&url);
                }
                false
            }
            MENU_PRINT => {
                self.print(ctx);
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
        AppMenu {
            title: "File".into(),
            items: vec![
                AppMenuItem::new(MENU_NEW, "New Sticky Note").key("cmd+n"),
                AppMenuItem::new(MENU_DUPLICATE, "Duplicate Sticky Note").key("cmd+d").enabled(has_focus),
                AppMenuItem::new(MENU_DELETE, "Delete Sticky Note").key("cmd+shift+backspace").enabled(has_focus),
                AppMenuItem::separator(),
                AppMenuItem::new(MENU_IMPORT, "Import Text...").key("cmd+o"),
                AppMenuItem::new(MENU_EXPORT, "Export Sticky Note...").enabled(has_focus),
                AppMenuItem::new(MENU_EXPORT_ALL, "Export All Sticky Notes...").enabled(any),
                AppMenuItem::separator(),
                AppMenuItem::new(MENU_PRINT, "Print...").key("cmd+p").enabled(has_focus),
            ],
        }
    }

    fn edit_menu(&self, ctx: &Ctx) -> AppMenu {
        let note = self.board.as_ref().and_then(|b| b.focused());
        let has_focus = note.is_some();
        let has_selection = note.is_some_and(|n| n.has_selection());
        let has_link = note.is_some_and(|n| n.link_at(n.caret).is_some());
        let can_paste = has_focus && ctx.pasteboard_string().is_some();
        let has_find = !self.last_find.is_empty() || self.find.as_ref().is_some_and(|q| !q.is_empty());
        let any = self.board.as_ref().is_some_and(|b| !b.notes.is_empty());
        AppMenu {
            title: "Edit".into(),
            items: vec![
                AppMenuItem::new(MENU_UNDO, &self.history.undo_title()).key("cmd+z").enabled(self.history.can_undo()),
                AppMenuItem::new(MENU_REDO, &self.history.redo_title()).key("cmd+shift+z").enabled(self.history.can_redo()),
                AppMenuItem::separator(),
                AppMenuItem::new(MENU_CUT, "Cut").key("cmd+x").enabled(has_selection),
                AppMenuItem::new(MENU_COPY, "Copy").key("cmd+c").enabled(has_focus),
                AppMenuItem::new(MENU_PASTE, "Paste").key("cmd+v").enabled(can_paste),
                AppMenuItem::new(MENU_PASTE_PLAIN, "Paste and Match Style").key("cmd+alt+shift+v").enabled(can_paste),
                AppMenuItem::new(MENU_SELECT_ALL, "Select All").key("cmd+a").enabled(has_focus),
                AppMenuItem::separator(),
                AppMenuItem::new(MENU_FIND, "Find...").key("cmd+f").enabled(any),
                AppMenuItem::new(MENU_REPLACE, "Replace...").key("cmd+alt+f").enabled(any),
                AppMenuItem::new(MENU_REPLACE_ALL, "Replace All").enabled(any && self.replace.is_some()),
                AppMenuItem::new(MENU_FIND_NEXT, "Find Next").key("cmd+g").enabled(any && has_find),
                AppMenuItem::new(MENU_FIND_PREVIOUS, "Find Previous").key("cmd+shift+g").enabled(any && has_find),
                AppMenuItem::new(MENU_USE_SELECTION, "Use Selection for Find").key("cmd+e").enabled(has_selection),
                AppMenuItem::separator(),
                AppMenuItem::new(MENU_OPEN_LINK, "Open Link").enabled(has_link),
                AppMenuItem::submenu("Transformations"),
                AppMenuItem::new(MENU_UPPER, "Make Upper Case").enabled(has_selection).depth(1),
                AppMenuItem::new(MENU_LOWER, "Make Lower Case").enabled(has_selection).depth(1),
                AppMenuItem::new(MENU_CAPITALISE, "Capitalise").enabled(has_selection).depth(1),
            ],
        }
    }

    fn font_menu(&self, has_focus: bool) -> AppMenu {
        let style = self.board.as_ref().and_then(|b| b.focused()).map(|n| n.style);
        let on = |f: fn(&Style) -> bool| style.as_ref().is_some_and(f);
        let mut items = vec![
            AppMenuItem::new(MENU_BOLD, "Bold").key("cmd+b").enabled(has_focus).checked(on(|s| s.bold)),
            AppMenuItem::new(MENU_ITALIC, "Italic").key("cmd+i").enabled(has_focus).checked(on(|s| s.italic)),
            AppMenuItem::new(MENU_UNDERLINE, "Underline").key("cmd+u").enabled(has_focus).checked(on(|s| s.underline)),
            AppMenuItem::new(MENU_STRIKE, "Strikethrough").enabled(has_focus).checked(on(|s| s.strike)),
            AppMenuItem::new(MENU_OUTLINE, "Outline").enabled(has_focus).checked(on(|s| s.outline)),
            AppMenuItem::submenu("Baseline"),
            AppMenuItem::new(MENU_BASELINE_BASE, "Use Default").enabled(has_focus).checked(on(|s| s.baseline == Baseline::Normal)).depth(1),
            AppMenuItem::new(MENU_BASELINE_BASE + 1, "Superscript").enabled(has_focus).checked(on(|s| s.baseline == Baseline::Superscript)).depth(1),
            AppMenuItem::new(MENU_BASELINE_BASE + 2, "Subscript").enabled(has_focus).checked(on(|s| s.baseline == Baseline::Subscript)).depth(1),
            AppMenuItem::separator(),
            AppMenuItem::new(MENU_BIGGER, "Bigger").key("cmd+=").enabled(style.is_some_and(|s| s.size != TextSize::Large)),
            AppMenuItem::new(MENU_SMALLER, "Smaller").key("cmd+-").enabled(style.is_some_and(|s| s.size != TextSize::Small)),
        ];
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
        let colour = self.board.as_ref().and_then(|b| b.focused()).map(|n| n.colour);
        let items = Colour::ALL
            .iter()
            .enumerate()
            .map(|(i, c)| AppMenuItem::new(MENU_COLOUR_BASE + i as u32, c.name()).enabled(has_focus).checked(colour == Some(*c)))
            .collect();
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
            self.board = Some(store::load(&ctx.data_dir));
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
                let ids = match &mut self.board {
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
        if let Some(board) = &self.board {
            board::draw(canvas, &ctx.ui, board);
            if let Some(query) = &self.find {
                board::draw_find(canvas, &ctx.ui, query, self.replace.as_deref(), self.replace_active, board.matches(query).len());
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

    #[test]
    fn every_mutating_menu_id_has_an_undo_name_and_the_others_do_not() {
        for id in [MENU_NEW, MENU_DELETE, MENU_DUPLICATE, MENU_BOLD, MENU_CUT, MENU_PASTE, MENU_ZOOM, MENU_UPPER, 7, 12, 62, 54] {
            assert!(undo_name(id).is_some(), "id {id}");
        }
        for id in [MENU_COPY, MENU_SELECT_ALL, MENU_FIND, MENU_PRINT, MENU_IMPORT, MENU_OPEN_LINK, MENU_UNDO, MENU_COPY_STYLE] {
            assert!(undo_name(id).is_none(), "id {id}");
        }
        assert_eq!(undo_name(MENU_ARRANGE_BASE), Some("Arrange"));
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
    fn replace_field_takes_typing_after_tab_and_enter_replaces_the_match() {
        let mut app = app_with(&["alpha beta"]);
        app.find = Some(String::new());
        app.replace = Some(String::new());
        for c in "beta".chars() {
            assert!(app.find_key(&KeyInput::Text(c.to_string())));
        }
        assert!(app.find_key(&KeyInput::Code(9)));
        assert!(app.replace_active);
        for c in "gamma".chars() {
            assert!(app.find_key(&KeyInput::Text(c.to_string())));
        }
        assert_eq!(app.replace.as_deref(), Some("gamma"));
        assert_eq!(app.find.as_deref(), Some("beta"));
        assert!(app.find_key(&KeyInput::Code(13)));
        assert_eq!(app.board.as_ref().unwrap().notes[0].text, "alpha gamma");
        assert!(app.history.can_undo());
        assert!(app.undo());
        assert_eq!(app.board.as_ref().unwrap().notes[0].text, "alpha beta");
        // Escape closes the bar and the replace field together
        assert!(app.find_key(&KeyInput::Code(27)));
        assert!(app.find.is_none() && app.replace.is_none() && !app.replace_active);
    }

    #[test]
    fn outline_and_baseline_menu_ids_are_undoable_style_changes() {
        for id in [MENU_OUTLINE, MENU_BASELINE_BASE, MENU_BASELINE_BASE + 2, MENU_REPLACE_ALL] {
            assert!(undo_name(id).is_some(), "id {id}");
        }
        assert!(undo_name(MENU_REPLACE).is_some());
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
    fn copy_style_and_paste_style_move_a_style_between_notes() {
        let mut app = app_with(&["one", "two"]);
        let (first, second) = {
            let b = app.board.as_ref().unwrap();
            (b.notes[0].id, b.notes[1].id)
        };
        assert!(!app.paste_style(), "nothing copied yet");
        {
            let b = app.board.as_mut().unwrap();
            b.find_mut(first).unwrap().style = Style { bold: true, underline: true, ..Style::default() };
            b.raise(first);
        }
        app.copy_style();
        assert_eq!(app.style_clip.map(|s| s.bold), Some(true));
        app.board.as_mut().unwrap().raise(second);
        assert!(app.paste_style());
        assert!(app.board.as_ref().unwrap().find(second).unwrap().style.underline);
        assert!(!app.paste_style(), "the style is already there");
        assert_eq!(undo_name(MENU_PASTE_STYLE), Some("Style"));
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
}
