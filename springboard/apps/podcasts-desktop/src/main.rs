//! Podcasts as an installed BifrOSt app: the former in-shell Podcasts, hosted through the SDK.
//! Playback goes through the audio service (`bifrost_app::audio`); text comes from the hardware keyboard.

mod feed;
mod icon;
mod podcasts;
mod transport;

use bifrost_app::kit::{Damage, Event, Surface};
use bifrost_app::{App, Ctx, Input, TouchKind, KeyInput};
use springboard::shell::keyboard::Out;
use std::path::PathBuf;
use std::sync::Arc;

struct PodcastsApp {
    podcasts: Option<podcasts::Podcasts>,
}

fn shell_dir() -> PathBuf {
    PathBuf::from(std::env::var("BIFROST_SERVICE_ROOT").unwrap_or_else(|_| "/var/lib/springboard".into()))
}

impl App for PodcastsApp {
    fn id(&self) -> &str {
        "is.olibuijr.podcasts.desktop"
    }

    fn configure(&mut self, ctx: &Ctx) {
        ctx.enable_sudden_termination(); // subscriptions and resume positions are written when they change
        let (w, h) = (ctx.cfg.w as i32, ctx.cfg.h as i32);
        match &mut self.podcasts {
            Some(p) => p.set_size(w, h),
            None => self.podcasts = Some(podcasts::Podcasts::new(&ctx.ui, w, h, ctx.data_dir.clone(), Arc::new(springboard::shell::music::radio::Http::default()), &shell_dir())),
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
            if let Some(app) = &mut self.podcasts { app.key(&ctx.ui, out, bifrost_app::proto::sys::now_ms()); return true; }
            return false;
        }
        let (Some(p), Input::Touch { kind, x, y, t, .. }) = (&mut self.podcasts, ev) else { return false };
        let e = match kind {
            TouchKind::Down => Event::Down(x, y),
            TouchKind::Move => Event::Move(x, y),
            TouchKind::Up | TouchKind::Cancel => Event::Up(x, y),
        };
        p.touch(&ctx.ui, e, t);
        true
    }

    fn controls(&self, ctx: &Ctx) -> Vec<bifrost_app::controls::Node> {
        self.podcasts.as_ref().map_or_else(Vec::new, |p| p.controls(&ctx.ui))
    }

    fn suspend(&mut self, _ctx: &Ctx) {
        if let Some(p) = &mut self.podcasts {
            p.suspend();
        }
    }

    fn tick(&mut self, ctx: &Ctx, t: f64) -> bool {
        self.podcasts.as_mut().is_some_and(|p| p.tick(&ctx.ui, t))
    }

    fn next_wake(&self, t: f64) -> Option<f64> {
        self.podcasts.as_ref().and_then(|p| p.next_wake(t))
    }

    fn draw(&mut self, ctx: &Ctx, s: &mut Surface, t: f64) -> Damage {
        if let Some(p) = &mut self.podcasts {
            p.draw(s, &ctx.ui, t);
        }
        let mut d = Damage::new(s.w, s.h);
        d.all();
        d
    }
}

fn main() {
    bifrost_app::run(PodcastsApp { podcasts: None });
}
