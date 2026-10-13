//! Contacts as an installed BifrOSt app in the Golden Gate full-size window: sidebar, list and card under a unified toolbar.

mod archive;
mod contacts;
mod details;
mod display_file;
mod dupes;
mod exchange;
mod extras;
mod group_file;
mod history;
mod names;
mod printing;
mod services;
mod smart;
mod textimport;

use bifrost_app::control::{RpcError, INVALID_PARAMS};
use bifrost_app::controls::internet_accounts::Service;
use bifrost_app::controls::Node;
use bifrost_app::intents::{self, Intent, IntentResult, Param, ParamType};
use bifrost_app::kit::{Damage, Event, Surface};
use bifrost_app::{App, AppMenu, Commands, Ctx, Input, KeyInput, TouchKind};
use bifrost_core::json::Json;
use springboard::shell::keyboard::Out;
use std::collections::BTreeMap;
use std::path::Path;

struct ContactsApp {
    app: Option<contacts::Contacts>,
    /// Whether the app has told the shell it may be killed at once (`ProcessInfo` sudden termination).
    terminable: bool,
    /// What this run has sent to the desktop search index.
    index: services::SearchIndex,
    /// Copies waiting for the pasteboard's reply, by request id, with the contact's label for the status line.
    copies: BTreeMap<u32, String>,
    /// Internet Accounts: the CardDAV address books switched on for Contacts are read in the background.
    dav: bifrost_app::dav_sync::Sync,
}

impl ContactsApp {
    /// Runs after each input: the termination flag, a Copy Contact press (sent to the pasteboard), and the search index
    /// (sent only when a contact changed).
    fn after_input(&mut self, ctx: &Ctx) {
        self.sync_termination(ctx);
        let Some(c) = &mut self.app else { return };
        if let Some((contact, extra)) = c.take_copy() {
            let request = services::copy_contact(ctx, &contact, &extra);
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
        if id == contacts::MENU_REFRESH {
            // the CardDAV sync starts on the next tick
            self.dav.refresh();
            if let Some(c) = &mut self.app {
                c.show_status("Refreshing the CardDAV accounts...".into());
            }
            return true;
        }
        let Some(c) = &mut self.app else { return false };
        let handled = c.menu_action(&ctx.ui, id);
        self.after_input(ctx);
        handled
    }

    fn intents(&self) -> Vec<Intent> {
        vec![
            Intent::new("view-card", "View Card", "Opens the card of a contact, given its name or identifier.")
                .param(Param::new("contact", "Contact", ParamType::String)),
            Intent::new("create-contact", "Create Contact", "Adds a contact and opens its card.")
                .param(Param::new("name", "Name", ParamType::String))
                .param(Param::new("phone", "Phone", ParamType::String).optional())
                .param(Param::new("email", "Email", ParamType::String).optional())
                .param(Param::new("company", "Company", ParamType::String).optional()),
            Intent::new("search-contacts", "Search Contacts", "Shows the contacts that match the text and lists them.")
                .param(Param::new("query", "Query", ParamType::String)),
        ]
    }

    fn perform_intent(&mut self, ctx: Option<&Ctx>, _data_dir: &Path, id: &str, params: &Json) -> Result<IntentResult, RpcError> {
        let (Some(ctx), Some(c)) = (ctx, self.app.as_mut()) else { return Err(intents::failed("Contacts is not running yet")) };
        let optional = |name: &str| params.get(name).and_then(Json::as_str).unwrap_or("").to_string();
        let result = match id {
            "view-card" => {
                let query = intents::str_param(params, "contact")?;
                let (found, title) = c.open_card(&ctx.ui, query).map_err(intents::failed)?;
                Ok(IntentResult::value(Json::Num(found as f64)).with_dialog(&format!("Opened {title}")))
            }
            "create-contact" => {
                let name = intents::str_param(params, "name")?;
                let created = c.create_contact(&ctx.ui, name, &optional("phone"), &optional("email"), &optional("company"));
                created
                    .map(|found| IntentResult::value(Json::Num(found as f64)).with_dialog(&format!("Added {}", name.trim())))
                    .map_err(intents::failed)
            }
            "search-contacts" => {
                let query = intents::str_param(params, "query")?;
                let found = c.search_contacts(&ctx.ui, query);
                let list = Json::Arr(
                    found
                        .iter()
                        .map(|(found, name, phone, email)| {
                            Json::Obj(vec![
                                ("id".into(), Json::Num(*found as f64)),
                                ("name".into(), Json::Str(name.clone())),
                                ("phone".into(), Json::Str(phone.clone())),
                                ("email".into(), Json::Str(email.clone())),
                            ])
                        })
                        .collect(),
                );
                let dialog = if found.len() == 1 { "1 contact matches".to_string() } else { format!("{} contacts match", found.len()) };
                Ok(IntentResult::value(list).with_dialog(&dialog))
            }
            _ => Err(RpcError::new(INVALID_PARAMS, format!("no intent {id}"))),
        };
        self.after_input(ctx);
        result
    }

    fn tick(&mut self, ctx: &Ctx, t: f64) -> bool {
        self.sync_termination(ctx);
        // Internet Accounts: a finished CardDAV sync merges its cards into the contacts.
        if let Some(done) = self.dav.poll(ctx, t, bifrost_app::dav_sync::unix_now()) {
            if let (Ok(texts), Some(c)) = (done.result, self.app.as_mut()) {
                c.merge_card_texts(&ctx.ui, texts.into_iter().map(Some).collect());
            }
        }
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
        if self.dav.reply(request, result) {
            return true;
        }
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
    bifrost_app::run(ContactsApp {
        app: None,
        terminable: false,
        index: Default::default(),
        copies: BTreeMap::new(),
        dav: bifrost_app::dav_sync::Sync::new(Service::Contacts),
    });
}
