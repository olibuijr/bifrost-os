//! Notes as an installed BifrOSt desktop app. The note code is the shared
//! Notes module (`apps/notes/src/notes`), shared unchanged; this crate only
//! hosts it through the SDK for the Wayland desktop.

use notes::notes;
use ::notes::notes::system;

use bifrost_app::kit::{Damage, Event, Surface};
use bifrost_app::app_windows::WindowId;
use bifrost_app::windows::{self, Level};
use ::notes::notes::note_windows;
use bifrost_app::widgets_timeline;
use bifrost_app::KeyInput;
use bifrost_app::proto::{DocErrorCode, DocMode};
use bifrost_app::{App, AppMenu, AppMenuItem, Command, Commands, Ctx, DocumentEvent, Input, TouchKind};
use bifrost_app::control::{RpcError, INVALID_PARAMS};
use bifrost_app::app_events::AppEvent;
use ::notes::scripting;
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
    /// the open note's text state and caret (window points) last told to the shell (`sync_text`)
    text_published: Option<TextReport>,
    /// Internet Accounts: the Exchange account whose Notes service is on is read and written in the background
    exchange: bifrost_app::exchange_notes::Sync,
}

/// What the main window last told the shell of the open note: its text state and caret rectangle in window points.
type TextReport = (bifrost_app::proto::TextState, (f32, f32, f32, f32));

/// The open note's text report (`Notes::text_report`, px) in window points at `ui`'s scale, while the main window has
/// the keys; `None` when no note is edited there.
fn text_report_pt(n: &notes::Notes, ctx: &Ctx) -> Option<TextReport> {
    if !ctx.key_window().is_main() {
        return None;
    }
    let (state, (x, y, w, h)) = n.text_report(&ctx.ui)?;
    let s = if ctx.ui.s > 0.0 { ctx.ui.s } else { 1.0 };
    Some((state, (x / s, y / s, w / s, h / s)))
}

/// The list view as a restoration key: `all`, `trash` or `folder:<name>`.
fn view_key(view: &notes::View) -> String {
    match view {
        notes::View::All => "all".into(),
        notes::View::Trash => "trash".into(),
        notes::View::Folder(f) => format!("folder:{f}"),
        notes::View::Smart(s) => format!("smart:{}", s.key()),
        notes::View::Tag(t) => format!("tag:{t}"),
    }
}

fn view_from_key(key: &str) -> notes::View {
    match key {
        "trash" => notes::View::Trash,
        k => {
            if let Some(s) = k.strip_prefix("smart:").and_then(notes::smart::Smart::from_key) {
                return notes::View::Smart(s);
            }
            if let Some(t) = k.strip_prefix("tag:").filter(|t| !t.is_empty()) {
                return notes::View::Tag(t.to_string());
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
    /// Tells the shell about the open note's text and caret when they changed (`ctx.text_state`, `ctx.caret_rect`), so
    /// Look Up and the Services menu find the selection or the word at the caret and Look Up opens under it. Once no
    /// note is edited in the main window (the list, a field, a note window with the keys) it reports an empty state.
    fn sync_text(&mut self, ctx: &Ctx) {
        let Some(n) = &self.notes else { return };
        let now = text_report_pt(n, ctx);
        if now == self.text_published {
            return;
        }
        match &now {
            Some((state, (x, y, w, h))) => {
                let last = self.text_published.as_ref();
                if last.map(|p| &p.0) != Some(state) {
                    ctx.text_state(state.clone());
                }
                // an empty state also clears the shell's caret rectangle: send it again after one
                if last.map(|p| p.1) != Some((*x, *y, *w, *h)) || *state == bifrost_app::proto::TextState::default() {
                    ctx.caret_rect(*x, *y, *w, *h);
                }
            }
            None => ctx.text_state(bifrost_app::proto::TextState::default()),
        }
        self.text_published = now;
    }

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
                let mut set = CSSearchableItemAttributeSet::new(&note.plain_title()).content_description(&body);
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
const MENU_PRINT: u32 = 5;
const MENU_COPY_MD: u32 = 6;
const MENU_PASTE_MD: u32 = 7;
const MENU_PASTE_PLAIN: u32 = 8;
const MENU_MATH: u32 = 9;
/// The Speech submenu: start speaking the selection (else the note), and stop.
const MENU_SPEAK_START: u32 = 10;
const MENU_SPEAK_STOP: u32 = 11;
/// View > as List / as Gallery.
const MENU_LAYOUT_LIST: u32 = 12;
const MENU_LAYOUT_GALLERY: u32 = 13;
/// Note > Open in Separate Window.
const MENU_OPEN_WINDOW: u32 = 14;
/// Window > Float on Top, for the focused note window.
const MENU_FLOAT_ON_TOP: u32 = 15;
/// Note > Sync Exchange Notes: a round now, not at the next interval.
const MENU_EXCHANGE: u32 = 17;
/// Notes > Settings: the pane over the window (text size, new note start, dark background, sort and grouping).
const MENU_SETTINGS: u32 = 16;
/// File > New Folder: the folder name prompt over the list.
const MENU_NEW_FOLDER: u32 = 18;
/// The Recent Notes submenu: `MENU_RECENT + n` is the n-th newest note.
const MENU_RECENT: u32 = 100;
/// The Format menu: `FORMAT_BASE + n` is the n-th row of `format_table`, `TABLE_BASE + n` the n-th table command.
const FORMAT_BASE: u32 = 200;
const TABLE_BASE: u32 = 300;
/// the menu bar extra's entries: New Quick Note and Resume Last Quick Note (their own id space, from `status_item_action`)
const QUICK_NEW: u32 = 1;
const QUICK_RESUME: u32 = 2;

/// What a Format menu row does.
#[derive(Clone, Copy)]
enum FormatCmd {
    Style(springboard::shell::md::Block),
    Do(notes::Format),
}

/// The Format menu rows in order: title, key equivalent and command; `None` is a separator.
fn format_table() -> Vec<Option<(&'static str, &'static str, FormatCmd)>> {
    use springboard::shell::md::Block;
    use ::notes::notes::{Fmt, Format};
    use FormatCmd::{Do, Style};
    vec![
        Some(("Title", "cmd+shift+t", Style(Block::Title))),
        Some(("Heading", "cmd+shift+h", Style(Block::Heading))),
        Some(("Subheading", "cmd+shift+j", Style(Block::Subheading))),
        Some(("Body", "cmd+shift+b", Style(Block::Body))),
        Some(("Monostyled", "cmd+shift+m", Style(Block::Mono))),
        Some(("Block Quote", "", Style(Block::Quote))),
        None,
        Some(("Bulleted List", "", Style(Block::Bullet))),
        Some(("Dashed List", "", Style(Block::Dashed))),
        Some(("Numbered List", "", Style(Block::Numbered(1)))),
        Some(("Checklist", "cmd+shift+l", Style(Block::Check(false)))),
        None,
        Some(("Bold", "cmd+b", Do(Format::Char(Fmt::Bold)))),
        Some(("Italic", "cmd+i", Do(Format::Char(Fmt::Italic)))),
        Some(("Underline", "cmd+u", Do(Format::Char(Fmt::Underline)))),
        Some(("Strikethrough", "cmd+shift+x", Do(Format::Char(Fmt::Strike)))),
        Some(("Highlight", "cmd+shift+e", Do(Format::Char(Fmt::Highlight)))),
        None,
        Some(("Increase Indent", "", Do(Format::Indent))),
        Some(("Decrease Indent", "", Do(Format::Outdent))),
        Some(("Move Item Up", "ctrl+cmd+up", Do(Format::MoveUp))),
        Some(("Move Item Down", "ctrl+cmd+down", Do(Format::MoveDown))),
        None,
        Some(("Mark All", "", Do(Format::CheckAll))),
        Some(("Unmark All", "", Do(Format::UncheckAll))),
        None,
        Some(("Add Link...", "cmd+k", Do(Format::AddLink))),
        Some(("Calculate Math Results", "", Do(Format::CalculateMath))),
        None,
        Some(("Collapse Section", "", Do(Format::CollapseSection))),
        Some(("Expand Section", "", Do(Format::ExpandSection))),
        Some(("Collapse All Sections", "", Do(Format::CollapseAll))),
        Some(("Expand All Sections", "", Do(Format::ExpandAll))),
    ]
}

/// The command a Format menu id stands for.
fn format_command(id: u32) -> Option<FormatCmd> {
    if let Some(row) = id.checked_sub(FORMAT_BASE).and_then(|i| format_table().get(i as usize).copied()) {
        return row.map(|(_, _, cmd)| cmd);
    }
    let i = id.checked_sub(TABLE_BASE)?;
    notes::table::Op::ALL.get(i as usize).map(|op| FormatCmd::Do(notes::Format::Table(*op)))
}

/// The View menu: the note list as rows or as a gallery of cards.
fn view_menu(n: &notes::Notes) -> AppMenu {
    let layout = n.prefs().layout;
    AppMenu {
        title: "View".into(),
        items: vec![
            AppMenuItem::new(MENU_LAYOUT_LIST, "as List").checked(layout == notes::gallery::Layout::List),
            AppMenuItem::new(MENU_LAYOUT_GALLERY, "as Gallery").checked(layout == notes::gallery::Layout::Gallery),
        ],
    }
}

/// The Format menu for the open note (every row is disabled while no note is open).
fn format_menu(n: &notes::Notes) -> AppMenu {
    let editing = n.is_editing();
    let ops = n.table_ops();
    let mut items = vec![];
    for (i, row) in format_table().into_iter().enumerate() {
        match row {
            None => items.push(AppMenuItem::separator()),
            Some((title, key, cmd)) => {
                let check_all = matches!(cmd, FormatCmd::Do(notes::Format::CheckAll | notes::Format::UncheckAll));
                let on = if check_all { n.has_checklist() } else { editing };
                let item = AppMenuItem::new(FORMAT_BASE + i as u32, title).enabled(on);
                items.push(if key.is_empty() { item } else { item.key(key) });
            }
        }
    }
    items.push(AppMenuItem::separator());
    for (i, op) in notes::table::Op::ALL.into_iter().enumerate() {
        items.push(AppMenuItem::new(TABLE_BASE + i as u32, op.label()).enabled(ops.contains(&op)));
    }
    AppMenu { title: "Format".into(), items }
}

/// The menu bar extra (Notes' status item): New Quick Note starts a note from the menu bar, as Cmd+N does.
fn quick_note_item(ctx: &Ctx) -> StatusItem {
    let item = StatusBar::system().status_item(StatusItemLength::Variable);
    item.set_symbol(ctx, "square.and.pencil");
    item.set_menu(ctx, &[StatusMenuItem::new(QUICK_NEW, "New Quick Note"), StatusMenuItem::new(QUICK_RESUME, "Resume Last Quick Note")]);
    item
}
/// evdev Z (the Undo and Redo chords are the editor's: Cmd+Z, Cmd+Shift+Z).
const KEY_Z: u32 = 44;
/// evdev C: Cmd+C copies the editor's selection to the pasteboard (rich text with a plain fallback); X cuts it.
const KEY_C: u32 = 46;
const KEY_X: u32 = 45;

impl NotesApp {
    /// Opens a note in a window of its own (`Ctx::open_window`). A note that is already open is focused, not opened
    /// twice; a window the shell refuses leaves the note where it was.
    fn open_note_window(&mut self, ctx: &Ctx, id: &str) -> bool {
        let Some(n) = &mut self.notes else { return false };
        if let Some(window) = n.windows.window_of(id) {
            n.windows.focus(window);
            return true;
        }
        let Some(title) = n.note_title(id) else { return false };
        match ctx.open_window(note_windows::window_spec(&title)) {
            Ok(window) => {
                n.windows.bind(window, id);
                n.windows.focus(window);
                true
            }
            Err(_) => false,
        }
    }

    /// Edit > Copy (or Cut, with `cut`): the focused field's selection, or the note's selection, which goes to the
    /// pasteboard as rich text with a plain fallback (`system::copy`). True when the window changed.
    fn copy_selection(&mut self, ctx: &Ctx, cut: bool) -> bool {
        let now = bifrost_app::proto::sys::now_ms();
        let Some(n) = &mut self.notes else { return false };
        if n.field_focused() {
            return n.field_command(&ctx.ui, if cut { "Cut" } else { "Copy" }, now);
        }
        let Some(markdown) = n.take_selection(&ctx.ui, cut, now) else { return false };
        system::copy(ctx, &markdown);
        cut
    }

    /// Edit > Copy (or Cut) in a note window: its selection goes to the pasteboard as the main editor's does.
    fn copy_window_selection(&mut self, ctx: &Ctx, window: WindowId, cut: bool) -> bool {
        let now = bifrost_app::proto::sys::now_ms();
        let Some(n) = &mut self.notes else { return false };
        let Some(markdown) = n.window_take_selection(&ctx.ui, window, cut, now) else { return false };
        system::copy(ctx, &markdown);
        cut
    }

    /// File > Import to Notes (Cmd+O): the open panel answers in `document`.
    fn open_import(&mut self, ctx: &Ctx) {
        self.import = Some(ctx.open_document(DocMode::Read, "Import to Notes", &["public.text", "public.html", "public.rtf"]));
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

/// The AppEvents Notes answers: `make new note` (`notes.make`, through the Create Note intent) and the core `getd` read
/// of a note property. Setting properties and deleting notes from a script are refused for now.
fn app_event(app: &mut NotesApp, ctx: Option<&Ctx>, data_dir: &std::path::Path, ev: &AppEvent) -> Result<bifrost_core::json::Json, RpcError> {
    use bifrost_core::json::Json;
    match (ev.class.as_str(), ev.id.as_str()) {
        ("notes", "make") => {
            let text = scripting::note_text(&ev.params)?;
            let made = app.perform_intent(ctx, data_dir, "create-note", &Json::Obj(vec![("text".into(), Json::Str(text))]))?;
            Ok(made.to_json())
        }
        ("core", "getd") => {
            let object = ev.params.get("object").cloned().ok_or_else(|| RpcError::new(INVALID_PARAMS, "getd needs an object"))?;
            let property = object.str_field("property");
            let rows = notes::rpc::call(data_dir, "notes.list", &Json::Obj(vec![]))?;
            scripting::get_property(&rows, &object, property.as_deref())
        }
        ("core", "setd") | ("core", "delt") => Err(RpcError::new(bifrost_app::control::METHOD_NOT_FOUND, "Notes cannot change notes from a script yet")),
        _ => Err(scripting::unhandled(&ev.method())),
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
        // "File" merges into the shell's File menu (New Note, New Folder, Import and Print, as in the reference);
        // "Note" and "Format" are the app's own. Undo, Redo, Cut, Copy, Paste and Select All are the shell's Edit
        // commands (`commands` and `Input::Command`), not items here.
        let editing = n.is_editing();
        let file = AppMenu {
            title: "File".into(),
            items: vec![
                AppMenuItem::new(MENU_NEW, "New Note").key("cmd+n"),
                AppMenuItem::new(MENU_NEW_FOLDER, "New Folder").key("cmd+shift+n"),
                AppMenuItem::separator(),
                AppMenuItem::new(MENU_IMPORT, "Import to Notes...").key("cmd+o"),
                AppMenuItem::separator(),
                AppMenuItem::new(MENU_PRINT, "Print...").key("cmd+p").enabled(editing),
            ],
        };
        let mut note = vec![
            AppMenuItem::new(MENU_DELETE, "Delete Note").key("cmd+backspace").enabled(n.can_delete()),
            AppMenuItem::separator(),
            AppMenuItem::new(MENU_FIND, "Find").key("cmd+f"),
            AppMenuItem::separator(),
            AppMenuItem::new(MENU_COPY_MD, "Copy as Markdown").enabled(n.selected_markdown().is_some()),
            AppMenuItem::new(MENU_PASTE_MD, "Paste as Markdown").enabled(editing),
            AppMenuItem::new(MENU_PASTE_PLAIN, "Paste and Match Style").key("cmd+alt+shift+v").enabled(editing),
            AppMenuItem::separator(),
            AppMenuItem::new(MENU_MATH, "Math Results").checked(n.math_results()),
        ];
        let recent = n.recent_notes();
        if !recent.is_empty() {
            note.push(AppMenuItem::separator());
            note.push(AppMenuItem::submenu("Recent Notes"));
            for (i, (_, title)) in recent.iter().enumerate() {
                note.push(AppMenuItem::new(MENU_RECENT + i as u32, title).depth(1));
            }
        }
        note.push(AppMenuItem::separator());
        note.push(AppMenuItem::submenu("Speech"));
        note.push(AppMenuItem::new(MENU_SPEAK_START, "Start Speaking").enabled(n.speakable_text().is_some()).depth(1));
        note.push(AppMenuItem::new(MENU_SPEAK_STOP, "Stop Speaking").depth(1));
        note.push(AppMenuItem::separator());
        note.push(AppMenuItem::new(MENU_OPEN_WINDOW, "Open in Separate Window").enabled(editing));
        note.push(AppMenuItem::new(MENU_EXCHANGE, "Sync Exchange Notes").enabled(!self.exchange.busy()));
        note.push(AppMenuItem::new(MENU_SETTINGS, "Settings...").key("cmd+,"));
        let focused = n.windows.focused();
        let float = AppMenu {
            title: "Window".into(),
            items: vec![AppMenuItem::new(MENU_FLOAT_ON_TOP, "Float on Top").enabled(focused.is_some()).checked(focused.is_some_and(|w| n.windows.is_floating(w)))],
        };
        vec![file, AppMenu { title: "Note".into(), items: note }, view_menu(n), format_menu(n), float]
    }

    fn commands(&self, ctx: &Ctx) -> Commands {
        let Some(n) = &self.notes else { return Commands::default() };
        // a note window that has the keys edits its own text: the Edit commands follow its selection and history
        let key = ctx.key_window();
        if !key.is_main() && n.windows.note_of(key).is_some() {
            let (undo, redo) = n.window_can_undo_redo(key);
            let selected = n.window_has_selection(key);
            return Commands::default()
                .with(Command::Undo, undo)
                .with(Command::Redo, redo)
                .with(Command::Cut, selected)
                .with(Command::Copy, selected)
                .with(Command::Paste, bifrost_app::kit::clipboard::text().is_some_and(|s| !s.is_empty()))
                .with(Command::SelectAll, true);
        }
        let (undo, redo) = n.can_undo_redo();
        // the note or a focused field takes the text commands (Edit menu, Cmd+X, C, V, A)
        let text = n.is_editing() || n.field_focused();
        let selected = n.has_selection();
        Commands::default()
            .with(Command::Undo, undo)
            .with(Command::Redo, redo)
            .with(Command::Cut, selected)
            .with(Command::Copy, selected)
            .with(Command::Paste, text && bifrost_app::kit::clipboard::text().is_some_and(|s| !s.is_empty()))
            .with(Command::SelectAll, text)
    }

    /// Edit > Undo and Redo name the step of the window that has the keys: a note window's own history, or the main
    /// editor's.
    fn undo_titles(&self, ctx: &Ctx) -> (String, String) {
        let Some(n) = &self.notes else { return Default::default() };
        let key = ctx.key_window();
        if !key.is_main() && n.windows.note_of(key).is_some() {
            return n.window_undo_titles(key);
        }
        n.undo_titles()
    }

    /// Look Up and the Services menu act on the selection of the window that has the keys, as plain text; with nothing
    /// selected the shell takes the word at the caret from the reported text state.
    fn selected_text(&mut self, ctx: &Ctx) -> Option<String> {
        self.notes.as_ref()?.look_up_selection(ctx.key_window())
    }

    /// Where the open note's caret is (window points), sent before the selection so Look Up opens under it.
    fn selection_rect(&mut self, ctx: &Ctx) -> Option<(f32, f32, f32, f32)> {
        text_report_pt(self.notes.as_ref()?, ctx).map(|(_, rect)| rect)
    }

    /// An input method's composition in the main window: the search field or the open note's editor.
    fn text_input(&mut self, ctx: &Ctx, ev: bifrost_app::proto::ToApp) -> bool {
        let now = bifrost_app::proto::sys::now_ms();
        self.notes.as_mut().is_some_and(|n| n.text_input(&ctx.ui, &ev, now))
    }

    /// The key window's composition: a note window composes in its own text, the main window in `text_input`.
    fn window_text_input(&mut self, ctx: &Ctx, window: WindowId, ev: bifrost_app::proto::ToApp) -> bool {
        if window.is_main() {
            return self.text_input(ctx, ev);
        }
        let now = bifrost_app::proto::sys::now_ms();
        self.notes.as_mut().is_some_and(|n| n.window_text_input(&ctx.ui, window, &ev, now))
    }

    /// A chosen entry of the menu bar extra: New Quick Note starts a note (or reopens the last quick note when the
    /// list menu's "resumes last" setting is on), and Resume Last Quick Note reopens it.
    fn status_item_action(&mut self, ctx: &Ctx, item: u32, id: u32) -> bool {
        if self.quick.as_ref().map(|q| q.id()) != Some(item) {
            return false;
        }
        let now = bifrost_app::proto::sys::now_ms();
        let Some(n) = &mut self.notes else { return false };
        match id {
            QUICK_NEW => n.quick_note(&ctx.ui, false, now),
            QUICK_RESUME => n.quick_note(&ctx.ui, true, now),
            _ => return false,
        }
        true
    }

    fn menu_action(&mut self, ctx: &Ctx, id: u32) -> bool {
        if id == MENU_IMPORT {
            self.open_import(ctx);
            return false;
        }
        let now = bifrost_app::proto::sys::now_ms();
        let Some(n) = &mut self.notes else { return false };
        match id {
            MENU_PRINT => {
                if let Some(text) = n.printable_text() {
                    let title: String = text.lines().next().unwrap_or("Note").chars().take(60).collect();
                    // the job goes to CUPS on the shared worker pool, so the window keeps drawing while the scheduler answers
                    let job = bifrost_app::print::PrintOperation::text(title, text);
                    bifrost_app::dispatch::DispatchQueue::global(bifrost_app::dispatch::Qos::Utility).async_(move || {
                        if let Err(e) = job.run() {
                            eprintln!("Notes print: {e:?}");
                        }
                    });
                }
                false
            }
            MENU_NEW => {
                n.compose(&ctx.ui, now);
                true
            }
            MENU_NEW_FOLDER => {
                n.new_folder(&ctx.ui, now);
                true
            }
            MENU_DELETE => n.delete_current(&ctx.ui, now),
            MENU_SPEAK_START => {
                if let Some(text) = n.speakable_text() {
                    bifrost_app::speech::speak(ctx, &bifrost_app::speech::Utterance::new(&text));
                }
                false
            }
            MENU_SPEAK_STOP => {
                bifrost_app::speech::stop(ctx);
                false
            }
            MENU_COPY_MD => {
                if let Some(markdown) = n.selected_markdown() {
                    system::copy_markdown(ctx, &markdown);
                }
                false
            }
            MENU_PASTE_MD => n.paste_markdown(&ctx.ui, now),
            MENU_PASTE_PLAIN => n.paste_match_style(&ctx.ui, now),
            MENU_MATH => {
                n.toggle_math_results();
                true
            }
            MENU_LAYOUT_LIST | MENU_LAYOUT_GALLERY => {
                let layout = if id == MENU_LAYOUT_GALLERY { notes::gallery::Layout::Gallery } else { notes::gallery::Layout::List };
                n.set_layout(&ctx.ui, layout);
                true
            }
            id if (MENU_RECENT..MENU_RECENT + notes::RECENT_MENU as u32).contains(&id) => {
                let recent = n.recent_notes();
                match recent.get((id - MENU_RECENT) as usize) {
                    Some((note, _)) => n.show_note(&ctx.ui, note, now),
                    None => false,
                }
            }
            id if format_command(id).is_some() => match format_command(id) {
                Some(FormatCmd::Style(block)) => n.style(&ctx.ui, block, now),
                Some(FormatCmd::Do(f)) => n.format(&ctx.ui, f, now),
                None => false,
            },
            MENU_FIND => {
                n.find(&ctx.ui, now);
                true
            }
            MENU_EXCHANGE => {
                self.exchange.refresh();
                false
            }
            MENU_OPEN_WINDOW => {
                n.request_window();
                true
            }
            MENU_SETTINGS => {
                n.open_settings();
                true
            }
            MENU_FLOAT_ON_TOP => {
                let Some(n) = &mut self.notes else { return false };
                let Some(window) = n.windows.focused() else { return false };
                let Some(on) = n.windows.toggle_floating(window) else { return false };
                windows::set_window_level(ctx, window, if on { Level::Floating } else { Level::Normal });
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
            Intent::new("open-note", "Open Note", "Opens the note with the given title.").param(Param::new("note", "Note", ParamType::String)),
            Intent::new("open-folder", "Open Folder", "Shows the notes of a folder.").param(Param::new("folder", "Folder", ParamType::String)),
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
            "open-note" => {
                let query = intents::str_param(params, "note")?;
                let (Some(ctx), Some(n)) = (ctx, self.notes.as_mut()) else { return Err(intents::failed("Notes is not running yet")) };
                match n.open_named(&ctx.ui, query, now) {
                    Some(title) => Ok(IntentResult::value(Json::Str(title.clone())).with_dialog(&format!("Opened {title}"))),
                    None => Err(intents::failed(format!("No note matches {query}"))),
                }
            }
            "open-folder" => {
                let name = intents::str_param(params, "folder")?;
                let (Some(ctx), Some(n)) = (ctx, self.notes.as_mut()) else { return Err(intents::failed("Notes is not running yet")) };
                if n.open_folder(&ctx.ui, name, now) {
                    Ok(IntentResult::value(Json::Str(name.to_string())).with_dialog(&format!("Showing {name}")))
                } else {
                    Err(intents::failed(format!("No folder named {name}")))
                }
            }
            _ => Err(RpcError::new(INVALID_PARAMS, format!("no intent {id}"))),
        }
    }

    fn rpc(&mut self, ctx: Option<&Ctx>, data_dir: &std::path::Path, method: &str, params: &bifrost_core::json::Json) -> Result<bifrost_core::json::Json, bifrost_app::control::RpcError> {
        if method == "notes.state" {
            return Ok(bifrost_core::json::Json::Str(self.notes.as_ref().map_or_else(String::new, |n| n.describe())));
        }
        // the shell's preview panel asks for a picture of a note (`[[preview]]` in the manifest)
        if method == ::notes::notes::preview::METHOD {
            return ::notes::notes::preview::handle(ctx.map(|c| &c.ui), data_dir, params);
        }
        // scripting: `tell app "Notes"` commands arrive as AppEvents (see `scripting` and the manifest's [[script.*]])
        if let Some(ev) = AppEvent::from_method(method, params) {
            return app_event(self, ctx, data_dir, &ev);
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
            // a notes://note/<id> link, from another app or from a note: shows that note when it still exists
            Input::Open(url) => {
                let Some(id) = notes::note_url::note_id(&url) else { return false };
                n.show_note(&ctx.ui, id, bifrost_app::proto::sys::now_ms())
            }
            // a right click: the menu for the note row or the folder under the pointer
            Input::Context { x, y } => {
                let rows = n.context_menu(&ctx.ui, x, y, bifrost_app::proto::sys::now_ms());
                if !rows.is_empty() {
                    ctx.context_menu(x, y, rows.into_iter().map(|(id, title)| bifrost_app::proto::ContextItem::new(id, &title)).collect());
                }
                true
            }
            Input::ContextAction(id) => n.context_action(&ctx.ui, id, bifrost_app::proto::sys::now_ms()),
            Input::Touch { kind, x, y, t, mods, .. } => {
                n.set_pointer_shift(mods.contains(bifrost_app::proto::ModifierFlags::SHIFT));
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
            // the shell's Edit menu (and Cmd+X, C, V, A, Z): Undo and Redo are the editor's own, as the Cmd+Z chords
            // are; Cut, Copy, Paste and Select All act on the focused field or the note
            Input::Command(command) => {
                let now = bifrost_app::proto::sys::now_ms();
                let mods = match command {
                    Command::Undo => bifrost_app::proto::CHORD_META,
                    Command::Redo => bifrost_app::proto::CHORD_META | bifrost_app::proto::CHORD_SHIFT,
                    Command::Cut => return self.copy_selection(ctx, true),
                    Command::Copy => return self.copy_selection(ctx, false),
                    Command::Paste => return n.paste(&ctx.ui, now),
                    Command::SelectAll => return n.select_all(&ctx.ui, now),
                };
                n.chord(&ctx.ui, KEY_Z, mods, now)
            }
            Input::Key(k) => {
                if let Some((key, mods)) = k.as_chord() {
                    use bifrost_app::proto::{CHORD_ALT, CHORD_CTRL, CHORD_META, CHORD_SHIFT};
                    if key == KEY_O && mods & (CHORD_CTRL | CHORD_META) != 0 && mods & (CHORD_SHIFT | CHORD_ALT) == 0 {
                        self.open_import(ctx);
                        return false;
                    }
                    // Copy and Cut with a selection in the note go to the pasteboard (a field's selection is the field's)
                    if (key == KEY_C || key == KEY_X) && mods & (CHORD_CTRL | CHORD_META) != 0 && mods & (CHORD_SHIFT | CHORD_ALT) == 0 && !n.field_focused() {
                        return self.copy_selection(ctx, key == KEY_X);
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
                // HTML and RTF documents become Markdown; text and Markdown come in as they are
                let text = notes::import::convert(&doc.name, &bytes);
                n.import_note(&ctx.ui, &text, bifrost_app::proto::sys::now_ms());
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
    fn service_reply(&mut self, ctx: &Ctx, request: u32, result: &Result<Json, String>) -> bool {
        // the password the Exchange sync asked Passwords for
        if self.exchange.reply(request, result) {
            return false;
        }
        if let Err(e) = result {
            ctx.log(&format!("service: {e}"));
        }
        true
    }

    /// The shell asks Notes to index again (`index_extension = true` in its manifest). A reindex-all clears the Search
    /// index of Notes and indexes every note again; identifiers are indexed again on their own.
    fn index_request(&mut self, ctx: &Ctx, request: bifrost_app::quicksearch::IndexRequest) -> bool {
        use bifrost_app::quicksearch::{CSSearchableIndex, IndexRequest};
        match request {
            IndexRequest::ReindexAll => {
                CSSearchableIndex::default_index().delete_all_searchable_items(ctx);
                self.indexed.clear();
            }
            IndexRequest::ReindexIdentifiers(ids) => {
                for id in &ids {
                    self.indexed.remove(id);
                }
            }
        }
        self.sync_index(ctx);
        false
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
        // Internet Accounts > Exchange: a finished round is taken into the Notes folder of the account
        let mut exchanged = false;
        if let Some(done) = self.exchange.poll_local(ctx, t, &mut |account| n.exchange_plan(account)) {
            let applied = n.exchange_apply(&ctx.ui, t, &done.account, &done.report);
            for e in &applied.errors {
                ctx.log(&format!("exchange notes: {e}"));
            }
            exchanged = applied.changed();
        }
        let redraw = n.tick(&ctx.ui, t) || exchanged;
        let requested = n.take_window_request();
        self.track_restorable(ctx);
        self.sync_text(ctx);
        let opened = requested.is_some_and(|id| self.open_note_window(ctx, &id));
        redraw || opened
    }

    /// The scroll wheel comes as itself rather than as a drag, so a drag on the note can select text.
    fn pointer_interest(&self) -> u32 {
        bifrost_app::pointer_events::WHEEL
    }

    fn pointer(&mut self, ctx: &Ctx, ev: &bifrost_app::pointer_events::PointerEvent) -> bool {
        let Some(n) = &mut self.notes else { return false };
        if let bifrost_app::pointer_events::PointerEvent::Wheel { x, y, dy, .. } = ev {
            return n.wheel(&ctx.ui, *x, *y, *dy);
        }
        false
    }

    /// The wheel over a note window (window points, turned into the px the window is drawn in) scrolls its text.
    fn window_pointer(&mut self, ctx: &Ctx, window: WindowId, ev: &bifrost_app::pointer_events::PointerEvent) -> bool {
        if window.is_main() {
            return self.pointer(ctx, ev);
        }
        let Some(n) = &mut self.notes else { return false };
        match ev {
            bifrost_app::pointer_events::PointerEvent::Wheel { dy, .. } => n.window_wheel(&ctx.ui, window, *dy * ctx.ui.pt(1.0)),
            _ => false,
        }
    }

    fn next_wake(&self, t: f64) -> Option<f64> {
        self.notes.as_ref().and_then(|n| n.next_wake(t))
    }

    /// A note window draws its note (`Notes::draw_window`), with its caret while it is the key window; any other extra
    /// window draws nothing.
    fn draw_window(&mut self, ctx: &Ctx, window: WindowId, s: &mut Surface, t: f64) -> Damage {
        if let Some(n) = &mut self.notes {
            n.draw_window(s, &ctx.ui, window, ctx.key_window() == window, t);
        }
        self.sync_text(ctx);
        let mut d = Damage::new(s.w, s.h);
        d.all();
        d
    }

    /// A note window edits its note as the main editor does on the desktop: the mouse places the caret and selects
    /// (a press also focuses the window, which the Window menu's Float on Top acts on), the keys the shell sends to the
    /// key window type and run the editing bindings, and the Edit commands (Cut, Copy, Paste, Select All, Undo, Redo
    /// and their Cmd chords) act on its text.
    fn window_event(&mut self, ctx: &Ctx, window: WindowId, ev: Input) -> bool {
        if window.is_main() {
            return self.event(ctx, ev);
        }
        let now = bifrost_app::proto::sys::now_ms();
        let Some(n) = &mut self.notes else { return false };
        if n.windows.note_of(window).is_none() {
            return false;
        }
        match ev {
            Input::Touch { kind, x, y, t, mods, .. } => {
                n.set_pointer_shift(mods.contains(bifrost_app::proto::ModifierFlags::SHIFT));
                let press = match kind {
                    TouchKind::Down => note_windows::Press::Down,
                    TouchKind::Move => note_windows::Press::Move,
                    TouchKind::Up | TouchKind::Cancel => note_windows::Press::Up,
                };
                // an extra window's touches arrive in window points (`WindowTouch`); the text is laid out in px
                let k = ctx.ui.pt(1.0);
                n.window_pointer(&ctx.ui, window, press, x * k, y * k, t)
            }
            Input::Command(command) => match command {
                Command::Cut => self.copy_window_selection(ctx, window, true),
                Command::Copy => self.copy_window_selection(ctx, window, false),
                Command::Paste => n.window_paste(&ctx.ui, window, now),
                Command::SelectAll => n.window_select_all(window),
                Command::Undo => n.window_undo(&ctx.ui, window, false, now),
                Command::Redo => n.window_undo(&ctx.ui, window, true, now),
            },
            Input::Key(k) => {
                if let Some((key, mods)) = k.as_chord() {
                    use bifrost_app::proto::{CHORD_ALT, CHORD_CTRL, CHORD_META, CHORD_SHIFT};
                    // Copy and Cut put the window's selection on the pasteboard, as in the main editor
                    if (key == KEY_C || key == KEY_X) && mods & (CHORD_CTRL | CHORD_META) != 0 && mods & (CHORD_SHIFT | CHORD_ALT) == 0 {
                        return self.copy_window_selection(ctx, window, key == KEY_X);
                    }
                    return n.window_chord(&ctx.ui, window, key, mods, now);
                }
                let out = match k {
                    KeyInput::Text(s) => Out::Text(s),
                    KeyInput::Code(8) => Out::Backspace,
                    KeyInput::Code(13) => Out::Enter,
                    KeyInput::Code(37) => Out::Named("Left"),
                    KeyInput::Code(38) => Out::Named("Up"),
                    KeyInput::Code(39) => Out::Named("Right"),
                    KeyInput::Code(40) => Out::Named("Down"),
                    KeyInput::Code(_) => return false,
                };
                n.window_key(&ctx.ui, window, out, now)
            }
            _ => false,
        }
    }

    /// The shell or the app closed a note window: it is no longer bound to its note.
    fn window_closed(&mut self, _ctx: &Ctx, window: WindowId) -> bool {
        self.notes.as_mut().is_some_and(|n| n.windows.unbind(window).is_some())
    }

    fn draw(&mut self, ctx: &Ctx, s: &mut Surface, t: f64) -> Damage {
        if let Some(n) = &mut self.notes {
            n.draw(s, &ctx.ui, t);
        }
        self.sync_text(ctx);
        let mut d = Damage::new(s.w, s.h);
        d.all();
        d
    }
}


fn main() {
    bifrost_app::run(NotesApp { notes: None, last_wall: 0.0, export: None, import: None, indexed: Default::default(), next_index: 0.0, restorable: None, widget: None, quick: None, text_published: None, exchange: bifrost_app::exchange_notes::Sync::new() });
}
