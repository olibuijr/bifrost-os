//! Photo Booth: the system camera in a viewfinder with Photo, 4-up Burst and Clip modes, a Timer with a countdown and a
//! flash, a row of effects (Core Image style filter chains), a strip of this session's captures and Export through the
//! save panel. Captures are saved in `~/Pictures/Photo Booth Library`; the layout, the effects, the names and the clip
//! container are in `model.rs` and `avi.rs`.

mod avi;
mod model;

use bifrost_app::kit::ui::{self, Ui, Weight};
use bifrost_app::kit::{pack, Damage, Image, Surface};
use bifrost_app::{App, AppMenu, AppMenuItem, Ctx, DocumentEvent, Input, KeyInput, TouchKind};
use model::{Button, Effect, Layout, Mode, Rect, Timer};
use springboard::media::v4l2::CameraInfo;
use springboard::media::v4l2::{self, Device, Frame};
use springboard::shell::camera::jpeg;
use springboard::shell::photos::library::{civil, Date};
use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// JPEG quality of a still, and of each clip frame.
const STILL_QUALITY: u8 = 92;
const CLIP_QUALITY: u8 = 80;
/// The clip frames are taken at most this wide.
const CLIP_MAX_W: usize = 640;
/// Gap between strip thumbnails, in points.
const SLOT_GAP_PT: f32 = 8.0;
/// Frame period while the camera streams, in ms.
const FRAME_MS: f64 = 33.0;
const WHITE: u32 = pack(255, 255, 255, 255);
const BLACK: u32 = pack(0, 0, 0, 255);
/// How long each capture shows in the slideshow, in ms.
const SLIDE_MS: f64 = 3000.0;

const MENU_TAKE: u32 = 1;
const MENU_EXPORT: u32 = 2;
const MENU_DELETE: u32 = 3;
const MENU_FLIP: u32 = 4;
const MENU_FLASH: u32 = 5;
const MENU_SLIDESHOW: u32 = 6;
const MENU_MODE_PHOTO: u32 = 7;
const MENU_MODE_BURST: u32 = 8;
const MENU_MODE_CLIP: u32 = 9;
const MENU_TIMER_OFF: u32 = 10;
const MENU_TIMER_3: u32 = 11;
const MENU_TIMER_10: u32 = 12;
/// Camera menu items: `MENU_CAMERA_BASE + index` into the list of cameras.
const MENU_CAMERA_BASE: u32 = 100;

/// A capture in this session: its file, a small thumbnail for the strip and the image shown when it is selected.
struct Capture {
    path: PathBuf,
    thumb: Image,
    shown: Image,
}

struct Burst {
    shots: Vec<Image>,
    next_at: f64,
}

struct Clip {
    frames: Vec<Vec<u8>>,
    size: (usize, usize),
    first: Option<Image>,
    next_at: f64,
    ends_at: f64,
}

struct PhotoBooth {
    device: Option<Device>,
    /// why there is no live picture, shown in the viewfinder; empty while the camera works
    camera_note: String,
    last_frame: Option<Frame>,
    /// the viewfinder picture, fitted to the viewfinder and with the effect applied
    view: Option<Image>,
    /// the viewfinder size in pixels, from the last draw
    view_px: (usize, usize),
    /// pixels per point and the window size in points, from the last draw
    scale: f32,
    size_pt: (f32, f32),
    mode: Mode,
    timer: Timer,
    effect: Effect,
    /// Auto Flip: the picture is mirrored like a looking glass, in the viewfinder and in every capture
    mirror: bool,
    /// whether the screen flashes at the shutter
    flash: bool,
    /// the cameras the system lists and the one in use (path)
    cameras: Vec<CameraInfo>,
    camera_path: Option<String>,
    /// the slideshow: the capture shown and when it advances
    slideshow: Option<(usize, f64)>,
    /// when the shutter fires, in ms on the app clock; set while the timer counts down
    countdown: Option<f64>,
    burst: Option<Burst>,
    clip: Option<Clip>,
    flash_at: Option<f64>,
    captures: Vec<Capture>,
    selected: Option<usize>,
    /// a save panel is open for this capture (request id, file)
    export: Option<(u32, PathBuf)>,
    library: PathBuf,
    now: f64,
    status: String,
}

impl PhotoBooth {
    fn new() -> Self {
        PhotoBooth {
            device: None,
            camera_note: String::new(),
            last_frame: None,
            view: None,
            view_px: (0, 0),
            scale: 1.0,
            size_pt: (0.0, 0.0),
            mode: Mode::Photo,
            timer: Timer::Off,
            effect: Effect::None,
            mirror: true,
            flash: true,
            cameras: Vec::new(),
            camera_path: None,
            slideshow: None,
            countdown: None,
            burst: None,
            clip: None,
            flash_at: None,
            captures: Vec::new(),
            selected: None,
            export: None,
            library: PathBuf::new(),
            now: 0.0,
            status: String::new(),
        }
    }

    /// Opens the first camera the system lists. Nothing is opened before the app is shown.
    fn start(&mut self) {
        if self.device.is_some() {
            return;
        }
        self.cameras = v4l2::probe();
        let chosen = self.camera_path.as_ref().and_then(|p| self.cameras.iter().find(|c| &c.path == p)).or_else(|| self.cameras.first()).cloned();
        match chosen.as_ref() {
            Some(info) => match Device::open(&info.path) {
                Ok(device) => {
                    self.camera_path = Some(info.path.clone());
                    self.device = Some(device);
                    self.camera_note.clear();
                }
                Err(e) => self.camera_note = format!("Camera not available: {e}"),
            },
            None => self.camera_note = "No camera found".to_string(),
        }
    }

    /// Releases the camera and drops the live picture; a clip in progress is cancelled.
    fn stop(&mut self) {
        self.device = None;
        self.last_frame = None;
        self.view = None;
        self.clip = None;
        self.burst = None;
        self.countdown = None;
    }

    fn timed(&self) -> bool {
        self.countdown.is_some() || self.burst.is_some() || self.clip.is_some() || self.flash_at.is_some()
    }

    /// The newest frame as pixels of `w` by `h`, with the effect applied.
    fn grab(&self, w: usize, h: usize) -> Option<Image> {
        let frame = self.last_frame.as_ref()?;
        let px = v4l2::to_pixels(frame, w, h)?;
        let img = Image { w, h, px };
        let img = if self.mirror { model::flip_horizontal(&img) } else { img };
        Some(self.effect.apply(&img))
    }

    /// The newest frame at full size.
    fn still(&self) -> Option<Image> {
        let frame = self.last_frame.as_ref()?;
        self.grab(frame.width, frame.height)
    }

    /// Rebuilds the viewfinder picture from the newest frame at the viewfinder's size.
    fn refresh_view(&mut self) {
        let (vw, vh) = self.view_px;
        let Some(frame) = self.last_frame.as_ref() else { return };
        if vw == 0 || vh == 0 || frame.width == 0 || frame.height == 0 {
            return;
        }
        let (w, h) = model::fit_size(frame.width, frame.height, vw.min(model::PREVIEW_MAX_W), vh);
        self.view = self.grab(w, h);
    }

    /// The shutter. A timer starts the countdown; a clip already recording ends now.
    fn press_shutter(&mut self, t: f64) {
        if let Some(clip) = self.clip.as_mut() {
            clip.ends_at = t;
            return;
        }
        if self.countdown.is_some() || self.burst.is_some() {
            return;
        }
        self.selected = None;
        let secs = self.timer.secs();
        if secs > 0 {
            self.countdown = Some(t + f64::from(secs) * 1000.0);
        } else {
            self.fire(t);
        }
    }

    /// Takes the picture the mode asks for.
    fn fire(&mut self, t: f64) {
        match self.mode {
            Mode::Photo => self.take_still(t),
            Mode::Burst => self.burst = Some(Burst { shots: Vec::new(), next_at: t }),
            Mode::Clip => self.start_clip(t),
        }
    }

    fn take_still(&mut self, t: f64) {
        let Some(img) = self.still() else {
            self.status = "No camera picture yet".to_string();
            return;
        };
        self.flash_now(t);
        if let Some(path) = self.write_still(&img, "") {
            self.add_capture(path, img);
        }
    }

    fn flash_now(&mut self, t: f64) {
        if self.flash {
            self.flash_at = Some(t);
        }
    }

    /// Deletes the selected capture's file and takes it off the strip. The file goes first: a failure keeps the strip.
    fn delete_selected(&mut self) {
        let Some(index) = self.selected else {
            self.status = "Select a capture to delete".to_string();
            return;
        };
        if let Err(e) = std::fs::remove_file(&self.captures[index].path) {
            if e.kind() != std::io::ErrorKind::NotFound {
                self.status = format!("Could not delete: {e}");
                return;
            }
        }
        let gone = self.captures.remove(index);
        self.selected = model::selection_after_delete(index, self.captures.len());
        self.slideshow = match self.slideshow {
            Some(_) if self.captures.is_empty() => None,
            Some((i, at)) => Some((i.min(self.captures.len() - 1), at)),
            None => None,
        };
        self.status = format!("Deleted {}", gone.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
    }

    /// Switches to the camera listed at `index`; the current one is released first.
    fn use_camera(&mut self, index: usize) {
        let Some(info) = self.cameras.get(index) else { return };
        if self.camera_path.as_deref() == Some(info.path.as_str()) && self.device.is_some() {
            return;
        }
        self.camera_path = Some(info.path.clone());
        self.stop();
        self.start();
    }

    fn start_slideshow(&mut self, t: f64) {
        if self.captures.is_empty() {
            self.status = "Nothing to show yet".to_string();
            return;
        }
        let first = self.selected.unwrap_or(self.captures.len() - 1);
        self.slideshow = Some((first, t + SLIDE_MS));
    }

    fn slide(&mut self, forward: bool, t: f64) {
        if let Some((i, _)) = self.slideshow {
            self.slideshow = Some((model::slide_step(i, self.captures.len(), forward), t + SLIDE_MS));
        }
    }

    fn start_clip(&mut self, t: f64) {
        let Some(frame) = self.last_frame.as_ref() else {
            self.status = "No camera picture yet".to_string();
            return;
        };
        let w = frame.width.min(CLIP_MAX_W).max(2) & !1;
        let h = (w * frame.height / frame.width.max(1)).max(2) & !1;
        self.clip = Some(Clip { frames: Vec::new(), size: (w, h), first: None, next_at: t, ends_at: t + model::CLIP_SECS * 1000.0 });
    }

    /// One clip frame when its time has come; the clip is written when its time runs out.
    fn clip_step(&mut self, t: f64) {
        let Some(mut clip) = self.clip.take() else { return };
        if t >= clip.next_at && t < clip.ends_at {
            let (w, h) = clip.size;
            if let Some(img) = self.grab(w, h) {
                let bytes = jpeg::encode_pixels(img.w, img.h, &img.px, CLIP_QUALITY, None);
                if clip.first.is_none() {
                    clip.first = Some(img.clone());
                }
                clip.frames.push(bytes);
            }
            clip.next_at += 1000.0 / f64::from(model::CLIP_FPS);
        }
        if t >= clip.ends_at {
            self.finish_clip(clip);
        } else {
            self.clip = Some(clip);
        }
    }

    fn finish_clip(&mut self, clip: Clip) {
        let Some(first) = clip.first else {
            self.status = "The clip had no frames".to_string();
            return;
        };
        let (w, h) = clip.size;
        let bytes = avi::mjpeg_avi(&clip.frames, w as u32, h as u32, model::CLIP_FPS);
        let stem = model::capture_stem(&now_date(), " Clip");
        if let Some(path) = self.write_file(&stem, "avi", &bytes) {
            self.add_capture(path, first);
        }
    }

    /// The burst's shots, one by one, then the contact sheet of all of them.
    fn burst_step(&mut self, t: f64) {
        let Some(mut burst) = self.burst.take() else { return };
        if t < burst.next_at {
            self.burst = Some(burst);
            return;
        }
        match self.still() {
            Some(img) => {
                self.flash_now(t);
                let n = burst.shots.len() + 1;
                let _ = self.write_still(&img, &format!(" Burst {n}"));
                burst.shots.push(img);
                burst.next_at = t + model::BURST_GAP_MS;
            }
            None => {
                self.status = "No camera picture yet".to_string();
                return;
            }
        }
        if burst.shots.len() < model::BURST_SHOTS {
            self.burst = Some(burst);
            return;
        }
        let (w, h) = (burst.shots[0].w * 2, burst.shots[0].h * 2);
        let sheet = model::contact_sheet(&burst.shots, w, h);
        if let Some(path) = self.write_still(&sheet, " Burst Sheet") {
            self.add_capture(path, sheet);
        }
    }

    /// Writes a still as JPEG into the library; a failure is shown in the status line.
    fn write_still(&mut self, img: &Image, suffix: &str) -> Option<PathBuf> {
        let bytes = jpeg::encode_pixels(img.w, img.h, &img.px, STILL_QUALITY, None);
        let stem = model::capture_stem(&now_date(), suffix);
        self.write_file(&stem, "jpg", &bytes)
    }

    fn write_file(&mut self, stem: &str, ext: &str, bytes: &[u8]) -> Option<PathBuf> {
        let result = std::fs::create_dir_all(&self.library).and_then(|_| {
            let path = self.library.join(model::free_name(&self.library, stem, ext));
            std::fs::write(&path, bytes).map(|_| path)
        });
        match result {
            Ok(path) => Some(path),
            Err(e) => {
                self.status = format!("Could not save: {e}");
                None
            }
        }
    }

    /// Adds a capture to the strip and selects it.
    fn add_capture(&mut self, path: PathBuf, img: Image) {
        let box_px = (model::THUMB_PT * self.scale).max(1.0) as usize;
        let (tw, th) = model::fit_size(img.w, img.h, box_px, box_px);
        let thumb = img.resized(tw, th);
        let (sw, sh) = model::fit_size(img.w, img.h, model::PREVIEW_MAX_W, model::PREVIEW_MAX_W);
        let shown = if (sw, sh) == (img.w, img.h) { img } else { img.resized(sw, sh) };
        self.status = format!("Saved {}", path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
        self.captures.push(Capture { path, thumb, shown });
        self.selected = Some(self.captures.len() - 1);
    }

    /// Starts the save panel for the selected capture.
    fn export_selected(&mut self, ctx: &Ctx) {
        let Some(index) = self.selected else {
            self.status = "Select a capture to export".to_string();
            return;
        };
        let path = self.captures[index].path.clone();
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let ext = path.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
        let request = ctx.save_document(&name, &[ext.as_str()]);
        self.export = Some((request, path));
    }

    /// Which strip thumbnail sits at `(x, y)` points, as an index into `captures`.
    fn strip_hit(&self, strip: Rect, x: f32, y: f32) -> Option<usize> {
        let (start, count) = strip_visible(strip.w, self.captures.len());
        (0..count).find(|k| strip_slot(strip, *k).contains(x, y)).map(|k| start + k)
    }
}

/// Which captures the strip shows: the newest that fit, as (first index, count).
fn strip_visible(strip_w: f32, total: usize) -> (usize, usize) {
    let slot = model::THUMB_PT + SLOT_GAP_PT;
    let fit = ((strip_w - 2.0 * SLOT_GAP_PT) / slot).floor().max(0.0) as usize;
    let count = total.min(fit);
    (total - count, count)
}

/// The thumbnail box of the `k`-th visible capture in the strip.
fn strip_slot(strip: Rect, k: usize) -> Rect {
    Rect {
        x: strip.x + SLOT_GAP_PT + k as f32 * (model::THUMB_PT + SLOT_GAP_PT),
        y: strip.y + (strip.h - model::THUMB_PT) / 2.0,
        w: model::THUMB_PT,
        h: model::THUMB_PT,
    }
}

fn now_date() -> Date {
    let unix = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    civil(unix, springboard::bare::timezone::offset_at(unix).unwrap_or(0))
}

/// Fills a disc of radius `r` px centred at (`cx`, `cy`) px, one row at a time.
fn fill_disc(s: &mut Surface, cx: f32, cy: f32, r: f32, color: u32) {
    let top = (cy - r).floor() as i32;
    let bottom = (cy + r).ceil() as i32;
    for y in top..bottom {
        let dy = y as f32 + 0.5 - cy;
        let half = (r * r - dy * dy).max(0.0).sqrt();
        if half > 0.0 {
            let x0 = (cx - half).round().max(0.0) as usize;
            let x1 = (cx + half).round().max(0.0) as usize;
            if y >= 0 && x1 > x0 {
                s.fill_rect(x0, y as usize, x1 - x0, 1, color);
            }
        }
    }
}

/// Draws `text` centred on `cx` (px), with its baseline box top at `y` (px).
fn label_centred(s: &mut Surface, ui: &Ui, text: &str, pt: f32, color: u32, cx: f32, y: f32, max_px: f32) {
    let fitted = ui.fit(text, pt, max_px);
    let img = ui.text(&fitted, pt, Weight::Regular, color);
    s.blit_colored(&img, (cx - img.w as f32 / 2.0) as i32, y as i32, color, 255);
}

fn label_at(s: &mut Surface, ui: &Ui, text: &str, pt: f32, color: u32, x: f32, y: f32, max_px: f32) {
    let fitted = ui.fit(text, pt, max_px);
    let img = ui.text(&fitted, pt, Weight::Regular, color);
    s.blit_colored(&img, x as i32, y as i32, color, 255);
}

impl App for PhotoBooth {
    fn id(&self) -> &str {
        "is.olibuijr.photobooth.desktop"
    }

    fn configure(&mut self, _ctx: &Ctx) {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        self.library = model::library_dir(&home);
        self.start();
    }

    fn resume(&mut self, _ctx: &Ctx) {
        self.start();
    }

    fn suspend(&mut self, _ctx: &Ctx) {
        self.stop();
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        let scale = ctx.ui.s;
        if self.slideshow.is_some() {
            match ev {
                Input::Key(KeyInput::Code(1 | 27)) | Input::Touch { kind: TouchKind::Down, .. } => self.slideshow = None,
                Input::Key(KeyInput::Code(37)) => self.slide(false, self.now),
                Input::Key(KeyInput::Code(39)) => self.slide(true, self.now),
                _ => return false,
            }
            return true;
        }
        match ev {
            Input::Key(KeyInput::Code(8 | 46)) => {
                if self.selected.is_none() {
                    return false;
                }
                self.delete_selected();
                true
            }
            Input::Touch { kind: TouchKind::Down, x, y, .. } => {
                let (x, y) = (x / scale, y / scale);
                let Some(layout) = self.layout() else { return false };
                if layout.controls.contains(x, y) {
                    for button in [Button::Mode, Button::Timer, Button::Export, Button::Shutter] {
                        if model::button_rect(layout.controls, button).contains(x, y) {
                            match button {
                                Button::Shutter => self.press_shutter(self.now),
                                Button::Mode if self.countdown.is_none() && self.burst.is_none() && self.clip.is_none() => self.mode = self.mode.next(),
                                Button::Timer => self.timer = self.timer.next(),
                                Button::Export => self.export_selected(ctx),
                                _ => {}
                            }
                            return true;
                        }
                    }
                    return false;
                }
                if layout.effects.contains(x, y) {
                    if let Some((effect, _)) = model::effect_cells(layout.effects).into_iter().find(|(_, r)| r.contains(x, y)) {
                        self.effect = effect;
                        self.view = None;
                        return true;
                    }
                    return false;
                }
                if layout.strip.contains(x, y) {
                    if let Some(index) = self.strip_hit(layout.strip, x, y) {
                        self.selected = Some(index);
                        return true;
                    }
                    return false;
                }
                if layout.viewfinder.contains(x, y) && self.selected.is_some() {
                    self.selected = None;
                    return true;
                }
                false
            }
            Input::Key(KeyInput::Text(text)) if text == " " => {
                self.press_shutter(self.now);
                true
            }
            _ => false,
        }
    }

    fn main_menu(&self, _ctx: &Ctx) -> Vec<AppMenu> {
        let busy = self.clip.is_some() || self.burst.is_some() || self.countdown.is_some();
        let has_sel = self.selected.is_some();
        let mode = |id: u32, title: &str, m: Mode| AppMenuItem::new(id, title).checked(self.mode == m).enabled(!busy);
        let timer = |id: u32, title: &str, t: Timer| AppMenuItem::new(id, title).checked(self.timer == t);
        let mut cameras: Vec<AppMenuItem> = self
            .cameras
            .iter()
            .enumerate()
            .map(|(i, c)| AppMenuItem::new(MENU_CAMERA_BASE + i as u32, &c.card).checked(self.camera_path.as_deref() == Some(c.path.as_str())))
            .collect();
        if cameras.is_empty() {
            cameras.push(AppMenuItem::new(MENU_CAMERA_BASE, "No Camera").enabled(false));
        }
        vec![
            AppMenu {
                title: "File".into(),
                items: vec![
                    AppMenuItem::new(MENU_TAKE, if self.clip.is_some() { "Stop Recording" } else { "Take Photo" }).key("cmd+t"),
                    AppMenuItem::separator(),
                    AppMenuItem::new(MENU_EXPORT, "Export...").key("cmd+shift+e").enabled(has_sel),
                    AppMenuItem::new(MENU_DELETE, "Delete").enabled(has_sel),
                ],
            },
            AppMenu {
                title: "View".into(),
                items: vec![
                    mode(MENU_MODE_PHOTO, "Photo", Mode::Photo),
                    mode(MENU_MODE_BURST, "Four-Up", Mode::Burst),
                    mode(MENU_MODE_CLIP, "Video Clip", Mode::Clip),
                    AppMenuItem::separator(),
                    timer(MENU_TIMER_OFF, "Countdown Off", Timer::Off),
                    timer(MENU_TIMER_3, "Countdown 3 Seconds", Timer::Three),
                    timer(MENU_TIMER_10, "Countdown 10 Seconds", Timer::Ten),
                    AppMenuItem::separator(),
                    AppMenuItem::new(MENU_FLIP, "Auto Flip").checked(self.mirror),
                    AppMenuItem::new(MENU_FLASH, "Screen Flash").checked(self.flash),
                    AppMenuItem::new(MENU_SLIDESHOW, "Play Slideshow").key("cmd+alt+s").enabled(!self.captures.is_empty()),
                ],
            },
            AppMenu { title: "Camera".into(), items: cameras },
        ]
    }

    fn menu_action(&mut self, ctx: &Ctx, id: u32) -> bool {
        match id {
            MENU_TAKE => self.press_shutter(self.now),
            MENU_EXPORT => self.export_selected(ctx),
            MENU_DELETE => self.delete_selected(),
            MENU_FLIP => {
                self.mirror = !self.mirror;
                self.view = None;
            }
            MENU_FLASH => self.flash = !self.flash,
            MENU_SLIDESHOW => self.start_slideshow(self.now),
            MENU_MODE_PHOTO | MENU_MODE_BURST | MENU_MODE_CLIP => {
                if self.countdown.is_none() && self.burst.is_none() && self.clip.is_none() {
                    self.mode = match id {
                        MENU_MODE_PHOTO => Mode::Photo,
                        MENU_MODE_BURST => Mode::Burst,
                        _ => Mode::Clip,
                    };
                }
            }
            MENU_TIMER_OFF => self.timer = Timer::Off,
            MENU_TIMER_3 => self.timer = Timer::Three,
            MENU_TIMER_10 => self.timer = Timer::Ten,
            n if n >= MENU_CAMERA_BASE => self.use_camera((n - MENU_CAMERA_BASE) as usize),
            _ => return false,
        }
        true
    }

    fn document(&mut self, ctx: &Ctx, ev: DocumentEvent) -> bool {
        match ev {
            DocumentEvent::SaveTo(mut doc) => {
                let Some((request, path)) = self.export.clone() else { return false };
                if doc.request != request {
                    return false;
                }
                let written = std::fs::read(&path).and_then(|bytes| doc.file.write_all(&bytes));
                match written {
                    Ok(()) => ctx.commit_document(request),
                    Err(e) => {
                        ctx.abort_document(request);
                        self.status = format!("Could not export: {e}");
                        self.export = None;
                    }
                }
                true
            }
            DocumentEvent::Saved(doc) => {
                if self.export.as_ref().is_some_and(|(r, _)| *r == doc.request) {
                    self.status = format!("Exported {}", doc.name);
                    self.export = None;
                    return true;
                }
                false
            }
            DocumentEvent::Failed { request, .. } => {
                if self.export.as_ref().is_some_and(|(r, _)| *r == request) {
                    self.status = "Export cancelled".to_string();
                    self.export = None;
                    return true;
                }
                false
            }
            _ => false,
        }
    }

    fn tick(&mut self, _ctx: &Ctx, t: f64) -> bool {
        self.now = t;
        let mut dirty = false;
        if let Some((i, at)) = self.slideshow {
            dirty = true;
            if t >= at {
                self.slideshow = Some((model::slide_step(i, self.captures.len(), true), t + SLIDE_MS));
            }
        }
        let polled = match self.device.as_mut() {
            Some(device) => device.latest(),
            None => Ok(None),
        };
        {
            match polled {
                Ok(Some(frame)) => {
                    self.last_frame = Some(frame);
                    self.refresh_view();
                    dirty = true;
                }
                Ok(None) => {}
                Err(e) => {
                    self.device = None;
                    self.camera_note = format!("Camera stopped: {e}");
                    dirty = true;
                }
            }
        }
        if let Some(fire) = self.countdown {
            dirty = true;
            if t >= fire {
                self.countdown = None;
                self.fire(t);
            }
        }
        if self.burst.is_some() {
            dirty = true;
            self.burst_step(t);
        }
        if self.clip.is_some() {
            dirty = true;
            self.clip_step(t);
        }
        if self.flash_at.is_some_and(|f| t - f >= model::FLASH_MS) {
            self.flash_at = None;
        }
        if self.flash_at.is_some() {
            dirty = true;
        }
        dirty
    }

    fn next_wake(&self, t: f64) -> Option<f64> {
        if self.device.is_some() || self.timed() || self.slideshow.is_some() {
            Some(t + FRAME_MS)
        } else {
            None
        }
    }

    fn draw(&mut self, ctx: &Ctx, s: &mut Surface, _t: f64) -> Damage {
        let ui = &ctx.ui;
        let scale = ui.s;
        self.scale = scale;
        let (w, h) = (s.w, s.h);
        self.size_pt = (w as f32 / scale, h as f32 / scale);
        let layout = model::layout(self.size_pt.0, self.size_pt.1);
        if let Some((i, _)) = self.slideshow {
            s.fill_rect(0, 0, w, h, BLACK);
            if let Some(c) = self.captures.get(i) {
                let img = &c.shown;
                s.blit(img, (w as i32 - img.w as i32) / 2, (h as i32 - img.h as i32) / 2, 255, 255);
            }
            let note = format!("{} of {}   Esc to stop", i + 1, self.captures.len());
            label_at(s, ui, &note, ui::MAC_FOOTNOTE_PT, WHITE, 14.0 * scale, 12.0 * scale, w as f32 - 28.0 * scale);
            let mut d = Damage::new(w, h);
            d.all();
            return d;
        }
        s.fill_rect(0, 0, w, h, ui::DETAIL_BG.now());

        self.draw_viewfinder(s, ui, layout.viewfinder, scale);
        self.draw_effects(s, ui, layout.effects, scale);
        self.draw_strip(s, ui, layout.strip, scale);
        self.draw_controls(s, ui, layout.controls, scale);

        let mut d = Damage::new(w, h);
        d.all();
        d
    }
}

impl PhotoBooth {
    /// The layout of the last drawn window, or none before the first draw.
    fn layout(&self) -> Option<Layout> {
        let (w, h) = self.size_pt;
        if w <= 0.0 || h <= 0.0 {
            return None;
        }
        Some(model::layout(w, h))
    }

    fn draw_viewfinder(&mut self, s: &mut Surface, ui: &Ui, vf: Rect, scale: f32) {
        let p = |v: f32| (v * scale).round().max(0.0) as usize;
        let (x0, y0, vw, vh) = (p(vf.x), p(vf.y), p(vf.w), p(vf.h));
        self.view_px = (vw, vh);
        s.fill_rect(x0, y0, vw, vh, BLACK);
        let centre_x = vf.x * scale + vw as f32 / 2.0;

        let picture = match self.selected {
            Some(i) => self.captures.get(i).map(|c| &c.shown),
            None => self.view.as_ref(),
        };
        match picture {
            Some(img) => {
                let x = x0 as i32 + (vw as i32 - img.w as i32) / 2;
                let y = y0 as i32 + (vh as i32 - img.h as i32) / 2;
                s.blit(img, x, y, 255, 255);
            }
            None => {
                let note = if self.camera_note.is_empty() { "Starting the camera" } else { self.camera_note.as_str() };
                let line_y = y0 as f32 + vh as f32 / 2.0 - 8.0 * scale;
                label_centred(s, ui, note, ui::MAC_BODY_PT, ui::LABEL_2.now(), centre_x, line_y, ui.pt(vf.w - 24.0));
            }
        }

        if let Some(fire) = self.countdown {
            let left = ((fire - self.now) / 1000.0).ceil().max(1.0) as u32;
            let size = ui::MAC_LARGE_TITLE_PT * 3.0;
            label_centred(s, ui, &left.to_string(), size, WHITE, centre_x, y0 as f32 + vh as f32 / 2.0 - size * scale / 2.0, vw as f32);
        }
        if self.flash_at.is_some() {
            s.fill_rect(x0, y0, vw, vh, WHITE);
        }
        if let Some(clip) = &self.clip {
            let left = ((clip.ends_at - self.now) / 1000.0).ceil().max(0.0) as u32;
            fill_disc(s, p(vf.x + 14.0) as f32 + 5.0 * scale, p(vf.y + 18.0) as f32, 5.0 * scale, ui::DESTRUCTIVE);
            label_at(s, ui, &format!("REC {left} s"), ui::MAC_FOOTNOTE_PT, WHITE, p(vf.x + 26.0) as f32, p(vf.y + 12.0) as f32, ui.pt(vf.w - 40.0));
        } else if let Some(burst) = &self.burst {
            let n = (burst.shots.len() + 1).min(model::BURST_SHOTS);
            label_at(s, ui, &format!("Burst {n} of {}", model::BURST_SHOTS), ui::MAC_FOOTNOTE_PT, WHITE, p(vf.x + 14.0) as f32, p(vf.y + 12.0) as f32, ui.pt(vf.w - 28.0));
        }
        if !self.status.is_empty() {
            let y = y0 as f32 + vh as f32 - ui::MAC_FOOTNOTE_PT * 2.4 * scale;
            label_at(s, ui, &self.status, ui::MAC_FOOTNOTE_PT, WHITE, p(vf.x + 14.0) as f32, y, ui.pt(vf.w - 28.0));
        }
    }

    fn draw_effects(&self, s: &mut Surface, ui: &Ui, row: Rect, scale: f32) {
        let p = |v: f32| (v * scale).round().max(0.0) as usize;
        s.fill_rect(p(row.x), p(row.y), p(row.w), p(row.h), ui::SIDEBAR_BG.now());
        for (effect, cell) in model::effect_cells(row) {
            if effect == self.effect {
                s.fill_rect(p(cell.x + 3.0), p(cell.y + 6.0), p(cell.w - 6.0), p(cell.h - 12.0), ui::SIDEBAR_SELECTED);
            }
            let colour = if effect == self.effect { ui::LABEL.now() } else { ui::LABEL_2.now() };
            label_centred(s, ui, effect.title(), ui::MAC_FOOTNOTE_PT, colour, (cell.x + cell.w / 2.0) * scale, cell.y * scale + 14.0 * scale, cell.w * scale - 6.0 * scale);
        }
    }

    fn draw_strip(&self, s: &mut Surface, ui: &Ui, strip: Rect, scale: f32) {
        let p = |v: f32| (v * scale).round().max(0.0) as usize;
        s.fill_rect(p(strip.x), p(strip.y), p(strip.w), p(strip.h), ui::SIDEBAR_BG.now());
        if self.captures.is_empty() {
            label_centred(s, ui, "Captures appear here", ui::MAC_FOOTNOTE_PT, ui::LABEL_2.now(), (strip.x + strip.w / 2.0) * scale, strip.y * scale + strip.h * scale / 2.0 - 6.0 * scale, strip.w * scale);
            return;
        }
        let (start, count) = strip_visible(strip.w, self.captures.len());
        for k in 0..count {
            let index = start + k;
            let slot = strip_slot(strip, k);
            let thumb = &self.captures[index].thumb;
            if self.selected == Some(index) {
                s.fill_rect(p(slot.x).saturating_sub(2), p(slot.y).saturating_sub(2), thumb.w + 4, thumb.h + 4, ui::LINK);
            }
            let x = p(slot.x) as i32 + (p(slot.w) as i32 - thumb.w as i32) / 2;
            let y = p(slot.y) as i32 + (p(slot.h) as i32 - thumb.h as i32) / 2;
            s.blit(thumb, x, y, 255, 255);
        }
    }

    fn draw_controls(&self, s: &mut Surface, ui: &Ui, bar: Rect, scale: f32) {
        let p = |v: f32| (v * scale).round().max(0.0) as usize;
        s.fill_rect(p(bar.x), p(bar.y), p(bar.w), p(bar.h), ui::SIDEBAR_BG.now());
        let text_y = |r: Rect| (r.y + 6.0) * scale;
        let mode = model::button_rect(bar, Button::Mode);
        label_centred(s, ui, self.mode.title(), ui::MAC_BODY_PT, ui::LINK, (mode.x + mode.w / 2.0) * scale, text_y(mode), mode.w * scale);
        let timer = model::button_rect(bar, Button::Timer);
        label_centred(s, ui, self.timer.title(), ui::MAC_BODY_PT, ui::LINK, (timer.x + timer.w / 2.0) * scale, text_y(timer), timer.w * scale);
        let export = model::button_rect(bar, Button::Export);
        let export_colour = if self.selected.is_some() { ui::LINK } else { ui::LABEL_2.now() };
        label_centred(s, ui, "Export", ui::MAC_BODY_PT, export_colour, (export.x + export.w / 2.0) * scale, text_y(export), export.w * scale);

        let shutter = model::button_rect(bar, Button::Shutter);
        let (cx, cy) = ((shutter.x + shutter.w / 2.0) * scale, (shutter.y + shutter.h / 2.0) * scale);
        let r = shutter.w * scale / 2.0;
        fill_disc(s, cx, cy, r, ui::LABEL.now());
        fill_disc(s, cx, cy, r - 3.0 * scale, if self.clip.is_some() { ui::DESTRUCTIVE } else { WHITE });
    }
}

fn main() {
    bifrost_app::run(PhotoBooth::new());
}
