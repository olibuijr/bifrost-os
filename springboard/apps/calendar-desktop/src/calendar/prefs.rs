//! The calendar's settings, kept in `calendar-prefs.txt` beside the events file as `key=value` lines.
//! Unknown keys and out-of-range values are ignored so an older or damaged file never blocks the app.
use std::path::{Path, PathBuf};

/// The choices for a new event's length, in minutes (Settings > Default duration).
pub const DURATIONS: [u16; 6] = [15, 30, 45, 60, 90, 120];

/// The alert choices are the store's `ALERTS` presets, by index.
const ALERT_COUNT: u8 = 7;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Prefs {
    /// Month grid: a leading column with ISO week numbers.
    pub week_numbers: bool,
    /// The first day of the week, 0 = Sunday .. 6 = Saturday.
    pub week_start: u8,
    /// The hour the Day and Week views scroll to when opened, 0..=12.
    pub day_start: u8,
    /// The length of a new event in minutes (one of `DURATIONS`).
    pub duration: u16,
    /// Alert preset (index into `store::ALERTS`) for a new timed event.
    pub alert: u8,
    /// Alert preset for a new all-day event.
    pub alert_all_day: u8,
    /// The hour row height in points of the Day and Week views.
    pub hour_h: f32,
}

impl Default for Prefs {
    fn default() -> Self {
        Self { week_numbers: false, week_start: 0, day_start: 8, duration: 60, alert: 0, alert_all_day: 0, hour_h: super::timeview::HOUR_H }
    }
}

impl Prefs {
    /// Reads the settings from `text`; every key that is missing or invalid keeps its default.
    pub fn parse(text: &str) -> Prefs {
        let mut p = Prefs::default();
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else { continue };
            let (key, value) = (key.trim(), value.trim());
            match key {
                "week_numbers" => p.week_numbers = value == "1",
                "week_start" => {
                    if let Some(v) = value.parse::<u8>().ok().filter(|v| *v < 7) {
                        p.week_start = v;
                    }
                }
                "day_start" => {
                    if let Some(v) = value.parse::<u8>().ok().filter(|v| *v <= 12) {
                        p.day_start = v;
                    }
                }
                "duration" => {
                    if let Some(v) = value.parse::<u16>().ok().filter(|v| DURATIONS.contains(v)) {
                        p.duration = v;
                    }
                }
                "alert" => {
                    if let Some(v) = value.parse::<u8>().ok().filter(|v| *v < ALERT_COUNT) {
                        p.alert = v;
                    }
                }
                "alert_all_day" => {
                    if let Some(v) = value.parse::<u8>().ok().filter(|v| *v < ALERT_COUNT) {
                        p.alert_all_day = v;
                    }
                }
                "hour_h" => {
                    if let Some(v) = value.parse::<f32>().ok().filter(|v| v.is_finite()) {
                        p.hour_h = super::timeview::zoomed(v, 0);
                    }
                }
                _ => {}
            }
        }
        p
    }

    pub fn to_text(&self) -> String {
        format!(
            "week_numbers={}\nweek_start={}\nday_start={}\nduration={}\nalert={}\nalert_all_day={}\nhour_h={}\n",
            u8::from(self.week_numbers),
            self.week_start,
            self.day_start,
            self.duration,
            self.alert,
            self.alert_all_day,
            self.hour_h
        )
    }

    /// Steps `week_start` by `n` days, wrapping around the week.
    pub fn step_week_start(&mut self, n: i32) {
        self.week_start = (self.week_start as i32 + n).rem_euclid(7) as u8;
    }

    /// Steps `day_start` by `n` hours, staying within 0..=12.
    pub fn step_day_start(&mut self, n: i32) {
        self.day_start = (self.day_start as i32 + n).clamp(0, 12) as u8;
    }

    /// Steps `duration` through `DURATIONS`, stopping at the ends.
    pub fn step_duration(&mut self, n: i32) {
        let at = DURATIONS.iter().position(|d| *d == self.duration).unwrap_or(3) as i32;
        self.duration = DURATIONS[(at + n).clamp(0, DURATIONS.len() as i32 - 1) as usize];
    }

    /// Steps an alert preset by `n`, wrapping around the list.
    pub fn step_alert(alert: u8, n: i32) -> u8 {
        (alert as i32 + n).rem_euclid(ALERT_COUNT as i32) as u8
    }
}

pub fn path(events: &Path) -> PathBuf {
    events.with_file_name("calendar-prefs.txt")
}

pub fn load(events: &Path) -> Prefs {
    std::fs::read_to_string(path(events)).map_or_else(|_| Prefs::default(), |text| Prefs::parse(&text))
}

pub fn save(events: &Path, prefs: &Prefs) -> std::io::Result<()> {
    springboard::shell::fsutil::write_atomic(&path(events), &prefs.to_text())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_when_the_file_is_empty_or_unknown() {
        assert_eq!(Prefs::parse(""), Prefs::default());
        assert_eq!(Prefs::parse("colour=blue\nnot a line"), Prefs::default());
    }

    #[test]
    fn round_trips_through_text() {
        let p = Prefs { week_numbers: true, week_start: 1, day_start: 6, duration: 90, alert: 3, alert_all_day: 6, hour_h: 72.0 };
        assert_eq!(Prefs::parse(&p.to_text()), p);
    }

    #[test]
    fn reads_the_old_week_numbers_only_file() {
        let p = Prefs::parse("week_numbers=1\n");
        assert!(p.week_numbers);
        assert_eq!(p.week_start, 0);
        assert_eq!(p.duration, 60);
    }

    #[test]
    fn out_of_range_values_keep_the_default() {
        let p = Prefs::parse("week_start=9\nday_start=40\nduration=47\nalert=12\nalert_all_day=x\nhour_h=nan");
        assert_eq!(p, Prefs::default());
    }

    #[test]
    fn hour_height_is_clamped_to_the_zoom_limits() {
        assert_eq!(Prefs::parse("hour_h=500").hour_h, 96.0);
        assert_eq!(Prefs::parse("hour_h=2").hour_h, 24.0);
    }

    #[test]
    fn steps_wrap_or_stop_where_they_should() {
        let mut p = Prefs::default();
        p.step_week_start(-1);
        assert_eq!(p.week_start, 6);
        p.step_week_start(1);
        assert_eq!(p.week_start, 0);
        p.step_day_start(-20);
        assert_eq!(p.day_start, 0);
        p.step_day_start(20);
        assert_eq!(p.day_start, 12);
        p.step_duration(-10);
        assert_eq!(p.duration, 15);
        p.step_duration(10);
        assert_eq!(p.duration, 120);
        assert_eq!(Prefs::step_alert(0, -1), 6);
        assert_eq!(Prefs::step_alert(6, 1), 0);
    }

    #[test]
    fn saves_and_loads_beside_the_events_file() {
        let dir = std::env::temp_dir().join(format!("springboard-calendar-prefs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let events = dir.join("events.tsv");
        let p = Prefs { week_start: 1, ..Prefs::default() };
        save(&events, &p).unwrap();
        assert_eq!(load(&events), p);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
