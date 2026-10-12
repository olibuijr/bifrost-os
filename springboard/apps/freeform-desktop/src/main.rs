//! Freeform as an installed BifrOSt app: the former in-shell Freeform,
//! unchanged, hosted through the SDK. It also talks to the shell's services: the cursor (the
//! pointer shape of each tool), the pasteboard (Copy) and the screens (the export scale).

mod freeform;
mod icon;
mod ink;
mod services;

use bifrost_app::kit::{Damage, Event, Surface};
use bifrost_app::{App, Ctx, Input, TouchKind};
use bifrost_core::json::Json;
use std::path::PathBuf;

struct FreeformApp {
    app: Option<freeform::Freeform>,
    services: services::Services,
}

impl FreeformApp {
    /// Shows the pointer shape of the point (x, y) (a service call only when the shape changes).
    fn show_pointer(&mut self, ctx: &Ctx, x: f32, y: f32) {
        let Some(c) = &self.app else { return };
        let kind = c.pointer_for(&ctx.ui, x, y);
        self.services.show_pointer(ctx, kind);
    }

    /// Puts a Copy's strokes on the pasteboard.
    fn drain(&mut self, ctx: &Ctx) {
        let Some(c) = &mut self.app else { return };
        if let Some(strokes) = c.take_copy() {
            self.services.copy(ctx, &strokes, ctx.ui.s);
        }
    }

    /// Gives the board the export factor of the main display, once the shell has said what it is.
    fn sync_export(&mut self, ctx: &Ctx) {
        if let (Some(c), Some(backing)) = (&mut self.app, self.services.backing()) {
            c.set_export_scale(ink::export_factor(backing, ctx.ui.s));
        }
    }
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
        self.services.configure(ctx);
        self.sync_export(ctx);
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        let (frame, pointer) = match ev {
            Input::Activate(id) => match &mut self.app {
                Some(c) => (c.activate_id(&ctx.ui, id), None),
                None => return false,
            },
            Input::Touch { kind, x, y, t, .. } => {
                let Some(c) = &mut self.app else { return false };
                let e = match kind {
                    TouchKind::Down => Event::Down(x, y),
                    TouchKind::Move => Event::Move(x, y),
                    TouchKind::Up | TouchKind::Cancel => Event::Up(x, y),
                };
                c.touch(&ctx.ui, e, t);
                (true, Some((x, y)))
            }
            _ => return false,
        };
        if let Some((x, y)) = pointer {
            self.show_pointer(ctx, x, y);
        }
        self.drain(ctx);
        frame
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

    fn suspend(&mut self, ctx: &Ctx) {
        self.services.suspend(ctx);
    }

    fn service_reply(&mut self, ctx: &Ctx, request: u32, result: &Result<Json, String>) -> bool {
        let took = self.services.reply(ctx, request, result);
        self.sync_export(ctx);
        took
    }

    fn service_event(&mut self, ctx: &Ctx, service: &str, event: &str, body: &Json) -> bool {
        self.services.event(ctx, service, event, body);
        self.sync_export(ctx);
        false
    }
}

fn main() {
    bifrost_app::run(FreeformApp { app: None, services: Default::default() })
}
