//! Audio Setup: the devices of the audio server, their format, rate, buffer size, latency, channels and volume, and
//! the aggregate and multi-output devices the user makes from them. The window is `window.rs` (state, geometry and
//! hit tests), `draw.rs` (painting), `draft.rs` (the form and its composition text) and `device.rs` (the server
//! calls through `bifrost-audio-hal`). The model in `model.rs` is pure and tested on the host.

mod device;
mod draft;
mod draw;
mod model;
mod window;

use bifrost_app::controls::Node;
use bifrost_app::kit::{Damage, Surface};
use bifrost_app::{App, Ctx, Input, KeyInput, TouchKind};
use window::{AudioSetup, Target, REFRESH_S};

impl App for AudioSetup {
    fn id(&self) -> &str {
        "is.olibuijr.audiosetup.desktop"
    }

    fn configure(&mut self, ctx: &Ctx) {
        ctx.enable_sudden_termination(); // keeps no unsaved state: an open draft is simply dropped
        self.size = (ctx.cfg.w as f32, ctx.cfg.h as f32);
        if self.last_refresh.is_infinite() {
            let t = self.now;
            self.refresh(t);
        }
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        match ev {
            Input::Touch { kind, x, y, t, .. } => {
                self.now = t;
                let scale = ctx.ui.pt(1.0);
                let (x, y) = (x / scale, y / scale);
                match kind {
                    TouchKind::Down => {
                        self.press = self.hit(&ctx.ui, x, y);
                        false
                    }
                    TouchKind::Up => {
                        let released = self.hit(&ctx.ui, x, y);
                        let pressed = std::mem::replace(&mut self.press, Target::Nothing);
                        if released == pressed && released != Target::Nothing {
                            self.activate(released, t)
                        } else {
                            false
                        }
                    }
                    TouchKind::Move => false,
                    TouchKind::Cancel => {
                        self.press = Target::Nothing;
                        false
                    }
                }
            }
            // a published control (accessibility, `bifrost pc midget tap-window`): the same action as a click
            Input::Activate(id) => self.activate(AudioSetup::target_of_id(id), self.now),
            Input::Key(key) => self.key(key),
            _ => false,
        }
    }

    fn controls(&self, ctx: &Ctx) -> Vec<Node> {
        draw::controls(self, &ctx.ui)
    }

    /// Reads the device list every `REFRESH_S` seconds, so hot-plugged devices and new aggregates show up.
    fn tick(&mut self, _ctx: &Ctx, t: f64) -> bool {
        self.now = t;
        if t - self.last_refresh >= REFRESH_S {
            self.refresh(t)
        } else {
            false
        }
    }

    fn next_wake(&self, _t: f64) -> Option<f64> {
        if self.last_refresh.is_finite() {
            Some(self.last_refresh + REFRESH_S)
        } else {
            None
        }
    }

    fn draw(&mut self, ctx: &Ctx, s: &mut Surface, t: f64) -> Damage {
        self.now = t;
        draw::paint(self, s, &ctx.ui);
        let mut d = Damage::new(s.w, s.h);
        d.all();
        d
    }
}

impl AudioSetup {
    /// Keys in the editor: text goes into the name, Delete removes the last character, Return saves, Escape cancels.
    fn key(&mut self, key: KeyInput) -> bool {
        if self.editor.is_none() {
            return false;
        }
        let now = self.now;
        match key {
            KeyInput::Text(text) => {
                let clean: String = text.chars().filter(|c| !c.is_control()).collect();
                if clean.is_empty() {
                    return false;
                }
                if let Some(e) = self.editor.as_mut() {
                    e.draft.name.push_str(&clean);
                }
                true
            }
            KeyInput::Code(8) => {
                if let Some(e) = self.editor.as_mut() {
                    e.draft.name.pop();
                }
                true
            }
            KeyInput::Code(13) => self.activate(Target::Save, now),
            KeyInput::Code(1 | 27) => self.activate(Target::Cancel, now),
            _ => false,
        }
    }
}

fn main() {
    bifrost_app::run(AudioSetup::new());
}
