//! Digital Color Meter as an installed BifrOSt system app: a magnified view around the pointer, the pixel value (or the
//! mean of an aperture) in several colour spaces as hex, decimal, 16-bit and percent, a readout locked on either or both
//! axes, copy as text or as a swatch, and the View and Color menus. `menu.rs` holds the menus.
//! `source.rs` is the only place pixels come from: the shell's `screen` service, asked on a refresh loop (`tick`) and
//! answered in `service_reply`. `color.rs` holds the pure conversions, `loupe.rs` the magnifier geometry and pointer
//! state, `window.rs` the state and layout, and `draw.rs` the painting.

mod color;
mod draw;
mod loupe;
mod menu;
mod source;
mod window;

use bifrost_app::controls::Node;
use bifrost_app::kit::{clipboard, pack, Damage, Image, Surface};
use bifrost_app::proto::AxAction;
use bifrost_app::{screen_sample, App, AppMenu, Ctx, Input, KeyInput, TouchKind};
use bifrost_core::json::Json;
use color::Rgb8;
use source::{Ask, ShellSource};
use window::{ColorMeter, Outcome, Target, CTL_APERTURE};

/// How often the refresh loop asks the shell for the pointer and the pixels around it, in milliseconds.
const POLL_MS: f64 = 50.0;

/// Arrow key codes, as the shell sends them.
const KEY_LEFT: u32 = 37;
const KEY_UP: u32 = 38;
const KEY_RIGHT: u32 = 39;
const KEY_DOWN: u32 = 40;

impl ColorMeter {
    /// Performs what an action asked for. True when the window should redraw.
    fn apply(&mut self, outcome: Outcome) -> bool {
        match outcome {
            Outcome::Nothing => false,
            Outcome::Redraw => true,
            Outcome::CopyText(text) => {
                clipboard::set_text(&text);
                true
            }
            Outcome::CopySwatch(c) => {
                copy_swatch(c);
                true
            }
        }
    }
}

/// Puts a square swatch of `c` on the clipboard as an image.
fn copy_swatch(c: Rgb8) {
    const N: usize = 64;
    let img = Image { w: N, h: N, px: vec![pack(c.r, c.g, c.b, 255); N * N] };
    if let Some(cb) = clipboard::global().as_mut() {
        cb.set_image(img);
    }
}

impl App for ColorMeter {
    fn id(&self) -> &str {
        "is.olibuijr.colormeter.desktop"
    }

    fn configure(&mut self, ctx: &Ctx) {
        ctx.enable_sudden_termination(); // keeps no unsaved state: the readout is taken again from the screen
        self.size = (ctx.cfg.w as f32, ctx.cfg.h as f32);
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        match ev {
            Input::Touch { kind, x, y, .. } => {
                let scale = ctx.ui.pt(1.0);
                let (x, y) = (x / scale, y / scale);
                match kind {
                    TouchKind::Down => {
                        self.press = self.hit(&ctx.ui, x, y);
                        // the aperture slider acts on the press, then follows the drag
                        self.press == Target::Aperture && self.drag_aperture(x)
                    }
                    TouchKind::Move => self.press == Target::Aperture && self.drag_aperture(x),
                    TouchKind::Up => {
                        if self.press == Target::Aperture {
                            self.press = Target::Nothing;
                            return self.drag_aperture(x);
                        }
                        let released = self.hit(&ctx.ui, x, y);
                        let pressed = std::mem::replace(&mut self.press, Target::Nothing);
                        if released != pressed {
                            return false;
                        }
                        match released {
                            // a tap on a magnified cell picks that display pixel
                            Target::Loupe => self.pick_loupe(x, y),
                            Target::Toolbar(_) => {
                                let outcome = self.activate(released);
                                self.apply(outcome)
                            }
                            Target::Aperture | Target::Nothing => false,
                        }
                    }
                    TouchKind::Cancel => {
                        self.press = Target::Nothing;
                        false
                    }
                }
            }
            // a published control (accessibility, `bifrost pc midget tap-window`): the same action as a click
            Input::Activate(id) => {
                let outcome = self.activate(Target::Toolbar(id as u32));
                self.apply(outcome)
            }
            Input::Key(KeyInput::Code(KEY_LEFT)) => self.nudge(-1, 0),
            Input::Key(KeyInput::Code(KEY_RIGHT)) => self.nudge(1, 0),
            Input::Key(KeyInput::Code(KEY_UP)) => self.nudge(0, -1),
            Input::Key(KeyInput::Code(KEY_DOWN)) => self.nudge(0, 1),
            _ => false,
        }
    }

    /// The refresh loop: takes the pointer the shell last reported and sends the requests that are not in flight.
    /// Returns true when the readout moved.
    fn tick(&mut self, ctx: &Ctx, _t: f64) -> bool {
        let moved = self.refresh();
        for ask in self.wants() {
            let request = match ask {
                Ask::Display => screen_sample::display(ctx),
                Ask::Pointer => screen_sample::pointer(ctx),
                Ask::Loupe(r) => screen_sample::sample(ctx, r.x, r.y, r.w, r.h),
            };
            self.track(request, ask);
        }
        moved
    }

    fn next_wake(&self, t: f64) -> Option<f64> {
        Some(t + POLL_MS)
    }

    fn service_reply(&mut self, _ctx: &Ctx, request: u32, result: &Result<Json, String>) -> bool {
        self.answer(request, result)
    }

    fn main_menu(&self, _ctx: &Ctx) -> Vec<AppMenu> {
        self.menus()
    }

    fn menu_action(&mut self, _ctx: &Ctx, id: u32) -> bool {
        let outcome = self.perform_menu(id);
        self.apply(outcome)
    }

    fn ax_set(&mut self, ctx: &Ctx, id: u64, action: AxAction) -> bool {
        if id == CTL_APERTURE as u64 {
            return self.aperture_action(&action);
        }
        match action {
            AxAction::Press => self.event(ctx, Input::Activate(id)),
            _ => false,
        }
    }

    fn controls(&self, ctx: &Ctx) -> Vec<Node> {
        draw::controls(self, &ctx.ui)
    }

    fn draw(&mut self, ctx: &Ctx, s: &mut Surface, _t: f64) -> Damage {
        draw::paint(self, s, &ctx.ui);
        let mut d = Damage::new(s.w, s.h);
        d.all();
        d
    }
}

fn main() {
    bifrost_app::run(ColorMeter::new(Box::new(ShellSource::default())));
}
