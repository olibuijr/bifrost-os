//! Clock as an installed BifrOSt app: World Clock, Alarm, Stopwatch and
//! Timer, hosted through the SDK. Alarms and timers are only edited here;
//! the shell's headless Clock service fires them and shows the alert, so
//! they keep ringing with this app closed.

#[allow(dead_code)]
mod clock;
#[allow(dead_code)]
mod icon;
#[allow(dead_code)]
mod system;

use bifrost_app::kit::{Damage, Event, Surface};
use bifrost_app::{App, Ctx, Input, TouchKind, KeyInput};
use bifrost_app::control::{RpcError, INVALID_PARAMS};
use bifrost_app::intents::{self, Intent, IntentResult};
use bifrost_app::widgets_timeline as wt;
use bifrost_core::json::Json;
use springboard::shell::keyboard::Out;
use springboard::bare::sys::wall_time;

struct ClockApp {
    clock: Option<clock::Clock>,
    last_wall: f64,
    system: system::System,
}

impl ClockApp {
    /// Running timers in the list (the shell removes a timer once it has fired).
    fn running_timers(&self) -> usize {
        self.clock.as_ref().map_or(0, |c| c.timers().list.iter().filter(|item| item.running()).count())
    }

    /// Sends the World Clock timeline when the chosen cities changed, or when `force` is set.
    fn sync_world(&mut self, ctx: &Ctx, force: bool) {
        let Some(c) = &self.clock else { return };
        let cities = c.cities();
        let unix = c.wall().now(bifrost_proto::sys::now_ms());
        self.system.sync_world(ctx, &cities, unix, force);
    }

    /// Input and the state it can change: the cities and the Dock bar for a running timer.
    fn after_input(&mut self, ctx: &Ctx) {
        self.sync_world(ctx, false);
        let running = self.running_timers() > 0;
        self.system.timers(ctx, running, false);
    }
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
                self.sync_world(ctx, true);
            }
        }
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        let handled = self.input(ctx, ev);
        self.after_input(ctx);
        handled
    }

    fn service_reply(&mut self, _ctx: &Ctx, request: u32, result: &Result<Json, String>) -> bool {
        // a shell without the service answers with an error; the Clock works without it
        if let Err(error) = result {
            eprintln!("clock: service request {request} failed: {error}");
        }
        false
    }

    fn service_event(&mut self, ctx: &Ctx, service: &str, event: &str, body: &Json) -> bool {
        // the shell's widget asks for a new timeline once the last one has run out
        if let Some(wt::Event::Reload { .. }) = wt::parse_event(service, event, body) {
            self.sync_world(ctx, true);
        }
        false
    }

    fn intents(&self) -> Vec<Intent> {
        let mut declared = clock::intent::intents();
        declared.extend([
            Intent::new("start-stopwatch", "Start Stopwatch", "Starts the stopwatch if it is stopped.").shortcut(),
            Intent::new("stop-stopwatch", "Stop Stopwatch", "Stops the stopwatch if it is running.").shortcut(),
            Intent::new("record-lap", "Record Lap", "Records a lap while the stopwatch is running.").shortcut(),
        ]);
        declared
    }

    fn perform_intent(&mut self, _ctx: Option<&Ctx>, _data_dir: &std::path::Path, id: &str, params: &Json) -> Result<IntentResult, RpcError> {
        if id == clock::intent::TIME_INTENT {
            let (unix, offset) = wall_time();
            return clock::intent::perform(id, params, unix, offset);
        }
        let t = bifrost_proto::sys::now_ms();
        if id == clock::intent::NEXT_ALARM_INTENT {
            let next = self.clock.as_ref().and_then(|c| c.next_alarm_text(t));
            return clock::intent::perform_alarm(id, next);
        }
        let Some(c) = &mut self.clock else { return Err(intents::failed("Clock is not ready yet")) };
        if id == clock::intent::SET_ALARM_INTENT {
            let alarm = clock::intent::alarm_from_params(params)?;
            let text = clock::intent::alarm_set_text(&alarm);
            let id = c.add_alarm(alarm, t);
            return Ok(IntentResult::value(Json::Num(id as f64)).with_dialog(&text));
        }
        let (changed, done) = match id {
            "start-stopwatch" => (c.change_stopwatch(t, |s| s.start(t)), "Stopwatch started"),
            "stop-stopwatch" => (c.change_stopwatch(t, |s| s.stop(t)), "Stopwatch stopped"),
            "record-lap" => (c.change_stopwatch(t, |s| s.record_lap(t)), "Lap recorded"),
            _ => return Err(RpcError::new(INVALID_PARAMS, format!("no intent {id}"))),
        };
        Ok(IntentResult::dialog(if changed { done } else { "Nothing to change" }))
    }

    fn tick(&mut self, ctx: &Ctx, t: f64) -> bool {
        let before = self.running_timers();
        let Some(c) = &mut self.clock else { return false };
        if t - self.last_wall >= 1000.0 {
            self.last_wall = t;
            let (unix, offset) = wall_time();
            c.set_wall(unix, offset, t);
        }
        let dirty = c.tick(&ctx.ui, t);
        // the shell removes a timer when it fires; a drop in the running count is a finished timer
        let after = self.running_timers();
        self.system.timers(ctx, after > 0, after < before);
        dirty
    }

    fn next_wake(&self, t: f64) -> Option<f64> {
        self.clock.as_ref().and_then(|c| c.next_wake(t, true))
    }

    /// The wheel comes as itself: one notch moves a picker drum one row (W5-3). Elsewhere it scrolls as the drag the
    /// shell emulated before (the lists and the World Clock cards).
    fn pointer_interest(&self) -> u32 {
        bifrost_app::pointer_events::WHEEL
    }

    fn pointer(&mut self, ctx: &Ctx, ev: &bifrost_app::pointer_events::PointerEvent) -> bool {
        let bifrost_app::pointer_events::PointerEvent::Wheel { x, y, dy, precise, .. } = *ev else { return false };
        let Some(c) = &mut self.clock else { return false };
        let t = bifrost_app::proto::sys::now_ms();
        if let Some(changed) = c.wheel(&ctx.ui, x, y, dy, precise, t) {
            return changed;
        }
        // not over a drum: a short drag of the content's travel, resting before the lift so it does not fling
        c.touch(&ctx.ui, Event::Down(x, y), t);
        c.touch(&ctx.ui, Event::Move(x, y + dy / 2.0), t + 8.0);
        c.touch(&ctx.ui, Event::Move(x, y + dy), t + 16.0);
        c.touch(&ctx.ui, Event::Up(x, y + dy), t + 16.0 + 200.0);
        true
    }

    fn controls(&self, ctx: &Ctx) -> Vec<bifrost_app::controls::Node> {
        self.clock.as_ref().map(|c| c.controls(&ctx.ui)).unwrap_or_default()
    }

    fn draw(&mut self, ctx: &Ctx, s: &mut Surface, t: f64) -> Damage {
        let mut d = Damage::new(s.w, s.h);
        // a countdown second repaints only the timer cards; anything else repaints the window
        match self.clock.as_mut().map(|c| c.draw(s, &ctx.ui, t)) {
            Some(Some(rects)) => {
                for (x, y, w, h) in rects {
                    d.add(x.floor() as i32, y.floor() as i32, w.ceil() as i32 + 1, h.ceil() as i32 + 1);
                }
            }
            _ => d.all(),
        }
        d
    }
}

impl ClockApp {
    fn input(&mut self, ctx: &Ctx, ev: Input) -> bool {
        // a right click on an alarm card opens its menu (Edit, Turn On/Off, Delete); the answer is a ContextAction
        if let Input::Context { x, y } = &ev {
            let Some(c) = &mut self.clock else { return false };
            if let Some(items) = c.alarm_context(&ctx.ui, *x, *y) {
                ctx.context_menu(*x, *y, items);
                return true;
            }
            return false;
        }
        if let Input::ContextAction(id) = &ev {
            let Some(c) = &mut self.clock else { return false };
            return c.context_action(&ctx.ui, *id, bifrost_app::proto::sys::now_ms()).is_some();
        }
        // Delete (Backspace) and forward Delete remove the selected alarm, or the one open in the editor
        if let Input::Key(KeyInput::Code(8 | 46)) = &ev {
            if let Some(c) = &mut self.clock {
                if c.delete_key(bifrost_app::proto::sys::now_ms()).is_some() {
                    return true;
                }
            }
        }
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
        // Copy Laps: the lap table goes on the general pasteboard as plain text
        if let Some(text) = c.take_export() {
            bifrost_app::pasteboard::NativePasteboard::general().set_string(ctx, &text);
        }
        true
    }
}

/// The app as the compliance test hosts it, with no shell behind it.
#[doc(hidden)]
pub fn app() -> impl App {
    ClockApp { clock: None, last_wall: 0.0, system: Default::default() }
}

fn main() {
    bifrost_app::run(ClockApp { clock: None, last_wall: 0.0, system: Default::default() });
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
