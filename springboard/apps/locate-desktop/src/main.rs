//! Locate as an installed BifrOSt app: a Golden Gate window with the People, Devices, Items and Me sidebar, hosted
//! through the SDK. The first run shows what is shared; the marker file in the app's data folder remembers that it
//! was dismissed. Control methods (`locate.locate`, `locate.play_sound`, `locate.share`) are served from `window`.

mod surfaces;
mod sync;
mod window;

use bifrost_app::control::RpcError;
use bifrost_app::kit::{Damage, Surface};
use bifrost_app::{App, Ctx, Input, KeyInput, TouchKind};
use bifrost_core::json::Json;
use bifrost_locate::control::METHODS;
use std::path::Path;

/// The file in the data folder that records a dismissed first-run sheet.
const ONBOARDED_MARK: &str = "onboarded";

struct LocateApp {
    w: window::LocateWindow,
}

impl App for LocateApp {
    fn id(&self) -> &str {
        "is.olibuijr.locate.desktop"
    }

    fn configure(&mut self, ctx: &Ctx) {
        ctx.enable_sudden_termination(); // keeps no unsaved state
        let onboarded = ctx.data_dir.join(ONBOARDED_MARK).exists();
        self.w = window::LocateWindow::new(onboarded);
        self.w.keys.load(ctx);
        self.w.start_session(ctx);
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        let scale = ctx.ui.pt(1.0).max(0.001);
        let h = ctx.cfg.h as f32 / scale; // the sidebar is measured in points
        match ev {
            Input::Touch { kind: TouchKind::Down, x, y, .. } => {
                let (x, y) = (x / scale, y / scale); // touches arrive in pixels, the layout is in points
                if let Some(section) = self.w.section_at(x, y, h) {
                    self.w.panel.select(section);
                    return true;
                }
                self.w.tap(ctx, x, y)
            }
            Input::Key(KeyInput::Code(13)) if self.w.panel.needs_onboarding() => {
                self.w.panel.finish_onboarding();
                let _ = std::fs::write(ctx.data_dir.join(ONBOARDED_MARK), "1");
                true
            }
            _ => false,
        }
    }

    fn keychain(&mut self, _ctx: &Ctx, reply: bifrost_app::keychain::Reply) -> bool {
        self.w.on_keychain(&reply)
    }

    fn service_reply(&mut self, ctx: &Ctx, request: u32, result: &Result<bifrost_core::json::Json, String>) -> bool {
        self.w.service_reply(ctx, request, result)
    }

    fn tick(&mut self, ctx: &Ctx, t: f64) -> bool {
        self.w.tick(ctx, t)
    }

    fn next_wake(&self, t: f64) -> Option<f64> {
        self.w.next_wake(t)
    }

    fn wake_fd(&self) -> Option<i32> {
        self.w.link.as_ref().map(|l| l.fd())
    }

    fn rpc_methods(&self) -> Vec<&'static str> {
        METHODS.to_vec()
    }

    fn rpc(&mut self, ctx: Option<&Ctx>, _data_dir: &Path, method: &str, params: &Json) -> Result<Json, RpcError> {
        let reply = self.w.rpc(method, params)?;
        if let Some(ctx) = ctx {
            self.w.ensure_key(ctx);
        }
        Ok(reply)
    }

    fn draw(&mut self, ctx: &Ctx, s: &mut Surface, _t: f64) -> Damage {
        self.w.draw(&ctx.ui, s);
        let mut d = Damage::new(s.w, s.h);
        d.all();
        d
    }
}

fn main() {
    bifrost_app::run(LocateApp { w: window::LocateWindow::new(true) });
}
