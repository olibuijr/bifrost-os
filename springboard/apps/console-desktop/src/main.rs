//! Console as an installed BifrOSt system app: the system journal, the crash reports and the files in `/var/log`.
//! The window is `console.rs` and `draw.rs`; the reading is `source.rs`; the entries, the parsers and the search
//! are `model.rs`.

mod console;
mod cursor;
mod draw;
mod model;
mod signposts;
mod source;
#[cfg(test)]
mod wire_tests;

use bifrost_app::controls::Node;
use bifrost_app::kit::{Damage, Surface};
use bifrost_app::{App, AppMenu, AppMenuItem, Ctx, Input, TouchKind};
use console::{Console, View, MENU_AFTER, MENU_CLEAR, MENU_DEBUG, MENU_FIND, MENU_INFO, MENU_STREAM, MENU_SUBSYSTEM};

struct ConsoleApp {
    console: Option<Console>,
}

impl App for ConsoleApp {
    fn id(&self) -> &str {
        "is.olibuijr.console.desktop"
    }

    fn configure(&mut self, ctx: &Ctx) {
        ctx.enable_sudden_termination(); // keeps no unsaved state
        let (w, h) = (ctx.cfg.w as f32, ctx.cfg.h as f32);
        match &mut self.console {
            Some(c) => c.resize(w, h),
            None => {
                let mut c = Console::new(w, h, source::log_files());
                c.open_view(View::Live);
                self.console = Some(c);
            }
        }
    }

    fn controls(&self, ctx: &Ctx) -> Vec<Node> {
        self.console.as_ref().map_or_else(Vec::new, |c| c.controls(&ctx.ui))
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        let Some(c) = &mut self.console else { return false };
        let changed = match ev {
            Input::Touch { kind, x, y, t, .. } => c.touch(&ctx.ui, kind, x, y, t),
            Input::Key(key) => c.key_input(key),
            // a published toolbar control (accessibility, `bifrost pc midget tap-window`): a click at its centre
            Input::Activate(id) => {
                let Some(node) = c.controls(&ctx.ui).into_iter().find(|n| n.id == id && n.enabled) else { return false };
                let x = ctx.ui.pt(node.bounds.x + node.bounds.w / 2.0);
                let y = ctx.ui.pt(node.bounds.y + node.bounds.h / 2.0);
                let t = bifrost_app::proto::sys::now_ms();
                c.touch(&ctx.ui, TouchKind::Down, x, y, t);
                c.touch(&ctx.ui, TouchKind::Up, x, y, t)
            }
            _ => false,
        };
        ctx.wants_keyboard(c.search_focused);
        changed
    }

    fn tick(&mut self, _ctx: &Ctx, t: f64) -> bool {
        let Some(c) = &mut self.console else { return false };
        let moving = c.animating(t);
        let changed = c.drain();
        moving || changed
    }

    fn main_menu(&self, _ctx: &Ctx) -> Vec<AppMenu> {
        let Some(c) = &self.console else { return Vec::new() };
        let stream = if c.streaming && c.is_live() { "Pause Streaming" } else { "Start Streaming" };
        // the shell bar owns File, Edit, View, Window and Help: the app's own menu is titled for its domain
        vec![AppMenu {
            title: "Log".into(),
            items: vec![
                AppMenuItem::new(MENU_STREAM, stream).enabled(c.is_live()),
                AppMenuItem::separator(),
                AppMenuItem::new(MENU_INFO, "Include Info Messages").checked(c.levels.info),
                AppMenuItem::new(MENU_DEBUG, "Include Debug Messages").checked(c.levels.debug),
                AppMenuItem::separator(),
                AppMenuItem::new(MENU_FIND, "Find Message").key("cmd+f"),
                AppMenuItem::new(MENU_CLEAR, "Clear Display").key("cmd+k"),
                AppMenuItem::separator(),
                AppMenuItem::new(MENU_AFTER, if c.after.is_some() { "Show All Messages" } else { "Show Messages After Selected" })
                    .enabled(c.after.is_some() || c.selected.is_some()),
                AppMenuItem::new(MENU_SUBSYSTEM, "Show Only This Subsystem").enabled(c.selected_entry().is_some_and(|e| !e.subsystem.is_empty())),
            ],
        }]
    }

    fn menu_action(&mut self, ctx: &Ctx, id: u32) -> bool {
        let Some(c) = &mut self.console else { return false };
        let changed = c.menu(id);
        ctx.wants_keyboard(c.search_focused);
        changed
    }

    fn draw(&mut self, ctx: &Ctx, s: &mut Surface, _t: f64) -> Damage {
        if let Some(c) = &mut self.console {
            c.draw(&ctx.ui, s);
        }
        let mut d = Damage::new(s.w, s.h);
        d.all();
        d
    }
}

fn main() {
    bifrost_app::run(ConsoleApp { console: None });
}
