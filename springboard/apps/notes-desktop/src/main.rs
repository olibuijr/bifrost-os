//! Notes as an installed BifrOSt desktop app. The note code is the shared
//! Notes module (`apps/notes/src/notes`), shared unchanged; this crate only
//! hosts it through the SDK for the Wayland desktop.

use notes::notes;

use bifrost_app::kit::{Damage, Event, Surface};
use bifrost_app::KeyInput;
use bifrost_app::proto::{DocErrorCode, DocMode};
use bifrost_app::{App, AppMenu, AppMenuItem, Command, Commands, Ctx, DocumentEvent, Input, TouchKind};
use bifrost_app::control::{RpcError, INVALID_PARAMS};
use bifrost_app::intents::{self, Intent, IntentResult, Param, ParamType};
use bifrost_core::json::Json;
use springboard::shell::keyboard::Out;
use springboard::shell::clock::Wall;

struct NotesApp {
    notes: Option<notes::Notes>,
    last_wall: f64,
    /// the save panel asked for by Export, with the bytes it will write
    export: Option<(u32, Vec<u8>)>,
    /// the open panel asked for by Import (Cmd+O)
    import: Option<u32>,
    /// Core Search: the `modified` stamp of each note as last indexed, by note id
    indexed: std::collections::HashMap<String, i64>,
    next_index: f64,
}

/// Search domain of every note.
const SPOT_DOMAIN: &str = "notes";
/// How often the app compares its notes with what it indexed (ms).
const INDEX_EVERY_MS: f64 = 3000.0;

impl NotesApp {
    /// Keeps the Core Search index in step with the notes: new and edited ones are indexed (title, first lines as the
    /// description, the folder as a keyword), deleted and trashed ones are removed.
    fn sync_index(&mut self, ctx: &Ctx) {
        use bifrost_app::quicksearch::{CSSearchableIndex, CSSearchableItem, CSSearchableItemAttributeSet};
        let Some(n) = &self.notes else { return };
        let mut current = std::collections::HashMap::new();
        let mut changed = vec![];
        for (note, folder) in n.indexable() {
            current.insert(note.id.clone(), note.modified);
            if self.indexed.get(&note.id) != Some(&note.modified) {
                let body: String = note.text.lines().map(str::trim).filter(|l| !l.is_empty()).skip(1).take(3).collect::<Vec<_>>().join(" ");
                let mut set = CSSearchableItemAttributeSet::new(note.title()).content_description(&body);
                if !folder.is_empty() {
                    set = set.keywords(&[folder]);
                }
                changed.push(CSSearchableItem::new(&note.id, SPOT_DOMAIN, set));
            }
        }
        let gone: Vec<&str> = self.indexed.keys().filter(|id| !current.contains_key(*id)).map(String::as_str).collect();
        let index = CSSearchableIndex::default_index();
        if !changed.is_empty() {
            index.index_searchable_items(ctx, &changed);
        }
        if !gone.is_empty() {
            index.delete_searchable_items(ctx, &gone);
        }
        self.indexed = current;
    }
}

/// evdev O: Cmd+O is File > Import to Notes.
const KEY_O: u32 = 24;
/// Imports larger than this are refused (a note is text; this keeps a mistaken pick from filling memory).
const IMPORT_MAX: u64 = 4 << 20;
/// `AppMenuItem` ids for `menu_action` (0 is reserved for separators).
const MENU_NEW: u32 = 1;
const MENU_DELETE: u32 = 2;
const MENU_IMPORT: u32 = 3;
const MENU_FIND: u32 = 4;
/// evdev Z (the Undo and Redo chords are the editor's: Cmd+Z, Cmd+Shift+Z).
const KEY_Z: u32 = 44;

impl NotesApp {
    /// File > Import to Notes (Cmd+O): the open panel answers in `document`.
    fn open_import(&mut self, ctx: &Ctx) {
        self.import = Some(ctx.open_document(DocMode::Read, "Import to Notes", &["public.text"]));
    }

    /// Share on the PC is Export (`NativeSavePanel`): the note goes to a file the user names, as Markdown.
    fn export(&mut self, ctx: &Ctx, item: springboard::shell::share::Item) {
        let (name, bytes) = match item {
            springboard::shell::share::Item::Text { title, body } => {
                let title: String = title.chars().filter(|c| *c != '/' && !c.is_control()).take(120).collect();
                (format!("{}.md", if title.trim().is_empty() { "Note" } else { title.trim() }), body.into_bytes())
            }
            springboard::shell::share::Item::File(path) => match std::fs::read(&path) {
                Ok(bytes) => (path.file_name().map_or_else(|| "Note".into(), |n| n.to_string_lossy().into_owned()), bytes),
                Err(e) => return ctx.log(&format!("export: {e}")),
            },
            // a contact card is not something Notes shares
            _ => return,
        };
        let request = ctx.save_document(&name, &[]);
        self.export = Some((request, bytes));
    }
}

impl App for NotesApp {
    fn id(&self) -> &str {
        "is.olibuijr.notes.desktop"
    }

    fn configure(&mut self, ctx: &Ctx) {
        let (w, h) = (ctx.cfg.w as i32, ctx.cfg.h as i32);
        match &mut self.notes {
            Some(n) => n.set_size(w, h),
            None => {
                let mut n = notes::Notes::new(&ctx.ui, w, h, ctx.data_dir.clone(), Wall::sample(0.0));
                n.set_hardware_keyboard(true);
                self.notes = Some(n);
                ctx.wants_keyboard(true);
            }
        }
    }

    fn rpc_methods(&self) -> Vec<&'static str> {
        notes::rpc::methods()
    }

    fn main_menu(&self, _ctx: &Ctx) -> Vec<AppMenu> {
        let Some(n) = &self.notes else { return Vec::new() };
        // "Note" is the app's own menu: File, Edit, View, Window and Help are the shell's, so none is repeated.
        // Undo and Redo are the shell's Edit commands (`commands` and `Input::Command`), not items here.
        vec![AppMenu {
            title: "Note".into(),
            items: vec![
                AppMenuItem::new(MENU_NEW, "New Note").key("cmd+n"),
                AppMenuItem::new(MENU_DELETE, "Delete Note").key("cmd+backspace").enabled(n.can_delete()),
                AppMenuItem::separator(),
                AppMenuItem::new(MENU_IMPORT, "Import to Notes...").key("cmd+o"),
                AppMenuItem::separator(),
                AppMenuItem::new(MENU_FIND, "Find").key("cmd+f"),
            ],
        }]
    }

    fn commands(&self, _ctx: &Ctx) -> Commands {
        let Some(n) = &self.notes else { return Commands::default() };
        Commands::default().with(Command::Undo, n.is_editing()).with(Command::Redo, n.is_editing())
    }

    fn menu_action(&mut self, ctx: &Ctx, id: u32) -> bool {
        if id == MENU_IMPORT {
            self.open_import(ctx);
            return false;
        }
        let now = bifrost_app::proto::sys::now_ms();
        let Some(n) = &mut self.notes else { return false };
        match id {
            MENU_NEW => {
                n.compose(&ctx.ui, now);
                true
            }
            MENU_DELETE => n.delete_current(&ctx.ui, now),
            MENU_FIND => {
                n.find(&ctx.ui, now);
                true
            }
            _ => false,
        }
    }

    fn intents(&self) -> Vec<Intent> {
        vec![
            Intent::new("new-note", "New Note", "Starts a new note in Notes.").shortcut(),
            Intent::new("create-note", "Create Note", "Creates a note with the given text and opens it.")
                .param(Param::new("text", "Text", ParamType::String).optional()),
            Intent::new("search-notes", "Search Notes", "Lists the notes that contain the query.").param(Param::new("query", "Query", ParamType::String)),
        ]
    }

    fn perform_intent(&mut self, ctx: Option<&Ctx>, data_dir: &std::path::Path, id: &str, params: &Json) -> Result<IntentResult, RpcError> {
        let now = bifrost_app::proto::sys::now_ms();
        match id {
            "new-note" | "create-note" => {
                let (Some(ctx), Some(n)) = (ctx, self.notes.as_mut()) else { return Err(intents::failed("Notes is not running yet")) };
                // no text is a blank note, which opens in the editor as New Note does
                let text = params.get("text").and_then(Json::as_str).unwrap_or("");
                let note = n.import_note(&ctx.ui, text, now);
                Ok(IntentResult::value(Json::Str(note)).with_dialog("Created a note"))
            }
            "search-notes" => {
                let query = intents::str_param(params, "query")?;
                let hits = notes::rpc::call(data_dir, "notes.list", &Json::Obj(vec![("query".into(), Json::Str(query.into()))]))?;
                let count = hits.as_array().map_or(0, |a| a.len());
                let dialog = if count == 1 { "1 note matches".to_string() } else { format!("{count} notes match") };
                Ok(IntentResult::value(hits).with_dialog(&dialog))
            }
            _ => Err(RpcError::new(INVALID_PARAMS, format!("no intent {id}"))),
        }
    }

    fn rpc(&mut self, _ctx: Option<&Ctx>, data_dir: &std::path::Path, method: &str, params: &bifrost_core::json::Json) -> Result<bifrost_core::json::Json, bifrost_app::control::RpcError> {
        notes::rpc::call(data_dir, method, params)
    }

    fn controls(&self, ctx: &Ctx) -> Vec<bifrost_app::controls::Node> {
        self.notes.as_ref().map_or_else(Vec::new, |n| n.controls(&ctx.ui))
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        let Some(n) = &mut self.notes else { return false };
        match ev {
            // a published toolbar control (accessibility, `bifrost pc midget tap-window`): a click at its centre
            Input::Activate(id) => {
                let Some(node) = n.controls(&ctx.ui).into_iter().find(|c| c.id == id && c.enabled) else { return false };
                let (x, y) = (ctx.ui.pt(node.bounds.x + node.bounds.w / 2.0), ctx.ui.pt(node.bounds.y + node.bounds.h / 2.0));
                let t = bifrost_app::proto::sys::now_ms();
                n.touch(&ctx.ui, Event::Down(x, y), t);
                n.touch(&ctx.ui, Event::Up(x, y), t);
                if let Some(item) = n.take_share() {
                    self.export(ctx, item);
                }
                true
            }
            Input::Touch { kind, x, y, t, .. } => {
                let e = match kind {
                    TouchKind::Down => Event::Down(x, y),
                    TouchKind::Move => Event::Move(x, y),
                    TouchKind::Up | TouchKind::Cancel => Event::Up(x, y),
                };
                n.touch(&ctx.ui, e, t);
                let share = n.take_share();
                if let Some(url) = n.take_link() {
                    ctx.open_url(&url);
                }
                // no share sheet yet: Share exports the note through the save panel
                if let Some(item) = share {
                    self.export(ctx, item);
                }
                true
            }
            // the shell's Edit > Undo and Redo: the editor's own undo and redo, as the Cmd+Z chords are
            Input::Command(command) => {
                let mods = match command {
                    Command::Undo => bifrost_app::proto::CHORD_META,
                    Command::Redo => bifrost_app::proto::CHORD_META | bifrost_app::proto::CHORD_SHIFT,
                    _ => return false,
                };
                n.chord(&ctx.ui, KEY_Z, mods, bifrost_app::proto::sys::now_ms())
            }
            Input::Key(k) => {
                if let Some((key, mods)) = k.as_chord() {
                    use bifrost_app::proto::{CHORD_ALT, CHORD_CTRL, CHORD_META, CHORD_SHIFT};
                    if key == KEY_O && mods & (CHORD_CTRL | CHORD_META) != 0 && mods & (CHORD_SHIFT | CHORD_ALT) == 0 {
                        self.open_import(ctx);
                        return false;
                    }
                    return n.chord(&ctx.ui, key, mods, bifrost_app::proto::sys::now_ms());
                }
                if k == KeyInput::Code(1) {
                    return n.escape(&ctx.ui, bifrost_app::proto::sys::now_ms());
                }
                let out = match k {
                    KeyInput::Text(s) => Out::Text(s),
                    KeyInput::Code(8) => Out::Backspace,
                    KeyInput::Code(13) => Out::Enter,
                    KeyInput::Code(9) => Out::Named("Tab"),
                    KeyInput::Code(37) => Out::Named("Left"),
                    KeyInput::Code(38) => Out::Named("Up"),
                    KeyInput::Code(39) => Out::Named("Right"),
                    KeyInput::Code(40) => Out::Named("Down"),
                    KeyInput::Code(_) => return false,
                };
                n.key(&ctx.ui, out, bifrost_app::proto::sys::now_ms());
                true
            }
            _ => false,
        }
    }

    fn document(&mut self, ctx: &Ctx, ev: DocumentEvent) -> bool {
        use std::io::{Read, Write};
        match ev {
            DocumentEvent::Opened(mut doc) if self.import == Some(doc.request) => {
                self.import = None;
                if doc.size > IMPORT_MAX {
                    ctx.log(&format!("import: {} is too large ({} bytes)", doc.name, doc.size));
                    return false;
                }
                let mut bytes = Vec::new();
                if let Err(e) = (&mut doc.file).take(IMPORT_MAX).read_to_end(&mut bytes) {
                    ctx.log(&format!("import: {e}"));
                    return false;
                }
                let Some(n) = &mut self.notes else { return false };
                n.import_note(&ctx.ui, &String::from_utf8_lossy(&bytes), bifrost_app::proto::sys::now_ms());
                // one import is one use of the file: the app keeps no grant to it
                ctx.release_document(doc.grant);
                true
            }
            DocumentEvent::SaveTo(mut doc) => {
                match self.export.take() {
                    Some((request, bytes)) if request == doc.request => match doc.file.write_all(&bytes) {
                        Ok(()) => ctx.commit_document(request),
                        Err(e) => {
                            ctx.log(&format!("export: {e}"));
                            ctx.abort_document(request);
                        }
                    },
                    other => {
                        self.export = other;
                        ctx.abort_document(doc.request);
                    }
                }
                false
            }
            DocumentEvent::Saved(doc) => {
                ctx.release_document(doc.grant);
                false
            }
            DocumentEvent::Failed { request, code } => {
                if self.import == Some(request) {
                    self.import = None;
                }
                if self.export.as_ref().is_some_and(|(r, _)| *r == request) {
                    self.export = None;
                }
                if code != DocErrorCode::Cancelled {
                    ctx.log(&format!("document request {request}: {code:?}"));
                }
                false
            }
            DocumentEvent::Opened(_) | DocumentEvent::OpenedMany { .. } => false,
        }
    }

    fn suspend(&mut self, _ctx: &Ctx) {
        if let Some(n) = &mut self.notes {
            n.flush();
        }
    }

    fn continue_activity(&mut self, ctx: &Ctx, activity: bifrost_app::quicksearch::UserActivity) -> bool {
        let now = bifrost_app::proto::sys::now_ms();
        activity.activity_type == bifrost_app::quicksearch::ACTION_TYPE && self.notes.as_mut().is_some_and(|n| n.show_note(&ctx.ui, &activity.unique_identifier, now))
    }

    fn tick(&mut self, ctx: &Ctx, t: f64) -> bool {
        if t >= self.next_index && self.notes.is_some() {
            self.next_index = t + INDEX_EVERY_MS;
            self.sync_index(ctx);
        }
        let Some(n) = &mut self.notes else { return false };
        if t - self.last_wall >= 1000.0 {
            self.last_wall = t;
            n.set_wall(Wall::sample(t));
        }
        n.tick(&ctx.ui, t)
    }

    fn next_wake(&self, t: f64) -> Option<f64> {
        self.notes.as_ref().and_then(|n| n.next_wake(t))
    }

    fn draw(&mut self, ctx: &Ctx, s: &mut Surface, t: f64) -> Damage {
        if let Some(n) = &mut self.notes {
            n.draw(s, &ctx.ui, t);
        }
        let mut d = Damage::new(s.w, s.h);
        d.all();
        d
    }
}


fn main() {
    bifrost_app::run(NotesApp { notes: None, last_wall: 0.0, export: None, import: None, indexed: Default::default(), next_index: 0.0 });
}
