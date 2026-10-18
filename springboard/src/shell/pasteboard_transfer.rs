//! Pasteboard transfer progress: how far a large paste has come, and when the progress panel should show.
//!
//! A paste that arrives quickly never shows a panel: the progress appears only once the transfer has run for
//! `SHOW_AFTER_MS`. The clipboard service reports each payload it handles (`report_begin` with its size, `report_done`
//! when it is complete), and the shell draws the panel at the top of the screen while one is still running. The
//! service handles a payload in one call today, so a transfer finishes in the same frame it starts and the panel does
//! not appear for it; a panel needs the service to hand bytes over in chunks.

use crate::gfx::Surface;
use crate::shell::ui::{Weight, MAC_CALLOUT_PT, WHITE};
use bifrost_render::gfx::pack;
use std::sync::Mutex;

/// The transfer the clipboard service is handling, if any.
static ACTIVE: Mutex<Option<Transfer>> = Mutex::new(None);

const TOP_PT: f32 = 56.0;
const PAD_PT: f32 = 12.0;
const BAR_W_PT: f32 = 220.0;
const BAR_H_PT: f32 = 4.0;
const FILL: u32 = pack(28, 28, 30, 235);
const TRACK: u32 = pack(255, 255, 255, 60);

/// The clipboard service starts handling a payload of `total` bytes at `t` (ms).
pub fn report_begin(total: u64, t: f64) {
    *ACTIVE.lock().unwrap_or_else(|e| e.into_inner()) = Some(Transfer::begin(total, t));
}

/// The clipboard service has finished the payload it reported.
pub fn report_done() {
    if let Some(x) = ACTIVE.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        x.finish();
    }
}

/// The transfer whose panel should be on screen at `t` (ms), if any.
pub fn showing(t: f64) -> Option<Transfer> {
    ACTIVE.lock().unwrap_or_else(|e| e.into_inner()).clone().filter(|x| x.shown(t))
}

/// A transfer must run this long (ms) before its progress panel shows, so quick pastes never flash one.
pub const SHOW_AFTER_MS: f64 = 600.0;

/// One paste being transferred.
#[derive(Clone, Debug, PartialEq)]
pub struct Transfer {
    total: u64,
    done: u64,
    started: f64,
    finished: bool,
}

impl Transfer {
    /// A transfer of `total` bytes that began at `t` (ms).
    pub fn begin(total: u64, t: f64) -> Self {
        Self { total, done: 0, started: t, finished: false }
    }

    /// `bytes` more arrived. Progress never passes the total.
    pub fn advance(&mut self, bytes: u64) {
        if !self.finished {
            self.done = self.done.saturating_add(bytes).min(self.total);
        }
    }

    /// The data is complete: the transfer is done whatever the byte count said.
    pub fn finish(&mut self) {
        self.finished = true;
        self.done = self.total;
    }

    pub fn finished(&self) -> bool {
        self.finished
    }

    /// Bytes received so far.
    pub fn done(&self) -> u64 {
        self.done
    }

    /// Progress from 0 to 1. An empty transfer is complete at once.
    pub fn fraction(&self) -> f32 {
        if self.total == 0 {
            1.0
        } else {
            self.done as f32 / self.total as f32
        }
    }

    /// Whether the progress panel should be on screen at `t` (ms): the transfer is still running and has run long enough.
    pub fn shown(&self, t: f64) -> bool {
        !self.finished && t - self.started >= SHOW_AFTER_MS
    }

    /// The panel's text, such as "Copying 4.2 MB of 12.0 MB".
    pub fn label(&self) -> String {
        format!("Copying {} of {}", size_label(self.done), size_label(self.total))
    }
}

impl super::Shell {
    /// Draws the progress panel at the top centre while a payload is still arriving.
    pub(super) fn draw_paste_progress(&self, s: &mut Surface, t: f64) {
        let Some(x) = showing(t) else { return };
        let img = self.ui.text(&x.label(), MAC_CALLOUT_PT, Weight::Regular, WHITE);
        let pad = self.ui.pt(PAD_PT);
        let bar_w = self.ui.pt(BAR_W_PT);
        let bar_h = self.ui.pt(BAR_H_PT);
        let w = (img.w as f32).max(bar_w) + 2.0 * pad;
        let h = img.h as f32 + bar_h + 3.0 * pad;
        let left = ((self.w as f32 - w) / 2.0).round();
        let top = self.ui.pt(TOP_PT);
        s.fill_rrect(left, top, w, h, h / 2.0, FILL);
        s.blit_colored(&img, (left + pad) as i32, (top + pad) as i32, WHITE, 255);
        let bar_y = top + pad + img.h as f32 + pad;
        let bar_x = left + (w - bar_w) / 2.0;
        s.fill_rrect(bar_x, bar_y, bar_w, bar_h, bar_h / 2.0, TRACK);
        let filled = (bar_w * x.fraction()).round();
        if filled > 0.0 {
            s.fill_rrect(bar_x, bar_y, filled, bar_h, bar_h / 2.0, WHITE);
        }
    }
}

/// A byte count in decimal units, as the panel shows it: "512 bytes", "4.2 MB", "1.3 GB".
pub fn size_label(bytes: u64) -> String {
    const UNITS: [(&str, u64); 3] = [("GB", 1_000_000_000), ("MB", 1_000_000), ("KB", 1_000)];
    for (unit, scale) in UNITS {
        if bytes >= scale {
            return format!("{:.1} {unit}", bytes as f64 / scale as f64);
        }
    }
    format!("{bytes} bytes")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_advances_and_never_passes_the_total() {
        let mut x = Transfer::begin(100, 0.0);
        x.advance(40);
        assert!((x.fraction() - 0.4).abs() < 1e-6);
        x.advance(1_000);
        assert_eq!(x.done(), 100);
        assert!((x.fraction() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn an_empty_transfer_is_complete_at_once() {
        let x = Transfer::begin(0, 0.0);
        assert_eq!(x.fraction(), 1.0);
    }

    #[test]
    fn the_panel_waits_for_the_delay_and_hides_when_finished() {
        let mut x = Transfer::begin(1_000_000, 0.0);
        assert!(!x.shown(SHOW_AFTER_MS - 1.0));
        assert!(x.shown(SHOW_AFTER_MS));
        x.finish();
        assert!(x.finished());
        assert!(!x.shown(10_000.0));
        assert_eq!(x.done(), 1_000_000);
    }

    #[test]
    fn advancing_after_finish_changes_nothing() {
        let mut x = Transfer::begin(10, 0.0);
        x.finish();
        x.advance(5);
        assert_eq!(x.done(), 10);
    }

    #[test]
    fn sizes_read_in_decimal_units() {
        assert_eq!(size_label(512), "512 bytes");
        assert_eq!(size_label(4_200_000), "4.2 MB");
        assert_eq!(size_label(1_300_000_000), "1.3 GB");
        assert_eq!(size_label(2_000), "2.0 KB");
    }

    #[test]
    fn the_label_reads_done_of_total() {
        let mut x = Transfer::begin(12_000_000, 0.0);
        x.advance(4_200_000);
        assert_eq!(x.label(), "Copying 4.2 MB of 12.0 MB");
    }
}
