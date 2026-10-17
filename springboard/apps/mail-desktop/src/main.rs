//! Mail as an installed BifrOSt desktop app: hosts `window::MailWindow` through the SDK.

use bifrost_app::app_windows::WindowId;
use bifrost_app::kit::{Damage, Surface};
use bifrost_app::{App, AppMenu, Command, Commands, Ctx, DocumentEvent, Input};
use bifrost_mail::viewer::Viewer;
use bifrost_mail::window::MailWindow;

struct MailApp {
    window: Option<MailWindow>,
    /// Viewer windows: a message in a window of its own (double-click, File > New Viewer Window).
    viewers: Vec<Viewer>,
    /// A `mailto:` link that arrived before the window existed; it opens once the window is made.
    pending_url: Option<String>,
}

impl MailApp {
    /// Opens a viewer window for each message the main window asked for. True when a window was opened.
    fn open_viewers(&mut self, ctx: &Ctx) -> bool {
        let Some(mail) = &mut self.window else { return false };
        let snapshots = mail.take_viewers();
        let mut opened = false;
        for snapshot in snapshots {
            match ctx.open_window(snapshot.spec()) {
                Ok(id) => {
                    self.viewers.push(Viewer::new(id, snapshot));
                    opened = true;
                }
                Err(e) => mail.mail.status = e.to_string(),
            }
        }
        opened
    }

    /// Copies a body into the viewers that were opened before it was fetched. True when one changed.
    fn refresh_viewers(&mut self) -> bool {
        let Some(mail) = &self.window else { return false };
        let mut changed = false;
        for viewer in self.viewers.iter_mut().filter(|v| !v.snapshot.loaded) {
            let snapshot = mail.viewer_snapshot(viewer.snapshot.uid);
            if snapshot.loaded {
                viewer.snapshot = snapshot;
                changed = true;
            }
        }
        changed
    }
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
                if let Some(url) = self.pending_url.take() {
                    mail.open_handoff(&url);
                }
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
        let changed = match &mut self.window {
            None => {
                // a mailto: link before the window exists waits for configure
                if let Input::Open(url) = ev {
                    self.pending_url = Some(url);
                }
                false
            }
            Some(mail) => match ev {
                Input::Touch { kind, x, y, t, .. } => mail.touch(ctx, kind, x, y, t),
                Input::Key(k) => mail.key(ctx, k),
                Input::Command(Command::Paste) => mail.paste(),
                Input::Open(url) => {
                    mail.open_handoff(&url);
                    true
                }
                _ => false,
            },
        };
        self.open_viewers(ctx) | changed
    }

    fn window_event(&mut self, ctx: &Ctx, window: WindowId, ev: Input) -> bool {
        if window.is_main() {
            return self.event(ctx, ev);
        }
        let Some(viewer) = self.viewers.iter_mut().find(|v| v.id == window) else { return false };
        match ev {
            Input::Touch { kind, y, t, .. } => viewer.touch(kind, y, t),
            _ => false,
        }
    }

    fn draw_window(&mut self, ctx: &Ctx, window: WindowId, s: &mut Surface, t: f64) -> Damage {
        if window.is_main() {
            return self.draw(ctx, s, t);
        }
        if let Some(viewer) = self.viewers.iter_mut().find(|v| v.id == window) {
            viewer.draw(s, &ctx.ui);
        }
        let mut d = Damage::new(s.w, s.h);
        d.all();
        d
    }

    fn window_closed(&mut self, _ctx: &Ctx, window: WindowId) -> bool {
        let before = self.viewers.len();
        self.viewers.retain(|v| v.id != window);
        before != self.viewers.len()
    }

    fn service_reply(&mut self, ctx: &Ctx, request: u32, result: &Result<bifrost_core::json::Json, String>) -> bool {
        self.window.as_mut().is_some_and(|mail| mail.service_reply(ctx, request, result))
    }

    fn document(&mut self, ctx: &Ctx, ev: DocumentEvent) -> bool {
        self.window.as_mut().is_some_and(|mail| mail.document(ctx, ev))
    }

    fn keychain(&mut self, ctx: &Ctx, reply: bifrost_app::keychain::Reply) -> bool {
        self.window.as_mut().is_some_and(|mail| mail.keychain(ctx, reply))
    }

    fn tick(&mut self, ctx: &Ctx, t: f64) -> bool {
        let mut changed = self.open_viewers(ctx) | self.refresh_viewers();
        changed |= self.window.as_mut().is_some_and(|mail| mail.tick(ctx, t));
        changed
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
    bifrost_app::run(MailApp { window: None, viewers: Vec::new(), pending_url: None });
}
