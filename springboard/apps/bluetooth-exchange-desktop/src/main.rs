//! Bluetooth File Exchange as an installed BifrOSt app: pick a paired or nearby Bluetooth device, send the files in
//! Downloads to it over OBEX object push, watch the progress, and list the files received. `model.rs` holds the state
//! and the rules, `transport.rs` talks to obexd on the session bus, `worker.rs` runs the bus and disk work on its own
//! thread, `window.rs` lays the window out and `draw.rs` paints it.

mod draw;
mod model;
mod transport;
mod window;
mod worker;

use bifrost_app::controls::Node;
use bifrost_app::kit::{Damage, Surface};
use bifrost_app::{App, Ctx, Input};
use bifrost_core::json::Json;
use window::{BluetoothExchange, Target};

impl App for BluetoothExchange {
    fn id(&self) -> &str {
        "is.olibuijr.bluetoothexchange.desktop"
    }

    fn configure(&mut self, ctx: &Ctx) {
        self.resize(ctx.cfg.w as f32, ctx.cfg.h as f32);
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        match ev {
            Input::Touch { kind, x, y, .. } => {
                let scale = ctx.ui.pt(1.0);
                self.touch(ctx, &ctx.ui, kind, x / scale, y / scale)
            }
            // a published control (accessibility, `bifrost pc midget tap-window`): the same action as a click
            Input::Activate(id) => self.activate(ctx, Target::Toolbar(id as u32)),
            _ => false,
        }
    }

    /// The refresh loop: takes the worker's replies, reads the folders and asks the shell for the radio on a timer.
    fn tick(&mut self, ctx: &Ctx, t: f64) -> bool {
        self.step(ctx, t)
    }

    fn next_wake(&self, t: f64) -> Option<f64> {
        Some(self.wake_at(t))
    }

    fn wake_fd(&self) -> Option<i32> {
        self.link.wake_fd()
    }

    fn service_reply(&mut self, ctx: &Ctx, request: u32, result: &Result<Json, String>) -> bool {
        self.on_service_reply(ctx, request, result)
    }

    fn service_event(&mut self, _ctx: &Ctx, service: &str, event: &str, body: &Json) -> bool {
        self.on_service_event(service, event, body)
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
    bifrost_app::run(BluetoothExchange::new());
}
