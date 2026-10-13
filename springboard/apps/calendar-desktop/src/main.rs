//! Calendar as an installed BifrOSt app: a full-size Golden Gate window (unified toolbar, Mac month grid),
//! hosted through the SDK. The calendar itself is the library (`bifrost_calendar::calendar`).

use bifrost_app::kit::{Damage, Event, Surface};
use bifrost_app::controls::internet_accounts::Service;
use bifrost_app::widgets_timeline;
use bifrost_app::{App, AppMenu, AppMenuItem, Ctx, Input, KeyInput, TouchKind};
use bifrost_calendar::calendar::connect::{self, Connect, Snap};
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
                items: vec![item(Command::NewEventFromText)],
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
                ],
            },
            AppMenu {
                title: "Go".into(),
                items: vec![
                    item(Command::Today),
                    item(Command::Previous),
                    item(Command::Next),
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
        ]
    }

    fn menu_action(&mut self, ctx: &Ctx, id: u32) -> bool {
        let Some(cmd) = Command::from_id(id) else { return false };
        let t = bifrost_app::proto::sys::now_ms();
        match &mut self.cal {
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
