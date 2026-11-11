//! Foundation `TimeZone` (Fundamentals): a zone by identifier from the system tz database (the same TZif files
//! the Date & Time page reads, through `springboard::bare::timezone`), a fixed offset from GMT, or the
//! auto-updating current zone. Instants are `coding::Date` values (seconds since 1970); the calendar is Gregorian.
//!
//! ```ignore
//! let london = TimeZone::new("Europe/London").unwrap();
//! london.seconds_from_gmt(Date(1_782_864_000.0)); // 3600 in summer
//! ```
//!
//! The person's date and time conventions come from the shell (`ToApp::DateTime`, on start and on every change in
//! Settings > Date & Time): the system zone behind [`TimeZone::current`] and the clock style, 24-hour or with AM and
//! PM. An app writes a time with [`time_text`] or [`time_text_at`] and never formats hours and minutes itself, so it
//! reads as the menu bar does. The runtime repaints the app's windows when either changes (`App::configure`, then a
//! whole frame); an app that caches formatted dates keys the cache on [`conventions_revision`].
//!
//! ```ignore
//! time_zone::uses_24_hour_clock();              // Date & Time > 24-Hour Time
//! time_zone::time_text(14, 5);                  // "14:05", or "2:05 PM" with it off
//! time_zone::time_text_at(Date(1_782_864_000.0)); // that instant's time of day in the system zone
//! ```

use crate::coding::Date;
use crate::locale::Locale;
use springboard::bare::timezone::{self as tzdb, Zone};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

const DAY: i64 = 86_400;
/// `TimeZone(secondsFromGMT:)` takes offsets from -18 to +18 hours.
const MAX_FIXED_SECONDS: i32 = 18 * 3600;
/// How far `next_daylight_saving_time_transition` looks ahead, in days (four years covers every rule in the database).
const LOOKAHEAD_DAYS: i64 = 4 * 366;
/// The name the system posts a notification under when its zone changes. Bifrost publishes nothing under this
/// name: the shell sends the new zone to every app (`ToApp::DateTime`), the runtime makes it the zone of
/// `TimeZone::current()` and repaints the app, and [`conventions_revision`] changes.
pub const SYSTEM_TIME_ZONE_DID_CHANGE: &str = "systemTimeZoneDidChange";

/// Date & Time > 24-Hour Time is off, as the shell last said. 24-hour time until it says so (an older shell never does).
static CLOCK_12_HOUR: AtomicBool = AtomicBool::new(false);
/// Counts the changes of the clock style and of the system zone.
static REVISION: AtomicU32 = AtomicU32::new(0);

/// Whether times are written in 24-hour style ("14:05") instead of with AM and PM ("2:05 PM"): Settings > Date & Time
/// > 24-Hour Time, as the shell told this app at start and on every change. True on a shell that does not say (and
/// so in a test, where no shell speaks).
pub fn uses_24_hour_clock() -> bool {
    !CLOCK_12_HOUR.load(Ordering::Relaxed)
}

/// A time of day in the person's clock style, as the menu bar writes it: "02:08", or "2:08 AM" with 24-Hour Time off.
pub fn time_text(hour: i32, minute: i32) -> String {
    springboard::status::clock_in(hour, minute, uses_24_hour_clock())
}

/// The time of day of `date` in the system zone ([`TimeZone::current`]), in the person's clock style.
pub fn time_text_at(date: Date) -> String {
    let local = unix(date) + i64::from(TimeZone::current().seconds_from_gmt(date));
    let secs = local.rem_euclid(DAY);
    time_text((secs / 3600) as i32, (secs % 3600 / 60) as i32)
}

/// Changes whenever the clock style or the system zone does. An app that keeps formatted dates compares it with the
/// value it formatted them at.
pub fn conventions_revision() -> u32 {
    REVISION.load(Ordering::Relaxed)
}

/// Makes the machine's zone (the `/etc/localtime` link, which an app's sandbox can read) the zone of
/// `TimeZone::current()`. The runtime calls it once at start, so the zone is right before the shell's first
/// `ToApp::DateTime` and on a shell that sends none; without it every app told the time in the built-in default zone.
pub(crate) fn adopt_system_zone() {
    if bifrost_core::pin::pinned() {
        return;
    }
    if let Some(name) = tzdb::system_name() {
        tzdb::set(&name);
    }
}

/// Applies the shell's `ToApp::DateTime`: the clock style from `flags` (`proto::date_time` bits) and the system zone
/// by name (an empty or unknown name keeps the zone in use). True when something changed, so the runtime repaints.
pub(crate) fn apply_system(flags: u32, zone: &str) -> bool {
    let twelve = flags & crate::proto::date_time::CLOCK_12_HOUR != 0;
    let mut changed = CLOCK_12_HOUR.swap(twelve, Ordering::Relaxed) != twelve;
    if !zone.is_empty() && zone != tzdb::current_name() {
        changed |= tzdb::set(zone);
    }
    if changed {
        REVISION.fetch_add(1, Ordering::Relaxed);
    }
    changed
}

/// `TimeZone.NameStyle`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameStyle {
    Standard,
    ShortStandard,
    DaylightSaving,
    ShortDaylightSaving,
    Generic,
    ShortGeneric,
}

#[derive(Clone, Debug, PartialEq)]
enum Backend {
    /// a zone from the tz database, by identifier
    Named { identifier: String, zone: Zone },
    /// `TimeZone(secondsFromGMT:)`: a fixed offset that never has daylight saving
    Fixed(i32),
    /// `TimeZone.autoupdatingCurrent`: the system zone, read at every call
    AutoUpdating,
}

/// `TimeZone`: the offset from GMT, the daylight-saving state and the abbreviation at any instant.
#[derive(Clone, Debug, PartialEq)]
pub struct TimeZone {
    backend: Backend,
}

impl TimeZone {
    /// `TimeZone(identifier:)`: None when the tz database has no zone of that name.
    pub fn new(identifier: &str) -> Option<TimeZone> {
        let zone = tzdb::load(identifier)?;
        Some(TimeZone { backend: Backend::Named { identifier: identifier.to_string(), zone } })
    }

    /// Coordinated Universal Time: offset 0, abbreviation "UTC", no daylight saving. Needs no tz database.
    pub fn utc() -> TimeZone {
        TimeZone::from_posix_rule("UTC", "UTC0").expect("UTC0 is a valid rule")
    }

    /// A zone from a POSIX rule such as `EST5EDT,M3.2.0,M11.1.0`, named `identifier`, with no tz database file behind
    /// it. None when the rule does not parse.
    pub fn from_posix_rule(identifier: &str, rule: &str) -> Option<TimeZone> {
        let zone = Zone::from_posix_rule(rule)?;
        Some(TimeZone { backend: Backend::Named { identifier: identifier.to_string(), zone } })
    }

    /// `TimeZone(secondsFromGMT:)`: None outside -18 to +18 hours.
    pub fn from_seconds_from_gmt(seconds: i32) -> Option<TimeZone> {
        (seconds.abs() <= MAX_FIXED_SECONDS).then_some(TimeZone { backend: Backend::Fixed(seconds) })
    }

    /// `TimeZone.current`: the zone the system uses now. GMT when the tz database cannot be read.
    pub fn current() -> TimeZone {
        TimeZone::new(&tzdb::current_name()).unwrap_or(TimeZone { backend: Backend::Fixed(0) })
    }

    /// `TimeZone.autoupdatingCurrent`: the system zone, as it is when each query runs.
    pub fn auto_updating_current() -> TimeZone {
        TimeZone { backend: Backend::AutoUpdating }
    }

    /// `knownTimeZoneIdentifiers`: every zone name in the tz database, sorted.
    pub fn known_time_zone_identifiers() -> Vec<String> {
        tzdb::list_zones()
    }

    /// `identifier`: "Europe/London", "GMT+0100" for a fixed offset, and the system zone's name for the auto-updating one.
    pub fn identifier(&self) -> String {
        match &self.backend {
            Backend::Named { identifier, .. } => identifier.clone(),
            Backend::Fixed(seconds) => fixed_identifier(*seconds),
            Backend::AutoUpdating => tzdb::current_name(),
        }
    }

    /// `secondsFromGMT(for:)`: the offset east of GMT in seconds at `date`.
    pub fn seconds_from_gmt(&self, date: Date) -> i32 {
        self.state(unix(date)).0
    }

    /// `abbreviation(for:)`: the abbreviation in force at `date` ("GMT", "BST"); a fixed offset gives its identifier.
    pub fn abbreviation(&self, date: Date) -> String {
        self.abbreviation_at(unix(date))
    }

    /// The long specific name in force at `date` ("Greenwich Mean Time", "British Summer Time"), English whatever the
    /// locale. None when the abbreviation in force has no entry in the built-in table.
    pub fn long_name(&self, date: Date) -> Option<String> {
        long_name(&self.abbreviation_at(unix(date))).map(str::to_string)
    }

    /// `isDaylightSavingTime(for:)`.
    pub fn is_daylight_saving_time(&self, date: Date) -> bool {
        self.state(unix(date)).1
    }

    /// `daylightSavingTimeOffset(for:)`: the extra seconds in force at `date`, 0 in standard time. The standard
    /// offset is read at the nearest standard-time sample within a year either side (30-minute shifts such as
    /// Lord Howe work); 3600 when the zone has none in that window.
    pub fn daylight_saving_time_offset(&self, date: Date) -> i32 {
        let t = unix(date);
        let (offset, daylight) = self.state(t);
        if !daylight {
            return 0;
        }
        let standard = (1..=14i64)
            .flat_map(|k| [t - k * 30 * DAY, t + k * 30 * DAY])
            .map(|s| self.state(s))
            .find(|&(_, dst)| !dst)
            .map(|(o, _)| o);
        standard.map_or(3600, |s| offset - s)
    }

    /// `nextDaylightSavingTimeTransition(after:)`: the first instant after `date` at which daylight saving starts or
    /// ends. None for a fixed offset, or when no change falls within the next four years.
    pub fn next_daylight_saving_time_transition(&self, after: Date) -> Option<Date> {
        if matches!(self.backend, Backend::Fixed(_)) {
            return None;
        }
        let start = unix(after);
        let was = self.state(start).1;
        let mut low = start;
        for day in 1..=LOOKAHEAD_DAYS {
            let high = start + day * DAY;
            if self.state(high).1 != was {
                // the change lies in (low, high]: bisect down to the second
                let (mut lo, mut hi) = (low, high);
                while hi - lo > 1 {
                    let mid = lo + (hi - lo) / 2;
                    if self.state(mid).1 == was {
                        lo = mid;
                    } else {
                        hi = mid;
                    }
                }
                return Some(Date(hi as f64));
            }
            low = high;
        }
        None
    }

    /// `localizedName(for:locale:)`. Names are English whatever the locale (no other language data is built in).
    /// Standard and daylight names come from the abbreviation a zone uses in that season. None when the zone has
    /// no such season, or when the long form of its abbreviation is not in the built-in table.
    pub fn localized_name(&self, style: NameStyle, _locale: &Locale) -> Option<String> {
        let daylight = matches!(style, NameStyle::DaylightSaving | NameStyle::ShortDaylightSaving);
        let abbreviation = self.abbreviation_in_season(daylight)?;
        match style {
            NameStyle::ShortStandard | NameStyle::ShortDaylightSaving | NameStyle::ShortGeneric => Some(abbreviation),
            NameStyle::Standard | NameStyle::DaylightSaving | NameStyle::Generic => {
                long_name(&abbreviation).map(str::to_string)
            }
        }
    }

    /// (offset east of GMT in seconds, is daylight time) at a Unix time.
    fn state(&self, unix: i64) -> (i32, bool) {
        match &self.backend {
            Backend::Named { zone, .. } => zone.at(unix),
            Backend::Fixed(seconds) => (*seconds, false),
            Backend::AutoUpdating => tzdb::with_current(|_, zone| zone.at(unix)).unwrap_or((0, false)),
        }
    }

    fn abbreviation_at(&self, unix: i64) -> String {
        match &self.backend {
            Backend::Named { zone, .. } => zone.abbreviation(unix),
            Backend::Fixed(seconds) => fixed_identifier(*seconds),
            Backend::AutoUpdating => tzdb::with_current(|_, zone| zone.abbreviation(unix)).unwrap_or_else(|| "GMT".into()),
        }
    }

    /// The abbreviation at the first instant in the next year whose daylight state is `daylight`, sampled every 46 days.
    fn abbreviation_in_season(&self, daylight: bool) -> Option<String> {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
        (0..8i64).map(|k| now + k * 46 * DAY).find(|&t| self.state(t).1 == daylight).map(|t| self.abbreviation_at(t))
    }
}

fn unix(date: Date) -> i64 {
    date.0.floor() as i64
}

/// The identifier of a fixed offset: "GMT" at zero, else "GMT+hhmm" / "GMT-hhmm".
fn fixed_identifier(seconds: i32) -> String {
    if seconds == 0 {
        return "GMT".into();
    }
    let (sign, a) = if seconds < 0 { ('-', -seconds) } else { ('+', seconds) };
    format!("GMT{sign}{:02}{:02}", a / 3600, a / 60 % 60)
}

/// English long names for the abbreviations the built-in zones use.
fn long_name(abbreviation: &str) -> Option<&'static str> {
    Some(match abbreviation {
        "UTC" => "Coordinated Universal Time",
        "GMT" => "Greenwich Mean Time",
        "WET" => "Western European Standard Time",
        "WEST" => "Western European Summer Time",
        "BST" => "British Summer Time",
        "CET" => "Central European Standard Time",
        "CEST" => "Central European Summer Time",
        "EET" => "Eastern European Standard Time",
        "EEST" => "Eastern European Summer Time",
        "MSK" => "Moscow Standard Time",
        "EST" => "Eastern Standard Time",
        "EDT" => "Eastern Daylight Time",
        "CST" => "Central Standard Time",
        "CDT" => "Central Daylight Time",
        "MST" => "Mountain Standard Time",
        "MDT" => "Mountain Daylight Time",
        "PST" => "Pacific Standard Time",
        "PDT" => "Pacific Daylight Time",
        "AKST" => "Alaska Standard Time",
        "AKDT" => "Alaska Daylight Time",
        "HST" => "Hawaii-Aleutian Standard Time",
        "JST" => "Japan Standard Time",
        "KST" => "Korean Standard Time",
        "AWST" => "Australian Western Standard Time",
        "AEST" => "Australian Eastern Standard Time",
        "AEDT" => "Australian Eastern Daylight Time",
        "NZST" => "New Zealand Standard Time",
        "NZDT" => "New Zealand Daylight Time",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use springboard::bare::timezone::{current_name, days_from_civil};

    /// The instant `hour:minute` UTC on a civil day.
    fn utc(y: i64, m: u32, d: u32, hour: i64, minute: i64) -> Date {
        Date((days_from_civil(y, m, d) * DAY + hour * 3600 + minute * 60) as f64)
    }

    /// The nine cities a fresh World Clock shows, as tz database identifiers.
    const DEFAULT_CITY_IDENTIFIERS: [&str; 9] = [
        "Atlantic/Reykjavik",
        "Europe/London",
        "Europe/Paris",
        "Europe/Moscow",
        "Asia/Dubai",
        "America/New_York",
        "America/Los_Angeles",
        "Asia/Tokyo",
        "Australia/Sydney",
    ];

    fn zone(name: &str) -> TimeZone {
        TimeZone::new(name).expect("the tz database has the zone")
    }

    #[test]
    fn identifier_round_trips_and_unknown_names_are_none() {
        for name in DEFAULT_CITY_IDENTIFIERS {
            assert_eq!(zone(name).identifier(), name);
        }
        assert_eq!(TimeZone::new("Mars/Olympus_Mons"), None);
        assert_eq!(TimeZone::new("../etc/localtime"), None, "names cannot leave the zone tree");
    }

    #[test]
    fn seconds_from_gmt_differs_in_winter_and_summer() {
        let london = zone("Europe/London");
        assert_eq!(london.seconds_from_gmt(utc(2026, 1, 15, 12, 0)), 0);
        assert_eq!(london.seconds_from_gmt(utc(2026, 7, 1, 12, 0)), 3600);
        let ny = zone("America/New_York");
        assert_eq!(ny.seconds_from_gmt(utc(2026, 1, 15, 12, 0)), -5 * 3600);
        assert_eq!(ny.seconds_from_gmt(utc(2026, 7, 1, 12, 0)), -4 * 3600);
        assert_eq!(zone("Atlantic/Reykjavik").seconds_from_gmt(utc(2026, 7, 1, 12, 0)), 0, "Iceland never shifts");
    }

    #[test]
    fn daylight_saving_switches_at_the_documented_instant() {
        // EU rule: the last Sunday of March (29 March 2026) at 01:00 UTC
        let london = zone("Europe/London");
        assert_eq!(london.seconds_from_gmt(utc(2026, 3, 29, 0, 59)), 0);
        assert_eq!(london.seconds_from_gmt(utc(2026, 3, 29, 1, 0)), 3600);
        assert!(!london.is_daylight_saving_time(utc(2026, 3, 29, 0, 59)));
        assert!(london.is_daylight_saving_time(utc(2026, 3, 29, 1, 0)));
    }

    #[test]
    fn abbreviation_names_the_season() {
        assert_eq!(zone("Europe/London").abbreviation(utc(2026, 1, 15, 12, 0)), "GMT");
        assert_eq!(zone("Europe/London").abbreviation(utc(2026, 7, 1, 12, 0)), "BST");
        assert_eq!(zone("America/New_York").abbreviation(utc(2026, 1, 15, 12, 0)), "EST");
        assert_eq!(zone("America/New_York").abbreviation(utc(2026, 7, 1, 12, 0)), "EDT");
    }

    #[test]
    fn is_daylight_saving_time_is_false_where_there_is_none() {
        let london = zone("Europe/London");
        assert!(london.is_daylight_saving_time(utc(2026, 7, 1, 12, 0)));
        assert!(!london.is_daylight_saving_time(utc(2026, 1, 15, 12, 0)));
        assert!(!zone("Asia/Dubai").is_daylight_saving_time(utc(2026, 7, 1, 12, 0)));
    }

    #[test]
    fn next_transition_is_the_documented_instant() {
        let london = zone("Europe/London");
        // summer time starts 29 March 2026 01:00 UTC and ends 25 October 2026 01:00 UTC
        assert_eq!(london.next_daylight_saving_time_transition(utc(2026, 3, 1, 0, 0)), Some(utc(2026, 3, 29, 1, 0)));
        assert_eq!(london.next_daylight_saving_time_transition(utc(2026, 3, 29, 1, 0)), Some(utc(2026, 10, 25, 1, 0)));
        assert_eq!(zone("Atlantic/Reykjavik").next_daylight_saving_time_transition(utc(2026, 1, 1, 0, 0)), None);
        let fixed = TimeZone::from_seconds_from_gmt(3600).expect("in range");
        assert_eq!(fixed.next_daylight_saving_time_transition(utc(2026, 1, 1, 0, 0)), None);
    }

    #[test]
    fn known_identifiers_are_sorted_and_cover_the_default_cities() {
        let known = TimeZone::known_time_zone_identifiers();
        let mut sorted = known.clone();
        sorted.sort();
        assert_eq!(known, sorted);
        for name in DEFAULT_CITY_IDENTIFIERS {
            assert!(known.iter().any(|k| k == name), "{name} is missing");
        }
    }

    #[test]
    fn fixed_offsets_have_a_gmt_identifier_and_no_daylight_saving() {
        let plus_one = TimeZone::from_seconds_from_gmt(3600).expect("in range");
        assert_eq!(plus_one.identifier(), "GMT+0100");
        assert_eq!(plus_one.seconds_from_gmt(utc(2026, 7, 1, 0, 0)), 3600);
        assert!(!plus_one.is_daylight_saving_time(utc(2026, 7, 1, 0, 0)));
        assert_eq!(TimeZone::from_seconds_from_gmt(-12_600).expect("in range").identifier(), "GMT-0330");
        assert_eq!(TimeZone::from_seconds_from_gmt(0).expect("in range").identifier(), "GMT");
        assert_eq!(TimeZone::from_seconds_from_gmt(18 * 3600 + 1), None);
    }

    #[test]
    fn current_and_auto_updating_current_follow_the_system_zone() {
        assert_eq!(TimeZone::current().identifier(), current_name());
        let auto = TimeZone::auto_updating_current();
        assert_eq!(auto.identifier(), current_name());
        let instant = utc(2026, 7, 1, 12, 0);
        assert_eq!(auto.seconds_from_gmt(instant), TimeZone::current().seconds_from_gmt(instant));
    }

    #[test]
    fn localized_names_are_english_and_follow_the_season() {
        let en = Locale::new("en_US");
        let london = zone("Europe/London");
        assert_eq!(london.localized_name(NameStyle::ShortStandard, &en), Some("GMT".into()));
        assert_eq!(london.localized_name(NameStyle::Standard, &en), Some("Greenwich Mean Time".into()));
        assert_eq!(london.localized_name(NameStyle::ShortDaylightSaving, &en), Some("BST".into()));
        assert_eq!(london.localized_name(NameStyle::DaylightSaving, &en), Some("British Summer Time".into()));
        assert_eq!(zone("Asia/Dubai").localized_name(NameStyle::DaylightSaving, &en), None, "no summer season");
        let plus_one = TimeZone::from_seconds_from_gmt(3600).expect("in range");
        assert_eq!(plus_one.localized_name(NameStyle::Standard, &en), None, "\"GMT+0100\" has no long name");
    }

    #[test]
    fn a_posix_rule_zone_switches_at_the_right_instants() {
        let ny = TimeZone::from_posix_rule("America/New_York", "EST5EDT,M3.2.0,M11.1.0").expect("valid rule");
        let spring = utc(2026, 3, 8, 7, 0);
        assert_eq!(ny.seconds_from_gmt(Date(spring.0 - 1.0)), -18_000);
        assert_eq!(ny.seconds_from_gmt(spring), -14_400);
        let fall = utc(2026, 11, 1, 6, 0);
        assert_eq!(ny.abbreviation(Date(fall.0 - 1.0)), "EDT");
        assert_eq!(ny.abbreviation(fall), "EST");
        assert_eq!(ny.next_daylight_saving_time_transition(utc(2026, 1, 1, 0, 0)), Some(spring));
        assert_eq!(ny.next_daylight_saving_time_transition(spring), Some(fall));
        assert_eq!(ny.identifier(), "America/New_York");
        assert_eq!(TimeZone::from_posix_rule("X", "EST5EDT,J60,J300"), None);
    }

    /// The clock style arrives from the shell; the zone is left alone here (an empty name keeps it), so the tests
    /// that read the current zone are not disturbed.
    #[test]
    fn the_clock_style_follows_what_the_shell_said() {
        use crate::proto::date_time::CLOCK_12_HOUR as TWELVE;
        assert!(uses_24_hour_clock(), "24-hour time until the shell says otherwise");
        assert_eq!(time_text(14, 5), "14:05");
        let before = conventions_revision();
        assert!(apply_system(TWELVE, ""), "the style changed");
        assert!(!uses_24_hour_clock());
        assert_eq!((time_text(14, 5), time_text(0, 8), time_text(12, 0)), ("2:05 PM".into(), "12:08 AM".into(), "12:00 PM".into()));
        assert_eq!(conventions_revision(), before + 1);
        assert!(!apply_system(TWELVE | 0x8000_0000, ""), "said again, with a bit this SDK does not know: no change");
        assert!(!apply_system(TWELVE, "Mars/Olympus_Mons"), "an unknown zone keeps the one in use");
        assert_eq!(conventions_revision(), before + 1);
        // an instant's time of day is read in the system zone
        let noon = utc(2026, 7, 1, 12, 0);
        let local = 12 * 3600 + i64::from(TimeZone::current().seconds_from_gmt(noon));
        assert_eq!(time_text_at(noon), time_text((local.rem_euclid(DAY) / 3600) as i32, (local.rem_euclid(DAY) % 3600 / 60) as i32));
        assert!(apply_system(0, ""), "24-Hour Time turned on again");
        assert_eq!(time_text(14, 5), "14:05");
    }

    #[test]
    fn utc_has_no_offset_or_transitions() {
        let u = TimeZone::utc();
        assert_eq!((u.identifier(), u.seconds_from_gmt(Date(123.0)), u.abbreviation(Date(0.0))), ("UTC".to_string(), 0, "UTC".to_string()));
        assert_eq!(u.next_daylight_saving_time_transition(Date(0.0)), None);
    }

    #[test]
    fn daylight_saving_offset_is_the_extra_seconds() {
        let ny = zone("America/New_York");
        assert_eq!(ny.daylight_saving_time_offset(utc(2026, 7, 1, 12, 0)), 3600);
        assert_eq!(ny.daylight_saving_time_offset(utc(2026, 1, 1, 12, 0)), 0);
        let lord_howe = zone("Australia/Lord_Howe");
        assert_eq!(lord_howe.daylight_saving_time_offset(utc(2026, 1, 15, 0, 0)), 1800);
        assert_eq!(lord_howe.daylight_saving_time_offset(utc(2026, 7, 15, 0, 0)), 0);
    }
}
