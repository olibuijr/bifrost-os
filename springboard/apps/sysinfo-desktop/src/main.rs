//! System Information as an installed BifrOSt system app: the hardware tree (processor, memory, graphics, PCI, USB,
//! displays, network, power, storage) and the software section (the operating system and the installed apps). The
//! facts are read by `bifrost-core`'s `sysinfo`; `model.rs` builds the rows, `window.rs` holds the state and the
//! geometry, and `draw.rs` paints them.

mod draw;
mod model;
mod window;

use bifrost_app::controls::Node;
use bifrost_app::kit::{Damage, Surface};
use bifrost_app::{App, Ctx, Input, KeyInput, TouchKind};
use window::{SysInfo, Target, STEP_PT};

impl App for SysInfo {
    fn id(&self) -> &str {
        "is.olibuijr.sysinfo.desktop"
    }

    fn configure(&mut self, ctx: &Ctx) {
        ctx.enable_sudden_termination(); // keeps no unsaved state: the facts are read again on every launch
        self.size = (ctx.cfg.w as f32, ctx.cfg.h as f32);
        if !self.loaded {
            self.reload();
        }
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        match ev {
            Input::Touch { kind, x, y, .. } => {
                let scale = ctx.ui.pt(1.0);
                let (x, y) = (x / scale, y / scale);
                match kind {
                    TouchKind::Down => {
                        self.press = self.hit(&ctx.ui, x, y);
                        self.drag_y = Some(y);
                        self.dragged = false;
                        false
                    }
                    TouchKind::Move => {
                        // a drag over the card scrolls it; the press then is not a tap
                        let Some(last) = self.drag_y else { return false };
                        if self.press != Target::Content {
                            return false;
                        }
                        self.drag_y = Some(y);
                        if (y - last).abs() < 0.5 {
                            return false;
                        }
                        self.dragged = true;
                        self.scroll_by(last - y)
                    }
                    TouchKind::Up => {
                        let released = self.hit(&ctx.ui, x, y);
                        let pressed = std::mem::replace(&mut self.press, Target::Nothing);
                        let was_drag = std::mem::replace(&mut self.dragged, false);
                        self.drag_y = None;
                        if !was_drag && released == pressed && released != Target::Nothing {
                            self.activate(released)
                        } else {
                            false
                        }
                    }
                    TouchKind::Cancel => {
                        self.press = Target::Nothing;
                        self.drag_y = None;
                        self.dragged = false;
                        false
                    }
                }
            }
            // a published control (accessibility, `bifrost pc midget tap-window`): the same action as a click
            Input::Activate(id) => self.activate(SysInfo::target_of_id(id)),
            Input::Key(KeyInput::Code(38)) => self.scroll_by(-STEP_PT),
            Input::Key(KeyInput::Code(40)) => self.scroll_by(STEP_PT),
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
    bifrost_app::run(SysInfo::new());
}
