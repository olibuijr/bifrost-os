//! Mail as an installed BifrOSt desktop app: hosts `window::MailWindow` through the SDK. The main window shows the
//! mailboxes and messages; viewer windows (`viewer`) and compose windows (`compose_windows`) are windows of their own.

use bifrost_app::app_windows::WindowId;
use bifrost_app::kit::{Damage, Surface};
use bifrost_app::{App, AppMenu, Command, Commands, Ctx, DocumentEvent, Input, TouchKind};
use bifrost_mail::compose_windows::Composes;
use bifrost_mail::viewer::Viewer;
use bifrost_mail::window::MailWindow;

struct MailApp {
    window: Option<MailWindow>,
    /// Viewer windows: a message in a window of its own (double-click, File > New Viewer Window).
    viewers: Vec<Viewer>,
    /// Compose windows, and the one whose compose is in the main window's sheet slot.
    composes: Composes,
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

    /// Runs one call on the main window, then moves compose windows into line with its sheet. True when a window
    /// changed and the app needs a frame.
    fn with_window(&mut self, ctx: &Ctx, f: impl FnOnce(&mut MailWindow) -> bool) -> bool {
        let changed = match &mut self.window {
            Some(mail) => f(mail),
            None => false,
        };
        self.sync(ctx) | changed
    }

    fn sync(&mut self, ctx: &Ctx) -> bool {
        match &mut self.window {
            Some(mail) => self.composes.sync(ctx, mail),
            None => false,
        }
    }
}

impl App for MailApp {
    fn id(&self) -> &str {
        "is.olibuijr.mail.desktop"
    }

    fn configure(&mut self, ctx: &Ctx) {
        let (w, h) = (ctx.cfg.w, ctx.cfg.h);
        self.composes.main_size = (w, h);
        if self.window.is_none() {
            let mut mail = MailWindow::new(ctx);
            mail.set_size(w, h);
            self.window = Some(mail);
            ctx.wants_keyboard(true);
            if let (Some(url), Some(mail)) = (self.pending_url.take(), &mut self.window) {
                mail.open_handoff(&url);
            }
            self.sync(ctx);
        } else if self.composes.active.is_none() {
            // while a compose window is active, the main window's size is the compose window's
            if let Some(mail) = &mut self.window {
                mail.set_size(w, h);
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
        self.route_focus(ctx);
        self.with_window(ctx, |mail| mail.menu(ctx, id))
    }

    /// The window with focus decides which compose a key or a command acts on: the focused compose window's message.
    fn route_focus(&mut self, ctx: &Ctx) {
        let focused = ctx.windows().focused();
        if let Some(mail) = &mut self.window {
            if self.composes.owns(focused) {
                self.composes.activate(mail, focused);
            } else if focused.is_main() {
                self.composes.deactivate(mail);
            }
        }
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        if matches!(ev, Input::Key(_) | Input::Command(_)) {
            self.route_focus(ctx);
        }
        let changed = match &mut self.window {
            None => {
                // a mailto: link before the window exists waits for configure
                if let Input::Open(url) = ev {
                    self.pending_url = Some(url);
                }
                false
            }
            Some(mail) => match ev {
                Input::Touch { kind, x, y, t, .. } => {
                    // a touch on the main window hands the sheet slot back to the main window
                    if matches!(kind, TouchKind::Down) && !self.composes.deactivate(mail) {
                        return false;
                    }
                    mail.touch(ctx, kind, x, y, t)
                }
                Input::Key(k) => mail.key(ctx, k),
                Input::Command(Command::Paste) => mail.paste(),
                Input::Open(url) => {
                    mail.open_handoff(&url);
                    true
                }
                _ => false,
            },
        };
        self.sync(ctx) | changed
    }

    fn window_event(&mut self, ctx: &Ctx, window: WindowId, ev: Input) -> bool {
        if window.is_main() {
            return self.event(ctx, ev);
        }
        if let Some(viewer) = self.viewers.iter_mut().find(|v| v.id == window) {
            return match ev {
                Input::Touch { kind, y, t, .. } => viewer.touch(kind, y, t),
                _ => false,
            };
        }
        let Input::Touch { kind, x, y, t, .. } = ev else { return false };
        let Some(mail) = &mut self.window else { return false };
        if !self.composes.owns(window) {
            return false;
        }
        if matches!(kind, TouchKind::Down) && !self.composes.activate(mail, window) {
            return false;
        }
        if self.composes.active != Some(window) {
            return false;
        }
        let changed = mail.touch(ctx, kind, x, y, t);
        self.sync(ctx) | changed
    }

    fn service_reply(&mut self, ctx: &Ctx, request: u32, result: &Result<bifrost_core::json::Json, String>) -> bool {
        self.with_window(ctx, |mail| mail.service_reply(ctx, request, result))
    }

    fn document(&mut self, ctx: &Ctx, ev: DocumentEvent) -> bool {
        self.with_window(ctx, |mail| mail.document(ctx, ev))
    }

    fn keychain(&mut self, ctx: &Ctx, reply: bifrost_app::keychain::Reply) -> bool {
        self.with_window(ctx, |mail| mail.keychain(ctx, reply))
    }

    fn tick(&mut self, ctx: &Ctx, t: f64) -> bool {
        let mut changed = self.open_viewers(ctx) | self.refresh_viewers();
        if let Some(mail) = &mut self.window {
            self.composes.autosave(mail, t);
        }
        changed |= self.with_window(ctx, |mail| mail.tick(ctx, t));
        changed
    }

    fn draw(&mut self, ctx: &Ctx, s: &mut Surface, _t: f64) -> Damage {
        if let Some(mail) = &mut self.window {
            mail.draw_hiding_compose(s, &ctx.ui, self.composes.active.is_some());
        }
        let mut d = Damage::new(s.w, s.h);
        d.all();
        d
    }

    fn draw_window(&mut self, ctx: &Ctx, window: WindowId, s: &mut Surface, t: f64) -> Damage {
        if window.is_main() {
            return self.draw(ctx, s, t);
        }
        if let Some(viewer) = self.viewers.iter_mut().find(|v| v.id == window) {
            viewer.draw(s, &ctx.ui);
        } else if let Some(mail) = &mut self.window {
            if let Some(compose) = self.composes.compose_of(window) {
                self.composes.resized(window, s.w as f32, s.h as f32);
                // the window that asks the question shows it; the others show the message
                let asking = self.composes.active == Some(window) && matches!(mail.mail.sheet, Some(bifrost_mail::state::Sheet::Question(_)));
                let sheet = if asking { bifrost_mail::state::Sheet::Question(compose) } else { bifrost_mail::state::Sheet::Compose(compose) };
                mail.draw_compose(s, &ctx.ui, &sheet);
            }
        }
        let mut d = Damage::new(s.w, s.h);
        d.all();
        d
    }

    fn window_closed(&mut self, ctx: &Ctx, window: WindowId) -> bool {
        let before = self.viewers.len();
        self.viewers.retain(|v| v.id != window);
        let mut changed = before != self.viewers.len();
        if let Some(mail) = &mut self.window {
            changed |= self.composes.closed(ctx, mail, window);
        }
        changed
    }

    fn next_wake(&self, t: f64) -> Option<f64> {
        self.window.as_ref().and_then(|mail| mail.next_wake(t))
    }

    fn wake_fd(&self) -> Option<i32> {
        self.window.as_ref().and_then(|mail| mail.wake_fd())
    }
}

fn main() {
    bifrost_app::run(MailApp { window: None, viewers: Vec::new(), composes: Composes::default(), pending_url: None });
}
