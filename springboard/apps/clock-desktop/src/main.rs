//! Clock as an installed BifrOSt app: World Clock, Alarm, Stopwatch and
//! Timer, hosted through the SDK. Alarms and timers are only edited here;
//! the shell's headless Clock service fires them and shows the alert, so
//! they keep ringing with this app closed.

#[allow(dead_code)]
mod clock;
#[allow(dead_code)]
mod icon;

use bifrost_app::kit::{Damage, Event, Surface};
use bifrost_app::{App, Ctx, Input, TouchKind, KeyInput};
use springboard::shell::keyboard::Out;
use springboard::bare::sys::wall_time;

struct ClockApp {
    clock: Option<clock::Clock>,
    last_wall: f64,
}

impl App for ClockApp {
    fn id(&self) -> &str {
        "is.olibuijr.clock.desktop"
    }

    fn configure(&mut self, ctx: &Ctx) {
        let (w, h) = (ctx.cfg.w as i32, ctx.cfg.h as i32);
        match &mut self.clock {
            Some(c) => c.set_size(w, h),
            None => {
                // Once, at startup: nothing is left unsaved (alarms, timers, laps and cities are written on every change).
                ctx.enable_sudden_termination();
                let mut c = clock::Clock::new(&ctx.ui, w, h);
                let (unix, offset) = wall_time();
                let t = bifrost_proto::sys::now_ms();
                c.set_wall(unix, offset, t);
                c.load(&ctx.data_dir.join("clock.ini").to_string_lossy(), t);
                self.clock = Some(c);
            }
        }
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        if let Input::Key(key) = &ev {
            let out = match key {
                KeyInput::Text(text) => Out::Text(text.clone()),
                KeyInput::Code(8) => Out::Backspace,
                KeyInput::Code(13) => Out::Enter,
                KeyInput::Code(9) => Out::Named("Tab"),
                KeyInput::Code(37) => Out::Named("Left"),
                KeyInput::Code(39) => Out::Named("Right"),
                _ => return false,
            };
            if let Some(app) = &mut self.clock { app.key(&ctx.ui, out, bifrost_app::proto::sys::now_ms()); return true; }
            return false;
        }
        let (Some(c), Input::Touch { kind, x, y, t, .. }) = (&mut self.clock, ev) else { return false };
        let e = match kind {
            TouchKind::Down => Event::Down(x, y),
            TouchKind::Move => Event::Move(x, y),
            TouchKind::Up | TouchKind::Cancel => Event::Up(x, y),
        };
        c.touch(&ctx.ui, e, t);
        true
    }

    fn tick(&mut self, ctx: &Ctx, t: f64) -> bool {
        let Some(c) = &mut self.clock else { return false };
        if t - self.last_wall >= 1000.0 {
            self.last_wall = t;
            let (unix, offset) = wall_time();
            c.set_wall(unix, offset, t);
        }
        c.tick(&ctx.ui, t)
    }

    fn next_wake(&self, t: f64) -> Option<f64> {
        self.clock.as_ref().and_then(|c| c.next_wake(t, true))
    }

    fn controls(&self, ctx: &Ctx) -> Vec<bifrost_app::controls::Node> {
        self.clock.as_ref().map(|c| c.controls(&ctx.ui)).unwrap_or_default()
    }

    fn draw(&mut self, ctx: &Ctx, s: &mut Surface, t: f64) -> Damage {
        if let Some(c) = &mut self.clock {
            c.draw(s, &ctx.ui, t);
        }
        let mut d = Damage::new(s.w, s.h);
        d.all();
        d
    }
}

/// The app as the compliance test hosts it, with no shell behind it.
#[doc(hidden)]
pub fn app() -> impl App {
    ClockApp { clock: None, last_wall: 0.0 }
}

fn main() {
    bifrost_app::run(ClockApp { clock: None, last_wall: 0.0 });
}

#[cfg(test)]
pub fn test_ui() -> bifrost_app::Ui {
    use bifrost_app::kit::Font;
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets");
    let data: &'static [u8] = Box::leak(std::fs::read(format!("{dir}/SFUI.ttf")).unwrap().into_boxed_slice());
    let mut ui = bifrost_app::Ui::new(Font::parse(data).unwrap(), 1200.0 / 768.0);
    ui.symbols = springboard::symbols::Symbols::load(dir);
    ui
}
