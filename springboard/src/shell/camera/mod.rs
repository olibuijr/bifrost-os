//! Camera: a live viewfinder, a SILENT shutter (no sound is ever played; the
//! only feedback is a short white flash), front/back switch, 3 s / 10 s
//! self-timer with a countdown, grid overlay, 4:3 / square capture, a
//! tap-to-focus indicator, a QR scanner and a last-photo thumbnail that opens
//! Photos. Photos are JPEGs (with EXIF orientation and date) in the folder
//! Photos scans; the encoding runs on its own thread.
//!
//! Nothing touches /dev/video* until the app is opened (`start`) and the
//! stream is released as soon as the app is left (`stop`). When no node
//! works the screen says "Camera not available" with a Try Again button.
//!
//! Cameras are listed by the device browser (`bifrost_ui::device_browser`): the
//! switch button picks among the attached ones and remembers the choice, and a
//! plug or unplug is noticed within about a second (the /dev nodes are re-read).
//! Import from Camera (the button at the top) lists the pictures on a camera or
//! SD card with a DCIM folder and copies the picked ones into the Photos folder.

pub mod devices;
pub mod import;
pub mod jpeg;
pub mod logic;
pub mod qr;

use self::import::{ImportAction, ImportView};
use self::logic::{Aspect, SelfTimer, ShutterAction};
use super::cache::TextCache;
use super::clock::Wall;
use super::photos::library;
use super::screenshot;
use super::switcher::draw_scaled;
use super::ui::{home_zone, Ui, Weight, BLACK, SECONDARY, WHITE};
use super::Event;
use crate::gfx::{pack, rgba, Image, Surface};
use crate::layout::Rect;
use crate::media::v4l2::{self, CameraInfo, Frame};
use bifrost_ui::device_browser::{DeviceBrowser, DeviceMask};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::path::PathBuf;

pub use self::logic::photo_name;

/// The viewfinder is converted at most this wide; the shutter converts the full frame.
pub(crate) const PREVIEW_MAX_W: usize = 640;
/// Frame poll period while streaming (ms).
pub(crate) const POLL_MS: f64 = 33.0;
/// JPEG quality of a capture.
const JPEG_QUALITY: u8 = 92;
/// Minimum time between QR scans of the viewfinder (ms).
const QR_SCAN_MS: f64 = 450.0;
/// How often the /dev nodes and the mounted cards are checked while the app is open (ms).
const HOTPLUG_MS: f64 = 1000.0;
/// Corner radius in px for every control (rule: at most 6).
const RADIUS: f32 = 6.0;
const PLATE: u32 = pack(44, 44, 46, 255);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Target {
    Shutter,
    Switch,
    Thumb,
    Retry,
    Timer,
    Grid,
    Mode(usize),
    Flash,
    Zoom(usize),
    Qr,
    QrBanner,
    /// opens Import from Camera
    Import,
    /// a tap on the picture itself: tap-to-focus
    View,
}

/// A finished capture, from the encoder thread.
struct Saved {
    path: Option<PathBuf>,
    name: String,
    thumb: Option<Image>,
}

#[derive(Clone, PartialEq, Debug)]
enum State {
    /// the app has not been opened yet: no device was touched
    Idle,
    Live,
    Unavailable(String),
}

/// The V4L2 capture session on its own thread: opening a node can take a while and
/// reading blocks, so the UI thread only ever polls a slot.
pub(crate) struct Worker {
    slot: Arc<Mutex<Slot>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

/// What the camera thread has to report: only the newest frame is kept, so a slow
/// UI never makes frames pile up.
#[derive(Default)]
struct Slot {
    opened: bool,
    frame: Option<Frame>,
    failed: Option<String>,
}

/// Everything new since the last `Worker::poll`.
#[derive(Default)]
pub(crate) struct Polled {
    pub(crate) opened: bool,
    pub(crate) frame: Option<Frame>,
    pub(crate) failed: Option<String>,
}

impl Worker {
    pub(crate) fn spawn(path: String) -> Worker {
        let slot = Arc::new(Mutex::new(Slot::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let (flag, shared) = (stop.clone(), slot.clone());
        let thread = std::thread::spawn(move || {
            let post = |f: &mut dyn FnMut(&mut Slot)| {
                if let Ok(mut s) = shared.lock() {
                    f(&mut s);
                }
            };
            // a node allows one open session: wait for the previous one to close
            let closing: Vec<_> = CLOSING.lock().map(|mut c| c.drain(..).collect()).unwrap_or_default();
            for t in closing {
                let _ = t.join();
            }
            let mut cam = match v4l2::Device::open(&path) {
                Ok(c) => c,
                Err(e) => {
                    let why = describe_error(&e.to_string());
                    post(&mut |s| s.failed = Some(why.clone()));
                    return;
                }
            };
            post(&mut |s| s.opened = true);
            while !flag.load(Ordering::Relaxed) {
                match cam.latest() {
                    Ok(Some(f)) => {
                        let mut frame = Some(f);
                        post(&mut |s| s.frame = frame.take());
                    }
                    Ok(None) => std::thread::sleep(std::time::Duration::from_millis(10)),
                    Err(e) => {
                        let why = describe_error(&e.to_string());
                        post(&mut |s| s.failed = Some(why.clone()));
                        break;
                    }
                }
            }
            // dropping the session closes the camera
            drop(cam);
        });
        Worker { slot, stop, thread: Some(thread) }
    }

    /// Takes what the camera thread reported since the last call. A thread that is gone
    /// without saying why (it panicked) counts as a failure, so nobody polls it forever.
    pub(crate) fn poll(&self) -> Polled {
        let mut p = self.slot.lock().map(|mut s| Polled { opened: std::mem::take(&mut s.opened), frame: s.frame.take(), failed: s.failed.take() }).unwrap_or_default();
        if p.failed.is_none() && p.frame.is_none() && self.thread.as_ref().is_some_and(|t| t.is_finished()) {
            p.failed = Some("The camera stopped unexpectedly".into());
        }
        p
    }
}

/// Sessions that are still closing: closing a session can take a while, so the UI thread
/// never waits for it; the next session's thread joins these before it opens a camera.
static CLOSING: std::sync::Mutex<Vec<std::thread::JoinHandle<()>>> = std::sync::Mutex::new(Vec::new());

impl Drop for Worker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            if let Ok(mut c) = CLOSING.lock() {
                c.push(t);
            }
        }
    }
}

/// A short, readable reason for a failed start.
fn describe_error(e: &str) -> String {
    if e.contains("in use") || e.contains("busy") {
        "The camera is in use by another process".into()
    } else {
        format!("Could not start the camera ({e})")
    }
}

pub struct Camera {
    w: i32,
    h: i32,
    dir: PathBuf,
    state: State,
    cameras: Vec<CameraInfo>,
    current: usize,
    /// the attached cameras, by stable id; keeps the chosen camera across unplug and replug
    cams: DeviceBrowser,
    /// the /dev video nodes seen by the last scan
    nodes: Vec<String>,
    /// when the devices were last checked (app clock, ms)
    scan_at: f64,
    /// the cards and camera storage that hold a DCIM folder, by id
    volumes: DeviceBrowser,
    /// Import from Camera, while it is open
    import: Option<ImportView>,
    copied_tx: Sender<usize>,
    copied_rx: Receiver<usize>,
    worker: Option<Worker>,
    preview: Option<Image>,
    last_raw: Option<Frame>,
    thumb: Option<(PathBuf, Image)>,
    flash_at: Option<f64>,
    aspect: Aspect,
    /// index into `MODES`; Photo by default
    mode: usize,
    flash: Flash,
    zoom: usize,
    timer: SelfTimer,
    grid: bool,
    qr_on: bool,
    qr_result: Option<String>,
    qr_scanned_at: f64,
    /// the white balance of the live preview, following the scene a frame at a time
    balance: crate::media::convert::Smoothed,
    /// QR scans run on their own thread; one at a time
    qr_busy: bool,
    qr_tx: Sender<Option<String>>,
    qr_rx: Receiver<Option<String>>,
    /// the newest photo's thumbnail is being read on a worker thread
    thumb_loading: bool,
    focus: Option<(f32, f32, f64)>,
    saved_tx: Sender<Saved>,
    saved_rx: Receiver<Saved>,
    /// names handed to encoder threads whose files are not on disk yet
    reserved: Vec<String>,
    open_photo: Option<PathBuf>,
    wall: Wall,
    pressed: Option<Target>,
    dirty: bool,
    cache: TextCache,
    cache_style: (u32, u32, bool),
}

/// The bifrost mode strip, top to bottom.
const MODES: [&str; 5] = ["TIME-LAPSE", "VIDEO", "PHOTO", "SQUARE", "PANO"];
const PHOTO_MODE: usize = 2;
const SQUARE_MODE: usize = 3;
/// Zoom pill choices.
const ZOOMS: [(&str, usize); 2] = [("1x", 1), ("2x", 2)];
/// systemYellow: selected mode, active controls.
const YELLOW: u32 = pack(255, 204, 0, 255);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Flash {
    Off,
    Auto,
    On,
}

impl Flash {
    fn cycled(self) -> Flash {
        match self {
            Flash::Auto => Flash::On,
            Flash::On => Flash::Off,
            Flash::Off => Flash::Auto,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Flash::Off => "Off",
            Flash::Auto => "Auto",
            Flash::On => "On",
        }
    }
}

/// `crop` (x, y, w, h) narrowed around its centre by the digital zoom factor.
fn zoomed((x, y, w, h): (usize, usize, usize, usize), factor: usize) -> (usize, usize, usize, usize) {
    let f = factor.max(1);
    let (nw, nh) = ((w / f).max(1), (h / f).max(1));
    (x + (w - nw) / 2, y + (h - nh) / 2, nw, nh)
}

type Rect4 = (f32, f32, f32, f32);

/// Most lines of the reason shown under "Camera not available"; longer text ends in an ellipsis.
const UNAVAILABLE_LINES: usize = 3;

/// Sizes in px of the parts of the "Camera not available" stack.
struct UnavailableMetrics {
    title_h: f32,
    line_h: f32,
    title_gap: f32,
    button_gap: f32,
    button: (f32, f32),
}

/// Lays the title, `lines` rows of reason and the Try Again button out top to bottom without overlap, the
/// stack centred vertically in `view` (pinned to its top when the pane is shorter than the stack) and each
/// part centred horizontally. Returns (title, one rect per line, button).
fn unavailable_stack((vx, vy, vw, vh): Rect4, lines: usize, m: &UnavailableMetrics) -> (Rect4, Vec<Rect4>, Rect4) {
    let text_h = if lines == 0 { 0.0 } else { m.title_gap + m.line_h * lines as f32 };
    let total = m.title_h + text_h + m.button_gap + m.button.1;
    let top = vy + ((vh - total) / 2.0).max(0.0);
    let title = (vx, top, vw, m.title_h);
    let first = top + m.title_h + m.title_gap;
    let rows = (0..lines).map(|i| (vx, first + m.line_h * i as f32, vw, m.line_h)).collect();
    let button_y = top + m.title_h + text_h + m.button_gap;
    let button = (vx + (vw - m.button.0) / 2.0, button_y, m.button.0, m.button.1);
    (title, rows, button)
}

/// The largest `(w, h)` with the frame's aspect that fits in `bw` x `bh`.
pub fn fit(fw: usize, fh: usize, bw: i32, bh: i32) -> (i32, i32) {
    if fw == 0 || fh == 0 {
        return (bw, bh);
    }
    let by_w = (bw, (bw as i64 * fh as i64 / fw as i64) as i32);
    if by_w.1 <= bh {
        by_w
    } else {
        ((bh as i64 * fw as i64 / fh as i64) as i32, bh)
    }
}

impl Camera {
    /// Creates the app without touching any device. Photos go to `dir`.
    pub fn new(ui: &Ui, w: i32, h: i32, dir: PathBuf, wall: Wall) -> Self {
        let (saved_tx, saved_rx) = channel();
        let (qr_tx, qr_rx) = channel();
        let (copied_tx, copied_rx) = channel();
        Self {
            w,
            h,
            dir,
            state: State::Idle,
            cameras: Vec::new(),
            current: 0,
            cams: DeviceBrowser::new(),
            nodes: Vec::new(),
            scan_at: f64::MIN,
            volumes: DeviceBrowser::new(),
            import: None,
            copied_tx,
            copied_rx,
            worker: None,
            preview: None,
            last_raw: None,
            thumb: None,
            flash_at: None,
            aspect: Aspect::FourThree,
            mode: PHOTO_MODE,
            flash: Flash::Auto,
            zoom: 0,
            timer: SelfTimer::new(),
            grid: false,
            qr_on: false,
            qr_result: None,
            qr_scanned_at: f64::MIN,
            balance: Default::default(),
            qr_busy: false,
            qr_tx,
            qr_rx,
            thumb_loading: false,
            focus: None,
            saved_tx,
            saved_rx,
            reserved: Vec::new(),
            open_photo: None,
            wall,
            pressed: None,
            dirty: true,
            cache: TextCache::default(),
            cache_style: (ui.s.to_bits(), ui.text_scale.to_bits(), ui.bold),
        }
    }

    pub fn set_wall(&mut self, wall: Wall) {
        self.wall = wall;
    }

    /// The app was opened by the user: probe the nodes, open the first and load the last-photo thumbnail.
    pub fn start(&mut self, ui: &Ui) {
        self.nodes = devices::node_listing();
        self.cameras = v4l2::probe();
        // the browser keeps its preferred camera from an earlier visit
        self.cams.start(DeviceMask::CAMERA);
        let _ = self.cams.update(devices::camera_devices(&self.cameras));
        self.choose_preferred();
        if self.thumb.is_none() && !self.thumb_loading {
            self.thumb_loading = true;
            let (dir, n, tx) = (self.dir.clone(), self.thumb_size(ui), self.saved_tx.clone());
            // decoding the newest photo takes a while: the viewfinder must not wait for it
            std::thread::spawn(move || {
                let saved = match Self::newest_thumb(&dir, n) {
                    Some((path, img)) => Saved { path: Some(path), name: String::new(), thumb: Some(img) },
                    None => Saved { path: None, name: String::new(), thumb: None },
                };
                let _ = tx.send(saved);
            });
        }
        self.dirty = true;
    }

    /// The app was left: release the camera so nothing keeps streaming in the background.
    pub fn stop(&mut self) {
        self.timer.cancel();
        self.cams.stop();
        self.close_import();
        if self.state != State::Idle {
            self.worker = None;
            self.state = State::Idle;
            self.preview = None;
            self.last_raw = None;
            self.dirty = true;
        }
    }

    /// Starts when the app is on screen and not streaming yet (first open, or back from the
    /// background). A failed start stays "not available" until Try Again.
    pub fn resume(&mut self, ui: &Ui) {
        if self.state == State::Idle {
            self.start(ui);
        }
    }

    /// The id of the camera that is streaming now, when one is.
    fn live_camera_id(&self) -> Option<String> {
        if self.state != State::Live {
            return None;
        }
        devices::camera_ids(&self.cameras).get(self.current).cloned()
    }

    /// Opens the preferred camera (the one picked last time), or the first attached one. With no camera
    /// attached the screen says so.
    fn choose_preferred(&mut self) {
        let wanted = self.cams.preferred_device().map(|d| d.id.clone());
        let at = wanted.and_then(|id| devices::camera_ids(&self.cameras).iter().position(|c| *c == id)).unwrap_or(0);
        self.current = at;
        self.open_current();
    }

    /// /dev changed: probes the cameras again and applies the plug and unplug events. The camera that is
    /// streaming keeps streaming when it is still attached. When it is gone, or nothing is streaming, the
    /// preferred camera is opened.
    fn refresh_cameras(&mut self, nodes: Vec<String>) {
        self.nodes = nodes;
        let streaming = self.live_camera_id();
        self.cameras = v4l2::probe();
        let _ = self.cams.update(devices::camera_devices(&self.cameras));
        let still = streaming.and_then(|id| devices::camera_ids(&self.cameras).iter().position(|c| *c == id));
        match still {
            Some(at) => self.current = at,
            None => self.choose_preferred(),
        }
        self.dirty = true;
    }

    /// Checks for plugged and unplugged cameras and cards. Runs about once a second while the app is open.
    fn check_devices(&mut self, t: f64) {
        self.scan_at = t;
        let nodes = devices::node_listing();
        if nodes != self.nodes {
            self.refresh_cameras(nodes);
        }
        if self.import.is_some() {
            let events = self.volumes.update(devices::volume_devices());
            if let Some(view) = &mut self.import {
                for e in &events {
                    view.on_event(e);
                }
            }
            if !events.is_empty() {
                self.dirty = true;
            }
        }
    }

    /// Opens Import from Camera with the cards and camera storage attached now.
    fn open_import(&mut self) {
        self.volumes = DeviceBrowser::new();
        self.volumes.start(DeviceMask::CAMERA);
        let mut view = ImportView::new(self.w, self.h);
        for e in self.volumes.update(devices::volume_devices()) {
            view.on_event(&e);
        }
        self.import = Some(view);
        self.dirty = true;
    }

    fn close_import(&mut self) {
        self.volumes.stop();
        self.import = None;
    }

    pub fn streaming(&self) -> bool {
        self.worker.is_some()
    }

    fn open_current(&mut self) {
        self.worker = None; // closes the previous session first
        self.preview = None;
        self.last_raw = None;
        if self.cameras.get(self.current).is_none() {
            self.state = State::Unavailable("No camera found".into());
            return;
        }
        self.worker = Some(Worker::spawn(self.cameras[self.current].path.clone()));
        self.state = State::Live;
    }

    fn thumb_size(&self, ui: &Ui) -> usize {
        ui.pt(64.0) as usize
    }

    fn newest_thumb(dir: &PathBuf, n: usize) -> Option<(PathBuf, Image)> {
        let newest = library::scan(std::slice::from_ref(dir)).pop()?;
        let img = crate::cg::imageio::decode(&std::fs::read(&newest.path).ok()?)?;
        Some((newest.path, square_thumb(&img, n)))
    }

    /// Set when the user tapped the thumbnail: the shell opens this photo in Photos.
    pub fn take_open_photo(&mut self) -> Option<PathBuf> {
        self.open_photo.take()
    }

    /// Takes the latest frame: the crop, JPEG encoding and file write run on a thread so the
    /// shell never stalls; the thumbnail updates when it finishes. Silent. False without a frame.
    fn shoot(&mut self, ui: &Ui, t: f64) -> bool {
        let Some(src) = self.last_raw.as_ref() else { return false };
        let frame = Frame { width: src.width, height: src.height, stride: src.stride, format: src.format, data: src.data.clone() };
        let unix = self.wall.now(t);
        let date = library::civil(unix, self.wall.offset);
        let name = logic::photo_name_avoiding(&self.dir, &date, &self.reserved);
        self.reserved.push(name.clone());
        let (dir, aspect, zoom, thumb_n, tx) = (self.dir.clone(), self.aspect, ZOOMS[self.zoom].1, self.thumb_size(ui), self.saved_tx.clone());
        let exif = jpeg::Exif { orientation: 1, date: logic::exif_date(&date) };
        std::thread::spawn(move || {
            let _ = tx.send(save_capture(&frame, &dir, name, aspect, zoom, thumb_n, unix, &exif));
        });
        self.flash_at = Some(t);
        true
    }

    /// Width of the right-hand control column.
    fn col_w(&self, ui: &Ui) -> f32 {
        ui.pt(112.0)
    }

    fn col_cx(&self, ui: &Ui) -> f32 {
        self.w as f32 - self.col_w(ui) / 2.0
    }

    fn view_rect(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        let top = ui.pt(28.0);
        (0.0, top, self.w as f32 - self.col_w(ui), (self.h as f32 - home_zone(ui) - top).max(0.0))
    }

    fn shutter_rect(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        let size = ui.pt(76.0);
        (self.col_cx(ui) - size / 2.0, self.h as f32 / 2.0 - size / 2.0, size, size)
    }

    fn thumb_rect(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        let size = ui.pt(56.0);
        let s = self.shutter_rect(ui);
        (self.col_cx(ui) - size / 2.0, s.1 + s.3 + ui.pt(32.0), size, size)
    }

    /// Flip button: a 44 pt circle at the bottom of the column.
    fn switch_rect(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        let size = ui.pt(44.0);
        (self.col_cx(ui) - size / 2.0, self.h as f32 - home_zone(ui) - ui.pt(24.0) - size, size, size)
    }

    /// Flash button: a 44 pt circle at the top of the column.
    fn flash_rect(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        let size = ui.pt(44.0);
        (self.col_cx(ui) - size / 2.0, self.view_rect(ui).1 + ui.pt(12.0), size, size)
    }

    /// Row `i` of the vertical mode strip, which ends above the shutter.
    fn mode_rect(&self, ui: &Ui, i: usize) -> (f32, f32, f32, f32) {
        let row = ui.pt(44.0);
        let bottom = self.shutter_rect(ui).1 - ui.pt(20.0);
        (self.w as f32 - self.col_w(ui), bottom - (MODES.len() - i) as f32 * row, self.col_w(ui), row)
    }

    /// The 1x / 2x pill centred at the bottom of the picture.
    fn zoom_rect(&self, ui: &Ui, i: usize) -> (f32, f32, f32, f32) {
        let (vx, vy, vw, vh) = self.view_rect(ui);
        let (d, gap) = (ui.pt(44.0), ui.pt(8.0));
        let total = ZOOMS.len() as f32 * d + (ZOOMS.len() - 1) as f32 * gap;
        (vx + (vw - total) / 2.0 + i as f32 * (d + gap), vy + vh - ui.pt(16.0) - d, d, d)
    }

    /// The toolbar buttons over the top of the picture: timer, grid, QR.
    fn tool_rect(&self, ui: &Ui, i: usize) -> (f32, f32, f32, f32) {
        let (w, h, gap) = (ui.pt(112.0), ui.pt(40.0), ui.pt(8.0));
        let (_, vy, _, _) = self.view_rect(ui);
        (ui.pt(16.0) + i as f32 * (w + gap), vy + ui.pt(12.0), w, h)
    }

    fn banner_rect(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        let (_, vy, vw, vh) = self.view_rect(ui);
        (ui.pt(16.0), vy + vh - ui.pt(136.0), vw - ui.pt(32.0), ui.pt(56.0))
    }

    fn retry_rect(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        self.unavailable_layout(ui).2
    }

    /// The "Camera not available" screen: the title, the reason wrapped to the viewfinder pane (at most
    /// `UNAVAILABLE_LINES` lines), and the Try Again button below them, the whole stack centred in the pane.
    fn unavailable_layout(&self, ui: &Ui) -> (Rect4, Vec<(String, Rect4)>, Rect4) {
        let view = self.view_rect(ui);
        let why = match &self.state {
            State::Unavailable(why) => why.as_str(),
            _ => "",
        };
        let lines = ui.wrap_capped(why, 14.0, Weight::Regular, (view.2 - ui.pt(48.0)).max(ui.pt(40.0)), UNAVAILABLE_LINES);
        let metrics = UnavailableMetrics {
            title_h: ui.pt(34.0),
            line_h: ui.pt(20.0),
            title_gap: ui.pt(6.0),
            button_gap: ui.pt(24.0),
            button: (ui.pt(180.0), ui.pt(56.0)),
        };
        let (title, rows, retry) = unavailable_stack(view, lines.len(), &metrics);
        (title, lines.into_iter().zip(rows).collect(), retry)
    }

    fn inside((x, y, w, h): (f32, f32, f32, f32), px: f32, py: f32) -> bool {
        px >= x && px < x + w && py >= y && py < y + h
    }

    fn hit(&self, ui: &Ui, x: f32, y: f32) -> Option<Target> {
        if y >= self.h as f32 - home_zone(ui) {
            return None;
        }
        if matches!(self.state, State::Unavailable(_)) && Self::inside(self.retry_rect(ui), x, y) {
            return Some(Target::Retry);
        }
        if self.state != State::Idle && Self::inside(self.tool_rect(ui, 3), x, y) {
            return Some(Target::Import);
        }
        if self.state == State::Live && Self::inside(self.shutter_rect(ui), x, y) && self.last_raw.is_some() && matches!(self.mode, PHOTO_MODE | SQUARE_MODE) {
            return Some(Target::Shutter);
        }
        if self.state == State::Live {
            for i in 0..MODES.len() {
                if Self::inside(self.mode_rect(ui, i), x, y) {
                    return Some(Target::Mode(i));
                }
            }
            if Self::inside(self.flash_rect(ui), x, y) {
                return Some(Target::Flash);
            }
            for i in 0..ZOOMS.len() {
                if Self::inside(self.zoom_rect(ui, i), x, y) {
                    return Some(Target::Zoom(i));
                }
            }
        }
        if self.cameras.len() > 1 && Self::inside(self.switch_rect(ui), x, y) {
            return Some(Target::Switch);
        }
        if self.thumb.is_some() && Self::inside(self.thumb_rect(ui), x, y) {
            return Some(Target::Thumb);
        }
        if self.state == State::Live {
            for (i, target) in [Target::Timer, Target::Grid, Target::Qr].into_iter().enumerate() {
                if Self::inside(self.tool_rect(ui, i), x, y) {
                    return Some(target);
                }
            }
            if self.qr_result.is_some() && Self::inside(self.banner_rect(ui), x, y) {
                return Some(Target::QrBanner);
            }
            if self.preview.is_some() && Self::inside(self.view_rect(ui), x, y) {
                return Some(Target::View);
            }
        }
        None
    }

    pub fn touch(&mut self, ui: &Ui, event: Event, t: f64) {
        if let Some(view) = &mut self.import {
            if view.touch(ui, event) == ImportAction::Close {
                self.close_import();
            }
            self.dirty = true;
            return;
        }
        match event {
            Event::Down(x, y) => {
                self.pressed = self.hit(ui, x, y);
                self.dirty = true;
            }
            Event::Move(x, y) => {
                // sliding off cancels, so a home swipe never fires a button
                if self.pressed.is_some() && self.hit(ui, x, y) != self.pressed {
                    self.pressed = None;
                    self.dirty = true;
                }
            }
            Event::Up(x, y) => {
                let pressed = self.pressed.take();
                if pressed.is_some() && self.hit(ui, x, y) == pressed {
                    match pressed.unwrap() {
                        Target::Shutter => match self.timer.press(t) {
                            ShutterAction::Capture => {
                                self.shoot(ui, t);
                            }
                            ShutterAction::Countdown | ShutterAction::Cancelled => {}
                        },
                        Target::Timer => self.timer.cycle(),
                        Target::Grid => self.grid = !self.grid,
                        Target::Mode(i) => {
                            self.mode = i;
                            self.aspect = if i == SQUARE_MODE { Aspect::Square } else { Aspect::FourThree };
                        }
                        Target::Flash => self.flash = self.flash.cycled(),
                        Target::Zoom(i) => self.zoom = i,
                        Target::Qr => {
                            self.qr_on = !self.qr_on;
                            self.qr_result = None;
                            self.qr_scanned_at = f64::MIN;
                        }
                        Target::QrBanner => self.qr_result = None,
                        Target::View => {
                            let p = logic::clamp_into(self.view_rect(ui), x, y);
                            self.focus = Some((p.0, p.1, t));
                        }
                        Target::Switch => {
                            self.timer.cancel();
                            self.current = (self.current + 1) % self.cameras.len().max(1);
                            // the picked camera is the one opened next time too
                            if let Some(id) = devices::camera_ids(&self.cameras).get(self.current) {
                                self.cams.set_preferred(id);
                            }
                            self.open_current();
                        }
                        Target::Import => self.open_import(),
                        Target::Retry => self.start(ui),
                        Target::Thumb => self.open_photo = self.thumb.as_ref().map(|(p, _)| p.clone()),
                    }
                }
                self.dirty = true;
            }
            _ => {}
        }
    }

    /// Pulls the newest frame; true when the picture changed.
    pub fn tick(&mut self, ui: &Ui, t: f64) -> bool {
        if t - self.scan_at >= HOTPLUG_MS {
            self.check_devices(t);
        }
        // a finished copy from Import from Camera, and a new copy request from the view
        while let Ok(copied) = self.copied_rx.try_recv() {
            if let Some(view) = &mut self.import {
                view.on_copied(copied);
            }
            self.dirty = true;
        }
        if let Some(view) = &mut self.import {
            if let Some(paths) = view.take_request() {
                let (dir, tx) = (self.dir.clone(), self.copied_tx.clone());
                std::thread::spawn(move || {
                    let _ = tx.send(devices::import_files(&paths, &dir));
                });
                self.dirty = true;
            }
        }
        let (_, _, vw, vh) = self.view_rect(ui);
        let mut failed = None;
        if let Some(w) = &self.worker {
            let polled = w.poll();
            self.dirty |= polled.opened;
            failed = polled.failed;
            let newest = polled.frame;
            if let Some(frame) = newest {
                let (cx, cy, cw, ch) = zoomed(self.aspect.crop(frame.width, frame.height), ZOOMS[self.zoom].1);
                let (fw, _fh) = fit(cw, ch, vw as i32, vh as i32);
                let pw = (fw as usize).min(PREVIEW_MAX_W).max(1);
                let ph = (pw * ch / cw.max(1)).max(1);
                // convert the whole frame at the matching scale, then cut the crop out of it
                let (full_w, full_h) = ((pw * frame.width / cw.max(1)).max(1), (ph * frame.height / ch.max(1)).max(1));
                if let Some(px) = v4l2::to_pixels(&frame, full_w, full_h) {
                    let mut full = Image { w: full_w, h: full_h, px };
                    // the preview follows the scene's white balance, smoothed so it does not flicker
                    let gains = self.balance.update(&full);
                    crate::media::convert::apply_gains(&mut full, gains);
                    if self.qr_on && !self.qr_busy && t - self.qr_scanned_at >= QR_SCAN_MS {
                        self.qr_scanned_at = t;
                        self.qr_busy = true;
                        let (luma, lw, lh, tx) = (qr::luma_of(&full.px), full.w, full.h, self.qr_tx.clone());
                        std::thread::spawn(move || {
                            let _ = tx.send(qr::decode_luma(&luma, lw, lh));
                        });
                    }
                    self.preview = Some(full.crop(cx * full_w / frame.width.max(1), cy * full_h / frame.height.max(1), pw, ph));
                    self.dirty = true;
                }
                self.last_raw = Some(frame);
            }
        }
        if let Some(e) = failed {
            self.worker = None;
            self.state = State::Unavailable(e);
            self.dirty = true;
        }
        while let Ok(found) = self.qr_rx.try_recv() {
            self.qr_busy = false;
            if let (true, Some(text)) = (self.qr_on, found) {
                self.qr_result = Some(text);
                self.dirty = true;
            }
        }
        while let Ok(done) = self.saved_rx.try_recv() {
            if done.name.is_empty() {
                // the thumbnail read at start-up; a photo taken meanwhile is newer
                self.thumb_loading = false;
                if self.thumb.is_some() {
                    continue;
                }
            }
            self.reserved.retain(|n| *n != done.name);
            if let (Some(path), Some(img)) = (done.path, done.thumb) {
                self.thumb = Some((path, img));
            }
            self.dirty = true;
        }
        if self.timer.poll(t) {
            self.shoot(ui, t);
        }
        if self.timer.running() {
            self.dirty = true;
        }
        if self.flash_at.is_some_and(|f| t - f < logic::SHUTTER_MS) {
            self.dirty = true;
        } else {
            self.flash_at = None;
        }
        if self.focus.is_some_and(|(_, _, f)| t - f < logic::FOCUS_MS) {
            self.dirty = true;
        } else {
            self.focus = None;
        }
        self.dirty
    }

    pub fn next_wake(&self, t: f64) -> Option<f64> {
        let busy = self.worker.is_some() || self.flash_at.is_some() || self.timer.running() || self.focus.is_some() || !self.reserved.is_empty() || self.import.as_ref().is_some_and(|v| v.busy());
        if busy {
            Some(t + POLL_MS)
        } else if self.state != State::Idle {
            // nothing to draw, but the devices are checked about once a second
            Some(t + HOTPLUG_MS)
        } else {
            None
        }
    }

    pub fn describe(&self) -> String {
        let state = match &self.state {
            State::Idle => "idle".to_string(),
            State::Live => "live".to_string(),
            State::Unavailable(m) => format!("unavailable:{m}"),
        };
        format!("camera_state={state} camera_nodes={} camera_current={} camera_import={} camera_has_frame={} camera_thumb={} camera_aspect={} camera_timer={} camera_grid={} camera_qr={} camera_mode={} camera_flash={} camera_zoom={}", self.cameras.len(), self.current, self.import.is_some(), self.last_raw.is_some(), self.thumb.is_some(), self.aspect.label(), self.timer.mode.seconds(), self.grid, self.qr_on, MODES[self.mode], self.flash.label(), ZOOMS[self.zoom].0)
    }

    fn label(&mut self, s: &mut Surface, ui: &Ui, text: &str, pt: f32, color: u32, (x, y, w, h): (f32, f32, f32, f32)) {
        let image = self.cache.get(ui, text, pt, Weight::Regular, color);
        s.blit(image, (x + (w - image.w as f32) / 2.0) as i32, (y + (h - image.h as f32) / 2.0) as i32, 255, 255);
    }

    pub fn draw(&mut self, s: &mut Surface, ui: &Ui, t: f64) {
        self.dirty = false;
        let style = (ui.s.to_bits(), ui.text_scale.to_bits(), ui.bold);
        if style != self.cache_style {
            self.cache.clear();
            self.cache_style = style;
        }
        if let Some(view) = &mut self.import {
            view.draw(s, ui);
            return;
        }
        s.fill_rect(0, 0, self.w as usize, self.h as usize, BLACK);
        let (vx, vy, vw, vh) = self.view_rect(ui);
        match self.state.clone() {
            State::Live => {
                if let Some(img) = self.preview.take() {
                    let (fw, fh) = fit(img.w, img.h, vw as i32, vh as i32);
                    let r = Rect { x: (vx + (vw - fw as f32) / 2.0) as i32, y: (vy + (vh - fh as f32) / 2.0) as i32, w: fw, h: fh };
                    draw_scaled(s, &img, r, 0.0, 255);
                    self.preview = Some(img);
                    let line = (ui.pt(1.0).round() as usize).max(1);
                    if self.grid {
                        let rule = rgba(255, 255, 255, 120);
                        for (x0, y0, x1, y1) in logic::grid_lines(r.x as f32, r.y as f32, fw as f32, fh as f32) {
                            if x0 == x1 {
                                s.fill_rect(x0 as usize, y0 as usize, line, (y1 - y0) as usize, rule);
                            } else {
                                s.fill_rect(x0 as usize, y0 as usize, (x1 - x0) as usize, line, rule);
                            }
                        }
                    }
                    if let Some(f) = self.flash_at {
                        let a = logic::shutter_flash(t - f);
                        s.fill_rrect(r.x as f32, r.y as f32, fw as f32, fh as f32, 0.0, rgba(255, 255, 255, a));
                    }
                    if let Some((fx, fy, ft)) = self.focus {
                        if let Some((scale, alpha)) = logic::focus_anim(t - ft) {
                            let side = ui.pt(84.0) * scale;
                            s.stroke_rrect(fx - side / 2.0, fy - side / 2.0, side, side, 0.0, ui.pt(2.0).max(1.0), rgba(255, 204, 0, alpha));
                        }
                    }
                    if let Some(n) = self.timer.remaining(t) {
                        self.label(s, ui, &n.to_string(), 140.0, WHITE, (vx, vy, vw, vh));
                    }
                    if self.qr_on && self.qr_result.is_none() {
                        self.label(s, ui, "Point at a QR code", 16.0, WHITE, (vx, vy + vh - ui.pt(60.0), vw, ui.pt(40.0)));
                    }
                } else {
                    self.label(s, ui, "Starting camera…", 18.0, SECONDARY, (vx, vy, vw, vh));
                }
            }
            State::Unavailable(_) => {
                let (title, lines, r) = self.unavailable_layout(ui);
                self.label(s, ui, "Camera not available", 24.0, WHITE, title);
                for (line, rect) in &lines {
                    self.label(s, ui, line, 14.0, SECONDARY, *rect);
                }
                let fill = if self.pressed == Some(Target::Retry) { pack(72, 72, 74, 255) } else { PLATE };
                s.fill_rrect(r.0, r.1, r.2, r.3, RADIUS, fill);
                self.label(s, ui, "Try Again", 17.0, WHITE, r);
            }
            State::Idle => {}
        }
        if self.state == State::Live {
            let labels = [self.timer.mode.label(), if self.grid { "Grid on" } else { "Grid off" }, if self.qr_on { "QR on" } else { "QR off" }];
            let active = [self.timer.mode != logic::TimerMode::Off, self.grid, self.qr_on];
            let targets = [Target::Timer, Target::Grid, Target::Qr];
            for i in 0..3 {
                let r = self.tool_rect(ui, i);
                let fill = match (active[i], self.pressed == Some(targets[i])) {
                    (true, _) => YELLOW,
                    (false, true) => pack(72, 72, 74, 255),
                    _ => pack(44, 44, 46, 220),
                };
                s.fill_rrect(r.0, r.1, r.2, r.3, RADIUS, fill);
                self.label(s, ui, labels[i], 14.0, if active[i] { BLACK } else { WHITE }, r);
            }
            if let Some(text) = self.qr_result.clone() {
                let r = self.banner_rect(ui);
                s.fill_rrect(r.0, r.1, r.2, r.3, RADIUS, if self.pressed == Some(Target::QrBanner) { pack(72, 72, 74, 255) } else { PLATE });
                let shown = ui.fit(&text, 16.0, r.2 - ui.pt(24.0));
                self.label(s, ui, &shown, 16.0, WHITE, r);
            }
            let mut r = self.shutter_rect(ui);
            if let Some(f) = self.flash_at {
                let k = logic::shutter_button_scale(t - f);
                let (nw, nh) = (r.2 * k, r.3 * k);
                r = (r.0 + (r.2 - nw) / 2.0, r.1 + (r.3 - nh) / 2.0, nw, nh);
            }
            let down = self.pressed == Some(Target::Shutter);
            let dim = !matches!(self.mode, PHOTO_MODE | SQUARE_MODE);
            let ring = ui.pt(4.0).max(1.0);
            let half = r.2 / 2.0;
            s.fill_rrect(r.0, r.1, r.2, r.3, half, if dim { pack(120, 120, 124, 255) } else { WHITE });
            s.fill_rrect(r.0 + ring, r.1 + ring, r.2 - 2.0 * ring, r.3 - 2.0 * ring, half - ring, BLACK);
            let gap = ring * 1.5;
            let dot = if self.timer.running() { pack(230, 60, 50, 255) } else if down { pack(200, 200, 200, 255) } else if dim { pack(120, 120, 124, 255) } else { WHITE };
            s.fill_rrect(r.0 + ring + gap, r.1 + ring + gap, r.2 - 2.0 * (ring + gap), r.3 - 2.0 * (ring + gap), half - ring - gap, dot);
            for i in 0..MODES.len() {
                let (mr, selected) = (self.mode_rect(ui, i), i == self.mode);
                let image = self.cache.get(ui, MODES[i], 13.0, Weight::Semibold, if selected { YELLOW } else { WHITE });
                s.blit(image, (mr.0 + (mr.2 - image.w as f32) / 2.0) as i32, (mr.1 + (mr.3 - image.h as f32) / 2.0) as i32, 255, 255);
            }
            let fr = self.flash_rect(ui);
            let lit = self.flash != Flash::Off;
            let fill = if self.pressed == Some(Target::Flash) { pack(72, 72, 74, 255) } else { pack(44, 44, 46, 220) };
            s.fill_rrect(fr.0, fr.1, fr.2, fr.3, fr.2 / 2.0, fill);
            self.label(s, ui, match self.flash { Flash::Off => "Off", Flash::Auto => "A", Flash::On => "On" }, 15.0, if lit { YELLOW } else { WHITE }, fr);
            for (i, (text, _)) in ZOOMS.iter().enumerate() {
                let zr = self.zoom_rect(ui, i);
                let on = i == self.zoom;
                let fill = if on { YELLOW } else if self.pressed == Some(Target::Zoom(i)) { pack(72, 72, 74, 255) } else { pack(44, 44, 46, 220) };
                s.fill_rrect(zr.0, zr.1, zr.2, zr.3, zr.2 / 2.0, fill);
                self.label(s, ui, text, 15.0, if on { BLACK } else { WHITE }, zr);
            }
        }
        if self.state != State::Idle {
            let r = self.tool_rect(ui, 3);
            let fill = if self.pressed == Some(Target::Import) { pack(72, 72, 74, 255) } else { pack(44, 44, 46, 220) };
            s.fill_rrect(r.0, r.1, r.2, r.3, RADIUS, fill);
            self.label(s, ui, "Import", 14.0, WHITE, r);
        }
        if self.cameras.len() > 1 {
            let r = self.switch_rect(ui);
            let fill = if self.pressed == Some(Target::Switch) { pack(72, 72, 74, 255) } else { PLATE };
            s.fill_rrect(r.0, r.1, r.2, r.3, r.2 / 2.0, fill);
            // two opposed arrows: the bifrost flip glyph
            let (cx, cy, k) = (r.0 + r.2 / 2.0, r.1 + r.3 / 2.0, r.2 / 44.0);
            let t = (2.0 * k).max(1.0);
            s.fill_rrect(cx - 9.0 * k, cy - 6.0 * k, 18.0 * k, t, 0.0, WHITE);
            s.fill_rrect(cx - 9.0 * k, cy + 6.0 * k, 18.0 * k, t, 0.0, WHITE);
            s.fill_rrect(cx + 5.0 * k, cy - 9.0 * k, t, 6.0 * k, 0.0, WHITE);
            s.fill_rrect(cx - 7.0 * k, cy + 7.0 * k, t, 6.0 * k, 0.0, WHITE);
        }
        if let Some((_, img)) = &self.thumb {
            let r = self.thumb_rect(ui);
            let pressed = self.pressed == Some(Target::Thumb);
            draw_scaled(s, img, Rect { x: r.0 as i32, y: r.1 as i32, w: r.2 as i32, h: r.3 as i32 }, r.2 / 2.0, if pressed { 150 } else { 255 });
        }
    }
}

/// The capture pipeline for one frame, on the encoder thread: full-size conversion, aspect
/// crop, JPEG encode with EXIF, atomic write, thumbnail.
fn save_capture(frame: &Frame, dir: &std::path::Path, name: String, aspect: Aspect, zoom: usize, thumb_n: usize, unix: i64, exif: &jpeg::Exif) -> Saved {
    let failed = |name: String| Saved { path: None, name, thumb: None };
    let Some(px) = v4l2::to_pixels(frame, frame.width, frame.height) else { return failed(name) };
    let mut full = Image { w: frame.width, h: frame.height, px };
    // gray-world white balance: a photo keeps its neutrals neutral (the preview is left as the sensor delivers it)
    crate::media::convert::white_balance(&mut full);
    let (x, y, w, h) = zoomed(aspect.crop(full.w, full.h), zoom);
    let img = full.crop(x, y, w, h);
    let bytes = jpeg::encode_pixels(img.w, img.h, &img.px, JPEG_QUALITY, Some(exif));
    if bytes.is_empty() {
        return failed(name);
    }
    match screenshot::write(dir, &name, &bytes, unix) {
        Ok(path) => Saved { path: Some(path), thumb: Some(square_thumb(&img, thumb_n)), name },
        Err(_) => failed(name),
    }
}

/// A centre-cropped square of `n` px.
fn square_thumb(img: &Image, n: usize) -> Image {
    let side = img.w.min(img.h);
    img.crop((img.w - side) / 2, (img.h - side) / 2, side, side).resized(n.max(1), n.max(1))
}

/// The Camera icon: a camera body on dark grey, a layered document (`res/icons/camera`) drawn by the icon renderer.
pub fn icon(size: usize) -> Image {
    const JSON: &str = include_str!("../../../res/icons/camera/icon.json");
    bifrost_render::icon_doc::render_builtin(JSON, &[("camera.png", include_bytes!("../../../res/icons/camera/camera.png"))], size)
        .unwrap_or_else(|| Image { w: size, h: size, px: vec![pack(58, 58, 60, 255); size * size] })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_stack_puts_the_button_below_every_line_of_the_reason() {
        let m = UnavailableMetrics { title_h: 34.0, line_h: 20.0, title_gap: 6.0, button_gap: 24.0, button: (180.0, 56.0) };
        let view = (0.0, 28.0, 800.0, 600.0);
        for lines in 0..=UNAVAILABLE_LINES {
            let (title, rows, button) = unavailable_stack(view, lines, &m);
            assert_eq!(rows.len(), lines);
            let mut bottom = title.1 + title.3;
            for r in &rows {
                assert!(r.1 >= bottom, "line {r:?} overlaps the text above it");
                bottom = r.1 + r.3;
            }
            assert!(button.1 >= bottom + m.button_gap - 0.01, "button {button:?} overlaps the reason (ends at {bottom})");
            // centred in the pane
            let (top, end) = (title.1 - view.1, view.1 + view.3 - (button.1 + button.3));
            assert!((top - end).abs() < 0.01);
            assert!((button.0 + button.2 / 2.0 - view.2 / 2.0).abs() < 0.01);
        }
        // a pane shorter than the stack pins it to the top instead of pushing the title out
        let (title, _, _) = unavailable_stack((0.0, 28.0, 300.0, 50.0), 3, &m);
        assert_eq!(title.1, 28.0);
    }

    #[test]
    fn viewfinder_fit_keeps_the_aspect() {
        assert_eq!(fit(1280, 720, 800, 1000), (800, 450));
        assert_eq!(fit(1280, 720, 2000, 450), (800, 450));
        assert_eq!(fit(0, 0, 10, 20), (10, 20));
    }

    #[test]
    fn saving_a_frame_writes_a_decodable_jpeg_with_exif() {
        let dir = std::env::temp_dir().join(format!("camera-save-{}", std::process::id()));
        // a 32x16 YUYV frame of mid grey
        let data: Vec<u8> = (0..32 * 16).flat_map(|_| [128u8, 128]).collect();
        let frame = Frame { width: 32, height: 16, stride: 64, format: v4l2::YUYV, data };
        let exif = jpeg::Exif { orientation: 1, date: "2026:10:04 00:23:15".into() };
        let saved = save_capture(&frame, &dir, "IMG_test.jpg".into(), Aspect::Square, 1, 8, 1_791_073_395, &exif);
        let path = saved.path.expect("written");
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[..2], &[0xFF, 0xD8]);
        let img = crate::cg::imageio::decode(&bytes).expect("decodes");
        assert_eq!((img.w, img.h), (16, 16), "square crop of the short side");
        assert_eq!(saved.thumb.map(|t| (t.w, t.h)), Some((8, 8)));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn frame(width: usize) -> Frame {
        Frame { width, height: 1, stride: width, format: v4l2::NV21, data: vec![] }
    }

    #[test]
    fn polling_keeps_only_the_newest_frame_and_reports_once() {
        let slot = Arc::new(Mutex::new(Slot::default()));
        let (park_tx, park_rx) = channel::<()>();
        let thread = std::thread::spawn(move || {
            let _ = park_rx.recv();
        });
        let w = Worker { slot: slot.clone(), stop: Arc::new(AtomicBool::new(false)), thread: Some(thread) };
        {
            let mut s = slot.lock().unwrap_or_else(|p| p.into_inner());
            s.opened = true;
            s.frame = Some(frame(1));
            s.frame = Some(frame(2));
        }
        let p = w.poll();
        assert!(p.opened && p.failed.is_none());
        assert_eq!(p.frame.map(|f| f.width), Some(2));
        let again = w.poll();
        assert!(!again.opened && again.frame.is_none() && again.failed.is_none());
        drop(park_tx);
    }

    #[test]
    fn a_camera_thread_that_died_silently_is_a_failure() {
        let thread = std::thread::spawn(|| {});
        while !thread.is_finished() {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let w = Worker { slot: Arc::new(Mutex::new(Slot::default())), stop: Arc::new(AtomicBool::new(false)), thread: Some(thread) };
        assert_eq!(w.poll().failed.as_deref(), Some("The camera stopped unexpectedly"));
    }

    #[test]
    fn tools_default_off() {
        let ui = crate::shell::tests::ui();
        let cam = Camera::new(&ui, 1200, 1920, PathBuf::from("/nonexistent"), Wall::default());
        let d = cam.describe();
        assert!(d.contains("camera_aspect=4:3") && d.contains("camera_timer=0") && d.contains("camera_grid=false") && d.contains("camera_qr=false"), "{d}");
    }

    #[test]
    fn new_camera_touches_no_device_and_is_idle() {
        let ui = crate::shell::tests::ui();
        let cam = Camera::new(&ui, 1200, 1920, PathBuf::from("/nonexistent"), Wall::default());
        assert!(!cam.streaming());
        assert!(cam.describe().contains("camera_state=idle"));
        assert!(cam.next_wake(0.0).is_none(), "nothing polls before the app is opened");
    }

    #[test]
    fn import_opens_and_closes_without_touching_a_device() {
        let ui = crate::shell::tests::ui();
        let mut cam = Camera::new(&ui, 1200, 1920, PathBuf::from("/nonexistent"), Wall::default());
        cam.open_import();
        assert!(cam.describe().contains("camera_import=true"));
        cam.close_import();
        assert!(cam.describe().contains("camera_import=false"));
        assert!(!cam.streaming());
    }

    #[test]
    fn with_no_camera_attached_the_screen_says_so() {
        let ui = crate::shell::tests::ui();
        let mut cam = Camera::new(&ui, 1200, 1920, PathBuf::from("/nonexistent"), Wall::default());
        // no camera is attached on the test host, so the preferred choice falls back to the first and fails
        cam.choose_preferred();
        let d = cam.describe();
        assert!(d.contains("camera_state=unavailable:No camera found"), "{d}");
        assert!(d.contains("camera_nodes=0"), "{d}");
        assert_eq!(cam.next_wake(5.0), Some(5.0 + HOTPLUG_MS), "an unavailable camera rechecks the devices every second");
    }

    #[test]
    fn square_thumb_crops_the_centre() {
        let mut img = Image::new(4, 2);
        for (i, p) in img.px.iter_mut().enumerate() {
            *p = i as u32;
        }
        let t = square_thumb(&img, 2);
        assert_eq!((t.w, t.h), (2, 2));
    }
}
