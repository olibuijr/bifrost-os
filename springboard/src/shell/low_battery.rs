//! Low battery: when to warn, when to shut down cleanly, what Low Power
//! Mode changes, and the bifrost-style "Low Battery" alert.
//!
//! The decisions are pure (`Monitor::update`, `relevant`, the Low Power
//! helpers) so they are testable without a screen; `Alert` only draws.
//! The alerts follow bifrost: an offer of Low Power Mode at 20 %, then
//! "Low Battery" at 15 % and 5 %. A level speaks once per discharge: it is
//! re-armed only after the charge climbs `HYSTERESIS` points above its
//! threshold, so a level that wobbles around a threshold never repeats.
//! At 3 % or less, unplugged, the shell shuts the bifrost down cleanly
//! instead of letting the battery brown it out mid-write.

use super::battery::Sample;
use super::pages::uptime;
use super::ui::{AlertCard, Ui, Weight};
use super::wifi_page::BLUE;
use crate::gfx::Surface;

/// Offer Low Power Mode at this level (percent).
pub const OFFER_AT: u8 = 20;
/// "Low Battery" alerts.
pub const WARN_AT: u8 = 15;
pub const CRITICAL_AT: u8 = 5;
/// Unplugged at or below this, shut down.
pub const SHUTDOWN_AT: u8 = 3;
/// A level re-arms once the charge reaches its threshold plus this.
pub const HYSTERESIS: u8 = 3;
/// How often the battery is looked at.
pub const CHECK_MS: f64 = 1000.0;
/// Consecutive low readings (one per `CHECK_MS`) before a shutdown, so a
/// glitched sysfs read never powers the bifrost off.
pub const SHUTDOWN_CONFIRMS: u32 = 3;

/// Low Power Mode caps the backlight at this fraction of full.
pub const LOW_POWER_BRIGHTNESS: f32 = 0.5;
/// ... and locks the screen after at most this long (like bifrost's 30 s).
pub const LOW_POWER_LOCK_MS: u32 = 30_000;
/// ... and draws at most 30 frames a second.
pub const LOW_POWER_FRAME_MS: f64 = 1000.0 / 30.0;

/// Backlight level after Low Power Mode's cap.
pub fn cap_brightness(level: f32, low_power: bool) -> f32 {
    if low_power {
        level.min(LOW_POWER_BRIGHTNESS)
    } else {
        level
    }
}

/// Auto-Lock in ms after Low Power Mode: never (0) and anything longer
/// than `LOW_POWER_LOCK_MS` shorten to it.
pub fn auto_lock_ms(ms: u32, low_power: bool) -> u32 {
    match (low_power, ms) {
        (true, 0) => LOW_POWER_LOCK_MS,
        (true, ms) => ms.min(LOW_POWER_LOCK_MS),
        (false, ms) => ms,
    }
}

/// Minimum time between presented frames; 0 means paced by the display only.
pub fn frame_interval_ms(low_power: bool) -> f64 {
    if low_power {
        LOW_POWER_FRAME_MS
    } else {
        0.0
    }
}

/// The alert levels, least severe first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Offer,
    Low,
    Critical,
}

const LEVELS: [Level; 3] = [Level::Offer, Level::Low, Level::Critical];

impl Level {
    pub fn threshold(self) -> u8 {
        match self {
            Level::Offer => OFFER_AT,
            Level::Low => WARN_AT,
            Level::Critical => CRITICAL_AT,
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// What the monitor wants done about a reading.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Nothing,
    Show(Level),
    Shutdown,
}

#[derive(Debug, Default)]
pub struct Monitor {
    /// per level: already spoken since it last re-armed
    shown: [bool; 3],
    low_reads: u32,
    shutting_down: bool,
}

impl Monitor {
    /// Feeds one reading. Only the most severe crossed level speaks, and
    /// it marks every milder one as spoken too.
    pub fn update(&mut self, percent: u8, charging: bool, low_power: bool) -> Verdict {
        for l in LEVELS {
            if percent >= l.threshold() + HYSTERESIS {
                self.shown[l.index()] = false;
            }
        }
        if charging {
            self.low_reads = 0;
            return Verdict::Nothing;
        }
        if percent <= SHUTDOWN_AT {
            self.low_reads += 1;
            if self.low_reads >= SHUTDOWN_CONFIRMS && !self.shutting_down {
                self.shutting_down = true;
                return Verdict::Shutdown;
            }
        } else {
            self.low_reads = 0;
        }
        let Some(level) = LEVELS.into_iter().rev().find(|l| percent <= l.threshold()) else { return Verdict::Nothing };
        if self.shown[level.index()] {
            return Verdict::Nothing;
        }
        for l in LEVELS.into_iter().filter(|l| l.index() <= level.index()) {
            self.shown[l.index()] = true;
        }
        // an offer for what is already on is pointless
        if level == Level::Offer && low_power {
            return Verdict::Nothing;
        }
        Verdict::Show(level)
    }
}

/// Whether an alert for `level` still describes the battery: plugging in,
/// or charging past the hysteresis band, takes it away by itself.
pub fn relevant(level: Level, percent: u8, charging: bool) -> bool {
    !charging && percent < level.threshold() + HYSTERESIS
}

/// Battery page's "Last Charged": the newest logged charging sample.
pub fn last_charged(samples: &[Sample], now: i64, charging: bool) -> String {
    if charging {
        return "Now".into();
    }
    match samples.iter().rev().find(|s| s.charging) {
        Some(s) => format!("{} ago", uptime((now - s.unix).max(0) / 60)),
        None => "—".into(),
    }
}

/// The alert's buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    LowPowerMode,
    Dismiss,
}

/// A NativeAlertController-style "Low Battery" alert.
pub struct Alert {
    pub level: Level,
    card: AlertCard,
    buttons: Vec<Button>,
}

impl Alert {
    /// `low_power`: Low Power Mode is already on, so only "OK" is offered.
    pub fn new(ui: &Ui, level: Level, percent: u8, low_power: bool) -> Self {
        let mut text = format!("{percent}% of battery remaining.");
        if !low_power {
            text.push_str(" Low Power Mode reduces brightness, screen-on time and animation smoothness until you can charge.");
        }
        let (buttons, labels) = if low_power {
            (vec![Button::Dismiss], vec![ui.text("OK", 17.0, Weight::Semibold, BLUE)])
        } else {
            let dismiss = if level == Level::Offer { "Not Now" } else { "Close" };
            (vec![Button::Dismiss, Button::LowPowerMode], vec![ui.text(dismiss, 17.0, Weight::Regular, BLUE), ui.text("Low Power Mode", 17.0, Weight::Semibold, BLUE)])
        };
        Alert { level, card: AlertCard::new(ui, "Low Battery", &text, labels), buttons }
    }

    /// The button under (x, y), if any.
    pub fn hit(&self, ui: &Ui, w: i32, h: i32, x: f32, y: f32) -> Option<Button> {
        self.card.hit(ui, w, h, x, y).map(|i| self.buttons[i])
    }

    /// Centre of a button, for tests.
    pub fn button(&self, ui: &Ui, w: i32, h: i32, which: Button) -> Option<(f32, f32)> {
        let i = self.buttons.iter().position(|b| *b == which)?;
        Some(self.card.button_centre(ui, w, h, i))
    }

    pub fn draw(&self, s: &mut Surface, ui: &Ui, w: i32, h: i32) {
        self.card.draw(s, ui, w, h);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(m: &mut Monitor, readings: &[u8]) -> Vec<Verdict> {
        readings.iter().map(|&p| m.update(p, false, false)).collect()
    }

    #[test]
    fn each_level_speaks_once_as_the_charge_falls() {
        let mut m = Monitor::default();
        let v = run(&mut m, &[30, 21, 20, 19, 16, 15, 14, 6, 5, 4]);
        assert_eq!(v[2], Verdict::Show(Level::Offer));
        assert_eq!(v[5], Verdict::Show(Level::Low));
        assert_eq!(v[8], Verdict::Show(Level::Critical));
        assert_eq!(v.iter().filter(|v| **v != Verdict::Nothing).count(), 3);
    }

    #[test]
    fn jumping_past_levels_shows_only_the_most_severe() {
        let mut m = Monitor::default();
        assert_eq!(m.update(4, false, false), Verdict::Show(Level::Critical));
        assert_eq!(m.update(4, false, false), Verdict::Nothing);
    }

    #[test]
    fn a_level_wobbling_around_its_threshold_does_not_repeat() {
        let mut m = Monitor::default();
        assert_eq!(m.update(15, false, false), Verdict::Show(Level::Low));
        // 16..=17 is inside the hysteresis band
        assert_eq!(run(&mut m, &[16, 17, 15, 17, 15]), vec![Verdict::Nothing; 5]);
        // 18 = 15 + 3 re-arms it
        assert_eq!(m.update(18, false, false), Verdict::Nothing);
        assert_eq!(m.update(15, false, false), Verdict::Show(Level::Low));
    }

    #[test]
    fn charging_stays_quiet_but_keeps_the_hysteresis_state() {
        let mut m = Monitor::default();
        assert_eq!(m.update(14, false, false), Verdict::Show(Level::Low));
        assert_eq!(m.update(14, true, false), Verdict::Nothing);
        assert_eq!(m.update(14, false, false), Verdict::Nothing, "unplugging again does not respeak");
        m.update(40, true, false);
        assert_eq!(m.update(14, false, false), Verdict::Show(Level::Low), "re-armed by a real charge");
    }

    #[test]
    fn the_offer_is_skipped_when_low_power_is_already_on() {
        let mut m = Monitor::default();
        assert_eq!(m.update(20, false, true), Verdict::Nothing);
        assert_eq!(m.update(15, false, true), Verdict::Show(Level::Low));
    }

    #[test]
    fn shutdown_needs_consecutive_unplugged_low_readings() {
        let mut m = Monitor::default();
        assert_eq!(m.update(3, false, false), Verdict::Show(Level::Critical));
        assert_eq!(m.update(3, false, false), Verdict::Nothing);
        assert_eq!(m.update(3, false, false), Verdict::Shutdown);
        assert_eq!(m.update(2, false, false), Verdict::Nothing, "only once");
        let mut m = Monitor::default();
        m.update(3, false, false);
        m.update(3, false, false);
        assert_eq!(m.update(9, false, false), Verdict::Nothing, "a good read resets the count");
        assert_eq!(m.update(3, false, false), Verdict::Show(Level::Critical), "re-armed by the 9 % read");
        assert_eq!(m.update(3, false, false), Verdict::Nothing, "the count restarted");
        let mut m = Monitor::default();
        assert!([3, 2, 1, 0].iter().all(|&p| m.update(p, true, false) == Verdict::Nothing), "never while charging");
    }

    #[test]
    fn an_alert_goes_away_when_it_stops_being_true() {
        assert!(relevant(Level::Low, 15, false));
        assert!(relevant(Level::Low, 17, false));
        assert!(!relevant(Level::Low, 18, false));
        assert!(!relevant(Level::Low, 10, true), "plugged in");
    }

    #[test]
    fn low_power_mode_caps_brightness_lock_time_and_frame_rate() {
        assert_eq!(cap_brightness(0.9, true), 0.5);
        assert_eq!(cap_brightness(0.3, true), 0.3);
        assert_eq!(cap_brightness(0.9, false), 0.9);
        assert_eq!(auto_lock_ms(0, true), LOW_POWER_LOCK_MS);
        assert_eq!(auto_lock_ms(300_000, true), LOW_POWER_LOCK_MS);
        assert_eq!(auto_lock_ms(30_000, true), 30_000);
        assert_eq!(auto_lock_ms(0, false), 0);
        assert_eq!(auto_lock_ms(120_000, false), 120_000);
        assert_eq!(frame_interval_ms(false), 0.0);
        assert!((frame_interval_ms(true) - 33.3).abs() < 0.1);
    }

    #[test]
    fn last_charged_reads_the_newest_charging_sample() {
        let s = |unix, charging| Sample { unix, percent: 50, charging };
        let log = [s(1000, true), s(2000, true), s(3000, false)];
        assert_eq!(last_charged(&log, 2000 + 125 * 60, false), "2h 5m ago");
        assert_eq!(last_charged(&log, 9999, true), "Now");
        assert_eq!(last_charged(&[s(1, false)], 100, false), "—");
        assert_eq!(last_charged(&[], 100, false), "—");
    }

    #[test]
    fn the_alert_offers_low_power_mode_unless_it_is_on() {
        let ui = crate::shell::tests::ui();
        let a = Alert::new(&ui, Level::Low, 15, false);
        let (x, y) = a.button(&ui, 1200, 1920, Button::LowPowerMode).unwrap();
        assert_eq!(a.hit(&ui, 1200, 1920, x, y), Some(Button::LowPowerMode));
        let (x, y) = a.button(&ui, 1200, 1920, Button::Dismiss).unwrap();
        assert_eq!(a.hit(&ui, 1200, 1920, x, y), Some(Button::Dismiss));
        assert_eq!(a.hit(&ui, 1200, 1920, 5.0, 5.0), None);
        let on = Alert::new(&ui, Level::Critical, 5, true);
        assert!(on.button(&ui, 1200, 1920, Button::LowPowerMode).is_none());
        let (x, y) = on.button(&ui, 1200, 1920, Button::Dismiss).unwrap();
        assert_eq!(on.hit(&ui, 1200, 1920, x, y), Some(Button::Dismiss));
    }
}
