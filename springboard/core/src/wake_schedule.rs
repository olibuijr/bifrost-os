//! Scheduled wake and power-on: the repeating times at which the computer should start itself (Settings > Battery >
//! Schedule), the next such moment, and what it takes to arm the hardware clock for it.
//!
//! The model is pure. A [`Schedule`] holds [`Entry`] values (days of the week, a minute of the day, whether the
//! machine is waking from sleep or starting from off). [`Schedule::next`] answers the next moment after `now` as a
//! Unix time, given the local offset from UTC in seconds at `now`. The offset is not re-derived for the future moment,
//! so a change of clock offset between now and then (summer time) shifts the moment by that change; the caller asks
//! again after every wake, which corrects it. Nothing here reads a clock or a device.
//!
//! The hardware side is the real-time clock alarm in `/sys/class/rtc/rtc0/wakealarm` (Linux). [`validate_alarm`] and
//! [`alarm_value`] produce the text the privileged helper writes, [`parse_alarm`] reads it back, and
//! [`parse_proc_wakeup`] with [`classify_source`] explain which device may wake the machine and why it woke.

/// Days of the week as a set. Bit 0 is Monday and bit 6 is Sunday.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Weekdays(pub u8);

impl Weekdays {
    pub const NONE: Weekdays = Weekdays(0);
    pub const EVERY_DAY: Weekdays = Weekdays(0b0111_1111);
    pub const WEEKDAYS: Weekdays = Weekdays(0b0001_1111);
    pub const WEEKEND: Weekdays = Weekdays(0b0110_0000);

    /// True when `day` (0 = Monday .. 6 = Sunday) is in the set. Any other number is never in it.
    pub fn contains(self, day: u8) -> bool {
        day < 7 && self.0 & (1 << day) != 0
    }

    /// The set with `day` added. A number above 6 changes nothing.
    pub fn with(self, day: u8) -> Weekdays {
        if day < 7 {
            Weekdays(self.0 | (1 << day))
        } else {
            self
        }
    }

    /// The word the schedule shows for the set.
    pub fn label(self) -> &'static str {
        match self {
            Weekdays::EVERY_DAY => "Every day",
            Weekdays::WEEKDAYS => "Weekdays",
            Weekdays::WEEKEND => "Weekend",
            Weekdays::NONE => "Never",
            _ => "Some days",
        }
    }
}

/// What the machine does at the scheduled time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Wake from sleep.
    Wake,
    /// Start from off.
    PowerOn,
}

/// One repeating time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    pub days: Weekdays,
    /// Minutes after local midnight, 0 to 1439.
    pub minute_of_day: u16,
    pub action: Action,
    pub enabled: bool,
}

impl Entry {
    /// An enabled entry. `minute_of_day` is clamped to the last minute of the day.
    pub fn new(days: Weekdays, minute_of_day: u16, action: Action) -> Entry {
        Entry { days, minute_of_day: minute_of_day.min(1439), action, enabled: true }
    }
}

const DAY: i64 = 86_400;

/// The weekday (0 = Monday) of a count of days since 1970-01-01, which was a Thursday.
pub fn weekday_of_day(days_since_epoch: i64) -> u8 {
    (days_since_epoch + 3).rem_euclid(7) as u8
}

/// The moment an entry fires, with the entry that fires it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Moment {
    /// Unix time in seconds.
    pub at: i64,
    pub entry: Entry,
}

/// The list of repeating times.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Schedule {
    entries: Vec<Entry>,
}

impl Schedule {
    pub fn new() -> Schedule {
        Schedule::default()
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn add(&mut self, entry: Entry) {
        self.entries.push(entry);
    }

    /// Removes entry `index`. Returns false when there is none.
    pub fn remove(&mut self, index: usize) -> bool {
        if index < self.entries.len() {
            self.entries.remove(index);
            true
        } else {
            false
        }
    }

    /// Turns entry `index` on or off. Returns false when there is none.
    pub fn set_enabled(&mut self, index: usize, enabled: bool) -> bool {
        match self.entries.get_mut(index) {
            Some(e) => {
                e.enabled = enabled;
                true
            }
            None => false,
        }
    }

    /// The earliest moment strictly after `now` (Unix seconds) at which an enabled entry fires. `utc_offset_secs` is the
    /// local time's offset from UTC at `now`. None when no enabled entry has a day.
    pub fn next(&self, now: i64, utc_offset_secs: i64) -> Option<Moment> {
        let local_now = now + utc_offset_secs;
        let today = local_now.div_euclid(DAY);
        let mut best: Option<Moment> = None;
        for entry in self.entries.iter().filter(|e| e.enabled && e.days.0 & 0b0111_1111 != 0) {
            // eight days ahead covers today's time having passed on the only day of the entry
            for ahead in 0..=7i64 {
                let day = today + ahead;
                if !entry.days.contains(weekday_of_day(day)) {
                    continue;
                }
                let local = day * DAY + i64::from(entry.minute_of_day) * 60;
                if local > local_now {
                    let at = local - utc_offset_secs;
                    if best.map_or(true, |b| at < b.at) {
                        best = Some(Moment { at, entry: *entry });
                    }
                    break;
                }
            }
        }
        best
    }

    /// The schedule as text: one line per entry, `days,minute,action,enabled`, days as the bit set number.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for e in &self.entries {
            let action = match e.action {
                Action::Wake => "wake",
                Action::PowerOn => "poweron",
            };
            out.push_str(&format!("{},{},{},{}\n", e.days.0, e.minute_of_day, action, u8::from(e.enabled)));
        }
        out
    }

    /// Reads the text [`Schedule::to_text`] wrote. A line that does not parse is skipped, so one damaged line does not
    /// lose the rest.
    pub fn from_text(text: &str) -> Schedule {
        let mut schedule = Schedule::new();
        for line in text.lines() {
            let parts: Vec<&str> = line.trim().split(',').collect();
            if parts.len() != 4 {
                continue;
            }
            let (Ok(days), Ok(minute)) = (parts[0].parse::<u8>(), parts[1].parse::<u16>()) else { continue };
            let action = match parts[2] {
                "wake" => Action::Wake,
                "poweron" => Action::PowerOn,
                _ => continue,
            };
            if days > 0b0111_1111 || minute > 1439 {
                continue;
            }
            let enabled = match parts[3] {
                "1" => true,
                "0" => false,
                _ => continue,
            };
            schedule.add(Entry { days: Weekdays(days), minute_of_day: minute, action, enabled });
        }
        schedule
    }
}

/// The real-time clock alarm file the privileged helper writes.
pub const RTC_WAKEALARM: &str = "/sys/class/rtc/rtc0/wakealarm";

/// The furthest ahead an alarm may be set: a little over a year.
pub const MAX_AHEAD_SECS: i64 = 400 * DAY;

/// Why an alarm time was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlarmError {
    /// The time is not after `now`.
    NotInFuture,
    /// The time is more than [`MAX_AHEAD_SECS`] ahead.
    TooFar,
}

/// Checks an alarm time (Unix seconds) against `now` before it is written to the clock.
pub fn validate_alarm(at: i64, now: i64) -> Result<i64, AlarmError> {
    if at <= now {
        Err(AlarmError::NotInFuture)
    } else if at - now > MAX_AHEAD_SECS {
        Err(AlarmError::TooFar)
    } else {
        Ok(at)
    }
}

/// The text to write to [`RTC_WAKEALARM`]. An alarm already set must be cleared first (write [`CLEAR_ALARM`]), because
/// the kernel refuses a new value while an old one is armed.
pub fn alarm_value(at: i64) -> String {
    format!("{at}\n")
}

/// The text that clears the alarm.
pub const CLEAR_ALARM: &str = "0\n";

/// The armed alarm in the text read from [`RTC_WAKEALARM`]. An empty file or `0` means none.
pub fn parse_alarm(text: &str) -> Option<i64> {
    match text.trim().parse::<i64>() {
        Ok(v) if v > 0 => Some(v),
        _ => None,
    }
}

/// What woke the machine, or what may.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WakeReason {
    Timer,
    PowerButton,
    SleepButton,
    Lid,
    Keyboard,
    Usb,
    Network,
    Other,
}

impl WakeReason {
    pub fn label(self) -> &'static str {
        match self {
            WakeReason::Timer => "Scheduled wake",
            WakeReason::PowerButton => "Power button",
            WakeReason::SleepButton => "Sleep button",
            WakeReason::Lid => "Opening the lid",
            WakeReason::Keyboard => "Keyboard",
            WakeReason::Usb => "USB device",
            WakeReason::Network => "Network activity",
            WakeReason::Other => "Other",
        }
    }
}

/// The reason for an ACPI wake device name (`LID0`, `PWRB`, `XHC`, `GLAN`, ...). The match is by prefix, without case.
pub fn classify_source(name: &str) -> WakeReason {
    let n = name.trim().to_ascii_uppercase();
    let starts = |p: &[&str]| p.iter().any(|q| n.starts_with(*q));
    if starts(&["RTC", "ALRM"]) {
        WakeReason::Timer
    } else if starts(&["PWRB", "PBTN"]) {
        WakeReason::PowerButton
    } else if starts(&["SLPB", "SBTN"]) {
        WakeReason::SleepButton
    } else if starts(&["LID"]) {
        WakeReason::Lid
    } else if starts(&["PS2", "KBD"]) {
        WakeReason::Keyboard
    } else if starts(&["XHC", "EHC", "UHC", "USB"]) {
        WakeReason::Usb
    } else if starts(&["GLAN", "LAN", "ETH", "WOL", "WLAN"]) {
        WakeReason::Network
    } else {
        WakeReason::Other
    }
}

/// One device in `/proc/acpi/wakeup`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WakeDevice {
    pub name: String,
    pub enabled: bool,
    pub reason: WakeReason,
}

/// Reads the table in `/proc/acpi/wakeup`: a header line, then `Device S-state Status Sysfs-node` per row, where the
/// status is `*enabled` or `*disabled`. Rows with fewer than three fields are skipped.
pub fn parse_proc_wakeup(text: &str) -> Vec<WakeDevice> {
    text.lines()
        .skip(1)
        .filter_map(|line| {
            let mut f = line.split_whitespace();
            let name = f.next()?;
            let _state = f.next()?;
            let status = f.next()?;
            let enabled = match status {
                "*enabled" => true,
                "*disabled" => false,
                _ => return None,
            };
            Some(WakeDevice { name: name.to_string(), enabled, reason: classify_source(name) })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // 2024-01-01 was a Monday; 00:00 UTC is 1704067200.
    const MON_2024: i64 = 1_704_067_200;

    #[test]
    fn the_epoch_day_was_a_thursday_and_a_known_monday_is_zero() {
        assert_eq!(weekday_of_day(0), 3);
        assert_eq!(weekday_of_day(MON_2024 / DAY), 0);
        assert_eq!(weekday_of_day(-1), 2);
    }

    #[test]
    fn a_time_later_today_fires_today() {
        let mut s = Schedule::new();
        s.add(Entry::new(Weekdays::EVERY_DAY, 7 * 60, Action::Wake));
        let m = s.next(MON_2024 + 3 * 3600, 0).unwrap();
        assert_eq!(m.at, MON_2024 + 7 * 3600);
        assert_eq!(m.entry.action, Action::Wake);
    }

    #[test]
    fn a_time_already_passed_waits_for_the_next_allowed_day() {
        let mut s = Schedule::new();
        s.add(Entry::new(Weekdays::WEEKEND, 9 * 60, Action::PowerOn));
        // Monday noon: next is Saturday 09:00, five days on
        let m = s.next(MON_2024 + 12 * 3600, 0).unwrap();
        assert_eq!(m.at, MON_2024 + 5 * DAY + 9 * 3600);
    }

    #[test]
    fn the_same_minute_does_not_fire_again() {
        let mut s = Schedule::new();
        s.add(Entry::new(Weekdays::EVERY_DAY, 60, Action::Wake));
        let m = s.next(MON_2024 + 3600, 0).unwrap();
        assert_eq!(m.at, MON_2024 + DAY + 3600);
    }

    #[test]
    fn a_single_day_entry_whose_time_passed_fires_in_a_week() {
        let mut s = Schedule::new();
        s.add(Entry::new(Weekdays::NONE.with(0), 60, Action::Wake));
        let m = s.next(MON_2024 + 7200, 0).unwrap();
        assert_eq!(m.at, MON_2024 + 7 * DAY + 3600);
    }

    #[test]
    fn the_local_offset_moves_the_moment_in_utc() {
        let mut s = Schedule::new();
        s.add(Entry::new(Weekdays::EVERY_DAY, 8 * 60, Action::Wake));
        // UTC+2: 08:00 local is 06:00 UTC
        let m = s.next(MON_2024, 2 * 3600).unwrap();
        assert_eq!(m.at, MON_2024 + 6 * 3600);
    }

    #[test]
    fn a_negative_offset_that_changes_the_local_day_is_handled() {
        let mut s = Schedule::new();
        s.add(Entry::new(Weekdays::NONE.with(6), 0, Action::Wake));
        // UTC-5 at Monday 00:00 UTC is Sunday 19:00 local; Sunday 00:00 local has passed, so next Sunday
        let m = s.next(MON_2024, -5 * 3600).unwrap();
        assert_eq!(m.at, MON_2024 - DAY + 7 * DAY + 5 * 3600);
    }

    #[test]
    fn the_earliest_enabled_entry_wins_and_disabled_ones_are_skipped() {
        let mut s = Schedule::new();
        s.add(Entry::new(Weekdays::EVERY_DAY, 6 * 60, Action::Wake));
        s.add(Entry::new(Weekdays::EVERY_DAY, 5 * 60, Action::PowerOn));
        s.set_enabled(1, false);
        assert_eq!(s.next(MON_2024, 0).unwrap().at, MON_2024 + 6 * 3600);
        s.set_enabled(1, true);
        assert_eq!(s.next(MON_2024, 0).unwrap().entry.action, Action::PowerOn);
        assert!(!s.set_enabled(9, true));
    }

    #[test]
    fn an_empty_schedule_or_an_entry_without_days_has_no_next() {
        assert_eq!(Schedule::new().next(MON_2024, 0), None);
        let mut s = Schedule::new();
        s.add(Entry::new(Weekdays::NONE, 60, Action::Wake));
        assert_eq!(s.next(MON_2024, 0), None);
    }

    #[test]
    fn the_schedule_round_trips_through_text_and_skips_damaged_lines() {
        let mut s = Schedule::new();
        s.add(Entry::new(Weekdays::WEEKDAYS, 450, Action::Wake));
        let mut off = Entry::new(Weekdays::WEEKEND, 600, Action::PowerOn);
        off.enabled = false;
        s.add(off);
        let text = s.to_text();
        assert_eq!(Schedule::from_text(&text), s);
        let damaged = format!("garbage\n{text}999,1,wake,1\n1,5000,wake,1\n");
        assert_eq!(Schedule::from_text(&damaged), s);
    }

    #[test]
    fn the_minute_is_clamped_and_weekday_numbers_above_six_are_ignored() {
        assert_eq!(Entry::new(Weekdays::EVERY_DAY, 5000, Action::Wake).minute_of_day, 1439);
        assert_eq!(Weekdays::NONE.with(9), Weekdays::NONE);
        assert!(!Weekdays::EVERY_DAY.contains(7));
        assert_eq!(Weekdays::WEEKDAYS.label(), "Weekdays");
    }

    #[test]
    fn an_alarm_must_be_in_the_future_and_not_too_far() {
        assert_eq!(validate_alarm(100, 100), Err(AlarmError::NotInFuture));
        assert_eq!(validate_alarm(101, 100), Ok(101));
        assert_eq!(validate_alarm(100 + MAX_AHEAD_SECS + 1, 100), Err(AlarmError::TooFar));
        assert_eq!(alarm_value(1_704_067_200), "1704067200\n");
    }

    #[test]
    fn the_alarm_file_is_read_back() {
        assert_eq!(parse_alarm("1704067200\n"), Some(1_704_067_200));
        assert_eq!(parse_alarm("0\n"), None);
        assert_eq!(parse_alarm(""), None);
        assert_eq!(parse_alarm("soon"), None);
    }

    #[test]
    fn wake_devices_are_named_by_what_they_are() {
        assert_eq!(classify_source("LID0"), WakeReason::Lid);
        assert_eq!(classify_source("pwrb"), WakeReason::PowerButton);
        assert_eq!(classify_source("XHC"), WakeReason::Usb);
        assert_eq!(classify_source("GLAN"), WakeReason::Network);
        assert_eq!(classify_source("RTC0"), WakeReason::Timer);
        assert_eq!(classify_source("PEG0"), WakeReason::Other);
        assert_eq!(WakeReason::Lid.label(), "Opening the lid");
    }

    #[test]
    fn the_acpi_wakeup_table_is_parsed() {
        let text = "Device\tS-state\t  Status   Sysfs node\nLID0\t  S4\t*enabled   platform:PNP0C0D:00\nXHC\t  S3\t*disabled  pci:0000:00:14.0\nbroken row\n";
        let d = parse_proc_wakeup(text);
        assert_eq!(d.len(), 2);
        assert_eq!(d[0], WakeDevice { name: "LID0".into(), enabled: true, reason: WakeReason::Lid });
        assert!(!d[1].enabled);
        assert_eq!(d[1].reason, WakeReason::Usb);
    }
}
