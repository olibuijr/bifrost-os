//! Mail as an installed BifrOSt desktop app: hosts `window::MailWindow` through the SDK.

use bifrost_app::kit::{Damage, Surface};
use bifrost_app::{App, AppMenu, Command, Commands, Ctx, Input};
use bifrost_mail::window::MailWindow;

struct MailApp {
    window: Option<MailWindow>,
}

impl App for MailApp {
    fn id(&self) -> &str {
        "is.olibuijr.mail.desktop"
    }

    fn configure(&mut self, ctx: &Ctx) {
        let (w, h) = (ctx.cfg.w, ctx.cfg.h);
        match &mut self.window {
            Some(mail) => mail.set_size(w, h),
            None => {
                let mut mail = MailWindow::new(ctx);
                mail.set_size(w, h);
                self.window = Some(mail);
                ctx.wants_keyboard(true);
            }
        }
    }

    fn commands(&self, _ctx: &Ctx) -> Commands {
        Commands(Command::Paste.bit())
    }

    fn main_menu(&self, _ctx: &Ctx) -> Vec<AppMenu> {
        self.window.as_ref().map_or_else(Vec::new, |mail| mail.menu_items())
    }

    fn menu_action(&mut self, ctx: &Ctx, id: u32) -> bool {
        self.window.as_mut().is_some_and(|mail| mail.menu(ctx, id))
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        let Some(mail) = &mut self.window else { return false };
        match ev {
            Input::Touch { kind, x, y, t, .. } => mail.touch(ctx, kind, x, y, t),
            Input::Key(k) => mail.key(ctx, k),
            Input::Command(Command::Paste) => mail.paste(),
            _ => false,
        }
    }

    fn service_reply(&mut self, ctx: &Ctx, request: u32, result: &Result<bifrost_core::json::Json, String>) -> bool {
        self.window.as_mut().is_some_and(|mail| mail.service_reply(ctx, request, result))
    }

    fn tick(&mut self, ctx: &Ctx, t: f64) -> bool {
        self.window.as_mut().is_some_and(|mail| mail.tick(ctx, t))
    }

    fn draw(&mut self, ctx: &Ctx, s: &mut Surface, _t: f64) -> Damage {
        if let Some(mail) = &mut self.window {
            mail.draw(s, &ctx.ui);
        }
        let mut d = Damage::new(s.w, s.h);
        d.all();
        d
    }

    fn next_wake(&self, t: f64) -> Option<f64> {
        self.window.as_ref().and_then(|mail| mail.next_wake(t))
    }

    fn wake_fd(&self) -> Option<i32> {
        self.window.as_ref().and_then(|mail| mail.wake_fd())
    }
}

fn main() {
    bifrost_app::run(MailApp { window: None });
}
