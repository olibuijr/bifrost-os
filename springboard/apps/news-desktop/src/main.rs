//! News as an installed BifrOSt app: a full-size window with a source list of feeds and a unified toolbar.

mod feed;
mod icon;
mod news;
mod pictures;
mod wire;

use bifrost_app::kit::{Damage, Event, Surface};
use bifrost_app::{App, Ctx, Input, TouchKind};
use std::sync::Arc;

struct NewsApp {
    news: Option<news::News>,
}

impl App for NewsApp {
    fn id(&self) -> &str {
        "is.olibuijr.news.desktop"
    }

    fn configure(&mut self, ctx: &Ctx) {
        let (w, h) = (ctx.cfg.w as i32, ctx.cfg.h as i32);
        match &mut self.news {
            Some(n) => n.set_size(w, h),
            None => {
                // the feed cache is rebuilt by the next fetch, so the app holds no unsaved state
                ctx.enable_sudden_termination();
                self.news = Some(news::News::new(&ctx.ui, w, h, ctx.data_dir.clone(), Arc::new(springboard::shell::music::radio::Http::default())));
            }
        }
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        let Some(n) = &mut self.news else { return false };
        let (kind, x, y, t) = match ev {
            Input::Touch { kind, x, y, t, .. } => (kind, x, y, t),
            Input::Activate(id) => return n.activate_control(&ctx.ui, id),
            _ => return false,
        };
        let e = match kind {
            TouchKind::Down => Event::Down(x, y),
            TouchKind::Move => Event::Move(x, y),
            TouchKind::Up | TouchKind::Cancel => Event::Up(x, y),
        };
        n.touch(&ctx.ui, e, t);
        true
    }

    fn tick(&mut self, ctx: &Ctx, t: f64) -> bool {
        self.news.as_mut().is_some_and(|n| {
            let pumped = n.pump();
            n.tick(&ctx.ui, t) | pumped
        })
    }

    fn controls(&self, ctx: &Ctx) -> Vec<bifrost_app::controls::Node> {
        self.news.as_ref().map(|n| n.controls(&ctx.ui)).unwrap_or_default()
    }

    fn full_redraw(&self) -> bool {
        true
    }

    fn next_wake(&self, t: f64) -> Option<f64> {
        self.news.as_ref().and_then(|n| n.next_wake(t))
    }

    fn draw(&mut self, ctx: &Ctx, s: &mut Surface, t: f64) -> Damage {
        if let Some(n) = &mut self.news {
            n.draw(s, &ctx.ui, t);
        }
        let mut d = Damage::new(s.w, s.h);
        d.all();
        d
    }
}

fn main() {
    bifrost_app::run(NewsApp { news: None });
}
