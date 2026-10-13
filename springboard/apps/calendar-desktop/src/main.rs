//! Calendar as an installed BifrOSt app: a full-size Golden Gate window (unified toolbar, Mac month grid),
//! hosted through the SDK. The calendar itself is the library (`bifrost_calendar::calendar`).

use bifrost_app::kit::{Damage, Event, Surface};
use bifrost_app::controls::internet_accounts::Service;
use bifrost_app::widgets_timeline;
use bifrost_app::proto::DocMode;
use bifrost_app::{App, AppMenu, AppMenuItem, Command as EditCommand, Commands, Ctx, DocumentEvent, Input, KeyInput, OpenOptions, TouchKind};
use bifrost_app::control::{RpcError, INVALID_PARAMS};
use bifrost_app::intents::{self, Intent, IntentResult, Param, ParamType};
use bifrost_calendar::calendar::{archive, intent as cal_intent, prefs};
use bifrost_calendar::calendar::connect::{self, Connect, Snap};
use bifrost_calendar::calendar::{self, Command};
use bifrost_core::json::Json;
use springboard::shell::clock::Wall;
use springboard::shell::keyboard::Out;
use std::io::Read;

/// The largest calendar file File > Import reads.
const MAX_IMPORT: u64 = 8 * 1024 * 1024;

struct CalendarApp {
    cal: Option<calendar::Calendar>,
    last_wall: f64,
    /// Shell services: alert requests, the Up Next timeline and the Dock badge.
    connect: Connect,
    /// Internet Accounts: the CalDAV calendars switched on for Calendars are read in the background.
    dav: bifrost_app::dav_sync::Sync,
}

/// Hands a calendar file's bytes to the calendar: an archive restores the whole calendar, anything else is imported.
fn load_document(c: &mut calendar::Calendar, name: &str, bytes: &[u8]) {
    let text = String::from_utf8_lossy(bytes);
    if name.ends_with(&format!(".{}", archive::EXTENSION)) {
        c.restore_archive(name, &text);
    } else {
        c.import_report(name, &text);
    }
}

/// The largest file the calendar reads for `name`.
fn read_limit(name: &str) -> u64 {
    if name.ends_with(&format!(".{}", archive::EXTENSION)) {
        archive::MAX_BYTES
    } else {
        MAX_IMPORT
    }
}

/// A calendar file opened from Files (or another app): `url` is a path or a `file://` URL.
fn open_path(c: &mut calendar::Calendar, url: &str) -> bool {
    let path = std::path::Path::new(url.strip_prefix("file://").unwrap_or(url));
    let name = path.file_name().map_or_else(|| url.to_string(), |n| n.to_string_lossy().into_owned());
    let mut bytes = Vec::new();
    match std::fs::File::open(path).and_then(|f| f.take(read_limit(&name)).read_to_end(&mut bytes)) {
        Ok(_) => load_document(c, &name, &bytes),
        Err(e) => c.show_error(format!("Could not read {name}: {e}")),
    }
    true
}

/// The event an intent or control call creates: `title`, `date` and an optional `time` (`HH:MM`; none makes it all-day).
fn create_from(data_dir: &std::path::Path, params: &Json) -> Result<calendar_event::Created, RpcError> {
    let invalid = |m: &str| RpcError::new(INVALID_PARAMS, m.to_string());
    let title = intents::str_param(params, "title")?;
    let date = cal_intent::parse_date(intents::str_param(params, "date")?).ok_or_else(|| invalid("date must be YYYY-MM-DD"))?;
    let minute = match params.get("time").and_then(Json::as_str).filter(|t| !t.is_empty()) {
        Some(t) => Some(cal_intent::parse_time(t).ok_or_else(|| invalid("time must be HH:MM"))?),
        None => None,
    };
    let events = data_dir.join("events.tsv");
    let duration = prefs::load(&events).duration;
    let event = cal_intent::create_event(&events, title, date, minute, duration).map_err(intents::failed)?;
    Ok(calendar_event::Created { id: event.id, title: event.title, date })
}

mod calendar_event {
    /// What `create_from` made.
    pub struct Created {
        pub id: u64,
        pub title: String,
        pub date: bifrost_calendar::calendar::Date,
    }
}

/// Writes a phone number the editor asked to copy to the pasteboard.
fn share_copy(c: &mut calendar::Calendar, ctx: &Ctx) {
    if let Some(text) = c.take_copied() {
        ctx.set_pasteboard_string(&text);
    }
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
                    item(Command::ImportCalendar),
                    item(Command::ImportArchive),
                    item(Command::ExportArchive),
                    AppMenuItem::separator(),
                    item(Command::Settings),
                ],
            },
            AppMenu {
                title: "Event".into(),
                items: vec![
                    item(Command::NewEvent),
                    item(Command::Save),
                    item(Command::Duplicate),
                    item(Command::CutEvent),
                    item(Command::CopyEvent),
                    item(Command::PasteEvent),
                    item(Command::ExportEvent),
                    item(Command::Delete),
                    AppMenuItem::separator(),
                    item(Command::ExportCalendar),
                ],
            },
            AppMenu {
                title: "Go".into(),
                items: vec![
                    item(Command::Today),
                    item(Command::Previous),
                    item(Command::Next),
                    item(Command::GoToDate),
                    item(Command::Find),
                    AppMenuItem::separator(),
                    item(Command::ShowDay),
                    item(Command::ShowWeek),
                    item(Command::ShowMonth),
                    item(Command::ShowYear),
                    AppMenuItem::separator(),
                    item(Command::ZoomIn),
                    item(Command::ZoomOut),
                    AppMenuItem::separator(),
                    item(Command::WeekNumbers),
                    item(Command::CalendarList),
                ],
            },
        ]
    }

    fn menu_action(&mut self, ctx: &Ctx, id: u32) -> bool {
        let Some(cmd) = Command::from_id(id) else { return false };
        let t = bifrost_app::proto::sys::now_ms();
        let done = match &mut self.cal {
            // the sentence is the pasteboard's text, read when the command runs
            Some(c) if cmd == Command::NewEventFromText => {
                let text = ctx.pasteboard_string().unwrap_or_default();
                c.enabled(cmd) && c.new_event_from_text(&ctx.ui, &text, t)
            }
            // the pasteboard holds an event copied with Copy Event (or any calendar text)
            Some(c) if cmd == Command::PasteEvent => {
                if !c.enabled(cmd) {
                    return false;
                }
                let text = ctx.pasteboard_string().unwrap_or_default();
                if let Err(e) = c.paste_event(&text) {
                    c.show_error(e);
                }
                true
            }
            Some(c) if cmd == Command::ImportCalendar => {
                if !c.enabled(cmd) {
                    return false;
                }
                let options = OpenOptions { mode: DocMode::Read, title: "Import".into(), filters: vec!["ics".into(), "vcs".into()], ..Default::default() };
                ctx.open_panel(&options);
                true
            }
            Some(c) if cmd == Command::ImportArchive => {
                if !c.enabled(cmd) {
                    return false;
                }
                let options = OpenOptions { mode: DocMode::Read, title: "Import Calendar Archive".into(), filters: vec![archive::EXTENSION.into()], ..Default::default() };
                ctx.open_panel(&options);
                true
            }
            Some(c) => c.perform(&ctx.ui, cmd, t),
            None => false,
        };
        if let Some(c) = &mut self.cal {
            share_copy(c, ctx);
        }
        done
    }

    /// The file chosen in File > Import... (added to the calendar) or Import Calendar Archive... (restores it).
    fn document(&mut self, _ctx: &Ctx, ev: DocumentEvent) -> bool {
        let docs = match ev {
            DocumentEvent::Opened(doc) => vec![doc],
            DocumentEvent::OpenedMany { docs, .. } => docs,
            _ => return false,
        };
        let Some(c) = &mut self.cal else { return false };
        for doc in docs {
            let mut bytes = Vec::new();
            match (&doc.file).take(read_limit(&doc.name)).read_to_end(&mut bytes) {
                Ok(_) => load_document(c, &doc.name, &bytes),
                Err(e) => c.show_error(format!("Could not read {}: {e}", doc.name)),
            }
        }
        true
    }

    /// Edit > Cut, Copy and Paste act on events: the saved event being edited, and the pasteboard's event.
    fn commands(&self, _ctx: &Ctx) -> Commands {
        let Some(c) = &self.cal else { return Commands::default() };
        Commands::default()
            .with(EditCommand::Cut, c.can_copy_event())
            .with(EditCommand::Copy, c.can_copy_event())
            .with(EditCommand::Paste, c.can_paste_event())
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        if let (Some(c), Input::Open(url)) = (&mut self.cal, &ev) {
            return open_path(c, url);
        }
        if let Input::Command(command) = &ev {
            let Some(c) = &mut self.cal else { return false };
            let t = bifrost_app::proto::sys::now_ms();
            let done = match command {
                EditCommand::Cut => c.perform(&ctx.ui, Command::CutEvent, t),
                EditCommand::Copy => c.perform(&ctx.ui, Command::CopyEvent, t),
                EditCommand::Paste if c.can_paste_event() => {
                    let text = ctx.pasteboard_string().unwrap_or_default();
                    if let Err(e) = c.paste_event(&text) {
                        c.show_error(e);
                    }
                    true
                }
                _ => false,
            };
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
        share_copy(c, ctx);
        c.tick(&ctx.ui, t)
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
        self.cal.as_ref().and_then(|c| c.next_wake(t))
    }

    fn intents(&self) -> Vec<Intent> {
        vec![
            Intent::new("create-event", "Create Event", "Adds an event to the calendar.")
                .param(Param::new("title", "Title", ParamType::String))
                .param(Param::new("date", "Date", ParamType::Date))
                .param(Param::new("time", "Start time (HH:MM, empty for all-day)", ParamType::String).optional()),
            Intent::new("show-date", "Show Date", "Shows a day in the calendar.").foreground().param(Param::new("date", "Date", ParamType::Date)),
        ]
    }

    fn perform_intent(&mut self, _ctx: Option<&Ctx>, data_dir: &std::path::Path, id: &str, params: &Json) -> Result<IntentResult, RpcError> {
        match id {
            "create-event" => {
                let made = create_from(data_dir, params)?;
                if let Some(c) = &mut self.cal {
                    c.reload();
                }
                Ok(IntentResult::value(Json::Num(made.id as f64)).with_dialog(&format!("Added {} on {}", made.title, made.date.iso())))
            }
            "show-date" => {
                let date = cal_intent::parse_date(intents::str_param(params, "date")?).ok_or_else(|| RpcError::new(INVALID_PARAMS, "date must be YYYY-MM-DD".to_string()))?;
                if let Some(c) = &mut self.cal {
                    c.go_to(date);
                }
                Ok(IntentResult::dialog(&format!("Showing {}", date.iso())).continue_in_foreground())
            }
            other => Err(RpcError::new(INVALID_PARAMS, format!("no intent {other}"))),
        }
    }

    fn rpc_methods(&self) -> Vec<&'static str> {
        vec!["calendar.state", "calendar.show_date", "calendar.create_event"]
    }

    fn rpc(&mut self, _ctx: Option<&Ctx>, data_dir: &std::path::Path, method: &str, params: &Json) -> Result<Json, RpcError> {
        match method {
            "calendar.state" => Ok(Json::Str(self.cal.as_ref().map_or_else(String::new, |c| c.describe()))),
            "calendar.show_date" => self.perform_intent(None, data_dir, "show-date", params).map(|r| r.value),
            "calendar.create_event" => self.perform_intent(None, data_dir, "create-event", params).map(|r| r.value),
            other => Err(RpcError::new(bifrost_app::control::METHOD_NOT_FOUND, format!("no method {other}"))),
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
    bifrost_app::run(CalendarApp { cal: None, last_wall: 0.0, connect: Connect::default(), dav: bifrost_app::dav_sync::Sync::new(Service::Calendars) });
}
