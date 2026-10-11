//! Alarms: a persisted list of (time, label, repeat weekdays, sound, snooze,
//! vibrate, fade-in) with the next fire time computed in local civil time,
//! so it holds across daylight-saving changes and manual clock changes.
//! The store is clock.ini (`alarm.N.key=value`), written atomically.

/// The stock snooze length: nine minutes.
pub const SNOOZE_SECS: i64 = 540;
/// An alarm that came due this long ago (the shell was down) still rings.
pub const GRACE_SECS: i64 = 600;
/// The alarm tones on offer.
pub const SOUNDS: [&str; 6] = ["Radar", "Beacon", "Chimes", "Circuit", "Reflection", "Summit"];

/// Repeat sets as a weekday bit mask, bit 0 = Sunday.
pub const EVERY_DAY: u8 = 0x7f;
pub const WEEKDAYS: u8 = 0b011_1110;
pub const WEEKEND: u8 = 0b100_0001;
const DAY_NAMES: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

#[derive(Clone, Debug, PartialEq)]
pub struct Alarm {
    /// stable identity, so a ringing alarm can be snoozed after the list changed
    pub id: u32,
    pub hour: u32,
    pub min: u32,
    pub label: String,
    pub on: bool,
    /// weekday mask (bit 0 = Sunday); 0 = once
    pub repeat: u8,
    pub sound: String,
    pub snooze: bool,
    pub vibrate: bool,
    /// the volume rises gradually while it rings
    pub ramp: bool,
    /// the local day (days since 1970) whose occurrence is skipped
    pub skip_day: Option<i64>,
    /// a pending snooze: Unix time it rings again
    pub snoozed_until: Option<i64>,
}

impl Alarm {
    pub fn new(hour: u32, min: u32) -> Self {
        Alarm { id: 0, hour, min, label: "Alarm".into(), on: true, repeat: 0, sound: SOUNDS[0].into(), snooze: true, vibrate: true, ramp: true, skip_day: None, snoozed_until: None }
    }

    pub fn time_text(&self) -> String {
        format!("{:02}:{:02}", self.hour, self.min)
    }

    pub fn repeat_text(&self) -> String {
        repeat_text(self.repeat)
    }

    /// The next time this alarm rings after `after`: the earlier of its
    /// pending snooze and its next regular occurrence.
    pub fn next_fire(&self, after: i64, offset_at: &dyn Fn(i64) -> i32) -> Option<i64> {
        let snooze = self.snoozed_until.filter(|&s| s > after);
        let regular = if self.on { self.regular_fire(after, offset_at) } else { None };
        match (snooze, regular) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// The next scheduled occurrence strictly after `after`, skipping a
    /// skipped day and days outside the repeat set.
    pub fn regular_fire(&self, after: i64, offset_at: &dyn Fn(i64) -> i32) -> Option<i64> {
        let today = (after + offset_at(after) as i64).div_euclid(86_400);
        let secs = (self.hour * 3600 + self.min * 60) as i64;
        (0..=14).find_map(|k| {
            let day = today + k;
            let wday = (day + 4).rem_euclid(7);
            if self.repeat != 0 && self.repeat & (1 << wday) == 0 {
                return None;
            }
            if self.skip_day == Some(day) {
                return None;
            }
            let at = local_to_unix(day, secs, offset_at);
            (at > after).then_some(at)
        })
    }
}

/// Unix time of local time-of-day `secs` on local day `day` (days since
/// 1970), for a zone whose UTC offset at any instant is `offset_at`. A
/// time that happens twice (clocks go back) takes the first; one that
/// never happens (clocks go forward) rings at the moment the clocks jump,
/// shown an hour later, as the stock Clock does.
pub fn local_to_unix(day: i64, secs: i64, offset_at: &dyn Fn(i64) -> i32) -> i64 {
    let local = day * 86_400 + secs;
    let (before, after) = (offset_at(local - 86_400), offset_at(local + 86_400));
    let mut best: Option<i64> = None;
    for o in [before, after] {
        let utc = local - o as i64;
        if offset_at(utc) == o {
            best = Some(best.map_or(utc, |b| b.min(utc)));
        }
    }
    best.unwrap_or(local - before as i64)
}

/// "Never", "Every day", "Weekdays", "Weekends", "Every Monday" or "Mon, Wed".
pub fn repeat_text(mask: u8) -> String {
    let mask = mask & EVERY_DAY;
    match mask {
        0 => "Never".into(),
        EVERY_DAY => "Every day".into(),
        WEEKDAYS => "Weekdays".into(),
        WEEKEND => "Weekends".into(),
        m if m.count_ones() == 1 => format!("Every {}", DAY_NAMES[m.trailing_zeros() as usize]),
        m => [1usize, 2, 3, 4, 5, 6, 0].iter().filter(|&&d| m & (1 << d) != 0).map(|&d| &DAY_NAMES[d][..3]).collect::<Vec<_>>().join(", "),
    }
}

/// "7 hr 5 min", "2 d 3 hr", "12 min" or "less than a minute".
pub fn eta_text(secs: i64) -> String {
    let minutes = secs.max(0) / 60;
    let (d, h, m) = (minutes / 1440, minutes / 60 % 24, minutes % 60);
    if d > 0 {
        format!("{d} d {h} hr")
    } else if h > 0 && m > 0 {
        format!("{h} hr {m} min")
    } else if h > 0 {
        format!("{h} hr")
    } else if m > 0 {
        format!("{m} min")
    } else {
        "less than a minute".into()
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Alarms {
    pub list: Vec<Alarm>,
    /// the Unix second `fire` last looked at, so an occurrence rings once
    checked: Option<i64>,
}

impl Alarms {
    /// Parses clock.ini: `alarm.N.time=HH:MM`, `.on`, `.label`, `.repeat`,
    /// `.sound`, `.snooze`, `.vibrate`, `.ramp`, `.skip`, `.snoozed`, `.id`
    /// and `meta.checked`; unknown keys are ignored, bad lines skipped.
    pub fn parse(ini: &str) -> Self {
        let mut slots: Vec<(usize, Alarm)> = vec![];
        let mut checked = None;
        for (key, value) in super::store::pairs(ini) {
            if key == "meta.checked" {
                checked = value.parse().ok();
                continue;
            }
            let mut parts = key.splitn(3, '.');
            if parts.next() != Some("alarm") {
                continue;
            }
            let (Some(Ok(n)), Some(field)) = (parts.next().map(str::parse::<usize>), parts.next()) else { continue };
            let slot = match slots.iter_mut().find(|(i, _)| *i == n) {
                Some(s) => &mut s.1,
                None => {
                    slots.push((n, Alarm { id: n as u32 + 1, ..Alarm::new(0, 0) }));
                    &mut slots.last_mut().unwrap().1
                }
            };
            match field {
                "time" => {
                    if let Some((h, m)) = value.split_once(':') {
                        if let (Ok(h), Ok(m)) = (h.parse::<u32>(), m.parse::<u32>()) {
                            if h < 24 && m < 60 {
                                (slot.hour, slot.min) = (h, m);
                            }
                        }
                    }
                }
                "on" => slot.on = value == "1",
                "label" => slot.label = value.to_string(),
                "repeat" => slot.repeat = value.parse::<u8>().map_or(0, |m| m & EVERY_DAY),
                "sound" => {
                    if SOUNDS.contains(&value) {
                        slot.sound = value.to_string();
                    }
                }
                "snooze" => slot.snooze = value == "1",
                "vibrate" => slot.vibrate = value == "1",
                "ramp" => slot.ramp = value == "1",
                "skip" => slot.skip_day = value.parse().ok(),
                "snoozed" => slot.snoozed_until = value.parse().ok(),
                "id" => {
                    if let Ok(id) = value.parse::<u32>() {
                        slot.id = id;
                    }
                }
                _ => {}
            }
        }
        slots.sort_by_key(|(i, _)| *i);
        let mut list: Vec<Alarm> = slots.into_iter().map(|(_, a)| a).collect();
        for i in 1..list.len() {
            if list[..i].iter().any(|o| o.id == list[i].id) {
                list[i].id = list.iter().map(|a| a.id).max().unwrap_or(0) + 1;
            }
        }
        Alarms { list, checked }
    }

    pub fn serialize(&self) -> String {
        let mut out = String::from("# SpringBoard Clock alarms\n");
        for (i, a) in self.list.iter().enumerate() {
            out += &format!("alarm.{i}.time={:02}:{:02}\nalarm.{i}.on={}\nalarm.{i}.label={}\n", a.hour, a.min, a.on as u8, a.label.replace(['\n', '\r'], " "));
            out += &format!("alarm.{i}.id={}\nalarm.{i}.repeat={}\nalarm.{i}.sound={}\n", a.id, a.repeat, a.sound);
            out += &format!("alarm.{i}.snooze={}\nalarm.{i}.vibrate={}\nalarm.{i}.ramp={}\n", a.snooze as u8, a.vibrate as u8, a.ramp as u8);
            if let Some(d) = a.skip_day {
                out += &format!("alarm.{i}.skip={d}\n");
            }
            if let Some(s) = a.snoozed_until {
                out += &format!("alarm.{i}.snoozed={s}\n");
            }
        }
        if let Some(c) = self.checked {
            out += &format!("meta.checked={c}\n");
        }
        out
    }

    /// Where `fire` resumes after a restart; without a stored value, from `now`.
    pub fn start_at(&mut self, now: i64) {
        self.checked.get_or_insert(now);
    }

    /// Adds `alarm` (it gets a fresh id), keeping the list sorted by time as
    /// the bifrost does. Returns the id.
    pub fn add(&mut self, mut alarm: Alarm) -> u32 {
        alarm.id = self.list.iter().map(|a| a.id).max().unwrap_or(0) + 1;
        let id = alarm.id;
        self.list.push(alarm);
        self.list.sort_by_key(|a| (a.hour, a.min));
        id
    }

    /// Replaces the alarm with `alarm.id`, re-sorting; its pending snooze and
    /// skip are dropped because the schedule changed.
    pub fn replace(&mut self, mut alarm: Alarm) {
        alarm.snoozed_until = None;
        alarm.skip_day = None;
        if let Some(slot) = self.list.iter_mut().find(|a| a.id == alarm.id) {
            *slot = alarm;
            self.list.sort_by_key(|a| (a.hour, a.min));
        }
    }

    pub fn index_of(&self, id: u32) -> Option<usize> {
        self.list.iter().position(|a| a.id == id)
    }

    /// Unix time of the next ring after `unix`, for the RTC wake-up.
    pub fn next_due(&self, unix: i64, offset_at: &dyn Fn(i64) -> i32) -> Option<i64> {
        self.next_alarm(unix, offset_at).map(|(_, at)| at)
    }

    /// The alarm that rings next and when.
    pub fn next_alarm(&self, unix: i64, offset_at: &dyn Fn(i64) -> i32) -> Option<(usize, i64)> {
        self.list.iter().enumerate().filter_map(|(i, a)| a.next_fire(unix, offset_at).map(|at| (i, at))).min_by_key(|&(_, at)| at)
    }

    /// "Alarm in 7 hr 5 min" for the lock screen and Control Center.
    pub fn summary(&self, unix: i64, offset_at: &dyn Fn(i64) -> i32) -> Option<String> {
        self.next_due(unix, offset_at).map(|at| format!("Alarm in {}", eta_text(at - unix)))
    }

    /// The alarms due since the last look, each once. One-shot alarms switch
    /// off; repeating ones stay on. A clock set back re-arms from `now`; an
    /// occurrence missed while the shell was down rings if under ten minutes old.
    pub fn fire(&mut self, now: i64, offset_at: &dyn Fn(i64) -> i32) -> Vec<Alarm> {
        let mut after = self.checked.unwrap_or(now - 1).max(now - GRACE_SECS);
        if after > now {
            after = now - 1;
        }
        self.checked = Some(now);
        let today = (now + offset_at(now) as i64).div_euclid(86_400);
        let mut fired = vec![];
        for a in &mut self.list {
            let snooze_due = a.snoozed_until.is_some_and(|s| s > after && s <= now);
            let regular_due = a.on && a.regular_fire(after, offset_at).is_some_and(|f| f <= now);
            if a.skip_day.is_some_and(|d| d < today) {
                a.skip_day = None;
            }
            if !snooze_due && !regular_due {
                continue;
            }
            if snooze_due {
                a.snoozed_until = None;
            }
            if regular_due && a.repeat == 0 {
                a.on = false;
            }
            fired.push(a.clone());
        }
        fired
    }

    /// Snoozes alarm `id` for nine minutes; false when it has snooze off.
    pub fn snooze(&mut self, id: u32, now: i64) -> bool {
        match self.list.iter_mut().find(|a| a.id == id && a.snooze) {
            Some(a) => {
                a.snoozed_until = Some(now + SNOOZE_SECS);
                true
            }
            None => false,
        }
    }

    /// Stops alarm `id` for good, dropping a pending snooze.
    pub fn dismiss(&mut self, id: u32) {
        if let Some(a) = self.list.iter_mut().find(|a| a.id == id) {
            a.snoozed_until = None;
        }
    }

    /// Skips the next occurrence of alarm `id`; a one-shot alarm just turns off.
    pub fn skip_next(&mut self, id: u32, now: i64, offset_at: &dyn Fn(i64) -> i32) {
        let Some(a) = self.list.iter_mut().find(|a| a.id == id) else { return };
        if a.repeat == 0 {
            a.on = false;
        } else if let Some(at) = a.regular_fire(now, offset_at) {
            a.skip_day = Some((at + offset_at(at) as i64).div_euclid(86_400));
        }
    }

    pub fn unskip(&mut self, id: u32) {
        if let Some(a) = self.list.iter_mut().find(|a| a.id == id) {
            a.skip_day = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::clock::tz::{days_from_civil, CITIES};

    fn zero(_unix: i64) -> i32 {
        0
    }

    const UTC: &dyn Fn(i64) -> i32 = &zero;

    fn utc(y: i64, m: u32, d: u32, h: i64, mi: i64) -> i64 {
        days_from_civil(y, m, d) * 86_400 + h * 3600 + mi * 60
    }

    fn alarm(h: u32, m: u32, on: bool) -> Alarm {
        Alarm { on, ..Alarm::new(h, m) }
    }

    fn london(u: i64) -> i32 {
        CITIES.iter().find(|c| c.name == "London").unwrap().offset_at(u)
    }

    #[test]
    fn ini_round_trips_and_tolerates_junk() {
        let mut a = Alarms::default();
        a.add(alarm(7, 30, true));
        a.add(Alarm { label: "Wake = up".into(), repeat: WEEKDAYS, sound: "Summit".into(), snooze: false, vibrate: false, ramp: false, skip_day: Some(20_000), snoozed_until: Some(5), ..alarm(6, 5, false) });
        a.start_at(1234);
        let text = a.serialize();
        assert!(text.contains("alarm.0.time=06:05\nalarm.0.on=0\nalarm.0.label=Wake = up\n"));
        assert!(text.contains("alarm.1.time=07:30\nalarm.1.on=1\n"));
        assert_eq!(Alarms::parse(&text), a);
        let junk = "garbage\nalarm.x.time=1:00\nalarm.2.time=25:00\nalarm.2.on=1\n\nalarm.0.time=12:15\nother=1\nalarm.0.colour=red\nalarm.0.sound=Nope\nalarm.0.repeat=999\n";
        let parsed = Alarms::parse(junk);
        let first = Alarm { id: 1, ..alarm(12, 15, true) };
        let second = Alarm { id: 3, ..alarm(0, 0, true) };
        assert_eq!(parsed.list, vec![first, second], "bad time keeps the default, unknown sound keeps Radar, a bad mask means once, order by index");
        assert!(Alarms::parse("").list.is_empty());
        let mut dup = Alarms::parse("alarm.0.id=4\nalarm.0.time=01:00\nalarm.1.id=4\nalarm.1.time=02:00\n");
        assert_ne!(dup.list[0].id, dup.list[1].id, "duplicate ids are repaired");
        dup.add(alarm(3, 0, true));
        assert_eq!(dup.list.iter().map(|a| a.id).collect::<std::collections::HashSet<_>>().len(), 3);
    }

    #[test]
    fn next_due_wraps_to_tomorrow_and_skips_disabled() {
        let mut a = Alarms::default();
        let noon = utc(2026, 10, 3, 12, 0);
        assert_eq!(a.next_due(noon, UTC), None);
        a.add(alarm(8, 0, true));
        a.add(alarm(18, 30, false));
        assert_eq!(a.next_due(noon, UTC), Some(noon + 20 * 3600), "08:00 tomorrow");
        a.list[1].on = true;
        assert_eq!(a.next_due(noon, UTC), Some(noon + 6 * 3600 + 1800));
        // a +2 h zone: local noon is 10:00 UTC; 18:30 local is 16:30 UTC
        let local_noon = noon - 2 * 3600;
        let plus2: &dyn Fn(i64) -> i32 = &|_| 7200;
        assert_eq!(a.next_due(local_noon, plus2), Some(local_noon + 6 * 3600 + 1800));
        assert_eq!(a.next_due(noon + 6 * 3600 + 1800, UTC), Some(noon + 20 * 3600), "an alarm due right now is not next");
    }

    #[test]
    fn repeat_sets_pick_the_next_matching_weekday() {
        // Fri 2026-10-02 18:00; a weekday 07:00 alarm rings Monday 5 Oct
        let fri = utc(2026, 10, 2, 18, 0);
        let a = Alarm { repeat: WEEKDAYS, ..alarm(7, 0, true) };
        assert_eq!(a.next_fire(fri, UTC), Some(utc(2026, 10, 5, 7, 0)));
        let sun_only = Alarm { repeat: 1, ..alarm(7, 0, true) };
        assert_eq!(sun_only.next_fire(fri, UTC), Some(utc(2026, 10, 4, 7, 0)));
        assert_eq!(repeat_text(WEEKDAYS), "Weekdays");
        assert_eq!(repeat_text(WEEKEND), "Weekends");
        assert_eq!(repeat_text(EVERY_DAY), "Every day");
        assert_eq!(repeat_text(0), "Never");
        assert_eq!(repeat_text(0b10), "Every Monday");
        assert_eq!(repeat_text(0b0010_1010), "Mon, Wed, Fri");
        assert_eq!(repeat_text(0b100_0010), "Mon, Sat");
    }

    #[test]
    fn local_alarm_time_holds_across_daylight_saving() {
        let a = alarm(7, 0, true);
        // London springs forward Sun 29 Mar 2026 01:00 UTC: 07:00 local becomes 06:00 UTC
        assert_eq!(a.next_fire(utc(2026, 3, 28, 8, 0), &london), Some(utc(2026, 3, 29, 6, 0)));
        assert_eq!(a.next_fire(utc(2026, 3, 28, 6, 59), &london), Some(utc(2026, 3, 28, 7, 0)));
        // and back on Sun 25 Oct 2026: 07:00 local is 07:00 UTC again
        assert_eq!(a.next_fire(utc(2026, 10, 24, 8, 0), &london), Some(utc(2026, 10, 25, 7, 0)));
        // 01:30 on the night the clocks go back happens twice: the first one rings
        let late = alarm(1, 30, true);
        assert_eq!(late.next_fire(utc(2026, 10, 24, 12, 0), &london), Some(utc(2026, 10, 25, 0, 30)));
        // 01:30 on the night the clocks go forward never happens: it rings at the jump
        assert_eq!(late.next_fire(utc(2026, 3, 28, 12, 0), &london), Some(utc(2026, 3, 29, 1, 30)));
        // a daily alarm keeps its wall time over the whole week
        let daily = Alarm { repeat: EVERY_DAY, ..alarm(7, 0, true) };
        let mut at = utc(2026, 3, 26, 12, 0);
        for expect in [utc(2026, 3, 27, 7, 0), utc(2026, 3, 28, 7, 0), utc(2026, 3, 29, 6, 0), utc(2026, 3, 30, 6, 0)] {
            at = daily.next_fire(at, &london).unwrap();
            assert_eq!(at, expect);
        }
    }

    #[test]
    fn fires_once_per_occurrence_one_shot_turns_off_repeat_stays() {
        let mut a = Alarms::default();
        a.add(alarm(7, 30, true));
        a.add(Alarm { repeat: EVERY_DAY, ..alarm(7, 30, true) });
        a.add(alarm(7, 30, false));
        let at = utc(2026, 10, 3, 7, 30);
        assert!(a.fire(at - 5, UTC).is_empty());
        assert_eq!(a.fire(at, UTC).len(), 2);
        assert!(a.fire(at + 1, UTC).is_empty(), "no repeat");
        assert!(a.fire(at + 30, UTC).is_empty());
        assert_eq!(a.list.iter().filter(|x| x.on).count(), 1, "the one-shot switched itself off");
        assert_eq!(a.fire(at + 86_400, UTC).len(), 1, "the repeating one rings tomorrow");
        let mut b = Alarms::default();
        b.add(alarm(9, 0, true));
        let nine_local = utc(2026, 10, 3, 9, 0) - 3600;
        let plus1: &dyn Fn(i64) -> i32 = &|_| 3600;
        b.start_at(nine_local - 5);
        assert_eq!(b.fire(nine_local, plus1).len(), 1, "local time, not UTC");
    }

    #[test]
    fn an_alarm_set_for_the_current_minute_waits_a_day() {
        let mut a = Alarms::default();
        let now = utc(2026, 10, 3, 23, 32) + 15;
        a.start_at(now);
        a.add(alarm(23, 32, true));
        assert!(a.fire(now + 10, UTC).is_empty());
        assert_eq!(a.next_due(now, UTC), Some(now - 15 + 86_400));
        assert_eq!(a.fire(now + 86_400, UTC).len(), 1);
    }

    #[test]
    fn manual_clock_changes_re_arm_and_missed_alarms_ring_within_grace() {
        let mut a = Alarms::default();
        a.add(alarm(8, 0, true));
        let eight = utc(2026, 10, 3, 8, 0);
        a.start_at(eight - 3600);
        // the clock is set back a day: nothing rings, the next one is still 08:00
        assert!(a.fire(eight - 86_400 + 100, UTC).is_empty());
        assert_eq!(a.next_due(eight - 86_400 + 100, UTC), Some(eight));
        // the shell was down at 08:00 and starts at 08:05: it rings, at 08:30 it would not
        let mut late = Alarms::default();
        late.add(alarm(8, 0, true));
        late.start_at(eight - 600);
        assert_eq!(late.fire(eight + 300, UTC).len(), 1);
        let mut later = Alarms::default();
        later.add(alarm(8, 0, true));
        later.start_at(eight - 600);
        assert!(later.fire(eight + 1800, UTC).is_empty());
    }

    #[test]
    fn snooze_rings_again_after_nine_minutes_and_dismiss_cancels_it() {
        let mut a = Alarms::default();
        let id = a.add(alarm(7, 30, true));
        let at = utc(2026, 10, 3, 7, 30);
        a.start_at(at - 1);
        assert_eq!(a.fire(at, UTC).len(), 1);
        assert!(a.snooze(id, at));
        assert_eq!(a.next_due(at, UTC), Some(at + 540), "the one-shot is off but the snooze is pending");
        assert!(a.fire(at + 539, UTC).is_empty());
        assert_eq!(a.fire(at + 540, UTC).len(), 1);
        assert!(a.snooze(id, at + 540));
        a.dismiss(id);
        assert_eq!(a.next_due(at + 541, UTC), None);
        a.list[0].snooze = false;
        assert!(!a.snooze(id, at));
    }

    #[test]
    fn skip_next_passes_one_occurrence_of_a_repeating_alarm() {
        let mut a = Alarms::default();
        let id = a.add(Alarm { repeat: EVERY_DAY, ..alarm(7, 0, true) });
        let now = utc(2026, 10, 3, 20, 0);
        a.skip_next(id, now, UTC);
        assert_eq!(a.next_due(now, UTC), Some(utc(2026, 10, 5, 7, 0)), "Sunday's is skipped");
        a.unskip(id);
        assert_eq!(a.next_due(now, UTC), Some(utc(2026, 10, 4, 7, 0)));
        let once = a.add(alarm(9, 0, true));
        a.skip_next(once, now, UTC);
        assert!(!a.list[a.index_of(once).unwrap()].on, "a one-shot alarm just turns off");
        a.skip_next(id, now, UTC);
        a.start_at(utc(2026, 10, 5, 6, 0));
        a.fire(utc(2026, 10, 5, 8, 0), UTC);
        assert_eq!(a.list[a.index_of(id).unwrap()].skip_day, None, "a past skip is forgotten");
    }

    #[test]
    fn replace_resets_snooze_and_resorts() {
        let mut a = Alarms::default();
        let id = a.add(alarm(7, 0, true));
        a.add(alarm(8, 0, true));
        a.snooze(id, 100);
        a.replace(Alarm { id, hour: 9, ..a.list[0].clone() });
        assert_eq!(a.list[1].id, id);
        assert_eq!(a.list[1].snoozed_until, None);
    }

    #[test]
    fn summary_and_eta_text() {
        assert_eq!(eta_text(30), "less than a minute");
        assert_eq!(eta_text(60 * 12), "12 min");
        assert_eq!(eta_text(3600 * 7 + 60 * 5), "7 hr 5 min");
        assert_eq!(eta_text(3600 * 7), "7 hr");
        assert_eq!(eta_text(86_400 * 2 + 3600 * 3), "2 d 3 hr");
        let mut a = Alarms::default();
        assert_eq!(a.summary(0, UTC), None);
        a.add(alarm(8, 0, true));
        assert_eq!(a.summary(utc(2026, 10, 3, 0, 55), UTC).as_deref(), Some("Alarm in 7 hr 5 min"));
    }
}
