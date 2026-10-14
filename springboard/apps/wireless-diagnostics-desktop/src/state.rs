//! The window's state and layout: the latest reading, the signal history, the scan results, the toolbar actions and the
//! footer text. Layout works in points, so it is the same at any scale; `draw.rs` paints what `layout` returns.

use crate::model::{self, History, Link, Rating};
use crate::source::{Reading, ScanOutcome};
use bifrost_app::controls::nav::{Toolbar, ToolbarItem, ToolbarLayout};
use bifrost_app::controls::Rect;
use bifrost_app::kit::{TextCache, Ui};
use bifrost_app::layout::UNIFIED_BAR_PT;
use bifrost_app::wlan::{self, Network};
use bifrost_app::widgets;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError};

pub const B_SCAN: u32 = 1;
pub const B_REPORT: u32 = 2;
pub const B_COPY: u32 = 3;

/// Outer margin and the gap between cards (points).
pub const MARGIN_PT: f32 = 16.0;
pub const GAP_PT: f32 = 12.0;
/// The height of the footer line under the cards (points).
pub const FOOTER_PT: f32 = 28.0;
/// The height of the two top cards (points): a title and eleven rows in the Connection card.
pub const TOP_CARD_PT: f32 = 290.0;
/// The height of one row in a card (points).
pub const ROW_PT: f32 = 22.0;

/// The state of the last scan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScanState {
    Idle,
    Scanning,
    Done { cached: bool },
    Failed(String),
}

/// The rectangles of the window, in points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    pub bar: Rect,
    pub connection: Rect,
    pub chart: Rect,
    pub networks: Rect,
    pub footer: Rect,
}

pub struct Diagnostics {
    /// The window size in points.
    pub size: (f32, f32),
    /// Where a saved report goes.
    pub data_dir: PathBuf,
    pub labels: TextCache,
    pub reading: Option<Reading>,
    pub history: History,
    pub networks: Vec<Network>,
    pub scan: ScanState,
    scan_rx: Option<Receiver<Result<ScanOutcome, String>>>,
    /// The last thing an action said (a saved report, a copy, an error), shown in the footer until the next action.
    pub message: Option<String>,
}

impl Diagnostics {
    pub fn new() -> Diagnostics {
        Diagnostics {
            size: (1200.0, 780.0),
            data_dir: PathBuf::from("."),
            labels: TextCache::default(),
            reading: None,
            history: History::default(),
            networks: Vec::new(),
            scan: ScanState::Idle,
            scan_rx: None,
            message: None,
        }
    }

    /// The link of the latest reading (disconnected until the first reading arrives).
    pub fn link(&self) -> Link {
        self.reading.as_ref().map(|r| r.link.clone()).unwrap_or_default()
    }

    /// The noise floor of the latest reading.
    pub fn noise(&self) -> Option<i32> {
        let reading = self.reading.as_ref()?;
        reading.ifaces.iter().find(|i| i.name == reading.iface).and_then(|i| i.noise_dbm)
    }

    /// The signal quality rating of the latest reading.
    pub fn rating(&self) -> Rating {
        model::rate_link(self.link().signal_dbm, self.noise())
    }

    /// Takes a poll's reading, and adds its signal to the chart.
    pub fn apply_reading(&mut self, reading: Reading) {
        self.history.push(reading.link.signal_dbm);
        self.reading = Some(reading);
    }

    /// Takes the scan's answer, if it has come back. True when the scan state changed.
    pub fn poll_scan(&mut self) -> bool {
        let Some(rx) = self.scan_rx.as_ref() else { return false };
        let outcome = match rx.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return false,
            Err(TryRecvError::Disconnected) => Err("Scan stopped unexpectedly".to_string()),
        };
        self.scan_rx = None;
        self.apply_scan(outcome);
        true
    }

    /// Records a scan's answer: the networks merged by name, strongest first.
    pub fn apply_scan(&mut self, result: Result<ScanOutcome, String>) {
        match result {
            Ok(outcome) => {
                self.networks = wlan::merge(&outcome.samples);
                self.scan = ScanState::Done { cached: outcome.cached };
            }
            Err(message) => self.scan = ScanState::Failed(message),
        }
    }

    /// Starts a scan on the interface of the latest reading. A scan already running is kept.
    pub fn start_scan(&mut self) -> bool {
        if self.scan_rx.is_some() {
            return false;
        }
        let iface = self.reading.as_ref().map_or_else(|| "wlan0".to_string(), |r| r.iface.clone());
        self.scan_rx = Some(crate::source::spawn_scan(iface));
        self.scan = ScanState::Scanning;
        self.message = None;
        true
    }

    /// The toolbar: Scan, Save Report and Copy Summary. Scan is greyed while a scan runs.
    pub fn toolbar(&self) -> Toolbar {
        Toolbar::new(vec![
            ToolbarItem::button(B_SCAN, "Scan").enabled(self.scan != ScanState::Scanning),
            ToolbarItem::button(B_REPORT, "Save Report"),
            ToolbarItem::button(B_COPY, "Copy Summary"),
        ])
    }

    pub fn toolbar_layout(&self, ui: &Ui) -> ToolbarLayout {
        widgets::toolbar_layout(ui, self.layout().bar, &self.toolbar())
    }

    /// The diagnostics report as it reads now.
    pub fn report_text(&self, unix: u64) -> String {
        let iface = self.reading.as_ref().map_or("", |r| r.iface.as_str());
        model::report(unix, iface, &self.link(), self.noise(), &self.networks, &self.history)
    }

    /// Writes the report into the data folder and returns its path.
    pub fn save_report(&self, unix: u64) -> std::io::Result<PathBuf> {
        let dir = self.data_dir.join("reports");
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(model::report_file_name(unix));
        std::fs::write(&path, self.report_text(unix))?;
        Ok(path)
    }

    /// The footer: the action's message, else the reason the link cannot be read, else the scan state, else the link.
    pub fn footer_text(&self) -> String {
        if let Some(message) = &self.message {
            return message.clone();
        }
        if let Some(note) = self.reading.as_ref().and_then(|r| r.note.as_ref()) {
            return note.clone();
        }
        match &self.scan {
            ScanState::Scanning => return "Scanning for networks".into(),
            ScanState::Failed(message) => return message.clone(),
            ScanState::Done { cached: true } => return "Showing the results of the last scan".into(),
            _ => {}
        }
        let link = self.link();
        if link.connected {
            format!("Connected to {}", if link.ssid.is_empty() { "a hidden network" } else { link.ssid.as_str() })
        } else {
            "Not connected".into()
        }
    }

    /// The window's rectangles for the current size. The two top cards split the width; the networks card takes the rest.
    pub fn layout(&self) -> Layout {
        let (w, h) = self.size;
        let bar = Rect { x: 0.0, y: 0.0, w, h: UNIFIED_BAR_PT };
        let footer = Rect { x: 0.0, y: (h - FOOTER_PT).max(0.0), w, h: FOOTER_PT };
        let body_top = bar.h + MARGIN_PT;
        let body_bottom = footer.y - MARGIN_PT / 2.0;
        let top_h = TOP_CARD_PT.min(((body_bottom - body_top) * 0.6).max(0.0));
        let inner_w = (w - 2.0 * MARGIN_PT - GAP_PT).max(0.0);
        let connection_w = (inner_w * 0.42).round();
        let connection = Rect { x: MARGIN_PT, y: body_top, w: connection_w, h: top_h };
        let chart = Rect { x: MARGIN_PT + connection_w + GAP_PT, y: body_top, w: (inner_w - connection_w).max(0.0), h: top_h };
        let nets_y = body_top + top_h + GAP_PT;
        let networks = Rect { x: MARGIN_PT, y: nets_y, w: (w - 2.0 * MARGIN_PT).max(0.0), h: (body_bottom - nets_y).max(0.0) };
        Layout { bar, connection, chart, networks, footer }
    }
}

/// Maps a point in the surface (in pixels) to points.
pub fn to_points(ui: &Ui, px: f32) -> f32 {
    px / ui.pt(1.0).max(f32::EPSILON)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_app::wlan::Sample;

    fn reading(signal: Option<i32>, noise: Option<i32>) -> Reading {
        use crate::model::Iface;
        Reading {
            ifaces: vec![Iface { name: "wlan0".into(), quality: Some(60), level_dbm: signal, noise_dbm: noise }],
            iface: "wlan0".into(),
            link: Link { connected: true, ssid: "Home".into(), signal_dbm: signal, ..Link::default() },
            note: None,
        }
    }

    #[test]
    fn readings_feed_the_chart_and_the_rating() {
        let mut d = Diagnostics::new();
        assert_eq!(d.rating(), Rating::Unknown);
        d.apply_reading(reading(Some(-45), Some(-95)));
        d.apply_reading(reading(None, None));
        assert_eq!(d.history.len(), 2);
        assert_eq!(d.history.latest(), Some(-45));
        assert_eq!(d.noise(), None, "noise is from the latest reading, not a stale sample");
        assert_eq!(d.rating(), Rating::Unknown, "no current signal has no current rating");
    }

    #[test]
    fn footer_explains_the_state_in_order() {
        let mut d = Diagnostics::new();
        assert_eq!(d.footer_text(), "Not connected");
        d.apply_reading(reading(Some(-60), Some(-92)));
        assert_eq!(d.footer_text(), "Connected to Home");
        d.apply_scan(Ok(ScanOutcome { samples: vec![], cached: true }));
        assert_eq!(d.footer_text(), "Showing the results of the last scan");
        d.apply_scan(Err("Scan failed: busy".into()));
        assert_eq!(d.footer_text(), "Scan failed: busy");
        d.message = Some("Report saved".into());
        assert_eq!(d.footer_text(), "Report saved");
        d.message = None;
        let mut no_iw = reading(None, None);
        no_iw.note = Some("iw could not run".into());
        d.apply_reading(no_iw);
        assert_eq!(d.footer_text(), "iw could not run");
    }

    #[test]
    fn scan_results_merge_by_network() {
        let mut d = Diagnostics::new();
        let sample = |bssid: &str, rssi: i32| Sample {
            bssid: bssid.into(),
            freq: 2437,
            rssi,
            flags: "[WPA2-PSK]".into(),
            ssid: "Cafe".into(),
        };
        d.apply_scan(Ok(ScanOutcome { samples: vec![sample("a", -60), sample("b", -50)], cached: false }));
        assert_eq!(d.networks.len(), 1);
        assert_eq!(d.networks[0].bssids, vec!["b".to_string(), "a".to_string()]);
        assert_eq!(d.scan, ScanState::Done { cached: false });
    }

    #[test]
    fn scan_button_is_greyed_while_scanning() {
        let mut d = Diagnostics::new();
        assert!(d.toolbar().items[0].enabled);
        d.scan = ScanState::Scanning;
        assert!(!d.toolbar().items[0].enabled);
    }

    #[test]
    fn layout_fits_the_window_without_overlap() {
        let mut d = Diagnostics::new();
        for size in [(1200.0, 780.0), (800.0, 600.0), (2000.0, 1000.0)] {
            d.size = size;
            let l = d.layout();
            assert!(l.connection.x >= 0.0 && l.chart.x + l.chart.w <= size.0 + 0.01);
            assert!(l.connection.x + l.connection.w <= l.chart.x, "cards overlap at {size:?}");
            assert!(l.networks.y >= l.connection.y + l.connection.h);
            assert!(l.networks.y + l.networks.h <= l.footer.y + 0.01);
            assert_eq!(l.footer.y + l.footer.h, size.1);
        }
    }
}
