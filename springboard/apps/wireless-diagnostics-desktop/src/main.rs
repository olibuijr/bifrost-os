//! Wireless Diagnostics as an installed BifrOSt system app: the Wi-Fi link (network, channel, band, signal, noise, rate
//! and a quality rating), a live signal chart, a scan of nearby networks and a plain-text diagnostics report.
//! `source.rs` reads the kernel and `iw` on dispatch queues, `model.rs` parses and rates what they return,
//! `state.rs` holds the window's state and layout, and `draw.rs` paints it.

mod draw;
mod model;
mod source;
mod state;

use bifrost_app::kit::{clipboard, Damage, Surface};
use bifrost_app::{App, Ctx, Input, TouchKind};
use state::{to_points, Diagnostics, B_COPY, B_REPORT, B_SCAN};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// How often the radio is read, in seconds.
const POLL: Duration = Duration::from_secs(1);
struct WirelessDiagnostics {
    state: Diagnostics,
    /// The poller, paused while the window is not visible. None under the compliance harness.
    poller: Option<std::sync::Arc<source::Poller>>,
}

impl WirelessDiagnostics {
    fn new() -> WirelessDiagnostics {
        let mut state = Diagnostics::new();
        let mut poller = None;
        if bifrost_app::testing::pinned() {
            // Under the compliance harness the radio is a fixed fixture, applied before the first frame: the snapshot
            // never depends on the host's Wi-Fi interfaces or on how many polls came back in time.
            for reading in source::fixture_readings() {
                state.apply_reading(reading);
            }
        } else {
            poller = Some(source::Poller::start(POLL, &state.readings));
        }
        WirelessDiagnostics { state, poller }
    }

    /// Takes every reading the poller sent since the last frame, and the scan's answer if it has come back.
    fn drain(&mut self) -> bool {
        self.state.poll_scan() | self.state.drain_readings()
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
    /// A single-window utility: the red light quits it (`applicationShouldTerminateAfterLastWindowClosed:`).
    fn terminates_after_last_window_closed(&self) -> bool {
        true
    }
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

    /// The window is hidden or minimised: no more `iw` runs until it is shown again.
    fn suspend(&mut self, _ctx: &Ctx) {
        if let Some(poller) = &self.poller {
            poller.pause();
        }
    }

    fn resume(&mut self, _ctx: &Ctx) {
        if let Some(poller) = &self.poller {
            poller.resume();
        }
    }

    /// The refresh loop: takes the readings the poller has sent and the scan's answer.
    fn tick(&mut self, _ctx: &Ctx, _t: f64) -> bool {
        self.drain()
    }

    fn next_wake(&self, _t: f64) -> Option<f64> {
        None
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
