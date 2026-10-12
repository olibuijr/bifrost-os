//! Freeform as an installed BifrOSt app: the former in-shell Freeform,
//! unchanged, hosted through the SDK.

mod freeform;
mod icon;
mod ink;

use bifrost_app::kit::{Damage, Event, Surface};
use bifrost_app::{App, Ctx, Input, TouchKind};
use std::path::PathBuf;

struct FreeformApp {
    app: Option<freeform::Freeform>,
}

impl App for FreeformApp {
    fn id(&self) -> &str {
        "is.olibuijr.freeform.desktop"
    }

    fn configure(&mut self, ctx: &Ctx) {
        let (w, h) = (ctx.cfg.w as i32, ctx.cfg.h as i32);
        match &mut self.app {
            Some(c) => c.set_size(w, h),
            None => {
                self.app = Some(freeform::Freeform::new(&ctx.ui, w, h).with_save_dir(PathBuf::from(std::env::var("HOME").unwrap_or_default()).join("Pictures")))
            }
        }
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        if let (Some(c), Input::Activate(id)) = (&mut self.app, &ev) {
            return c.activate_id(&ctx.ui, *id);
        }
        let (Some(c), Input::Touch { kind, x, y, t, .. }) = (&mut self.app, ev) else { return false };
        let e = match kind {
            TouchKind::Down => Event::Down(x, y),
            TouchKind::Move => Event::Move(x, y),
            TouchKind::Up | TouchKind::Cancel => Event::Up(x, y),
        };
        c.touch(&ctx.ui, e, t);
        true
    }

    fn controls(&self, _ctx: &Ctx) -> Vec<bifrost_app::controls::Node> {
        self.app.as_ref().map(|c| c.controls()).unwrap_or_default()
    }

    fn tick(&mut self, ctx: &Ctx, t: f64) -> bool {
        self.app.as_mut().is_some_and(|c| c.tick(&ctx.ui, t))
    }

    fn next_wake(&self, t: f64) -> Option<f64> {
        self.app.as_ref().and_then(|c| c.next_wake(t))
    }

    fn draw(&mut self, ctx: &Ctx, s: &mut Surface, t: f64) -> Damage {
        if let Some(c) = &mut self.app {
            c.draw(s, &ctx.ui, t);
        }
        let mut d = Damage::new(s.w, s.h);
        d.all();
        d
    }
}

fn main() {
    bifrost_app::run(FreeformApp { app: None })
}
