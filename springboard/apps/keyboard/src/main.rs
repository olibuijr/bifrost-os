//! The on-screen keyboard as a system app (`is.olibuijr.keyboard`, `kind = "system"`, `provides = ["keyboard"]`).
//!
//! The shell hosts it like any app but composites its frame at the bottom edge and sends it the focused field's
//! configuration (`ToApp::KeyboardConfig`) and touches (`ToApp::Touch`, y relative to the keyboard's top). What the
//! user types goes back as `ToShell::KeyboardOut`, which the shell forwards to the focused field. The keyboard
//! draws at its resting position; the shell animates the slide.
#![allow(dead_code)]

mod emoji;
mod kb_emoji;
mod kb_floating;
mod kb_pressed;
mod kb_shiftswipe;
mod kb_shortcutbar;
mod kb_trackpad;
mod kb_variants;
mod keyboard;
mod predict;
#[cfg(test)]
mod testutil;

use bifrost_app::proto::{kbflags, KeyboardConfig, KeyboardLayout, KeyboardOut, ReturnKey};
use bifrost_app::kit::{Damage, Surface};
use bifrost_app::speech_recognizer::SFSpeechAudioBufferRecognitionRequest;
use bifrost_app::speech_whisper;
use bifrost_app::{App, Ctx, Input, TouchKind};
use bifrost_core::json::Json;
use keyboard::{Keyboard, Mode, Out};
use springboard::shell::kb_capslock::KbPrefs;
use springboard::shell::kb_clicks::Click;
use springboard::shell::kb_layouts;

struct KeyboardApp {
    kb: Option<Keyboard>,
    config: KeyboardConfig,
    /// the last height reported to the shell, px
    reported: u32,
    /// the input language the shell last knew (index)
    language: u8,
    dictation: Dictation,
}

/// The locale the on-device recognizer serves.
const DICTATION_LOCALE: &str = "en_US";
/// How often a running dictation asks the shell for its transcripts, ms.
const POLL_MS: f64 = 250.0;

/// The microphone key. Final transcripts are typed where the focus is; the shell keeps the audio on this machine.
#[derive(Default)]
struct Dictation {
    /// the `start` request in flight
    start: Option<u32>,
    /// the `poll` request in flight
    poll: Option<u32>,
    /// true from `start` until the shell reports the pipeline ended
    active: bool,
    next_poll: f64,
}

fn mode_of(layout: KeyboardLayout) -> Mode {
    match layout {
        KeyboardLayout::Docked => Mode::Docked,
        KeyboardLayout::Split => Mode::Split,
        KeyboardLayout::Floating => Mode::Floating,
    }
}

fn return_kind(key: ReturnKey) -> kb_pressed::ReturnKind {
    use kb_pressed::ReturnKind as K;
    match key {
        ReturnKey::Return => K::Return,
        ReturnKey::Search => K::Search,
        ReturnKey::Go => K::Go,
        ReturnKey::Done => K::Done,
        ReturnKey::Next => K::Next,
        ReturnKey::Send => K::Send,
    }
}

/// `Out::Named` carries `&'static str`; map what travels over the wire back to the static names.
fn static_name(name: &str) -> Option<&'static str> {
    ["Escape", "Tab", "Paste", "Up", "Down", "Left", "Right", "More", "Ctrl"].into_iter().find(|n| *n == name)
}

fn send_out(ctx: &Ctx, out: Out) {
    ctx.keyboard_out(match out {
        Out::Text(t) => KeyboardOut::Text(t),
        Out::Backspace => KeyboardOut::Backspace,
        Out::Enter => KeyboardOut::Enter,
        Out::Hide => KeyboardOut::Hide,
        Out::Named(n) => KeyboardOut::Named(n.into()),
        Out::Dictate => return,
    });
}

impl KeyboardApp {
    /// Sends an output, starting or stopping dictation for the microphone key.
    fn route(&mut self, ctx: &Ctx, out: Out) {
        match out {
            Out::Dictate => self.toggle_dictation(ctx),
            other => send_out(ctx, other),
        }
    }

    fn toggle_dictation(&mut self, ctx: &Ctx) {
        if self.dictation.active {
            speech_whisper::stop(ctx);
            return;
        }
        let mut request = SFSpeechAudioBufferRecognitionRequest::new();
        request.should_report_partial_results = false;
        self.dictation = Dictation { start: Some(speech_whisper::start(ctx, DICTATION_LOCALE, &request)), poll: None, active: true, next_poll: 0.0 };
    }
}

impl KeyboardApp {
    fn flush(&mut self, ctx: &Ctx) {
        let Some(kb) = &mut self.kb else { return };
        // the globe key changed the input language: the shell owns and stores it
        let (_, language) = kb_layouts::state();
        if language != self.language {
            self.language = language;
            ctx.keyboard_out(KeyboardOut::Language(language));
        }
        for click in kb.take_clicks() {
            ctx.keyboard_out(KeyboardOut::Click(match click {
                Click::Key => 0,
                Click::Delete => 1,
                Click::Modifier => 2,
            }));
        }
        let h = kb.h.ceil() as u32;
        if h != self.reported {
            self.reported = h;
            ctx.keyboard_height(h);
        }
    }

    fn apply(&mut self) {
        let Some(kb) = &mut self.kb else { return };
        let c = self.config;
        // the shell owns the language choice: take it, and do not echo it back
        kb_layouts::apply(c.languages, c.language);
        self.language = kb_layouts::state().1;
        kb.set_mode(mode_of(c.layout));
        kb.set_return_kind(return_kind(c.return_key));
        kb.set_predictive(c.predictive());
        kb.set_auto_caps(c.flags & kbflags::AUTO_CAPS != 0);
        kb.set_kb_prefs(KbPrefs { enable_caps_lock: c.flags & kbflags::CAPS_LOCK != 0, period_shortcut: c.flags & kbflags::PERIOD_SHORTCUT != 0 });
        // a keyboard that is hidden resets its typing state when it is shown again
        kb.set_shown(c.shown(), 0.0);
    }
}

impl App for KeyboardApp {
    fn id(&self) -> &str {
        "is.olibuijr.keyboard"
    }

    fn configure(&mut self, ctx: &Ctx) {
        ctx.enable_sudden_termination();
        let (w, s) = (ctx.cfg.w as f32, ctx.cfg.scale);
        match &mut self.kb {
            Some(kb) => {
                kb.resize(w);
                kb.set_scale(s);
            }
            None => {
                let mut kb = Keyboard::new(w, s, self.config.flags & kbflags::ACCESSORY != 0);
                kb.set_shown(true, 0.0);
                self.kb = Some(kb);
            }
        }
        self.apply();
        self.flush(ctx);
    }

    fn keyboard_config(&mut self, ctx: &Ctx, config: KeyboardConfig) -> bool {
        let accessory_changed = (config.flags ^ self.config.flags) & kbflags::ACCESSORY != 0;
        self.config = config;
        if accessory_changed {
            // the terminal row changes the key set: rebuild at the current size
            if let Some(kb) = self.kb.take() {
                let (w, s) = (kb.w, ctx.cfg.scale);
                self.kb = Some(Keyboard::new(w, s, config.flags & kbflags::ACCESSORY != 0));
            }
        }
        self.apply();
        self.flush(ctx);
        true
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        let Input::Touch { kind, x, y, t, .. } = ev else { return false };
        let Some(kb) = &mut self.kb else { return false };
        let t = t as f64;
        let mut released = None;
        match kind {
            TouchKind::Down => kb.down(x, y, t),
            TouchKind::Move => kb.moved(x, y),
            TouchKind::Up => released = kb.up(x, y, t),
            _ => {}
        }
        if let Some(out) = released {
            self.route(ctx, out);
        }
        self.flush(ctx);
        true
    }

    fn tick(&mut self, ctx: &Ctx, t: f64) -> bool {
        let Some(kb) = &mut self.kb else { return false };
        let outs = kb.tick(t);
        let animating = kb.animating();
        for out in outs {
            self.route(ctx, out);
        }
        if self.dictation.active && self.dictation.start.is_none() && self.dictation.poll.is_none() && t >= self.dictation.next_poll {
            self.dictation.poll = Some(speech_whisper::poll(ctx));
            self.dictation.next_poll = t + POLL_MS;
        }
        self.flush(ctx);
        animating
    }

    fn service_reply(&mut self, ctx: &Ctx, request: u32, result: &Result<Json, String>) -> bool {
        if self.dictation.start == Some(request) {
            self.dictation.start = None;
            if result.is_err() {
                self.dictation.active = false;
            }
            return true;
        }
        if self.dictation.poll == Some(request) {
            self.dictation.poll = None;
            match speech_whisper::poll_from_reply(result) {
                Ok(reply) => {
                    for t in reply.transcripts.iter().filter(|t| t.is_final) {
                        send_out(ctx, Out::Text(format!("{} ", t.text)));
                    }
                    if !reply.running {
                        self.dictation.active = false;
                    }
                }
                Err(_) => self.dictation.active = false,
            }
            return true;
        }
        false
    }

    fn next_wake(&self, t: f64) -> Option<f64> {
        if self.dictation.active {
            return Some(t + POLL_MS);
        }
        self.kb.as_ref().filter(|kb| kb.animating()).map(|_| t + 16.0)
    }

    fn draw(&mut self, ctx: &Ctx, surface: &mut Surface, t: f64) -> Damage {
        if let Some(kb) = &mut self.kb {
            kb.draw(surface, &ctx.ui, 0.0, t);
        }
        let mut d = Damage::new(surface.w, surface.h);
        d.all();
        d
    }
}

fn main() {
    // no access to the shell's data in the sandbox: layout state arrives in `KeyboardConfig`, the emoji font and word
    // lists come from this package
    kb_layouts::use_remote_state();
    springboard::emoji::reload_from(&predict::package_dir().join("assets/NotoColorEmoji.ttf"));
    predict::reload_words(&predict::package_words());
    let config = KeyboardConfig { flags: kbflags::SHOWN, languages: 1, language: 0, ..Default::default() };
    bifrost_app::run(KeyboardApp { kb: None, config, reported: 0, language: 0, dictation: Dictation::default() });
}
