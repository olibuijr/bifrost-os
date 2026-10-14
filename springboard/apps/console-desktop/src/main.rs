//! Console as an installed BifrOSt system app: the system journal, the crash reports and the files in `/var/log`.
//! The window is `console.rs` and `draw.rs`; the reading is `source.rs`; the entries, the parsers and the search
//! are `model.rs`. Files opened and archives captured are `opened.rs` and `capture.rs`; the kernel's crash records are
//! `kernel.rs`. This file is the glue to the shell: the menus and the document, workspace and power services.

mod activity;
mod capture;
mod columns;
mod console;
mod cursor;
mod diagnostics;
mod draw;
mod export;
mod find;
mod identity;
mod kernel;
mod model;
mod opened;
mod rows;
mod security;
mod services;
mod signposts;
mod source;
#[cfg(test)]
mod filter_tests;
#[cfg(test)]
mod toolbar_tests;
#[cfg(test)]
mod wire_tests;

use bifrost_app::controls::Node;
use bifrost_app::kit::{Damage, Surface};
use bifrost_app::power_service;
use bifrost_app::proto::{DocErrorCode, DocMode, GrantId};
use bifrost_app::share::INBOUND;
use bifrost_app::workspace::{self, Status};
use bifrost_app::{App, AppMenu, AppMenuItem, Ctx, DocumentEvent, Input, TouchKind};
use bifrost_core::json::Json;
use console::*;
use model::LogEntry;
use opened::OpenedLog;
use services::PowerNote;
use std::cell::RefCell;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// What a save panel was asked for.
#[derive(Clone, Debug, PartialEq)]
enum Saving {
    /// File > Save Messages, with the text to write.
    Messages(String),
    /// File > Capture Archive.
    Archive,
}

/// An answer from the workspace service, left by its completion closure for `tick` to apply.
#[derive(Debug, PartialEq)]
enum FileNote {
    Revealed(bool),
    Trashed { grant: GrantId, ok: bool },
}

#[derive(Default)]
struct ConsoleApp {
    console: Option<Console>,
    /// The power service's state, shown in the footer.
    power: PowerNote,
    /// The request for the power snapshot, asked once when the window opens.
    power_request: Option<u32>,
    /// Copy Message writes in flight, each with the text to put on the plain clipboard if the shell has no pasteboard service.
    copy_pending: Vec<(u32, String)>,
    /// The open panel's request (File > Open).
    open_request: Option<u32>,
    /// The save panel's request and what it is for.
    saving: Option<(u32, Saving)>,
    /// The archive capture running on its queue.
    capture: Option<capture::Capture>,
    /// The entries the capture read, kept until the saved file's grant arrives.
    captured: Option<Vec<LogEntry>>,
    /// Reveal in Files and Move to Trash answers, applied by `tick`.
    file_notes: Rc<RefCell<Vec<FileNote>>>,
}

impl ConsoleApp {
    /// Pushes the power line into the window's footer.
    fn sync_power(&mut self) {
        let line = self.power.line();
        if let Some(c) = &mut self.console {
            c.power_note = line;
        }
    }

    /// Log menu: Copy Message. The line goes to the general board; an older shell answers with an error, and then the
    /// text goes to the plain clipboard (see `service_reply`).
    fn copy_selected(&mut self, ctx: &Ctx) {
        let Some(text) = self.console.as_ref().and_then(|c| c.selected_entry()).map(services::copy_line) else { return };
        let id = services::copy_to_general(ctx, &text);
        self.copy_pending.push((id, text));
    }

    /// Puts a note in the window's status line.
    fn say(&mut self, text: impl Into<String>) {
        if let Some(c) = &mut self.console {
            c.status = text.into();
        }
    }

    /// File > Save Messages: the save panel picks the place; the text is written when the shell answers `SaveTo`.
    fn begin_save_messages(&mut self, ctx: &Ctx) {
        let Some(c) = &self.console else { return };
        let text = c.messages_to_save();
        if text.is_empty() {
            self.say("No messages to save.");
            return;
        }
        let request = ctx.save_document(&export::file_name(bifrost_app::testing::unix_secs()), &["log", "txt"]);
        self.saving = Some((request, Saving::Messages(text)));
    }

    /// File > Open: the open panel picks a log file or an archive.
    fn begin_open(&mut self, ctx: &Ctx) {
        self.open_request = Some(ctx.open_document(DocMode::Read, "Open Log", &[]));
    }

    /// File > Capture Archive: the save panel picks the place; the capture starts when the shell answers `SaveTo`.
    fn begin_capture(&mut self, ctx: &Ctx) {
        if self.capture.is_some() || matches!(self.saving, Some((_, Saving::Archive))) {
            self.say("An archive is already being captured.");
            return;
        }
        let request = ctx.save_document(&capture::file_name(bifrost_app::testing::unix_secs()), &["logarchive"]);
        self.saving = Some((request, Saving::Archive));
    }

    /// The grant of the opened log in view, when it is a log the app may reveal; with `writable` only a writable one.
    fn active_grant(&self, writable: bool) -> Option<GrantId> {
        self.console.as_ref()?.active_opened().filter(|o| o.writable || !writable).and_then(|o| o.grant)
    }

    /// File > Reveal in Files.
    fn reveal_active(&mut self, ctx: &Ctx) {
        let Some(grant) = self.active_grant(false) else { return };
        let notes = self.file_notes.clone();
        workspace::activate_file_viewer_selecting(ctx, &[grant], move |status| notes.borrow_mut().push(FileNote::Revealed(status == Status::Ok)));
    }

    /// File > Move to Trash.
    fn trash_active(&mut self, ctx: &Ctx) {
        let Some(grant) = self.active_grant(true) else { return };
        let notes = self.file_notes.clone();
        workspace::recycle(ctx, &[grant], move |status, moved| notes.borrow_mut().push(FileNote::Trashed { grant, ok: status == Status::Ok && moved > 0 }));
    }

    /// Applies the workspace answers that arrived. True when the window changed.
    fn apply_file_notes(&mut self, ctx: &Ctx) -> bool {
        let notes = std::mem::take(&mut *self.file_notes.borrow_mut());
        let changed = !notes.is_empty();
        for note in notes {
            match note {
                FileNote::Revealed(true) => {}
                FileNote::Revealed(false) => self.say("Files could not show the file."),
                FileNote::Trashed { grant, ok: true } => {
                    if let Some(c) = &mut self.console {
                        c.forget_opened(grant);
                    }
                    ctx.release_document(grant);
                    self.say("Moved to the Trash.");
                }
                FileNote::Trashed { ok: false, .. } => self.say("The file could not be moved to the Trash."),
            }
        }
        changed
    }

    /// Reads the file the open panel returned and shows it. The window learns the grant, so Reveal in Files works.
    fn opened_document(&mut self, ctx: &Ctx, mut doc: bifrost_app::Document) {
        let mut bytes = Vec::new();
        let read = (&mut doc.file).take(opened::MAX_INPUT as u64 + 1).read_to_end(&mut bytes).map_err(|e| e.to_string());
        let decoded = read.and_then(|_| if bytes.len() > opened::MAX_INPUT { Err("The file is too large.".to_string()) } else { opened::decode(&doc.name, &bytes) });
        match (decoded, &mut self.console) {
            (Ok(entries), Some(c)) => {
                let log = OpenedLog { name: doc.name.clone(), entries, grant: Some(doc.grant), writable: doc.mode == DocMode::Write };
                if let Some(dropped) = c.add_opened(log) {
                    ctx.release_document(dropped);
                }
            }
            (decoded, console) => {
                if let (Err(e), Some(c)) = (decoded, console) {
                    c.status = format!("Could not open {}. {e}", doc.name);
                }
                ctx.release_document(doc.grant);
            }
        }
    }

    /// The save panel returned a file to write: the messages go in now, an archive capture starts.
    fn save_to(&mut self, ctx: &Ctx, mut doc: bifrost_app::Document) {
        let kind = self.saving.as_ref().filter(|(r, _)| *r == doc.request).map(|(_, k)| k.clone());
        match kind {
            Some(Saving::Messages(text)) => match doc.file.write_all(text.as_bytes()) {
                Ok(()) => ctx.commit_document(doc.request),
                Err(e) => {
                    ctx.abort_document(doc.request);
                    self.saving = None;
                    self.say(format!("Could not save: {e}"));
                }
            },
            Some(Saving::Archive) => {
                self.capture = Some(capture::Capture::start(doc.request, doc.file));
                self.say("Capturing the journal...");
            }
            None => ctx.abort_document(doc.request),
        }
    }

    /// The save is in place. Messages: a note. Archive: the captured entries open as a log with the new file's grant.
    fn saved(&mut self, ctx: &Ctx, doc: bifrost_app::Document) {
        let kind = self.saving.take_if(|(r, _)| *r == doc.request).map(|(_, k)| k);
        match (kind, &mut self.console) {
            (Some(Saving::Archive), Some(c)) => {
                let entries = self.captured.take().unwrap_or_default();
                let log = OpenedLog { name: doc.name.clone(), entries, grant: Some(doc.grant), writable: doc.mode == DocMode::Write };
                c.status.clear();
                if let Some(dropped) = c.add_opened(log) {
                    ctx.release_document(dropped);
                }
            }
            (Some(Saving::Messages(_)), Some(c)) => {
                c.status = format!("Saved {}", doc.name);
                ctx.release_document(doc.grant);
            }
            _ => ctx.release_document(doc.grant),
        }
    }

    /// A document request failed or was cancelled. A cancel is silent. A shell without the document service still
    /// gets Save Messages: the text goes to Documents/Console as it did before the save panel.
    fn document_failed(&mut self, request: u32, code: DocErrorCode, home: &Path) {
        if self.open_request == Some(request) {
            self.open_request = None;
            if code != DocErrorCode::Cancelled {
                self.say(format!("Could not open the file. {}", failure_text(code)));
            }
            return;
        }
        let Some((_, kind)) = self.saving.take_if(|(r, _)| *r == request) else { return };
        self.captured = None;
        match kind {
            Saving::Messages(_) if code == DocErrorCode::Unsupported => {
                if let Some(c) = &mut self.console {
                    c.save_messages(home, bifrost_app::testing::unix_secs());
                }
            }
            _ if code == DocErrorCode::Cancelled => {}
            Saving::Messages(_) => self.say(format!("Could not save. {}", failure_text(code))),
            Saving::Archive => self.say(format!("Could not save the archive. {}", failure_text(code))),
        }
    }
}

/// The account's home directory, where Save Messages writes when the shell has no save panel.
fn home_dir() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default()
}

/// A sentence for a document request that failed.
fn failure_text(code: DocErrorCode) -> &'static str {
    match code {
        DocErrorCode::Cancelled => "It was cancelled.",
        DocErrorCode::Denied => "Access was denied.",
        DocErrorCode::NotFound => "The file was not found.",
        DocErrorCode::Stale => "The file changed after it was chosen.",
        DocErrorCode::Revoked => "Access to the file was withdrawn.",
        DocErrorCode::Unsupported => "Files are not available to this app.",
    }
}

impl App for ConsoleApp {
    fn id(&self) -> &str {
        "is.olibuijr.console.desktop"
    }

    fn configure(&mut self, ctx: &Ctx) {
        ctx.enable_sudden_termination(); // keeps no unsaved state
        let (w, h) = (ctx.cfg.w as f32, ctx.cfg.h as f32);
        match &mut self.console {
            Some(c) => c.resize(w, h),
            None => {
                let mut c = Console::new(w, h, source::log_files());
                c.open_view(View::Live);
                self.console = Some(c);
            }
        }
        // one snapshot when the window opens (a real event, not a frame); later changes are pushed
        if self.power_request.is_none() {
            self.power_request = Some(power_service::snapshot(ctx));
        }
    }

    fn service_reply(&mut self, ctx: &Ctx, request: u32, result: &Result<Json, String>) -> bool {
        if self.power_request == Some(request) {
            // an older shell answers an error for "power"; the footer then shows nothing
            if let Some(snapshot) = power_service::decode_snapshot(result) {
                self.power.apply_snapshot(snapshot);
                self.sync_power();
                return true;
            }
            return false;
        }
        if let Some(at) = self.copy_pending.iter().position(|(id, _)| *id == request) {
            let (_, text) = self.copy_pending.remove(at);
            if result.is_err() {
                ctx.set_pasteboard_string(&text);
            }
        }
        false
    }

    fn service_event(&mut self, _ctx: &Ctx, service: &str, event: &str, body: &Json) -> bool {
        let Some(change) = power_service::decode_change(service, event, body) else { return false };
        self.power.apply_change(change);
        self.sync_power();
        true
    }

    fn controls(&self, ctx: &Ctx) -> Vec<Node> {
        self.console.as_ref().map_or_else(Vec::new, |c| c.controls(&ctx.ui))
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        let Some(c) = &mut self.console else { return false };
        let changed = match ev {
            Input::Touch { kind, x, y, t, .. } => c.touch(&ctx.ui, kind, x, y, t),
            Input::Key(key) => c.key_input(key),
            // a published toolbar control (accessibility, `bifrost pc midget tap-window`): a click at its centre
            Input::Activate(id) => {
                let Some(node) = c.controls(&ctx.ui).into_iter().find(|n| n.id == id && n.enabled) else { return false };
                let x = ctx.ui.pt(node.bounds.x + node.bounds.w / 2.0);
                let y = ctx.ui.pt(node.bounds.y + node.bounds.h / 2.0);
                let t = bifrost_app::proto::sys::now_ms();
                c.touch(&ctx.ui, TouchKind::Down, x, y, t);
                c.touch(&ctx.ui, TouchKind::Up, x, y, t)
            }
            _ => false,
        };
        ctx.wants_keyboard(c.search_focused);
        changed
    }

    fn tick(&mut self, ctx: &Ctx, t: f64) -> bool {
        let mut changed = self.apply_file_notes(ctx);
        // the archive capture finished: put the saved file in place (its entries open when the shell answers `Saved`)
        let done = self.capture.as_ref().and_then(|cap| cap.take().map(|result| (cap.request, result)));
        if let Some((request, result)) = done {
            self.capture = None;
            match result {
                Ok(entries) => {
                    self.captured = Some(entries);
                    ctx.commit_document(request);
                    self.say("Saving the archive...");
                }
                Err(e) => {
                    ctx.abort_document(request);
                    self.saving = None;
                    self.say(e);
                }
            }
            changed = true;
        }
        let Some(c) = &mut self.console else { return changed };
        let moving = c.animating(t);
        let drained = c.drain();
        changed || moving || drained
    }

    fn document(&mut self, ctx: &Ctx, ev: DocumentEvent) -> bool {
        match ev {
            DocumentEvent::Opened(doc) if self.open_request == Some(doc.request) => {
                self.open_request = None;
                self.opened_document(ctx, doc);
            }
            DocumentEvent::SaveTo(doc) => self.save_to(ctx, doc),
            DocumentEvent::Saved(doc) => self.saved(ctx, doc),
            DocumentEvent::Failed { request, code } => self.document_failed(request, code, &home_dir()),
            // files the shell hands over (opened with Console or dropped on it) carry the inbound bit; they open as logs
            DocumentEvent::Opened(doc) if doc.request & INBOUND != 0 => self.opened_document(ctx, doc),
            DocumentEvent::OpenedMany { request, docs } if request & INBOUND != 0 => docs.into_iter().for_each(|doc| self.opened_document(ctx, doc)),
            // a grant this window did not ask for: nothing holds it
            DocumentEvent::Opened(doc) => ctx.release_document(doc.grant),
            DocumentEvent::OpenedMany { docs, .. } => docs.iter().for_each(|d| ctx.release_document(d.grant)),
        }
        true
    }

    fn main_menu(&self, _ctx: &Ctx) -> Vec<AppMenu> {
        self.console.as_ref().map_or_else(Vec::new, |c| menus(c, self.active_grant(false).is_some(), self.active_grant(true).is_some()))
    }

    fn menu_action(&mut self, ctx: &Ctx, id: u32) -> bool {
        match id {
            MENU_COPY => self.copy_selected(ctx),
            MENU_SAVE_MESSAGES => self.begin_save_messages(ctx),
            MENU_OPEN => self.begin_open(ctx),
            MENU_CAPTURE => self.begin_capture(ctx),
            MENU_REVEAL => self.reveal_active(ctx),
            MENU_TRASH => self.trash_active(ctx),
            _ => {
                let Some(c) = &mut self.console else { return false };
                let changed = c.menu(id);
                ctx.wants_keyboard(c.search_focused);
                return changed;
            }
        }
        true
    }

    fn draw(&mut self, ctx: &Ctx, s: &mut Surface, _t: f64) -> Damage {
        if let Some(c) = &mut self.console {
            c.draw(&ctx.ui, s);
        }
        let mut d = Damage::new(s.w, s.h);
        d.all();
        d
    }
}


/// The app's menus. The shell's File, Edit and View menus take the items of the same name (`desktop_bar`), so the file,
/// find and view commands sit where a user looks for them; the Log menu is the Console's own. `can_reveal` and
/// `can_trash` say whether the log in view has a grant (and a writable one).
fn menus(c: &Console, can_reveal: bool, can_trash: bool) -> Vec<AppMenu> {
    let stream = if c.streaming && c.is_live() { "Pause Streaming" } else { "Start Streaming" };
    let mut file = vec![AppMenuItem::new(MENU_OPEN, "Open...").key("cmd+o"), AppMenuItem::submenu("Open Recent").enabled(!c.opened.is_empty())];
    // the newest first
    for (i, log) in c.opened.iter().rev().enumerate().take(MAX_OPENED) {
        file.push(AppMenuItem::new(MENU_RECENT + i as u32, &log.name).depth(1));
    }
    file.extend([
        AppMenuItem::separator(),
        AppMenuItem::new(MENU_CAPTURE, "Capture Archive..."),
        AppMenuItem::separator(),
        AppMenuItem::new(MENU_REVEAL, "Reveal in Files").enabled(can_reveal),
        AppMenuItem::new(MENU_TRASH, "Move to Trash").enabled(can_trash),
    ]);
    let edit = vec![
        AppMenuItem::new(MENU_FIND_NEXT, "Find Next").key("cmd+g").enabled(!c.visible.is_empty()),
        AppMenuItem::new(MENU_FIND_PREVIOUS, "Find Previous").key("cmd+shift+g").enabled(!c.visible.is_empty()),
        AppMenuItem::new(MENU_USE_SELECTION, "Use Selection for Find").key("cmd+e").enabled(c.selected_entry().is_some()),
    ];
    let mut view = vec![
        AppMenuItem::new(MENU_RELOAD, "Reload").key("cmd+r"),
        AppMenuItem::separator(),
        AppMenuItem::new(MENU_SIDEBAR, "Show Sidebar").key("ctrl+cmd+s").checked(c.show_sidebar),
        AppMenuItem::new(MENU_DETAILS, "Show Details").checked(c.show_details),
        AppMenuItem::new(MENU_TOOLBAR, if c.show_toolbar { "Hide Toolbar" } else { "Show Toolbar" }),
        AppMenuItem::new(MENU_ACTIVITY, if c.activity.is_some() { "Show All Activities" } else { "Show Only This Activity" })
            .enabled(c.activity.is_some() || c.selected_entry().is_some_and(|e| !e.invocation.is_empty())),
        AppMenuItem::separator(),
        AppMenuItem::submenu("Visible Columns"),
    ];
    for (i, col) in Col::ALL.iter().enumerate() {
        view.push(AppMenuItem::new(MENU_COLUMN + i as u32, col.title()).checked(c.columns.shows(*col)).depth(1));
    }
    view.extend([
        AppMenuItem::separator(),
        AppMenuItem::new(MENU_EXPAND, if c.selected_expanded() { "Collapse Message" } else { "Expand Message" }).enabled(c.selected_entry().is_some()),
        AppMenuItem::new(MENU_COLLAPSE_ALL, "Collapse All Messages").enabled(c.any_expanded()),
    ]);
    let log = vec![
        AppMenuItem::new(MENU_STREAM, stream).enabled(c.is_live()),
        AppMenuItem::separator(),
        AppMenuItem::new(MENU_INFO, "Include Info Messages").checked(c.levels.info),
        AppMenuItem::new(MENU_DEBUG, "Include Debug Messages").checked(c.levels.debug),
        AppMenuItem::separator(),
        AppMenuItem::new(MENU_FIND, "Find Message").key("cmd+f"),
        AppMenuItem::new(MENU_SAVE_SEARCH, "Save Search").enabled(c.can_save_search()),
        AppMenuItem::new(MENU_DELETE_SEARCH, "Delete Saved Search").enabled(c.can_delete_search()),
        AppMenuItem::new(MENU_CLEAR, "Clear Display").key("cmd+k"),
        AppMenuItem::separator(),
        AppMenuItem::new(MENU_AFTER, if c.after.is_some() { "Show All Messages" } else { "Show Messages After Selected" })
            .enabled(c.after.is_some() || c.selected.is_some()),
        AppMenuItem::new(MENU_SUBSYSTEM, "Show Only This Subsystem").enabled(c.selected_entry().is_some_and(|e| !e.subsystem.is_empty())),
        AppMenuItem::separator(),
        AppMenuItem::new(MENU_COPY, "Copy Message").enabled(c.selected_entry().is_some()),
        AppMenuItem::new(MENU_SAVE_MESSAGES, "Save Messages...").enabled(!c.visible.is_empty()),
    ];
    vec![
        AppMenu { title: "File".into(), items: file },
        AppMenu { title: "Find".into(), items: edit },
        AppMenu { title: "View".into(), items: view },
        AppMenu { title: "Log".into(), items: log },
    ]
}

fn main() {
    bifrost_app::run(ConsoleApp::default());
}


#[cfg(test)]
mod glue_tests {
    use super::*;
    use crate::model::{Level, LogEntry};
    use bifrost_app::proto::ToShell;
    use bifrost_core::json::parse;

    fn app() -> ConsoleApp {
        ConsoleApp::default()
    }

    fn reply(text: &str) -> Result<Json, String> {
        Ok(parse(text).unwrap())
    }

    #[test]
    fn a_snapshot_reply_fills_the_power_note() {
        let ctx = services::test_ctx();
        let mut a = app();
        a.power_request = Some(7);
        let body = reply(r#"{"thermalState":"nominal","lowPowerMode":true,"battery":{"present":false}}"#);
        assert!(a.service_reply(&ctx, 7, &body));
        assert_eq!(a.power.line(), "Thermal Nominal · Low Power Mode");
    }

    #[test]
    fn an_older_shell_error_for_power_leaves_the_footer_empty() {
        let ctx = services::test_ctx();
        let mut a = app();
        a.power_request = Some(7);
        assert!(!a.service_reply(&ctx, 7, &Err("no service power".to_string())));
        assert_eq!(a.power.line(), "");
    }

    #[test]
    fn a_pushed_power_change_updates_the_note_and_other_services_are_ignored() {
        let ctx = services::test_ctx();
        let mut a = app();
        let body = parse(r#"{"thermalState":"serious"}"#).unwrap();
        assert!(!a.service_event(&ctx, "pasteboard", "thermalStateDidChange", &body));
        assert_eq!(a.power.line(), "");
        assert!(a.service_event(&ctx, "power", "thermalStateDidChange", &body));
        assert_eq!(a.power.line(), "Thermal Serious");
    }

    #[test]
    fn copy_message_writes_the_selected_line_to_the_general_board() {
        let ctx = services::test_ctx();
        let mut a = app();
        let mut c = Console::new(1200.0, 780.0, Vec::new());
        let seq = c.buffer.push(LogEntry { process: "notes".into(), pid: 9, level: Level::Error, message: "boom".into(), ..LogEntry::default() });
        c.rebuild();
        c.selected = Some(seq);
        a.console = Some(c);
        a.menu_action(&ctx, MENU_COPY);
        let sent = ctx.take_sent();
        let [ToShell::ServiceCall { request, service, method, body }] = sent.as_slice() else { panic!("one pasteboard write") };
        assert_eq!((service.as_str(), method.as_str()), ("pasteboard", "write"));
        assert!(body.contains("notes[9] Error boom"), "the copied line: {body}");
        assert_eq!(a.copy_pending.len(), 1);
        // the shell has no pasteboard service: the reply is an error, the request leaves the queue
        assert!(!a.service_reply(&ctx, *request, &Err("no service pasteboard".to_string())));
        assert!(a.copy_pending.is_empty());
    }

    #[test]
    fn copy_without_a_selected_message_sends_nothing() {
        let ctx = services::test_ctx();
        let mut a = app();
        a.console = Some(Console::new(1200.0, 780.0, Vec::new()));
        a.menu_action(&ctx, MENU_COPY);
        assert!(ctx.take_sent().is_empty());
        assert!(a.copy_pending.is_empty());
    }

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("console-glue-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn doc(request: u32, name: &str, mode: DocMode, file: std::fs::File) -> bifrost_app::Document {
        bifrost_app::Document { request, grant: [9; 16], name: name.into(), size: 0, mode, file }
    }

    fn app_with_message() -> ConsoleApp {
        let mut a = app();
        let mut c = Console::new(1200.0, 780.0, Vec::new());
        c.buffer.push(LogEntry { process: "notes".into(), pid: 9, level: Level::Error, message: "boom".into(), ..LogEntry::default() });
        c.rebuild();
        a.console = Some(c);
        a
    }

    fn status(a: &ConsoleApp) -> &str {
        &a.console.as_ref().unwrap().status
    }

    #[test]
    fn save_messages_asks_the_save_panel_and_writes_what_the_shell_returns() {
        let ctx = services::test_ctx();
        let mut a = app_with_message();
        a.menu_action(&ctx, MENU_SAVE_MESSAGES);
        let sent = ctx.take_sent();
        let [ToShell::DocSave { request, suggested_name, filters }] = sent.as_slice() else { panic!("one save panel: {sent:?}") };
        assert!(suggested_name.starts_with("Console-") && suggested_name.ends_with(".log"), "{suggested_name}");
        assert_eq!(filters, &vec!["log".to_string(), "txt".to_string()]);

        let dir = scratch("save");
        let path = dir.join("out.log");
        let file = std::fs::OpenOptions::new().write(true).create(true).truncate(true).open(&path).unwrap();
        a.document(&ctx, DocumentEvent::SaveTo(doc(*request, "out.log", DocMode::Write, file)));
        assert_eq!(ctx.take_sent(), vec![ToShell::DocCommit { request: *request }]);
        assert!(std::fs::read_to_string(&path).unwrap().contains("notes[9] Error boom"));

        a.document(&ctx, DocumentEvent::Saved(doc(*request, "out.log", DocMode::Write, std::fs::File::open(&path).unwrap())));
        assert_eq!(status(&a), "Saved out.log");
        assert_eq!(ctx.take_sent(), vec![ToShell::DocRelease { grant: [9; 16] }], "the app keeps no grant to a file it only wrote");
        assert!(a.saving.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_messages_with_nothing_to_save_opens_no_panel() {
        let ctx = services::test_ctx();
        let mut a = app();
        a.console = Some(Console::new(1200.0, 780.0, Vec::new()));
        a.menu_action(&ctx, MENU_SAVE_MESSAGES);
        assert!(ctx.take_sent().is_empty());
        assert_eq!(status(&a), "No messages to save.");
    }

    #[test]
    fn a_cancelled_save_is_silent_and_a_shell_without_the_panel_gets_the_old_file() {
        let ctx = services::test_ctx();
        let mut a = app_with_message();
        a.menu_action(&ctx, MENU_SAVE_MESSAGES);
        let sent = ctx.take_sent();
        let [ToShell::DocSave { request, .. }] = sent.as_slice() else { panic!("one save panel") };
        a.document_failed(*request, DocErrorCode::Cancelled, Path::new("/nonexistent"));
        assert_eq!(status(&a), "");
        assert!(a.saving.is_none());

        a.menu_action(&ctx, MENU_SAVE_MESSAGES);
        let sent = ctx.take_sent();
        let [ToShell::DocSave { request, .. }] = sent.as_slice() else { panic!("one save panel") };
        let home = scratch("fallback");
        a.document_failed(*request, DocErrorCode::Unsupported, &home);
        assert!(status(&a).starts_with("Saved "), "{}", status(&a));
        let saved: Vec<_> = std::fs::read_dir(home.join(export::EXPORT_DIR)).unwrap().collect();
        assert_eq!(saved.len(), 1);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn open_asks_the_open_panel_and_shows_the_file_that_comes_back() {
        let ctx = services::test_ctx();
        let mut a = app_with_message();
        a.menu_action(&ctx, MENU_OPEN);
        let sent = ctx.take_sent();
        let [ToShell::DocOpen { request, mode, filters, .. }] = sent.as_slice() else { panic!("one open panel: {sent:?}") };
        assert_eq!(*mode, DocMode::Read);
        assert!(filters.is_empty(), "log files have no common extension");

        let dir = scratch("open");
        let path = dir.join("auth.log");
        std::fs::write(&path, "first line\nsecond line\n").unwrap();
        a.document(&ctx, DocumentEvent::Opened(doc(*request, "auth.log", DocMode::Read, std::fs::File::open(&path).unwrap())));
        let c = a.console.as_ref().unwrap();
        assert_eq!(c.view, View::Opened(0));
        assert_eq!(c.visible.len(), 2);
        assert_eq!(c.opened[0].grant, Some([9; 16]));
        assert!(!c.opened[0].writable);
        assert!(ctx.take_sent().is_empty(), "the grant stays with the opened log");
        assert!(a.open_request.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_that_is_not_a_log_is_reported_and_its_grant_released() {
        let ctx = services::test_ctx();
        let mut a = app_with_message();
        a.menu_action(&ctx, MENU_OPEN);
        let sent = ctx.take_sent();
        let [ToShell::DocOpen { request, .. }] = sent.as_slice() else { panic!("one open panel") };
        let dir = scratch("binary");
        let path = dir.join("blob");
        std::fs::write(&path, [1u8, 2, 0, 3]).unwrap();
        a.document(&ctx, DocumentEvent::Opened(doc(*request, "blob", DocMode::Read, std::fs::File::open(&path).unwrap())));
        assert_eq!(status(&a), "Could not open blob. This file is not a text log.");
        assert!(a.console.as_ref().unwrap().opened.is_empty());
        assert_eq!(ctx.take_sent(), vec![ToShell::DocRelease { grant: [9; 16] }]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_refused_open_is_reported_and_a_cancelled_one_is_not() {
        let ctx = services::test_ctx();
        let mut a = app_with_message();
        a.menu_action(&ctx, MENU_OPEN);
        let sent = ctx.take_sent();
        let [ToShell::DocOpen { request, .. }] = sent.as_slice() else { panic!("one open panel") };
        a.document(&ctx, DocumentEvent::Failed { request: *request, code: DocErrorCode::Cancelled });
        assert_eq!(status(&a), "");
        assert!(a.open_request.is_none());
        a.menu_action(&ctx, MENU_OPEN);
        let sent = ctx.take_sent();
        let [ToShell::DocOpen { request, .. }] = sent.as_slice() else { panic!("one open panel") };
        a.document(&ctx, DocumentEvent::Failed { request: *request, code: DocErrorCode::Denied });
        assert_eq!(status(&a), "Could not open the file. Access was denied.");
    }

    #[test]
    fn a_file_nobody_asked_for_is_released() {
        let ctx = services::test_ctx();
        let mut a = app_with_message();
        let dir = scratch("stray");
        let path = dir.join("x.log");
        std::fs::write(&path, "x").unwrap();
        a.document(&ctx, DocumentEvent::Opened(doc(77, "x.log", DocMode::Read, std::fs::File::open(&path).unwrap())));
        assert_eq!(ctx.take_sent(), vec![ToShell::DocRelease { grant: [9; 16] }]);
        assert!(a.console.as_ref().unwrap().opened.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_log_the_shell_hands_over_opens_in_the_sidebar() {
        let ctx = services::test_ctx();
        let mut a = app_with_message();
        let dir = scratch("inbound");
        let path = dir.join("system.log");
        std::fs::write(&path, "one line\n").unwrap();
        a.document(&ctx, DocumentEvent::Opened(doc(INBOUND | 5, "system.log", DocMode::Read, std::fs::File::open(&path).unwrap())));
        assert_eq!(a.console.as_ref().unwrap().opened.len(), 1);
        assert_eq!(a.console.as_ref().unwrap().opened[0].name, "system.log");
        assert!(ctx.take_sent().is_empty(), "the grant is kept while the log is open");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn capture_archive_asks_for_a_place_and_opens_the_captured_entries_when_saved() {
        let ctx = services::test_ctx();
        let mut a = app_with_message();
        a.menu_action(&ctx, MENU_CAPTURE);
        let sent = ctx.take_sent();
        let [ToShell::DocSave { request, suggested_name, .. }] = sent.as_slice() else { panic!("one save panel: {sent:?}") };
        assert!(suggested_name.ends_with(".logarchive"), "{suggested_name}");
        assert_eq!(a.saving, Some((*request, Saving::Archive)));
        // a second capture while one is waiting for its place is refused
        a.menu_action(&ctx, MENU_CAPTURE);
        assert!(ctx.take_sent().is_empty());
        assert_eq!(status(&a), "An archive is already being captured.");

        // the capture's entries are ready and the shell confirms the saved file
        a.captured = Some(vec![LogEntry { process: "sshd".into(), message: "accepted".into(), ..LogEntry::default() }]);
        let dir = scratch("archive");
        let path = dir.join("boot.logarchive");
        std::fs::write(&path, "").unwrap();
        a.document(&ctx, DocumentEvent::Saved(doc(*request, "boot.logarchive", DocMode::Write, std::fs::File::open(&path).unwrap())));
        let c = a.console.as_ref().unwrap();
        assert_eq!(c.view, View::Opened(0));
        assert_eq!(c.opened[0].name, "boot.logarchive");
        assert!(c.opened[0].writable, "a file the app saved can be moved to the Trash");
        assert_eq!(c.visible.len(), 1);
        assert!(a.saving.is_none() && a.captured.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn app_with_opened(writable: bool) -> ConsoleApp {
        let mut a = app_with_message();
        let log = OpenedLog { name: "boot.logarchive".into(), entries: vec![LogEntry { message: "x".into(), ..LogEntry::default() }], grant: Some([3; 16]), writable };
        a.console.as_mut().unwrap().add_opened(log);
        a
    }

    #[test]
    fn reveal_and_trash_ask_the_workspace_about_the_log_in_view() {
        use bifrost_app::proto::WorkspaceOp;
        let ctx = services::test_ctx();
        let mut a = app_with_opened(true);
        a.menu_action(&ctx, MENU_REVEAL);
        let sent = ctx.take_sent();
        let [ToShell::WorkspaceRequest { op, grants, .. }] = sent.as_slice() else { panic!("one workspace request: {sent:?}") };
        assert_eq!((*op, grants.as_slice()), (WorkspaceOp::Reveal, &[[3u8; 16]][..]));
        a.menu_action(&ctx, MENU_TRASH);
        let sent = ctx.take_sent();
        let [ToShell::WorkspaceRequest { op, grants, .. }] = sent.as_slice() else { panic!("one workspace request") };
        assert_eq!((*op, grants.as_slice()), (WorkspaceOp::Recycle, &[[3u8; 16]][..]));
    }

    #[test]
    fn trash_needs_a_writable_grant_and_neither_works_without_an_opened_log() {
        let ctx = services::test_ctx();
        let mut read_only = app_with_opened(false);
        read_only.menu_action(&ctx, MENU_TRASH);
        assert!(ctx.take_sent().is_empty());
        read_only.menu_action(&ctx, MENU_REVEAL);
        assert_eq!(ctx.take_sent().len(), 1, "a read grant is enough to reveal");
        let mut live = app_with_message();
        live.menu_action(&ctx, MENU_REVEAL);
        live.menu_action(&ctx, MENU_TRASH);
        assert!(ctx.take_sent().is_empty());
    }

    #[test]
    fn a_trashed_log_leaves_the_sidebar_and_its_grant_is_released() {
        let ctx = services::test_ctx();
        let mut a = app_with_opened(true);
        // the live journal opens when the log in view is gone: do not follow it from a test
        a.console.as_mut().unwrap().streaming = false;
        a.file_notes.borrow_mut().push(FileNote::Trashed { grant: [3; 16], ok: true });
        assert!(a.apply_file_notes(&ctx));
        assert!(a.console.as_ref().unwrap().opened.is_empty());
        assert_eq!(ctx.take_sent(), vec![ToShell::DocRelease { grant: [3; 16] }]);
        assert_eq!(status(&a), "Moved to the Trash.");
        assert!(!a.apply_file_notes(&ctx), "the notes were taken");
    }

    #[test]
    fn workspace_failures_reach_the_status_line() {
        let ctx = services::test_ctx();
        let mut a = app_with_opened(true);
        a.file_notes.borrow_mut().push(FileNote::Trashed { grant: [3; 16], ok: false });
        a.apply_file_notes(&ctx);
        assert_eq!(status(&a), "The file could not be moved to the Trash.");
        assert_eq!(a.console.as_ref().unwrap().opened.len(), 1, "a failed trash keeps the log");
        a.file_notes.borrow_mut().push(FileNote::Revealed(false));
        a.apply_file_notes(&ctx);
        assert_eq!(status(&a), "Files could not show the file.");
    }

    fn all_items(menus: &[AppMenu]) -> Vec<&AppMenuItem> {
        menus.iter().flat_map(|m| m.items.iter()).collect()
    }

    #[test]
    fn the_menus_merge_into_the_shells_file_and_view_and_keep_find_and_log_menus() {
        let a = app_with_opened(true);
        let menus = menus(a.console.as_ref().unwrap(), true, true);
        assert_eq!(menus.iter().map(|m| m.title.as_str()).collect::<Vec<_>>(), vec!["File", "Find", "View", "Log"]);
        let items = all_items(&menus);
        let ids: Vec<u32> = items.iter().filter(|i| !i.separator && !i.submenu).map(|i| i.id).collect();
        let mut unique = ids.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), ids.len(), "every action has its own id");
        assert!(ids.iter().all(|id| *id != 0));
        let keys: Vec<&str> = items.iter().map(|i| i.key.as_str()).filter(|k| !k.is_empty()).collect();
        let mut distinct = keys.clone();
        distinct.sort_unstable();
        distinct.dedup();
        assert_eq!(distinct.len(), keys.len(), "no key equivalent is used twice: {keys:?}");
    }

    #[test]
    fn visible_columns_lists_every_column_with_its_state() {
        let mut a = app_with_message();
        a.console.as_mut().unwrap().columns.toggle(columns::Col::Time);
        let menus = menus(a.console.as_ref().unwrap(), false, false);
        let view = &menus.iter().find(|m| m.title == "View").unwrap().items;
        let at = view.iter().position(|i| i.submenu && i.title == "Visible Columns").expect("the submenu");
        let children: Vec<&AppMenuItem> = view[at + 1..].iter().take_while(|i| i.depth == 1).collect();
        assert_eq!(children.iter().map(|i| i.title.as_str()).collect::<Vec<_>>(), vec!["Type", "Time", "Process", "PID", "Subsystem", "Category", "Message"]);
        assert_eq!(children.iter().map(|i| i.checked).collect::<Vec<_>>(), vec![true, false, true, false, true, true, true]);
        assert_eq!(children[0].id, MENU_COLUMN);
    }

    #[test]
    fn open_recent_lists_the_newest_log_first_and_the_file_items_follow_the_log_in_view() {
        let mut a = app_with_opened(true);
        let older = OpenedLog { name: "older.log".into(), entries: Vec::new(), grant: None, writable: false };
        a.console.as_mut().unwrap().add_opened(older);
        let menus = menus(a.console.as_ref().unwrap(), false, false);
        let file = &menus[0].items;
        let at = file.iter().position(|i| i.submenu && i.title == "Open Recent").unwrap();
        assert!(file[at].enabled);
        assert_eq!((file[at + 1].title.as_str(), file[at + 1].id), ("older.log", MENU_RECENT));
        assert_eq!((file[at + 2].title.as_str(), file[at + 2].id), ("boot.logarchive", MENU_RECENT + 1));
        let reveal = file.iter().find(|i| i.id == MENU_REVEAL).unwrap();
        let trash = file.iter().find(|i| i.id == MENU_TRASH).unwrap();
        assert!(!reveal.enabled && !trash.enabled);
    }
}
