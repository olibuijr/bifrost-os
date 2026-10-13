//! Wireless Diagnostics as an installed BifrOSt system app: the Wi-Fi link (network, channel, band, signal, noise, rate
//! and a quality rating), a live signal chart, a scan of nearby networks and a plain-text diagnostics report.
//! `source.rs` reads the kernel and `iw` on background threads, `model.rs` parses and rates what they return,
//! `state.rs` holds the window's state and layout, and `draw.rs` paints it.

mod draw;
mod model;
mod source;
mod state;

use bifrost_app::kit::{clipboard, Damage, Surface};
use bifrost_app::{App, Ctx, Input, TouchKind};
use source::Reading;
use state::{to_points, Diagnostics, B_COPY, B_REPORT, B_SCAN};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// How often the radio is read, in seconds.
const POLL: Duration = Duration::from_secs(1);
/// How often the window asks to be woken to take readings that arrived, in milliseconds.
const WAKE_MS: f64 = 500.0;

struct WirelessDiagnostics {
    state: Diagnostics,
    poll: Receiver<Reading>,
}

impl WirelessDiagnostics {
    fn new() -> WirelessDiagnostics {
        WirelessDiagnostics { state: Diagnostics::new(), poll: source::spawn_poller(POLL) }
    }

    /// Takes every reading the poller sent since the last frame, and the scan's answer if it has come back.
    fn drain(&mut self) -> bool {
        let mut changed = false;
        loop {
            match self.poll.try_recv() {
                Ok(reading) => {
                    self.state.apply_reading(reading);
                    changed = true;
                }
                Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => break,
            }
        }
        self.state.poll_scan() || changed
    }

    /// Runs the toolbar action `id`. True when the window needs a new frame.
    fn activate(&mut self, id: u32) -> bool {
        match id {
            B_SCAN => self.state.start_scan(),
            B_REPORT => {
                let unix = unix_now();
                self.state.message = Some(match self.state.save_report(unix) {
                    Ok(path) => format!("Report saved to {}", path.display()),
                    Err(e) => format!("Could not save the report: {e}"),
                });
                true
            }
            B_COPY => {
                clipboard::set_text(&self.state.report_text(unix_now()));
                self.state.message = Some("Summary copied to the clipboard".into());
                true
            }
            _ => false,
        }
    }
}

fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

impl App for WirelessDiagnostics {
    fn id(&self) -> &str {
        "is.olibuijr.wirelessdiagnostics.desktop"
    }

    fn configure(&mut self, ctx: &Ctx) {
        ctx.enable_sudden_termination(); // keeps no unsaved state: every reading is taken again from the radio
        self.state.size = (to_points(&ctx.ui, ctx.cfg.w as f32), to_points(&ctx.ui, ctx.cfg.h as f32));
        self.state.data_dir = ctx.data_dir.clone();
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        match ev {
            Input::Touch { kind: TouchKind::Up, x, y, .. } => {
                let (x, y) = (to_points(&ctx.ui, x), to_points(&ctx.ui, y));
                match self.state.toolbar_layout(&ctx.ui).hit(x, y) {
                    Some(id) => self.activate(id),
                    None => false,
                }
            }
            // a published control (accessibility, `bifrost pc midget tap-window`): the same action as a click
            Input::Activate(id) => self.activate(id as u32),
            _ => false,
        }
    }

    /// The refresh loop: takes the readings the poller has sent and the scan's answer.
    fn tick(&mut self, _ctx: &Ctx, _t: f64) -> bool {
        self.drain()
    }

    fn next_wake(&self, t: f64) -> Option<f64> {
        Some(t + WAKE_MS)
    }

    fn draw(&mut self, ctx: &Ctx, s: &mut Surface, _t: f64) -> Damage {
        draw::paint(&mut self.state, s, &ctx.ui);
        let mut d = Damage::new(s.w, s.h);
        d.all();
        d
    }
}

fn main() {
    bifrost_app::run(WirelessDiagnostics::new());
}
