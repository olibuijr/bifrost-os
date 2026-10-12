//! Contacts as an installed BifrOSt app in the Golden Gate full-size window: sidebar, list and card under a unified toolbar.

mod contacts;
mod display_file;
mod dupes;
mod exchange;
mod group_file;
mod history;
mod names;
mod services;

use bifrost_app::kit::{Damage, Event, Surface};
use bifrost_app::controls::Node;
use bifrost_app::{App, AppMenu, Commands, Ctx, Input, KeyInput, TouchKind};
use springboard::shell::keyboard::Out;
use std::collections::BTreeMap;

struct ContactsApp {
    app: Option<contacts::Contacts>,
    /// Whether the app has told the shell it may be killed at once (`ProcessInfo` sudden termination).
    terminable: bool,
    /// What this run has sent to the desktop search index.
    index: services::SearchIndex,
    /// Copies waiting for the pasteboard's reply, by request id, with the contact's label for the status line.
    copies: BTreeMap<u32, String>,
}

impl ContactsApp {
    /// Runs after each input: the termination flag, a Copy Contact press (sent to the pasteboard), and the search index
    /// (sent only when a contact changed).
    fn after_input(&mut self, ctx: &Ctx) {
        self.sync_termination(ctx);
        let Some(c) = &mut self.app else { return };
        if let Some(contact) = c.take_copy() {
            let request = services::copy_contact(ctx, &contact);
            self.copies.insert(request, services::label(&contact));
        }
        if let Some(all) = c.all_contacts() {
            self.index.sync(ctx, all);
        }
    }

    /// Contacts saves each edit when the form is done, so only an open editor holds unsaved state. The shell counts
    /// enable and disable calls, so the state is only sent when it flips.
    fn sync_termination(&mut self, ctx: &Ctx) {
        let unsaved = self.app.as_ref().is_some_and(|c| c.editing());
        if unsaved == self.terminable {
            self.terminable = !unsaved;
            if self.terminable {
                ctx.enable_sudden_termination();
            } else {
                ctx.disable_sudden_termination();
            }
        }
    }
}

impl App for ContactsApp {
    fn id(&self) -> &str {
        "is.olibuijr.contacts.desktop"
    }

    fn configure(&mut self, ctx: &Ctx) {
        let (w, h) = (ctx.cfg.w as i32, ctx.cfg.h as i32);
        match &mut self.app {
            Some(c) => c.set_size(&ctx.ui, w, h),
            None => self.app = Some(contacts::Contacts::new(&ctx.ui, w, h, ctx.data_dir.join("Contacts/contacts.tsv"))),
        }
        self.sync_termination(ctx);
        // The first call indexes every contact; later calls only send what changed since.
        if let Some(all) = self.app.as_ref().and_then(|c| c.all_contacts()) {
            self.index.sync(ctx, all);
        }
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        let Some(c) = &mut self.app else { return false };
        match ev {
            Input::Touch { kind, x, y, t, .. } => {
                let e = match kind {
                    TouchKind::Down => Event::Down(x, y),
                    TouchKind::Move => Event::Move(x, y),
                    TouchKind::Up | TouchKind::Cancel => Event::Up(x, y),
                };
                c.touch(&ctx.ui, e, t);
                let _ = c.take_share(); // the share sheet is a shell feature; not available to packages yet
                self.after_input(ctx);
                true
            }
            Input::Key(k) => {
                let out = match k {
                    KeyInput::Text(s) => Out::Text(s),
                    KeyInput::Code(8) => Out::Backspace,
                    KeyInput::Code(13) => Out::Enter,
                    KeyInput::Code(9) => Out::Named("Tab"),
                    KeyInput::Code(27) => Out::Named("Escape"),
                    KeyInput::Code(37) => Out::Named("Left"),
                    KeyInput::Code(39) => Out::Named("Right"),
                    KeyInput::Code(_) => return false,
                };
                c.key(&ctx.ui, out, bifrost_app::proto::sys::now_ms());
                self.after_input(ctx);
                true
            }
            Input::Command(command) => {
                let handled = c.command(&ctx.ui, command);
                self.after_input(ctx);
                handled
            }
            Input::Activate(id) => {
                let handled = c.activate_control(&ctx.ui, id);
                self.after_input(ctx);
                handled
            }
            Input::Open(_) => false,
            _ => false,
        }
    }

    fn controls(&self, ctx: &Ctx) -> Vec<Node> {
        self.app.as_ref().map(|c| c.controls(&ctx.ui)).unwrap_or_default()
    }

    fn commands(&self, _ctx: &Ctx) -> Commands {
        self.app.as_ref().map(|c| c.commands()).unwrap_or_default()
    }

    fn main_menu(&self, _ctx: &Ctx) -> Vec<AppMenu> {
        self.app.as_ref().map(|c| c.main_menu()).unwrap_or_default()
    }

    fn menu_action(&mut self, ctx: &Ctx, id: u32) -> bool {
        let Some(c) = &mut self.app else { return false };
        let handled = c.menu_action(&ctx.ui, id);
        self.after_input(ctx);
        handled
    }

    fn tick(&mut self, ctx: &Ctx, t: f64) -> bool {
        self.sync_termination(ctx);
        self.app.as_mut().is_some_and(|c| c.tick(&ctx.ui, t))
    }

    fn next_wake(&self, t: f64) -> Option<f64> {
        self.app.as_ref().and_then(|c| c.next_wake(t))
    }

    fn suspend(&mut self, _ctx: &Ctx) {
        if let Some(c) = &mut self.app {
            c.flush();
        }
    }

    fn service_reply(&mut self, _ctx: &Ctx, request: u32, result: &Result<bifrost_core::json::Json, String>) -> bool {
        let Some(label) = self.copies.remove(&request) else { return false };
        if let Some(c) = &mut self.app {
            c.show_status(services::copy_reply_text(&label, result.is_ok()));
        }
        true
    }

    fn draw(&mut self, ctx: &Ctx, s: &mut Surface, t: f64) -> Damage {
        if let Some(c) = &mut self.app {
            c.draw(s, &ctx.ui, t);
        }
        let mut d = Damage::new(s.w, s.h);
        d.all();
        d
    }
}

fn main() {
    bifrost_app::run(ContactsApp { app: None, terminable: false, index: Default::default(), copies: BTreeMap::new() });
}
