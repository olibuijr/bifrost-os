//! Digital Color Meter as an installed BifrOSt system app: a magnified view around the pointer, the pixel value in
//! sRGB, Display P3 and Generic RGB as hex, decimal and percent, a locked readout, and copy as text or as a swatch.
//! `source.rs` is the only place pixels come from; until the shell delivers screen pixels to apps it reports that
//! nothing can be read. `color.rs` holds the pure conversions, `loupe.rs` the magnifier geometry and pointer state,
//! `window.rs` the state and layout, and `draw.rs` the painting.

mod color;
mod draw;
mod loupe;
mod source;
mod window;

use bifrost_app::controls::Node;
use bifrost_app::kit::{clipboard, pack, Damage, Image, Surface};
use bifrost_app::{App, Ctx, Input, KeyInput, TouchKind};
use color::Rgb8;
use source::NoCapture;
use window::{ColorMeter, Outcome, Target};

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
                        false
                    }
                    TouchKind::Move => false,
                    TouchKind::Up => {
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
                            Target::Nothing => false,
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

    fn controls(&self, ctx: &Ctx) -> Vec<Node> {
        draw::controls(self, &ctx.ui)
    }

    fn draw(&mut self, ctx: &Ctx, s: &mut Surface, _t: f64) -> Damage {
        self.refresh();
        draw::paint(self, s, &ctx.ui);
        let mut d = Damage::new(s.w, s.h);
        d.all();
        d
    }
}

fn main() {
    bifrost_app::run(ColorMeter::new(Box::new(NoCapture)));
}
