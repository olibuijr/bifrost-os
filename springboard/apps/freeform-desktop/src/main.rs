//! Freeform as an installed BifrOSt app: the former in-shell Freeform,
//! unchanged, hosted through the SDK. It also talks to the shell's services: the cursor (the
//! pointer shape of each tool), the pasteboard (Copy) and the screens (the export scale).

mod freeform;
mod ink;
mod services;
mod style;

use bifrost_app::kit::{Damage, Event, Surface};
use bifrost_app::{App, AppMenu, AppMenuItem, Ctx, Input, TouchKind};
use bifrost_core::json::Json;
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

struct FreeformApp {
    app: Option<freeform::Freeform>,
    services: services::Services,
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
        vec![
            AppMenu { title: "File".into(), items: vec![AppMenuItem::new(MENU_EXPORT, "Export as PNG...").key("cmd+shift+e").enabled(has)] },
            AppMenu {
                title: "Edit".into(),
                items: vec![
                    AppMenuItem::new(MENU_UNDO, "Undo").key("cmd+z").enabled(can_undo),
                    AppMenuItem::new(MENU_REDO, "Redo").key("cmd+shift+z").enabled(can_redo),
                    AppMenuItem::separator(),
                    AppMenuItem::new(MENU_COPY, "Copy").key("cmd+c").enabled(selected),
                    AppMenuItem::new(MENU_PASTE, "Paste").key("cmd+v"),
                    AppMenuItem::new(MENU_DUPLICATE, "Duplicate").key("cmd+d").enabled(selected),
                    AppMenuItem::new(MENU_DELETE, "Delete").enabled(selected),
                ],
            },
            AppMenu {
                title: "Arrange".into(),
                items: vec![
                    AppMenuItem::new(MENU_FRONT, "Bring to Front").key("cmd+alt+f").enabled(selected),
                    AppMenuItem::new(MENU_BACK, "Send to Back").key("cmd+alt+b").enabled(selected),
                    AppMenuItem::separator(),
                    AppMenuItem::new(MENU_ROW, "Arrange in Row").enabled(selected),
                ],
            },
        ]
    }

    fn menu_action(&mut self, ctx: &Ctx, id: u32) -> bool {
        if id == MENU_PASTE {
            self.services.paste(ctx);
            return true;
        }
        let Some(c) = &mut self.app else { return false };
        match id {
            MENU_EXPORT => c.export_as_png(&ctx.ui),
            MENU_UNDO => c.undo_step(),
            MENU_REDO => c.redo_step(),
            MENU_COPY => c.copy_selected(),
            MENU_DUPLICATE => c.duplicate_selection(&ctx.ui),
            MENU_DELETE => c.delete_selected(&ctx.ui),
            MENU_FRONT => c.reorder_selection(&ctx.ui, true),
            MENU_BACK => c.reorder_selection(&ctx.ui, false),
            MENU_ROW => c.arrange_in_row(&ctx.ui),
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

    fn service_reply(&mut self, ctx: &Ctx, request: u32, result: &Result<Json, String>) -> bool {
        let took = self.services.reply(ctx, request, result);
        if let (Some(c), Some(strokes)) = (&mut self.app, self.services.take_pasted()) {
            c.paste_strokes(&ctx.ui, &strokes);
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
    bifrost_app::run(FreeformApp { app: None, services: Default::default() })
}
