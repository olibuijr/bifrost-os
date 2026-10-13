//! Photo Booth: the system camera in a viewfinder with Photo, 4-up Burst and Clip modes, a Timer with a countdown and a
//! flash, a row of effects (Core Image style filter chains), a strip of this session's captures and Export through the
//! save panel. Captures are saved in `~/Pictures/Photo Booth Library`; the layout, the effects, the names and the clip
//! container are in `model.rs` and `avi.rs`. The menus (File, Edit, View, Camera) are in `menu.rs`; the distortion
//! effects and the mirror flip in `warp.rs`; the slideshow's timing in `slideshow.rs`; the backdrops, which swap the
//! room for a gradient using a picture of the empty scene, in `backdrop.rs`.

mod avi;
mod backdrop;
mod menu;
mod model;
mod slideshow;
mod warp;

use backdrop::{Backdrop, Reference};
use bifrost_app::defaults::Value;
use bifrost_app::kit::ui::{self, Ui, Weight};
use bifrost_app::kit::{pack, Damage, Image, Surface};
use bifrost_app::{App, AppMenu, Ctx, DocumentEvent, Input, KeyInput, TouchKind};
use menu::{Command, MenuState};
use model::{Button, Effect, Layout, Mode, Rect, Timer};
use slideshow::Slideshow;
use springboard::media::v4l2::{self, CameraInfo, Device, Frame};
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
/// Keys, as the evdev translation hands them to an app.
const KEY_ESCAPE: u32 = 1;
const KEY_BACKSPACE: u32 = 8;
const KEY_ENTER: u32 = 13;
const KEY_ESCAPE_ASCII: u32 = 27;
const KEY_LEFT: u32 = 37;
const KEY_RIGHT: u32 = 39;
const KEY_DELETE: u32 = 46;
/// Preference keys in the app's defaults.
const PREF_AUTO_FLIP: &str = "autoFlip";
const PREF_SCREEN_FLASH: &str = "screenFlash";
const PREF_CAMERA: &str = "camera";

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
    /// the cameras the system lists, and the device node of the one in use or last chosen
    cameras: Vec<CameraInfo>,
    camera_path: Option<String>,
    /// Edit > Auto Flip: pictures are turned left to right, in the viewfinder and in every capture
    auto_flip: bool,
    /// View > Screen Flash: the white flash over the viewfinder when a photo is taken
    screen_flash: bool,
    /// View > Full Screen Viewfinder: the bars are hidden and the picture fills the window
    full_screen: bool,
    slideshow: Option<Slideshow>,
    /// the slide fitted to the window, kept until the slide or the window size changes: (index, width, height)
    slide: Option<((usize, usize, usize), Image)>,
    /// the backdrop that replaces the room, the picture of the empty scene it needs, and when that picture is taken
    backdrop: Backdrop,
    reference: Option<Reference>,
    scene_at: Option<f64>,
    prefs_loaded: bool,
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
            cameras: Vec::new(),
            camera_path: None,
            auto_flip: true,
            screen_flash: true,
            full_screen: false,
            slideshow: None,
            slide: None,
            backdrop: Backdrop::None,
            reference: None,
            scene_at: None,
            prefs_loaded: false,
        }
    }

    /// Lists the cameras and opens the one chosen last, or the first. Nothing is opened before the app is shown.
    fn start(&mut self) {
        if self.device.is_some() {
            return;
        }
        self.cameras = v4l2::probe();
        let wanted = self.camera_path.as_ref().and_then(|path| self.cameras.iter().position(|c| &c.path == path));
        let Some(info) = self.cameras.get(wanted.unwrap_or(0)) else {
            self.camera_note = "No camera found".to_string();
            return;
        };
        match Device::open(&info.path) {
            Ok(device) => {
                self.device = Some(device);
                self.camera_path = Some(info.path.clone());
                self.camera_note.clear();
            }
            Err(e) => self.camera_note = format!("Camera not available: {e}"),
        }
    }

    /// Camera menu: switches to the `index`-th camera in the list and remembers the choice.
    fn select_camera(&mut self, ctx: &Ctx, index: usize) -> bool {
        let Some(path) = self.cameras.get(index).map(|c| c.path.clone()) else { return false };
        if self.device.is_some() && self.camera_path.as_deref() == Some(path.as_str()) {
            return false;
        }
        ctx.defaults_mut().set_string(PREF_CAMERA, &path);
        self.camera_path = Some(path);
        self.stop();
        self.start();
        // another camera sees another scene, so the empty scene has to be taken again
        self.reference = None;
        if self.backdrop != Backdrop::None {
            self.schedule_scene();
        }
        true
    }

    /// The index in the camera list of the camera in use.
    fn current_camera(&self) -> Option<usize> {
        let path = self.camera_path.as_ref()?;
        self.cameras.iter().position(|c| &c.path == path)
    }

    /// Releases the camera and drops the live picture; a clip in progress is cancelled.
    fn stop(&mut self) {
        self.device = None;
        self.last_frame = None;
        self.view = None;
        self.clip = None;
        self.burst = None;
        self.countdown = None;
        self.scene_at = None;
    }

    fn timed(&self) -> bool {
        self.countdown.is_some()
            || self.burst.is_some()
            || self.clip.is_some()
            || self.flash_at.is_some()
            || self.scene_at.is_some()
            || self.slideshow.as_ref().is_some_and(|show| !show.paused())
    }

    /// The newest frame as pixels of `w` by `h`: the backdrop first (it compares against the camera's own picture),
    /// then Auto Flip, then the effect.
    fn grab(&self, w: usize, h: usize) -> Option<Image> {
        let frame = self.last_frame.as_ref()?;
        let px = v4l2::to_pixels(frame, w, h)?;
        let mut img = Image { w, h, px };
        if self.backdrop != Backdrop::None {
            if let Some(reference) = self.reference.as_ref().filter(|r| r.fits(frame.width, frame.height)) {
                img = backdrop::replace(&img, reference, self.backdrop);
            }
        }
        if self.auto_flip {
            img = warp::flip_horizontal(&img);
        }
        Some(self.effect.apply(img))
    }

    /// Picks a backdrop. A backdrop needs a picture of the empty scene, so the person gets a few seconds to step out.
    fn set_backdrop(&mut self, backdrop: Backdrop) {
        self.backdrop = backdrop;
        self.reference = None;
        self.scene_at = None;
        self.view = None;
        if backdrop != Backdrop::None {
            self.schedule_scene();
        }
    }

    fn schedule_scene(&mut self) {
        self.scene_at = Some(self.now + backdrop::SETTLE_MS);
        self.status = "Step out of the frame".to_string();
    }

    /// Takes the picture of the empty scene from the newest camera frame.
    fn take_scene(&mut self) {
        let Some(frame) = self.last_frame.as_ref() else {
            self.status = "No camera picture yet".to_string();
            return;
        };
        let Some(px) = v4l2::to_pixels(frame, frame.width, frame.height) else {
            self.status = "No camera picture yet".to_string();
            return;
        };
        self.reference = Some(Reference::from_picture(&Image { w: frame.width, h: frame.height, px }));
        self.status = format!("Backdrop: {}", self.backdrop.title());
    }

    /// Starts the screen flash when it is switched on.
    fn flash(&mut self, t: f64) {
        if self.screen_flash {
            self.flash_at = Some(t);
        }
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
        if self.countdown.is_some() || self.burst.is_some() || self.slideshow.is_some() {
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
        self.flash(t);
        if let Some(path) = self.write_still(&img, "") {
            self.add_capture(path, img);
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
                self.flash(t);
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

    /// Removes the selected capture's file and takes it out of the strip; the next capture is selected.
    fn delete_selected(&mut self) {
        let Some(index) = self.selected.filter(|i| *i < self.captures.len()) else {
            self.status = "Select a capture to delete".to_string();
            return;
        };
        if let Err(e) = std::fs::remove_file(&self.captures[index].path) {
            // a file that is already gone is not a reason to keep the capture in the strip
            if e.kind() != std::io::ErrorKind::NotFound {
                self.status = format!("Could not delete: {e}");
                return;
            }
        }
        let gone = self.captures.remove(index);
        self.slide = None;
        self.selected = model::selection_after_delete(index, self.captures.len());
        self.status = format!("Deleted {}", gone.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
    }

    /// View > Slideshow: all captures of the session, one after the other, over the whole window.
    fn start_slideshow(&mut self) {
        if self.captures.is_empty() {
            self.status = "Nothing to show yet".to_string();
            return;
        }
        let start = self.selected.unwrap_or(0).min(self.captures.len() - 1);
        self.slideshow = Some(Slideshow::start(start, self.now));
        self.slide = None;
    }

    /// Ends the slideshow and selects the capture it stopped on.
    fn stop_slideshow(&mut self) {
        if let Some(show) = self.slideshow.take() {
            if show.index < self.captures.len() {
                self.selected = Some(show.index);
            }
        }
        self.slide = None;
    }

    fn set_full_screen(&mut self, on: bool) {
        self.full_screen = on;
        self.view = None;
        self.status = if on { "Press Escape to show the controls again".to_string() } else { String::new() };
    }

    /// Runs a command from the menu bar; true when the window needs a new frame.
    fn command(&mut self, ctx: &Ctx, command: Command) -> bool {
        match command {
            Command::TakePhoto => self.press_shutter(self.now),
            Command::Export => self.export_selected(ctx),
            Command::ShowLibrary => {
                let _ = std::fs::create_dir_all(&self.library);
                ctx.open_url(&format!("file://{}", self.library.display()));
            }
            Command::Delete => self.delete_selected(),
            Command::AutoFlip => {
                self.auto_flip = !self.auto_flip;
                ctx.defaults_mut().set_bool(PREF_AUTO_FLIP, self.auto_flip);
                self.view = None;
            }
            Command::FullScreen => self.set_full_screen(!self.full_screen),
            Command::Slideshow => {
                if self.slideshow.is_some() {
                    self.stop_slideshow();
                } else {
                    self.start_slideshow();
                }
            }
            Command::ScreenFlash => {
                self.screen_flash = !self.screen_flash;
                ctx.defaults_mut().set_bool(PREF_SCREEN_FLASH, self.screen_flash);
            }
            Command::RetakeScene => {
                self.reference = None;
                self.schedule_scene();
            }
            Command::Backdrop(backdrop) => self.set_backdrop(backdrop),
            Command::Camera(index) => return self.select_camera(ctx, index),
        }
        true
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

    fn configure(&mut self, ctx: &Ctx) {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        self.library = model::library_dir(&home);
        if !self.prefs_loaded {
            self.prefs_loaded = true;
            let mut defaults = ctx.defaults_mut();
            defaults.register([(PREF_AUTO_FLIP, Value::Bool(true)), (PREF_SCREEN_FLASH, Value::Bool(true))]);
            self.auto_flip = defaults.bool(PREF_AUTO_FLIP);
            self.screen_flash = defaults.bool(PREF_SCREEN_FLASH);
            self.camera_path = defaults.string(PREF_CAMERA);
        }
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
            return self.slideshow_event(ev);
        }
        match ev {
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
            Input::Key(KeyInput::Code(KEY_DELETE | KEY_BACKSPACE)) if self.selected.is_some() => {
                self.delete_selected();
                true
            }
            Input::Key(KeyInput::Code(KEY_ESCAPE | KEY_ESCAPE_ASCII)) if self.full_screen => {
                self.set_full_screen(false);
                true
            }
            Input::Key(KeyInput::Code(KEY_ENTER)) if self.full_screen => {
                self.press_shutter(self.now);
                true
            }
            _ => false,
        }
    }

    fn main_menu(&self, _ctx: &Ctx) -> Vec<AppMenu> {
        let cameras: Vec<String> = self.cameras.iter().map(|c| c.card.clone()).collect();
        let browsing = self.slideshow.is_none();
        menu::menus(&MenuState {
            live: self.device.is_some() && browsing,
            selected: self.selected.is_some() && browsing,
            has_captures: !self.captures.is_empty(),
            auto_flip: self.auto_flip,
            screen_flash: self.screen_flash,
            full_screen: self.full_screen,
            slideshow: !browsing,
            backdrop: self.backdrop,
            cameras: &cameras,
            current_camera: self.current_camera(),
        })
    }

    fn menu_action(&mut self, ctx: &Ctx, id: u32) -> bool {
        match menu::decode(id) {
            Some(command) => self.command(ctx, command),
            None => false,
        }
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
        let polled = match self.device.as_mut() {
            Some(device) => device.latest(),
            None => Ok(None),
        };
        {
            match polled {
                Ok(Some(frame)) => {
                    self.last_frame = Some(frame);
                    // the slideshow covers the viewfinder, so the live picture is not rebuilt behind it
                    if self.slideshow.is_none() {
                        self.refresh_view();
                    }
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
        if let Some(at) = self.scene_at {
            dirty = true;
            if t >= at {
                self.scene_at = None;
                self.take_scene();
            }
        }
        let count = self.captures.len();
        if let Some(show) = self.slideshow.as_mut() {
            if show.tick(count, t) {
                dirty = true;
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
        if self.device.is_some() || self.timed() {
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
        let layout = self.window_layout(self.size_pt.0, self.size_pt.1);
        s.fill_rect(0, 0, w, h, ui::DETAIL_BG.now());

        if self.slideshow.is_some() {
            self.draw_slideshow(s, ui, scale);
        } else {
            self.draw_viewfinder(s, ui, layout.viewfinder, scale);
            if !self.full_screen {
                self.draw_effects(s, ui, layout.effects, scale);
                self.draw_strip(s, ui, layout.strip, scale);
                self.draw_controls(s, ui, layout.controls, scale);
            }
        }

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
        Some(self.window_layout(w, h))
    }

    /// The window split into its parts, or the viewfinder alone while it fills the window.
    fn window_layout(&self, w: f32, h: f32) -> Layout {
        if self.full_screen {
            model::layout_full_screen(w, h)
        } else {
            model::layout(w, h)
        }
    }

    /// Keys and clicks while the slideshow is up: arrows step, space pauses, Escape or a click ends it.
    fn slideshow_event(&mut self, ev: Input) -> bool {
        let t = self.now;
        match ev {
            Input::Touch { kind: TouchKind::Down, .. } | Input::Key(KeyInput::Code(KEY_ESCAPE | KEY_ESCAPE_ASCII)) => {
                self.stop_slideshow();
                true
            }
            Input::Key(KeyInput::Code(KEY_RIGHT)) => self.step_slide(1),
            Input::Key(KeyInput::Code(KEY_LEFT)) => self.step_slide(-1),
            Input::Key(KeyInput::Text(text)) if text == " " => {
                if let Some(show) = self.slideshow.as_mut() {
                    show.toggle_pause(t);
                }
                true
            }
            _ => false,
        }
    }

    /// Moves the slideshow `delta` captures on or back.
    fn step_slide(&mut self, delta: i32) -> bool {
        let (count, t) = (self.captures.len(), self.now);
        if let Some(show) = self.slideshow.as_mut() {
            show.step(delta, count, t);
        }
        true
    }

    /// The slideshow: the capture on screen, as large as the window allows, with its name and position underneath.
    fn draw_slideshow(&mut self, s: &mut Surface, ui: &Ui, scale: f32) {
        s.fill_rect(0, 0, s.w, s.h, BLACK);
        let count = self.captures.len();
        let Some(show) = self.slideshow.filter(|show| show.index < count) else { return };
        let (cw, ch) = (self.captures[show.index].shown.w, self.captures[show.index].shown.h);
        let (fw, fh) = model::fit_inside(cw, ch, s.w, s.h);
        let key = (show.index, fw, fh);
        if self.slide.as_ref().map(|slide| slide.0) != Some(key) {
            self.slide = Some((key, self.captures[show.index].shown.resized(fw, fh)));
        }
        if let Some((_, img)) = &self.slide {
            s.blit(img, (s.w as i32 - img.w as i32) / 2, (s.h as i32 - img.h as i32) / 2, 255, 255);
        }
        let name = self.captures[show.index].path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let paused = if show.paused() { "  (paused)" } else { "" };
        let caption = format!("{name}   {} of {count}{paused}", show.index + 1);
        let y = s.h as f32 - ui::MAC_FOOTNOTE_PT * 2.4 * scale;
        label_centred(s, ui, &caption, ui::MAC_FOOTNOTE_PT, WHITE, s.w as f32 / 2.0, y, s.w as f32 - 24.0 * scale);
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
        if let Some(at) = self.scene_at {
            let left = ((at - self.now) / 1000.0).ceil().max(1.0) as u32;
            let size = ui::MAC_LARGE_TITLE_PT * 3.0;
            let top = y0 as f32 + vh as f32 / 2.0 - size * scale / 2.0;
            label_centred(s, ui, "Step out of the frame", ui::MAC_BODY_PT, WHITE, centre_x, top - 28.0 * scale, vw as f32);
            label_centred(s, ui, &left.to_string(), size, WHITE, centre_x, top, vw as f32);
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
