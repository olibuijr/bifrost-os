//! Calendar as an installed BifrOSt app: a full-size Golden Gate window (unified toolbar, Mac month grid),
//! hosted through the SDK. The calendar itself is the library (`bifrost_calendar::calendar`).

use bifrost_app::kit::{Damage, Event, Surface};
use bifrost_app::{App, AppMenu, AppMenuItem, Ctx, Input, KeyInput, TouchKind};
use bifrost_calendar::calendar::{self, Command};
use springboard::shell::clock::Wall;
use springboard::shell::keyboard::Out;

struct CalendarApp {
    cal: Option<calendar::Calendar>,
    last_wall: f64,
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
        // "Event" and "Go" are the app's own menus: File, Edit, View and Window are the shell's, so none is repeated.
        let item = |cmd: Command| {
            let base = AppMenuItem::new(cmd.id(), c.command_title(cmd)).enabled(c.enabled(cmd)).checked(c.checked(cmd));
            match cmd.key() {
                "" => base,
                key => base.key(key),
            }
        };
        vec![
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
        true
    }

    fn tick(&mut self, ctx: &Ctx, t: f64) -> bool {
        let Some(c) = &mut self.cal else { return false };
        if t - self.last_wall > 30_000.0 {
            self.last_wall = t;
            c.set_wall(Wall::sample(t));
        }
        for alert in c.take_alerts(t) {
            ctx.notify(&alert.title, &alert.body);
        }
        c.tick(&ctx.ui, t)
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
    bifrost_app::run(CalendarApp { cal: None, last_wall: 0.0 });
}
