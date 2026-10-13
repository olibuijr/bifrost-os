//! The pure model behind `UNUserNotificationCenter`: authorization (`UNAuthorizationStatus`, `UNAuthorizationOptions`,
//! `UNNotificationSettings`), triggers (`UNCalendarNotificationTrigger` with `DateComponents`,
//! `UNTimeIntervalNotificationTrigger`), foreground presentation (`UNNotificationPresentationOptions`), sounds
//! (`UNNotificationSound`) and attachments (`UNNotificationAttachment`). No I/O: the shell keeps the state and calls in.
//!
//! Gaps, said plainly: remote push (device tokens, background push) and the notification service extension are not built;
//! a `DateComponents` second left out means second 0, not "every second".

use std::collections::BTreeMap;

/// `UNAuthorizationStatus`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthorizationStatus {
    NotDetermined,
    Denied,
    Authorized,
    Provisional,
    Ephemeral,
}

impl AuthorizationStatus {
    fn code(self) -> u8 {
        self as u8
    }
    fn from_code(c: u8) -> Option<Self> {
        Some(match c {
            0 => Self::NotDetermined,
            1 => Self::Denied,
            2 => Self::Authorized,
            3 => Self::Provisional,
            4 => Self::Ephemeral,
            _ => return None,
        })
    }
    /// Whether the app may post at all.
    pub fn allows_posting(self) -> bool {
        matches!(self, Self::Authorized | Self::Provisional | Self::Ephemeral)
    }
}

/// `UNAuthorizationOptions` (an option set).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AuthorizationOptions(pub u8);

impl AuthorizationOptions {
    pub const BADGE: Self = Self(1);
    pub const SOUND: Self = Self(2);
    pub const ALERT: Self = Self(4);
    pub const CRITICAL_ALERT: Self = Self(16);
    pub const PROVISIONAL: Self = Self(64);
    pub const fn union(self, o: Self) -> Self {
        Self(self.0 | o.0)
    }
    pub const fn contains(self, o: Self) -> bool {
        self.0 & o.0 == o.0
    }
}

/// `UNNotificationSetting`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Setting {
    NotSupported,
    Disabled,
    Enabled,
}

/// `UNNotificationSettings`: a snapshot, what `getNotificationSettings` answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    pub authorization_status: AuthorizationStatus,
    pub alert: Setting,
    pub sound: Setting,
    pub badge: Setting,
    pub critical_alert: Setting,
}

/// The user's answer to the authorization prompt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Deny,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Entry {
    status: AuthorizationStatus,
    options: AuthorizationOptions,
}

/// The authorization state of every app, persisted as text lines `app<TAB>status<TAB>options`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AuthorizationStore {
    apps: BTreeMap<String, Entry>,
}

impl AuthorizationStore {
    pub fn status(&self, app: &str) -> AuthorizationStatus {
        self.apps.get(app).map_or(AuthorizationStatus::NotDetermined, |e| e.status)
    }

    /// `requestAuthorization(options:)`. An app that was asked before is not asked again (the answer stands); a
    /// provisional request is granted without a prompt; otherwise `decision` is the user's answer to the prompt.
    /// Returns whether the app is granted.
    pub fn request(&mut self, app: &str, options: AuthorizationOptions, decision: Decision) -> bool {
        let current = self.status(app);
        if current != AuthorizationStatus::NotDetermined {
            return current.allows_posting();
        }
        let (status, granted) = if options.contains(AuthorizationOptions::PROVISIONAL) {
            (AuthorizationStatus::Provisional, options)
        } else if decision == Decision::Allow {
            (AuthorizationStatus::Authorized, options)
        } else {
            (AuthorizationStatus::Denied, AuthorizationOptions::default())
        };
        self.apps.insert(app.to_string(), Entry { status, options: granted });
        status.allows_posting()
    }

    /// `getNotificationSettings`. The alert, sound and badge settings are what was granted; critical alerts are
    /// `NotSupported` unless asked for.
    pub fn settings(&self, app: &str) -> Settings {
        let e = self.apps.get(app).copied().unwrap_or(Entry { status: AuthorizationStatus::NotDetermined, options: AuthorizationOptions::default() });
        let pick = |flag: AuthorizationOptions| match (e.status, e.options.contains(flag)) {
            (AuthorizationStatus::NotDetermined, _) => Setting::NotSupported,
            (_, true) => Setting::Enabled,
            _ => Setting::Disabled,
        };
        let critical = if e.options.contains(AuthorizationOptions::CRITICAL_ALERT) { Setting::Enabled } else { Setting::NotSupported };
        Settings { authorization_status: e.status, alert: pick(AuthorizationOptions::ALERT), sound: pick(AuthorizationOptions::SOUND), badge: pick(AuthorizationOptions::BADGE), critical_alert: critical }
    }

    pub fn to_text(&self) -> String {
        self.apps.iter().map(|(a, e)| format!("{a}\t{}\t{}\n", e.status.code(), e.options.0)).collect()
    }

    /// Reads [`to_text`](Self::to_text); malformed lines are skipped.
    pub fn from_text(text: &str) -> Self {
        let mut apps = BTreeMap::new();
        for line in text.lines() {
            let mut f = line.split('\t');
            let (Some(app), Some(s), Some(o), None) = (f.next(), f.next(), f.next(), f.next()) else { continue };
            let (Ok(s), Ok(o)) = (s.parse::<u8>(), o.parse::<u8>()) else { continue };
            if let (false, Some(status)) = (app.is_empty(), AuthorizationStatus::from_code(s)) {
                apps.insert(app.to_string(), Entry { status, options: AuthorizationOptions(o) });
            }
        }
        AuthorizationStore { apps }
    }
}

/// `UNNotificationPresentationOptions` (an option set).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PresentationOptions(pub u8);

impl PresentationOptions {
    pub const BADGE: Self = Self(1);
    pub const SOUND: Self = Self(2);
    pub const LIST: Self = Self(8);
    pub const BANNER: Self = Self(16);
    pub const NONE: Self = Self(0);
    pub const fn union(self, o: Self) -> Self {
        Self(self.0 | o.0)
    }
    pub const fn contains(self, o: Self) -> bool {
        self.0 & o.0 == o.0
    }
}

/// What to show for a notification that arrives while its app is in front: the options the delegate's
/// `willPresent` returned (`None` = the app has no delegate, so nothing is shown), cut down to what the user allowed.
pub fn foreground_presentation(delegate: Option<PresentationOptions>, settings: &Settings) -> PresentationOptions {
    let Some(asked) = delegate else { return PresentationOptions::NONE };
    if !settings.authorization_status.allows_posting() {
        return PresentationOptions::NONE;
    }
    let mut out = PresentationOptions::NONE;
    let on = |s: Setting| s == Setting::Enabled;
    if asked.contains(PresentationOptions::BANNER) && on(settings.alert) {
        out = out.union(PresentationOptions::BANNER);
    }
    if asked.contains(PresentationOptions::LIST) && on(settings.alert) {
        out = out.union(PresentationOptions::LIST);
    }
    if asked.contains(PresentationOptions::SOUND) && on(settings.sound) {
        out = out.union(PresentationOptions::SOUND);
    }
    if asked.contains(PresentationOptions::BADGE) && on(settings.badge) {
        out = out.union(PresentationOptions::BADGE);
    }
    out
}

/// `UNNotificationSound`.
#[derive(Clone, Debug, PartialEq)]
pub enum Sound {
    /// `.default`
    Default,
    /// `.defaultCritical`
    DefaultCritical,
    /// `defaultCriticalSound(withAudioVolume:)`, volume 0...1
    DefaultCriticalVolume(f32),
    /// `UNNotificationSound(named:)`, a sound file name from the app's bundle
    Named(String),
}

impl Sound {
    pub const MAX_NAME: usize = 255;

    /// `UNNotificationSound(named:)`; the name is a bare file name.
    pub fn named(name: &str) -> Result<Sound, String> {
        if name.is_empty() || name.len() > Self::MAX_NAME || name.contains('/') || name.contains("..") {
            return Err("a sound name is a bare file name of 1 to 255 bytes".into());
        }
        Ok(Sound::Named(name.to_string()))
    }

    /// `defaultCriticalSound(withAudioVolume:)`.
    pub fn critical_with_volume(volume: f32) -> Result<Sound, String> {
        if !(0.0..=1.0).contains(&volume) {
            return Err("the critical sound volume is from 0 to 1".into());
        }
        Ok(Sound::DefaultCriticalVolume(volume))
    }

    pub fn is_critical(&self) -> bool {
        matches!(self, Sound::DefaultCritical | Sound::DefaultCriticalVolume(_))
    }

    /// Whether the sound may play: a critical one needs the critical-alert grant, any other the sound setting.
    pub fn allowed_by(&self, settings: &Settings) -> bool {
        if self.is_critical() {
            settings.critical_alert == Setting::Enabled
        } else {
            settings.sound == Setting::Enabled
        }
    }
}

/// `UNNotificationAttachment`: validated by type and size.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attachment {
    pub identifier: String,
    pub file_name: String,
    pub type_identifier: String,
}

impl Attachment {
    pub const MAX_PER_REQUEST: usize = 10;

    /// The type and size limit of a file name's extension, or `None` for a type notifications do not take.
    fn limit(file_name: &str) -> Option<(&'static str, u64)> {
        let ext = file_name.rsplit_once('.')?.1.to_ascii_lowercase();
        Some(match ext.as_str() {
            "png" => ("public.png", 10 << 20),
            "jpg" | "jpeg" => ("public.jpeg", 10 << 20),
            "gif" => ("com.compuserve.gif", 10 << 20),
            "wav" => ("com.microsoft.waveform-audio", 5 << 20),
            "mp3" => ("public.mp3", 5 << 20),
            "aif" | "aiff" => ("public.aiff-audio", 5 << 20),
            "m4a" => ("public.mpeg-4-audio", 5 << 20),
            "mpg" | "mpeg" => ("public.mpeg", 50 << 20),
            "mp4" => ("public.mpeg-4", 50 << 20),
            _ => return None,
        })
    }

    /// `UNNotificationAttachment(identifier:url:options:)`.
    pub fn new(identifier: &str, file_name: &str, size: u64) -> Result<Attachment, String> {
        let (ty, max) = Self::limit(file_name).ok_or_else(|| format!("{file_name} is not an image, audio or video type notifications take"))?;
        if size > max {
            return Err(format!("{file_name} is {size} bytes; the limit for {ty} is {max}"));
        }
        Ok(Attachment { identifier: identifier.to_string(), file_name: file_name.to_string(), type_identifier: ty.to_string() })
    }
}

/// `DateComponents` as a calendar trigger reads them; a field left `None` matches any value.
/// `weekday` is 1 = Sunday .. 7 = Saturday.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DateComponents {
    pub year: Option<i64>,
    pub month: Option<u32>,
    pub day: Option<u32>,
    pub weekday: Option<u32>,
    pub hour: Option<u32>,
    pub minute: Option<u32>,
    pub second: Option<u32>,
}

fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (i64::from(m) + if m > 2 { -3 } else { 9 }) + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    ((yoe + era * 400) + i64::from(m <= 2), m, d)
}

impl DateComponents {
    pub fn is_valid(&self) -> bool {
        self.month.map_or(true, |v| (1..=12).contains(&v))
            && self.day.map_or(true, |v| (1..=31).contains(&v))
            && self.weekday.map_or(true, |v| (1..=7).contains(&v))
            && self.hour.map_or(true, |v| v < 24)
            && self.minute.map_or(true, |v| v < 60)
            && self.second.map_or(true, |v| v < 60)
    }

    fn day_matches(&self, days: i64) -> bool {
        let (y, m, d) = civil_from_days(days);
        let wd = (days + 4).rem_euclid(7) as u32 + 1;
        self.year.map_or(true, |v| v == y) && self.month.map_or(true, |v| v == m) && self.day.map_or(true, |v| v == d) && self.weekday.map_or(true, |v| v == wd)
    }
}

/// `UNCalendarNotificationTrigger(dateMatching:repeats:)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CalendarTrigger {
    pub date: DateComponents,
    pub repeats: bool,
}

impl CalendarTrigger {
    /// `nextTriggerDate()` as Unix seconds: the first matching local time strictly after `after`. `offset(t)` is the
    /// zone's offset from UTC in seconds at Unix time `t`, so a DST change is followed. A local time inside a spring
    /// gap fires right after the jump; an hour that happens twice in autumn fires the first time. `None` when no date
    /// within the search window (nine years, three for a fixed year) matches.
    pub fn next_trigger_date(&self, after: i64, offset: &dyn Fn(i64) -> i64) -> Option<i64> {
        if !self.date.is_valid() {
            return None;
        }
        let first_day = (after + offset(after)).div_euclid(86_400) - 1;
        let span = if self.date.year.is_some() { 366 * 3 } else { 366 * 9 };
        for days in first_day..first_day + span {
            if !self.date.day_matches(days) {
                continue;
            }
            let mut best: Option<i64> = None;
            for h in (0..24).filter(|h| self.date.hour.map_or(true, |v| v == *h)) {
                for mi in (0..60).filter(|m| self.date.minute.map_or(true, |v| v == *m)) {
                    let s = self.date.second.unwrap_or(0);
                    let local = days * 86_400 + i64::from(h) * 3600 + i64::from(mi) * 60 + i64::from(s);
                    let t = resolve_local(local, offset);
                    if t > after && best.map_or(true, |b| t < b) {
                        best = Some(t);
                    }
                }
            }
            if best.is_some() {
                return best;
            }
        }
        None
    }
}

/// A local wall-clock reading (seconds since the epoch as the zone's clock shows) to Unix time: the first
/// occurrence when the reading happens twice, the moment after the jump when it does not exist.
fn resolve_local(local: i64, offset: &dyn Fn(i64) -> i64) -> i64 {
    let before = offset(local - 86_400);
    let after = offset(local + 86_400);
    let valid = |c: i64| c + offset(c) == local;
    let (a, b) = (local - before, local - after);
    match (valid(a), valid(b)) {
        (true, true) => a.min(b),
        (true, false) => a,
        (false, true) => b,
        (false, false) => a,
    }
}

/// `UNTimeIntervalNotificationTrigger(timeInterval:repeats:)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IntervalTrigger {
    pub seconds: f64,
    pub repeats: bool,
}

impl IntervalTrigger {
    /// A repeating trigger needs at least 60 seconds; any trigger needs a positive interval.
    pub const MIN_REPEAT_SECS: f64 = 60.0;

    pub fn new(seconds: f64, repeats: bool) -> Result<Self, String> {
        if !(seconds > 0.0) || !seconds.is_finite() {
            return Err("the time interval must be greater than 0".into());
        }
        if repeats && seconds < Self::MIN_REPEAT_SECS {
            return Err("a repeating time interval must be at least 60 seconds".into());
        }
        Ok(IntervalTrigger { seconds, repeats })
    }

    /// `nextTriggerDate()` for a request made at `created` (Unix seconds), asked at `now`: a one-shot fires once at
    /// `created + seconds` (`None` once past); a repeating one on the grid of `seconds` from `created`.
    pub fn next_trigger_date(&self, created: f64, now: f64) -> Option<f64> {
        let first = created + self.seconds;
        if now < first {
            return Some(first);
        }
        if !self.repeats {
            return None;
        }
        let n = ((now - first) / self.seconds).floor() + 1.0;
        Some(first + n * self.seconds)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: AuthorizationOptions = AuthorizationOptions(1 | 2 | 4);

    #[test]
    fn authorization_moves_from_not_determined_once() {
        let mut s = AuthorizationStore::default();
        assert_eq!(s.status("a"), AuthorizationStatus::NotDetermined);
        assert!(s.request("a", ALL, Decision::Allow));
        assert_eq!(s.status("a"), AuthorizationStatus::Authorized);
        assert!(s.request("a", ALL, Decision::Deny), "asked before: the stored answer stands");
        assert!(!s.request("b", ALL, Decision::Deny));
        assert_eq!(s.status("b"), AuthorizationStatus::Denied);
        assert!(!s.request("b", ALL, Decision::Allow), "a denial is not asked again");
    }

    #[test]
    fn provisional_is_granted_without_the_prompt() {
        let mut s = AuthorizationStore::default();
        assert!(s.request("a", AuthorizationOptions::ALERT.union(AuthorizationOptions::PROVISIONAL), Decision::Deny));
        assert_eq!(s.status("a"), AuthorizationStatus::Provisional);
    }

    #[test]
    fn settings_follow_what_was_granted() {
        let mut s = AuthorizationStore::default();
        let none = s.settings("a");
        assert_eq!((none.authorization_status, none.alert), (AuthorizationStatus::NotDetermined, Setting::NotSupported));
        s.request("a", AuthorizationOptions::ALERT.union(AuthorizationOptions::BADGE), Decision::Allow);
        let st = s.settings("a");
        assert_eq!((st.alert, st.badge, st.sound, st.critical_alert), (Setting::Enabled, Setting::Enabled, Setting::Disabled, Setting::NotSupported));
        s.request("d", ALL, Decision::Deny);
        assert_eq!(s.settings("d").alert, Setting::Disabled);
    }

    #[test]
    fn the_store_round_trips_and_skips_bad_lines() {
        let mut s = AuthorizationStore::default();
        s.request("a", ALL, Decision::Allow);
        s.request("b", ALL, Decision::Deny);
        let text = format!("{}junk\nx\t9\t1\n", s.to_text());
        assert_eq!(AuthorizationStore::from_text(&text), s);
    }

    fn granted() -> Settings {
        let mut s = AuthorizationStore::default();
        s.request("a", AuthorizationOptions::ALERT.union(AuthorizationOptions::SOUND), Decision::Allow);
        s.settings("a")
    }

    #[test]
    fn will_present_options_are_cut_to_the_settings() {
        let all = PresentationOptions::BANNER.union(PresentationOptions::LIST).union(PresentationOptions::SOUND).union(PresentationOptions::BADGE);
        let out = foreground_presentation(Some(all), &granted());
        assert!(out.contains(PresentationOptions::BANNER) && out.contains(PresentationOptions::LIST) && out.contains(PresentationOptions::SOUND));
        assert!(!out.contains(PresentationOptions::BADGE), "badge was not granted");
        assert_eq!(foreground_presentation(None, &granted()), PresentationOptions::NONE, "no delegate, no presentation");
        assert_eq!(foreground_presentation(Some(all), &AuthorizationStore::default().settings("x")), PresentationOptions::NONE);
        assert_eq!(foreground_presentation(Some(PresentationOptions::SOUND), &granted()), PresentationOptions::SOUND);
    }

    #[test]
    fn sounds_are_validated_and_gated() {
        assert_eq!(Sound::named("ding.caf"), Ok(Sound::Named("ding.caf".into())));
        assert!(Sound::named("").is_err() && Sound::named("a/b.caf").is_err() && Sound::named("../x").is_err());
        assert!(Sound::critical_with_volume(1.5).is_err());
        let crit = Sound::critical_with_volume(0.5).unwrap();
        assert!(crit.is_critical() && Sound::DefaultCritical.is_critical() && !Sound::Default.is_critical());
        assert!(Sound::Default.allowed_by(&granted()));
        assert!(!crit.allowed_by(&granted()), "critical needs its own grant");
    }

    #[test]
    fn attachments_are_checked_by_type_and_size() {
        let a = Attachment::new("pic", "photo.PNG", 1000).unwrap();
        assert_eq!(a.type_identifier, "public.png");
        assert!(Attachment::new("pic", "photo.png", 11 << 20).is_err());
        assert!(Attachment::new("clip", "movie.mp4", 40 << 20).is_ok());
        assert!(Attachment::new("snd", "a.mp3", 6 << 20).is_err());
        assert!(Attachment::new("doc", "notes.txt", 10).is_err());
        assert!(Attachment::new("x", "noext", 1).is_err());
    }

    #[test]
    fn civil_dates_round_trip() {
        for d in [-800_000i64, -1, 0, 59, 11_016, 20_000, 400_000] {
            let (y, m, dd) = civil_from_days(d);
            assert_eq!(days_from_civil(y, m, dd), d);
        }
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1));
    }

    fn utc(_: i64) -> i64 {
        0
    }

    #[test]
    fn a_daily_calendar_trigger_is_the_next_time_after_now() {
        let t = CalendarTrigger { date: DateComponents { hour: Some(8), minute: Some(30), ..Default::default() }, repeats: true };
        assert_eq!(t.next_trigger_date(28_800, &utc), Some(30_600));
        assert_eq!(t.next_trigger_date(30_600, &utc), Some(30_600 + 86_400));
    }

    #[test]
    fn weekday_month_day_and_year_narrow_the_match() {
        // 1970-01-01 was a Thursday (5); Monday (2) is 1970-01-05
        let mon = CalendarTrigger { date: DateComponents { weekday: Some(2), hour: Some(8), minute: Some(30), ..Default::default() }, repeats: true };
        assert_eq!(mon.next_trigger_date(28_800, &utc), Some(4 * 86_400 + 30_600));
        // 14 February at 09:00, from 1970-01-01
        let feb = CalendarTrigger { date: DateComponents { month: Some(2), day: Some(14), hour: Some(9), minute: Some(0), ..Default::default() }, repeats: true };
        assert_eq!(feb.next_trigger_date(0, &utc), Some((31 + 13) * 86_400 + 9 * 3600));
        // a fixed past year never fires
        let past = CalendarTrigger { date: DateComponents { year: Some(1960), month: Some(1), day: Some(1), hour: Some(0), minute: Some(0), ..Default::default() }, repeats: false };
        assert_eq!(past.next_trigger_date(0, &utc), None);
        let bad = CalendarTrigger { date: DateComponents { hour: Some(24), ..Default::default() }, repeats: false };
        assert_eq!(bad.next_trigger_date(0, &utc), None);
    }

    #[test]
    fn a_wildcard_hour_fires_every_hour() {
        let t = CalendarTrigger { date: DateComponents { minute: Some(15), ..Default::default() }, repeats: true };
        assert_eq!(t.next_trigger_date(0, &utc), Some(900));
        assert_eq!(t.next_trigger_date(900, &utc), Some(900 + 3600));
    }

    #[test]
    fn a_leap_day_trigger_waits_for_the_leap_year() {
        let t = CalendarTrigger { date: DateComponents { month: Some(2), day: Some(29), hour: Some(0), minute: Some(0), ..Default::default() }, repeats: true };
        // from 1970-01-01 the next 29 February is 1972-02-29 = day 789
        assert_eq!(t.next_trigger_date(0, &utc), Some(789 * 86_400));
    }

    // a zone at +0 until T0 and +1 h after: a spring forward at UTC 02:00 on day 100
    const T0: i64 = 100 * 86_400 + 2 * 3600;
    fn spring(t: i64) -> i64 {
        if t < T0 { 0 } else { 3600 }
    }

    #[test]
    fn a_daily_time_keeps_its_wall_clock_across_a_forward_change() {
        let t = CalendarTrigger { date: DateComponents { hour: Some(9), minute: Some(0), ..Default::default() }, repeats: true };
        let after_jump = t.next_trigger_date(99 * 86_400 + 12 * 3600, &spring).unwrap();
        assert_eq!(after_jump, 100 * 86_400 + 8 * 3600, "09:00 local after the jump is 08:00 UTC");
        let before_jump = t.next_trigger_date(98 * 86_400 + 12 * 3600, &spring).unwrap();
        assert_eq!(before_jump, 99 * 86_400 + 9 * 3600);
    }

    #[test]
    fn a_time_inside_the_spring_gap_fires_after_the_jump() {
        // local 02:30 does not exist on day 100 (clocks go 02:00 -> 03:00)
        let t = CalendarTrigger { date: DateComponents { hour: Some(2), minute: Some(30), ..Default::default() }, repeats: true };
        let got = t.next_trigger_date(99 * 86_400 + 12 * 3600, &spring).unwrap();
        assert!(got >= T0 && got <= T0 + 3600, "fires right after the jump, got {got}");
    }

    // a zone at +1 h until T1 and +0 after: clocks go back at UTC 02:00 on day 200
    const T1: i64 = 200 * 86_400 + 2 * 3600;
    fn autumn(t: i64) -> i64 {
        if t < T1 { 3600 } else { 0 }
    }

    #[test]
    fn a_time_that_happens_twice_fires_the_first_time() {
        let t = CalendarTrigger { date: DateComponents { hour: Some(2), minute: Some(30), ..Default::default() }, repeats: true };
        let got = t.next_trigger_date(199 * 86_400 + 12 * 3600, &autumn).unwrap();
        assert_eq!(got, 200 * 86_400 + 3600 + 1800, "02:30 at +1 h is 01:30 UTC, the first of the two");
    }

    #[test]
    fn interval_triggers_enforce_the_repeat_minimum() {
        assert!(IntervalTrigger::new(0.0, false).is_err());
        assert!(IntervalTrigger::new(f64::NAN, false).is_err());
        assert!(IntervalTrigger::new(5.0, false).is_ok());
        assert!(IntervalTrigger::new(59.0, true).is_err());
        assert!(IntervalTrigger::new(60.0, true).is_ok());
    }

    #[test]
    fn interval_next_trigger_date_follows_the_grid() {
        let once = IntervalTrigger::new(10.0, false).unwrap();
        assert_eq!(once.next_trigger_date(100.0, 105.0), Some(110.0));
        assert_eq!(once.next_trigger_date(100.0, 110.0), None);
        let rep = IntervalTrigger::new(60.0, true).unwrap();
        assert_eq!(rep.next_trigger_date(0.0, 30.0), Some(60.0));
        assert_eq!(rep.next_trigger_date(0.0, 60.0), Some(120.0));
        assert_eq!(rep.next_trigger_date(0.0, 200.0), Some(240.0));
    }
}
