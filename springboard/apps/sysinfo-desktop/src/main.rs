//! System Information as an installed BifrOSt system app: the hardware tree (processor, memory, graphics, PCI, USB,
//! displays, audio, Bluetooth, cameras, printers, Thunderbolt, power, storage), the network section (Wi-Fi, Ethernet,
//! WWAN, firewall, volumes) and the software section (the operating system, installed apps, extensions, startup items,
//! fonts, logs, language). The facts are read by `bifrost-core`'s `sysinfo` and by `probe.rs`; `model.rs` builds the
//! rows, `window.rs` holds the state and the geometry, `draw.rs` paints them, and `export.rs` saves a report and reads
//! one back.

mod draw;
mod export;
mod extras;
mod help;
mod model;
mod probe;
mod recents;
mod speech;
mod window;

use bifrost_app::controls::Node;
use bifrost_app::kit::{Damage, Surface};
use bifrost_app::proto::{AnnouncePriority, DocErrorCode, DocMode};
use bifrost_app::displays;
use bifrost_app::{App, AppMenu, AppMenuItem, Command, Commands, Ctx, Document, DocumentEvent, Input, KeyInput, TouchKind};
use bifrost_core::json::Json;
use export::Format;
use std::io::{Read, Write};
use window::{
    Focus, PendingExport, SysInfo, Target, COLLAPSED_KEY, MENU_CLEAR_RECENTS, MENU_COPY, MENU_FIND, MENU_FIND_NEXT, MENU_FIND_PREVIOUS, MENU_HELP, MENU_JUMP_SELECTION, MENU_LESS, MENU_NEXT_CATEGORY, MENU_OPEN,
    MENU_ONLINE_HELP, MENU_OVERVIEW, MENU_PREVIOUS_CATEGORY, MENU_RECENT, MENU_REFRESH, MENU_SAVE, MENU_SAVE_JSON, MENU_SAVE_RTF, MENU_SELECT_ALL, MENU_SPEAK, MENU_THIS_COMPUTER, MENU_USE_SELECTION, STEP_PT,
};

/// The help page the Help menu opens.
const HELP_URL: &str = "https://bifrost.olibuijr.com/docs/system-information";

impl App for SysInfo {
    fn id(&self) -> &str {
        "is.olibuijr.sysinfo.desktop"
    }

    fn commands(&self, _ctx: &Ctx) -> Commands {
        Commands::default().with(Command::Copy, self.can_copy()).with(Command::SelectAll, true)
    }

    fn configure(&mut self, ctx: &Ctx) {
        ctx.enable_sudden_termination(); // keeps no unsaved state: the facts are read again on every launch
        let scale = ctx.ui.pt(1.0);
        self.size = (ctx.cfg.w as f32 / scale, ctx.cfg.h as f32 / scale);
        if !self.started {
            if let Some(text) = ctx.defaults().string(COLLAPSED_KEY) {
                self.set_collapsed_text(&text);
            }
            self.load_recents(ctx.defaults().string(recents::KEY));
            self.start_load();
        }
        self.sync(ctx);
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        let changed = self.handle(ctx, ev);
        self.sync(ctx);
        changed
    }

    fn main_menu(&self, _ctx: &Ctx) -> Vec<AppMenu> {
        let finding = !self.query().trim().is_empty();
        let mut file = vec![AppMenuItem::new(MENU_OPEN, "Open Report...").key("cmd+o"), AppMenuItem::submenu("Open Recent")];
        for (i, r) in self.recents.iter().enumerate() {
            file.push(AppMenuItem::new(MENU_RECENT + i as u32, &r.name).depth(1));
        }
        if !self.recents.is_empty() {
            file.push(AppMenuItem::separator().depth(1));
        }
        file.push(AppMenuItem::new(MENU_CLEAR_RECENTS, "Clear Menu").enabled(!self.recents.is_empty()).depth(1));
        file.extend([
            AppMenuItem::separator(),
            AppMenuItem::new(MENU_THIS_COMPUTER, "Show This Computer").enabled(self.stored.is_some()),
            AppMenuItem::separator(),
            AppMenuItem::new(MENU_SAVE, "Save Report As Text...").key("cmd+s"),
            AppMenuItem::new(MENU_SAVE_RTF, "Save Report As Rich Text..."),
            AppMenuItem::new(MENU_SAVE_JSON, "Save Report As JSON..."),
        ]);
        vec![
            AppMenu { title: "File".into(), items: file },
            AppMenu {
                title: "Find".into(),
                items: vec![
                    AppMenuItem::new(MENU_FIND, "Find").key("cmd+f"),
                    AppMenuItem::new(MENU_FIND_NEXT, "Find Next").key("cmd+g").enabled(finding),
                    AppMenuItem::new(MENU_FIND_PREVIOUS, "Find Previous").key("cmd+shift+g").enabled(finding),
                    AppMenuItem::new(MENU_USE_SELECTION, "Use Selection for Find").key("cmd+e").enabled(self.selection_text().is_some()),
                    AppMenuItem::new(MENU_JUMP_SELECTION, "Jump to Selection").key("cmd+j").enabled(self.can_jump()),
                ],
            },
            AppMenu { title: "Tools".into(), items: vec![AppMenuItem::new(MENU_SPEAK, "Speak Serial Number")] },
            AppMenu {
                title: "View".into(),
                items: vec![
                    AppMenuItem::new(MENU_REFRESH, "Refresh Information").key("cmd+r").enabled(!self.is_loading()),
                    AppMenuItem::new(MENU_LESS, "Show Less Information").checked(self.mini),
                ],
            },
            AppMenu {
                title: "Window".into(),
                items: vec![
                    AppMenuItem::new(MENU_OVERVIEW, "Show Overview").key("cmd+1"),
                    AppMenuItem::separator(),
                    AppMenuItem::new(MENU_PREVIOUS_CATEGORY, "Previous Category").key("cmd+[").enabled(!self.mini),
                    AppMenuItem::new(MENU_NEXT_CATEGORY, "Next Category").key("cmd+]").enabled(!self.mini),
                ],
            },
            AppMenu {
                title: "Help".into(),
                items: vec![
                    AppMenuItem::new(MENU_HELP, "System Information Help").checked(self.help),
                    AppMenuItem::separator(),
                    AppMenuItem::new(MENU_ONLINE_HELP, "System Information Online Help"),
                ],
            },
        ]
    }

    fn menu_action(&mut self, ctx: &Ctx, id: u32) -> bool {
        let changed = self.menu(ctx, id);
        self.sync(ctx);
        changed
    }

    fn document(&mut self, ctx: &Ctx, ev: DocumentEvent) -> bool {
        let changed = match ev {
            DocumentEvent::SaveTo(mut doc) => {
                match self.export.take() {
                    Some(pending) if pending.request == doc.request => {
                        let format = export::format_for(&doc.name).unwrap_or(pending.format);
                        let bytes = export::render(&pending.contents, format);
                        match doc.file.write_all(&bytes) {
                            Ok(()) => ctx.commit_document(pending.request),
                            Err(e) => {
                                ctx.log(&format!("save report: {e}"));
                                ctx.abort_document(pending.request);
                            }
                        }
                    }
                    other => self.export = other,
                }
                false
            }
            DocumentEvent::Opened(doc) => self.report_opened(ctx, doc),
            DocumentEvent::Failed { request, code } => self.report_failed(ctx, request, code),
            _ => false,
        };
        self.sync(ctx);
        changed
    }

    fn resume(&mut self, ctx: &Ctx) {
        // the shell may have dropped the keyboard request while the app was away
        self.keyboard_wanted = false;
        self.sync(ctx);
    }

    fn tick(&mut self, _ctx: &Ctx, t: f64) -> bool {
        self.poll_load(t)
    }

    /// The display list the display service answered with: the refresh rate and scale of each display.
    fn service_reply(&mut self, ctx: &Ctx, request: u32, result: &Result<Json, String>) -> bool {
        if self.screens_request != Some(request) {
            return false;
        }
        self.screens_request = None;
        let changed = match displays::decode_list(result) {
            Ok(list) => {
                self.set_screens(&list);
                true
            }
            Err(e) => {
                ctx.log(&format!("displays: {e}"));
                false
            }
        };
        self.sync(ctx);
        changed
    }

    /// The display service says the displays changed: the list is asked for again.
    fn service_event(&mut self, ctx: &Ctx, service: &str, event: &str, _body: &Json) -> bool {
        if service == displays::SERVICE && event == displays::RECONFIGURED {
            self.screens_stale = true;
            self.sync(ctx);
        }
        false
    }

    fn next_wake(&self, _t: f64) -> Option<f64> {
        None // load completion is delivered on the main dispatch queue
    }

    fn controls(&self, ctx: &Ctx) -> Vec<Node> {
        draw::controls(self, &ctx.ui)
    }

    fn draw(&mut self, ctx: &Ctx, s: &mut Surface, _t: f64) -> Damage {
        draw::paint(self, s, &ctx.ui);
        let mut d = Damage::new(s.w, s.h);
        d.all();
        d
    }
}

impl SysInfo {
    /// Saves what changed on the way: the folded sections go to the preferences, and the keyboard is asked for while
    /// the find bar is open.
    fn sync(&mut self, ctx: &Ctx) {
        if self.prefs_dirty {
            ctx.defaults_mut().set_string(COLLAPSED_KEY, &self.collapsed_text());
            self.prefs_dirty = false;
        }
        let wanted = self.find.is_some();
        if wanted != self.keyboard_wanted {
            ctx.wants_keyboard(wanted);
            self.keyboard_wanted = wanted;
        }
        if self.screens_stale && self.screens_request.is_none() {
            self.screens_request = Some(displays::list(ctx, displays::Kind::Active));
            self.screens_stale = false;
        }
    }

    fn save_recents(&self, ctx: &Ctx) {
        ctx.defaults_mut().set_string(recents::KEY, &self.recents_text());
    }

    /// A touch, key or published control.
    fn handle(&mut self, ctx: &Ctx, ev: Input) -> bool {
        match ev {
            Input::Command(Command::Copy) => self.menu(ctx, MENU_COPY),
            Input::Command(Command::SelectAll) => self.select_all(),
            Input::Touch { kind, x, y, .. } => {
                let scale = ctx.ui.pt(1.0);
                let (x, y) = (x / scale, y / scale);
                match kind {
                    TouchKind::Down => {
                        self.press = self.hit(&ctx.ui, x, y);
                        self.drag_y = Some(y);
                        self.dragged = false;
                        false
                    }
                    TouchKind::Move => {
                        // a drag scrolls the pane it started in; the press then is not a tap
                        let Some(last) = self.drag_y else { return false };
                        if matches!(self.press, Target::Nothing | Target::Toolbar(_) | Target::Search | Target::SearchClear | Target::Cancel) {
                            return false;
                        }
                        self.drag_y = Some(y);
                        if (y - last).abs() < 0.5 {
                            return false;
                        }
                        self.dragged = true;
                        let press = self.press;
                        self.drag_scroll(press, last - y)
                    }
                    TouchKind::Up => {
                        let released = self.hit(&ctx.ui, x, y);
                        let pressed = std::mem::replace(&mut self.press, Target::Nothing);
                        let was_drag = std::mem::replace(&mut self.dragged, false);
                        self.drag_y = None;
                        if !was_drag && released == pressed && released != Target::Nothing {
                            self.activate(released)
                        } else {
                            false
                        }
                    }
                    TouchKind::Cancel => {
                        self.press = Target::Nothing;
                        self.drag_y = None;
                        self.dragged = false;
                        false
                    }
                }
            }
            // a published control (accessibility, `bifrost pc midget tap-window`): the same action as a click
            Input::Activate(id) => self.activate(SysInfo::target_of_id(id)),
            Input::Key(key) => self.key(key),
            _ => false,
        }
    }

    /// A key press. While the machine is read, Escape cancels. While the find bar is open it takes the text; otherwise
    /// the arrows move the sidebar selection, the item list or the card, whichever has focus, and Tab changes the focus.
    fn key(&mut self, key: KeyInput) -> bool {
        if self.is_loading() {
            return matches!(key, KeyInput::Code(1 | 27)) && self.cancel_load();
        }
        if self.find.is_some() {
            return match key {
                KeyInput::Text(text) => self.find_type(&text),
                KeyInput::Code(8) => self.find_backspace(),
                KeyInput::Code(13) => self.find_step(true),
                KeyInput::Code(1 | 27) => self.end_find(),
                _ => false,
            };
        }
        match key {
            KeyInput::Code(9) => self.toggle_focus(),
            KeyInput::Code(38) => match self.focus {
                Focus::Sidebar => self.step_category(-1),
                Focus::List => self.step_item(-1),
                Focus::Detail => self.scroll_by(-STEP_PT),
            },
            KeyInput::Code(40) => match self.focus {
                Focus::Sidebar => self.step_category(1),
                Focus::List => self.step_item(1),
                Focus::Detail => self.scroll_by(STEP_PT),
            },
            _ => false,
        }
    }

    /// A menu item chosen from the bar or by its key.
    fn menu(&mut self, ctx: &Ctx, id: u32) -> bool {
        match id {
            MENU_SAVE => self.begin_save(ctx, Format::Text),
            MENU_SAVE_RTF => self.begin_save(ctx, Format::Rtf),
            MENU_SAVE_JSON => self.begin_save(ctx, Format::Json),
            MENU_OPEN => {
                let request = ctx.open_document(DocMode::Read, "Open Report", &[Format::Json.filter()]);
                self.pending_open.push((request, None));
                false
            }
            MENU_CLEAR_RECENTS => {
                for r in std::mem::take(&mut self.recents) {
                    ctx.release_document(r.grant);
                }
                self.save_recents(ctx);
                true
            }
            i if (MENU_RECENT..MENU_RECENT + recents::MAX as u32).contains(&i) => {
                if let Some(r) = self.recents.get((i - MENU_RECENT) as usize).cloned() {
                    let request = ctx.reopen_document(r.grant);
                    self.pending_open.push((request, Some(r.grant)));
                }
                false
            }
            MENU_THIS_COMPUTER => self.show_live(),
            MENU_REFRESH => self.refresh(),
            MENU_LESS => self.toggle_mini(),
            MENU_COPY => {
                if let Some(text) = self.copy_text() {
                    ctx.set_pasteboard_string(&text);
                }
                false
            }
            MENU_SELECT_ALL => self.select_all(),
            MENU_FIND => self.begin_find(),
            MENU_FIND_NEXT => self.find_step(true),
            MENU_FIND_PREVIOUS => self.find_step(false),
            MENU_USE_SELECTION => self.use_selection_for_find(),
            MENU_JUMP_SELECTION => self.jump_to_selection(),
            MENU_SPEAK => {
                let Some(text) = self.serial_speech() else {
                    ctx.announce("No serial number is available", AnnouncePriority::High);
                    return false;
                };
                // the engine speaks whether or not the screen reader is on; with no engine the reader's banner carries it
                if self.speaker.start(&text) == Err(speech::SpeakError::NoEngine) {
                    ctx.announce(&text, AnnouncePriority::High);
                }
                false
            }
            MENU_OVERVIEW => self.select(model::Category::Overview),
            MENU_PREVIOUS_CATEGORY => self.step_category(-1),
            MENU_NEXT_CATEGORY => self.step_category(1),
            MENU_HELP => self.toggle_help(),
            MENU_ONLINE_HELP => {
                ctx.open_url(HELP_URL);
                false
            }
            _ => false,
        }
    }

    /// Shows the save panel; the report is written when its file arrives.
    fn begin_save(&mut self, ctx: &Ctx, format: Format) -> bool {
        let request = ctx.save_document(&format.suggested_name(), &[format.filter()]);
        self.export = Some(PendingExport { request, format, contents: self.export_contents() });
        false
    }

    /// The open panel or a recent report gave a file. It becomes the report shown when it reads as one; otherwise the
    /// line under the title says why not. True when the window changed.
    fn report_opened(&mut self, ctx: &Ctx, doc: Document) -> bool {
        let Some(at) = self.pending_open.iter().position(|(request, _)| *request == doc.request) else { return false };
        let (_, recent) = self.pending_open.remove(at);
        let mut bytes = Vec::new();
        let limit = export::MAX_REPORT_BYTES;
        let read = (&doc.file).take(limit + 1).read_to_end(&mut bytes);
        let result = match read {
            Err(e) => Err(format!("The report could not be read: {e}")),
            Ok(_) if bytes.len() as u64 > limit => Err("That file is too large to be a system report.".to_string()),
            Ok(_) => export::parse(&bytes, &doc.name),
        };
        match result {
            Ok(stored) => {
                self.show_stored(stored);
                for dropped in self.note_recent(doc.grant, &doc.name) {
                    ctx.release_document(dropped);
                }
                self.save_recents(ctx);
            }
            Err(message) => {
                self.notice = Some(message);
                if recent.is_none() {
                    ctx.release_document(doc.grant);
                }
            }
        }
        true
    }

    /// An open or reopen request failed (or the user cancelled the panel).
    fn report_failed(&mut self, ctx: &Ctx, request: u32, code: DocErrorCode) -> bool {
        if self.export.as_ref().is_some_and(|p| p.request == request) {
            self.export = None;
        }
        let Some(at) = self.pending_open.iter().position(|(r, _)| *r == request) else {
            if code != DocErrorCode::Cancelled {
                ctx.log(&format!("document: {code:?}"));
            }
            return false;
        };
        let (_, recent) = self.pending_open.remove(at);
        if code == DocErrorCode::Cancelled {
            return false;
        }
        if let Some(grant) = recent.filter(|_| window::forgets_recent(code)) {
            if recents::forget(&mut self.recents, &grant) {
                ctx.release_document(grant);
                self.save_recents(ctx);
            }
        }
        self.notice = Some(window::open_failure(code).to_string());
        true
    }
}

fn main() {
    bifrost_app::run(SysInfo::new());
}
