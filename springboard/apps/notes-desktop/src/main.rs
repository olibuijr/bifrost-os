//! Notes as an installed BifrOSt desktop app. The note code is the shared
//! Notes module (`apps/notes/src/notes`), shared unchanged; this crate only
//! hosts it through the SDK for the Wayland desktop.

use notes::notes;
use ::notes::notes::system;

use bifrost_app::kit::{Damage, Event, Surface};
use bifrost_app::widgets_timeline;
use bifrost_app::KeyInput;
use bifrost_app::proto::{DocErrorCode, DocMode};
use bifrost_app::{App, AppMenu, AppMenuItem, Command, Commands, Ctx, DocumentEvent, Input, TouchKind};
use bifrost_app::control::{RpcError, INVALID_PARAMS};
use bifrost_app::intents::{self, Intent, IntentResult, Param, ParamType};
use bifrost_app::status_item::{MenuItem as StatusMenuItem, StatusBar, StatusItem, StatusItemLength};
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
    /// what state restoration last saw (the list's view and the open note id), to notice a change
    restorable: Option<(notes::View, Option<String>)>,
    /// the recent-note titles last given to the widget timeline (sent again only when they change)
    widget: Option<Vec<String>>,
    /// the menu bar extra with New Quick Note, made once the window is configured
    quick: Option<StatusItem>,
}

/// The list view as a restoration key: `all`, `trash` or `folder:<name>`.
fn view_key(view: &notes::View) -> String {
    match view {
        notes::View::All => "all".into(),
        notes::View::Trash => "trash".into(),
        notes::View::Folder(f) => format!("folder:{f}"),
        notes::View::Smart(s) => format!("smart:{}", s.key()),
    }
}

fn view_from_key(key: &str) -> notes::View {
    match key {
        "trash" => notes::View::Trash,
        k => {
            if let Some(s) = k.strip_prefix("smart:").and_then(notes::smart::Smart::from_key) {
                return notes::View::Smart(s);
            }
            k.strip_prefix("folder:").map_or(notes::View::All, |f| notes::View::Folder(f.to_string()))
        }
    }
}

/// Search domain of every note.
const SPOT_DOMAIN: &str = "notes";
/// How often the app compares its notes with what it indexed (ms).
const INDEX_EVERY_MS: f64 = 3000.0;

impl NotesApp {
    /// `invalidateRestorableState()` when the view or the open note changed since the last look.
    fn track_restorable(&mut self, ctx: &Ctx) {
        let Some(n) = &self.notes else { return };
        let now = n.restorable();
        if self.restorable.as_ref() != Some(&now) {
            self.restorable = Some(now);
            ctx.invalidate_restorable_state();
        }
    }

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
                let body = system::index_body(&note.text);
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

    /// Gives the widget timeline the newest notes when their titles or order changed (or on `force`, when the shell
    /// asked for a reload). Nothing is sent while the list is the same.
    fn sync_widget(&mut self, ctx: &Ctx, force: bool) {
        let Some(n) = &self.notes else { return };
        let titles = system::recent_titles(&n.indexable());
        if !force && self.widget.as_ref() == Some(&titles) {
            return;
        }
        system::reload_widget(ctx, &titles, bifrost_app::proto::sys::now_ms() / 1000.0);
        self.widget = Some(titles);
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
/// the menu bar extra's entry: New Quick Note (its own id space, from `status_item_action`)
const QUICK_NEW: u32 = 1;

/// The menu bar extra (Notes' status item): New Quick Note starts a note from the menu bar, as Cmd+N does.
fn quick_note_item(ctx: &Ctx) -> StatusItem {
    let item = StatusBar::system().status_item(StatusItemLength::Variable);
    item.set_symbol(ctx, "square.and.pencil");
    item.set_menu(ctx, &[StatusMenuItem::new(QUICK_NEW, "New Quick Note")]);
    item
}
/// evdev Z (the Undo and Redo chords are the editor's: Cmd+Z, Cmd+Shift+Z).
const KEY_Z: u32 = 44;
/// evdev C: Cmd+C copies the editor's selection to the pasteboard (rich text with a plain fallback).
const KEY_C: u32 = 46;

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
                self.quick = Some(quick_note_item(ctx));
            }
        }
    }

    fn rpc_methods(&self) -> Vec<&'static str> {
        let mut methods = notes::rpc::methods();
        methods.push("notes.state");
        methods
    }

    fn restoration_identifier(&self) -> Option<&str> {
        Some("notes.main")
    }

    fn encode_restorable_state(&self, _ctx: &Ctx, state: &mut bifrost_app::RestorableState) {
        let Some(n) = &self.notes else { return };
        let (view, open) = n.restorable();
        state.encode_for("view", &view_key(&view));
        if let Some(id) = open {
            state.encode_for("note", &id);
        }
    }

    fn restore_state(&mut self, ctx: &Ctx, state: &bifrost_app::RestorableState) -> bool {
        let Some(n) = &mut self.notes else { return false };
        let view = state.decode_for::<String>("view").map_or(notes::View::All, |k| view_from_key(&k));
        let open = state.decode_for::<String>("note");
        let redraw = n.restore(&ctx.ui, view, open.as_deref(), bifrost_app::proto::sys::now_ms());
        self.restorable = Some(n.restorable());
        redraw
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
        let (undo, redo) = n.can_undo_redo();
        Commands::default().with(Command::Undo, undo).with(Command::Redo, redo)
    }

    fn undo_titles(&self, _ctx: &Ctx) -> (String, String) {
        self.notes.as_ref().map(|n| n.undo_titles()).unwrap_or_default()
    }

    /// A chosen entry of the menu bar extra: New Quick Note runs the same action as Note > New Note.
    fn status_item_action(&mut self, ctx: &Ctx, item: u32, id: u32) -> bool {
        if self.quick.as_ref().map(|q| q.id()) != Some(item) || id != QUICK_NEW {
            return false;
        }
        self.menu_action(ctx, MENU_NEW)
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
        if method == "notes.state" {
            return Ok(bifrost_core::json::Json::Str(self.notes.as_ref().map_or_else(String::new, |n| n.describe())));
        }
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
                    // Copy with a selection in the editor goes to the pasteboard; without one the chord is left alone
                    if key == KEY_C && mods & (CHORD_CTRL | CHORD_META) != 0 && mods & (CHORD_SHIFT | CHORD_ALT) == 0 {
                        if let Some(md) = n.selected_markdown() {
                            system::copy(ctx, &md);
                            return false;
                        }
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

    /// The widget timeline's reload request from the shell: the recent notes are sent again.
    fn service_event(&mut self, ctx: &Ctx, service: &str, event: &str, body: &Json) -> bool {
        if let Some(widgets_timeline::Event::Reload { kind }) = widgets_timeline::parse_event(service, event, body) {
            if kind == system::WIDGET_KIND {
                self.sync_widget(ctx, true);
            }
        }
        false
    }

    /// A service call's error (an older shell answers "no service ..." for a service it does not have) is logged;
    /// the app keeps going without the feature.
    fn service_reply(&mut self, ctx: &Ctx, _request: u32, result: &Result<Json, String>) -> bool {
        if let Err(e) = result {
            ctx.log(&format!("service: {e}"));
        }
        true
    }

    fn continue_activity(&mut self, ctx: &Ctx, activity: bifrost_app::quicksearch::UserActivity) -> bool {
        let now = bifrost_app::proto::sys::now_ms();
        activity.activity_type == bifrost_app::quicksearch::ACTION_TYPE && self.notes.as_mut().is_some_and(|n| n.show_note(&ctx.ui, &activity.unique_identifier, now))
    }

    fn tick(&mut self, ctx: &Ctx, t: f64) -> bool {
        if t >= self.next_index && self.notes.is_some() {
            self.next_index = t + INDEX_EVERY_MS;
            self.sync_index(ctx);
            self.sync_widget(ctx, false);
        }
        let Some(n) = &mut self.notes else { return false };
        if t - self.last_wall >= 1000.0 {
            self.last_wall = t;
            n.set_wall(Wall::sample(t));
        }
        let redraw = n.tick(&ctx.ui, t);
        self.track_restorable(ctx);
        redraw
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
    bifrost_app::run(NotesApp { notes: None, last_wall: 0.0, export: None, import: None, indexed: Default::default(), next_index: 0.0, restorable: None, widget: None, quick: None });
}
