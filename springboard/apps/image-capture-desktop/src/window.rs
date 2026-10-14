//! Image Capture's window state and geometry: the toolbar, the sources on the left (cameras and SD cards, then
//! scanners), and on the right the pictures of the selected camera or the settings and preview of the selected
//! scanner, with the footer under them. Layout is in points and derives from the window size; the drawing and the
//! taps read the same rectangles. Disk and scanner work go through `worker.rs`; the rules are in `model.rs`.

use crate::model::{self, Action, Model, Reply, Source};
use crate::worker::{self, Link, Request};
use bifrost_app::controls::nav::{Toolbar, ToolbarItem, ToolbarLayout};
use bifrost_app::controls::Rect;
use bifrost_app::image_source::ImageSource;
use bifrost_app::kit::{Image, TextCache, Ui};
use bifrost_app::layout::UNIFIED_BAR_PT;
use bifrost_app::{widgets, TouchKind};
use bifrost_ui::device_browser::DeviceKind;
use std::path::{Path, PathBuf};

/// The sidebar's width (points).
pub const SIDE_PT: f32 = 220.0;
/// One source row (points).
pub const SRC_ROW_PT: f32 = 44.0;
/// The gap above the first source row (points).
const LIST_PAD_PT: f32 = 8.0;
/// The detail pane's header: the source's name (points).
pub const HEAD_PT: f32 = 44.0;
/// One picture row on a camera (points).
pub const FILE_ROW_PT: f32 = 26.0;
/// One setting row of a scanner: resolution, colour, format (points).
pub const SET_ROW_PT: f32 = 34.0;
/// Padding around the detail pane's content (points).
pub const PAD_PT: f32 = 8.0;
/// The footer (points).
pub const FOOT_PT: f32 = 30.0;
/// How often the cameras and scanners are listed again while the window is in the foreground (milliseconds).
pub const POLL_MS: f64 = 4000.0;
/// How soon the loop comes back while a request is in flight (milliseconds).
const BUSY_WAKE_MS: f64 = 50.0;

pub const B_REFRESH: u32 = 1;
pub const B_IMPORT_PICKED: u32 = 2;
pub const B_IMPORT_ALL: u32 = 3;
pub const B_PREVIEW: u32 = 4;
pub const B_SCAN: u32 = 5;

/// A scanner setting row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Setting {
    Dpi,
    Mode,
    Format,
}

/// What a press or a tap landed on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Nothing,
    Toolbar(u32),
    /// The source at this index of the sidebar.
    Source(usize),
    /// The picture at this index of the selected camera's list.
    File(usize),
    Setting(Setting),
}

pub struct ImageCapture {
    pub model: Model,
    link: Link,
    /// Requests sent whose replies have not come back.
    in_flight: usize,
    /// When the sources are listed next (loop milliseconds).
    next_poll: f64,
    /// The window size, in points.
    pub size: (f32, f32),
    pub labels: TextCache,
    /// The target a press started on; the release acts only when it lands on the same one.
    pub press: Target,
    /// Where imports and scans go (the person's home folder).
    pub home: PathBuf,
    /// The decoded preview, shown above the scanner settings.
    pub preview_image: Option<Image>,
    /// The preview scaled to the pane, kept while the pane keeps its size: (width, height) and the image.
    pub preview_scaled: Option<((usize, usize), Image)>,
}

impl ImageCapture {
    pub fn new() -> ImageCapture {
        ImageCapture::with_link(Link::start(), worker::home_dir())
    }

    fn with_link(link: Link, home: PathBuf) -> ImageCapture {
        ImageCapture {
            model: Model::default(),
            link,
            in_flight: 0,
            next_poll: 0.0,
            // The app host configures the actual client size before the first frame.
            size: (0.0, 0.0),
            labels: TextCache::default(),
            press: Target::Nothing,
            home,
            preview_image: None,
            preview_scaled: None,
        }
    }

    pub fn resize(&mut self, w: f32, h: f32) {
        self.size = (w, h);
    }

    fn send(&mut self, request: Request) {
        self.link.send(request);
        self.in_flight += 1;
    }

    /// Lists the cameras and scanners.
    fn refresh(&mut self) {
        self.send(Request::Refresh);
    }

    /// Performs a person's action on the selection.
    pub fn act(&mut self, action: Action) {
        match action {
            Action::Refresh => self.refresh(),
            Action::Import { all } => {
                let paths = self.model.import_paths(all);
                if paths.is_empty() {
                    self.model.status = "Pick some pictures to import".into();
                    return;
                }
                self.model.status = "Importing…".into();
                let home = self.home.clone();
                self.send(Request::Import { paths, home });
            }
            Action::Preview | Action::Scan => {
                let Some(device) = self.model.selected_scanner().map(str::to_owned) else { return };
                let preview = action == Action::Preview;
                // the scanner's job gate refuses a scan that cannot run (no unit, a job already running)
                let started = match self.model.scanner.as_mut() {
                    Some(scanner) if preview => scanner.request_overview(),
                    Some(scanner) => scanner.request_scan(),
                    None => return,
                };
                if let Err(e) = started {
                    self.model.status = model::refusal(e).into();
                    return;
                }
                let settings = self.model.settings();
                self.model.status = if preview { "Scanning a preview…".into() } else { "Scanning…".into() };
                let home = self.home.clone();
                self.send(Request::Scan { device, settings, preview, home });
            }
        }
    }

    /// Shows the source at `index` and asks for what it holds: its pictures, or what the scanner offers.
    fn select(&mut self, index: usize) {
        self.model.select(index);
        self.preview_image = None;
        self.preview_scaled = None;
        let request = match self.model.selected_source() {
            Some(Source { kind: DeviceKind::Camera, id, .. }) => Some(Request::Contents { volume: id.clone() }),
            Some(Source { kind: DeviceKind::Scanner, .. }) => self.model.selected_scanner().map(|device| Request::Capabilities { device: device.to_string() }),
            None => None,
        };
        if let Some(request) = request {
            self.send(request);
        }
    }

    /// Takes the replies that arrived. True when a frame is needed.
    fn poll(&mut self) -> bool {
        let replies = self.link.drain();
        if replies.is_empty() {
            return false;
        }
        self.in_flight = self.in_flight.saturating_sub(replies.len());
        for reply in replies {
            let preview_ready = match &reply {
                Reply::Scanned { preview: true, path } => Some(path.clone()),
                _ => None,
            };
            let volume = self.model.selected_volume().map(str::to_owned);
            self.model.apply(reply, volume.as_deref());
            if let Some(path) = preview_ready {
                self.preview_image = load_preview(&path);
                self.preview_scaled = None;
                self.model.preview = Some(path);
            }
        }
        true
    }

    /// One loop turn in the foreground: takes the replies and lists the sources every `POLL_MS`. True when a frame is
    /// needed.
    pub fn step(&mut self, t: f64) -> bool {
        let changed = self.poll();
        if t >= self.next_poll && self.in_flight == 0 {
            self.next_poll = t + POLL_MS;
            self.refresh();
        }
        changed
    }

    /// When the loop should come back: soon while a request is in flight, else at the next listing.
    pub fn wake_at(&self, t: f64) -> f64 {
        self.next_poll.max(t + BUSY_WAKE_MS)
    }

    pub fn bar_rect(&self) -> Rect {
        Rect { x: 0.0, y: 0.0, w: self.size.0, h: UNIFIED_BAR_PT }
    }

    fn body_h(&self) -> f32 {
        (self.size.1 - UNIFIED_BAR_PT - FOOT_PT).max(0.0)
    }

    /// The sources, under the toolbar on the left.
    pub fn side_rect(&self) -> Rect {
        Rect { x: 0.0, y: UNIFIED_BAR_PT, w: SIDE_PT.min(self.size.0), h: self.body_h() }
    }

    /// The detail pane, on the right of the sources.
    pub fn detail_rect(&self) -> Rect {
        let x = SIDE_PT.min(self.size.0);
        Rect { x, y: UNIFIED_BAR_PT, w: (self.size.0 - x).max(0.0), h: self.body_h() }
    }

    pub fn footer_rect(&self) -> Rect {
        Rect { x: 0.0, y: (self.size.1 - FOOT_PT).max(0.0), w: self.size.0, h: FOOT_PT }
    }

    pub fn source_row(&self, i: usize) -> Rect {
        Rect { x: 0.0, y: UNIFIED_BAR_PT + LIST_PAD_PT + i as f32 * SRC_ROW_PT, w: SIDE_PT, h: SRC_ROW_PT }
    }

    /// The top of the detail pane's rows (below its header).
    pub fn rows_top(&self) -> f32 {
        UNIFIED_BAR_PT + HEAD_PT
    }

    pub fn file_row(&self, i: usize) -> Rect {
        let d = self.detail_rect();
        Rect { x: d.x + PAD_PT, y: self.rows_top() + i as f32 * FILE_ROW_PT, w: (d.w - 2.0 * PAD_PT).max(0.0), h: FILE_ROW_PT }
    }

    pub fn setting_row(&self, k: usize) -> Rect {
        let d = self.detail_rect();
        Rect { x: d.x + PAD_PT, y: self.rows_top() + k as f32 * SET_ROW_PT, w: (d.w - 2.0 * PAD_PT).max(0.0), h: SET_ROW_PT }
    }

    /// The preview's frame: below the three settings, down to the footer.
    pub fn preview_rect(&self) -> Rect {
        let d = self.detail_rect();
        let top = self.rows_top() + 3.0 * SET_ROW_PT + PAD_PT;
        let bottom = UNIFIED_BAR_PT + self.body_h();
        Rect { x: d.x + PAD_PT, y: top, w: (d.w - 2.0 * PAD_PT).max(0.0), h: (bottom - top - PAD_PT).max(0.0) }
    }

    /// The toolbar. Import and scan are offered only when the selection can take them and nothing is in flight.
    pub fn toolbar(&self) -> Toolbar {
        let camera = self.model.selected_volume().is_some();
        let scanner = self.model.scanner_idle();
        let idle = self.in_flight == 0;
        Toolbar::new(vec![
            ToolbarItem::button(B_REFRESH, "Refresh"),
            ToolbarItem::button(B_IMPORT_PICKED, "Import Selected").enabled(camera && idle && !self.model.picked.is_empty()),
            ToolbarItem::button(B_IMPORT_ALL, "Import All").enabled(camera && idle && !self.model.files.is_empty()),
            ToolbarItem::button(B_PREVIEW, "Preview").enabled(scanner && idle),
            ToolbarItem::button(B_SCAN, "Scan").enabled(scanner && idle),
        ])
    }

    pub fn toolbar_layout(&self, ui: &Ui) -> ToolbarLayout {
        widgets::toolbar_layout(ui, self.bar_rect(), &self.toolbar())
    }

    /// The footer's words: the last result, or a hint for the selection.
    pub fn footer_text(&self) -> String {
        if !self.model.status.is_empty() {
            return self.model.status.clone();
        }
        match self.model.selected_source() {
            None if self.model.sources.is_empty() => "No cameras, SD cards or scanners are connected".into(),
            None => "Choose a camera or a scanner".into(),
            Some(s) if s.kind == DeviceKind::Camera => format!("{} pictures on {}", self.model.files.len(), s.name),
            Some(s) => format!("Scanner {}", s.name),
        }
    }

    /// What is under the point `(x, y)`, in points.
    pub fn hit(&self, ui: &Ui, x: f32, y: f32) -> Target {
        if y < UNIFIED_BAR_PT {
            return self.hit_toolbar(ui, x, y);
        }
        self.hit_body(x, y)
    }

    /// What is under `(x, y)` below the toolbar: a source, a picture, or a scanner setting.
    pub fn hit_body(&self, x: f32, y: f32) -> Target {
        if x < SIDE_PT {
            let top = UNIFIED_BAR_PT + LIST_PAD_PT;
            if y >= top {
                let i = ((y - top) / SRC_ROW_PT) as usize;
                if i < self.model.sources.len() {
                    return Target::Source(i);
                }
            }
            return Target::Nothing;
        }
        if y < self.rows_top() {
            return Target::Nothing;
        }
        let k = ((y - self.rows_top()) / FILE_ROW_PT) as usize;
        if self.model.selected_volume().is_some() {
            if k < self.model.files.len() {
                return Target::File(k);
            }
            return Target::Nothing;
        }
        if self.model.selected_scanner().is_some() {
            return match ((y - self.rows_top()) / SET_ROW_PT) as usize {
                0 => Target::Setting(Setting::Dpi),
                1 => Target::Setting(Setting::Mode),
                2 => Target::Setting(Setting::Format),
                _ => Target::Nothing,
            };
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
    pub fn touch(&mut self, ui: &Ui, kind: TouchKind, x: f32, y: f32) -> bool {
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
                self.activate(released)
            }
            TouchKind::Cancel => {
                self.press = Target::Nothing;
                false
            }
        }
    }

    /// Performs what a target asks for. True when the window should redraw.
    pub fn activate(&mut self, target: Target) -> bool {
        match target {
            Target::Nothing => false,
            Target::Toolbar(id) => {
                match id {
                    B_REFRESH => self.act(Action::Refresh),
                    B_IMPORT_PICKED => self.act(Action::Import { all: false }),
                    B_IMPORT_ALL => self.act(Action::Import { all: true }),
                    B_PREVIEW => self.act(Action::Preview),
                    B_SCAN => self.act(Action::Scan),
                    _ => return false,
                }
                true
            }
            Target::Source(i) => {
                self.select(i);
                true
            }
            Target::File(i) => {
                self.model.toggle_pick(i);
                true
            }
            Target::Setting(Setting::Dpi) => {
                self.model.cycle_dpi();
                true
            }
            Target::Setting(Setting::Mode) => {
                self.model.cycle_mode();
                true
            }
            Target::Setting(Setting::Format) => {
                self.model.cycle_format();
                true
            }
        }
    }
}

/// Decodes the preview file. A preview that cannot be read leaves the pane empty.
fn load_preview(path: &Path) -> Option<Image> {
    let bytes = std::fs::read(path).ok()?;
    let source = ImageSource::new(&bytes).ok()?;
    source.decode().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_ui::device_browser::{Device, Transport};
    use bifrost_ui::scanner;

    fn window() -> ImageCapture {
        ImageCapture::with_link(Link::detached(), PathBuf::from("/home/u"))
    }

    fn sources(w: &mut ImageCapture) {
        w.model.sources = vec![
            Source { id: "volume:/run/media/u/EOS".into(), name: "EOS".into(), kind: DeviceKind::Camera },
            Source { id: "scanner:genesys:libusb:001,004".into(), name: "Canon LiDE".into(), kind: DeviceKind::Scanner },
        ];
    }

    #[test]
    fn a_source_row_is_hit_where_it_is_drawn() {
        let mut w = window();
        sources(&mut w);
        let r = w.source_row(1);
        assert_eq!(w.hit_body(r.x + 10.0, r.y + 5.0), Target::Source(1));
        assert_eq!(w.hit_body(10.0, w.source_row(5).y + 5.0), Target::Nothing);
    }

    #[test]
    fn a_picture_row_is_hit_and_a_tap_picks_it() {
        let mut w = window();
        sources(&mut w);
        w.model.select(0);
        w.model.files = vec![crate::model::CameraFile { path: PathBuf::from("/a.jpg"), name: "a.jpg".into(), size: 1 }];
        let r = w.file_row(0);
        assert_eq!(w.hit_body(r.x + 20.0, r.y + 4.0), Target::File(0));
        assert!(w.activate(Target::File(0)));
        assert!(w.model.picked.contains(&0));
    }

    #[test]
    fn the_scanner_settings_are_hit_by_their_rows_and_each_tap_changes_one() {
        let mut w = window();
        sources(&mut w);
        w.model.select(1);
        w.model.apply(Reply::Capabilities { device: "genesys:libusb:001,004".into(), caps: scanner::Capabilities::default() }, None);
        let dpi_row = w.setting_row(0);
        let format_row = w.setting_row(2);
        assert_eq!(w.hit_body(dpi_row.x + 10.0, dpi_row.y + 2.0), Target::Setting(Setting::Dpi));
        assert_eq!(w.hit_body(format_row.x + 10.0, format_row.y + 2.0), Target::Setting(Setting::Format));
        let before = w.model.settings().format;
        w.activate(Target::Setting(Setting::Format));
        assert_ne!(w.model.settings().format, before);
    }

    #[test]
    fn import_with_nothing_picked_says_so_and_sends_nothing() {
        let mut w = window();
        sources(&mut w);
        w.model.select(0);
        w.act(Action::Import { all: false });
        assert_eq!(w.model.status, "Pick some pictures to import");
        assert_eq!(w.in_flight, 0);
    }

    #[test]
    fn scan_needs_a_scanner_and_import_all_needs_pictures() {
        let mut w = window();
        sources(&mut w);
        let ids: Vec<u32> = w.toolbar().items.iter().filter(|i| i.enabled).map(|i| i.id).collect();
        assert_eq!(ids, vec![B_REFRESH], "nothing selected: only Refresh");
        w.model.select(1);
        let ids: Vec<u32> = w.toolbar().items.iter().filter(|i| i.enabled).map(|i| i.id).collect();
        assert_eq!(ids, vec![B_REFRESH], "a scanner that is not read yet cannot scan");
        w.model.apply(Reply::Capabilities { device: "genesys:libusb:001,004".into(), caps: scanner::Capabilities::default() }, None);
        let ids: Vec<u32> = w.toolbar().items.iter().filter(|i| i.enabled).map(|i| i.id).collect();
        assert_eq!(ids, vec![B_REFRESH, B_PREVIEW, B_SCAN]);
    }

    #[test]
    fn the_footer_names_the_selection() {
        let mut w = window();
        assert_eq!(w.footer_text(), "No cameras, SD cards or scanners are connected");
        sources(&mut w);
        assert_eq!(w.footer_text(), "Choose a camera or a scanner");
        w.model.select(1);
        assert_eq!(w.footer_text(), "Scanner Canon LiDE");
    }

    #[test]
    fn a_missing_preview_file_leaves_the_pane_empty() {
        assert!(load_preview(Path::new("/no/such/preview.png")).is_none());
    }

    #[test]
    fn a_device_listing_becomes_sidebar_sources() {
        let mut w = window();
        w.model.apply(
            Reply::Listed {
                volumes: vec![Device { id: "volume:/m/x".into(), name: "x".into(), kind: DeviceKind::Camera, transport: Transport::Other, serial: None }],
                scanners: Ok(vec![]),
            },
            None,
        );
        assert_eq!(w.model.sources.len(), 1);
    }
}
