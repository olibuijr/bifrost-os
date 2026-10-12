//! Settings > Screen Saver: the saver choice, the Start After delay, idle activation, the Preview, and the three savers
//! (Clock, Colours, Black) drawn over the whole screen.
//!
//! A saver starts once the computer has been idle for the chosen time. The idle clock is `bifrost_core`'s
//! `IdleMonitor`, fed with the shell's last-input time. It does not change the lock or the display state. The first
//! touch or key press ends it and is not passed on, so whatever was on screen before (the desktop, or the lock screen)
//! shows again through the existing path. Input in the first `GRACE_MS` after a start is ignored, so the touch that
//! opened a Preview does not end it. A saver also ends as soon as the display goes off, and the display sleep timer
//! (Auto-Lock) runs underneath as before.
//!
//! The frame clock and the random helpers come from `bifrost_ui::screensaver::ScreenSaverView`. Times here and in
//! `Shell` are the shell's milliseconds; the view takes seconds.

use super::ui::{centred, Ui, Weight, SECONDARY, WHITE};
use super::Shell;
use crate::gfx::{pack, Image, Surface};
use crate::status;
use bifrost_core::idle_monitor::{IdleEvent, IdleMonitor};
use bifrost_ui::screensaver::{ss_random_point_for_size_within_rect, ScreenSaverView};
use bifrost_ui::Rect;

/// The savers, in the order the Screen Saver page lists them. Index 0 is the default.
pub const SAVERS: [&str; 3] = ["Clock", "Colours", "Black"];
/// Start After choices as (label, minutes); 0 is Never.
pub const START_AFTER: [(&str, u32); 7] = [
    ("1 minute", 1),
    ("2 minutes", 2),
    ("5 minutes", 5),
    ("10 minutes", 10),
    ("20 minutes", 20),
    ("30 minutes", 30),
    ("Never", 0),
];
/// Input this soon after a saver starts does not end it, in milliseconds.
pub const GRACE_MS: f64 = 400.0;
/// The Clock saver keeps its clock still for this long, then moves it to a new spot.
const MOVE_EVERY_S: f64 = 60.0;
const BLACK: u32 = pack(0, 0, 0, 255);

/// The name of a saver by index, clamped to the list.
pub fn saver_name(index: usize) -> &'static str {
    SAVERS[index.min(SAVERS.len() - 1)]
}

/// A unit value in [0, 1) from a seed, with a fixed integer hash, so the same seed always gives the same spot.
fn unit(seed: u64) -> f64 {
    let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    (z >> 11) as f64 / (1u64 << 53) as f64
}

/// The clock text the Clock saver last drew, with its images.
struct Text {
    clock: String,
    date: String,
    time: Image,
    day: Image,
}

/// The screen saver state the shell holds.
pub struct ScreenSaver {
    view: ScreenSaverView,
    /// index into `SAVERS`
    saver: usize,
    /// shell milliseconds when the saver last started
    started_ms: f64,
    /// idle time before the saver starts; its timeout is 0 (never) when Start After is Never
    idle: IdleMonitor,
    text: Option<Text>,
}

impl ScreenSaver {
    pub fn new() -> Self {
        let frame = Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 };
        ScreenSaver { view: ScreenSaverView::new(frame, false), saver: 0, started_ms: 0.0, idle: IdleMonitor::new(0, 0), text: None }
    }

    /// Takes the saver choice and the Start After delay from Settings (see `Options`). An index past the list is the
    /// last saver.
    pub fn configure(&mut self, saver: usize, minutes: u32) {
        self.saver = saver.min(SAVERS.len() - 1);
        self.idle.set_timeout(u64::from(minutes) * 60_000);
    }

    /// Whether a saver is on screen.
    pub fn active(&self) -> bool {
        self.view.is_animating()
    }

    fn start(&mut self, now_ms: f64) {
        self.view.start_animation(now_ms / 1000.0);
        self.started_ms = now_ms;
    }

    fn stop(&mut self) {
        self.view.stop_animation();
    }

    /// Settings > Screen Saver > Preview: the saver starts now, whatever the idle time.
    pub fn start_preview(&mut self, now_ms: f64) {
        self.stop();
        self.start(now_ms);
    }

    fn past_grace(&self, now_ms: f64) -> bool {
        now_ms - self.started_ms >= GRACE_MS
    }

    /// A touch or key press. Returns true when a saver was on screen and past its grace period: it ends and the caller
    /// does not pass the event on.
    pub fn wake(&mut self, now_ms: f64) -> bool {
        if self.active() && self.past_grace(now_ms) {
            self.stop();
            true
        } else {
            false
        }
    }

    /// Runs once per frame. Ends a saver on input that came after it started, starts one when the idle time has passed,
    /// and advances the saver's frame clock. `last_input_ms` is the shell's last input time. Returns true when the shell
    /// should draw again.
    pub fn tick(&mut self, now_ms: f64, last_input_ms: f64, screen_on: bool) -> bool {
        if !screen_on {
            self.stop();
            return false;
        }
        let mut redraw = false;
        if self.active() {
            if last_input_ms > self.started_ms && self.past_grace(now_ms) {
                self.stop();
                return true;
            }
            let frames = self.view.tick(now_ms / 1000.0);
            for _ in 0..frames {
                self.view.animate_one_frame();
            }
            redraw = frames > 0;
        }
        // input resets the idle clock (and ends an idle period); the poll starts the saver once the gap is long enough
        self.idle.note_activity(last_input_ms.max(0.0) as u64);
        if self.idle.poll(now_ms.max(0.0) as u64) == Some(IdleEvent::BecameIdle) && !self.active() {
            self.start(now_ms);
            redraw = true;
        }
        redraw
    }

    /// Draws the saver over the whole `w` by `h` screen. `clock` and `date` are the status bar's texts, used by Clock.
    pub fn draw(&mut self, s: &mut Surface, ui: &Ui, w: i32, h: i32, clock: &str, date: &str, now_ms: f64) {
        let elapsed = (now_ms - self.started_ms).max(0.0) / 1000.0;
        match self.saver {
            0 => self.draw_clock(s, ui, w, h, clock, date, elapsed),
            1 => {
                let phase = elapsed * 0.05;
                let channel = |k: f64| (128.0 + 127.0 * (phase + k).sin()) as u8;
                s.fill_rrect(0.0, 0.0, w as f32, h as f32, 0.0, pack(channel(0.0), channel(2.094), channel(4.188), 255));
            }
            _ => s.fill_rrect(0.0, 0.0, w as f32, h as f32, 0.0, BLACK),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_clock(&mut self, s: &mut Surface, ui: &Ui, w: i32, h: i32, clock: &str, date: &str, elapsed: f64) {
        s.fill_rrect(0.0, 0.0, w as f32, h as f32, 0.0, BLACK);
        let current = self.text.as_ref().is_some_and(|t| t.clock == clock && t.date == date);
        if !current {
            self.text = Some(Text {
                clock: clock.to_string(),
                date: date.to_string(),
                time: ui.text(clock, 96.0, Weight::Semibold, WHITE),
                day: ui.text(date, 24.0, Weight::Semibold, SECONDARY),
            });
        }
        let Some(text) = &self.text else { return };
        let gap = ui.pt(8.0);
        let size = (text.time.w.max(text.day.w) as f32, text.time.h as f32 + gap + text.day.h as f32);
        let cell = (elapsed / MOVE_EVERY_S).floor() as u64;
        let area = Rect { x: 0.0, y: 0.0, w: w as f32, h: h as f32 };
        let (x, y) = ss_random_point_for_size_within_rect(unit(cell * 2), unit(cell * 2 + 1), size, area);
        let cx = (x + size.0 / 2.0) as i32;
        centred(s, &text.time, cx, y as i32);
        centred(s, &text.day, cx, (y + text.time.h as f32 + gap) as i32);
    }
}

impl Shell {
    /// Starts an idle saver and advances a running one. Called from the tick, before the display sleep check.
    pub(super) fn saver_tick(&mut self, t: f64) {
        if self.saver.tick(t, self.last_input, self.screen_on) {
            self.dirty = true;
        }
    }

    /// A touch or key press: true when it ended a saver, and the event is then not passed on.
    pub(super) fn saver_wake(&mut self, t: f64) -> bool {
        self.saver.wake(t)
    }

    /// Settings > Screen Saver > Preview.
    pub(super) fn saver_preview(&mut self, t: f64) {
        self.saver.start_preview(t);
        self.dirty = true;
    }

    /// Draws the saver when one is on screen; false when the desktop or lock screen should draw.
    pub(super) fn saver_draw(&mut self, s: &mut Surface, t: f64) -> bool {
        if !self.screen_on || !self.saver.active() {
            return false;
        }
        let clock = status::clock_in(self.info.hour, self.info.min, self.clock_24h);
        let date = status::date_text(self.info.wday, self.info.mday, self.info.mon);
        self.saver.draw(s, &self.ui, self.w, self.h, &clock, &date, t);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_saver_index_is_clamped_to_the_list() {
        let mut s = ScreenSaver::new();
        s.configure(9, 5);
        assert_eq!((s.saver, s.idle.timeout_ms()), (2, 300_000));
        assert_eq!(saver_name(9), "Black");
        assert_eq!(saver_name(0), "Clock");
    }

    #[test]
    fn unit_values_stay_in_range() {
        for seed in [0u64, 1, 2, 1 << 40, u64::MAX] {
            let u = unit(seed);
            assert!((0.0..1.0).contains(&u), "{seed} -> {u}");
        }
        assert_eq!(unit(7), unit(7), "the same seed gives the same value");
    }

    #[test]
    fn idle_for_the_delay_starts_the_saver() {
        let mut s = ScreenSaver::new();
        s.configure(1, 1);
        // the last input at 1 000 ms; the idle timeout is 60 000 ms, reached at 61 000 ms
        assert!(!s.tick(60_999.0, 1_000.0, true));
        assert!(!s.active());
        assert!(s.tick(61_000.0, 1_000.0, true));
        assert!(s.active());
    }

    #[test]
    fn a_touch_after_the_grace_period_ends_the_saver() {
        let mut s = ScreenSaver::new();
        s.configure(1, 1);
        s.tick(61_000.0, 1_000.0, true);
        assert!(s.active());
        assert!(!s.wake(61_000.0 + 100.0), "inside the grace period the touch is not taken");
        assert!(s.active());
        assert!(s.wake(61_000.0 + GRACE_MS), "after it, the touch ends the saver");
        assert!(!s.active());
        // that touch is activity, so the idle clock restarts and nothing starts again at once
        assert!(!s.tick(61_500.0, 61_400.0, true));
        assert!(!s.active());
    }

    #[test]
    fn input_after_the_start_ends_a_preview_on_the_next_tick() {
        let mut s = ScreenSaver::new();
        s.configure(0, 0);
        s.start_preview(10_000.0);
        assert!(s.active());
        // the input that opened the preview is not newer than its start, so it does not end it
        s.tick(10_100.0, 10_000.0, true);
        assert!(s.active());
        // movement 100 ms later is inside the grace period
        s.tick(10_200.0, 10_100.0, true);
        assert!(s.active(), "inside the grace period the input is ignored");
        // a later input, past the grace period, ends it
        assert!(s.tick(10_500.0, 10_450.0, true));
        assert!(!s.active());
    }

    #[test]
    fn the_saver_stops_when_the_display_goes_off() {
        let mut s = ScreenSaver::new();
        s.configure(0, 1);
        s.start_preview(0.0);
        assert!(s.active());
        assert!(!s.tick(1_000.0, 0.0, false));
        assert!(!s.active());
    }

    #[test]
    fn never_starts_without_a_delay() {
        let mut s = ScreenSaver::new();
        s.configure(0, 0);
        assert!(!s.tick(9_999_999.0, 1.0, true));
        assert!(!s.active());
    }
}
