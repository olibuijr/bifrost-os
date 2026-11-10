//! Setup Assistant: the first-run window after install. It walks the steps in `springboard::setup::Step` (time zone,
//! keyboard, network, accessibility shortcut, account, appearance, privacy, BifrOSt Account) and, on Get Started,
//! writes the state file the shell reads at start-up. The window is `window.rs` (state, layout and hit tests),
//! `draw.rs` (painting); the model is the `springboard::setup` module.

mod draw;
mod window;

use bifrost_app::kit::{Damage, Surface};
use bifrost_app::{App, Ctx, Input, KeyInput, TouchKind};
use springboard::setup::{self, Cloud, State};
use window::{Act, Setup, Switch};

impl App for Setup {
    fn id(&self) -> &str {
        setup::APP_ID
    }

    fn configure(&mut self, ctx: &Ctx) {
        // the window keeps no unsaved state until Get Started, so the shell may end it at once
        ctx.enable_sudden_termination();
        let scale = ctx.ui.pt(1.0);
        self.size = (ctx.cfg.w as f32 / scale, ctx.cfg.h as f32 / scale);
        self.dark = ctx.cfg.dark;
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        match ev {
            Input::Touch { kind, x, y, .. } => {
                let scale = ctx.ui.pt(1.0);
                let (x, y) = (x / scale, y / scale);
                match kind {
                    TouchKind::Down => {
                        self.press = self.act_at(x, y);
                        false
                    }
                    TouchKind::Up => {
                        let released = self.act_at(x, y);
                        let pressed = self.press.take();
                        match (released, pressed) {
                            (Some(r), Some(p)) if r == p => self.activate(ctx, r),
                            _ => false,
                        }
                    }
                    TouchKind::Move => false,
                    TouchKind::Cancel => {
                        self.press = None;
                        false
                    }
                }
            }
            Input::Key(key) => match key {
                KeyInput::Code(13) => self.activate(ctx, Act::Next),
                KeyInput::Code(1 | 27) => self.activate(ctx, Act::Back),
                _ => false,
            },
            _ => false,
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

impl Setup {
    /// Runs the action of a control. True when the window needs a new frame.
    fn activate(&mut self, ctx: &Ctx, act: Act) -> bool {
        self.error = None;
        let zones = self.zones();
        match act {
            Act::Next => {
                if self.step == setup::Step::Done {
                    return self.finish(ctx);
                }
                self.step = self.step.next();
            }
            Act::Back => self.step = self.step.back(),
            Act::Zone(i) => {
                if let Some(z) = zones.get(i) {
                    self.choices.timezone = z.clone();
                }
            }
            Act::Layout(i) => {
                if let Some((_, id)) = springboard::installer::app::LANGUAGE_LAYOUTS.get(i) {
                    self.choices.keyboard = id.to_string();
                }
            }
            Act::Switch(s) => {
                let flag = match s {
                    Switch::Shortcut => &mut self.choices.accessibility_shortcut,
                    Switch::Location => &mut self.choices.location,
                    Switch::Analytics => &mut self.choices.analytics,
                };
                *flag = !*flag;
            }
            Act::Look(m) => self.choices.appearance = m,
            Act::Tint(a) => self.choices.accent = a,
            Act::Cloud(c) => self.choices.cloud = c,
        }
        true
    }

    /// Records what the user chose and that the run is done. The shell reads the file when this app exits, copies the
    /// choices and sets the chosen time zone: this app runs in the sandbox, where the system service cannot be reached,
    /// so it never sets the zone itself. The account page opens only for Sign In. A failure stays on the Done screen
    /// with a message, so the user can try again.
    fn finish(&mut self, ctx: &Ctx) -> bool {
        // in the app's own data directory: the sandbox shows this app no other part of the shell's tree
        let path = setup::own_state_path();
        let state = State { done: true, applied: false, choices: self.choices.clone() };
        if state.save(&path).is_err() {
            self.error = Some("Your choices could not be saved. Try again.".into());
            return true;
        }
        if self.choices.cloud == Cloud::SignIn {
            ctx.open_url(setup::ACCOUNT_URL);
        }
        // the state file is written: the shell copies the choices and sets the zone when this app is gone
        std::process::exit(0)
    }
}

fn main() {
    bifrost_app::run(Setup::start());
}
