//! Console as an installed BifrOSt system app: the system journal, the crash reports and the files in `/var/log`.
//! The window is `console.rs` and `draw.rs`; the reading is `source.rs`; the entries, the parsers and the search
//! are `model.rs`.

mod console;
mod cursor;
mod draw;
mod identity;
mod model;
mod security;
mod services;
mod signposts;
mod source;
#[cfg(test)]
mod wire_tests;

use bifrost_app::controls::Node;
use bifrost_app::kit::{Damage, Surface};
use bifrost_app::{App, AppMenu, AppMenuItem, Ctx, Input, TouchKind};
use bifrost_app::power_service;
use bifrost_core::json::Json;
use console::{Console, View, MENU_AFTER, MENU_CLEAR, MENU_COPY, MENU_DEBUG, MENU_FIND, MENU_INFO, MENU_STREAM, MENU_SUBSYSTEM};
use services::PowerNote;

struct ConsoleApp {
    console: Option<Console>,
    /// The power service's state, shown in the footer.
    power: PowerNote,
    /// The request for the power snapshot, asked once when the window opens.
    power_request: Option<u32>,
    /// Copy Message writes in flight, each with the text to put on the plain clipboard if the shell has no pasteboard service.
    copy_pending: Vec<(u32, String)>,
}

impl ConsoleApp {
    /// Pushes the power line into the window's footer.
    fn sync_power(&mut self) {
        let line = self.power.line();
        if let Some(c) = &mut self.console {
            c.power_note = line;
        }
    }

    /// Log menu: Copy Message. The line goes to the general board; an older shell answers with an error, and then the
    /// text goes to the plain clipboard (see `service_reply`).
    fn copy_selected(&mut self, ctx: &Ctx) {
        let Some(text) = self.console.as_ref().and_then(|c| c.selected_entry()).map(services::copy_line) else { return };
        let id = services::copy_to_general(ctx, &text);
        self.copy_pending.push((id, text));
    }
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
        // one snapshot when the window opens (a real event, not a frame); later changes are pushed
        if self.power_request.is_none() {
            self.power_request = Some(power_service::snapshot(ctx));
        }
    }

    fn service_reply(&mut self, ctx: &Ctx, request: u32, result: &Result<Json, String>) -> bool {
        if self.power_request == Some(request) {
            // an older shell answers an error for "power"; the footer then shows nothing
            if let Some(snapshot) = power_service::decode_snapshot(result) {
                self.power.apply_snapshot(snapshot);
                self.sync_power();
                return true;
            }
            return false;
        }
        if let Some(at) = self.copy_pending.iter().position(|(id, _)| *id == request) {
            let (_, text) = self.copy_pending.remove(at);
            if result.is_err() {
                ctx.set_pasteboard_string(&text);
            }
        }
        false
    }

    fn service_event(&mut self, _ctx: &Ctx, service: &str, event: &str, body: &Json) -> bool {
        let Some(change) = power_service::decode_change(service, event, body) else { return false };
        self.power.apply_change(change);
        self.sync_power();
        true
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
                AppMenuItem::separator(),
                AppMenuItem::new(MENU_COPY, "Copy Message").enabled(c.selected_entry().is_some()),
            ],
        }]
    }

    fn menu_action(&mut self, ctx: &Ctx, id: u32) -> bool {
        if id == MENU_COPY {
            self.copy_selected(ctx);
            return false;
        }
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
    bifrost_app::run(ConsoleApp { console: None, power: PowerNote::default(), power_request: None, copy_pending: Vec::new() });
}

#[cfg(test)]
mod glue_tests {
    use super::*;
    use crate::model::{Level, LogEntry};
    use bifrost_app::proto::ToShell;
    use bifrost_core::json::parse;

    fn app() -> ConsoleApp {
        ConsoleApp { console: None, power: PowerNote::default(), power_request: None, copy_pending: Vec::new() }
    }

    fn reply(text: &str) -> Result<Json, String> {
        Ok(parse(text).unwrap())
    }

    #[test]
    fn a_snapshot_reply_fills_the_power_note() {
        let ctx = services::test_ctx();
        let mut a = app();
        a.power_request = Some(7);
        let body = reply(r#"{"thermalState":"nominal","lowPowerMode":true,"battery":{"present":false}}"#);
        assert!(a.service_reply(&ctx, 7, &body));
        assert_eq!(a.power.line(), "Thermal Nominal · Low Power Mode");
    }

    #[test]
    fn an_older_shell_error_for_power_leaves_the_footer_empty() {
        let ctx = services::test_ctx();
        let mut a = app();
        a.power_request = Some(7);
        assert!(!a.service_reply(&ctx, 7, &Err("no service power".to_string())));
        assert_eq!(a.power.line(), "");
    }

    #[test]
    fn a_pushed_power_change_updates_the_note_and_other_services_are_ignored() {
        let ctx = services::test_ctx();
        let mut a = app();
        let body = parse(r#"{"thermalState":"serious"}"#).unwrap();
        assert!(!a.service_event(&ctx, "pasteboard", "thermalStateDidChange", &body));
        assert_eq!(a.power.line(), "");
        assert!(a.service_event(&ctx, "power", "thermalStateDidChange", &body));
        assert_eq!(a.power.line(), "Thermal Serious");
    }

    #[test]
    fn copy_message_writes_the_selected_line_to_the_general_board() {
        let ctx = services::test_ctx();
        let mut a = app();
        let mut c = Console::new(1200.0, 780.0, Vec::new());
        let seq = c.buffer.push(LogEntry { process: "notes".into(), pid: 9, level: Level::Error, message: "boom".into(), ..LogEntry::default() });
        c.rebuild();
        c.selected = Some(seq);
        a.console = Some(c);
        a.menu_action(&ctx, MENU_COPY);
        let sent = ctx.take_sent();
        let [ToShell::ServiceCall { request, service, method, body }] = sent.as_slice() else { panic!("one pasteboard write") };
        assert_eq!((service.as_str(), method.as_str()), ("pasteboard", "write"));
        assert!(body.contains("notes[9] Error boom"), "the copied line: {body}");
        assert_eq!(a.copy_pending.len(), 1);
        // the shell has no pasteboard service: the reply is an error, the request leaves the queue
        assert!(!a.service_reply(&ctx, *request, &Err("no service pasteboard".to_string())));
        assert!(a.copy_pending.is_empty());
    }

    #[test]
    fn copy_without_a_selected_message_sends_nothing() {
        let ctx = services::test_ctx();
        let mut a = app();
        a.console = Some(Console::new(1200.0, 780.0, Vec::new()));
        a.menu_action(&ctx, MENU_COPY);
        assert!(ctx.take_sent().is_empty());
        assert!(a.copy_pending.is_empty());
    }
}
