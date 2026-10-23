//! The shell's side of the Clock: the headless alarm and timer service and
//! the full-screen "time's up" alert. The Clock app itself (World Clock,
//! alarm cards and editor, Stopwatch, Timer tab) is the
//! `is.olibuijr.clock` system-app package; it persists alarms and timers
//! beside its data (`clock.ini`, `clock-timers.ini`) and this service
//! reloads them when they change, so alarms and timers keep ringing with
//! the app closed or the shell restarted. The app never makes a sound
//! itself: a firing alert carries the chosen sound and the shell plays it
//! on `Effect::Alarm`.

pub mod alarm;
pub mod picker;
pub mod store;
pub mod timer;
pub mod tz;

use super::cache::TextCache;
use super::glyphs;
use super::ui::{Ui, Weight, BLACK, SECONDARY, WHITE};
use super::{Effect, Event};
use crate::gfx::Surface;
use alarm::Alarms;
use timer::Timers;
use tz::{civil, City};

pub const ORANGE: u32 = crate::shell::ui::SYSTEM_ORANGE;
const GREY_ACTIVE: u32 = crate::shell::ui::CALC_NUMERIC;

/// The package id of the Clock app's data (the folder under appdata, and the legacy migration key).
pub const APP_ID: &str = "is.olibuijr.clock";

/// The manifest id of the installed Clock app (`apps/clock-desktop/manifest.toml`). Notifications for fired alarms
/// are filed under the installed app with this id, which is found whether or not the app is running.
pub const MANIFEST_ID: &str = "is.olibuijr.clock.desktop";

/// The files the Clock app and this service share.
pub const FILES: [&str; 4] = ["clock.ini", "clock-timers.ini", "clock-stopwatch.ini", "world-cities.ini"];

/// The folder the service reads: the installed Clock app's own data folder (`appdata/<manifest id>`, the app's
/// `ctx.data_dir`). The service read `appdata/is.olibuijr.clock`, a folder the installed app never writes, so its timers
/// and alarms never rang and no notification was posted (W5-2).
pub fn service_dir(root: &std::path::Path) -> std::path::PathBuf {
    root.join("appdata").join(MANIFEST_ID)
}

/// Moves the files of the old package folder (`appdata/is.olibuijr.clock`) into `new` where `new` has none of its own,
/// once, so alarms set before the move are kept. Best effort.
pub fn adopt_old_dir(old: &std::path::Path, new: &std::path::Path) {
    for name in FILES {
        let (from, to) = (old.join(name), new.join(name));
        if from.exists() && !to.exists() {
            let _ = std::fs::rename(&from, &to);
        }
    }
}

/// How often the service looks for changes the app made to its files while the Clock app runs (it is their only
/// writer), and while it does not: a once-a-second poll with no app open was the shell's main idle wake-up (round 75).
const POLL_MS: f64 = 1000.0;
const IDLE_POLL_MS: f64 = 60_000.0;

/// Wall-clock time anchored to the monotonic frame clock.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Wall {
    pub unix: i64,
    /// local offset from UTC in seconds
    pub offset: i32,
    /// monotonic ms at which `unix` was read
    pub at: f64,
}

impl Wall {
    /// The system clock and local zone read now, anchored at monotonic `at`.
    /// Under the test pin (`BIFROST_TEST_PIN=1`) it is the pinned instant in UTC, so snapshots do not follow the date.
    pub fn sample(at: f64) -> Self {
        if bifrost_core::pin::pinned() {
            return Self { unix: bifrost_core::pin::PINNED_UNIX_SECS, offset: 0, at };
        }
        let (unix, offset) = crate::bare::sys::wall_time();
        Self { unix, offset, at }
    }

    pub fn now(&self, t: f64) -> i64 {
        self.unix + ((t - self.at) / 1000.0).floor() as i64
    }

    /// Unix milliseconds at monotonic `t`.
    pub fn now_ms(&self, t: f64) -> i64 {
        self.unix * 1000 + (t - self.at).floor() as i64
    }

    /// Monotonic ms at which Unix time `unix` arrives.
    pub fn mono(&self, unix: i64) -> f64 {
        self.at + (unix - self.unix) as f64 * 1000.0
    }
}

/// The UTC offset in seconds at any instant: the home zone's rule when one
/// is set, else the device's current offset.
pub fn offset_fn(zone: Option<&'static City>, offset: i32) -> impl Fn(i64) -> i32 {
    move |unix| zone.map_or(offset, |c| c.offset_at(unix))
}

/// The full-screen "time's up" screen.
#[derive(Clone, Debug, PartialEq)]
pub struct Alert {
    pub title: String,
    pub time: String,
    /// a timer rather than an alarm
    pub is_timer: bool,
    /// the ringing alarm, for snooze and dismiss
    pub alarm_id: Option<u32>,
    /// the tone to play, with vibration and a rising volume: for the shell's
    /// sound path on `Effect::Alarm`
    pub sound: String,
    pub vibrate: bool,
    pub ramp: bool,
    /// the alarm may be snoozed
    pub snooze: bool,
    /// the device is muted: the alert is silent and only vibrates
    pub silent: bool,
    /// the repeat set ("Weekdays"), shown under the title; empty for a one-shot
    pub repeat: String,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Target {
    Stop,
    Snooze,
}

pub struct Clock {
    w: i32,
    h: i32,
    wall: Wall,
    timers: Timers,
    alarms: Alarms,
    /// the ringer is muted (volume at zero): alerts only vibrate
    muted: bool,
    path: Option<String>,
    /// the files' modification times as last read or written
    seen: store::Stamp,
    next_poll: f64,
    /// the Clock app is running, so its files may change at any moment (set by the shell before each tick)
    pub app_running: bool,
    /// the home zone's daylight-saving rule, when known
    zone: Option<&'static City>,
    alert: Option<Alert>,
    /// alerts waiting behind the one on screen
    queue: Vec<Alert>,
    /// fired alerts the shell has not turned into notifications yet
    fired: Vec<Alert>,
    pressed: Option<Target>,
    cache: TextCache,
}

impl Clock {
    /// Lays the alert out for a screen of `w` x `h` px.
    pub fn set_size(&mut self, w: i32, h: i32) {
        self.w = w;
        self.h = h;
    }

    pub fn new(_ui: &Ui, w: i32, h: i32) -> Self {
        Clock {
            w,
            h,
            wall: Wall::default(),
            timers: Timers::default(),
            alarms: Alarms::default(),
            muted: false,
            path: None,
            seen: store::Stamp::default(),
            next_poll: 0.0,
            app_running: false,
            zone: None,
            alert: None,
            queue: vec![],
            fired: vec![],
            pressed: None,
            cache: TextCache::default(),
        }
    }

    fn read_files(&mut self, t: f64) {
        let Some(path) = self.path.clone() else { return };
        self.alarms = Alarms::parse(&std::fs::read_to_string(&path).unwrap_or_default());
        self.alarms.start_at(self.wall.now(t));
        let timers = std::fs::read_to_string(store::sibling(&path, "clock-timers.ini")).unwrap_or_default();
        self.timers = Timers::parse(&timers, t, self.wall.now_ms(t));
        self.seen = store::stamp(&path);
    }

    /// Loads alarms from `path` and the timers beside it, and watches both
    /// from now on. Returns the RTC wake effect for the next one.
    pub fn load(&mut self, path: &str, t: f64) -> Vec<Effect> {
        self.path = Some(path.to_string());
        self.read_files(t);
        vec![self.wake_effect(t)]
    }

    /// Tells the Clock which zone's daylight-saving rule the device follows,
    /// so alarm times hold across a change of offset. Without it the
    /// device's current offset is assumed to stay.
    pub fn set_zone(&mut self, zone: Option<&'static City>) {
        self.zone = zone;
    }

    /// Anchors the wall clock: `unix` seconds and the local `offset` as of
    /// monotonic `t`.
    pub fn set_wall(&mut self, unix: i64, offset: i32, t: f64) {
        self.wall = Wall { unix, offset, at: t };
    }

    /// The current wall-clock anchor, shared with apps that show dates.
    pub fn wall(&self) -> Wall {
        self.wall
    }

    pub fn alert(&self) -> Option<&Alert> {
        self.alert.as_ref()
    }

    pub fn timers(&self) -> &Timers {
        &self.timers
    }

    pub fn alarms(&self) -> &Alarms {
        &self.alarms
    }

    /// The alerts that fired since the last call, for the notification path.
    pub fn drain_fired(&mut self) -> Vec<Alert> {
        std::mem::take(&mut self.fired)
    }

    /// "Alarm in 7 hr 5 min" for the lock screen and Control Center, when an
    /// alarm is armed.
    pub fn next_alarm_text(&self, t: f64) -> Option<String> {
        self.alarms.summary(self.wall.now(t), &offset_fn(self.zone, self.wall.offset))
    }

    /// Follows the ringer: a muted device rings its alarms silently.
    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
    }

    /// Arms the RTC for the earlier of the next alarm and the next timer.
    fn wake_effect(&self, t: f64) -> Effect {
        let alarm = self.alarms.next_due(self.wall.now(t), &offset_fn(self.zone, self.wall.offset));
        let at = match (alarm, self.timers.next_due_unix()) {
            (Some(a), Some(b)) => a.min(b),
            (a, b) => a.or(b).unwrap_or(0),
        };
        Effect::WakeAt(at)
    }

    fn write(&mut self, name: Option<&str>, text: String) {
        let Some(p) = &self.path else { return };
        let path = name.map_or_else(|| p.clone(), |n| store::sibling(p, n));
        if let Err(e) = store::write_atomic(&path, &text) {
            eprintln!("springboard: clock: {e}");
        }
        self.seen = store::stamp(p);
    }

    fn persist(&mut self, t: f64) -> Vec<Effect> {
        self.write(None, self.alarms.serialize());
        vec![self.wake_effect(t)]
    }

    /// Starts a Clock timer for another caller (the `timer_start` system intent, which Shortcuts and Search run): the
    /// files are read first so a change the Clock app just made is kept, then the timer is written beside the alarms,
    /// so the Clock app lists it and this service rings it with the app closed. None for a zero duration.
    pub fn start_timer(&mut self, label: &str, duration_ms: i64, t: f64) -> Option<u32> {
        self.read_files(t);
        let id = self.timers.start(label, duration_ms, t, self.wall.now_ms(t))?;
        let _ = self.persist_timers(t);
        Some(id)
    }

    fn persist_timers(&mut self, t: f64) -> Vec<Effect> {
        self.write(Some("clock-timers.ini"), self.timers.serialize());
        vec![self.wake_effect(t)]
    }

    /// Shows `alert` now, or queues it behind the one on screen, and
    /// remembers it for the notification path.
    fn push_alert(&mut self, alert: Alert) {
        self.fired.push(alert.clone());
        if self.alert.is_none() {
            self.alert = Some(alert);
        } else {
            self.queue.push(alert);
        }
    }

    /// Closes the alert on screen (snoozing its alarm when asked) and shows
    /// the next queued one.
    fn close_alert(&mut self, snooze: bool, t: f64) -> Vec<Effect> {
        let Some(done) = self.alert.take() else { return vec![] };
        self.alert = if self.queue.is_empty() { None } else { Some(self.queue.remove(0)) };
        let Some(id) = done.alarm_id else { return vec![] };
        if snooze {
            self.alarms.snooze(id, self.wall.now(t));
        } else {
            self.alarms.dismiss(id);
        }
        self.persist(t)
    }

    // ----- geometry -----

    fn snooze_button(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        let (x, y, w, h) = self.stop_button(ui);
        (x, y - h - ui.pt(16.0), w, h)
    }

    fn stop_button(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        let (bw, bh) = (ui.pt(300.0), ui.pt(56.0));
        ((self.w as f32 - bw) / 2.0, self.h as f32 - ui.pt(200.0), bw, bh)
    }

    fn within(x: f32, y: f32, (rx, ry, rw, rh): (f32, f32, f32, f32)) -> bool {
        x >= rx && x < rx + rw && y >= ry && y < ry + rh
    }

    fn hit(&self, ui: &Ui, x: f32, y: f32) -> Option<Target> {
        let alert = self.alert.as_ref()?;
        if alert.snooze && Self::within(x, y, self.snooze_button(ui)) {
            return Some(Target::Snooze);
        }
        Self::within(x, y, self.stop_button(ui)).then_some(Target::Stop)
    }

    // ----- input -----

    /// Touches for the alert on screen: Stop and Snooze.
    pub fn touch(&mut self, ui: &Ui, ev: Event, t: f64) -> Vec<Effect> {
        match ev {
            Event::Down(x, y) => {
                self.pressed = self.hit(ui, x, y);
                vec![]
            }
            Event::Up(x, y) => match self.pressed.take() {
                Some(p) if Some(p) == self.hit(ui, x, y) => match p {
                    Target::Stop => self.close_alert(false, t),
                    Target::Snooze => self.close_alert(true, t),
                },
                _ => vec![],
            },
            _ => vec![],
        }
    }

    // ----- time -----

    /// Look at the app's files on the next tick (the app just started).
    pub fn next_poll_now(&mut self, t: f64) {
        self.next_poll = self.next_poll.min(t);
    }

    /// Fires due timers and alarms and picks up the app's edits. Returns
    /// effects (Alarm, WakeAt) and whether the alert's screen changed.
    pub fn tick(&mut self, _ui: &Ui, t: f64) -> (Vec<Effect>, bool) {
        let mut effects = vec![];
        let mut dirty = false;
        if t >= self.next_poll {
            self.next_poll = t + if self.app_running { POLL_MS } else { IDLE_POLL_MS };
            if let Some(p) = &self.path {
                if store::stamp(p) != self.seen {
                    self.read_files(t);
                    effects.push(self.wake_effect(t));
                }
            }
        }
        let wall_now = self.wall.now(t);
        let hhmm = {
            let now = civil(wall_now, self.wall.offset);
            format!("{:02}:{:02}", now.hour, now.min)
        };
        let finished = self.timers.poll(t);
        for item in &finished {
            self.push_alert(Alert { title: item.label.clone(), time: hhmm.clone(), is_timer: true, alarm_id: None, sound: alarm::SOUNDS[0].into(), vibrate: true, ramp: false, snooze: false, silent: self.muted, repeat: String::new() });
        }
        if !finished.is_empty() {
            effects.push(Effect::Alarm);
            effects.extend(self.persist_timers(t));
            dirty = true;
        }
        let offset = offset_fn(self.zone, self.wall.offset);
        let fired = self.alarms.fire(wall_now, &offset);
        for a in &fired {
            self.push_alert(Alert { title: if a.label.trim().is_empty() { "Alarm".into() } else { a.label.clone() }, time: a.time_text(), is_timer: false, alarm_id: Some(a.id), sound: a.sound.clone(), vibrate: a.vibrate, ramp: a.ramp, snooze: a.snooze, silent: self.muted, repeat: if a.repeat == 0 { String::new() } else { a.repeat_text() } });
        }
        if !fired.is_empty() {
            effects.push(Effect::Alarm);
            effects.extend(self.persist(t));
            dirty = true;
        }
        dirty |= self.alert.is_some();
        (effects, dirty)
    }

    /// When the service next needs a tick on its own: a timer or alarm
    /// coming due, the once-a-second look at the app's files, the seconds
    /// of the alert on screen.
    pub fn next_wake(&self, t: f64, _visible: bool) -> Option<f64> {
        let mut due: Option<f64> = None;
        let mut want = |at: f64| due = Some(due.map_or(at, |d: f64| d.min(at)));
        if let Some(fire) = self.timers.next_due() {
            want(fire);
        }
        if let Some(next) = self.alarms.next_due(self.wall.now(t), &offset_fn(self.zone, self.wall.offset)) {
            want(self.wall.mono(next));
        }
        if self.alert.is_some() {
            let elapsed = (t - self.wall.at).rem_euclid(1000.0);
            want(t + 1000.0 - elapsed);
        }
        if self.path.is_some() {
            want(self.next_poll.max(t));
        }
        due
    }

    // ----- drawing -----

    #[allow(clippy::too_many_arguments)]
    fn label(&mut self, s: &mut Surface, ui: &Ui, text: &str, pt: f32, weight: Weight, color: u32, x: f32, mid: f32, align: f32) -> f32 {
        let img = self.cache.get(ui, text, pt, weight, color);
        let (w, h) = (img.w as f32, img.h as f32);
        s.blit(img, (x - w * align) as i32, (mid - h / 2.0) as i32, 255, 255);
        w
    }

    /// Text that changes often, drawn glyph by glyph; `align` 0 = left,
    /// 0.5 = centred, 1 = right of `x`.
    #[allow(clippy::too_many_arguments)]
    fn dynamic(&mut self, s: &mut Surface, ui: &Ui, text: &str, pt: f32, weight: Weight, color: u32, x: f32, mid: f32, align: f32) {
        let w = ui.font.measure(text, ui.pt(pt));
        let h = self.cache.line_h(ui, pt);
        self.cache.draw_chars(s, ui, text, pt, weight, color, x - w * align, mid - h / 2.0);
    }

    /// The full-screen alert over whatever is on screen.
    pub fn draw_alert(&mut self, s: &mut Surface, ui: &Ui, t: f64) {
        let Some(alert) = self.alert.clone() else { return };
        s.fill_rect(0, 0, self.w as usize, self.h as usize, BLACK);
        let cx = self.w as f32 / 2.0;
        self.dynamic(s, ui, &alert.title, 34.0, Weight::Regular, WHITE, cx, self.h as f32 * 0.3, 0.5);
        self.dynamic(s, ui, &alert.time, 120.0, Weight::Regular, WHITE, cx, self.h as f32 * 0.42, 0.5);
        let pulse = ((t / 500.0).sin() * 0.5 + 0.5) as f32;
        let r = ui.pt(14.0 + 4.0 * pulse);
        let c = (cx, self.h as f32 * 0.56);
        glyphs::alarm_clock(s, c, r * 2.0, ui.pt(3.0), ORANGE);
        if !alert.repeat.is_empty() {
            self.label(s, ui, &alert.repeat, 17.0, Weight::Regular, SECONDARY, cx, self.h as f32 * 0.34, 0.5);
        }
        if alert.silent {
            self.label(s, ui, "Silent", 15.0, Weight::Regular, SECONDARY, cx, self.h as f32 * 0.62, 0.5);
        }
        let (bx, by, bw, bh) = self.stop_button(ui);
        s.fill_rrect(bx, by, bw, bh, 1.0e4, ORANGE);
        self.label(s, ui, "Stop", 20.0, Weight::Semibold, WHITE, bx + bw / 2.0, by + bh / 2.0, 0.5);
        if alert.snooze {
            let (sx, sy, sw, sh) = self.snooze_button(ui);
            s.fill_rrect(sx, sy, sw, sh, 1.0e4, GREY_ACTIVE);
            self.label(s, ui, "Snooze", 20.0, Weight::Regular, WHITE, sx + sw / 2.0, sy + sh / 2.0, 0.5);
            self.label(s, ui, &format!("Rings again in {} min", alarm::SNOOZE_SECS / 60), 13.0, Weight::Regular, SECONDARY, cx, sy + sh + ui.pt(18.0), 0.5);
        }
    }
}

#[cfg(test)]
mod service_dir_tests {
    use super::*;

    #[test]
    fn the_service_reads_the_installed_app_s_data_folder_and_adopts_the_old_one() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/test-clock-service-dir");
        let _ = std::fs::remove_dir_all(&root);
        let new = service_dir(&root);
        assert_eq!(new, root.join("appdata").join("is.olibuijr.clock.desktop"), "the folder apphost gives the app as ctx.data_dir");
        let old = root.join("appdata").join(APP_ID);
        std::fs::create_dir_all(&old).unwrap();
        std::fs::create_dir_all(&new).unwrap();
        std::fs::write(old.join("clock.ini"), "alarm.0.time=07:00\n").unwrap();
        std::fs::write(old.join("clock-timers.ini"), "old").unwrap();
        std::fs::write(new.join("clock-timers.ini"), "new").unwrap();
        adopt_old_dir(&old, &new);
        assert_eq!(std::fs::read_to_string(new.join("clock.ini")).unwrap(), "alarm.0.time=07:00\n", "old alarms are kept");
        assert_eq!(std::fs::read_to_string(new.join("clock-timers.ini")).unwrap(), "new", "the app's own file wins");
        let _ = std::fs::remove_dir_all(&root);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_notification_app_id_is_the_clock_manifest_id() {
        // fired alarms are filed under the installed app by this id; the manifest must say the same
        let manifest = include_str!("../../../apps/clock-desktop/manifest.toml");
        assert!(manifest.contains(&format!("id = \"{MANIFEST_ID}\"")), "the Clock manifest id is {MANIFEST_ID}");
    }

    #[test]
    fn sample_reads_the_system_clock_with_the_zone_offset() {
        let w = Wall::sample(1234.0);
        assert_eq!(w.at, 1234.0);
        assert!(w.unix > 1_700_000_000);
        assert!(w.offset.abs() <= 14 * 3600);
    }

    fn ui() -> Ui {
        crate::shell::tests::ui()
    }

    fn clock(ui: &Ui) -> Clock {
        let mut c = Clock::new(ui, 1200, 1920);
        // Sat 3 Oct 2026 23:32:00 UTC, Reykjavik
        c.set_wall(tz::days_from_civil(2026, 10, 3) * 86_400 + 23 * 3600 + 32 * 60, 0, 1000.0);
        c
    }

    fn tap(c: &mut Clock, ui: &Ui, x: f32, y: f32, t: f64) -> Vec<Effect> {
        let mut fx = c.touch(ui, Event::Down(x, y), t);
        fx.extend(c.touch(ui, Event::Up(x, y), t + 20.0));
        fx
    }

    fn scratch(name: &str) -> String {
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/test-clock").join(std::process::id().to_string()).join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("clock.ini").to_string_lossy().into_owned()
    }

    fn draw(c: &mut Clock, ui: &Ui, t: f64) -> Vec<u32> {
        let mut buf = vec![0u32; 1200 * 1920];
        c.draw_alert(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut buf, y0: 0 }, ui, t);
        buf
    }

    #[test]
    fn a_timer_the_app_started_fires_an_alert_and_stop_clears_it() {
        let ui = ui();
        let path = scratch("timer");
        let mut c = clock(&ui);
        assert_eq!(c.load(&path, 1000.0), vec![Effect::WakeAt(0)]);
        // the app writes a one-second timer beside the alarms
        let mut timers = Timers::default();
        timers.start("", 1000, 2000.0, c.wall.now_ms(2000.0)).unwrap();
        store::write_atomic(&store::sibling(&path, "clock-timers.ini"), &timers.serialize()).unwrap();
        let (fx, _) = c.tick(&ui, 2000.0);
        assert_eq!(fx.len(), 1, "the service picked the timer up and armed the RTC");
        assert!(matches!(fx[0], Effect::WakeAt(at) if at > 0));
        assert_eq!(c.next_wake(2100.0, false), Some(3000.0), "wake when the timer fires even in the background");
        let (fx, dirty) = c.tick(&ui, 3000.0);
        assert_eq!(fx, vec![Effect::Alarm, Effect::WakeAt(0)]);
        assert!(dirty);
        assert_eq!(c.alert().map(|a| a.title.as_str()), Some("Timer"));
        assert!(c.alert().unwrap().is_timer);
        assert_eq!(c.drain_fired().len(), 1);
        assert!(c.drain_fired().is_empty(), "drained once");
        assert!(draw(&mut c, &ui, 3100.0).contains(&ORANGE));
        let (bx, by, bw, bh) = c.stop_button(&ui);
        tap(&mut c, &ui, bx + bw / 2.0, by + bh / 2.0, 5000.0);
        assert!(c.alert().is_none());
        assert!(c.timers().list.is_empty());
        assert!(std::fs::read_to_string(store::sibling(&path, "clock-timers.ini")).unwrap().lines().all(|l| l.starts_with('#')), "the finished timer is gone from the file");
    }

    #[test]
    fn a_timer_started_for_an_intent_is_written_for_the_app_and_rings() {
        let ui = ui();
        let path = scratch("intent");
        let mut c = clock(&ui);
        c.load(&path, 1000.0);
        assert_eq!(c.start_timer("Tea", 0, 1000.0), None, "a zero duration starts nothing");
        let id = c.start_timer("Tea", 2000, 1000.0).expect("a timer");
        let saved = Timers::parse(&std::fs::read_to_string(store::sibling(&path, "clock-timers.ini")).unwrap(), 1000.0, c.wall.now_ms(1000.0));
        assert!(saved.list.iter().any(|i| i.id == id && i.label == "Tea"), "the Clock app reads it from its file");
        let (fx, _) = c.tick(&ui, 3000.0);
        assert!(fx.contains(&Effect::Alarm));
        assert_eq!(c.alert().map(|a| a.title.as_str()), Some("Tea"));
    }

    #[test]
    fn a_timer_that_ended_while_the_shell_was_down_rings_on_the_first_tick() {
        let ui = ui();
        let path = scratch("late");
        let mut c = clock(&ui);
        c.load(&path, 1000.0);
        let mut timers = Timers::default();
        timers.start("10 min", 600_000, 1000.0, c.wall.now_ms(1000.0)).unwrap();
        store::write_atomic(&store::sibling(&path, "clock-timers.ini"), &timers.serialize()).unwrap();
        let mut late = Clock::new(&ui, 1200, 1920);
        late.set_wall(c.wall.unix + 700, 0, 100.0);
        late.load(&path, 100.0);
        let (fx, _) = late.tick(&ui, 200.0);
        assert!(fx.contains(&Effect::Alarm));
        assert_eq!(late.alert().map(|a| a.title.as_str()), Some("10 min"));
    }

    #[test]
    fn a_ringing_alarm_can_be_snoozed_nine_minutes() {
        let ui = ui();
        let path = scratch("snooze");
        let mut c = clock(&ui);
        c.app_running = true; // the Clock app writes the alarm below, so its files are watched every second
        c.load(&path, 1000.0);
        c.tick(&ui, 1500.0);
        let mut alarms = Alarms::default();
        alarms.add(alarm::Alarm::new(23, 33));
        store::write_atomic(&path, &alarms.serialize()).unwrap();
        c.tick(&ui, 2600.0);
        let (fx, _) = c.tick(&ui, 62_000.0);
        assert!(fx.contains(&Effect::Alarm), "{fx:?}");
        let ringing = c.alert().unwrap().clone();
        assert!(ringing.snooze && ringing.vibrate && ringing.ramp && !ringing.silent);
        assert_eq!((ringing.sound.as_str(), ringing.is_timer), ("Radar", false));
        draw(&mut c, &ui, 62_100.0);
        let (sx, sy, sw, sh) = c.snooze_button(&ui);
        tap(&mut c, &ui, sx + sw / 2.0, sy + sh / 2.0, 70_000.0);
        assert!(c.alert().is_none());
        assert_eq!(c.alarms().list[0].snoozed_until, Some(c.wall.now(70_020.0) + 540));
        assert!(c.next_wake(70_100.0, false).is_some(), "the snooze arms the wake-up");
        let (fx, _) = c.tick(&ui, 70_020.0 + 541_000.0);
        assert!(fx.contains(&Effect::Alarm));
        assert!(c.alert().is_some());
        // stopping it for good clears the snooze
        let (bx, by, bw, bh) = c.stop_button(&ui);
        tap(&mut c, &ui, bx + bw / 2.0, by + bh / 2.0, 700_000.0);
        assert_eq!(c.alarms().list[0].snoozed_until, None);
        assert!(c.next_alarm_text(700_100.0).is_none(), "a one-shot alarm that rang is off");
    }

    #[test]
    fn an_alarm_fires_once_and_the_app_sees_it_switched_off() {
        let ui = ui();
        let path = scratch("once");
        let mut c = clock(&ui);
        c.load(&path, 1000.0);
        let mut alarms = Alarms::default();
        alarms.add(alarm::Alarm::new(23, 33));
        store::write_atomic(&path, &alarms.serialize()).unwrap();
        c.tick(&ui, 2600.0);
        let (fx, _) = c.tick(&ui, 1000.0 + 61_000.0);
        assert!(fx.contains(&Effect::Alarm), "{fx:?}");
        assert_eq!(c.alert().map(|a| a.time.as_str()), Some("23:33"));
        assert!(c.tick(&ui, 1000.0 + 62_000.0).0.is_empty());
        assert!(!Alarms::parse(&std::fs::read_to_string(&path).unwrap()).list[0].on, "the one-shot alarm is off on disk");
    }

    #[test]
    fn the_alert_takes_no_touches_outside_its_buttons() {
        let ui = ui();
        let mut c = clock(&ui);
        assert!(tap(&mut c, &ui, 10.0, 10.0, 0.0).is_empty());
        assert!(c.alert().is_none());
        assert!(c.next_wake(0.0, true).is_none(), "nothing pending without files or alerts");
    }
}
