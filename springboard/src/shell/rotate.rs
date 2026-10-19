//! Landscape: the shell lays out and draws for whichever way the device is
//! held. `set_orientation` (fed by the accelerometer every 200 ms) resizes
//! every size-dependent part — layout, dock backdrop, labels, pager,
//! keyboard, Settings, the Terminal's grid, switcher cards — and plays the
//! bifrost turn: the old frame turns into the new one over 0.3 s with a
//! cross-fade. Control Center's Rotation Lock holds the current orientation.
//! The Clock and Deildu are portrait-only: in landscape they draw into a
//! portrait frame that is letterboxed, scaled to the screen height, between
//! black bars, and their touches are mapped back the same way.

use super::ui::{Weight, BLACK, WHITE};
use super::{AppId, Drawn, Effect, Event, Screen, Shell, apps, IN_DOCK};
use crate::gfx::rotate::{draw_turn, orientation_from_gravity, scale_into, Orientation, Rotation};
use crate::gfx::{Damage, Image, Surface, View};
use crate::layout::Layout;
use crate::scene::{dock_background, Tile};
use super::applib::AppLibrary;
use bifrost_core::device_motion::DeviceMotionManager;
use bifrost_core::inertial_sensors::Sensors;
use bifrost_core::motion_manager::SensorSource;

/// How often the shell reads the gravity direction for the orientation, in seconds (the sensor's 200 ms cadence).
pub const ORIENTATION_INTERVAL: f64 = 0.2;

/// The shell's own device-motion client for the orientation feed. It runs the device motion estimate on the sensors the
/// kernel exposes (`Sensors::scan_default`) and reads the gravity direction every [`ORIENTATION_INTERVAL`]. A device with
/// no accelerometer or no gyroscope has no estimate, so the feed stays idle and the sensed orientation does not move.
pub struct SensorFeed {
    motion: DeviceMotionManager,
    sensors: Sensors,
    next: f64,
}

impl SensorFeed {
    /// The feed on the kernel's sensor tree.
    pub fn scan() -> Self {
        Self::with_sensors(Sensors::scan_default())
    }

    /// The feed on the given sensors. Its device motion is available when there is an accelerometer.
    pub fn with_sensors(sensors: Sensors) -> Self {
        let available = sensors.accelerometer.is_some();
        let mut motion = DeviceMotionManager::new(available);
        motion.set_update_interval(ORIENTATION_INTERVAL);
        motion.start(|_| {});
        SensorFeed { motion, sensors, next: f64::NEG_INFINITY }
    }

    /// One shell tick at `t` seconds: the estimate advances on every tick, and the gravity direction (x, y, in g) comes
    /// back at most once per interval, and only when there is an estimate.
    pub fn sample(&mut self, t: f64) -> Option<(f32, f32)> {
        let SensorFeed { motion, sensors, next } = self;
        feed_step(motion, next, t, sensors)
    }
}

/// The body of [`SensorFeed::sample`], with the sensor source given.
fn feed_step(motion: &mut DeviceMotionManager, next: &mut f64, t: f64, source: &mut dyn SensorSource) -> Option<(f32, f32)> {
    motion.tick(t, source);
    if t < *next {
        return None;
    }
    let gravity = motion.device_motion()?.gravity;
    *next = t + ORIENTATION_INTERVAL;
    Some((gravity.x as f32, gravity.y as f32))
}

/// Duration of the turn animation.
pub const TURN_MS: f64 = 300.0;

/// The apps that lay out natively for the landscape frame (Settings, the Terminal); every
/// other app is laid out for the portrait panel and letterboxed when the frame is landscape.
pub fn landscape_native(app: AppId) -> bool {
    matches!(app, AppId::Settings | AppId::Terminal)
}

/// The apps that are letterboxed in landscape.
pub fn portrait_only(app: AppId) -> bool {
    !landscape_native(app)
}


/// The turn in progress: both frames and where the old one lies.
struct Turn {
    old: Image,
    new: Image,
    /// how the old content is turned in the new frame's coordinates
    from: Rotation,
    start: f64,
    /// distinguishes turns, so the GPU uploads the two frames once per turn
    id: u64,
}

/// One frame of the turn for the GPU: the same pictures `draw_turn` blends on the CPU.
pub struct TurnView<'a> {
    pub id: u64,
    pub old: &'a Image,
    pub new: &'a Image,
    pub from: Rotation,
    /// eased progress, 0 = the old frame, 1 = the new one
    pub k: f32,
}

/// Where a portrait-only app lands in a landscape frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Letterbox {
    pub x: i32,
    pub w: usize,
    pub h: usize,
    /// landscape px per portrait px
    pub scale: f32,
}

impl Letterbox {
    pub fn to_portrait(&self, x: f32, y: f32) -> (f32, f32) {
        ((x - self.x as f32) / self.scale, y / self.scale)
    }
}

/// Orientation state kept by the shell.
pub struct Rotate {
    /// what the accelerometer last reported
    sensed: Orientation,
    /// what the frame is laid out for
    current: Orientation,
    locked: bool,
    /// the orientation the rest of the shell holds: followed from the sensor while Rotation
    /// Lock is off, kept while it is on
    base: Orientation,
    turn: Option<Turn>,
    /// the portrait frame a letterboxed app draws into
    scratch: Option<Image>,
    /// the wallpaper scaled to cover the landscape frame
    wallpaper: Option<Image>,
    /// app labels already set at a label width: (label width, `apps()` index, label)
    labels: Vec<(i32, usize, Image)>,
    /// the App Library of the other frame shape, by (w, h, label width), kept for turning back
    other_library: Option<((i32, i32, i32), AppLibrary)>,
    /// (w, h, label width) the current App Library was built for
    library_key: (i32, i32, i32),
    /// the turn was just set up: its clock starts on the next tick, after the relayout
    turn_fresh: bool,
    turn_seq: u64,
    /// the device-motion client that senses the orientation (idle without an accelerometer)
    feed: SensorFeed,
}

impl Rotate {
    pub fn new() -> Self {
        Rotate { sensed: Orientation::Portrait, current: Orientation::Portrait, locked: false, base: Orientation::Portrait, turn: None, scratch: None, wallpaper: None, labels: Vec::new(), other_library: None, library_key: (0, 0, 0), turn_fresh: false, turn_seq: 0, feed: SensorFeed::scan() }
    }
}

impl Default for Rotate {
    fn default() -> Self {
        Self::new()
    }
}

/// `view` scaled up to cover `w` x `h`, centred.
pub(super) fn cover(view: View, w: usize, h: usize) -> Image {
    let scale = (w as f32 / view.w as f32).max(h as f32 / view.h as f32);
    let (cw, ch) = (((w as f32 / scale).round() as usize).min(view.w), ((h as f32 / scale).round() as usize).min(view.h));
    let src = view.crop((view.w - cw) / 2, (view.h - ch) / 2, cw, ch);
    let mut out = Image::new(w, h);
    scale_into(&mut Surface { w, h, stride: w, px: &mut out.px, y0: 0 }, &src, 0, 0, w, h);
    out
}

/// On a desktop panel the 1200 x 1920 stills do not match the frame: `v` scaled to cover
/// `w` x `h`, or None when `v` already fits or the bifrost profile is active (use `v` itself).
pub(super) fn fit_wallpaper(v: View<'_>, w: i32, h: i32) -> Option<Image> {
    if !crate::layout::adaptive() || (v.w, v.h) == (w as usize, h as usize) {
        return None;
    }
    Some(cover(v, w as usize, h as usize))
}

/// The still to draw: the fitted copy when there is one, else the caller's mapping.
pub(super) fn still<'a>(src: View<'a>, fit: &'a Option<Image>) -> View<'a> {
    fit.as_ref().map_or(src, |i| i.view())
}

impl Shell {
    /// The orientation the frame is laid out for.
    pub fn orientation(&self) -> Orientation {
        self.rot.current
    }

    /// Logical frame size, (w, h).
    pub fn size(&self) -> (i32, i32) {
        (self.w, self.h)
    }

    /// The panel's own (portrait) size.
    pub(super) fn portrait_size(&self) -> (i32, i32) {
        if self.desk_active() { (self.w, self.h) } else { (self.w.min(self.h), self.w.max(self.h)) }
    }

    /// The frame is turned on its side.
    pub(super) fn rot_landscape(&self) -> bool {
        self.rot.current.is_landscape()
    }

    pub fn rotation_locked(&self) -> bool {
        self.rot.locked
    }

    /// True while the turn animation plays; input waits for it.
    pub(super) fn turning(&self) -> bool {
        self.rot.turn.is_some()
    }

    /// Whether the shell needs the accelerometer: unless Rotation Lock holds the orientation
    /// there is something to follow.
    pub fn wants_orientation(&self) -> bool {
        // a desktop panel keeps its own orientation: no sensor, no turning
        !crate::layout::adaptive() && !self.rot.locked
    }

    /// The accelerometer's orientation; adopted unless Rotation Lock holds.
    /// The sensed orientation from the device-motion feed, read on each shell tick. Adopts a new orientation only when
    /// the gravity direction gives one and Rotation Lock does not hold it; a desktop panel never asks.
    pub(super) fn sensor_tick(&mut self, t: f64) {
        if !self.wants_orientation() {
            return;
        }
        let Some((x, y)) = self.rot.feed.sample(t) else { return };
        if let Some(o) = orientation_from_gravity(x, y, self.rot.sensed) {
            if o != self.rot.sensed {
                self.set_orientation(o, t);
            }
        }
    }

    pub fn set_orientation(&mut self, o: Orientation, t: f64) {
        self.rot.sensed = o;
        self.update_orientation(t);
    }

    /// Holds the current orientation (on) or follows the sensor again (off).
    pub fn set_rotation_lock(&mut self, on: bool, t: f64) {
        if on && !self.rot.locked {
            self.rot.base = self.rot.current;
        }
        self.rot.locked = on;
        self.update_orientation(t);
    }

    /// Lays the frame out for what the sensor and Rotation Lock call for;
    /// cheap when nothing changed, so the shell calls it every tick.
    pub(super) fn update_orientation(&mut self, t: f64) {
        if crate::layout::adaptive() {
            return;
        }
        if !self.rot.locked {
            self.rot.base = self.rot.sensed;
        }
        self.apply_orientation(self.rot.base, t);
    }

    /// Flips Rotation Lock from Control Center; the effect persists it.
    pub(super) fn toggle_rotation_lock(&mut self, t: f64) -> Vec<Effect> {
        let on = !self.rot.locked;
        self.set_rotation_lock(on, t);
        vec![Effect::RotationLock(on)]
    }

    fn apply_orientation(&mut self, o: Orientation, t: f64) {
        if o == self.rot.current {
            return;
        }
        if !self.screen_on {
            self.rot.turn = None;
            self.resize(o);
            return;
        }
        // the old frame as the panel shows it now (a turn in progress included)
        let old = self.snapshot_frame(t);
        let from = self.rot.current.rotation().then(o.rotation().inverse());
        self.rot.turn = None;
        self.resize(o);
        let new = self.snapshot_frame(t);
        self.rot.turn_seq += 1;
        self.rot.turn = Some(Turn { old, new, from, start: t, id: self.rot.turn_seq });
        self.rot.turn_fresh = true;
        self.dirty = true;
    }

    /// The whole frame drawn afresh at the current size.
    fn snapshot_frame(&mut self, t: f64) -> Image {
        let mut img = Image::new(self.w as usize, self.h as usize);
        self.drawn = Drawn::Nothing;
        self.draw(&mut Surface { w: img.w, h: img.h, stride: img.w, px: &mut img.px, y0: 0 }, t);
        self.dirty = true;
        img
    }

    /// Completes an open/unlock animation so the screen it leads to is not lost.
    fn finish_anim(&mut self) {
        match self.anim.take() {
            Some((super::Anim::Open { app, .. }, _)) => self.screen = Screen::App(app),
            Some((super::Anim::Unlock { to, .. }, _)) if to >= 1.0 => {
                self.screen = Screen::Home;
                self.wallpaper_owed();
            }
            _ => {}
        }
        self.transition = None;
    }

    /// Lays everything out for orientation `o`.
    fn resize(&mut self, o: Orientation) {
        // Split View is portrait-only: the two panes end before the frame changes shape
        self.end_multi();
        let (pw, ph) = self.portrait_size();
        let (w, h) = o.size(pw, ph);
        self.rot.current = o;
        self.w = w;
        self.h = h;
        self.layout = Layout::zoomed(w, h, self.dock_apps.len() + 1, self.zoom).with_dock_sep(IN_DOCK);
        let ui = &self.ui;
        let lw = self.layout.label_w;
        let labels = &mut self.rot.labels;
        for (tile, i) in self.dock.iter_mut().zip(self.dock_apps.iter().copied()).chain(self.grid.iter_mut().zip(IN_DOCK..apps().len())) {
            // labels are kept per width: turning back and forth sets no text again
            tile.label = match labels.iter().find(|(w, a, _)| *w == lw && *a == i) {
                Some((_, _, img)) => img.clone(),
                None => {
                    let img = ui.styled(&ui.fit(apps()[i].name, 12.0, lw as f32), 12.0, Weight::Regular, WHITE, true);
                    labels.push((lw, i, img.clone()));
                    img
                }
            };
        }
        self.refresh_wallpaper();
        // the library of the shape turned to is reused when it was built before
        let key = (w, h, lw);
        let lib = match self.rot.other_library.take() {
            Some((k, lib)) if k == key => lib,
            _ => AppLibrary::new(&self.ui, &self.layout, &self.base_icons),
        };
        let prev = std::mem::replace(&mut self.library, lib);
        self.rot.other_library = Some((self.rot.library_key, prev));
        self.rot.library_key = key;
        self.home_gen = self.home_gen.wrapping_add(1);
        self.dirty = true;
        self.rebuild_home();
        self.pager.resize(w as f32);
        self.lock_pager.resize(w as f32);
        self.quicksearch.resize(w as f32);
        self.today.resize(w, h);
        self.today_stale = true;
        self.keyboard.resize(w as f32);
        if let Some(login) = &mut self.login {
            login.resize(w, h);
        }
        if let Some(st) = &mut self.settings {
            st.resize(&self.ui, w, h);
        }
        self.resize_terminals();
        self.damage = Damage::new(w as usize, h as usize);
        self.rows.clear();
        self.drawn = Drawn::Nothing;
        self.dirty = true;
        self.finish_anim();
        self.cc.hide();
        self.home_gesture = None;
        self.lock_drag = None;
        self.kb_touch = false;
        self.dark_tap = None;
        self.switcher_resized();
        // the cards show the apps at the new size, most recent kept first
        // drawing an app sets `fullscreen` for it; the foreground app's value must survive
        let fullscreen = self.fullscreen;
        for app in self.switcher.apps().into_iter().rev() {
            let frame = self.render_app_frame(app);
            self.store_card(app, &frame);
        }
        self.fullscreen = fullscreen;
    }

    /// The wallpaper for the current frame: the still itself in portrait,
    /// scaled to cover the frame in landscape.
    pub(super) fn wallpaper(&self) -> View<'_> {
        match &self.rot.wallpaper {
            Some(img) if self.rot.current.is_landscape() => img.view(),
            _ => self.home_wp(),
        }
    }

    /// Rebuilds what derives from the wallpaper for the current frame: the
    /// landscape cover and the dock backdrop.
    pub(super) fn refresh_wallpaper(&mut self) {
        if self.rot.current.is_landscape() {
            let (w, h) = (self.w as usize, self.h as usize);
            let home = self.home_wp();
            if (home.w, home.h) == (w, h) {
                // already the frame's size (every desktop panel): no 1:1 copy of the still
                self.rot.wallpaper = None;
            } else if self.rot.wallpaper.as_ref().is_none_or(|img| (img.w, img.h) != (w, h)) {
                self.rot.wallpaper = Some(cover(home, w, h));
            }
        }
        self.dock_bg = dock_background(self.wallpaper(), &self.layout);
        self.dirty = true;
    }

    /// Forgets the landscape wallpaper after the still changed.
    pub(super) fn wallpaper_changed(&mut self) {
        self.rot.wallpaper = None;
        self.refresh_wallpaper();
    }

    /// Draws the turn frame at `t` if one is playing; true when it did.
    pub(super) fn draw_turn(&mut self, s: &mut Surface, t: f64) -> bool {
        let Some(turn) = &self.rot.turn else { return false };
        // bifrost turns ease out: fast off the mark, settling gently
        let k = super::gestures::ease_out_quartic(((t - turn.start) / TURN_MS).clamp(0.0, 1.0) as f32);
        draw_turn(s, &turn.old, &turn.new, turn.from, k);
        true
    }

    /// The turn frame at `t` for the GPU to draw, if a turn plays.
    pub fn turn_view(&self, t: f64) -> Option<TurnView<'_>> {
        let turn = self.rot.turn.as_ref()?;
        let k = super::gestures::ease_out_quartic(((t - turn.start) / TURN_MS).clamp(0.0, 1.0) as f32);
        Some(TurnView { id: turn.id, old: &turn.old, new: &turn.new, from: turn.from, k })
    }

    /// The GPU drew the turn frame `turn_view` described: the same bookkeeping as `draw` after `draw_turn`.
    pub fn turn_drawn(&mut self) {
        self.dirty = false;
        self.drawn = Drawn::App;
    }

    /// Starts the clock of a turn just set up at `now`: laying the frame out again takes a
    /// while, and a turn timed from before that would be over by its first frame.
    pub fn start_turn_clock(&mut self, now: f64) {
        if std::mem::take(&mut self.rot.turn_fresh) {
            if let Some(turn) = &mut self.rot.turn {
                turn.start = turn.start.max(now);
            }
        }
    }

    /// Forgets the App Library kept for the other frame shape (the apps or icons changed).
    pub(super) fn forget_other_library(&mut self) {
        self.rot.other_library = None;
    }

    /// Ends the turn once its time is up.
    pub(super) fn tick_turn(&mut self, t: f64) {
        if self.rot.turn.as_ref().is_some_and(|turn| t - turn.start >= TURN_MS) {
            self.end_turn();
        }
    }

    /// Drops a turn in progress (screen off); the next frame repaints in full.
    pub(super) fn end_turn(&mut self) {
        if self.rot.turn.take().is_some() {
            self.drawn = Drawn::Nothing;
            self.dirty = true;
        }
    }

    /// Where a portrait-only app sits in the frame; None when the frame is
    /// portrait and the app fills it.
    pub(super) fn letterbox(&self) -> Option<Letterbox> {
        if !self.rot.current.is_landscape() {
            return None;
        }
        let (pw, ph) = self.portrait_size();
        let scale = self.h as f32 / ph as f32;
        let w = (pw as f32 * scale).round() as usize;
        Some(Letterbox { x: (self.w - w as i32) / 2, w, h: self.h as usize, scale })
    }

    /// A touch in frame coordinates as a portrait-only app sees it.
    pub(super) fn letterbox_event(&self, ev: Event) -> Event {
        let Some(lb) = self.letterbox() else { return ev };
        match ev {
            Event::Down(x, y) => {
                let (x, y) = lb.to_portrait(x, y);
                Event::Down(x, y)
            }
            Event::Move(x, y) => {
                let (x, y) = lb.to_portrait(x, y);
                Event::Move(x, y)
            }
            Event::Up(x, y) => {
                let (x, y) = lb.to_portrait(x, y);
                Event::Up(x, y)
            }
            other => other,
        }
    }

    /// Draws a portrait frame with `paint`, letterboxed into the landscape
    /// frame; in portrait `paint` draws straight into `s`.
    pub(super) fn draw_letterboxed(&mut self, s: &mut Surface, paint: impl FnOnce(&mut Self, &mut Surface)) {
        let Some(lb) = self.letterbox() else { return paint(self, s) };
        let (pw, ph) = self.portrait_size();
        let mut img = self.rot.scratch.take().filter(|i| (i.w, i.h) == (pw as usize, ph as usize)).unwrap_or_else(|| Image::new(pw as usize, ph as usize));
        paint(self, &mut Surface { w: img.w, h: img.h, stride: img.w, px: &mut img.px, y0: 0 });
        let right = lb.x as usize + lb.w;
        s.fill_rect(0, 0, lb.x as usize, lb.h, BLACK);
        s.fill_rect(right, 0, (self.w as usize).saturating_sub(right), lb.h, BLACK);
        scale_into(s, &img, lb.x, 0, lb.w, lb.h);
        self.rot.scratch = Some(img);
    }

    /// Lines for `describe`.
    pub(super) fn describe_orientation(&self) -> String {
        format!("orientation={}\nrotation_lock={}\nturning={}\n", self.rot.current.name(), self.rot.locked, self.turning())
    }

    /// The dock and grid tiles, for the landscape preview.
    pub fn tiles(&self) -> (&[Tile], &[Tile]) {
        (&self.dock, &self.grid)
    }

    /// The command the Terminal runs; previews on the host use `/bin/sh`.
    pub fn set_terminal_command(&mut self, cmd: &'static [&'static str]) {
        self.term_cmd = cmd;
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{settle, shell, swipe, unlock};
    use super::super::{control_center, Screen};
    use super::*;
    use crate::gfx::pack;

    fn frame(sh: &mut Shell, t: f64) -> Vec<u32> {
        let (w, h) = sh.size();
        let mut buf = vec![0u32; (w * h) as usize];
        sh.draw(&mut Surface { w: w as usize, h: h as usize, stride: w as usize, px: &mut buf, y0: 0 }, t);
        buf
    }

    fn landscape(sh: &mut Shell, t: f64) {
        sh.set_orientation(Orientation::LandscapeLeft, t);
        sh.tick(t + TURN_MS);
        assert!(!sh.turning());
    }

    #[test]
    fn turning_relays_out_the_home_screen_and_plays_the_turn() {
        let mut sh = shell();
        unlock(&mut sh);
        frame(&mut sh, 1000.0);
        assert_eq!(sh.orientation(), Orientation::Portrait);
        sh.set_orientation(Orientation::LandscapeLeft, 2000.0);
        assert_eq!(sh.orientation(), Orientation::LandscapeLeft);
        assert_eq!(sh.size(), (1920, 1200));
        assert_eq!(sh.layout.cols, crate::layout::LAND_COLS);
        assert!(sh.turning() && sh.needs_frame());
        assert!(sh.describe().contains("orientation=landscape-left"));
        // touches wait for the turn
        assert!(sh.event(Event::Down(100.0, 100.0), 2010.0).is_empty());
        let first = frame(&mut sh, 2000.0);
        assert!(sh.damage.is_all());
        let mid = frame(&mut sh, 2150.0);
        assert_eq!(mid[0], pack(0, 0, 0, 255), "a corner is black half way through the turn");
        assert_ne!(first, mid);
        sh.tick(2300.0);
        assert!(!sh.turning());
        let done = frame(&mut sh, 2300.0);
        assert!(sh.damage.is_all(), "the first landscape frame repaints in full");
        assert_eq!(done[0], sh.wallpaper().px[0], "the wallpaper covers the landscape frame");
        assert_eq!(sh.wallpaper().w, 1920);
        assert!(frame(&mut sh, 2316.0).is_empty() || sh.damage.is_empty(), "then nothing changes");
        let l = &sh.layout;
        assert!(l.dock_icons.iter().all(|r| r.x > 0 && r.x + r.w < 1920 && r.y + r.h < 1200));
        assert_eq!(sh.keyboard.w, 1920.0);
        assert_eq!(sh.pager.width, 1920.0);
        assert_eq!(sh.dock_bg.as_ref().map(|b| b.w as i32), Some(sh.layout.dock.w));
        // and back
        sh.set_orientation(Orientation::Portrait, 5000.0);
        assert_eq!(sh.size(), (1200, 1920));
        sh.tick(5300.0);
        assert_eq!(frame(&mut sh, 5300.0)[0], sh.wallpaper().px[0]);
        assert_eq!(sh.wallpaper().w, 1200);
    }

    #[test]
    fn a_tap_on_the_landscape_dock_opens_the_app() {
        let mut sh = shell();
        unlock(&mut sh);
        landscape(&mut sh, 1000.0);
        let r = sh.layout.dock_icons[0];
        let (x, y) = (r.x as f32 + 10.0, r.y as f32 + 10.0);
        sh.event(Event::Down(x, y), 5000.0);
        sh.event(Event::Up(x, y), 5050.0);
        settle(&mut sh, 5050.0);
        assert_eq!(sh.screen, Screen::App(AppId::Settings));
        let buf = frame(&mut sh, 6000.0);
        assert_eq!(buf.len(), 1920 * 1200);
        // the go-home swipe starts from the landscape frame's bottom edge
        swipe(&mut sh, 960.0, 1195.0, 700.0, 7000.0);
        settle(&mut sh, 7200.0);
        assert_eq!(sh.screen, Screen::Home);
        assert_eq!(sh.switcher.apps(), vec![AppId::Settings]);
        assert_eq!(sh.snapshot_app(AppId::Settings).w, (1920.0f32 * 0.42).round() as usize);
    }

    #[test]
    fn rotation_lock_holds_the_orientation_until_released() {
        let mut sh = shell();
        unlock(&mut sh);
        landscape(&mut sh, 1000.0);
        assert_eq!(sh.set_rotation_lock(true, 2000.0), ());
        assert!(sh.rotation_locked());
        sh.set_orientation(Orientation::Portrait, 3000.0);
        assert_eq!(sh.orientation(), Orientation::LandscapeLeft, "locked");
        assert!(!sh.turning());
        sh.set_rotation_lock(false, 4000.0);
        assert_eq!(sh.orientation(), Orientation::Portrait, "follows the last reading once released");
        assert!(sh.turning());
        sh.tick(4300.0);
        // the Control Center tile toggles it and asks for persistence
        swipe(&mut sh, 1100.0, 5.0, 500.0, 5000.0);
        settle(&mut sh, 5110.0);
        assert!(sh.cc.is_shown());
        let (x, y, w, h) = control_center::tile_rect(&sh.ui, sh.w, control_center::Tile::RotationLock);
        let (cx, cy) = (x + w / 2.0, y + h / 2.0);
        let mut fx = sh.event(Event::Down(cx, cy), 6000.0);
        fx.extend(sh.event(Event::Up(cx, cy), 6030.0));
        assert_eq!(fx, vec![Effect::RotationLock(true)]);
        assert!(sh.rotation_locked() && sh.describe().contains("rotation_lock=true"));
        frame(&mut sh, 6100.0);
        let mut fx = sh.event(Event::Down(cx, cy), 7000.0);
        fx.extend(sh.event(Event::Up(cx, cy), 7030.0));
        assert_eq!(fx, vec![Effect::RotationLock(false)]);
    }

    #[test]
    fn portrait_only_apps_are_letterboxed_and_their_touches_mapped() {
        let mut sh = shell();
        unlock(&mut sh);
        assert!(sh.letterbox().is_none());
        landscape(&mut sh, 1000.0);
        let lb = sh.letterbox().unwrap();
        assert_eq!(lb, Letterbox { x: 585, w: 750, h: 1200, scale: 0.625 });
        assert_eq!(lb.to_portrait(585.0 + 375.0, 600.0), (600.0, 960.0));
        assert!(portrait_only(AppId::Files) && portrait_only(AppId::Deildu) && !portrait_only(AppId::Settings));
        sh.open(AppId::Files, 2000.0);
        settle(&mut sh, 2000.0);
        assert_eq!(sh.screen, Screen::App(AppId::Files));
        let buf = frame(&mut sh, 3000.0);
        let at = |x: usize, y: usize| buf[y * 1920 + x];
        assert_eq!(at(300, 600), pack(0, 0, 0, 255), "left bar");
        assert_eq!(at(1700, 600), pack(0, 0, 0, 255), "right bar");
        // the app draws inside the column
        assert!((585..1335).any(|x| (0..1200).any(|y| at(x, y) != pack(0, 0, 0, 255))), "the app draws inside the letterbox");
        // the switcher card and the launch snapshot are landscape frames
        let card = sh.snapshot_app(AppId::Files);
        assert_eq!((card.w, card.h), ((1920.0f32 * 0.42).round() as usize, (1200.0f32 * 0.42).round() as usize));
        // back in portrait the app fills the frame again
        sh.set_orientation(Orientation::Portrait, 5000.0);
        sh.tick(5300.0);
        let buf = frame(&mut sh, 5300.0);
        assert_eq!(buf.len(), 1200 * 1920);
        assert!(sh.letterbox().is_none());
    }

    #[test]
    fn the_terminal_reflows_to_the_landscape_width() {
        let mut sh = shell();
        unlock(&mut sh);
        sh.term_cmd = &["/bin/sh"];
        sh.open(AppId::Terminal, 1000.0);
        settle(&mut sh, 1000.0);
        landscape(&mut sh, 2000.0);
        assert_eq!(sh.term_area().2, 1920);
        assert!(sh.keyboard.shown());
        let kb_top = sh.kb_top();
        assert!(kb_top > 0.0 && kb_top < 1200.0 - sh.keyboard.h() + 1.0);
        frame(&mut sh, 3000.0);
        assert_eq!(sh.screen, Screen::App(AppId::Terminal));
    }

    #[test]
    fn turning_with_the_screen_off_or_mid_animation_is_safe() {
        let mut sh = shell();
        unlock(&mut sh);
        sh.event(Event::Power, 1000.0);
        sh.set_orientation(Orientation::LandscapeRight, 2000.0);
        assert_eq!(sh.size(), (1920, 1200));
        assert!(!sh.turning(), "no animation while dark");
        sh.event(Event::Power, 3000.0);
        assert!(sh.is_locked());
        let buf = frame(&mut sh, 3100.0);
        assert_eq!(buf.len(), 1920 * 1200);
        // a lock-screen swipe up in landscape unlocks
        swipe(&mut sh, 960.0, 1100.0, 300.0, 4000.0);
        settle(&mut sh, 4200.0);
        assert_eq!(sh.screen, Screen::Home);
        // turning while an app launch animates lands in the app
        sh.open(AppId::Settings, 5000.0);
        assert!(sh.anim.is_some());
        sh.set_orientation(Orientation::Portrait, 5010.0);
        assert_eq!(sh.screen, Screen::App(AppId::Settings));
        assert!(sh.anim.is_none() && sh.turning());
        // a second turn mid-turn starts from the frame on screen
        frame(&mut sh, 5100.0);
        sh.set_orientation(Orientation::LandscapeLeft, 5150.0);
        assert!(sh.turning() && sh.size() == (1920, 1200));
        // the screen going off ends the turn
        sh.event(Event::Power, 5200.0);
        assert!(!sh.turning());
        sh.set_orientation(Orientation::LandscapeLeft, 5300.0);
        assert!(!sh.turning(), "same orientation again changes nothing");
    }

    #[test]
    fn a_new_wallpaper_is_covered_for_landscape_too() {
        let mut sh = shell();
        landscape(&mut sh, 1000.0);
        let px: &'static [u32] = Box::leak(vec![pack(200, 40, 40, 255); 1200 * 1920].into_boxed_slice());
        sh.set_wallpaper(View { w: 1200, h: 1920, px });
        assert_eq!(sh.wallpaper().px[1920 * 600 + 960], pack(200, 40, 40, 255));
        assert_eq!((sh.wallpaper().w, sh.wallpaper().h), (1920, 1200));
        let bg = sh.dock_bg.as_ref().unwrap();
        assert!(bg.px[bg.w * bg.h / 2 + bg.w / 2] & 0xff > 40);
        let small = Image { w: 10, h: 30, px: (0..300).map(|i| pack(i as u8, 0, 0, 255)).collect() };
        let c = cover(small.view(), 60, 20);
        assert_eq!((c.w, c.h), (60, 20));
        assert!(c.px.iter().all(|&p| p >> 24 == 255));
    }
}
#[cfg(test)]
mod tmp_bench {
    use super::*;
    #[test]
    fn bench_turn_start() {
        let mut sh = super::super::tests::shell();
        let _ = sh.snapshot_frame(1000.0);
        let t = std::time::Instant::now();
        let _ = sh.snapshot_frame(1000.0);
        eprintln!("snap {:?}", t.elapsed());
        let t = std::time::Instant::now();
        sh.resize(Orientation::LandscapeLeft);
        eprintln!("resize {:?}", t.elapsed());
        let t = std::time::Instant::now();
        let _ = sh.snapshot_frame(1000.0);
        eprintln!("snap2 {:?}", t.elapsed());
    }
}

#[cfg(test)]
mod sensor_feed_tests {
    use super::*;
    use bifrost_core::motion::Vec3;

    /// A device lying tilted: the accelerometer reads `accel` and the gyroscope is still.
    struct Still {
        accel: Vec3,
    }

    impl SensorSource for Still {
        fn accelerometer(&mut self) -> Option<Vec3> {
            Some(self.accel)
        }
        fn gyroscope(&mut self) -> Option<Vec3> {
            Some(Vec3 { x: 0.0, y: 0.0, z: 0.0 })
        }
    }

    #[test]
    fn a_feed_without_an_accelerometer_never_samples() {
        let mut feed = SensorFeed::with_sensors(Sensors::default());
        for i in 0..20 {
            assert_eq!(feed.sample(i as f64 * 0.05), None);
        }
    }

    #[test]
    fn with_a_tilted_device_the_gravity_direction_comes_once_per_interval() {
        let mut motion = DeviceMotionManager::new(true);
        motion.set_update_interval(ORIENTATION_INTERVAL);
        motion.start(|_| {});
        let mut next = f64::NEG_INFINITY;
        let mut src = Still { accel: Vec3 { x: 0.9, y: 0.0, z: -0.4 } };
        let mut samples = Vec::new();
        for i in 0..60 {
            if let Some(g) = feed_step(&mut motion, &mut next, i as f64 * 0.01, &mut src) {
                samples.push(g);
            }
        }
        assert!(samples.len() >= 2 && samples.len() <= 4, "one sample per 200 ms, got {}", samples.len());
        let (x, _) = *samples.last().unwrap();
        // Gravity points down, so it is the opposite of the accelerometer reading (a flat device reads (0, 0, -1)).
        assert!((x + 0.9).abs() < 0.2, "gravity x is the opposite of the reading: {x}");
    }
}

