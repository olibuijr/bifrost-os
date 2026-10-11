//! Weather as an installed BifrOSt app: a Golden Gate window with a Locations sidebar and the forecast,
//! hosted through the SDK. The shell draws the keyboard; the app
//! asks for it with `wants_keyboard` and receives typed keys as `Input::Key`.

mod weather;

use bifrost_app::kit::{Damage, Event, Surface};
use bifrost_app::{App, Ctx, Input, KeyInput, TouchKind};
use springboard::shell::keyboard::Out;
use springboard::shell::music::radio::{Fetch, Http};
use std::sync::Arc;

struct WeatherApp {
    w: Option<weather::Weather>,
    kb: bool,
}

impl WeatherApp {
    fn sync_keyboard(&mut self, ctx: &Ctx) {
        let on = self.w.as_ref().is_some_and(|w| w.keyboard_shown());
        if on != self.kb {
            self.kb = on;
            ctx.wants_keyboard(on);
        }
    }
}

/// The network, except under the testing kit (`BIFROST_TEST_PIN=1`): offline, so a snapshot never shows live weather.
fn pinned_fetcher() -> Arc<dyn Fetch> {
    struct Offline;
    impl Fetch for Offline {
        fn get(&self, _url: &str, _limit: usize) -> std::io::Result<Vec<u8>> {
            Err(std::io::Error::other("offline"))
        }
    }
    if bifrost_app::testing::pinned() { Arc::new(Offline) } else { Arc::new(Http::default()) }
}

impl App for WeatherApp {
    fn id(&self) -> &str {
        "is.olibuijr.weather.desktop"
    }

    fn configure(&mut self, ctx: &Ctx) {
        ctx.enable_sudden_termination(); // keeps no unsaved state
        let (w, h) = (ctx.cfg.w as i32, ctx.cfg.h as i32);
        match &mut self.w {
            Some(c) => c.set_size(w, h),
            None => self.w = Some(weather::Weather::new(&ctx.ui, w, h, ctx.data_dir.clone(), pinned_fetcher())),
        }
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
            Input::Key(k) => {
                let out = match k {
                    KeyInput::Text(s) => Out::Text(s),
                    KeyInput::Code(8) => Out::Backspace,
                    KeyInput::Code(13) => Out::Enter,
                    KeyInput::Code(9) => Out::Named("Tab"),
                    KeyInput::Code(37) => Out::Named("Left"),
                    KeyInput::Code(39) => Out::Named("Right"),
                    KeyInput::Code(_) => return false,
                };
                c.key(&ctx.ui, out, bifrost_app::proto::sys::now_ms());
            }
            Input::Activate(id) => {
                c.activate_control(&ctx.ui, id, bifrost_app::proto::sys::now_ms());
            }
            Input::Open(_) => return false,
            _ => return false,
        }
        self.sync_keyboard(ctx);
        true
    }

    fn tick(&mut self, ctx: &Ctx, t: f64) -> bool {
        let r = self.w.as_mut().is_some_and(|c| c.tick(&ctx.ui, t));
        self.sync_keyboard(ctx);
        r
    }

    fn next_wake(&self, t: f64) -> Option<f64> {
        self.w.as_ref().and_then(|c| c.next_wake(t))
    }

    fn controls(&self, ctx: &Ctx) -> Vec<bifrost_app::controls::Node> {
        self.w.as_ref().map(|w| w.controls(&ctx.ui)).unwrap_or_default()
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
    bifrost_app::run(WeatherApp { w: None, kb: false });
}
