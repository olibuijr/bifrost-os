//! Voice Memos as an installed BifrOSt app: the former in-shell Voice Memos, hosted through the SDK.
//! Recording and playback go through the audio service (`bifrost_app::audio`). The system services are glue in
//! `services`: the microphone permission (`privacy`), the audio devices (`audio-devices`) and Now Playing.

mod services;
mod store;
mod voicememos;

use bifrost_app::audio_devices;
use bifrost_app::kit::{Damage, Event, Surface};
use bifrost_app::now_playing;
use bifrost_app::{App, AppMenu, AppMenuItem, Ctx, Input, TouchKind, KeyInput};
use bifrost_core::json::Json;
use springboard::shell::keyboard::Out;
use springboard::shell::clock::Wall;
use std::path::PathBuf;

struct VoiceMemosApp {
    memos: Option<voicememos::VoiceMemos>,
    last_t: f64,
    bus: services::Bus,
}

fn shell_dir() -> PathBuf {
    PathBuf::from(std::env::var("BIFROST_SERVICE_ROOT").unwrap_or_else(|_| "/var/lib/springboard".into()))
}

impl VoiceMemosApp {
    /// Sends the service calls the model queued on the last event, tick or reply.
    fn flush(&mut self, ctx: &Ctx) {
        if let Some(m) = &mut self.memos {
            for want in m.take_wants() {
                self.bus.send(ctx, want);
            }
        }
    }

    fn input(&mut self, ctx: &Ctx, ev: Input) -> bool {
        if let Input::Key(key) = &ev {
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
        self.flush(ctx);
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        let needs_frame = self.input(ctx, ev);
        self.flush(ctx);
        needs_frame
    }

    fn suspend(&mut self, ctx: &Ctx) {
        if let Some(m) = &mut self.memos {
            m.suspend();
        }
        self.flush(ctx);
    }

    fn tick(&mut self, ctx: &Ctx, t: f64) -> bool {
        self.last_t = t;
        // the wall clock is re-read every tick, so day titles and timestamps follow midnight and time-zone changes
        let needs_frame = self.memos.as_mut().is_some_and(|m| {
            m.set_wall(Wall::sample(t));
            m.tick(&ctx.ui, t)
        });
        self.flush(ctx);
        needs_frame
    }

    fn next_wake(&self, t: f64) -> Option<f64> {
        self.memos.as_ref().and_then(|m| m.next_wake(t))
    }

    fn controls(&self, ctx: &Ctx) -> Vec<bifrost_app::controls::Node> {
        self.memos.as_ref().map(|m| m.controls(&ctx.ui)).unwrap_or_default()
    }

    /// A service answered. Only the replies to this app's questions are taken here; others stay for `take_reply`.
    fn service_reply(&mut self, ctx: &Ctx, request: u32, result: &Result<Json, String>) -> bool {
        let Some(ask) = self.bus.take(request) else { return false };
        if let Some(m) = &mut self.memos {
            m.reply(ask, result);
        }
        self.flush(ctx);
        true
    }

    /// The shell pushed an event: a device or route change, or a transport command on the Now Playing tile.
    fn service_event(&mut self, ctx: &Ctx, service: &str, event: &str, body: &Json) -> bool {
        let Some(m) = &mut self.memos else { return false };
        if service == audio_devices::SERVICE && audio_devices::decode_event(event, body).is_some() {
            m.devices_changed();
        } else if let Some(command) = now_playing::command_from_event(service, event, body) {
            m.command(command);
        } else {
            return false;
        }
        self.flush(ctx);
        true
    }

    /// One app menu, "Recording", between App and the shell's File, Edit, View, Window and Help.
    fn main_menu(&self, _ctx: &Ctx) -> Vec<AppMenu> {
        let on = self.memos.as_ref().is_some_and(|m| m.has_selection());
        let split = self.memos.as_ref().is_some_and(|m| m.can_split());
        let skip = self.memos.as_ref().is_some_and(|m| m.skip_silence_on());
        vec![AppMenu {
            title: "Recording".into(),
            items: vec![
                AppMenuItem::new(voicememos::MENU_TRIM, "Trim\u{2026}").key("cmd+t").enabled(on),
                AppMenuItem::new(voicememos::MENU_SPLIT, "Split at Playhead").enabled(split),
                AppMenuItem::new(voicememos::MENU_NOTES, "Notes\u{2026}").key("cmd+shift+n").enabled(on),
                AppMenuItem::separator(),
                AppMenuItem::new(voicememos::MENU_RENAME, "Rename\u{2026}").enabled(on),
                AppMenuItem::new(voicememos::MENU_EXPORT, "Export to Downloads").key("cmd+shift+e").enabled(on),
                AppMenuItem::separator(),
                AppMenuItem::new(voicememos::MENU_SKIP, "Skip Silence").checked(skip).enabled(on),
            ],
        }]
    }

    fn menu_action(&mut self, ctx: &Ctx, id: u32) -> bool {
        let Some(m) = &mut self.memos else { return false };
        let handled = m.menu(&ctx.ui, id);
        self.flush(ctx);
        handled
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
    bifrost_app::run(VoiceMemosApp { memos: None, last_t: 0.0, bus: services::Bus::default() });
}
