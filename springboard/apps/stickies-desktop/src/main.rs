//! Sticky Notes: small floating notes on one board. Each note has a colour, a collapse control, Bold and Italic,
//! three text sizes and a translucent look when it is not the focused note. The board saves on every change.

mod board;
mod model;
mod store;

use bifrost_app::kit::canvas::Canvas;
use bifrost_app::kit::{Damage, Surface};
use bifrost_app::proto::{DocErrorCode, DocMode};
use bifrost_app::{App, AppMenu, AppMenuItem, Command, Ctx, DocumentEvent, Input, KeyInput, TouchKind};
use model::{Board, Colour, Part, TextSize};
use std::io::{Read, Write};

const MENU_NEW: u32 = 1;
const MENU_DELETE: u32 = 2;
const MENU_COLLAPSE: u32 = 3;
const MENU_BOLD: u32 = 4;
const MENU_ITALIC: u32 = 5;
const MENU_SIZE_BASE: u32 = 6; // 6, 7, 8: small, medium, large
const MENU_COLOUR_BASE: u32 = 10; // 10..=15, one per colour
const MENU_IMPORT: u32 = 20;
const MENU_EXPORT: u32 = 21;
const MENU_EXPORT_ALL: u32 = 22;

/// Stacking offset of each new note, so new notes do not land on each other.
const CASCADE_PT: f32 = 24.0;
const CASCADE_STEPS: usize = 8;

/// A pointer gesture in progress.
#[derive(Clone, Copy, Debug)]
enum Drag {
    None,
    /// Down on a note's disclosure or body: a release on the disclosure toggles the note.
    Press { id: u64, part: Part },
    /// Down on a title bar: the note follows the pointer, keeping the grab offset.
    Move { id: u64, dx: f32, dy: f32 },
    /// Down on the grip: the note resizes from its size at the start.
    Resize { id: u64, x: f32, y: f32, w: f32, h: f32 },
}

struct StickiesApp {
    board: Option<Board>,
    drag: Drag,
    dirty: bool,
    /// Export waiting for the save panel's file: the request and the bytes to write.
    export: Option<(u32, Vec<u8>)>,
    /// The request of an import panel, when one is open.
    import: Option<u32>,
}

impl StickiesApp {
    fn new() -> StickiesApp {
        StickiesApp { board: None, drag: Drag::None, dirty: false, export: None, import: None }
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

    fn touch(&mut self, ctx: &Ctx, kind: TouchKind, x: f32, y: f32) -> bool {
        let (px, py) = Self::to_points(ctx, x, y);
        match kind {
            TouchKind::Down => {
                let Some(board) = &mut self.board else { return false };
                let Some(id) = board.hit(px, py) else {
                    // a press on the bare board takes focus away from every note
                    let had = board.focus.is_some();
                    board.focus = None;
                    return had;
                };
                board.raise(id);
                let Some(note) = board.find(id) else { return true };
                let part = note.part_at(px, py).unwrap_or(Part::Body);
                self.drag = match part {
                    Part::Title => Drag::Move { id, dx: px - note.x, dy: py - note.y },
                    Part::Grip => Drag::Resize { id, x: px, y: py, w: note.w, h: note.h },
                    part => Drag::Press { id, part },
                };
                true
            }
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
                    Drag::Resize { id, x, y, w, h } => {
                        if let Some(n) = board.find_mut(id) {
                            n.resize(w + px - x, h + py - y);
                        }
                        self.changed()
                    }
                    _ => false,
                }
            }
            TouchKind::Up | TouchKind::Cancel => {
                let drag = std::mem::replace(&mut self.drag, Drag::None);
                let Some(board) = &mut self.board else { return false };
                match (drag, kind) {
                    (Drag::Press { id, part: Part::Disclosure, .. }, TouchKind::Up) => {
                        let on_disclosure = board.find(id).is_some_and(|n| n.part_at(px, py) == Some(Part::Disclosure));
                        if on_disclosure {
                            if let Some(n) = board.find_mut(id) {
                                n.collapsed = !n.collapsed;
                            }
                            return self.changed();
                        }
                        true
                    }
                    (Drag::Move { .. } | Drag::Resize { .. }, _) => self.changed(),
                    _ => true,
                }
            }
        }
    }

    fn key(&mut self, key: &KeyInput) -> bool {
        let Some(board) = &mut self.board else { return false };
        let now = Self::now();
        let Some(note) = board.focused_mut() else { return false };
        match key {
            KeyInput::Text(text) => note.type_text(text, now),
            KeyInput::Code(8) => {
                if !note.backspace(now) {
                    return false;
                }
            }
            KeyInput::Code(13) => note.newline(now),
            _ => return false,
        }
        self.changed()
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

    /// The main-menu action with `id`. Returns true when the board changed.
    fn action(&mut self, ctx: &Ctx, id: u32) -> bool {
        match id {
            MENU_NEW => self.new_note(),
            MENU_DELETE => {
                let Some(board) = &mut self.board else { return false };
                let Some(focus) = board.focus else { return false };
                board.remove(focus);
                self.changed()
            }
            MENU_COLLAPSE => {
                let Some(note) = self.board.as_mut().and_then(|b| b.focused_mut()) else { return false };
                note.collapsed = !note.collapsed;
                self.changed()
            }
            MENU_BOLD | MENU_ITALIC => {
                let Some(note) = self.board.as_mut().and_then(|b| b.focused_mut()) else { return false };
                if id == MENU_BOLD {
                    note.style.bold = !note.style.bold;
                } else {
                    note.style.italic = !note.style.italic;
                }
                self.changed()
            }
            MENU_SIZE_BASE..=8 => {
                let size = [TextSize::Small, TextSize::Medium, TextSize::Large][(id - MENU_SIZE_BASE) as usize];
                let Some(note) = self.board.as_mut().and_then(|b| b.focused_mut()) else { return false };
                note.style.size = size;
                self.changed()
            }
            MENU_COLOUR_BASE..=15 => {
                let colour = Colour::from_index((id - MENU_COLOUR_BASE) as usize);
                let Some(note) = self.board.as_mut().and_then(|b| b.focused_mut()) else { return false };
                note.colour = colour;
                self.changed()
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
            Input::Touch { kind, x, y, .. } => self.touch(ctx, kind, x, y),
            Input::Key(key) => self.key(&key),
            Input::Command(Command::Copy) => {
                if let Some(note) = self.board.as_ref().and_then(|b| b.focused()) {
                    ctx.set_pasteboard_string(&note.text);
                }
                false
            }
            Input::Command(Command::Paste) => match ctx.pasteboard_string() {
                Some(text) => {
                    let now = Self::now();
                    match self.board.as_mut().and_then(|b| b.focused_mut()) {
                        Some(note) => {
                            note.type_text(&text, now);
                            self.changed()
                        }
                        None => false,
                    }
                }
                None => false,
            },
            _ => false,
        };
        self.flush(ctx);
        changed
    }

    fn main_menu(&self, _ctx: &Ctx) -> Vec<AppMenu> {
        let focus = self.board.as_ref().and_then(|b| b.focused());
        let has_focus = focus.is_some();
        let collapsed = focus.is_some_and(|n| n.collapsed);
        let style = focus.map(|n| n.style);
        let colour = focus.map(|n| n.colour);
        let mut items = vec![
            AppMenuItem::new(MENU_NEW, "New Sticky Note").key("cmd+n"),
            AppMenuItem::new(MENU_DELETE, "Delete Sticky Note").key("cmd+shift+backspace").enabled(has_focus),
            AppMenuItem::new(MENU_COLLAPSE, if collapsed { "Expand Sticky Note" } else { "Collapse Sticky Note" }).enabled(has_focus),
            AppMenuItem::separator(),
            AppMenuItem::new(MENU_BOLD, "Bold").key("cmd+b").enabled(has_focus).checked(style.is_some_and(|s| s.bold)),
            AppMenuItem::new(MENU_ITALIC, "Italic").key("cmd+i").enabled(has_focus).checked(style.is_some_and(|s| s.italic)),
        ];
        for (i, (size, title)) in [(TextSize::Small, "Small Text"), (TextSize::Medium, "Medium Text"), (TextSize::Large, "Large Text")].into_iter().enumerate() {
            items.push(AppMenuItem::new(MENU_SIZE_BASE + i as u32, title).enabled(has_focus).checked(style.is_some_and(|s| s.size == size)));
        }
        items.push(AppMenuItem::separator());
        for (i, c) in Colour::ALL.iter().enumerate() {
            items.push(AppMenuItem::new(MENU_COLOUR_BASE + i as u32, c.name()).enabled(has_focus).checked(colour == Some(*c)));
        }
        items.push(AppMenuItem::separator());
        items.push(AppMenuItem::new(MENU_IMPORT, "Import Text...").key("cmd+o"));
        items.push(AppMenuItem::new(MENU_EXPORT, "Export Sticky Note...").enabled(has_focus));
        items.push(AppMenuItem::new(MENU_EXPORT_ALL, "Export All Sticky Notes...").enabled(self.board.as_ref().is_some_and(|b| !b.notes.is_empty())));
        vec![AppMenu { title: "Sticky".into(), items }]
    }

    fn menu_action(&mut self, ctx: &Ctx, id: u32) -> bool {
        let changed = self.action(ctx, id);
        self.flush(ctx);
        changed
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
                let ids = match &mut self.board {
                    Some(board) => board.import_text(&text, Self::now()),
                    None => Vec::new(),
                };
                // one import is one use of the file: the app keeps no grant to it
                ctx.release_document(doc.grant);
                if ids.is_empty() {
                    return false;
                }
                self.changed();
                self.flush(ctx);
                true
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
