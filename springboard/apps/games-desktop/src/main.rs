//! Games (2048, Minesweeper, Solitaire) as an installed BifrOSt app: the former
//! in-shell Games hosted through the SDK, in a Golden Gate full-size window (source list and unified toolbar).

mod games;
mod solitaire;

use bifrost_app::kit::{Damage, Event, Surface};
use bifrost_app::controls::Node;
use bifrost_app::{App, Ctx, Input, TouchKind};

struct GamesApp {
    app: Option<games::Games>,
}

impl App for GamesApp {
    fn id(&self) -> &str {
        "is.olibuijr.games.desktop"
    }

    fn configure(&mut self, ctx: &Ctx) {
        let (w, h) = (ctx.cfg.w as i32, ctx.cfg.h as i32);
        match &mut self.app {
            Some(c) => c.set_size(w, h),
            None => {
                // Best scores are written when they are set, and a game in progress is not kept: nothing is unsaved.
                ctx.enable_sudden_termination();
                self.app = Some(games::Games::new(&ctx.ui, w, h).with_storage(ctx.data_dir.clone()));
            }
        }
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        let Some(c) = &mut self.app else { return false };
        let (kind, x, y, t) = match ev {
            Input::Touch { kind, x, y, t, .. } => (kind, x, y, t),
            Input::Activate(id) => {
                c.command(id as u32);
                return true;
            }
            _ => return false,
        };
        let e = match kind {
            TouchKind::Down => Event::Down(x, y),
            TouchKind::Move => Event::Move(x, y),
            TouchKind::Up | TouchKind::Cancel => Event::Up(x, y),
        };
        let _ = c.touch(&ctx.ui, e, t);
        true
    }

    fn controls(&self, ctx: &Ctx) -> Vec<Node> {
        self.app.as_ref().map_or(vec![], |c| c.controls(&ctx.ui))
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
    bifrost_app::run(GamesApp { app: None });
}
