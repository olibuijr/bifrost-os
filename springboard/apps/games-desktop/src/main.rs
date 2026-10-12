//! Games (2048, Minesweeper, Solitaire) as an installed BifrOSt app: the former
//! in-shell Games hosted through the SDK, in a Golden Gate full-size window (source list and unified toolbar).

mod games;
mod phase;
mod solitaire;
mod system;

use bifrost_app::kit::{Damage, Event, Surface};
use bifrost_app::controls::Node;
use bifrost_app::{App, Ctx, Input, TouchKind};
use bifrost_core::json::Json;
use system::{Act, Link};

struct GamesApp {
    app: Option<games::Games>,
    /// the shell's controller, power and display services, as the games see them
    link: Link,
}

impl GamesApp {
    /// Passes the power and display state on to the game (it draws less motion in Low Power Mode).
    fn sync(&mut self) {
        if let Some(c) = &mut self.app {
            c.set_reduced_effects(self.link.reduce_effects());
            c.set_display(self.link.display());
        }
    }

    /// Runs what a system event asked for. Returns true when the screen needs a frame.
    fn handle(&mut self, act: Option<Act>) -> bool {
        self.sync();
        let Some(act) = act else { return false };
        let Some(c) = &mut self.app else { return false };
        match act {
            Act::Slide(dir) => c.gamepad_slide(dir),
            Act::NewGame => c.gamepad_new_game(),
            Act::Notice(text) => c.set_notice(Some(text)),
            Act::State => {}
        }
        true
    }
}

impl App for GamesApp {
    fn id(&self) -> &str {
        "is.olibuijr.games.desktop"
    }

    fn configure(&mut self, ctx: &Ctx) {
        let (w, h) = (ctx.cfg.w as i32, ctx.cfg.h as i32);
        if let Some(c) = &mut self.app {
            c.set_size(w, h);
            return;
        }
        // Best scores are written when they are set, and a game in progress is not kept: nothing is unsaved.
        ctx.enable_sudden_termination();
        self.app = Some(games::Games::new(&ctx.ui, w, h).with_storage(ctx.data_dir.clone()));
        // once, when the app opens: the replies arrive in `service_reply`, the changes in `service_event`
        self.link.start(ctx);
        self.sync();
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

    fn service_reply(&mut self, _ctx: &Ctx, request: u32, result: &Result<Json, String>) -> bool {
        let taken = self.link.on_reply(request, result);
        if taken {
            self.sync();
        }
        taken
    }

    fn service_event(&mut self, _ctx: &Ctx, service: &str, event: &str, body: &Json) -> bool {
        let act = self.link.on_event(service, event, body);
        if act.is_none() {
            return false;
        }
        self.handle(act)
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
    bifrost_app::run(GamesApp { app: None, link: Link::new() });
}
