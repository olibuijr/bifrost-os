//! The desktop's own settings, kept beside the events file as `key=value` lines (`calendar-prefs.txt`).
//! Pure: reading and writing the text is separate from touching the disk, so the rules are host-testable.
use std::path::{Path, PathBuf};

/// The default height of one hour in the Day and Week grids (points), and the zoom range and step.
pub const HOUR_H_DEFAULT: f32 = 48.0;
pub const HOUR_H_MIN: f32 = 24.0;
pub const HOUR_H_MAX: f32 = 144.0;
pub const HOUR_H_STEP: f32 = 12.0;
/// The lengths a new timed event can default to (minutes), and the hours the day view can open at.
pub const DURATIONS: [u16; 4] = [30, 60, 90, 120];
pub const DAY_STARTS: [u8; 5] = [6, 7, 8, 9, 10];
/// Number of default-alert choices: the indexes of `store::ALERTS`.
pub const ALERT_CHOICES: u8 = 7;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Prefs {
    /// Month view: a leading column with ISO week numbers.
    pub week_numbers: bool,
    /// Day and Week views: points per hour (View > Zoom In / Zoom Out).
    pub hour_h: f32,
    /// The week starts on Monday instead of Sunday.
    pub monday_first: bool,
    /// Minutes a new timed event lasts.
    pub duration: u16,
    /// The hour the Day and Week views scroll to when they open.
    pub day_start: u8,
    /// `CalendarEvent::alert` given to a new timed event, and to a new all-day event.
    pub alert: u8,
    pub alert_all_day: u8,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs { week_numbers: false, hour_h: HOUR_H_DEFAULT, monday_first: false, duration: 60, day_start: 8, alert: 0, alert_all_day: 0 }
    }
}

/// The settings file beside the events file.
pub fn path(events: &Path) -> PathBuf {
    events.with_file_name("calendar-prefs.txt")
}

/// The hour height one zoom step away, kept inside the range.
pub fn zoom(hour_h: f32, dir: i32) -> f32 {
    (hour_h + HOUR_H_STEP * dir as f32).clamp(HOUR_H_MIN, HOUR_H_MAX)
}

/// The value after `current` in `choices`, wrapping; the first one when `current` is not listed.
pub fn cycle<T: Copy + PartialEq>(choices: &[T], current: T) -> T {
    match choices.iter().position(|c| *c == current) {
        Some(i) => choices[(i + 1) % choices.len()],
        None => choices[0],
    }
}

/// One choice in the Settings menu. Each has a menu id (`id`, `from_id`) so the app can tell which one was picked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Setting {
    /// Start the week on Monday (a toggle).
    MondayFirst,
    /// Index into `DURATIONS`.
    Length(usize),
    /// Index into `DAY_STARTS`.
    DayStart(usize),
    /// A `CalendarEvent::alert` for new timed events.
    Alert(u8),
    /// A `CalendarEvent::alert` for new all-day events.
    AllDayAlert(u8),
}

/// Menu ids of the settings start here, clear of the app's `Command` ids.
const ID_BASE: u32 = 100;
const ID_SPAN: u32 = 10;

impl Setting {
    pub fn id(self) -> u32 {
        match self {
            Setting::MondayFirst => ID_BASE,
            Setting::Length(i) => ID_BASE + ID_SPAN + i as u32,
            Setting::DayStart(i) => ID_BASE + 2 * ID_SPAN + i as u32,
            Setting::Alert(v) => ID_BASE + 3 * ID_SPAN + v as u32,
            Setting::AllDayAlert(v) => ID_BASE + 4 * ID_SPAN + v as u32,
        }
    }

    pub fn from_id(id: u32) -> Option<Setting> {
        let n = id.checked_sub(ID_BASE)?;
        let (group, i) = ((n / ID_SPAN) as usize, (n % ID_SPAN) as usize);
        match group {
            0 if i == 0 => Some(Setting::MondayFirst),
            1 if i < DURATIONS.len() => Some(Setting::Length(i)),
            2 if i < DAY_STARTS.len() => Some(Setting::DayStart(i)),
            3 if i < ALERT_CHOICES as usize => Some(Setting::Alert(i as u8)),
            4 if i < ALERT_CHOICES as usize => Some(Setting::AllDayAlert(i as u8)),
            _ => None,
        }
    }

    /// The menu title of this choice (the alert choices use the event editor's wording).
    pub fn title(self) -> String {
        match self {
            Setting::MondayFirst => "Start Week on Monday".into(),
            Setting::Length(i) => match DURATIONS[i] {
                60 => "1 Hour".into(),
                m if m % 60 == 0 => format!("{} Hours", m / 60),
                m => format!("{m} Minutes"),
            },
            Setting::DayStart(i) => format!("{} AM", DAY_STARTS[i]),
            Setting::Alert(v) | Setting::AllDayAlert(v) => super::store::ALERTS[v as usize].to_string(),
        }
    }
}

impl Prefs {
    /// Reads `key=value` lines. Unknown keys and unreadable values are ignored; numbers are clamped to what the app
    /// can show, so a hand-edited file cannot break the layout.
    pub fn parse(text: &str) -> Prefs {
        let mut p = Prefs::default();
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else { continue };
            let (key, value) = (key.trim(), value.trim());
            match key {
                "week_numbers" => p.week_numbers = value == "1",
                "monday_first" => p.monday_first = value == "1",
                "hour_h" => {
                    if let Ok(v) = value.parse::<f32>() {
                        if v.is_finite() {
                            p.hour_h = v.clamp(HOUR_H_MIN, HOUR_H_MAX);
                        }
                    }
                }
                "duration" => {
                    if let Ok(v) = value.parse::<u16>() {
                        if DURATIONS.contains(&v) {
                            p.duration = v;
                        }
                    }
                }
                "day_start" => {
                    if let Ok(v) = value.parse::<u8>() {
                        if DAY_STARTS.contains(&v) {
                            p.day_start = v;
                        }
                    }
                }
                "alert" => {
                    if let Some(v) = value.parse::<u8>().ok().filter(|v| *v < ALERT_CHOICES) {
                        p.alert = v;
                    }
                }
                "alert_all_day" => {
                    if let Some(v) = value.parse::<u8>().ok().filter(|v| *v < ALERT_CHOICES) {
                        p.alert_all_day = v;
                    }
                }
                _ => {}
            }
        }
        p
    }

    pub fn to_text(&self) -> String {
        format!(
            "week_numbers={}\nmonday_first={}\nhour_h={}\nduration={}\nday_start={}\nalert={}\nalert_all_day={}\n",
            u8::from(self.week_numbers),
            u8::from(self.monday_first),
            self.hour_h,
            self.duration,
            self.day_start,
            self.alert,
            self.alert_all_day
        )
    }

    /// The settings saved beside `events`; the defaults when there is no file.
    pub fn load(events: &Path) -> Prefs {
        std::fs::read_to_string(path(events)).map(|t| Prefs::parse(&t)).unwrap_or_default()
    }

    pub fn save(&self, events: &Path) -> std::io::Result<()> {
        springboard::shell::fsutil::write_atomic(&path(events), &self.to_text())
    }

    /// Whether `setting` is the current choice (its check mark).
    pub fn is(&self, setting: Setting) -> bool {
        match setting {
            Setting::MondayFirst => self.monday_first,
            Setting::Length(i) => self.duration == DURATIONS[i],
            Setting::DayStart(i) => self.day_start == DAY_STARTS[i],
            Setting::Alert(v) => self.alert == v,
            Setting::AllDayAlert(v) => self.alert_all_day == v,
        }
    }

    /// Picks `setting`; the week-start toggle flips.
    pub fn apply(&mut self, setting: Setting) {
        match setting {
            Setting::MondayFirst => self.monday_first = !self.monday_first,
            Setting::Length(i) => self.duration = DURATIONS[i],
            Setting::DayStart(i) => self.day_start = DAY_STARTS[i],
            Setting::Alert(v) => self.alert = v,
            Setting::AllDayAlert(v) => self.alert_all_day = v,
        }
    }

    /// The weekday column (0 = first day of the week) of a weekday number where 0 is Sunday.
    pub fn column(&self, weekday: usize) -> usize {
        (weekday + if self.monday_first { 6 } else { 0 }) % 7
    }

    /// The weekday number (0 = Sunday) shown in column `col`.
    pub fn weekday_at(&self, col: usize) -> usize {
        (col + if self.monday_first { 1 } else { 0 }) % 7
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_old_week_numbers_file_still_reads() {
        let p = Prefs::parse("week_numbers=1\n");
        assert!(p.week_numbers);
        assert_eq!(p, Prefs { week_numbers: true, ..Prefs::default() });
    }

    #[test]
    fn settings_round_trip() {
        let p = Prefs { week_numbers: true, hour_h: 72.0, monday_first: true, duration: 90, day_start: 7, alert: 3, alert_all_day: 6 };
        assert_eq!(Prefs::parse(&p.to_text()), p);
    }

    #[test]
    fn odd_values_fall_back_or_clamp() {
        let p = Prefs::parse("hour_h=9999\nduration=45\nday_start=3\nalert=9\nalert_all_day=x\nnonsense\nunknown=1\n");
        assert_eq!(p.hour_h, HOUR_H_MAX);
        assert_eq!(p.duration, 60);
        assert_eq!(p.day_start, 8);
        assert_eq!((p.alert, p.alert_all_day), (0, 0));
        assert_eq!(Prefs::parse("hour_h=NaN").hour_h, HOUR_H_DEFAULT);
        assert_eq!(Prefs::parse("hour_h=1").hour_h, HOUR_H_MIN);
    }

    #[test]
    fn zoom_steps_stay_in_range() {
        assert_eq!(zoom(48.0, 1), 60.0);
        assert_eq!(zoom(48.0, -1), 36.0);
        assert_eq!(zoom(HOUR_H_MAX, 1), HOUR_H_MAX);
        assert_eq!(zoom(HOUR_H_MIN, -1), HOUR_H_MIN);
    }

    #[test]
    fn cycling_wraps_and_recovers_from_unlisted_values() {
        assert_eq!(cycle(&DURATIONS, 30), 60);
        assert_eq!(cycle(&DURATIONS, 120), 30);
        assert_eq!(cycle(&DURATIONS, 45), 30);
    }

    #[test]
    fn monday_first_rotates_the_columns() {
        let sunday = Prefs::default();
        assert_eq!((sunday.column(0), sunday.column(6), sunday.weekday_at(0)), (0, 6, 0));
        let monday = Prefs { monday_first: true, ..Prefs::default() };
        // Monday is column 0, Sunday is column 6
        assert_eq!((monday.column(1), monday.column(0), monday.column(6)), (0, 6, 5));
        assert_eq!((monday.weekday_at(0), monday.weekday_at(6)), (1, 0));
        for col in 0..7 {
            assert_eq!(monday.column(monday.weekday_at(col)), col);
        }
    }

    #[test]
    fn the_file_sits_beside_the_events() {
        assert_eq!(path(Path::new("/data/events.tsv")), PathBuf::from("/data/calendar-prefs.txt"));
    }

    #[test]
    fn every_setting_has_its_own_menu_id_that_reads_back() {
        let mut all = vec![Setting::MondayFirst];
        all.extend((0..DURATIONS.len()).map(Setting::Length));
        all.extend((0..DAY_STARTS.len()).map(Setting::DayStart));
        all.extend((0..ALERT_CHOICES).map(Setting::Alert));
        all.extend((0..ALERT_CHOICES).map(Setting::AllDayAlert));
        let mut ids: Vec<u32> = all.iter().map(|s| s.id()).collect();
        for s in &all {
            assert_eq!(Setting::from_id(s.id()), Some(*s));
        }
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), all.len());
        assert_eq!(Setting::from_id(1), None);
        assert_eq!(Setting::from_id(ID_BASE + 5), None);
        assert_eq!(Setting::from_id(ID_BASE + ID_SPAN + DURATIONS.len() as u32), None);
    }

    #[test]
    fn picking_a_setting_changes_it_and_checks_only_that_choice() {
        let mut p = Prefs::default();
        assert!(p.is(Setting::Length(1)) && !p.is(Setting::Length(0)));
        p.apply(Setting::Length(2));
        assert_eq!(p.duration, 90);
        assert!(p.is(Setting::Length(2)) && !p.is(Setting::Length(1)));
        p.apply(Setting::DayStart(0));
        assert_eq!(p.day_start, 6);
        p.apply(Setting::Alert(3));
        p.apply(Setting::AllDayAlert(6));
        assert_eq!((p.alert, p.alert_all_day), (3, 6));
        assert!(!p.monday_first);
        p.apply(Setting::MondayFirst);
        assert!(p.monday_first && p.is(Setting::MondayFirst));
        p.apply(Setting::MondayFirst);
        assert!(!p.monday_first);
    }

    #[test]
    fn titles_read_naturally() {
        let titles: Vec<String> = (0..DURATIONS.len()).map(|i| Setting::Length(i).title()).collect();
        assert_eq!(titles, vec!["30 Minutes", "1 Hour", "90 Minutes", "2 Hours"]);
        assert_eq!(Setting::DayStart(2).title(), "8 AM");
        assert_eq!(Setting::Alert(0).title(), "None");
        assert_eq!(Setting::AllDayAlert(6).title(), "1 day before");
    }
}
