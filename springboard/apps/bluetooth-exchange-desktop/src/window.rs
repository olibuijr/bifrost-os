//! Bluetooth File Exchange's window state and geometry: the toolbar, the devices on the left (from the shell's Bluetooth
//! service), and on the right the files to send, the transfers with their progress, and the files received, with the
//! footer under them. Layout is in points and derives from the window size; the drawing and the taps read the same
//! rectangles. Disk and bus work go through `worker.rs`; the rules are in `model.rs`.

use crate::model::{self, Model};
use crate::worker::{Link, Reply, Request};
use bifrost_app::bluetooth_service::{self, Bluetooth, RadioState};
use bifrost_app::controls::nav::{Toolbar, ToolbarItem, ToolbarLayout};
use bifrost_app::controls::Rect;
use bifrost_app::kit::{TextCache, Ui};
use bifrost_app::layout::UNIFIED_BAR_PT;
use bifrost_app::{widgets, Ctx, TouchKind};
use bifrost_core::bt_address::BdAddr;
use bifrost_core::json::Json;
use std::path::PathBuf;

/// The device list's width (points).
pub const SIDE_PT: f32 = 220.0;
/// One device row (points).
pub const DEV_ROW_PT: f32 = 44.0;
/// The gap above the first device row (points).
const LIST_PAD_PT: f32 = 8.0;
/// The detail pane's header: the selected device's name (points).
pub const HEAD_PT: f32 = 44.0;
/// A small section label above each list (points).
pub const LABEL_PT: f32 = 22.0;
/// One file or transfer row (points).
pub const ROW_PT: f32 = 26.0;
/// Padding around the detail pane's rows (points).
pub const PAD_PT: f32 = 8.0;
/// The footer (points).
pub const FOOT_PT: f32 = 30.0;
/// How many files to send are shown (the rest need the window to be taller; the label says how many there are).
pub const SEND_ROWS: usize = 8;
/// How many transfers are shown: the newest ones.
pub const TRANSFER_ROWS: usize = 4;
/// How often the folders and the radio are read again while the window is open (milliseconds).
pub const POLL_MS: f64 = 5000.0;
/// How soon the loop comes back while a transfer is running (milliseconds).
const BUSY_WAKE_MS: f64 = 250.0;

pub const B_REFRESH: u32 = 1;
pub const B_SCAN: u32 = 2;
pub const B_PAIR: u32 = 3;
pub const B_CLEAR: u32 = 4;

/// What a press or a tap landed on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Nothing,
    Toolbar(u32),
    /// The device at this index of the list, paired devices first.
    Device(usize),
    /// The file at this index of the files to send.
    Send(usize),
    /// The transfer at this index of the queue (see `transfer_window_start`).
    Transfer(usize),
}

pub struct BluetoothExchange {
    pub model: Model,
    link: Link,
    /// When the folders and the radio are read next (loop milliseconds).
    next_poll: f64,
    /// The window size, in points.
    pub size: (f32, f32),
    pub labels: TextCache,
    /// The target a press started on; the release acts only when it lands on the same one.
    pub press: Target,
    /// Where Downloads and its Bluetooth folder are (the person's home folder).
    pub home: PathBuf,
    /// The shell's Bluetooth service as this app sees it: the radio, the devices and the connect events.
    pub bt: Bluetooth,
    /// The device the next file goes to.
    pub selected: Option<BdAddr>,
    /// The shell request ids this app is waiting for.
    status_request: Option<u32>,
    pair_request: Option<u32>,
}

impl BluetoothExchange {
    pub fn new() -> BluetoothExchange {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
        BluetoothExchange {
            model: Model::default(),
            link: Link::start(),
            next_poll: 0.0,
            // The app host configures the actual client size before the first frame.
            size: (0.0, 0.0),
            labels: TextCache::default(),
            press: Target::Nothing,
            home,
            bt: Bluetooth::new(),
            selected: None,
            status_request: None,
            pair_request: None,
        }
    }

    pub fn resize(&mut self, w: f32, h: f32) {
        self.size = (w, h);
    }

    /// Reads the folders and asks the shell for the radio state and the devices.
    fn refresh(&mut self, ctx: &Ctx) {
        self.link.send(Request::List { downloads: model::outbox_dir(&self.home), received: model::inbox_dir(&self.home) });
        self.status_request = Some(bluetooth_service::status(ctx));
    }

    /// Starts the oldest waiting transfer when nothing is sending.
    fn pump(&mut self) {
        let Some(id) = self.model.next_waiting() else { return };
        let Some(t) = self.model.transfer(id) else { return };
        let request = Request::Send { id, address: t.device.to_string(), path: t.file.path.clone() };
        self.model.start(id);
        self.link.send(request);
    }

    /// Takes the replies that arrived and starts the next transfer. True when a frame is needed.
    fn poll(&mut self) -> bool {
        let replies = self.link.drain();
        if replies.is_empty() {
            return false;
        }
        for reply in replies {
            self.apply(reply);
        }
        self.pump();
        true
    }

    fn apply(&mut self, reply: Reply) {
        match reply {
            Reply::Listed { outbox, inbox } => self.model.set_listing(outbox, inbox),
            Reply::Progress { id, sent } => self.model.progress(id, sent),
            Reply::Done { id } => {
                if let Some(name) = self.model.transfer(id).map(|t| t.file.name.clone()) {
                    self.model.status = format!("Sent {name}");
                }
                self.model.finish(id, Ok(()));
            }
            Reply::Failed { id, text } => {
                self.model.status = text.clone();
                self.model.finish(id, Err(text));
            }
            Reply::Cancelled { id } => self.model.cancelled(id),
        }
    }

    /// One loop turn in the foreground: takes the replies and reads the folders and the radio every `POLL_MS`. True
    /// when a frame is needed.
    pub fn step(&mut self, ctx: &Ctx, t: f64) -> bool {
        let changed = self.poll();
        if t >= self.next_poll {
            self.next_poll = t + POLL_MS;
            self.refresh(ctx);
        }
        changed
    }

    /// When the loop should come back: soon while a transfer runs, else at the next reading.
    pub fn wake_at(&self, t: f64) -> f64 {
        if self.model.busy() {
            t + BUSY_WAKE_MS
        } else {
            self.next_poll.max(t + BUSY_WAKE_MS)
        }
    }

    /// A reply to one of the shell's Bluetooth requests. True when a frame is needed.
    pub fn on_service_reply(&mut self, ctx: &Ctx, request: u32, result: &Result<Json, String>) -> bool {
        if self.status_request == Some(request) {
            self.status_request = None;
            if let Ok(body) = result {
                let _ = self.bt.apply_status(body);
            }
            return true;
        }
        if self.pair_request == Some(request) {
            self.pair_request = None;
            self.model.status = match result {
                Ok(_) => {
                    self.status_request = Some(bluetooth_service::status(ctx));
                    "Pairing finished".to_string()
                }
                Err(text) => text.clone(),
            };
            return true;
        }
        false
    }

    /// A shell event for the Bluetooth service (a device connected or disconnected). True when a frame is needed.
    pub fn on_service_event(&mut self, service: &str, event: &str, body: &Json) -> bool {
        if service != bluetooth_service::SERVICE {
            return false;
        }
        let changed = self.bt.apply_event(event, body);
        let _ = self.bt.take_events();
        changed
    }

    pub fn bar_rect(&self) -> Rect {
        Rect { x: 0.0, y: 0.0, w: self.size.0, h: UNIFIED_BAR_PT }
    }

    fn body_h(&self) -> f32 {
        (self.size.1 - UNIFIED_BAR_PT - FOOT_PT).max(0.0)
    }

    /// The bottom of the body, above the footer.
    fn bottom(&self) -> f32 {
        UNIFIED_BAR_PT + self.body_h()
    }

    /// The device list, under the toolbar on the left.
    pub fn side_rect(&self) -> Rect {
        Rect { x: 0.0, y: UNIFIED_BAR_PT, w: SIDE_PT.min(self.size.0), h: self.body_h() }
    }

    /// The detail pane, on the right of the device list.
    pub fn detail_rect(&self) -> Rect {
        let x = SIDE_PT.min(self.size.0);
        Rect { x, y: UNIFIED_BAR_PT, w: (self.size.0 - x).max(0.0), h: self.body_h() }
    }

    pub fn footer_rect(&self) -> Rect {
        Rect { x: 0.0, y: (self.size.1 - FOOT_PT).max(0.0), w: self.size.0, h: FOOT_PT }
    }

    pub fn device_row(&self, i: usize) -> Rect {
        Rect { x: 0.0, y: UNIFIED_BAR_PT + LIST_PAD_PT + i as f32 * DEV_ROW_PT, w: SIDE_PT, h: DEV_ROW_PT }
    }

    /// The top of the files-to-send section, under the header.
    pub fn rows_top(&self) -> f32 {
        UNIFIED_BAR_PT + HEAD_PT
    }

    /// The top of the transfers section, under the files to send.
    pub fn transfers_top(&self) -> f32 {
        self.rows_top() + LABEL_PT + SEND_ROWS as f32 * ROW_PT
    }

    /// The top of the received section, under the transfers.
    pub fn received_top(&self) -> f32 {
        self.transfers_top() + LABEL_PT + TRANSFER_ROWS as f32 * ROW_PT
    }

    /// How many received files fit above the footer.
    pub fn received_capacity(&self) -> usize {
        ((self.bottom() - self.received_top() - LABEL_PT) / ROW_PT).max(0.0) as usize
    }

    /// The first transfer shown: the queue's newest `TRANSFER_ROWS`.
    pub fn transfer_window_start(&self) -> usize {
        self.model.transfers.len().saturating_sub(TRANSFER_ROWS)
    }

    fn row(&self, y: f32) -> Rect {
        let d = self.detail_rect();
        Rect { x: d.x + PAD_PT, y, w: (d.w - 2.0 * PAD_PT).max(0.0), h: ROW_PT }
    }

    /// The row of file `i` to send.
    pub fn send_row(&self, i: usize) -> Rect {
        self.row(self.rows_top() + LABEL_PT + i as f32 * ROW_PT)
    }

    /// The row of transfer `k` of the shown ones (k 0 is `transfer_window_start`).
    pub fn transfer_row(&self, k: usize) -> Rect {
        self.row(self.transfers_top() + LABEL_PT + k as f32 * ROW_PT)
    }

    /// The row of received file `k`.
    pub fn received_row(&self, k: usize) -> Rect {
        self.row(self.received_top() + LABEL_PT + k as f32 * ROW_PT)
    }

    pub fn toolbar(&self) -> Toolbar {
        let on = self.bt.state == RadioState::On;
        Toolbar::new(vec![
            ToolbarItem::button(B_REFRESH, "Refresh"),
            ToolbarItem::button(B_SCAN, "Scan").enabled(on && !self.bt.scanning),
            ToolbarItem::button(B_PAIR, "Pair").enabled(on && self.selected_unpaired() && self.bt.busy.is_none()),
            ToolbarItem::button(B_CLEAR, "Clear Finished").enabled(self.model.any_finished()),
        ])
    }

    pub fn toolbar_layout(&self, ui: &Ui) -> ToolbarLayout {
        widgets::toolbar_layout(ui, self.bar_rect(), &self.toolbar())
    }

    /// The selected device's name, when it is in the list.
    pub fn selected_name(&self) -> Option<String> {
        self.selected.and_then(|a| self.bt.devices.get(a)).map(|d| d.label())
    }

    fn selected_unpaired(&self) -> bool {
        self.selected.and_then(|a| self.bt.devices.get(a)).is_some_and(|d| !d.paired)
    }

    /// The footer's words: the last message, or what the selection needs.
    pub fn footer_text(&self) -> String {
        if !self.model.status.is_empty() {
            return self.model.status.clone();
        }
        match self.bt.state {
            RadioState::Unavailable => "This computer has no Bluetooth".into(),
            RadioState::Off => "Turn Bluetooth on to send files".into(),
            RadioState::On => match self.selected_name() {
                Some(name) => format!("Sending to {name}"),
                None => "Choose a device in the list".into(),
            },
        }
    }

    /// What is under the point `(x, y)`, in points.
    pub fn hit(&self, ui: &Ui, x: f32, y: f32) -> Target {
        if y < UNIFIED_BAR_PT {
            return self.hit_toolbar(ui, x, y);
        }
        if x < SIDE_PT {
            return self.hit_devices(y);
        }
        self.hit_detail(x, y)
    }

    fn hit_devices(&self, y: f32) -> Target {
        let top = UNIFIED_BAR_PT + LIST_PAD_PT;
        if y < top {
            return Target::Nothing;
        }
        let i = ((y - top) / DEV_ROW_PT) as usize;
        if i < self.bt.devices.all().len() {
            Target::Device(i)
        } else {
            Target::Nothing
        }
    }

    fn hit_detail(&self, x: f32, y: f32) -> Target {
        for i in 0..self.model.outbox.len().min(SEND_ROWS) {
            if self.send_row(i).contains(x, y) {
                return Target::Send(i);
            }
        }
        let start = self.transfer_window_start();
        for k in 0..self.model.transfers.len() - start {
            if self.transfer_row(k).contains(x, y) {
                return Target::Transfer(start + k);
            }
        }
        Target::Nothing
    }

    fn hit_toolbar(&self, ui: &Ui, x: f32, y: f32) -> Target {
        let bar = self.toolbar();
        self.toolbar_layout(ui)
            .visible
            .iter()
            .find(|(_, r)| r.contains(x, y))
            .and_then(|(id, _)| bar.items.iter().find(|i| i.id == *id).filter(|i| i.enabled).map(|i| Target::Toolbar(i.id)))
            .unwrap_or(Target::Nothing)
    }

    /// A touch in points. A tap is a press and a release on the same target.
    pub fn touch(&mut self, ctx: &Ctx, ui: &Ui, kind: TouchKind, x: f32, y: f32) -> bool {
        match kind {
            TouchKind::Down => {
                self.press = self.hit(ui, x, y);
                false
            }
            TouchKind::Move => false,
            TouchKind::Up => {
                let released = self.hit(ui, x, y);
                let pressed = std::mem::replace(&mut self.press, Target::Nothing);
                if released != pressed {
                    return false;
                }
                self.activate(ctx, released)
            }
            TouchKind::Cancel => {
                self.press = Target::Nothing;
                false
            }
        }
    }

    /// Performs what a target asks for. True when the window should redraw.
    pub fn activate(&mut self, ctx: &Ctx, target: Target) -> bool {
        match target {
            Target::Nothing => false,
            Target::Toolbar(id) => {
                match id {
                    B_REFRESH => self.refresh(ctx),
                    B_SCAN => {
                        self.model.status = "Looking for devices".into();
                        bluetooth_service::scan(ctx);
                    }
                    B_PAIR => self.pair(ctx),
                    B_CLEAR => self.model.clear_finished(),
                    _ => return false,
                }
                true
            }
            Target::Device(i) => {
                if let Some(addr) = self.bt.devices.all().get(i).map(|d| d.address) {
                    self.selected = Some(addr);
                    self.model.status.clear();
                }
                true
            }
            Target::Send(i) => self.queue_file(i),
            Target::Transfer(i) => self.cancel_at(i),
        }
    }

    /// Asks the shell to pair the selected device.
    fn pair(&mut self, ctx: &Ctx) {
        let Some(addr) = self.selected else {
            self.model.status = "Choose a device to pair with".into();
            return;
        };
        self.model.status = format!("Pairing with {}", self.selected_name().unwrap_or_else(|| addr.to_string()));
        self.pair_request = Some(bluetooth_service::pair(ctx, addr));
    }

    /// Queues file `i` of the outbox for the selected device.
    fn queue_file(&mut self, i: usize) -> bool {
        let Some(addr) = self.selected else {
            self.model.status = "Choose a device before sending".into();
            return true;
        };
        let Some(file) = self.model.outbox.get(i).cloned() else { return false };
        let label = self.selected_name().unwrap_or_else(|| addr.to_string());
        self.model.queue(file, addr, label);
        self.model.status.clear();
        self.pump();
        true
    }

    /// Cancels the transfer at queue index `i`: a waiting one is dropped, a sending one is stopped by the worker.
    fn cancel_at(&mut self, i: usize) -> bool {
        let Some(id) = self.model.transfers.get(i).map(|t| t.id) else { return false };
        if self.model.request_cancel(id) {
            self.link.send(Request::Cancel { id });
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Phase;

    fn window() -> BluetoothExchange {
        BluetoothExchange {
            model: Model::default(),
            link: Link::start(),
            next_poll: 0.0,
            size: (1000.0, 640.0),
            labels: TextCache::default(),
            press: Target::Nothing,
            home: PathBuf::from("/home/u"),
            bt: Bluetooth::new(),
            selected: None,
            status_request: None,
            pair_request: None,
        }
    }

    #[test]
    fn the_send_rows_sit_under_the_header_in_order() {
        let w = window();
        let first = w.send_row(0);
        let second = w.send_row(1);
        assert!(second.y > first.y);
        assert!((second.y - first.y - ROW_PT).abs() < 1e-3);
        assert!(w.transfers_top() > w.send_row(SEND_ROWS - 1).y);
    }

    #[test]
    fn a_transfer_row_is_hit_for_the_queue_index_it_shows() {
        let mut w = window();
        for n in 0..6 {
            let file = model::FileEntry { name: format!("f{n}.txt"), path: PathBuf::from(format!("/home/u/Downloads/f{n}.txt")), size: 1, modified: 0 };
            w.model.queue(file, BdAddr([1, 2, 3, 4, 5, 6]), "Speaker".into());
        }
        let start = w.transfer_window_start();
        assert_eq!(start, 2, "six transfers show the newest four");
        let r = w.transfer_row(1);
        assert_eq!(w.hit_detail(r.x + 10.0, r.y + 4.0), Target::Transfer(3));
    }

    #[test]
    fn the_received_capacity_fits_above_the_footer() {
        let w = window();
        let fit = w.received_capacity();
        let last = w.received_top() + LABEL_PT + (fit.max(1) - 1) as f32 * ROW_PT;
        assert!(last + ROW_PT <= w.bottom() + 1e-3);
    }

    #[test]
    fn a_file_with_no_device_chosen_says_so_and_is_not_queued() {
        let mut w = window();
        w.model.outbox.push(model::FileEntry { name: "a.pdf".into(), path: PathBuf::from("/home/u/Downloads/a.pdf"), size: 9, modified: 0 });
        assert!(w.queue_file(0));
        assert_eq!(w.model.status, "Choose a device before sending");
        assert!(w.model.transfers.is_empty());
    }

    #[test]
    fn the_footer_says_what_the_radio_needs() {
        let mut w = window();
        w.bt.state = RadioState::Off;
        assert_eq!(w.footer_text(), "Turn Bluetooth on to send files");
        w.bt.state = RadioState::On;
        assert_eq!(w.footer_text(), "Choose a device in the list");
        w.model.status = "Sent a.pdf".into();
        assert_eq!(w.footer_text(), "Sent a.pdf");
    }

    #[test]
    fn a_finished_send_reports_its_file_name() {
        let mut w = window();
        let file = model::FileEntry { name: "photo.jpg".into(), path: PathBuf::from("/home/u/Downloads/photo.jpg"), size: 3, modified: 0 };
        let id = w.model.queue(file, BdAddr([1, 2, 3, 4, 5, 6]), "Speaker".into());
        w.model.start(id);
        w.apply(Reply::Done { id });
        assert_eq!(w.model.status, "Sent photo.jpg");
        assert_eq!(w.model.transfer(id).unwrap().phase, Phase::Sent);
    }

    #[test]
    fn a_failure_shows_its_reason() {
        let mut w = window();
        let file = model::FileEntry { name: "a".into(), path: PathBuf::from("/home/u/Downloads/a"), size: 3, modified: 0 };
        let id = w.model.queue(file, BdAddr([1, 2, 3, 4, 5, 6]), "Speaker".into());
        w.model.start(id);
        w.apply(Reply::Failed { id, text: "The file transfer service (obexd) is not running".into() });
        assert_eq!(w.footer_text(), "The file transfer service (obexd) is not running");
    }
}
