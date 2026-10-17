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
use bifrost_app::dnd::{self, dragop, DragPhase, Payload};
use bifrost_app::intents::{self, Entity, EntityQuery, Intent, IntentResult};
use bifrost_app::kit::{Damage, Event, Surface};
use bifrost_app::login_items_approval as login;
use bifrost_app::proto::{DocErrorCode, DocMode};
use bifrost_app::{App, AppMenu, Ctx, DocumentEvent, Input, TouchKind};
use bifrost_core::json::Json;
use springboard::shortcuts::{droplet, flow};
use std::io::{Read, Write};
use std::path::Path;

/// The most bytes read from a file chosen for import; a list of shortcuts is far smaller.
const MAX_IMPORT_BYTES: u64 = 256 * 1024;
/// How long files that arrived without a drop on a tile (a drop on the Dock icon) wait for the rest of their batch
/// before they run, in milliseconds. A drop on a tile is run by its `Perform`, which comes right after the files.
const DOCK_DROP_WAIT_MS: f64 = 400.0;

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
    /// files handed to the app and not yet run: the request, when they arrived (ms) and their paths. A drop on a tile
    /// takes its entry at `Perform`; an entry nobody takes is a drop on the Dock icon and runs after `DOCK_DROP_WAIT_MS`.
    dropped: Vec<(u32, f64, Vec<String>)>,
    /// whether the drag over the window carries files
    drag_files: bool,
}

/// The absolute path of an open file, read from the descriptor table. The shell hands files over as descriptors, so
/// this is the only name the app has; `None` when the kernel cannot name it.
fn path_of(file: &std::fs::File) -> Option<String> {
    use std::os::fd::AsRawFd;
    let path = std::fs::read_link(format!("/proc/self/fd/{}", file.as_raw_fd())).ok()?;
    path.to_str().filter(|p| p.starts_with('/')).map(str::to_string)
}

impl ShortcutsApp {
    fn new() -> Self {
        ShortcutsApp {
            w: None,
            planner: automation::Planner::new(),
            reminders: None,
            asked_login: false,
            keyboard: false,
            export: None,
            import: None,
            app_intents_loaded: false,
            dropped: Vec::new(),
            drag_files: false,
        }
    }

    /// Keeps the files of an inbound batch until a drop takes them (or the Dock-drop wait ends).
    fn keep_dropped(&mut self, request: u32, t: f64, docs: &[bifrost_app::Document]) {
        let paths: Vec<String> = docs.iter().filter_map(|d| path_of(&d.file)).collect();
        if paths.is_empty() {
            return;
        }
        match self.dropped.iter_mut().find(|(r, ..)| *r == request) {
            Some((_, _, have)) => have.extend(paths),
            None => self.dropped.push((request, t, paths)),
        }
    }

    /// The paths that arrived under `request`, taken out of the waiting list.
    fn take_dropped(&mut self, request: u32) -> Vec<String> {
        let at = self.dropped.iter().position(|(r, ..)| *r == request);
        at.map(|i| self.dropped.remove(i).2).unwrap_or_default()
    }

    /// The batches nobody took within the wait, oldest first: drops on the Dock icon.
    fn take_settled(&mut self, now: f64) -> Vec<Vec<String>> {
        let (settled, waiting): (Vec<_>, Vec<_>) = std::mem::take(&mut self.dropped).into_iter().partition(|(_, at, _)| now - *at >= DOCK_DROP_WAIT_MS);
        self.dropped = waiting;
        settled.into_iter().map(|(_, _, paths)| paths).collect()
    }

    /// Runs droplet `index` with the dropped `paths` as its input and shows the steps in the window. `None` when that
    /// shortcut is not a droplet or none of the paths is usable.
    fn run_droplet(&mut self, ctx: &Ctx, index: usize, paths: &[String]) -> Option<Vec<flow::Step>> {
        let list = self.shortcuts(&ctx.data_dir);
        let shortcut = list.get(index)?;
        let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
        let steps = droplet::run(shortcut, &refs, &mut runner::AppHost::new(ctx))?;
        if let Some(c) = self.w.as_mut() {
            let t = c.now();
            c.show_run(&shortcut.name, &steps, t);
        }
        Some(steps)
    }

    /// Files dropped on the Dock icon: they go to the droplet in front, else to the only droplet there is. With
    /// several to choose from the app does not guess; it says where to drop.
    fn dock_drop(&mut self, ctx: &Ctx, paths: &[String]) {
        let list = self.shortcuts(&ctx.data_dir);
        let droplets: Vec<usize> = (0..list.len()).filter(|&i| droplet::accepts(&list[i])).collect();
        let pick = match self.frontmost().filter(|i| droplets.contains(i)) {
            Some(i) => Some(i),
            None if droplets.len() == 1 => Some(droplets[0]),
            None => None,
        };
        match pick {
            Some(i) => {
                if self.run_droplet(ctx, i, paths).is_none() {
                    self.say("None of those files could be passed to the shortcut.");
                }
            }
            None if droplets.is_empty() => self.say("No shortcut takes dropped files. Give one the automation When a File Is Dropped."),
            None => self.say("Drop the files on the tile of the shortcut that should take them."),
        }
    }

    /// The answer to a drag over the window: copy while it carries files and is over a droplet's tile.
    fn answer_drag(&mut self, ctx: &Ctx, session: u32, x: f32, y: f32, allowed: u8) {
        let over = self.drag_files && allowed & dragop::COPY != 0 && self.w.as_ref().and_then(|c| c.droplet_at(&ctx.ui, x, y)).is_some();
        dnd::reply(ctx, session, if over { dragop::COPY } else { 0 });
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
            // files dragged onto a droplet's tile
            dnd::register_types(ctx, &["public.item"]);
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
            // files that came without a request of ours: a drop on a tile, or on the Dock icon
            DocumentEvent::Opened(doc) if bifrost_app::share::is_inbound(doc.request) => {
                let t = bifrost_app::proto::sys::now_ms();
                let request = doc.request;
                self.keep_dropped(request, t, &[doc]);
            }
            DocumentEvent::OpenedMany { request, docs } if bifrost_app::share::is_inbound(request) => {
                let t = bifrost_app::proto::sys::now_ms();
                self.keep_dropped(request, t, &docs);
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
        let settled = self.take_settled(t);
        let ran = !settled.is_empty();
        for paths in settled {
            self.dock_drop(ctx, &paths);
        }
        self.w.as_mut().is_some_and(|c| c.tick(&ctx.ui, t)) || ran
    }

    fn next_wake(&self, t: f64) -> Option<f64> {
        let waiting = self.dropped.iter().map(|(_, at, _)| at + DOCK_DROP_WAIT_MS).reduce(f64::min);
        [self.w.as_ref().and_then(|c| c.next_wake(t)), waiting].into_iter().flatten().reduce(f64::min)
    }

    fn drag(&mut self, ctx: &Ctx, ev: dnd::Event) -> bool {
        let dnd::Event::Dest { session, phase, x, y, allowed, items } = ev else { return false };
        match phase {
            DragPhase::Entered => {
                self.drag_files = items.iter().any(|i| matches!(i.payload, Payload::File { .. }));
                self.answer_drag(ctx, session, x, y, allowed);
                false
            }
            DragPhase::Updated => {
                self.answer_drag(ctx, session, x, y, allowed);
                false
            }
            DragPhase::Exited => {
                self.drag_files = false;
                false
            }
            DragPhase::Perform => {
                self.drag_files = false;
                let target = self.w.as_ref().and_then(|c| c.droplet_at(&ctx.ui, x, y));
                let paths = self.take_dropped(session);
                let ran = match target {
                    Some(i) if !paths.is_empty() => self.run_droplet(ctx, i, &paths).is_some(),
                    _ => false,
                };
                dnd::performed(ctx, session, ran);
                true
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_app::kit::{Font, Ui};
    use bifrost_app::proto::{Configure, ToShell};
    use springboard::shortcuts::{Action, Automation, Shortcut};

    fn ctx() -> Ctx {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets");
        let data: &'static [u8] = Box::leak(std::fs::read(format!("{dir}/SFUI.ttf")).unwrap().into_boxed_slice());
        let cfg = Configure { w: 1200, h: 780, scale: 1.0, safe_top: 0.0, safe_bottom: 0.0, dark: true, text_scale: 1.0, reduce_motion: false };
        Ctx::testing(Ui::new(Font::parse(data).unwrap(), 1.0), cfg)
    }

    fn droplet_shortcut(name: &str) -> Shortcut {
        let mut s = Shortcut::new(name, 0, vec![Action::ShowResult("{Output}".into())]);
        s.automation = Some(Automation::FileDropped);
        s
    }

    /// An app whose window holds `list`.
    fn app_with(ctx: &Ctx, tag: &str, list: Vec<Shortcut>) -> ShortcutsApp {
        let dir = std::env::temp_dir().join(format!("shortcuts-app-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut app = ShortcutsApp::new();
        let mut w = widget::Shortcuts::new(&ctx.ui, 1200, 780, dir.join("shortcuts.tsv"));
        w.edit_list(|l| *l = list);
        app.w = Some(w);
        app
    }

    fn dest(session: u32, phase: DragPhase, at: (f32, f32)) -> dnd::Event {
        let items = vec![dnd::DragItem::new(Payload::File { grant: Default::default(), name: "x.txt".into() })];
        dnd::Event::Dest { session, phase, x: at.0, y: at.1, allowed: dragop::COPY, items }
    }

    fn replies(ctx: &Ctx) -> Vec<ToShell> {
        ctx.take_sent().into_iter().filter(|m| matches!(m, ToShell::DragReply { .. } | ToShell::DragPerformed { .. })).collect()
    }

    #[test]
    fn the_path_of_an_open_file_is_read_from_the_descriptor_table() {
        let path = std::env::temp_dir().join(format!("shortcuts-path-of-{}.txt", std::process::id()));
        std::fs::write(&path, "x").unwrap();
        let file = std::fs::File::open(&path).unwrap();
        if let Some(found) = path_of(&file) {
            assert_eq!(std::path::Path::new(&found), std::fs::canonicalize(&path).unwrap());
        }
    }

    #[test]
    fn a_drag_of_files_is_accepted_over_a_droplet_tile_and_refused_elsewhere() {
        let ctx = ctx();
        let mut app = app_with(&ctx, "hover", vec![Shortcut::new("Plain", 0, vec![Action::ShowResult("x".into())]), droplet_shortcut("Convert")]);
        app.configure(&ctx);
        assert!(ctx.take_sent().iter().any(|m| matches!(m, ToShell::DragRegister { types } if types.iter().any(|t| t == "public.item"))), "the window registers for files");
        let (plain, convert) = {
            let w = app.w.as_ref().unwrap();
            (w.tile_centre(&ctx.ui, 0).unwrap(), w.tile_centre(&ctx.ui, 1).unwrap())
        };
        app.drag(&ctx, dest(7, DragPhase::Entered, convert));
        assert!(matches!(replies(&ctx)[..], [ToShell::DragReply { session: 7, op }] if op == dragop::COPY));
        app.drag(&ctx, dest(7, DragPhase::Updated, plain));
        assert!(matches!(replies(&ctx)[..], [ToShell::DragReply { session: 7, op: 0 }]));
    }

    #[test]
    fn dropping_files_on_a_droplet_runs_it_with_the_paths_as_input() {
        let ctx = ctx();
        let mut app = app_with(&ctx, "perform", vec![droplet_shortcut("Convert")]);
        let at = app.w.as_ref().unwrap().tile_centre(&ctx.ui, 0).unwrap();
        app.drag(&ctx, dest(9, DragPhase::Entered, at));
        ctx.take_sent();
        // the shell hands the files over just before Perform
        app.dropped.push((9, 0.0, vec!["/home/a/x.txt".into(), "/home/a/y.png".into()]));
        assert!(app.drag(&ctx, dest(9, DragPhase::Perform, at)));
        assert!(matches!(replies(&ctx)[..], [ToShell::DragPerformed { session: 9, accepted: true }]));
        let shown = app.w.as_ref().unwrap().result_text().unwrap();
        assert!(shown.starts_with("Convert |") && shown.contains("/home/a/x.txt") && shown.contains("/home/a/y.png"), "{shown}");
        assert!(app.dropped.is_empty(), "the batch was taken");
    }

    #[test]
    fn a_drop_off_a_droplet_or_without_files_is_refused() {
        let ctx = ctx();
        let mut app = app_with(&ctx, "refuse", vec![Shortcut::new("Plain", 0, vec![Action::ShowResult("x".into())]), droplet_shortcut("Convert")]);
        let plain = app.w.as_ref().unwrap().tile_centre(&ctx.ui, 0).unwrap();
        let convert = app.w.as_ref().unwrap().tile_centre(&ctx.ui, 1).unwrap();
        app.dropped.push((3, 0.0, vec!["/a".into()]));
        app.drag(&ctx, dest(3, DragPhase::Perform, plain));
        assert!(matches!(replies(&ctx)[..], [ToShell::DragPerformed { session: 3, accepted: false }]));
        app.drag(&ctx, dest(4, DragPhase::Perform, convert));
        assert!(matches!(replies(&ctx)[..], [ToShell::DragPerformed { session: 4, accepted: false }]), "no files arrived for that session");
        assert!(app.w.as_ref().unwrap().result_text().is_none(), "nothing ran");
    }

    #[test]
    fn files_dropped_on_the_dock_icon_run_the_only_droplet_after_the_wait() {
        let ctx = ctx();
        let mut app = app_with(&ctx, "dock", vec![Shortcut::new("Plain", 0, vec![Action::ShowResult("x".into())]), droplet_shortcut("Convert")]);
        app.dropped.push((5, 1000.0, vec!["/home/a/x.txt".into()]));
        app.tick(&ctx, 1100.0);
        assert_eq!(app.dropped.len(), 1, "still inside the wait: a tile drop may yet claim it");
        assert!(app.w.as_ref().unwrap().result_text().is_none());
        assert_eq!(app.next_wake(1100.0), Some(1000.0 + DOCK_DROP_WAIT_MS));
        assert!(app.tick(&ctx, 1000.0 + DOCK_DROP_WAIT_MS));
        let shown = app.w.as_ref().unwrap().result_text().unwrap();
        assert!(shown.starts_with("Convert |") && shown.contains("/home/a/x.txt"), "{shown}");
        assert!(app.dropped.is_empty());
    }

    #[test]
    fn with_several_droplets_or_none_the_dock_drop_says_where_to_drop() {
        let ctx = ctx();
        let mut two = app_with(&ctx, "dock-two", vec![droplet_shortcut("One"), droplet_shortcut("Two")]);
        two.dropped.push((5, 0.0, vec!["/a".into()]));
        two.tick(&ctx, DOCK_DROP_WAIT_MS);
        assert!(two.w.as_ref().unwrap().result_text().is_none());
        assert!(two.w.as_ref().unwrap().toast_text().unwrap().contains("Drop the files on the tile"));
        let mut none = app_with(&ctx, "dock-none", vec![Shortcut::new("Plain", 0, vec![Action::ShowResult("x".into())])]);
        none.dropped.push((5, 0.0, vec!["/a".into()]));
        none.tick(&ctx, DOCK_DROP_WAIT_MS);
        assert!(none.w.as_ref().unwrap().toast_text().unwrap().contains("When a File Is Dropped"));
    }
}
