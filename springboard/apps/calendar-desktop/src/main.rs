//! Calendar as an installed BifrOSt app: a full-size Golden Gate window (unified toolbar, Mac month grid),
//! hosted through the SDK. The calendar itself is the library (`bifrost_calendar::calendar`).

use bifrost_app::kit::{Damage, Event, Surface};
use bifrost_app::controls::internet_accounts::Service;
use bifrost_app::widgets_timeline;
use bifrost_app::control::RpcError;
use bifrost_app::intents::{self, Entity, EntityQuery, Intent, IntentResult};
use bifrost_app::print::{PrintError, PrintOperation};
use bifrost_app::proto::{DocErrorCode, DocMode};
use bifrost_app::{App, AppMenu, AppMenuItem, Commands, Ctx, DocumentEvent, Input, KeyInput, TouchKind};
use bifrost_calendar::calendar::archive;
use bifrost_calendar::calendar::subscribe::{self, Fetcher};
use std::collections::HashMap;
use std::io::Write;
use bifrost_calendar::calendar::connect::{self, Connect, Snap};
use bifrost_calendar::calendar::prefs::{self, Setting};
use bifrost_calendar::calendar::{self, Command};
use bifrost_core::json::Json;
use springboard::shell::clock::Wall;
use springboard::shell::keyboard::Out;

struct CalendarApp {
    cal: Option<calendar::Calendar>,
    last_wall: f64,
    /// Shell services: alert requests, the Up Next timeline and the Dock badge.
    connect: Connect,
    /// Internet Accounts: the CalDAV calendars switched on for Calendars are read in the background.
    dav: bifrost_app::dav_sync::Sync,
    /// Open and save panels waiting for the user's answer, by request.
    jobs: HashMap<u32, Job>,
    /// Reads the subscribed calendars in the background.
    fetcher: Fetcher,
}

/// Menu ids of File > Unsubscribe from, one per subscription, clear of the commands and the settings.
const UNSUBSCRIBE_BASE: u32 = 300;

/// What an open or save panel is for.
enum Job {
    /// File > Import... or Import Archive...: the text of the chosen file goes to `Calendar::import_file`.
    Import,
    /// File > Export Archive...: the archive text, written to the chosen place.
    Export(Vec<u8>),
}

/// Writes a phone number the editor asked to copy to the pasteboard.
fn share_copy(c: &mut calendar::Calendar, ctx: &Ctx) {
    if let Some(text) = c.take_copied() {
        ctx.set_pasteboard_string(&text);
    }
}

/// File > Subscribe to Calendar from Clipboard, and a submenu to unsubscribe from each calendar that is subscribed to.
fn subscription_items(c: &calendar::Calendar, entry: &AppMenuItem) -> Vec<AppMenuItem> {
    let mut items = vec![AppMenuItem::separator(), entry.clone()];
    let names = c.subscription_names();
    if !names.is_empty() {
        items.push(AppMenuItem::submenu("Unsubscribe from"));
        items.extend(names.iter().enumerate().map(|(i, name)| AppMenuItem::new(UNSUBSCRIBE_BASE + i as u32, name).enabled(c.settings_enabled()).depth(1)));
    }
    items
}

/// The Settings menu: the week start, then a submenu of radio choices for each default.
fn settings_items(c: &calendar::Calendar) -> Vec<AppMenuItem> {
    let choice = |s: Setting| AppMenuItem::new(s.id(), &s.title()).checked(c.setting_checked(s)).enabled(c.settings_enabled()).depth(1);
    let mut items = vec![AppMenuItem::new(Setting::MondayFirst.id(), &Setting::MondayFirst.title())
        .checked(c.setting_checked(Setting::MondayFirst))
        .enabled(c.settings_enabled())];
    items.push(AppMenuItem::separator());
    items.push(AppMenuItem::submenu("Default Event Length"));
    items.extend((0..prefs::DURATIONS.len()).map(|i| choice(Setting::Length(i))));
    items.push(AppMenuItem::submenu("Day Starts at"));
    items.extend((0..prefs::DAY_STARTS.len()).map(|i| choice(Setting::DayStart(i))));
    items.push(AppMenuItem::submenu("Default Alert"));
    items.extend((0..prefs::ALERT_CHOICES).map(|v| choice(Setting::Alert(v))));
    items.push(AppMenuItem::submenu("Default All-Day Alert"));
    items.extend((0..prefs::ALERT_CHOICES).map(|v| choice(Setting::AllDayAlert(v))));
    items
}

impl App for CalendarApp {
    fn id(&self) -> &str {
        "is.olibuijr.calendar.desktop"
    }

    fn configure(&mut self, ctx: &Ctx) {
        let (w, h) = (ctx.cfg.w as i32, ctx.cfg.h as i32);
        match &mut self.cal {
            Some(c) => c.set_size(w, h),
            None => {
                // Every change is written when it is saved. The one thing held in memory is an event draft that is
                // still open, and that is discarded on quit, so the shell may end the app at once.
                ctx.enable_sudden_termination();
                let _ = std::fs::create_dir_all(&ctx.data_dir);
                let mut c = calendar::Calendar::new(&ctx.ui, w, h, ctx.data_dir.join("events.tsv"), Wall::sample(0.0));
                c.enable_file_export();
                self.cal = Some(c);
            }
        }
    }

    fn controls(&self, ctx: &Ctx) -> Vec<bifrost_app::controls::Node> {
        self.cal.as_ref().map_or_else(Vec::new, |c| c.controls(&ctx.ui))
    }

    fn main_menu(&self, _ctx: &Ctx) -> Vec<AppMenu> {
        let Some(c) = &self.cal else { return Vec::new() };
        // "Event" and "Go" are the app's own menus. "File" holds only New Event from Text: the shell's File menu takes
        // the app's File items, so Edit, View and Window are not repeated.
        let item = |cmd: Command| {
            let base = AppMenuItem::new(cmd.id(), c.command_title(cmd)).enabled(c.enabled(cmd)).checked(c.checked(cmd));
            match cmd.key() {
                "" => base,
                key => base.key(key),
            }
        };
        vec![
            AppMenu {
                title: "File".into(),
                items: vec![
                    item(Command::NewEventFromText),
                    AppMenuItem::separator(),
                    item(Command::Import),
                    item(Command::ImportArchive),
                    item(Command::ExportArchive),
                    AppMenuItem::separator(),
                    item(Command::Print),
                ]
                .into_iter()
                .chain(subscription_items(c, &item(Command::Subscribe)))
                .collect(),
            },
            AppMenu {
                title: "Event".into(),
                items: vec![
                    item(Command::NewEvent),
                    item(Command::Save),
                    item(Command::Duplicate),
                    item(Command::ExportEvent),
                    item(Command::Delete),
                    AppMenuItem::separator(),
                    item(Command::ExportCalendar),
                    AppMenuItem::separator(),
                    item(Command::Search),
                ],
            },
            AppMenu {
                title: "Go".into(),
                items: vec![
                    item(Command::Today),
                    item(Command::Previous),
                    item(Command::Next),
                    item(Command::GoToDate),
                    AppMenuItem::separator(),
                    item(Command::ShowDay),
                    item(Command::ShowWeek),
                    item(Command::ShowMonth),
                    item(Command::ShowYear),
                    AppMenuItem::separator(),
                    item(Command::WeekNumbers),
                    item(Command::CalendarList),
                ],
            },
            // merges into the shell's View menu
            AppMenu {
                title: "View".into(),
                items: vec![item(Command::ZoomIn), item(Command::ZoomOut)],
            },
            AppMenu { title: "Settings".into(), items: settings_items(c) },
        ]
    }

    /// The Edit menu: Cut, Copy and Paste act on events, or on the focused field of the event editor.
    fn commands(&self, ctx: &Ctx) -> Commands {
        self.cal.as_ref().map_or_else(Commands::default, |c| c.edit_commands(ctx.pasteboard_string().as_deref()))
    }

    fn menu_action(&mut self, ctx: &Ctx, id: u32) -> bool {
        if let Some(setting) = Setting::from_id(id) {
            return self.cal.as_mut().is_some_and(|c| c.change_setting(&ctx.ui, setting));
        }
        if let Some(i) = id.checked_sub(UNSUBSCRIBE_BASE).filter(|i| (*i as usize) < subscribe::MAX_SUBSCRIPTIONS) {
            return self.cal.as_mut().is_some_and(|c| c.unsubscribe(i as usize));
        }
        let Some(cmd) = Command::from_id(id) else { return false };
        let t = bifrost_app::proto::sys::now_ms();
        match &mut self.cal {
            // the address is the pasteboard's text, read when the command runs
            Some(c) if cmd == Command::Subscribe => {
                let text = ctx.pasteboard_string().unwrap_or_default();
                match c.subscribe(&text) {
                    Ok(name) => c.show_notice(&format!("Subscribed to {name}")),
                    Err(e) => c.show_error(&e),
                }
                true
            }
            Some(c) if matches!(cmd, Command::Import | Command::ImportArchive) => {
                if !c.enabled(cmd) {
                    return false;
                }
                let (title, filters): (&str, &[&str]) = if cmd == Command::Import {
                    ("Import Calendar", &["ics", "vcs", archive::EXTENSION])
                } else {
                    ("Import Calendar Archive", &[archive::EXTENSION])
                };
                let request = ctx.open_document(DocMode::Read, title, filters);
                self.jobs.insert(request, Job::Import);
                false
            }
            Some(c) if cmd == Command::ExportArchive => {
                if !c.enabled(cmd) {
                    return false;
                }
                let request = ctx.save_document(&c.archive_name(), &[archive::EXTENSION]);
                self.jobs.insert(request, Job::Export(c.archive_text().into_bytes()));
                false
            }
            Some(c) if cmd == Command::Print => {
                if !c.enabled(cmd) {
                    return false;
                }
                let (job, text) = c.print_job();
                match PrintOperation::text(job, text).run() {
                    Ok(_) => c.show_notice("Sent to the printer"),
                    Err(PrintError::NoCups) => c.show_error("The printing service is not running"),
                    Err(PrintError::NoPrinter) => c.show_error("No printer is set up"),
                    Err(PrintError::Failed(why)) => c.show_error(&format!("Printing failed: {why}")),
                }
                true
            }
            // the sentence is the pasteboard's text, read when the command runs
            Some(c) if cmd == Command::NewEventFromText => {
                let text = ctx.pasteboard_string().unwrap_or_default();
                c.enabled(cmd) && c.new_event_from_text(&ctx.ui, &text, t)
            }
            Some(c) => c.perform(&ctx.ui, cmd, t),
            None => false,
        }
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        // a calendar file opened from Files, or dropped on the app: its events are imported
        if let Input::Open(url) = &ev {
            // a webcal link subscribes to the calendar it names
            if subscribe::is_calendar_link(url) {
                let Some(c) = &mut self.cal else { return false };
                match c.subscribe(url) {
                    Ok(name) => c.show_notice(&format!("Subscribed to {name}")),
                    Err(e) => c.show_error(&e),
                }
                return true;
            }
            let (Some(c), Some(path)) = (&mut self.cal, archive::path_from_open(url)) else { return false };
            match std::fs::File::open(&path).map_err(|e| e.to_string()).and_then(archive::read_text) {
                Ok(text) => {
                    c.import_file(&text);
                }
                Err(e) => c.show_error(&format!("Could not open the file: {e}")),
            }
            return true;
        }
        // Edit menu commands: events are copied as .ics text and pasted from the pasteboard
        if let Input::Command(cmd) = &ev {
            let Some(c) = &mut self.cal else { return false };
            let text = ctx.pasteboard_string();
            let done = c.edit_command(&ctx.ui, *cmd, text.as_deref(), bifrost_app::proto::sys::now_ms());
            share_copy(c, ctx);
            return done;
        }
        // a published toolbar control (accessibility, `bifrost pc midget tap-window`): a click at its centre
        if let (Some(c), Input::Activate(id)) = (&mut self.cal, &ev) {
            let Some(node) = c.controls(&ctx.ui).into_iter().find(|n| n.id == *id && n.enabled) else { return false };
            let (x, y) = (ctx.ui.pt(node.bounds.x + node.bounds.w / 2.0), ctx.ui.pt(node.bounds.y + node.bounds.h / 2.0));
            let t = bifrost_app::proto::sys::now_ms();
            c.touch(&ctx.ui, Event::Down(x, y), t);
            c.touch(&ctx.ui, Event::Up(x, y), t);
            share_copy(c, ctx);
            return true;
        }
        if let Input::Key(key) = &ev {
            if let Some((code, mods)) = key.as_chord() {
                let now = bifrost_app::proto::sys::now_ms();
                return match &mut self.cal {
                    Some(app) => app.chord(&ctx.ui, code, mods, now),
                    None => false,
                };
            }
            if *key == KeyInput::Code(1) {
                let now = bifrost_app::proto::sys::now_ms();
                return match &mut self.cal {
                    Some(app) => app.escape(&ctx.ui, now),
                    None => false,
                };
            }
            let out = match key {
                KeyInput::Text(text) => Out::Text(text.clone()),
                KeyInput::Code(8) => Out::Backspace,
                KeyInput::Code(13) => Out::Enter,
                KeyInput::Code(9) => Out::Named("Tab"),
                KeyInput::Code(37) => Out::Named("Left"),
                KeyInput::Code(38) => Out::Named("Up"),
                KeyInput::Code(39) => Out::Named("Right"),
                KeyInput::Code(40) => Out::Named("Down"),
                _ => return false,
            };
            if let Some(app) = &mut self.cal { app.key(&ctx.ui, out, bifrost_app::proto::sys::now_ms()); return true; }
            return false;
        }
        let (Some(c), Input::Touch { kind, x, y, t, .. }) = (&mut self.cal, ev) else { return false };
        let e = match kind {
            TouchKind::Down => Event::Down(x, y),
            TouchKind::Move => Event::Move(x, y),
            TouchKind::Up | TouchKind::Cancel => Event::Up(x, y),
        };
        c.touch(&ctx.ui, e, t);
        share_copy(c, ctx);
        true
    }

    fn tick(&mut self, ctx: &Ctx, t: f64) -> bool {
        let Some(c) = &mut self.cal else { return false };
        if t - self.last_wall > 30_000.0 {
            self.last_wall = t;
            c.set_wall(Wall::sample(t));
        }
        // Services run on real changes only: a save, delete or import, or the local day rolling over.
        let wall = c.wall();
        let now = wall.now(t);
        let day = connect::local_day(now, wall.offset);
        let gen = c.store_generation();
        if self.connect.is_stale(gen, day) {
            let rich = c.rich_events();
            self.connect.sync(ctx, &Snap { events: c.events(), rich: &rich, now, offset: wall.offset }, gen, day);
        }
        // Without the alert service (an older shell), the app posts its own alerts as before.
        if self.connect.fallback() {
            for alert in c.take_alerts(t) {
                ctx.notify(&alert.title, &alert.body);
            }
        }
        // Internet Accounts: a finished CalDAV sync imports its calendar objects into the store.
        if let Some(done) = self.dav.poll(ctx, t, now) {
            if let Ok(texts) = done.result {
                for text in &texts {
                    let _ = c.import_ics(text);
                }
            }
        }
        self.fetcher.poll(c, t);
        share_copy(c, ctx);
        c.tick(&ctx.ui, t)
    }

    fn intents(&self) -> Vec<Intent> {
        calendar::intents::declared()
    }

    fn perform_intent(&mut self, ctx: Option<&Ctx>, _data_dir: &std::path::Path, id: &str, params: &Json) -> Result<IntentResult, RpcError> {
        let (Some(ctx), Some(c)) = (ctx, self.cal.as_mut()) else { return Err(intents::failed("Calendar is not running yet")) };
        calendar::intents::perform(c, &ctx.ui, id, params)
    }

    fn entities(&mut self, _data_dir: &std::path::Path, ty: &str, query: &EntityQuery) -> Result<Vec<Entity>, RpcError> {
        match (&self.cal, ty) {
            (Some(c), calendar::intents::EVENT) => Ok(c.event_entities(query)),
            (None, calendar::intents::EVENT) => Ok(Vec::new()),
            _ => Err(RpcError::new(bifrost_app::control::INVALID_PARAMS, format!("unknown entity type {ty}"))),
        }
    }

    fn document(&mut self, ctx: &Ctx, ev: DocumentEvent) -> bool {
        let Some(c) = &mut self.cal else { return false };
        match ev {
            DocumentEvent::Opened(mut doc) => {
                if !matches!(self.jobs.remove(&doc.request), Some(Job::Import)) {
                    ctx.release_document(doc.grant);
                    return false;
                }
                let text = archive::read_text(&mut doc.file);
                // one import is one use of the file: the app keeps no grant to it
                ctx.release_document(doc.grant);
                match text {
                    Ok(text) => {
                        c.import_file(&text);
                    }
                    Err(e) => c.show_error(&format!("Could not read {}: {e}", doc.name)),
                }
                true
            }
            DocumentEvent::SaveTo(mut doc) => match self.jobs.remove(&doc.request) {
                Some(Job::Export(bytes)) => {
                    match doc.file.write_all(&bytes) {
                        Ok(()) => ctx.commit_document(doc.request),
                        Err(e) => {
                            ctx.log(&format!("export archive: {e}"));
                            ctx.abort_document(doc.request);
                            c.show_error(&format!("Could not export: {e}"));
                        }
                    }
                    true
                }
                other => {
                    if let Some(job) = other {
                        self.jobs.insert(doc.request, job);
                    }
                    false
                }
            },
            DocumentEvent::Saved(doc) => {
                ctx.release_document(doc.grant);
                c.show_notice(&format!("Exported {}", doc.name));
                true
            }
            DocumentEvent::Failed { request, code } => {
                self.jobs.remove(&request);
                if code != DocErrorCode::Cancelled {
                    ctx.log(&format!("document: {code:?}"));
                    c.show_error("The file could not be opened or saved");
                    return true;
                }
                false
            }
            DocumentEvent::OpenedMany { .. } => false,
        }
    }

    fn service_reply(&mut self, _ctx: &Ctx, request: u32, result: &Result<Json, String>) -> bool {
        if self.dav.reply(request, result) {
            return true;
        }
        self.connect.on_reply(result);
        true
    }

    fn service_event(&mut self, ctx: &Ctx, service: &str, event: &str, body: &Json) -> bool {
        let Some(widgets_timeline::Event::Reload { kind }) = widgets_timeline::parse_event(service, event, body) else {
            return false;
        };
        if kind != connect::UP_NEXT {
            return false;
        }
        let Some(c) = &self.cal else { return false };
        let wall = Wall::sample(0.0);
        let rich = c.rich_events();
        let snap = Snap { events: c.events(), rich: &rich, now: wall.unix, offset: wall.offset };
        self.connect.refresh_up_next(ctx, &snap);
        false
    }

    fn next_wake(&self, t: f64) -> Option<f64> {
        let c = self.cal.as_ref()?;
        match (c.next_wake(t), self.fetcher.wake(c, t)) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    fn draw(&mut self, ctx: &Ctx, s: &mut Surface, t: f64) -> Damage {
        if let Some(c) = &mut self.cal {
            c.draw(s, &ctx.ui, t);
        }
        let mut d = Damage::new(s.w, s.h);
        d.all();
        d
    }
}

fn main() {
    bifrost_app::run(CalendarApp { cal: None, last_wall: 0.0, connect: Connect::default(), dav: bifrost_app::dav_sync::Sync::new(Service::Calendars), jobs: HashMap::new(), fetcher: Fetcher::default() });
}
