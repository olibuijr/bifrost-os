//! Image Capture as an installed BifrOSt app: the cameras and SD cards with their pictures (copied into Pictures), and
//! the scanners with their resolution, colour, format, preview and scan (saved into Documents/Scans). `model.rs` holds
//! the state and the rules, `worker.rs` does the disk and scanner work on its own thread, `window.rs` lays the window
//! out and `draw.rs` paints it.

mod draw;
mod model;
mod window;
mod worker;

use bifrost_app::controls::Node;
use bifrost_app::kit::{Damage, Surface};
use bifrost_app::{App, Ctx, Input};
use window::{ImageCapture, Target};

impl App for ImageCapture {
    fn id(&self) -> &str {
        "is.olibuijr.imagecapture.desktop"
    }

    fn configure(&mut self, ctx: &Ctx) {
        self.resize(ctx.cfg.w as f32, ctx.cfg.h as f32);
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        match ev {
            Input::Touch { kind, x, y, .. } => {
                let scale = ctx.ui.pt(1.0);
                self.touch(&ctx.ui, kind, x / scale, y / scale)
            }
            // a published control (accessibility, `bifrost pc midget tap-window`): the same action as a click
            Input::Activate(id) => self.activate(Target::Toolbar(id as u32)),
            _ => false,
        }
    }

    /// The refresh loop: takes the replies the worker sent and lists the sources on a timer.
    fn tick(&mut self, _ctx: &Ctx, t: f64) -> bool {
        self.step(t)
    }

    fn next_wake(&self, t: f64) -> Option<f64> {
        Some(self.wake_at(t))
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
    bifrost_app::run(ImageCapture::new());
}
