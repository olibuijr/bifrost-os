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
use super::prefs::Prefs;
use bifrost_core::idle_monitor::{IdleEvent, IdleMonitor};
use bifrost_ui::saver_random::SaverRandom;
use bifrost_ui::screensaver::{ConfigureSheet, ScreenSaverView};
use bifrost_ui::screensaver_manifest::SaverManifest;
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

/// The descriptor of each saver in `SAVERS` order: its name, frame interval and the options its configuration sheet
/// shows in Settings > Screen Saver. `SaverManifest::parse` turns each into the saver's view settings and sheet.
const DESCRIPTORS: [&str; 3] = [
    "name = Clock\nframe_interval = 1\noption.show_date = bool true\noption.move_every = choice 30s,1m,5m 1m\noption.size = choice small,medium,large medium\n",
    "name = Colours\nframe_interval = 0.1\noption.speed = choice slow,normal,fast normal\n",
    "name = Black\n",
];

/// The parsed descriptor of a saver by index, clamped to the list.
pub fn manifest(index: usize) -> Option<SaverManifest> {
    SaverManifest::parse(DESCRIPTORS[index.min(DESCRIPTORS.len() - 1)]).ok()
}

/// The preference key of one saver option. It has the form `ScreenSaver.<module>.<key>`, the same key space the SDK's
/// per-module defaults use, so one module never reads another's values.
pub fn pref_key(index: usize, key: &str) -> String {
    format!("ScreenSaver.{}.{}", saver_name(index).replace('.', "_"), key)
}

/// The saver's configuration sheet with the values saved in `prefs` applied. A saved value the option refuses is
/// skipped, so an old or edited preference cannot break the sheet.
pub fn load_sheet(index: usize, prefs: &Prefs) -> ConfigureSheet {
    let mut sheet = manifest(index).map(|m| m.sheet().clone()).unwrap_or_default();
    let keys: Vec<String> = sheet.keys().iter().map(|k| k.to_string()).collect();
    for key in keys {
        if let Some(text) = prefs.get(&pref_key(index, &key)) {
            sheet.set_from_text(&key, text);
        }
    }
    sheet
}

/// Saves one option value (as the text `ConfigureSheet::text_of` gives) to the saver's preferences.
pub fn save_option(prefs: &mut Prefs, index: usize, key: &str, text: &str) {
    let _ = prefs.set(&pref_key(index, key), text);
}

/// The label of an option in Settings.
pub fn option_title(key: &str) -> String {
    match key {
        "show_date" => "Show Date".to_string(),
        "move_every" => "Move Every".to_string(),
        "size" => "Clock Size".to_string(),
        "speed" => "Colour Change".to_string(),
        _ => capitalised(key),
    }
}

/// The label of a choice value in Settings.
pub fn choice_title(value: &str) -> String {
    match value {
        "30s" => "30 seconds".to_string(),
        "1m" => "1 minute".to_string(),
        "5m" => "5 minutes".to_string(),
        _ => capitalised(value),
    }
}

fn capitalised(text: &str) -> String {
    let spaced = text.replace('_', " ");
    let mut chars = spaced.chars();
    chars.next().map_or(String::new(), |c| c.to_uppercase().collect::<String>() + chars.as_str())
}

/// Where the Clock saver puts its clock in move number `cell`: the same cell always gives the same spot, because the
/// position comes from a `SaverRandom` seeded with the cell number.
pub fn clock_spot(cell: u64, size: (f32, f32), area: Rect) -> (f32, f32) {
    SaverRandom::new(cell).next_point_within(size, area)
}

/// The clock text the Clock saver last drew, with its images.
struct Text {
    clock: String,
    date: String,
    /// the clock size in points the images were made at
    time_pt: f32,
    time: Image,
    /// None when Show Date is off
    day: Option<Image>,
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
    /// the chosen saver's options and their saved values (see `load_sheet`)
    sheet: ConfigureSheet,
}

impl ScreenSaver {
    pub fn new() -> Self {
        let frame = Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 };
        ScreenSaver { view: ScreenSaverView::new(frame, false), saver: 0, started_ms: 0.0, idle: IdleMonitor::new(0, 0), text: None, sheet: ConfigureSheet::new() }
    }

    /// Takes the saver choice and the Start After delay from Settings (see `Options`). An index past the list is the
    /// last saver.
    pub fn configure(&mut self, saver: usize, minutes: u32) {
        self.saver = saver.min(SAVERS.len() - 1);
        self.idle.set_timeout(u64::from(minutes) * 60_000);
    }

    /// Takes the chosen saver's options from its saved values (`load_sheet`) and its frame interval from its
    /// descriptor, so a static clock is not redrawn thirty times a second.
    pub fn set_sheet(&mut self, sheet: ConfigureSheet) {
        self.sheet = sheet;
        if let Some(m) = manifest(self.saver) {
            self.view.set_animation_time_interval(m.frame_interval());
        }
    }

    /// Seconds the Clock saver keeps its clock still before it moves it.
    fn move_every_s(&self) -> f64 {
        match self.sheet.choice_of("move_every") {
            Some("30s") => 30.0,
            Some("5m") => 300.0,
            _ => MOVE_EVERY_S,
        }
    }

    /// The Clock saver's time text size in points.
    fn clock_pt(&self) -> f32 {
        match self.sheet.choice_of("size") {
            Some("small") => 64.0,
            Some("large") => 128.0,
            _ => 96.0,
        }
    }

    /// How fast the Colours saver turns through the colour wheel, in turns of the phase per second.
    fn colour_rate(&self) -> f64 {
        match self.sheet.choice_of("speed") {
            Some("slow") => 0.02,
            Some("fast") => 0.12,
            _ => 0.05,
        }
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
                let phase = elapsed * self.colour_rate();
                let channel = |k: f64| (128.0 + 127.0 * (phase + k).sin()) as u8;
                s.fill_rrect(0.0, 0.0, w as f32, h as f32, 0.0, pack(channel(0.0), channel(2.094), channel(4.188), 255));
            }
            _ => s.fill_rrect(0.0, 0.0, w as f32, h as f32, 0.0, BLACK),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_clock(&mut self, s: &mut Surface, ui: &Ui, w: i32, h: i32, clock: &str, date: &str, elapsed: f64) {
        s.fill_rrect(0.0, 0.0, w as f32, h as f32, 0.0, BLACK);
        let show_date = self.sheet.bool_or("show_date", true);
        let time_pt = self.clock_pt();
        let current = self.text.as_ref().is_some_and(|t| t.clock == clock && t.date == date && t.time_pt == time_pt && t.day.is_some() == show_date);
        if !current {
            self.text = Some(Text {
                clock: clock.to_string(),
                date: date.to_string(),
                time_pt,
                time: ui.text(clock, time_pt, Weight::Semibold, WHITE),
                day: show_date.then(|| ui.text(date, 24.0, Weight::Semibold, SECONDARY)),
            });
        }
        let cell = (elapsed / self.move_every_s()).floor() as u64;
        let Some(text) = &self.text else { return };
        let gap = if text.day.is_some() { ui.pt(8.0) } else { 0.0 };
        let day_w = text.day.as_ref().map_or(0, |d| d.w);
        let day_h = text.day.as_ref().map_or(0, |d| d.h);
        let size = (text.time.w.max(day_w) as f32, text.time.h as f32 + gap + day_h as f32);
        let area = Rect { x: 0.0, y: 0.0, w: w as f32, h: h as f32 };
        let (x, y) = clock_spot(cell, size, area);
        let cx = (x + size.0 / 2.0) as i32;
        centred(s, &text.time, cx, y as i32);
        if let Some(day) = &text.day {
            centred(s, day, cx, (y + text.time.h as f32 + gap) as i32);
        }
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
        let clock = status::clock_in(self.info.hour, self.info.min, self.clock_style.use_24h);
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
    fn the_clock_spot_is_repeatable_and_inside_the_screen() {
        let area = Rect { x: 0.0, y: 0.0, w: 1920.0, h: 1080.0 };
        let size = (400.0, 160.0);
        for cell in [0u64, 1, 2, 59, u64::MAX] {
            let (x, y) = clock_spot(cell, size, area);
            assert!(x >= 0.0 && x + size.0 <= area.w, "cell {cell}: x {x}");
            assert!(y >= 0.0 && y + size.1 <= area.h, "cell {cell}: y {y}");
            assert_eq!(clock_spot(cell, size, area), (x, y), "the same cell gives the same spot");
        }
        assert_ne!(clock_spot(0, size, area), clock_spot(1, size, area), "the next move goes somewhere else");
    }

    #[test]
    fn every_saver_has_a_valid_descriptor() {
        for (i, name) in SAVERS.iter().enumerate() {
            let m = manifest(i).expect("descriptor parses");
            assert_eq!(m.name(), *name);
            assert!(m.frame_interval() > 0.0);
        }
        assert_eq!(manifest(0).map(|m| m.sheet().len()), Some(3), "Clock: date, move and size");
        assert_eq!(manifest(1).map(|m| m.sheet().len()), Some(1), "Colours: speed");
        assert_eq!(manifest(2).map(|m| m.sheet().len()), Some(0), "Black has no options");
    }

    #[test]
    fn saved_option_values_load_and_bad_ones_are_skipped() {
        let mut prefs = Prefs::memory();
        assert_eq!(pref_key(0, "show_date"), "ScreenSaver.Clock.show_date");
        save_option(&mut prefs, 0, "show_date", "false");
        save_option(&mut prefs, 0, "move_every", "5m");
        save_option(&mut prefs, 0, "size", "huge");
        let sheet = load_sheet(0, &prefs);
        assert!(!sheet.bool_or("show_date", true));
        assert_eq!(sheet.choice_of("move_every"), Some("5m"));
        assert_eq!(sheet.choice_of("size"), Some("medium"), "a refused value leaves the default");
        // another module's values are not read
        assert_eq!(load_sheet(1, &prefs).choice_of("speed"), Some("normal"));
    }

    #[test]
    fn the_options_change_the_clock_and_colour_settings() {
        let mut prefs = Prefs::memory();
        let mut s = ScreenSaver::new();
        s.configure(0, 1);
        s.set_sheet(load_sheet(0, &prefs));
        assert_eq!((s.move_every_s(), s.clock_pt()), (MOVE_EVERY_S, 96.0));
        assert!((s.view.animation_time_interval() - 1.0).abs() < 1e-9, "the Clock descriptor asks for one frame a second");
        save_option(&mut prefs, 0, "move_every", "30s");
        save_option(&mut prefs, 0, "size", "large");
        s.set_sheet(load_sheet(0, &prefs));
        assert_eq!((s.move_every_s(), s.clock_pt()), (30.0, 128.0));
        s.configure(1, 1);
        save_option(&mut prefs, 1, "speed", "fast");
        s.set_sheet(load_sheet(1, &prefs));
        assert_eq!(s.colour_rate(), 0.12);
    }

    #[test]
    fn option_titles_read_well() {
        assert_eq!(option_title("show_date"), "Show Date");
        assert_eq!(option_title("custom_key"), "Custom key");
        assert_eq!(choice_title("30s"), "30 seconds");
        assert_eq!(choice_title("medium"), "Medium");
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
