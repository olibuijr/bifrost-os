//! Tips as an installed BifrOSt app: a library of short how-to cards about the system, with search, collections and
//! New markers for the cards not read yet. Hosted through the SDK in a full-size window.

mod app;
mod catalogue;
mod model;
mod store;

use app::Tips;
use bifrost_app::controls::Node;
use bifrost_app::kit::{Damage, Surface};
use bifrost_app::{App, Ctx, Input};

struct TipsApp {
    tips: Option<Tips>,
}

impl App for TipsApp {
    fn id(&self) -> &str {
        "is.olibuijr.tips.desktop"
    }

    fn configure(&mut self, ctx: &Ctx) {
        let (w, h) = (ctx.cfg.w as i32, ctx.cfg.h as i32);
        match &mut self.tips {
            Some(tips) => tips.set_size(w, h),
            None => {
                // nothing is kept in memory that is not already saved (the read state is written on every change)
                ctx.enable_sudden_termination();
                self.tips = Some(Tips::new(ctx.data_dir.clone(), w, h));
            }
        }
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        let Some(tips) = &mut self.tips else { return false };
        match ev {
            Input::Touch { kind, x, y, t, .. } => tips.touch(ctx, kind, x, y, t),
            Input::Key(key) => tips.key(ctx, key),
            _ => false,
        }
    }

    fn tick(&mut self, _ctx: &Ctx, t: f64) -> bool {
        self.tips.as_mut().is_some_and(|tips| tips.tick(t))
    }

    fn controls(&self, ctx: &Ctx) -> Vec<Node> {
        self.tips.as_ref().map(|tips| tips.controls(&ctx.ui)).unwrap_or_default()
    }

    fn draw(&mut self, ctx: &Ctx, s: &mut Surface, _t: f64) -> Damage {
        if let Some(tips) = &mut self.tips {
            tips.draw(&ctx.ui, s);
        }
        let mut damage = Damage::new(s.w, s.h);
        damage.all();
        damage
    }
}

fn main() {
    bifrost_app::run(TipsApp { tips: None });
}
