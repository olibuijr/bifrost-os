//! Scheduled user notifications through the `notification-schedule` shell service (see
//! `springboard/src/shell/services/notification_schedule.rs`). A request has the same content as a user notification
//! (`usernotif`) and one of two triggers:
//!
//! - [`Trigger::Interval`]: after `seconds`, and every `seconds` until removed when `repeats` is set.
//! - [`Trigger::Calendar`]: at a local `hour:minute` on some weekdays (1 = Sunday .. 7 = Saturday; none = every day),
//!   daily or weekly when `repeats` is set, otherwise once.
//!
//! A request with an identifier already pending or delivered replaces it. Needs the `notifications` permission in the
//! manifest, like `usernotif`. Calls are asynchronous: the answer arrives as the service reply, which the app reads
//! with [`reply`] in `App::service_reply` (or `service::take_reply`).
//!
//! ```ignore
//! let s = notification_schedule::Schedule::calendar("standup", "Standup", 9, 15, &[2, 3, 4, 5, 6], true).body("Daily sync");
//! let id = notification_schedule::add(ctx, &s);
//! // later, in App::service_reply(ctx, request, result):
//! //   match notification_schedule::reply(result) { Reply::Added(id) => .., Reply::Failed(msg) => .., _ => {} }
//! ```
use crate::service;
use bifrost_core::json::Json;
use crate::Ctx;

/// The shell service name.
pub const SERVICE: &str = "notification-schedule";

/// The trigger of a scheduled request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Trigger {
    /// Time-interval trigger: after `seconds` (at least 1), and every `seconds` when `repeats`.
    Interval { seconds: u32, repeats: bool },
    /// Calendar trigger: local `hour` (0..=23) and `minute` (0..=59), on `weekdays` (1 = Sunday, 7 =
    /// Saturday; empty = every day), repeating or once.
    Calendar { hour: u8, minute: u8, weekdays: Vec<u8>, repeats: bool },
}

/// The shortest interval a repeating time-interval trigger may name, in seconds.
pub const MIN_REPEAT_SECS: u32 = 60;

const DAY_SECS: i64 = 86_400;

/// Why a trigger is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TriggerError {
    /// a time-interval trigger of 0 seconds never fires
    ZeroInterval,
    /// a repeating time-interval trigger shorter than [`MIN_REPEAT_SECS`]
    RepeatBelowMinimum,
    /// a calendar trigger whose hour is above 23 or whose minute is above 59
    InvalidTime,
    /// a calendar weekday outside 1 (Sunday) ..= 7 (Saturday)
    InvalidWeekday(u8),
}

impl Trigger {
    /// Checks the trigger the way the shell will read it; the constructors do not check, so a bad request is
    /// refused by the shell instead.
    pub fn validate(&self) -> Result<(), TriggerError> {
        match self {
            Trigger::Interval { seconds, repeats } => {
                if *seconds == 0 {
                    return Err(TriggerError::ZeroInterval);
                }
                if *repeats && *seconds < MIN_REPEAT_SECS {
                    return Err(TriggerError::RepeatBelowMinimum);
                }
                Ok(())
            }
            Trigger::Calendar { hour, minute, weekdays, .. } => {
                if *hour > 23 || *minute > 59 {
                    return Err(TriggerError::InvalidTime);
                }
                match weekdays.iter().find(|w| !(1..=7).contains(*w)) {
                    Some(w) => Err(TriggerError::InvalidWeekday(*w)),
                    None => Ok(()),
                }
            }
        }
    }

    /// The next fire time of the trigger as Unix seconds, for a request scheduled at `now_unix` in a zone `offset_secs`
    /// east of UTC. `None` when the trigger is invalid.
    ///
    /// - Interval: `now_unix + seconds`; the zone does not matter.
    /// - Calendar: the next local `hour:minute` strictly after `now_unix` on one of the weekdays (any day when the
    ///   list is empty). A time that is exactly now counts as passed.
    pub fn next_trigger_date(&self, now_unix: i64, offset_secs: i32) -> Option<i64> {
        self.validate().ok()?;
        match self {
            Trigger::Interval { seconds, .. } => Some(now_unix + i64::from(*seconds)),
            Trigger::Calendar { hour, minute, weekdays, .. } => {
                let offset = i64::from(offset_secs);
                let local_now = now_unix + offset;
                let today = local_now.div_euclid(DAY_SECS);
                let time_of_day = i64::from(*hour) * 3_600 + i64::from(*minute) * 60;
                // a weekday set repeats within seven days, so eight days always holds the next match
                (0..=7).find_map(|k| {
                    let day = today + k;
                    if !weekdays.is_empty() && !weekdays.contains(&weekday_of(day)) {
                        return None;
                    }
                    let local = day * DAY_SECS + time_of_day;
                    (local > local_now).then_some(local - offset)
                })
            }
        }
    }
}

/// The calendar weekday (1 = Sunday .. 7 = Saturday) of a day counted from 1970-01-01, which was a Thursday.
fn weekday_of(day: i64) -> u8 {
    ((day + 4).rem_euclid(7) + 1) as u8
}

/// A request under construction: content plus trigger.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Schedule {
    pub id: String,
    pub title: String,
    pub subtitle: String,
    pub body: String,
    pub thread: String,
    /// a category registered with `usernotif::set_categories`; empty for none
    pub category: String,
    /// the Dock badge when the request is delivered (0 clears it)
    pub badge: Option<u32>,
    pub trigger: Trigger,
}

impl Schedule {
    /// A request on a time-interval trigger.
    pub fn interval(id: &str, title: &str, seconds: u32, repeats: bool) -> Self {
        Self::with(id, title, Trigger::Interval { seconds, repeats })
    }

    /// A request on a calendar trigger; see [`Trigger::Calendar`].
    pub fn calendar(id: &str, title: &str, hour: u8, minute: u8, weekdays: &[u8], repeats: bool) -> Self {
        Self::with(id, title, Trigger::Calendar { hour, minute, weekdays: weekdays.to_vec(), repeats })
    }

    /// A time-interval request that is refused when its trigger is invalid (see [`Trigger::validate`]), such as a
    /// repeating interval under [`MIN_REPEAT_SECS`]. [`Schedule::interval`] does not check.
    pub fn checked_interval(id: &str, title: &str, seconds: u32, repeats: bool) -> Result<Self, TriggerError> {
        let s = Self::interval(id, title, seconds, repeats);
        s.validate().map(|()| s)
    }

    /// A calendar request that is refused when its trigger is invalid; [`Schedule::calendar`] does not check.
    pub fn checked_calendar(id: &str, title: &str, hour: u8, minute: u8, weekdays: &[u8], repeats: bool) -> Result<Self, TriggerError> {
        let s = Self::calendar(id, title, hour, minute, weekdays, repeats);
        s.validate().map(|()| s)
    }

    /// Checks the trigger of this request; see [`Trigger::validate`].
    pub fn validate(&self) -> Result<(), TriggerError> {
        self.trigger.validate()
    }

    fn with(id: &str, title: &str, trigger: Trigger) -> Self {
        Schedule { id: id.into(), title: title.into(), subtitle: String::new(), body: String::new(), thread: String::new(), category: String::new(), badge: None, trigger }
    }

    pub fn subtitle(mut self, v: &str) -> Self {
        self.subtitle = v.into();
        self
    }
    pub fn body(mut self, v: &str) -> Self {
        self.body = v.into();
        self
    }
    pub fn thread(mut self, v: &str) -> Self {
        self.thread = v.into();
        self
    }
    pub fn category(mut self, v: &str) -> Self {
        self.category = v.into();
        self
    }
    pub fn badge(mut self, n: u32) -> Self {
        self.badge = Some(n);
        self
    }

    /// The JSON the service reads.
    pub fn to_json(&self) -> Json {
        let mut o = vec![
            ("id".to_string(), Json::Str(self.id.clone())),
            ("title".into(), Json::Str(self.title.clone())),
            ("subtitle".into(), Json::Str(self.subtitle.clone())),
            ("body".into(), Json::Str(self.body.clone())),
            ("thread".into(), Json::Str(self.thread.clone())),
            ("category".into(), Json::Str(self.category.clone())),
        ];
        if let Some(n) = self.badge {
            o.push(("badge".into(), Json::Num(f64::from(n))));
        }
        o.push(("trigger".into(), trigger_json(&self.trigger)));
        Json::Obj(o)
    }
}

fn trigger_json(t: &Trigger) -> Json {
    match t {
        Trigger::Interval { seconds, repeats } => Json::Obj(vec![
            ("kind".into(), Json::Str("interval".into())),
            ("seconds".into(), Json::Num(f64::from(*seconds))),
            ("repeats".into(), Json::Bool(*repeats)),
        ]),
        Trigger::Calendar { hour, minute, weekdays, repeats } => Json::Obj(vec![
            ("kind".into(), Json::Str("calendar".into())),
            ("hour".into(), Json::Num(f64::from(*hour))),
            ("minute".into(), Json::Num(f64::from(*minute))),
            ("weekdays".into(), Json::Arr(weekdays.iter().map(|w| Json::Num(f64::from(*w))).collect())),
            ("repeats".into(), Json::Bool(*repeats)),
        ]),
    }
}

/// Schedules `s` (replacing a pending or delivered request with the same identifier). Returns the service request id.
pub fn add(ctx: &Ctx, s: &Schedule) -> u32 {
    service::call(ctx, SERVICE, "add", &s.to_json())
}

/// Removes the pending requests named by `ids`; no ids removes all of the app's pending requests.
pub fn remove(ctx: &Ctx, ids: &[&str]) -> u32 {
    let list = ids.iter().map(|i| Json::Str(i.to_string())).collect();
    service::call(ctx, SERVICE, "remove", &Json::Obj(vec![("ids".into(), Json::Arr(list))]))
}

/// The identifiers of the pending requests of both kinds (answer as [`Reply::Pending`]).
pub fn pending(ctx: &Ctx) -> u32 {
    service::call(ctx, SERVICE, "pending", &Json::Null)
}

/// What a service reply to one of this module's calls says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reply {
    /// `add`: the identifier that was scheduled
    Added(String),
    /// `remove`: how many pending requests went
    Removed(u32),
    /// `pending`: the pending identifiers
    Pending(Vec<String>),
    /// the shell refused the call; the message says why
    Failed(String),
}

/// Reads a service reply to one of this module's calls (pass the `result` of `App::service_reply`).
pub fn reply(result: &Result<Json, String>) -> Reply {
    let json = match result {
        Ok(json) => json,
        Err(msg) => return Reply::Failed(msg.clone()),
    };
    if let Some(id) = json.get("added").and_then(Json::as_str) {
        return Reply::Added(id.to_string());
    }
    if let Some(n) = json.get("removed").and_then(Json::as_u64) {
        return Reply::Removed(n as u32);
    }
    if let Some(list) = json.get("pending").and_then(Json::as_array) {
        return Reply::Pending(list.iter().filter_map(|x| x.as_str().map(str::to_string)).collect());
    }
    Reply::Failed("unexpected reply from notification-schedule".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_interval_request_encodes_its_trigger() {
        let j = Schedule::interval("tick", "Tick", 60, true).body("B").to_json();
        assert_eq!(j.get("id").and_then(Json::as_str), Some("tick"));
        assert_eq!(j.get("body").and_then(Json::as_str), Some("B"));
        assert!(j.get("badge").is_none(), "no badge key unless one is set");
        let t = j.get("trigger").unwrap();
        assert_eq!(t.get("kind").and_then(Json::as_str), Some("interval"));
        assert_eq!(t.get("seconds").and_then(Json::number), Some(60.0));
        assert_eq!(t.get("repeats"), Some(&Json::Bool(true)));
    }

    #[test]
    fn a_calendar_request_encodes_time_weekdays_and_badge() {
        let s = Schedule::calendar("standup", "Standup", 9, 15, &[2, 6], false).badge(3);
        let j = s.to_json();
        assert_eq!(j.get("badge").and_then(Json::number), Some(3.0));
        let t = j.get("trigger").unwrap();
        assert_eq!(t.get("kind").and_then(Json::as_str), Some("calendar"));
        assert_eq!(t.get("hour").and_then(Json::number), Some(9.0));
        assert_eq!(t.get("minute").and_then(Json::number), Some(15.0));
        assert_eq!(t.get("weekdays"), Some(&Json::Arr(vec![Json::Num(2.0), Json::Num(6.0)])));
        assert_eq!(t.get("repeats"), Some(&Json::Bool(false)));
    }

    #[test]
    fn the_builder_sets_content_fields() {
        let s = Schedule::interval("a", "T", 5, false).subtitle("S").thread("th").category("C");
        assert_eq!((s.subtitle.as_str(), s.thread.as_str(), s.category.as_str()), ("S", "th", "C"));
        assert_eq!(s.trigger, Trigger::Interval { seconds: 5, repeats: false });
    }

    // 1970-01-01 is a Thursday; day 0 of the epoch is at 00:00 UTC.
    const DAY: i64 = 86_400;
    const HOUR: i64 = 3_600;

    #[test]
    fn an_interval_trigger_fires_its_interval_after_now() {
        let t = Trigger::Interval { seconds: 90, repeats: false };
        assert_eq!(t.next_trigger_date(1_000, 0), Some(1_090));
        assert_eq!(t.next_trigger_date(1_000, 7_200), Some(1_090), "an interval ignores the zone offset");
    }

    #[test]
    fn a_repeating_interval_below_a_minute_is_refused() {
        let repeating = Schedule::checked_interval("tick", "Tick", 30, true);
        assert_eq!(repeating, Err(TriggerError::RepeatBelowMinimum));
        assert_eq!(Schedule::checked_interval("tick", "Tick", 60, true).map(|s| s.trigger), Ok(Trigger::Interval { seconds: 60, repeats: true }));
        assert!(Schedule::checked_interval("once", "Once", 5, false).is_ok(), "the minimum applies to repeats only");
    }

    #[test]
    fn a_zero_interval_is_refused() {
        assert_eq!(Trigger::Interval { seconds: 0, repeats: false }.validate(), Err(TriggerError::ZeroInterval));
        assert_eq!(Trigger::Interval { seconds: 0, repeats: false }.next_trigger_date(0, 0), None);
    }

    #[test]
    fn a_calendar_time_out_of_range_is_refused() {
        assert_eq!(Trigger::Calendar { hour: 24, minute: 0, weekdays: vec![], repeats: true }.validate(), Err(TriggerError::InvalidTime));
        assert_eq!(Trigger::Calendar { hour: 1, minute: 60, weekdays: vec![], repeats: true }.validate(), Err(TriggerError::InvalidTime));
        assert_eq!(Trigger::Calendar { hour: 1, minute: 1, weekdays: vec![0], repeats: true }.validate(), Err(TriggerError::InvalidWeekday(0)));
        assert_eq!(Trigger::Calendar { hour: 1, minute: 1, weekdays: vec![8], repeats: true }.validate(), Err(TriggerError::InvalidWeekday(8)));
        assert_eq!(Trigger::Calendar { hour: 23, minute: 59, weekdays: vec![1, 7], repeats: true }.validate(), Ok(()));
    }

    #[test]
    fn a_daily_calendar_time_later_today_is_today() {
        // 08:00 UTC, the time is 09:15 every day
        let t = Trigger::Calendar { hour: 9, minute: 15, weekdays: vec![], repeats: true };
        assert_eq!(t.next_trigger_date(8 * HOUR, 0), Some(9 * HOUR + 15 * 60));
    }

    #[test]
    fn a_daily_calendar_time_already_passed_is_tomorrow() {
        // 10:00 UTC is past 09:15, so the next one is tomorrow at 09:15
        let t = Trigger::Calendar { hour: 9, minute: 15, weekdays: vec![], repeats: true };
        assert_eq!(t.next_trigger_date(10 * HOUR, 0), Some(DAY + 9 * HOUR + 15 * 60));
    }

    #[test]
    fn the_exact_trigger_moment_counts_as_passed() {
        let t = Trigger::Calendar { hour: 9, minute: 0, weekdays: vec![], repeats: true };
        assert_eq!(t.next_trigger_date(9 * HOUR, 0), Some(DAY + 9 * HOUR));
    }

    #[test]
    fn a_weekday_calendar_picks_the_next_matching_weekday() {
        // Thursday 08:00 UTC; Monday is weekday 2 and is 1970-01-05 (day 4)
        let t = Trigger::Calendar { hour: 9, minute: 0, weekdays: vec![2], repeats: true };
        assert_eq!(t.next_trigger_date(8 * HOUR, 0), Some(4 * DAY + 9 * HOUR));
    }

    #[test]
    fn a_weekday_set_picks_the_nearest_day_in_the_set() {
        // Thursday 08:00 UTC; Friday (6) is the nearest of {Friday, Sunday}
        let t = Trigger::Calendar { hour: 9, minute: 0, weekdays: vec![1, 6], repeats: true };
        assert_eq!(t.next_trigger_date(8 * HOUR, 0), Some(DAY + 9 * HOUR));
    }

    #[test]
    fn a_weekday_that_is_today_but_passed_waits_a_week() {
        // Thursday (5) 10:00 UTC, the time 09:00 on Thursdays is past: next Thursday
        let t = Trigger::Calendar { hour: 9, minute: 0, weekdays: vec![5], repeats: true };
        assert_eq!(t.next_trigger_date(10 * HOUR, 0), Some(7 * DAY + 9 * HOUR));
    }

    #[test]
    fn the_zone_offset_moves_the_fire_time() {
        // 00:00 UTC is 01:00 at UTC+1; 09:00 local is 08:00 UTC
        let t = Trigger::Calendar { hour: 9, minute: 0, weekdays: vec![], repeats: true };
        assert_eq!(t.next_trigger_date(0, HOUR as i32), Some(8 * HOUR));
        // at UTC-5, 00:00 UTC is the previous evening, so 09:00 local today is 14:00 UTC
        assert_eq!(t.next_trigger_date(0, -5 * 3_600), Some(14 * HOUR));
    }

    #[test]
    fn a_non_repeating_calendar_fires_at_its_next_occurrence() {
        // a one-shot has no date of its own: its next date is the next matching time, as the shell delivers it
        let t = Trigger::Calendar { hour: 8, minute: 0, weekdays: vec![], repeats: false };
        assert_eq!(t.next_trigger_date(9 * HOUR, 0), Some(DAY + 8 * HOUR));
    }

    #[test]
    fn a_schedule_reports_its_trigger_validity() {
        let bad = Schedule::calendar("x", "X", 25, 0, &[], false);
        assert_eq!(bad.validate(), Err(TriggerError::InvalidTime));
        assert_eq!(Schedule::interval("y", "Y", 5, false).validate(), Ok(()));
    }

    #[test]
    fn replies_are_read_by_their_key() {
        assert_eq!(reply(&Ok(Json::Obj(vec![("added".into(), Json::Str("a".into()))]))), Reply::Added("a".into()));
        assert_eq!(reply(&Ok(Json::Obj(vec![("removed".into(), Json::Num(2.0))]))), Reply::Removed(2));
        let pending = Json::Obj(vec![("pending".into(), Json::Arr(vec![Json::Str("x".into())]))]);
        assert_eq!(reply(&Ok(pending)), Reply::Pending(vec!["x".into()]));
        assert_eq!(reply(&Err("no permission".into())), Reply::Failed("no permission".into()));
        assert!(matches!(reply(&Ok(Json::Null)), Reply::Failed(_)));
    }
}
