//! Shortcuts as an installed BifrOSt app. The text, variable and scripting steps of a shortcut run in the app; the
//! steps that need the shell (Wi-Fi, Low Power, DND, brightness, open app, screenshot, timer) cannot be sent over the
//! app protocol yet and are reported as not run.

mod automation;
use springboard::shortcuts as model;
mod services;
mod widget;

use bifrost_app::control::RpcError;
use bifrost_app::intents::{self, Entity, EntityQuery, Intent, IntentResult};
use bifrost_app::kit::{Damage, Event, Surface};
use bifrost_app::login_items_approval as login;
use bifrost_app::proto::DocMode;
use bifrost_app::{App, AppMenu, AppMenuItem, Command, Ctx, DocumentEvent, Input, TouchKind};
use bifrost_core::json::Json;
use std::io::{Read, Write};
use std::path::Path;

/// The largest shortcut file an import reads, in bytes.
const IMPORT_MAX: u64 = 1 << 20;
/// How often the battery is read for automations, in milliseconds.
const POLL_MS: f64 = 60_000.0;

const MENU_NEW: u32 = 1;
const MENU_DUPLICATE: u32 = 2;
const MENU_IMPORT: u32 = 3;
const MENU_EXPORT: u32 = 4;
const MENU_RUN: u32 = 5;
const MENU_DELETE: u32 = 6;
const MENU_FIND: u32 = 7;
const MENU_ALL: u32 = 8;
const MENU_GALLERY: u32 = 9;
/// The menu item of shortcut `i` is `MENU_RUN_BASE + i`.
const MENU_RUN_BASE: u32 = 100;

/// A file panel the app has open and what it is for.
enum Pending {
    Import,
    Export(String),
}

struct ShortcutsApp {
    w: Option<widget::Shortcuts>,
    /// runs the dry-run plans off the window's thread
    planner: automation::Planner,
    /// the reminders the shell was last sent; `None` until the first sync, when the app clears any left over
    reminders: Option<Vec<services::Reminder>>,
    /// whether the Login Items status has been asked for
    asked_login: bool,
    /// the file panels that are open, by request id
    pending: Vec<(u32, Pending)>,
    /// whether the shell was last told a text field has the keyboard
    keyboard: bool,
    /// when the battery is read next for automations (ms)
    next_poll: f64,
}

impl ShortcutsApp {
    fn new() -> ShortcutsApp {
        ShortcutsApp {
            w: None,
            planner: automation::Planner::new(),
            reminders: None,
            asked_login: false,
            pending: Vec::new(),
            keyboard: false,
            next_poll: 0.0,
        }
    }

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

    /// Brings the shell's reminders in line with the list. Sends only what changed, so it is cheap to call after each
    /// input or script change.
    fn sync(&mut self, ctx: &Ctx) {
        let list = self.shortcuts(&ctx.data_dir);
        self.reminders = Some(services::sync_reminders(ctx, self.reminders.as_deref(), &list));
    }

    /// Does what the last input asked of the shell: queued steps, saved files, file panels, the login toggle, the
    /// keyboard, reminders and intents.
    fn settle(&mut self, ctx: &Ctx) {
        let Some(c) = self.w.as_mut() else { return };
        let t = c.now();
        if !c.take_runs().is_empty() {
            c.show_run_unavailable(t);
        }
        for (name, content) in c.take_saves() {
            match services::save_file(&ctx.data_dir, &name, &content) {
                Ok(_) => c.note("", &format!("Saved {name}"), t),
                Err(e) => c.note("", &e, t),
            }
        }
        let jobs = c.take_file_jobs();
        let requests = c.take_intent_requests();
        let login_request = c.take_login_request();
        let want_keyboard = c.wants_keyboard();
        if let Some(on) = login_request {
            services::set_login(ctx, on);
        }
        if want_keyboard != self.keyboard {
            self.keyboard = want_keyboard;
            ctx.wants_keyboard(want_keyboard);
        }
        for job in jobs {
            match job {
                widget::FileJob::Import => {
                    let request = ctx.open_document(DocMode::Read, "Import Shortcut", &[model::EXPORT_EXT]);
                    self.pending.push((request, Pending::Import));
                }
                widget::FileJob::Export { name, text } => {
                    let request = ctx.save_document(&name, &[model::EXPORT_EXT]);
                    self.pending.push((request, Pending::Export(text)));
                }
            }
        }
        self.sync(ctx);
        for (id, params) in requests {
            self.run_intent(&ctx.data_dir, &id, &params, t);
        }
    }

    /// The menus: File (new, duplicate, import, export), Shortcut (run, delete and one item per shortcut with its menu
    /// key) and View (find, all shortcuts, gallery).
    fn menus(&self) -> Vec<AppMenu> {
        let Some(c) = self.w.as_ref() else { return Vec::new() };
        let open = c.open_shortcut().is_some();
        let file = vec![
            AppMenuItem::new(MENU_NEW, "New Shortcut").key("cmd+n").enabled(c.has_room()),
            AppMenuItem::new(MENU_DUPLICATE, "Duplicate Shortcut").key("cmd+d").enabled(open && c.has_room()),
            AppMenuItem::separator(),
            AppMenuItem::new(MENU_IMPORT, "Import Shortcut...").key("cmd+i").enabled(c.has_room()),
            AppMenuItem::new(MENU_EXPORT, "Export Shortcut...").key("cmd+e").enabled(open),
        ];
        let mut shortcut = vec![
            AppMenuItem::new(MENU_RUN, "Run Shortcut").key("cmd+r").enabled(open),
            AppMenuItem::new(MENU_DELETE, "Delete Shortcut").key("cmd+backspace").enabled(open),
        ];
        if !c.list().is_empty() {
            shortcut.push(AppMenuItem::separator());
        }
        for (i, s) in c.list().iter().enumerate() {
            shortcut.push(AppMenuItem::new(MENU_RUN_BASE + i as u32, &s.name).key(&s.hotkey));
        }
        let view = vec![
            AppMenuItem::new(MENU_FIND, "Find").key("cmd+f"),
            AppMenuItem::separator(),
            AppMenuItem::new(MENU_ALL, "All Shortcuts"),
            AppMenuItem::new(MENU_GALLERY, "Gallery"),
        ];
        vec![
            AppMenu { title: "File".into(), items: file },
            AppMenu { title: "Shortcut".into(), items: shortcut },
            AppMenu { title: "View".into(), items: view },
        ]
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
        let Some(c) = &mut self.w else { return false };
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
                if !c.key(&key) {
                    return false;
                }
            }
            Input::Command(Command::Paste) => {
                let text = ctx.pasteboard_string().unwrap_or_default();
                if !c.paste(&text) {
                    return false;
                }
            }
            _ => return false,
        }
        self.settle(ctx);
        true
    }

    fn commands(&self, _ctx: &Ctx) -> bifrost_app::Commands {
        bifrost_app::Commands::default().with(Command::Paste, self.keyboard)
    }

    fn main_menu(&self, _ctx: &Ctx) -> Vec<AppMenu> {
        self.menus()
    }

    fn menu_action(&mut self, ctx: &Ctx, id: u32) -> bool {
        let Some(c) = self.w.as_mut() else { return false };
        match id {
            MENU_NEW => c.new_shortcut(),
            MENU_DUPLICATE => c.duplicate_open(),
            MENU_IMPORT => c.request_import(),
            MENU_EXPORT => c.request_export(),
            MENU_RUN => c.run_open(),
            MENU_DELETE => c.delete_open(),
            MENU_FIND => c.focus_search(),
            MENU_ALL => c.show_all(),
            MENU_GALLERY => c.show_gallery(),
            id if id >= MENU_RUN_BASE => c.run_index((id - MENU_RUN_BASE) as usize),
            _ => return false,
        }
        self.settle(ctx);
        true
    }

    fn document(&mut self, ctx: &Ctx, ev: DocumentEvent) -> bool {
        match ev {
            DocumentEvent::Opened(mut doc) => {
                let Some(pos) = self.pending.iter().position(|(r, p)| *r == doc.request && matches!(p, Pending::Import)) else { return false };
                self.pending.remove(pos);
                let mut text = String::new();
                let read = (&mut doc.file).take(IMPORT_MAX).read_to_string(&mut text);
                // one import is one use of the file: the app keeps no grant to it
                ctx.release_document(doc.grant);
                if let Some(c) = self.w.as_mut() {
                    match read {
                        Ok(_) => {
                            c.import(&text);
                        }
                        Err(e) => c.failed(&format!("Could not read the file: {e}")),
                    }
                }
                self.settle(ctx);
                true
            }
            DocumentEvent::SaveTo(mut doc) => {
                let Some(pos) = self.pending.iter().position(|(r, p)| *r == doc.request && matches!(p, Pending::Export(_))) else { return false };
                let (_, Pending::Export(text)) = self.pending.remove(pos) else { return false };
                match doc.file.write_all(text.as_bytes()) {
                    Ok(()) => ctx.commit_document(doc.request),
                    Err(e) => {
                        ctx.abort_document(doc.request);
                        if let Some(c) = self.w.as_mut() {
                            c.failed(&format!("Could not export: {e}"));
                        }
                    }
                }
                true
            }
            DocumentEvent::Saved(doc) => {
                ctx.release_document(doc.grant);
                if let Some(c) = self.w.as_mut() {
                    c.exported();
                }
                true
            }
            DocumentEvent::Failed { request, .. } => {
                // the person closed the panel, or the shell refused it: nothing more to do for that request
                self.pending.retain(|(r, _)| *r != request);
                false
            }
            _ => false,
        }
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
        let mut dirty = self.w.as_mut().is_some_and(|c| c.tick(&ctx.ui, t));
        if t >= self.next_poll {
            self.next_poll = t + POLL_MS;
            if self.w.as_ref().is_some_and(|c| c.has_battery_trigger()) {
                let battery = services::battery_percent();
                let started = self.w.as_mut().map(|c| c.check_triggers(battery)).unwrap_or_default();
                if !started.is_empty() {
                    self.settle(ctx);
                    dirty = true;
                }
            }
        }
        dirty
    }

    fn next_wake(&self, t: f64) -> Option<f64> {
        let widget = self.w.as_ref().and_then(|c| c.next_wake(t));
        let poll = self.w.as_ref().filter(|c| c.has_battery_trigger()).map(|_| self.next_poll);
        match (widget, poll) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
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
    bifrost_app::run(ShortcutsApp::new());
}
