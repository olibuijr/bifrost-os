//! Reminders as an installed BifrOSt app: a full-size Golden Gate window (sidebar of smart lists and My Lists,
//! unified toolbar), hosted through the SDK. Text input comes from the hardware keyboard.

mod alerts;
mod app;
mod icon;
mod model;
mod quick;
mod repeat;
mod store;

use bifrost_app::kit::{Damage, Event, Surface};
use bifrost_app::control::{RpcError, INVALID_PARAMS};
use bifrost_app::intents::{self, Intent, IntentResult, Param, ParamType};
use bifrost_app::{App, AppMenu, AppMenuItem, Ctx, Input, KeyInput, TouchKind};
use bifrost_core::json::Json;
use std::path::Path;
use springboard::shell::clock::Wall;
use springboard::shell::keyboard::Out;

/// `AppMenuItem` ids for `menu_action` (0 is reserved for separators).
const MENU_NEW: u32 = 1;
const MENU_NEW_LIST: u32 = 2;
const MENU_DELETE: u32 = 3;
const MENU_FIND: u32 = 4;
const MENU_COMPLETE: u32 = 5;

struct RemindersApp {
    app: Option<app::Reminders>,
    kb: bool,
    alerts: alerts::AlertState,
    wall: Wall,
    /// The count last sent as the dock badge.
    badge: Option<usize>,
    /// Whether the app is currently opted into sudden termination (it starts disabled, as the count does).
    terminable: bool,
}

impl RemindersApp {
    fn sync_keyboard(&mut self, ctx: &Ctx) {
        let on = self.app.as_ref().is_some_and(|a| a.keyboard_shown());
        if on != self.kb {
            self.kb = on;
            ctx.wants_keyboard(on);
        }
        self.sync_termination(ctx);
    }

    /// Every change is saved as it is made, so the app may be ended at once (sudden termination) except while an
    /// editor holds a draft that is not saved yet.
    fn sync_termination(&mut self, ctx: &Ctx) {
        let terminable = self.app.as_ref().is_some_and(|a| !a.has_draft());
        if terminable != self.terminable {
            self.terminable = terminable;
            if terminable {
                ctx.enable_sudden_termination();
            } else {
                ctx.disable_sudden_termination();
            }
        }
    }
}

impl App for RemindersApp {
    fn id(&self) -> &str {
        "is.olibuijr.reminders.desktop"
    }

    fn configure(&mut self, ctx: &Ctx) {
        let (w, h) = (ctx.cfg.w as i32, ctx.cfg.h as i32);
        match &mut self.app {
            Some(a) => {
                a.set_size(w, h);
                a.set_top_inset(bifrost_app::layout::Layout::new(&ctx.cfg).top);
            }
            None => {
                self.wall = Wall::sample(bifrost_proto::sys::now_ms());
                let mut r = app::Reminders::new(&ctx.ui, w, h, ctx.data_dir.join("reminders.tsv"), self.wall);
                r.set_top_inset(bifrost_app::layout::Layout::new(&ctx.cfg).top);
                r.enable_file_export();
                self.app = Some(r);
            }
        }
    }

    fn main_menu(&self, _ctx: &Ctx) -> Vec<AppMenu> {
        let Some(a) = &self.app else { return Vec::new() };
        let on_items = a.on_items();
        let selected = on_items && a.selected_item().is_some();
        // One app menu between App and the shell's File, Edit, View, Window and Help (the shell always supplies those,
        // so the app does not repeat them): the Reminders commands only.
        vec![AppMenu {
            title: "Reminder".into(),
            items: vec![
                AppMenuItem::new(MENU_NEW, "New Reminder").key("cmd+n").enabled(on_items),
                AppMenuItem::new(MENU_NEW_LIST, "New List").key("cmd+shift+n").enabled(on_items),
                AppMenuItem::separator(),
                AppMenuItem::new(MENU_DELETE, "Delete Reminder").key("cmd+backspace").enabled(selected),
                AppMenuItem::new(MENU_COMPLETE, "Mark as Completed").enabled(selected),
                AppMenuItem::separator(),
                AppMenuItem::new(MENU_FIND, "Find").key("cmd+f").enabled(on_items),
            ],
        }]
    }

    fn menu_action(&mut self, ctx: &Ctx, id: u32) -> bool {
        let t = bifrost_proto::sys::now_ms();
        let Some(a) = &mut self.app else { return false };
        let handled = match id {
            MENU_NEW => {
                a.new_reminder(&ctx.ui, t);
                true
            }
            MENU_NEW_LIST => {
                a.new_list(&ctx.ui, t);
                true
            }
            MENU_DELETE => a.delete_selected(&ctx.ui, t),
            MENU_FIND => {
                a.find(&ctx.ui, t);
                true
            }
            MENU_COMPLETE => a.toggle_selected(&ctx.ui),
            _ => false,
        };
        self.sync_keyboard(ctx);
        handled
    }

    fn intents(&self) -> Vec<Intent> {
        vec![Intent::new("add-reminder", "Add Reminder", "Adds a reminder to the default list, with an optional due date.")
            .param(Param::new("title", "Title", ParamType::String))
            .param(Param::new("due", "Due", ParamType::Date).optional())]
    }

    fn perform_intent(&mut self, ctx: Option<&Ctx>, _data_dir: &Path, id: &str, params: &Json) -> Result<IntentResult, RpcError> {
        if id != "add-reminder" {
            return Err(RpcError::new(INVALID_PARAMS, format!("no intent {id}")));
        }
        let (Some(ctx), Some(a)) = (ctx, self.app.as_mut()) else { return Err(intents::failed("Reminders is not running yet")) };
        let title = intents::str_param(params, "title")?;
        let due = params.get("due").and_then(Json::as_str);
        let id = a.add_reminder(&ctx.ui, title, due).map_err(|e| intents::failed(e))?;
        Ok(IntentResult::value(Json::Num(id as f64)).with_dialog(&format!("Added \"{title}\"")))
    }

    fn controls(&self, ctx: &Ctx) -> Vec<bifrost_app::controls::Node> {
        self.app.as_ref().map_or_else(Vec::new, |a| a.controls(&ctx.ui))
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        let Some(a) = &mut self.app else { return false };
        let handled = match ev {
            // a published toolbar control (accessibility, `bifrost pc midget tap-window`): a click at its centre
            Input::Activate(id) => {
                let Some(node) = a.controls(&ctx.ui).into_iter().find(|n| n.id == id && n.enabled) else { return false };
                let (x, y) = (ctx.ui.pt(node.bounds.x + node.bounds.w / 2.0), ctx.ui.pt(node.bounds.y + node.bounds.h / 2.0));
                let t = bifrost_proto::sys::now_ms();
                a.touch(&ctx.ui, Event::Down(x, y), t);
                a.touch(&ctx.ui, Event::Up(x, y), t);
                true
            }
            Input::Touch { kind, x, y, t, .. } => {
                let e = match kind {
                    TouchKind::Down => Event::Down(x, y),
                    TouchKind::Move => Event::Move(x, y),
                    TouchKind::Up | TouchKind::Cancel => Event::Up(x, y),
                };
                a.touch(&ctx.ui, e, t);
                true
            }
            Input::Key(k) => {
                if let Some((key, mods)) = k.as_chord() {
                    a.chord(&ctx.ui, key, mods, bifrost_proto::sys::now_ms());
                    self.sync_keyboard(ctx);
                    return true;
                }
                if k == KeyInput::Code(1) {
                    let used = a.escape(&ctx.ui, bifrost_proto::sys::now_ms());
                    self.sync_keyboard(ctx);
                    return used;
                }
                let out = match k {
                    KeyInput::Text(s) => Some(Out::Text(s)),
                    KeyInput::Code(8) => Some(Out::Backspace),
                    KeyInput::Code(13) => Some(Out::Enter),
                    KeyInput::Code(37) => Some(Out::Named("Left")),
                    KeyInput::Code(38) => Some(Out::Named("Up")),
                    KeyInput::Code(39) => Some(Out::Named("Right")),
                    KeyInput::Code(40) => Some(Out::Named("Down")),
                    KeyInput::Code(_) => None,
                };
                if let Some(out) = out {
                    a.key(&ctx.ui, out, bifrost_proto::sys::now_ms());
                }
                true
            }
            Input::Open(_) => false,
            _ => false,
        };
        self.sync_keyboard(ctx);
        handled
    }

    fn tick(&mut self, ctx: &Ctx, t: f64) -> bool {
        let Some(a) = &mut self.app else { return false };
        self.wall = Wall::sample(t);
        a.set_wall(self.wall);
        let redraw = a.tick(&ctx.ui, t);
        let due = a.due_count();
        if self.badge != Some(due) {
            self.badge = Some(due);
            ctx.set_badge(&model::badge_label(due));
        }
        let path = ctx.data_dir.join("reminders.tsv");
        // Alerts are shell triggers: only the plan changes are sent (see alerts.rs).
        let change = alerts::step(&mut self.alerts, self.wall.now(t), a.generation(), || store::load(&path).items, &|t| springboard::bare::timezone::offset_at(t).unwrap_or(self.wall.offset));
        if let Some(change) = change {
            alerts::apply(ctx, &change);
        }
        self.sync_keyboard(ctx);
        redraw
    }

    fn next_wake(&self, t: f64) -> Option<f64> {
        let a = self.app.as_ref()?;
        let alert = self.alerts.wake(self.wall.unix, self.wall.at);
        match (a.next_wake(t), alert) {
            (Some(x), Some(y)) => Some(x.min(y)),
            (x, y) => x.or(y),
        }
    }

    fn draw(&mut self, ctx: &Ctx, s: &mut Surface, t: f64) -> Damage {
        if let Some(a) = &mut self.app {
            a.draw(s, &ctx.ui, t);
        }
        let mut d = Damage::new(s.w, s.h);
        d.all();
        d
    }
}

fn main() {
    bifrost_app::run(RemindersApp { app: None, kb: false, alerts: alerts::AlertState::default(), wall: Wall::default(), badge: None, terminable: false });
}
