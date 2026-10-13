//! Disk Utility as an installed BifrOSt app in the Golden Gate full-size window: the disks and volumes in a sidebar,
//! and an info pane with capacity, file system, mount point and SMART status. Mount, unmount and eject go through the
//! disk session in `bifrost-core`. The window is `window.rs`, painting is `draw.rs`, the actions are `actions.rs`, the
//! model and its parsers are `model.rs`, and the reads are `sysfs.rs` and `probe.rs`.

mod actions;
mod draw;
mod model;
mod probe;
mod sysfs;
mod window;

use bifrost_app::controls::Node;
use bifrost_app::kit::{Damage, Surface};
use bifrost_app::{App, AppMenu, AppMenuItem, Ctx, Input, TouchKind};
use window::{DiskUtility, Target, B_DETACH, REFRESH_S};

/// The Image menu's detach item.
const MENU_DETACH: u32 = 1;

impl App for DiskUtility {
    fn id(&self) -> &str {
        "is.olibuijr.diskutil.desktop"
    }

    /// The Image menu: detaching works on the selected disk image, the same action as the toolbar's Detach.
    fn main_menu(&self, _ctx: &Ctx) -> Vec<AppMenu> {
        vec![AppMenu { title: "Image".into(), items: vec![AppMenuItem::new(MENU_DETACH, "Detach Disk Image").enabled(self.selected_image())] }]
    }

    fn menu_action(&mut self, _ctx: &Ctx, id: u32) -> bool {
        match id {
            MENU_DETACH => {
                let t = self.now;
                self.activate(Target::Toolbar(B_DETACH), t)
            }
            _ => false,
        }
    }

    fn configure(&mut self, ctx: &Ctx) {
        ctx.enable_sudden_termination(); // keeps no unsaved state
        self.size = (ctx.cfg.w as f32, ctx.cfg.h as f32);
        if self.last_refresh.is_infinite() {
            let t = self.now;
            self.refresh(t, true);
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
            Input::Activate(id) => {
                let t = self.now;
                self.activate(DiskUtility::target_of_id(id), t)
            }
            _ => false,
        }
    }

    fn controls(&self, ctx: &Ctx) -> Vec<Node> {
        draw::controls(self, &ctx.ui)
    }

    /// Polls a running action and re-reads the disks every `REFRESH_S` seconds, so attached and removed devices show up.
    fn tick(&mut self, _ctx: &Ctx, t: f64) -> bool {
        self.now = t;
        let finished = self.poll(t);
        if t - self.last_refresh >= REFRESH_S {
            self.refresh(t, false) || finished
        } else {
            finished
        }
    }

    fn next_wake(&self, t: f64) -> Option<f64> {
        if self.job.is_some() {
            return Some(t + 0.25);
        }
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

fn main() {
    bifrost_app::run(DiskUtility::new());
}
