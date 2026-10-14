//! Freeform as an installed BifrOSt app: the former in-shell Freeform,
//! unchanged, hosted through the SDK. It also talks to the shell's services: the cursor (the
//! pointer shape of each tool), the pasteboard (Copy) and the screens (the export scale).

mod arrange;
mod boards;
mod canvas_view;
mod freeform;
mod ink;
mod items;
mod pdfout;
mod services;
mod style;

use arrange::Align;
use bifrost_app::kit::{Damage, Event, Surface};
use freeform::Shared;
use ink::ShapeKind;
use bifrost_app::{App, AppMenu, AppMenuItem, Ctx, Document, DocumentEvent, Input, OpenOptions, TouchKind};
use bifrost_core::json::Json;
use std::io::Read;
use std::path::PathBuf;

/// The File menu's Export as PNG item.
const MENU_EXPORT: u32 = 1;
const MENU_UNDO: u32 = 2;
const MENU_REDO: u32 = 3;
const MENU_COPY: u32 = 4;
const MENU_PASTE: u32 = 5;
const MENU_DUPLICATE: u32 = 6;
const MENU_DELETE: u32 = 7;
const MENU_FRONT: u32 = 8;
const MENU_BACK: u32 = 9;
const MENU_ROW: u32 = 10;
const MENU_GROUP: u32 = 11;
const MENU_UNGROUP: u32 = 12;
const MENU_LOCK: u32 = 13;
const MENU_UNLOCK: u32 = 14;
const MENU_ALIGN_LEFT: u32 = 15;
const MENU_ALIGN_CENTER: u32 = 16;
const MENU_ALIGN_RIGHT: u32 = 17;
const MENU_ALIGN_TOP: u32 = 18;
const MENU_ALIGN_MIDDLE: u32 = 19;
const MENU_ALIGN_BOTTOM: u32 = 20;
const MENU_DISTRIBUTE_H: u32 = 21;
const MENU_DISTRIBUTE_V: u32 = 22;
const MENU_EXPORT_PDF: u32 = 23;
const MENU_PRINT: u32 = 24;
const MENU_NEW_BOARD: u32 = 25;
const MENU_DUPLICATE_BOARD: u32 = 26;
const MENU_DELETE_BOARD: u32 = 27;
const MENU_NEXT_BOARD: u32 = 28;
const MENU_PREV_BOARD: u32 = 29;
const MENU_GRID: u32 = 30;
const MENU_SNAP: u32 = 31;
const MENU_SEARCH: u32 = 32;
const MENU_RENAME_BOARD: u32 = 33;
const MENU_ZOOM_IN: u32 = 34;
const MENU_ZOOM_OUT: u32 = 35;
const MENU_ACTUAL_SIZE: u32 = 36;
const MENU_HAND: u32 = 37;
/// Insert menu shapes: `MENU_SHAPE + index` into `SHAPES`.
const MENU_SHAPE: u32 = 40;
const MENU_TEXT_BOX: u32 = 50;
const MENU_NOTE: u32 = 51;
const MENU_TABLE: u32 = 52;
const MENU_CONNECTOR: u32 = 53;
const MENU_LINK: u32 = 54;
const MENU_FILE: u32 = 55;
const MENU_TABLE_ROW: u32 = 56;
const MENU_TABLE_COLUMN: u32 = 57;
const MENU_LARGER: u32 = 58;
const MENU_SMALLER: u32 = 59;
/// The Boards menu: `MENU_BOARD + board id` opens that board.
const MENU_BOARD: u32 = 1000;

/// The shapes of the Insert menu, with their titles.
const SHAPES: [(ShapeKind, &str); 6] = [
    (ShapeKind::Line, "Line"),
    (ShapeKind::Arrow, "Arrow"),
    (ShapeKind::Rectangle, "Rectangle"),
    (ShapeKind::Oval, "Oval"),
    (ShapeKind::Triangle, "Triangle"),
    (ShapeKind::Star, "Star"),
];

struct FreeformApp {
    app: Option<freeform::Freeform>,
    services: services::Services,
    /// The open panel of Insert > Picture or File, while it is up.
    insert: Option<u32>,
}

impl FreeformApp {
    /// Shows the pointer shape of the point (x, y) (a service call only when the shape changes).
    fn show_pointer(&mut self, ctx: &Ctx, x: f32, y: f32) {
        let Some(c) = &self.app else { return };
        let kind = c.pointer_for(&ctx.ui, x, y);
        self.services.show_pointer(ctx, kind);
    }

    /// Puts a Copy's strokes on the pasteboard.
    fn drain(&mut self, ctx: &Ctx) {
        let Some(c) = &mut self.app else { return };
        if let Some(strokes) = c.take_copy() {
            self.services.copy(ctx, &strokes, ctx.ui.s);
        }
        if let Some(url) = c.take_open_url() {
            ctx.open_url(&url);
        }
    }

    /// The files an open panel or a share brought, read whole (a file over the board's limit is skipped and logged)
    /// and released: the board keeps its own copy.
    fn read_files(ctx: &Ctx, docs: Vec<Document>) -> Vec<(String, Vec<u8>)> {
        let limit = freeform::MAX_FILE_BYTES;
        let mut files = Vec::new();
        for mut doc in docs {
            let mut bytes = Vec::new();
            let read = (doc.size as usize) <= limit && (&mut doc.file).take(limit as u64 + 1).read_to_end(&mut bytes).is_ok();
            ctx.release_document(doc.grant);
            if read && bytes.len() <= limit {
                files.push((doc.name, bytes));
            } else {
                ctx.log(&format!("freeform: {} was not read (too large or unreadable)", doc.name));
            }
        }
        files
    }

    /// Files from an open panel go onto the open board; files somebody shared go onto a board of their own.
    fn take_files(&mut self, ctx: &Ctx, request: u32, docs: Vec<Document>) -> bool {
        let inbound = bifrost_app::share::is_inbound(request);
        if !inbound && self.insert != Some(request) {
            for doc in docs {
                ctx.release_document(doc.grant);
            }
            return false;
        }
        if !inbound {
            self.insert = None;
        }
        let files = Self::read_files(ctx, docs);
        let Some(c) = &mut self.app else { return false };
        if inbound {
            c.import_shared(&ctx.ui, files.into_iter().map(|(name, bytes)| Shared::File { name, bytes }).collect());
        } else {
            c.insert_files(&ctx.ui, files);
        }
        true
    }

    /// Gives the board the export factor of the main display, once the shell has said what it is.
    fn sync_export(&mut self, ctx: &Ctx) {
        if let (Some(c), Some(backing)) = (&mut self.app, self.services.backing()) {
            c.set_export_scale(ink::export_factor(backing, ctx.ui.s));
        }
    }
}

impl App for FreeformApp {
    fn id(&self) -> &str {
        "is.olibuijr.freeform.desktop"
    }

    fn configure(&mut self, ctx: &Ctx) {
        let (w, h) = (ctx.cfg.w as i32, ctx.cfg.h as i32);
        match &mut self.app {
            Some(c) => c.set_size(w, h),
            None => {
                let home = PathBuf::from(std::env::var("HOME").unwrap_or_default());
                let board = home.join(".local/share/is.olibuijr.freeform.desktop");
                self.app = Some(freeform::Freeform::new(&ctx.ui, w, h).with_save_dir(home.join("Pictures")).with_tips(tip_store(&home)).with_store(board))
            }
        }
        self.services.configure(ctx);
        self.sync_export(ctx);
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        let (frame, pointer) = match ev {
            Input::Activate(id) => match &mut self.app {
                Some(c) => (c.activate_id(&ctx.ui, id), None),
                None => return false,
            },
            Input::Key(key) => match &mut self.app {
                Some(c) => (c.key(&ctx.ui, &key), None),
                None => return false,
            },
            Input::Open(url) => {
                let Some(c) = &mut self.app else { return false };
                let piece = match bifrost_app::share::text_from_url(&url) {
                    Some(text) => Shared::Text(text),
                    None if url.starts_with("http://") || url.starts_with("https://") => Shared::Url(url),
                    None => return false,
                };
                c.import_shared(&ctx.ui, vec![piece]);
                (true, None)
            }
            Input::Touch { kind, x, y, t, .. } => {
                let Some(c) = &mut self.app else { return false };
                let e = match kind {
                    TouchKind::Down => Event::Down(x, y),
                    TouchKind::Move => Event::Move(x, y),
                    TouchKind::Up | TouchKind::Cancel => Event::Up(x, y),
                };
                c.touch(&ctx.ui, e, t);
                (true, Some((x, y)))
            }
            _ => return false,
        };
        if let Some((x, y)) = pointer {
            self.show_pointer(ctx, x, y);
        }
        self.drain(ctx);
        frame
    }

    fn controls(&self, _ctx: &Ctx) -> Vec<bifrost_app::controls::Node> {
        self.app.as_ref().map(|c| c.controls()).unwrap_or_default()
    }

    /// The app's menus (the shell bar merges them into its own File and Edit menus). Export is off until the board has
    /// a stroke; the edit and arrange items follow the undo stacks and the selection.
    fn main_menu(&self, _ctx: &Ctx) -> Vec<AppMenu> {
        let app = self.app.as_ref();
        let has = app.is_some_and(|c| c.has_strokes());
        let selected = app.is_some_and(|c| c.has_selection());
        let can_undo = app.is_some_and(|c| c.can_undo());
        let can_redo = app.is_some_and(|c| c.can_redo());
        let boards = app.map(|c| c.board_count()).unwrap_or(1);
        let two = app.is_some_and(|c| c.has_units(2));
        let three = app.is_some_and(|c| c.has_units(3));
        let tool = app.and_then(|c| c.shape_tool());
        let picked = app.is_some_and(|c| c.has_pick());
        let text_tool = app.and_then(|c| c.text_tool());
        let flag = |f: fn(&freeform::Freeform) -> bool| app.is_some_and(f);
        let mut board_items: Vec<AppMenuItem> = vec![
            AppMenuItem::new(MENU_SEARCH, "Search Boards...").key("cmd+shift+f"),
            AppMenuItem::separator(),
            AppMenuItem::new(MENU_NEXT_BOARD, "Next Board").key("cmd+shift+]").enabled(boards > 1),
            AppMenuItem::new(MENU_PREV_BOARD, "Previous Board").key("cmd+shift+[").enabled(boards > 1),
            AppMenuItem::separator(),
        ];
        if let Some(c) = app {
            for (id, title, open) in c.board_entries() {
                board_items.push(AppMenuItem::new(MENU_BOARD + id, &title).checked(open));
            }
        }
        let mut insert_items: Vec<AppMenuItem> =
            SHAPES.iter().enumerate().map(|(i, (kind, title))| AppMenuItem::new(MENU_SHAPE + i as u32, title).checked(tool == Some(*kind))).collect();
        insert_items.extend([
            AppMenuItem::separator(),
            AppMenuItem::new(MENU_TEXT_BOX, "Text Box").key("cmd+alt+t").checked(text_tool == Some(items::Kind::Text)),
            AppMenuItem::new(MENU_NOTE, "Sticky Note").key("cmd+alt+n").checked(text_tool == Some(items::Kind::Sticky)),
            AppMenuItem::new(MENU_TABLE, "Table"),
            AppMenuItem::new(MENU_CONNECTOR, "Connector").checked(flag(|c| c.connector_tool())),
            AppMenuItem::separator(),
            AppMenuItem::new(MENU_LINK, "Link"),
            AppMenuItem::new(MENU_FILE, "Picture or File...").key("cmd+alt+i"),
            AppMenuItem::separator(),
            AppMenuItem::new(MENU_TABLE_ROW, "Add Table Row").enabled(flag(|c| c.can_add_table_part())),
            AppMenuItem::new(MENU_TABLE_COLUMN, "Add Table Column").enabled(flag(|c| c.can_add_table_part())),
        ]);
        vec![
            AppMenu {
                title: "File".into(),
                items: vec![
                    AppMenuItem::new(MENU_NEW_BOARD, "New Board").key("cmd+n"),
                    AppMenuItem::new(MENU_DUPLICATE_BOARD, "Duplicate Board"),
                    AppMenuItem::new(MENU_RENAME_BOARD, "Rename Board..."),
                    AppMenuItem::new(MENU_DELETE_BOARD, "Delete Board").enabled(boards > 1),
                    AppMenuItem::separator(),
                    AppMenuItem::new(MENU_EXPORT, "Export as PNG...").key("cmd+shift+e").enabled(has),
                    AppMenuItem::new(MENU_EXPORT_PDF, "Export as PDF...").key("cmd+alt+e").enabled(has),
                    AppMenuItem::separator(),
                    AppMenuItem::new(MENU_PRINT, "Print...").key("cmd+p").enabled(has),
                ],
            },
            AppMenu {
                title: "Edit".into(),
                items: vec![
                    AppMenuItem::new(MENU_UNDO, "Undo").key("cmd+z").enabled(can_undo),
                    AppMenuItem::new(MENU_REDO, "Redo").key("cmd+shift+z").enabled(can_redo),
                    AppMenuItem::separator(),
                    AppMenuItem::new(MENU_COPY, "Copy").key("cmd+c").enabled(selected),
                    AppMenuItem::new(MENU_PASTE, "Paste").key("cmd+v"),
                    AppMenuItem::new(MENU_DUPLICATE, "Duplicate").key("cmd+d").enabled(selected || picked),
                    AppMenuItem::new(MENU_DELETE, "Delete").enabled(selected || picked),
                ],
            },
            AppMenu { title: "Insert".into(), items: insert_items },
            AppMenu {
                title: "Arrange".into(),
                items: vec![
                    AppMenuItem::new(MENU_FRONT, "Bring to Front").key("cmd+alt+f").enabled(selected),
                    AppMenuItem::new(MENU_BACK, "Send to Back").key("cmd+alt+b").enabled(selected),
                    AppMenuItem::separator(),
                    AppMenuItem::new(MENU_GROUP, "Group").key("cmd+alt+g").enabled(flag(|c| c.can_group())),
                    AppMenuItem::new(MENU_UNGROUP, "Ungroup").key("cmd+alt+shift+g").enabled(flag(|c| c.can_ungroup())),
                    AppMenuItem::new(MENU_LOCK, "Lock").key("cmd+alt+l").enabled(flag(|c| c.can_lock())),
                    AppMenuItem::new(MENU_UNLOCK, "Unlock").key("cmd+alt+shift+l").enabled(flag(|c| c.can_unlock())),
                    AppMenuItem::separator(),
                    AppMenuItem::new(MENU_ALIGN_LEFT, "Align Left").enabled(two),
                    AppMenuItem::new(MENU_ALIGN_CENTER, "Align Centre").enabled(two),
                    AppMenuItem::new(MENU_ALIGN_RIGHT, "Align Right").enabled(two),
                    AppMenuItem::new(MENU_ALIGN_TOP, "Align Top").enabled(two),
                    AppMenuItem::new(MENU_ALIGN_MIDDLE, "Align Middle").enabled(two),
                    AppMenuItem::new(MENU_ALIGN_BOTTOM, "Align Bottom").enabled(two),
                    AppMenuItem::new(MENU_DISTRIBUTE_H, "Distribute Horizontally").enabled(three),
                    AppMenuItem::new(MENU_DISTRIBUTE_V, "Distribute Vertically").enabled(three),
                    AppMenuItem::separator(),
                    AppMenuItem::new(MENU_LARGER, "Make Item Larger").enabled(flag(|c| c.can_scale_item())),
                    AppMenuItem::new(MENU_SMALLER, "Make Item Smaller").enabled(flag(|c| c.can_scale_item())),
                    AppMenuItem::separator(),
                    AppMenuItem::new(MENU_ROW, "Arrange in Row").enabled(selected),
                ],
            },
            AppMenu {
                title: "View".into(),
                items: vec![
                    AppMenuItem::new(MENU_GRID, if flag(|c| c.grid_shown()) { "Hide Grid" } else { "Show Grid" }).key("cmd+'"),
                    AppMenuItem::new(MENU_SNAP, "Snap to Grid").checked(flag(|c| c.snapping())),
                    AppMenuItem::separator(),
                    AppMenuItem::new(MENU_ZOOM_IN, "Zoom In").key("cmd+=").enabled(flag(|c| c.can_zoom_in())),
                    AppMenuItem::new(MENU_ZOOM_OUT, "Zoom Out").key("cmd+-").enabled(flag(|c| c.can_zoom_out())),
                    AppMenuItem::new(MENU_ACTUAL_SIZE, "Actual Size").key("cmd+0").enabled(flag(|c| c.view_changed_from_plain())),
                    AppMenuItem::new(MENU_HAND, "Hand Tool").checked(flag(|c| c.hand_tool())),
                ],
            },
            AppMenu { title: "Boards".into(), items: board_items },
        ]
    }

    fn menu_action(&mut self, ctx: &Ctx, id: u32) -> bool {
        if id == MENU_PASTE {
            if let Some(c) = &mut self.app {
                if c.is_editing() {
                    // typing into an item or the search: the pasteboard's text goes there, not the board's strokes
                    if let Some(text) = ctx.pasteboard_string() {
                        c.paste_text(&ctx.ui, &text);
                    }
                    return true;
                }
            }
            self.services.paste(ctx);
            return true;
        }
        let Some(c) = &mut self.app else { return false };
        match id {
            MENU_EXPORT => c.export_as_png(&ctx.ui),
            MENU_EXPORT_PDF => c.export_as_pdf(&ctx.ui),
            MENU_PRINT => c.print_board(&ctx.ui),
            MENU_NEW_BOARD => c.new_board(&ctx.ui),
            MENU_DUPLICATE_BOARD => c.duplicate_board(&ctx.ui),
            MENU_DELETE_BOARD => c.delete_board(&ctx.ui),
            MENU_NEXT_BOARD => c.step_board(&ctx.ui, true),
            MENU_PREV_BOARD => c.step_board(&ctx.ui, false),
            MENU_UNDO => c.undo_step(),
            MENU_REDO => c.redo_step(),
            MENU_COPY => c.copy_selected(),
            MENU_DUPLICATE => c.duplicate_selection(&ctx.ui),
            MENU_DELETE => c.delete_selected(&ctx.ui),
            MENU_FRONT => c.reorder_selection(&ctx.ui, true),
            MENU_BACK => c.reorder_selection(&ctx.ui, false),
            MENU_ROW => c.arrange_in_row(&ctx.ui),
            MENU_GROUP => c.group_selection(),
            MENU_UNGROUP => c.ungroup_selection(),
            MENU_LOCK => c.lock_selection(),
            MENU_UNLOCK => c.unlock_selection(),
            MENU_ALIGN_LEFT => c.align_selection(&ctx.ui, Align::Left),
            MENU_ALIGN_CENTER => c.align_selection(&ctx.ui, Align::Center),
            MENU_ALIGN_RIGHT => c.align_selection(&ctx.ui, Align::Right),
            MENU_ALIGN_TOP => c.align_selection(&ctx.ui, Align::Top),
            MENU_ALIGN_MIDDLE => c.align_selection(&ctx.ui, Align::Middle),
            MENU_ALIGN_BOTTOM => c.align_selection(&ctx.ui, Align::Bottom),
            MENU_DISTRIBUTE_H => c.distribute_selection(&ctx.ui, true),
            MENU_DISTRIBUTE_V => c.distribute_selection(&ctx.ui, false),
            MENU_SEARCH => c.open_search(&ctx.ui),
            MENU_RENAME_BOARD => c.open_rename(&ctx.ui),
            MENU_TEXT_BOX => c.insert_text_box(&ctx.ui),
            MENU_NOTE => c.insert_note(&ctx.ui),
            MENU_TABLE => c.insert_table(&ctx.ui),
            MENU_CONNECTOR => c.choose_connector(&ctx.ui),
            MENU_LINK => c.insert_link(&ctx.ui, ctx.pasteboard_string().as_deref()),
            MENU_FILE => {
                let options = OpenOptions { title: "Insert".into(), multiple: true, ..OpenOptions::default() };
                self.insert = Some(ctx.open_panel(&options));
            }
            MENU_TABLE_ROW => c.add_table_part(&ctx.ui, true),
            MENU_TABLE_COLUMN => c.add_table_part(&ctx.ui, false),
            MENU_LARGER => c.scale_pick(&ctx.ui, 1.25),
            MENU_SMALLER => c.scale_pick(&ctx.ui, 0.8),
            MENU_ZOOM_IN => c.zoom_in(&ctx.ui),
            MENU_ZOOM_OUT => c.zoom_out(&ctx.ui),
            MENU_ACTUAL_SIZE => c.zoom_actual(&ctx.ui),
            MENU_HAND => c.choose_hand(&ctx.ui),
            MENU_GRID => c.toggle_grid(),
            MENU_SNAP => c.toggle_snap(),
            i if (MENU_SHAPE..MENU_SHAPE + SHAPES.len() as u32).contains(&i) => c.choose_shape(SHAPES[(i - MENU_SHAPE) as usize].0),
            i if i > MENU_BOARD => c.open_board(&ctx.ui, i - MENU_BOARD),
            _ => return false,
        }
        self.drain(ctx);
        true
    }

    fn tick(&mut self, ctx: &Ctx, t: f64) -> bool {
        self.app.as_mut().is_some_and(|c| c.tick(&ctx.ui, t))
    }

    fn next_wake(&self, t: f64) -> Option<f64> {
        self.app.as_ref().and_then(|c| c.next_wake(t))
    }

    fn draw(&mut self, ctx: &Ctx, s: &mut Surface, t: f64) -> Damage {
        if let Some(c) = &mut self.app {
            c.draw(s, &ctx.ui, t);
        }
        let mut d = Damage::new(s.w, s.h);
        d.all();
        d
    }

    fn suspend(&mut self, ctx: &Ctx) {
        if let Some(c) = &mut self.app {
            c.flush();
        }
        self.services.suspend(ctx);
    }

    fn document(&mut self, ctx: &Ctx, ev: DocumentEvent) -> bool {
        match ev {
            DocumentEvent::Opened(doc) => {
                let request = doc.request;
                self.take_files(ctx, request, vec![doc])
            }
            DocumentEvent::OpenedMany { request, docs } => self.take_files(ctx, request, docs),
            DocumentEvent::Failed { request, code } => {
                if self.insert == Some(request) {
                    self.insert = None;
                }
                if code != bifrost_app::proto::DocErrorCode::Cancelled {
                    ctx.log(&format!("freeform: document request {request}: {code:?}"));
                }
                false
            }
            // Freeform never asks to save a document through a panel; refuse one that arrives
            DocumentEvent::SaveTo(doc) => {
                ctx.abort_document(doc.request);
                false
            }
            DocumentEvent::Saved(doc) => {
                ctx.release_document(doc.grant);
                false
            }
        }
    }

    fn service_reply(&mut self, ctx: &Ctx, request: u32, result: &Result<Json, String>) -> bool {
        let took = self.services.reply(ctx, request, result);
        if let Some(c) = &mut self.app {
            if let Some(strokes) = self.services.take_pasted() {
                c.paste_strokes(&ctx.ui, &strokes);
            }
            if let Some(picture) = self.services.take_pasted_picture() {
                c.insert_files(&ctx.ui, vec![("Pasted picture.png".to_string(), picture)]);
            }
            if let Some(text) = self.services.take_pasted_text() {
                c.paste_text_item(&ctx.ui, text);
            }
        }
        self.sync_export(ctx);
        took
    }

    fn service_event(&mut self, ctx: &Ctx, service: &str, event: &str, body: &Json) -> bool {
        self.services.event(ctx, service, event, body);
        self.sync_export(ctx);
        false
    }
}

/// The first-use tip state, in the app's data folder. When that file cannot be read the tips live in memory only.
fn tip_store(home: &std::path::Path) -> bifrost_app::controls::tips::Datastore {
    use bifrost_app::controls::tips::{configure, ConfigOption};
    let path = home.join(".local/share/is.olibuijr.freeform.desktop/tips.txt");
    match configure(&[ConfigOption::DatastorePath(path)]) {
        Ok(store) => store,
        Err(_) => configure(&[]).expect("an in-memory tip store always loads"),
    }
}

fn main() {
    bifrost_app::run(FreeformApp { app: None, services: Default::default(), insert: None })
}
