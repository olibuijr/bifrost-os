//! Voice Memos as an installed BifrOSt app: the former in-shell Voice Memos, hosted through the SDK.
//! Recording and playback go through the audio service (`bifrost_app::audio`).

mod store;
mod voicememos;

use bifrost_app::kit::{Damage, Event, Surface};
use bifrost_app::{App, Ctx, Input, TouchKind, KeyInput};
use springboard::shell::keyboard::Out;
use springboard::shell::clock::Wall;
use std::path::PathBuf;

struct VoiceMemosApp {
    memos: Option<voicememos::VoiceMemos>,
    last_t: f64,
}

fn shell_dir() -> PathBuf {
    PathBuf::from(std::env::var("BIFROST_SERVICE_ROOT").unwrap_or_else(|_| "/var/lib/springboard".into()))
}

impl App for VoiceMemosApp {
    fn id(&self) -> &str {
        "is.olibuijr.voicememos.desktop"
    }

    fn configure(&mut self, ctx: &Ctx) {
        let (w, h) = (ctx.cfg.w as i32, ctx.cfg.h as i32);
        match &mut self.memos {
            Some(m) => m.set_size(w, h),
            None => self.memos = Some(voicememos::VoiceMemos::new(&ctx.ui, w, h, ctx.data_dir.clone(), Wall::sample(0.0), &shell_dir())),
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
            self.last_t = bifrost_app::proto::sys::now_ms();
            if let Some(memos) = &mut self.memos { memos.key(&ctx.ui, out, self.last_t); return true; }
            return false;
        }
        let (Some(m), Input::Touch { kind, x, y, t, .. }) = (&mut self.memos, ev) else { return false };
        let e = match kind {
            TouchKind::Down => Event::Down(x, y),
            TouchKind::Move => Event::Move(x, y),
            TouchKind::Up | TouchKind::Cancel => Event::Up(x, y),
        };
        self.last_t = t;
        m.touch(&ctx.ui, e, t);
        true
    }

    fn suspend(&mut self, _ctx: &Ctx) {
        if let Some(m) = &mut self.memos {
            m.suspend();
        }
    }

    fn tick(&mut self, ctx: &Ctx, t: f64) -> bool {
        self.last_t = t;
        self.memos.as_mut().is_some_and(|m| m.tick(&ctx.ui, t))
    }

    fn next_wake(&self, t: f64) -> Option<f64> {
        self.memos.as_ref().and_then(|m| m.next_wake(t))
    }

    fn controls(&self, ctx: &Ctx) -> Vec<bifrost_app::controls::Node> {
        self.memos.as_ref().map(|m| m.controls(&ctx.ui)).unwrap_or_default()
    }

    fn draw(&mut self, ctx: &Ctx, s: &mut Surface, t: f64) -> Damage {
        if let Some(m) = &mut self.memos {
            m.draw(s, &ctx.ui, t);
        }
        let mut d = Damage::new(s.w, s.h);
        d.all();
        d
    }
}

fn main() {
    bifrost_app::run(VoiceMemosApp { memos: None, last_t: 0.0 });
}
