//! Shortcuts as an installed BifrOSt app. A run walks the steps with `springboard::shortcuts::flow`: text, variables,
//! If and Repeat blocks, copying, notifications and results happen in the app (`runner`). Actions that need the shell
//! (Wi-Fi, Low Power, DND, brightness, open app, screenshot, timer) cannot be sent over the app protocol yet; a run
//! that reaches one stops there and says so.

mod automation;
mod menus;
use springboard::shortcuts as model;
mod runner;
mod services;
mod widget;

use bifrost_app::control::RpcError;
use bifrost_app::intents::{self, Entity, EntityQuery, Intent, IntentResult};
use bifrost_app::kit::{Damage, Event, Surface};
use bifrost_app::login_items_approval as login;
use bifrost_app::proto::{DocErrorCode, DocMode};
use bifrost_app::{App, AppMenu, Ctx, DocumentEvent, Input, TouchKind};
use bifrost_core::json::Json;
use springboard::shortcuts::flow;
use std::io::{Read, Write};
use std::path::Path;

/// The most bytes read from a file chosen for import; a list of shortcuts is far smaller.
const MAX_IMPORT_BYTES: u64 = 256 * 1024;

struct ShortcutsApp {
    w: Option<widget::Shortcuts>,
    /// runs the dry-run plans off the window's thread
    planner: automation::Planner,
    /// the reminders the shell was last sent; `None` until the first sync, when the app clears any left over
    reminders: Option<Vec<services::Reminder>>,
    /// whether the Login Items status has been asked for
    asked_login: bool,
    /// whether the shell was last told the app wants the keyboard
    keyboard: bool,
    /// the export in progress: the save panel's request and the bytes to write once it answers
    export: Option<(u32, Vec<u8>)>,
    /// the request of the open panel an import waits for
    import: Option<u32>,
    /// whether the other apps' shortcut intents have been read from disk (the library lists them)
    app_intents_loaded: bool,
}

impl ShortcutsApp {
    /// The shortcut list: the window's copy when it is open, otherwise the saved file.
    fn shortcuts(&self, data_dir: &Path) -> Vec<model::Shortcut> {
        match &self.w {
            Some(c) => c.list().to_vec(),
            None => model::stored(data_dir),
        }
    }

    /// The shortcut in front in the window (an open editor or picker), if any.
    fn frontmost(&self) -> Option<usize> {
        let shown = self.w.as_ref().map(|c| c.on_screen()).unwrap_or_default();
        automation::frontmost(&automation::onscreen_context(&shown))
    }

    /// Runs a declared intent from the action library through the same path the control socket uses, on the shortcut
    /// in front, and shows its dialog (or the reason it did not run) in the window.
    /// A result with a snippet opens the result panel (its rows, and a follow-on button when it opens an intent);
    /// any other result, or a failure, is a toast.
    fn run_intent(&mut self, data_dir: &Path, id: &str, params: &Json, t: f64) {
        let list = self.shortcuts(data_dir);
        let outcome = automation::perform(&list, id, params, self.frontmost(), &self.planner);
        let Some(c) = self.w.as_mut() else { return };
        match outcome {
            Ok(r) => match r.snippet {
                Some(snippet) => c.show_snippet(&snippet, r.dialog.as_deref(), r.opens, t),
                None => c.show_result(&r.dialog.unwrap_or_else(|| "Done.".to_string()), t),
            },
            Err(e) => c.show_result(&e.message, t),
        }
    }

    /// Does what the window asked for during the last input: runs the shortcuts it queued, opens the panels of an
    /// import or an export, passes on the login toggle, tells the shell whether a field wants the keyboard, and runs
    /// the intents it queued.
    fn settle(&mut self, ctx: &Ctx) {
        let Some(c) = self.w.as_mut() else { return };
        let t = c.now();
        for s in c.take_run_requests() {
            let steps = flow::run(&s.actions, &mut runner::AppHost::new(ctx));
            c.show_run(&s.name, &steps, t);
        }
        let requests = c.take_intent_requests();
        let app_requests = c.take_app_intent_requests();
        let login_request = c.take_login_request();
        let export = c.take_export_request();
        let import = c.take_import_request();
        let keyboard = c.wants_keyboard();
        if let Some(on) = login_request {
            services::set_login(ctx, on);
        }
        if let Some((name, text)) = export {
            self.export = Some((ctx.save_document(&name, &["public.text"]), text.into_bytes()));
        }
        if import {
            self.import = Some(ctx.open_document(DocMode::Read, "Import Shortcuts", &["public.text"]));
        }
        if keyboard != self.keyboard {
            self.keyboard = keyboard;
            ctx.wants_keyboard(keyboard);
        }
        self.sync(ctx);
        for (id, params) in requests {
            self.run_intent(&ctx.data_dir, &id, &params, t);
        }
        for (app_id, name, id) in app_requests {
            self.run_app_intent(&app_id, &name, &id);
        }
    }

    /// Shows `text` in the window for a few seconds.
    fn say(&mut self, text: &str) {
        if let Some(c) = self.w.as_mut() {
            let t = c.now();
            c.say(text, t);
        }
    }

    /// Brings the shell's reminders in line with the list. Sends only what changed, so it is cheap to call after each
    /// input or script change.
    fn sync(&mut self, ctx: &Ctx) {
        let list = self.shortcuts(&ctx.data_dir);
        self.reminders = Some(services::sync_reminders(ctx, self.reminders.as_deref(), &list));
        self.sync_app_intents();
    }

    /// Gives the library the shortcut intents of the installed apps. They are read from the manifests on disk, again
    /// only when the apps registry changed (a cheap stat), so an input never rescans the apps.
    fn sync_app_intents(&mut self) {
        let changed = springboard::shell::apphost::disk_changed();
        if self.app_intents_loaded && !changed {
            return;
        }
        self.app_intents_loaded = true;
        let list = automation::app_intents(&springboard::shell::apphost::root());
        if let Some(c) = self.w.as_mut() {
            c.set_app_intents(list);
        }
    }

    /// Runs an intent another app declares, from the library's "From Apps" section, and shows its dialog (or why it
    /// did not run) in the window.
    fn run_app_intent(&mut self, app_id: &str, app_name: &str, intent_id: &str) {
        let outcome = automation::run_app_intent(app_id, app_name, intent_id);
        let Some(c) = self.w.as_mut() else { return };
        let t = c.now();
        match outcome {
            Ok(dialog) => c.show_result(&dialog.unwrap_or_else(|| "Done.".to_string()), t),
            Err(e) => c.show_result(&e, t),
        }
    }
}

impl App for ShortcutsApp {
    fn id(&self) -> &str {
        "is.olibuijr.shortcuts.desktop"
    }

    fn configure(&mut self, ctx: &Ctx) {
        let (w, h) = (ctx.cfg.w as i32, ctx.cfg.h as i32);
        match &mut self.w {
            Some(c) => c.set_size(w, h),
            None => self.w = Some(widget::Shortcuts::new(&ctx.ui, w, h, ctx.data_dir.join("shortcuts.tsv"))),
        }
        if !self.asked_login {
            self.asked_login = true;
            services::ask_login(ctx);
        }
        self.sync(ctx);
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        let Some(c) = self.w.as_mut() else { return false };
        match ev {
            Input::Touch { kind, x, y, t, .. } => {
                let e = match kind {
                    TouchKind::Down => Event::Down(x, y),
                    TouchKind::Move => Event::Move(x, y),
                    TouchKind::Up | TouchKind::Cancel => Event::Up(x, y),
                };
                c.touch(&ctx.ui, e, t);
            }
            Input::Key(key) => {
                if !c.key(key) {
                    return false;
                }
            }
            _ => return false,
        }
        self.settle(ctx);
        true
    }

    fn main_menu(&self, _ctx: &Ctx) -> Vec<AppMenu> {
        match &self.w {
            Some(c) => menus::build(c.list(), c.in_editor()),
            None => Vec::new(),
        }
    }

    fn menu_action(&mut self, ctx: &Ctx, id: u32) -> bool {
        let (Some(command), Some(c)) = (menus::command(id), self.w.as_mut()) else { return false };
        c.command(command);
        self.settle(ctx);
        true
    }

    fn document(&mut self, ctx: &Ctx, ev: DocumentEvent) -> bool {
        match ev {
            DocumentEvent::SaveTo(mut doc) => match self.export.take() {
                Some((request, bytes)) if request == doc.request => match doc.file.write_all(&bytes) {
                    Ok(()) => ctx.commit_document(request),
                    Err(e) => {
                        ctx.log(&format!("export shortcuts: {e}"));
                        ctx.abort_document(request);
                        self.say("The shortcuts could not be written.");
                    }
                },
                other => self.export = other,
            },
            DocumentEvent::Saved(_) => self.say("Exported"),
            DocumentEvent::Opened(mut doc) if self.import == Some(doc.request) => {
                self.import = None;
                let mut text = String::new();
                let read = (&mut doc.file).take(MAX_IMPORT_BYTES).read_to_string(&mut text);
                match read {
                    Ok(_) => {
                        if let Some(c) = self.w.as_mut() {
                            let t = c.now();
                            c.import_text(&text, t);
                        }
                    }
                    Err(_) => self.say("That file could not be read as text."),
                }
            }
            DocumentEvent::Failed { request, code } => {
                if self.import == Some(request) {
                    self.import = None;
                }
                if self.export.as_ref().is_some_and(|(r, _)| *r == request) {
                    self.export = None;
                }
                if code != DocErrorCode::Cancelled {
                    ctx.log(&format!("document: {code:?}"));
                    self.say("The file could not be used.");
                }
            }
            _ => {}
        }
        true
    }

    fn service_reply(&mut self, _ctx: &Ctx, _request: u32, result: &Result<Json, String>) -> bool {
        if let Err(msg) = result {
            eprintln!("shortcuts: shell service: {msg}");
        }
        if let (Some(status), Some(c)) = (login::status_from_reply(result), self.w.as_mut()) {
            c.set_login(Some(status));
        }
        true
    }

    fn service_event(&mut self, _ctx: &Ctx, service: &str, event: &str, body: &Json) -> bool {
        match (login::status_from_event(service, event, body), self.w.as_mut()) {
            (Some(status), Some(c)) => {
                c.set_login(Some(status));
                true
            }
            _ => false,
        }
    }

    fn tick(&mut self, ctx: &Ctx, t: f64) -> bool {
        self.w.as_mut().is_some_and(|c| c.tick(&ctx.ui, t))
    }

    fn next_wake(&self, t: f64) -> Option<f64> {
        self.w.as_ref().and_then(|c| c.next_wake(t))
    }

    fn controls(&self, ctx: &Ctx) -> Vec<bifrost_app::controls::Node> {
        self.w.as_ref().map(|c| c.controls(&ctx.ui)).unwrap_or_default()
    }

    fn intents(&self) -> Vec<Intent> {
        automation::declared()
    }

    fn perform_intent(&mut self, _ctx: Option<&Ctx>, data_dir: &Path, id: &str, params: &Json) -> Result<IntentResult, RpcError> {
        automation::perform(&self.shortcuts(data_dir), id, params, self.frontmost(), &self.planner)
    }

    fn entities(&mut self, data_dir: &Path, ty: &str, query: &EntityQuery) -> Result<Vec<Entity>, RpcError> {
        automation::entities(&self.shortcuts(data_dir), ty, query)
    }

    fn rpc_methods(&self) -> Vec<&'static str> {
        vec!["script.dictionary", "script.get", "script.set"]
    }

    fn rpc(&mut self, ctx: Option<&Ctx>, data_dir: &Path, method: &str, params: &Json) -> Result<Json, RpcError> {
        match method {
            "script.dictionary" => Ok(automation::dictionary_json()),
            "script.get" => automation::script_get(&self.shortcuts(data_dir), params),
            "script.set" => {
                let out = match self.w.as_mut() {
                    Some(c) => c.edit_list(|l| automation::script_set(l, params)),
                    None => {
                        // no window yet: change the saved file, so the next launch shows the change
                        let mut list = model::stored(data_dir);
                        automation::script_set(&mut list, params).and_then(|out| {
                            model::save(&data_dir.join(model::FILE), &list)
                                .map(|_| out)
                                .map_err(|e| intents::failed(format!("could not save the list: {e}")))
                        })
                    }
                };
                // a changed reminder time reaches the shell now; without a context it waits for the next input
                if let (Some(ctx), Ok(_)) = (ctx, &out) {
                    self.sync(ctx);
                }
                out
            }
            _ => Err(RpcError::new(bifrost_app::control::METHOD_NOT_FOUND, format!("no method {method}"))),
        }
    }

    fn draw(&mut self, ctx: &Ctx, s: &mut Surface, t: f64) -> Damage {
        if let Some(c) = &mut self.w {
            c.draw(s, &ctx.ui, t);
        }
        let mut d = Damage::new(s.w, s.h);
        d.all();
        d
    }
}

fn main() {
    bifrost_app::run(ShortcutsApp {
        w: None,
        planner: automation::Planner::new(),
        reminders: None,
        asked_login: false,
        keyboard: false,
        export: None,
        import: None,
        app_intents_loaded: false,
    });
}
