//! Messages as an installed BifrOSt desktop app: hosts `window::MessagesWindow` through the SDK.

use bifrost_app::kit::{Damage, Surface};
use bifrost_app::{App, AppMenu, Command, Commands, Ctx, DocumentEvent, Input};
use bifrost_core::json::Json;
use bifrost_messages::window::MessagesWindow;

struct MessagesApp {
    window: Option<MessagesWindow>,
}

impl App for MessagesApp {
    fn id(&self) -> &str {
        "is.olibuijr.messages.desktop"
    }

    fn configure(&mut self, ctx: &Ctx) {
        let (w, h) = (ctx.cfg.w, ctx.cfg.h);
        match &mut self.window {
            Some(messages) => messages.set_size(w, h),
            None => {
                let mut messages = MessagesWindow::new(ctx);
                messages.set_size(w, h);
                self.window = Some(messages);
                ctx.wants_keyboard(true);
            }
        }
    }

    fn commands(&self, _ctx: &Ctx) -> Commands {
        Commands(Command::Paste.bit())
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        let Some(messages) = &mut self.window else { return false };
        match ev {
            Input::Touch { kind, x, y, t, .. } => messages.touch(ctx, kind, x, y, t),
            Input::Key(k) => messages.key(ctx, k),
            Input::Command(Command::Paste) => messages.paste(ctx),
            Input::Context { x, y } => messages.context(ctx, x, y),
            Input::ContextAction(id) => messages.context_action(ctx, id),
            _ => false,
        }
    }

    fn main_menu(&self, _ctx: &Ctx) -> Vec<AppMenu> {
        self.window.as_ref().map(|messages| messages.main_menu()).unwrap_or_default()
    }

    fn menu_action(&mut self, ctx: &Ctx, id: u32) -> bool {
        self.window.as_mut().is_some_and(|messages| messages.menu_action(ctx, id))
    }

    fn service_reply(&mut self, ctx: &Ctx, request: u32, result: &Result<Json, String>) -> bool {
        self.window.as_mut().is_some_and(|messages| messages.service_reply(ctx, request, result))
    }

    fn document(&mut self, ctx: &Ctx, ev: DocumentEvent) -> bool {
        self.window.as_mut().is_some_and(|messages| messages.document(ctx, ev))
    }

    fn drag(&mut self, ctx: &Ctx, ev: bifrost_app::dnd::Event) -> bool {
        self.window.as_mut().is_some_and(|messages| messages.drag(ctx, ev))
    }

    fn tick(&mut self, ctx: &Ctx, t: f64) -> bool {
        self.window.as_mut().is_some_and(|messages| messages.tick(ctx, t))
    }

    fn draw(&mut self, ctx: &Ctx, s: &mut Surface, _t: f64) -> Damage {
        if let Some(messages) = &mut self.window {
            messages.draw(s, &ctx.ui);
        }
        let mut d = Damage::new(s.w, s.h);
        d.all();
        d
    }

    fn next_wake(&self, t: f64) -> Option<f64> {
        self.window.as_ref().and_then(|messages| messages.next_wake(t))
    }

    fn wake_fd(&self) -> Option<i32> {
        self.window.as_ref().and_then(|messages| messages.wake_fd())
    }
}

fn main() {
    bifrost_app::run(MessagesApp { window: None });
}
