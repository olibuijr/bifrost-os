//! Pasteboard transfer progress: how far a large paste has come, and when the progress panel should show.
//!
//! Pure state. A paste that arrives quickly never shows a panel: the progress appears only once the transfer has run
//! for `SHOW_AFTER_MS`. The caller starts a transfer with the item's size, advances it with the bytes received, and
//! finishes it when the data is complete.

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
