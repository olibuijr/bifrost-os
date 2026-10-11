//! Shortcuts as an installed BifrOSt app. Actions that need the shell
//! (Wi-Fi, Low Power, DND, brightness, open app, screenshot, timer) cannot be
//! sent over the app protocol yet and are only logged.

mod icon;
mod model;
mod widget;

use bifrost_app::kit::{Damage, Event, Surface};
use bifrost_app::{App, Ctx, Input, TouchKind};

struct ShortcutsApp {
    w: Option<widget::Shortcuts>,
}

impl App for ShortcutsApp {
    fn id(&self) -> &str {
        "is.olibuijr.shortcuts.desktop"
    }

    fn configure(&mut self, ctx: &Ctx) {
        let (w, h) = (ctx.cfg.w as i32, ctx.cfg.h as i32);
        match &mut self.w {
            Some(c) => c.set_size(w, h),
            None => self.w = Some(widget::Shortcuts::new(&ctx.ui, w, h, ctx.data_dir.join("shortcuts.tsv"))),
        }
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        let (Some(c), Input::Touch { kind, x, y, t, .. }) = (&mut self.w, ev) else { return false };
        let e = match kind {
            TouchKind::Down => Event::Down(x, y),
            TouchKind::Move => Event::Move(x, y),
            TouchKind::Up | TouchKind::Cancel => Event::Up(x, y),
        };
        c.touch(&ctx.ui, e, t);
        if !c.take_runs().is_empty() {
            c.show_run_unavailable(t);
        }
        true
    }

    fn tick(&mut self, ctx: &Ctx, t: f64) -> bool {
        self.w.as_mut().is_some_and(|c| c.tick(&ctx.ui, t))
    }

    fn next_wake(&self, t: f64) -> Option<f64> {
        self.w.as_ref().and_then(|c| c.next_wake(t))
    }

    fn controls(&self, ctx: &Ctx) -> Vec<bifrost_app::controls::Node> {
        self.w.as_ref().map(|c| c.controls(&ctx.ui)).unwrap_or_default()
    }

    fn draw(&mut self, ctx: &Ctx, s: &mut Surface, t: f64) -> Damage {
        if let Some(c) = &mut self.w {
            c.draw(s, &ctx.ui, t);
        }
        let mut d = Damage::new(s.w, s.h);
        d.all();
        d
    }
}

fn main() {
    bifrost_app::run(ShortcutsApp { w: None });
}
