//! Magnifier: the camera as a loupe. A live preview with a digital zoom slider, a freeze button
//! that holds the current frame (zoom and filters keep working on it) and an Original / Invert /
//! High Contrast filter control. No flashlight, no photo is ever saved and no sound plays.
//!
//! It reuses the Camera app's V4L2 worker thread; like Camera it touches no device until the app
//! is on screen (`resume`) and releases the camera as soon as it is left (`stop`).

use super::cache::TextCache;
use super::camera::{fit, Worker, POLL_MS, PREVIEW_MAX_W};
use super::switcher::draw_scaled;
use super::ui::{home_zone, Ui, Weight, BLACK, FILL, FILL_PRESSED, SECONDARY, SYSTEM_YELLOW, TINT, TRACK, WHITE};
use super::Event;
use crate::gfx::{pack, Image, Surface};
use crate::layout::Rect;
use crate::media::v4l2;

/// Corner radius in px for every control (rule: at most 6, no pills).
const RADIUS: f32 = 6.0;
pub const ZOOM_MIN: f32 = 1.0;
pub const ZOOM_MAX: f32 = 8.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Filter {
    Original,
    Invert,
    HighContrast,
}

impl Filter {
    const ALL: [Filter; 3] = [Filter::Original, Filter::Invert, Filter::HighContrast];

    pub fn label(self) -> &'static str {
        match self {
            Filter::Original => "Original",
            Filter::Invert => "Invert",
            Filter::HighContrast => "Contrast",
        }
    }

    /// One pixel through the filter.
    pub fn apply(self, p: u32) -> u32 {
        let (r, g, b) = ((p & 0xFF) as i32, ((p >> 8) & 0xFF) as i32, ((p >> 16) & 0xFF) as i32);
        match self {
            Filter::Original => p,
            Filter::Invert => pack(255 - r as u8, 255 - g as u8, 255 - b as u8, 255),
            Filter::HighContrast => {
                // luma, then a steep S curve around mid grey
                let y = (r * 77 + g * 151 + b * 28) >> 8;
                let v = ((y - 128) * 3 + 128).clamp(0, 255) as u8;
                pack(v, v, v, 255)
            }
        }
    }
}

/// The centre crop `(x, y, w, h)` of a `w` x `h` picture at `zoom`.
pub fn zoom_crop(w: usize, h: usize, zoom: f32) -> (usize, usize, usize, usize) {
    let z = zoom.clamp(ZOOM_MIN, ZOOM_MAX);
    let cw = ((w as f32 / z) as usize).clamp(1, w.max(1));
    let ch = ((h as f32 / z) as usize).clamp(1, h.max(1));
    ((w.saturating_sub(cw)) / 2, (h.saturating_sub(ch)) / 2, cw, ch)
}

/// Slider position 0..=1 to a zoom factor.
pub fn zoom_at(frac: f32) -> f32 {
    ZOOM_MIN + frac.clamp(0.0, 1.0) * (ZOOM_MAX - ZOOM_MIN)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Target {
    Slider,
    Freeze,
    Filter(usize),
    Retry,
}

#[derive(Clone, PartialEq, Debug)]
enum State {
    Idle,
    Live,
    Unavailable(String),
}

pub struct Magnifier {
    w: i32,
    h: i32,
    state: State,
    worker: Option<Worker>,
    /// the newest (or frozen) camera frame, as the sensor delivered it
    raw: Option<v4l2::Frame>,
    view: Option<Image>,
    zoom: f32,
    filter: Filter,
    frozen: bool,
    pressed: Option<Target>,
    dragging: bool,
    dirty: bool,
    redo: bool,
    cache: TextCache,
    cache_style: (u32, u32, bool),
}

impl Magnifier {
    pub fn new(ui: &Ui, w: i32, h: i32) -> Self {
        Self {
            w,
            h,
            state: State::Idle,
            worker: None,
            raw: None,
            view: None,
            zoom: ZOOM_MIN,
            filter: Filter::Original,
            frozen: false,
            pressed: None,
            dragging: false,
            dirty: true,
            redo: false,
            cache: TextCache::default(),
            cache_style: (ui.s.to_bits(), ui.text_scale.to_bits(), ui.bold),
        }
    }

    pub fn set_size(&mut self, w: i32, h: i32) {
        self.w = w;
        self.h = h;
        self.dirty = true;
    }

    /// Opens the first camera when the app comes on screen.
    pub fn resume(&mut self) {
        if self.state == State::Idle {
            self.worker = Some(Worker::spawn(v4l2::probe().into_iter().next().map(|c| c.path).unwrap_or_default()));
            self.state = State::Live;
            self.frozen = false;
            self.dirty = true;
        }
    }

    /// The app was left: release the camera.
    pub fn stop(&mut self) {
        if self.state != State::Idle {
            self.worker = None;
            self.state = State::Idle;
            self.raw = None;
            self.view = None;
            self.frozen = false;
            self.dirty = true;
        }
    }

    pub fn streaming(&self) -> bool {
        self.worker.is_some()
    }

    fn bar_top(&self, ui: &Ui) -> f32 {
        self.h as f32 - home_zone(ui) - ui.pt(136.0)
    }

    fn view_rect(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        let top = ui.pt(28.0);
        (0.0, top, self.w as f32, (self.bar_top(ui) - top).max(0.0))
    }

    fn slider_rect(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        let m = ui.pt(56.0);
        (m, self.bar_top(ui) + ui.pt(12.0), self.w as f32 - 2.0 * m, ui.pt(44.0))
    }

    fn freeze_rect(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        let (w, h) = (ui.pt(128.0), ui.pt(44.0));
        (self.w as f32 - ui.pt(16.0) - w, self.bar_top(ui) + ui.pt(72.0), w, h)
    }

    fn filter_rect(&self, ui: &Ui, i: usize) -> (f32, f32, f32, f32) {
        let x0 = ui.pt(16.0);
        let total = self.freeze_rect(ui).0 - ui.pt(12.0) - x0;
        let w = total / 3.0;
        (x0 + i as f32 * w, self.bar_top(ui) + ui.pt(72.0), w, ui.pt(44.0))
    }

    fn retry_rect(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        let (w, h) = (ui.pt(180.0), ui.pt(44.0));
        ((self.w as f32 - w) / 2.0, self.h as f32 / 2.0 + ui.pt(30.0), w, h)
    }

    fn inside((x, y, w, h): (f32, f32, f32, f32), px: f32, py: f32) -> bool {
        px >= x && px < x + w && py >= y && py < y + h
    }

    fn hit(&self, ui: &Ui, x: f32, y: f32) -> Option<Target> {
        if y >= self.h as f32 - home_zone(ui) {
            return None;
        }
        if matches!(self.state, State::Unavailable(_)) {
            return Self::inside(self.retry_rect(ui), x, y).then_some(Target::Retry);
        }
        if self.state != State::Live {
            return None;
        }
        if Self::inside(self.slider_rect(ui), x, y) {
            return Some(Target::Slider);
        }
        if Self::inside(self.freeze_rect(ui), x, y) && self.raw.is_some() {
            return Some(Target::Freeze);
        }
        (0..3).find(|&i| Self::inside(self.filter_rect(ui, i), x, y)).map(Target::Filter)
    }

    fn set_zoom_from(&mut self, ui: &Ui, x: f32) {
        let (sx, _, sw, _) = self.slider_rect(ui);
        let pad = ui.pt(14.0);
        self.zoom = zoom_at((x - sx - pad) / (sw - 2.0 * pad).max(1.0));
        self.redo = true;
        self.dirty = true;
    }

    pub fn touch(&mut self, ui: &Ui, event: Event, _t: f64) {
        match event {
            Event::Down(x, y) => {
                self.pressed = self.hit(ui, x, y);
                self.dragging = self.pressed == Some(Target::Slider);
                if self.dragging {
                    self.set_zoom_from(ui, x);
                }
                self.dirty = true;
            }
            Event::Move(x, y) => {
                if self.dragging {
                    self.set_zoom_from(ui, x);
                } else if self.pressed.is_some() && self.hit(ui, x, y) != self.pressed {
                    self.pressed = None;
                    self.dirty = true;
                }
            }
            Event::Up(x, y) => {
                let pressed = self.pressed.take();
                if self.dragging {
                    self.set_zoom_from(ui, x);
                    self.dragging = false;
                } else if pressed.is_some() && self.hit(ui, x, y) == pressed {
                    match pressed.unwrap() {
                        Target::Freeze => self.frozen = !self.frozen,
                        Target::Filter(i) => {
                            self.filter = Filter::ALL[i.min(2)];
                            self.redo = true;
                        }
                        Target::Retry => {
                            self.state = State::Idle;
                            self.resume();
                        }
                        Target::Slider => {}
                    }
                }
                self.dirty = true;
            }
            _ => {}
        }
    }

    /// Rebuilds the zoomed, filtered picture from the camera frame: the zoom picks its
    /// region of the sensor first and only that is shrunk to the screen, so zooming in
    /// shows real detail instead of an enlarged small picture.
    fn rebuild(&mut self, vw: f32, vh: f32) {
        let Some(frame) = &self.raw else { return };
        let (x, y, cw, ch) = zoom_crop(frame.width, frame.height, self.zoom);
        let (fw, _) = fit(cw, ch, vw as i32, vh as i32);
        let pw = (fw as usize).min(PREVIEW_MAX_W).max(1);
        let ph = (pw * ch / cw.max(1)).max(1);
        let Some(px) = v4l2::to_pixels_region(frame, (x, y, cw, ch), pw, ph) else { return };
        let mut view = Image { w: pw, h: ph, px };
        if self.filter != Filter::Original {
            for p in view.px.iter_mut() {
                *p = self.filter.apply(*p);
            }
        }
        self.view = Some(view);
    }

    pub fn tick(&mut self, ui: &Ui, _t: f64) -> bool {
        let (_, _, vw, vh) = self.view_rect(ui);
        let mut failed = None;
        if let Some(w) = &self.worker {
            let polled = w.poll();
            self.dirty |= polled.opened;
            failed = polled.failed;
            // a frozen picture ignores new frames
            if let (Some(frame), false) = (polled.frame, self.frozen) {
                self.raw = Some(frame);
                self.redo = true;
            }
        }
        if let Some(e) = failed {
            self.worker = None;
            self.state = State::Unavailable(e);
            self.dirty = true;
        }
        if self.redo {
            self.redo = false;
            self.rebuild(vw, vh);
            self.dirty = true;
        }
        self.dirty
    }

    pub fn next_wake(&self, t: f64) -> Option<f64> {
        (self.worker.is_some() && !self.frozen).then_some(t + POLL_MS)
    }

    pub fn describe(&self) -> String {
        let state = match &self.state {
            State::Idle => "idle".to_string(),
            State::Live => "live".to_string(),
            State::Unavailable(m) => format!("unavailable:{m}"),
        };
        format!("magnifier_state={state} magnifier_zoom={:.1} magnifier_filter={} magnifier_frozen={} magnifier_has_frame={}\n", self.zoom, self.filter.label(), self.frozen, self.raw.is_some())
    }

    fn label(&mut self, s: &mut Surface, ui: &Ui, text: &str, pt: f32, color: u32, (x, y, w, h): (f32, f32, f32, f32)) {
        let image = self.cache.get(ui, text, pt, Weight::Regular, color);
        s.blit(image, (x + (w - image.w as f32) / 2.0) as i32, (y + (h - image.h as f32) / 2.0) as i32, 255, 255);
    }

    pub fn draw(&mut self, s: &mut Surface, ui: &Ui, _t: f64) {
        self.dirty = false;
        let style = (ui.s.to_bits(), ui.text_scale.to_bits(), ui.bold);
        if style != self.cache_style {
            self.cache.clear();
            self.cache_style = style;
        }
        s.fill_rect(0, 0, self.w as usize, self.h as usize, BLACK);
        let (vx, vy, vw, vh) = self.view_rect(ui);
        match self.state.clone() {
            State::Live => {
                if let Some(img) = self.view.take() {
                    let (fw, fh) = fit(img.w, img.h, vw as i32, vh as i32);
                    let r = Rect { x: (vx + (vw - fw as f32) / 2.0) as i32, y: (vy + (vh - fh as f32) / 2.0) as i32, w: fw, h: fh };
                    draw_scaled(s, &img, r, 0.0, 255);
                    self.view = Some(img);
                } else {
                    self.label(s, ui, "Starting camera…", 17.0, SECONDARY, (vx, vy, vw, vh));
                }
            }
            State::Unavailable(why) => {
                self.label(s, ui, "Camera not available", 22.0, WHITE, (vx, vy, vw, vh));
                let detail = ui.fit(&why, 13.0, self.w as f32 - ui.pt(48.0));
                self.label(s, ui, &detail, 13.0, SECONDARY, (vx, vy + ui.pt(44.0), vw, vh));
                let r = self.retry_rect(ui);
                s.fill_rrect(r.0, r.1, r.2, r.3, RADIUS, if self.pressed == Some(Target::Retry) { FILL_PRESSED } else { FILL });
                self.label(s, ui, "Try Again", 17.0, WHITE, r);
            }
            State::Idle => {}
        }
        if self.state != State::Live {
            return;
        }
        // zoom slider: capsule track, filled to the thumb, a white knob
        let (sx, sy, sw, sh) = self.slider_rect(ui);
        self.label(s, ui, "1×", 15.0, SECONDARY, (sx - ui.pt(48.0), sy, ui.pt(40.0), sh));
        self.label(s, ui, "8×", 15.0, SECONDARY, (sx + sw + ui.pt(8.0), sy, ui.pt(40.0), sh));
        let pad = ui.pt(14.0);
        let (tx, tw, th) = (sx + pad, sw - 2.0 * pad, ui.pt(4.0));
        let ty = sy + (sh - th) / 2.0;
        let frac = (self.zoom - ZOOM_MIN) / (ZOOM_MAX - ZOOM_MIN);
        s.fill_rrect(tx, ty, tw, th, RADIUS, TRACK);
        s.fill_rrect(tx, ty, tw * frac, th, RADIUS, TINT);
        let k = ui.pt(28.0);
        s.fill_rrect(tx + tw * frac - k / 2.0, sy + (sh - k) / 2.0, k, k, RADIUS, WHITE);
        let zoom_text = format!("{:.1}×", self.zoom);
        self.label(s, ui, &zoom_text, 13.0, WHITE, (sx, sy - ui.pt(10.0), sw, ui.pt(14.0)));
        // filter segments
        let r0 = self.filter_rect(ui, 0);
        let track_w = self.filter_rect(ui, 2).0 + self.filter_rect(ui, 2).2 - r0.0;
        s.fill_rrect(r0.0, r0.1, track_w, r0.3, RADIUS, FILL);
        for i in 0..3 {
            let r = self.filter_rect(ui, i);
            let on = self.filter == Filter::ALL[i];
            if on {
                s.fill_rrect(r.0 + ui.pt(2.0), r.1 + ui.pt(2.0), r.2 - ui.pt(4.0), r.3 - ui.pt(4.0), RADIUS, TINT);
            }
            self.label(s, ui, Filter::ALL[i].label(), 15.0, WHITE, r);
        }
        // freeze button
        let r = self.freeze_rect(ui);
        let fill = match (self.frozen, self.pressed == Some(Target::Freeze)) {
            (true, _) => SYSTEM_YELLOW,
            (false, true) => FILL_PRESSED,
            _ => FILL,
        };
        s.fill_rrect(r.0, r.1, r.2, r.3, RADIUS, fill);
        self.label(s, ui, if self.frozen { "Unfreeze" } else { "Freeze" }, 16.0, if self.frozen { BLACK } else { WHITE }, r);
    }
}

/// The Magnifier icon: a loupe on dark grey, a layered document (`res/icons/magnifier`) drawn by the icon renderer.
pub fn icon(size: usize) -> Image {
    const JSON: &str = include_str!("../../res/icons/magnifier/icon.json");
    bifrost_render::icon_doc::render_builtin(JSON, &[("lens.png", include_bytes!("../../res/icons/magnifier/lens.png"))], size)
        .unwrap_or_else(|| Image { w: size, h: size, px: vec![pack(28, 28, 30, 255); size * size] })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_crop_is_centred_and_shrinks() {
        assert_eq!(zoom_crop(640, 480, 1.0), (0, 0, 640, 480));
        assert_eq!(zoom_crop(640, 480, 2.0), (160, 120, 320, 240));
        assert_eq!(zoom_crop(640, 480, 99.0), (280, 210, 80, 60));
    }

    #[test]
    fn zooming_samples_the_sensor_region_not_a_shrunk_picture() {
        let ui = crate::shell::tests::ui();
        let mut m = Magnifier::new(&ui, 1200, 1920);
        // 4x4 NV21: a white 2x2 square in the middle of black, neutral chroma
        let mut data = vec![16u8, 16, 16, 16, 16, 235, 235, 16, 16, 235, 235, 16, 16, 16, 16, 16];
        data.extend([128u8; 8]);
        m.raw = Some(v4l2::Frame { data, width: 4, height: 4, stride: 4, format: v4l2::NV21 });
        m.zoom = 2.0;
        m.rebuild(8.0, 8.0);
        let view = m.view.as_ref().expect("built");
        assert!(view.px.iter().all(|&p| p == pack(255, 255, 255, 255)), "only the centre is in view");
        m.zoom = 1.0;
        m.rebuild(8.0, 8.0);
        let full = m.view.as_ref().unwrap();
        assert!(full.px.iter().any(|&p| p == pack(0, 0, 0, 255)), "the whole frame includes the black border");
    }

    #[test]
    fn filters_invert_and_threshold() {
        let p = pack(10, 20, 30, 255);
        assert_eq!(Filter::Original.apply(p), p);
        assert_eq!(Filter::Invert.apply(p), pack(245, 235, 225, 255));
        assert_eq!(Filter::HighContrast.apply(pack(250, 250, 250, 255)), pack(255, 255, 255, 255));
        assert_eq!(Filter::HighContrast.apply(pack(5, 5, 5, 255)), pack(0, 0, 0, 255));
    }

    #[test]
    fn new_magnifier_is_idle_and_touches_no_device() {
        let ui = crate::shell::tests::ui();
        let m = Magnifier::new(&ui, 1200, 1920);
        assert!(!m.streaming());
        assert!(m.describe().contains("magnifier_state=idle"));
        assert!(m.next_wake(0.0).is_none());
    }
}
