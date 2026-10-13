//! Print Center as an installed BifrOSt app: the printers the scheduler knows, the queue of the selected printer, and the
//! actions that pause and resume a printer, set the default, and hold, release or cancel a job. `model.rs` holds the
//! state and the rules, `worker.rs` talks to the scheduler on its own thread, `window.rs` lays the window out and
//! `draw.rs` paints it.

mod draw;
mod model;
mod rpc;
mod window;
mod worker;

use bifrost_app::controls::Node;
use bifrost_app::kit::{Damage, Surface};
use bifrost_app::control::RpcError;
use bifrost_app::{App, AppMenu, Ctx, DocumentEvent, Input, KeyInput, OpenOptions};
use bifrost_core::json::Json;
use window::{PrintCenter, Target, B_PRINT_FILE};

impl App for PrintCenter {
    fn id(&self) -> &str {
        "is.olibuijr.printers.desktop"
    }

    fn configure(&mut self, ctx: &Ctx) {
        ctx.enable_sudden_termination(); // keeps no unsaved state: the printers are read again from the scheduler
        self.resize(ctx.cfg.w as f32, ctx.cfg.h as f32);
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        match ev {
            Input::Touch { kind, x, y, .. } => {
                let scale = ctx.ui.pt(1.0);
                self.touch(&ctx.ui, kind, x / scale, y / scale)
            }
            // a published control (accessibility, `bifrost pc midget tap-window`): the same action as a click
            Input::Activate(id) => self.activate(Target::Toolbar(id as u32)),
            // a secondary click: choose what is under it, then offer the menu for it
            Input::Context { x, y } => {
                let scale = ctx.ui.pt(1.0);
                let target = self.hit_body(x / scale, y / scale);
                let items = self.context_items(target);
                if items.is_empty() {
                    return false;
                }
                ctx.context_menu(x, y, items);
                true
            }
            Input::ContextAction(id) => {
                self.toolbar_action(id);
                true
            }
            Input::Key(KeyInput::Text(text)) => self.text(&text),
            Input::Key(KeyInput::Code(code)) => self.key(code),
            Input::Command(command) => self.command(command),
            // a file dropped on the window or the Dock icon, or opened with the app: print it
            Input::Open(path) => {
                self.print_path(path.strip_prefix("file://").unwrap_or(&path));
                true
            }
        }
    }

    fn main_menu(&self, _ctx: &Ctx) -> Vec<AppMenu> {
        self.menus()
    }

    fn menu_action(&mut self, ctx: &Ctx, id: u32) -> bool {
        if id == B_PRINT_FILE {
            let options = OpenOptions { multiple: true, title: "Print Document".into(), ..OpenOptions::default() };
            self.open_request = Some(ctx.open_panel(&options));
            return true;
        }
        self.toolbar_action(id);
        true
    }

    /// The documents the open panel returned: each one goes to the selected printer.
    fn document(&mut self, _ctx: &Ctx, ev: DocumentEvent) -> bool {
        match ev {
            DocumentEvent::Opened(mut doc) => self.print_reader(&doc.name, &mut doc.file),
            DocumentEvent::OpenedMany { docs, .. } => {
                for mut doc in docs {
                    self.print_reader(&doc.name, &mut doc.file);
                }
            }
            DocumentEvent::Failed { .. } => {}
            _ => return false,
        }
        self.open_request = None;
        true
    }

    /// The refresh loop: takes the replies the worker sent and lists the printers on a timer. A change in the number of
    /// waiting jobs moves the Dock badge, and a printer that needs attention is announced once.
    fn tick(&mut self, ctx: &Ctx, t: f64) -> bool {
        let changed = self.step(t);
        if let Some(label) = self.badge_change() {
            ctx.set_badge(&label);
        }
        for (title, body) in self.take_alerts() {
            ctx.notify(&title, &body);
        }
        changed
    }

    fn rpc_methods(&self) -> Vec<&'static str> {
        rpc::methods()
    }

    fn rpc(&mut self, _ctx: Option<&Ctx>, _data_dir: &std::path::Path, method: &str, params: &Json) -> Result<Json, RpcError> {
        rpc::call(self, method, params)
    }

    fn next_wake(&self, t: f64) -> Option<f64> {
        Some(self.wake_at(t))
    }

    fn controls(&self, ctx: &Ctx) -> Vec<Node> {
        draw::controls(self, &ctx.ui)
    }

    fn draw(&mut self, ctx: &Ctx, s: &mut Surface, _t: f64) -> Damage {
        draw::paint(self, s, &ctx.ui);
        let mut d = Damage::new(s.w, s.h);
        d.all();
        d
    }
}

fn main() {
    bifrost_app::run(PrintCenter::new());
}
