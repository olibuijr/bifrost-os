//! The `notification-schedule` service: scheduled user notifications with two kinds of trigger.
//!
//! - An interval trigger (`{"kind":"interval","seconds":N,"repeats":bool}`) is handed to the registry that already
//!   keeps the time-interval requests of the wire path (`usernotif::Registry`, with its repeat grid).
//! - A calendar trigger (`{"kind":"calendar","hour":H,"minute":M,"weekdays":[1..7],"repeats":bool}`) is kept here. The
//!   next time comes from the alarm model (`clock::alarm::Alarm`, local civil time, so it follows the zone's offset).
//!   Weekdays are numbered from 1 = Sunday; an empty list means every day. `tick` delivers it through the same
//!   path as any due request (`Shell::usernotif_fire`).
//!
//! Both kinds form one pending list per app, and a request with an identifier replaces the earlier pending request of
//! either kind. Delivery is the normal notification UI, so the `notifications` permission is required for every method.
//! Pending calendar requests are not kept across a shell restart (the interval ones are not either).
//!
//! Methods (JSON params; the app is the caller):
//! - `add`: `{ "id", "title", "subtitle"?, "body"?, "thread"?, "category"?, "badge"?, "trigger" }`. Answers `{"added": id}`.
//! - `remove`: `{ "ids": [..] }`; no ids (or an empty list) removes all pending. Answers `{"removed": n}`.
//! - `pending`: answers `{"pending": [ids]}`, the pending identifiers of both kinds.
use super::{Event, Shell, SystemService};
use crate::shell::clock::alarm::Alarm;
use crate::shell::clock::offset_fn;
use crate::shell::usernotif::{self, Added, MAX_PENDING};
use bifrost_core::json::Json;
use bifrost_proto::rich::{MAX_NOTIF_ID, MAX_NOTIF_TEXT};
use bifrost_proto::{NotifRequest, NotifStatus};

/// The longest interval one request may name (about 49 days, the millisecond delay of the wire message is a `u32`).
const MAX_INTERVAL_SECS: f64 = 4_294_967.0;

/// A calendar-trigger request waiting for its next time.
#[derive(Clone, Debug, PartialEq)]
struct Calendar {
    app: String,
    req: NotifRequest,
    alarm: Alarm,
    repeats: bool,
    /// Unix time (s) of the next delivery
    due: i64,
}

#[derive(Default)]
pub struct NotificationSchedule {
    calendars: Vec<Calendar>,
}

/// A trigger as the request names it.
#[derive(Clone, Debug, PartialEq)]
enum Trigger {
    Interval { ms: u32, repeats: bool },
    Calendar { hour: u32, minute: u32, weekdays: u8, repeats: bool },
}

impl SystemService for NotificationSchedule {
    fn name(&self) -> &'static str {
        "notification-schedule"
    }

    fn call(&mut self, shell: &mut Shell, app_id: &str, method: &str, params: &Json, t: f64) -> Result<Json, String> {
        if !usernotif::permitted(app_id) {
            return Err("the app has no notifications permission".into());
        }
        match method {
            "add" => self.add(shell, app_id, params, t),
            "remove" => self.remove(shell, app_id, params),
            "pending" => Ok(Json::Obj(vec![("pending".into(), Json::Arr(self.pending(shell, app_id).into_iter().map(Json::Str).collect()))])),
            _ => Err(format!("notification-schedule has no method {method}")),
        }
    }

    fn tick(&mut self, shell: &mut Shell, t: f64) -> Vec<Event> {
        if self.calendars.is_empty() {
            return Vec::new();
        }
        let now = shell.wall.now(t);
        let offset = offset_fn(None, shell.wall.offset);
        let mut fired = Vec::new();
        let mut kept = Vec::with_capacity(self.calendars.len());
        for mut c in std::mem::take(&mut self.calendars) {
            if c.due > now {
                kept.push(c);
                continue;
            }
            fired.push((c.app.clone(), c.req.clone()));
            if c.repeats {
                // the first time after now: a shell that slept past several times delivers once
                if let Some(next) = c.alarm.regular_fire(now, &offset) {
                    c.due = next;
                    kept.push(c);
                }
            }
        }
        self.calendars = kept;
        for (app, req) in fired {
            shell.usernotif_fire(&app, req, t);
        }
        Vec::new()
    }
}

impl NotificationSchedule {
    fn add(&mut self, shell: &mut Shell, app: &str, params: &Json, t: f64) -> Result<Json, String> {
        let req = request(params)?;
        let trigger = trigger(params.get("trigger").ok_or("trigger is required")?)?;
        if let Some(status) = shell.usernotif.check(app, &req) {
            return Err(refusal(status));
        }
        let id = req.id.clone();
        // the identifier replaces the pending request of either kind
        self.calendars.retain(|c| !(c.app == app && c.req.id == id));
        shell.usernotif.remove(app, false, std::slice::from_ref(&id));
        if self.pending(shell, app).len() >= MAX_PENDING {
            return Err(refusal(NotifStatus::Full));
        }
        match trigger {
            Trigger::Interval { ms, repeats } => match shell.usernotif.add(app, NotifRequest { delay_ms: ms, repeats, ..req }, t) {
                Added::Scheduled => {}
                Added::Refused(status) => return Err(refusal(status)),
                Added::Deliver { .. } => return Err("an interval trigger must be later than now".into()),
            },
            Trigger::Calendar { hour, minute, weekdays, repeats } => {
                let now = shell.wall.now(t);
                let offset = offset_fn(None, shell.wall.offset);
                let alarm = Alarm { repeat: weekdays, ..Alarm::new(hour, minute) };
                let due = alarm.regular_fire(now, &offset).ok_or("no date matches the calendar trigger")?;
                self.calendars.push(Calendar { app: app.to_string(), req, alarm, repeats, due });
            }
        }
        Ok(Json::Obj(vec![("added".into(), Json::Str(id))]))
    }

    fn remove(&mut self, shell: &mut Shell, app: &str, params: &Json) -> Result<Json, String> {
        let ids = string_list(params.get("ids"))?;
        let hit = |id: &str| ids.is_empty() || ids.iter().any(|i| i == id);
        let before = self.pending(shell, app).len();
        self.calendars.retain(|c| !(c.app == app && hit(&c.req.id)));
        shell.usernotif.remove(app, false, &ids);
        let removed = before.saturating_sub(self.pending(shell, app).len());
        Ok(Json::Obj(vec![("removed".into(), Json::Num(removed as f64))]))
    }

    /// The pending identifiers of both kinds: the interval ones first, then the calendar ones, in the order they were added.
    fn pending(&self, shell: &Shell, app: &str) -> Vec<String> {
        let mut ids = shell.usernotif.list(app, false);
        ids.extend(self.calendars.iter().filter(|c| c.app == app).map(|c| c.req.id.clone()));
        ids
    }
}

/// The text of a refusal, as the request status names it.
fn refusal(status: NotifStatus) -> String {
    match status {
        NotifStatus::Ok => "ok".into(),
        NotifStatus::NotPermitted => "the app has no notifications permission".into(),
        NotifStatus::Param => "the notification request is not valid".into(),
        NotifStatus::Full => "too many pending notification requests".into(),
    }
}

/// The content of an `add` request, checked against the wire limits.
fn request(params: &Json) -> Result<NotifRequest, String> {
    let req = NotifRequest {
        id: text(params, "id", MAX_NOTIF_ID)?,
        title: text(params, "title", MAX_NOTIF_TEXT)?,
        subtitle: text(params, "subtitle", MAX_NOTIF_TEXT)?,
        body: text(params, "body", MAX_NOTIF_TEXT * 4)?,
        thread: text(params, "thread", MAX_NOTIF_ID)?,
        category: text(params, "category", MAX_NOTIF_ID)?,
        badge: match params.get("badge") {
            None | Some(Json::Null) => None,
            Some(_) => Some(integer(params, "badge", f64::from(u32::MAX))? as u32),
        },
        ..Default::default()
    };
    Ok(req)
}

/// The trigger of an `add` request.
fn trigger(t: &Json) -> Result<Trigger, String> {
    let repeats = match t.get("repeats") {
        None | Some(Json::Null) => false,
        Some(Json::Bool(b)) => *b,
        Some(_) => return Err("trigger.repeats must be true or false".into()),
    };
    match t.str_field("kind").as_deref() {
        Some("interval") => {
            let secs = t.get("seconds").and_then(Json::number).ok_or("trigger.seconds must be a number")?;
            if secs.fract() != 0.0 || secs < 1.0 || secs > MAX_INTERVAL_SECS {
                return Err(format!("trigger.seconds must be a whole number from 1 to {}", MAX_INTERVAL_SECS as u32));
            }
            if repeats && secs < 60.0 {
                return Err("trigger.seconds must be at least 60 for a repeating trigger".into());
            }
            Ok(Trigger::Interval { ms: secs as u32 * 1000, repeats })
        }
        Some("calendar") => {
            let hour = integer(t, "hour", 23.0)?;
            let minute = integer(t, "minute", 59.0)?;
            let mut weekdays = 0u8;
            match t.get("weekdays") {
                None | Some(Json::Null) => {}
                Some(Json::Arr(list)) => {
                    for d in list {
                        let w = d.number().filter(|w| w.fract() == 0.0 && (1.0..=7.0).contains(w)).ok_or("trigger.weekdays must be numbers from 1 (Sunday) to 7 (Saturday)")?;
                        weekdays |= 1 << (w as u32 - 1);
                    }
                }
                Some(_) => return Err("trigger.weekdays must be a list".into()),
            }
            Ok(Trigger::Calendar { hour, minute, weekdays, repeats })
        }
        _ => Err("trigger.kind must be interval or calendar".into()),
    }
}

/// An optional text field: missing is empty, a non-text value or one over `max` bytes is refused.
fn text(j: &Json, key: &str, max: usize) -> Result<String, String> {
    match j.get(key) {
        None | Some(Json::Null) => Ok(String::new()),
        Some(Json::Str(s)) if s.len() <= max => Ok(s.clone()),
        Some(Json::Str(_)) => Err(format!("{key} is longer than {max} bytes")),
        Some(_) => Err(format!("{key} must be text")),
    }
}

/// A whole number from 0 to `max`.
fn integer(j: &Json, key: &str, max: f64) -> Result<u32, String> {
    let v = j.get(key).and_then(Json::number).ok_or_else(|| format!("{key} must be a number"))?;
    if v.fract() != 0.0 || v < 0.0 || v > max {
        return Err(format!("{key} must be a whole number from 0 to {}", max as u64));
    }
    Ok(v as u32)
}

/// A list of identifiers; missing or null is empty.
fn string_list(v: Option<&Json>) -> Result<Vec<String>, String> {
    match v {
        None | Some(Json::Null) => Ok(Vec::new()),
        Some(Json::Arr(list)) => list.iter().map(|x| x.as_str().map(str::to_string).ok_or_else(|| "ids must be text".to_string())).collect(),
        Some(_) => Err("ids must be a list".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json(text: &str) -> Json {
        bifrost_core::json::parse(text).unwrap()
    }

    #[test]
    fn an_interval_trigger_is_a_whole_number_of_seconds() {
        assert_eq!(trigger(&json(r#"{"kind":"interval","seconds":90,"repeats":true}"#)), Ok(Trigger::Interval { ms: 90_000, repeats: true }));
        assert_eq!(trigger(&json(r#"{"kind":"interval","seconds":5}"#)), Ok(Trigger::Interval { ms: 5000, repeats: false }));
        assert!(trigger(&json(r#"{"kind":"interval","seconds":0}"#)).is_err());
        assert!(trigger(&json(r#"{"kind":"interval","seconds":30,"repeats":true}"#)).is_err(), "a repeating interval is at least 60 s");
        assert!(trigger(&json(r#"{"kind":"interval","seconds":60,"repeats":true}"#)).is_ok());
        assert!(trigger(&json(r#"{"kind":"interval","seconds":1.5}"#)).is_err());
        assert!(trigger(&json(r#"{"kind":"interval"}"#)).is_err());
    }

    #[test]
    fn a_calendar_trigger_maps_weekdays_to_the_alarm_mask() {
        // 1 = Sunday is bit 0, as in the alarm model; Monday and Friday are bits 1 and 5
        assert_eq!(
            trigger(&json(r#"{"kind":"calendar","hour":8,"minute":30,"weekdays":[2,6,2],"repeats":true}"#)),
            Ok(Trigger::Calendar { hour: 8, minute: 30, weekdays: 0b10_0010, repeats: true })
        );
        assert_eq!(trigger(&json(r#"{"kind":"calendar","hour":7,"minute":0}"#)), Ok(Trigger::Calendar { hour: 7, minute: 0, weekdays: 0, repeats: false }));
        assert!(trigger(&json(r#"{"kind":"calendar","hour":24,"minute":0}"#)).is_err());
        assert!(trigger(&json(r#"{"kind":"calendar","hour":1,"minute":60}"#)).is_err());
        assert!(trigger(&json(r#"{"kind":"calendar","hour":1,"minute":1,"weekdays":[0]}"#)).is_err());
        assert!(trigger(&json(r#"{"kind":"calendar","hour":1,"minute":1,"weekdays":[8]}"#)).is_err());
        assert!(trigger(&json(r#"{"kind":"weekly"}"#)).is_err());
    }

    #[test]
    fn a_daily_calendar_time_is_the_next_one_strictly_after_now() {
        let offset = |_: i64| 0;
        // 1970-01-01 08:00 UTC, a Thursday, for 08:30 every day
        let daily = Alarm::new(8, 30);
        assert_eq!(daily.regular_fire(28_800, &offset), Some(30_600));
        // at 08:30 itself the next one is tomorrow
        assert_eq!(daily.regular_fire(30_600, &offset), Some(30_600 + 86_400));
    }

    #[test]
    fn a_weekday_calendar_time_skips_the_other_days() {
        let offset = |_: i64| 0;
        // Mondays only (bit 1); Thursday 1970-01-01 08:00 leads to Monday 1970-01-05 08:30
        let monday = Alarm { repeat: 0b10, ..Alarm::new(8, 30) };
        assert_eq!(monday.regular_fire(28_800, &offset), Some(4 * 86_400 + 30_600));
    }

    #[test]
    fn requests_are_checked_against_the_wire_limits() {
        let ok = request(&json(r#"{"id":"a","title":"T","body":"B","badge":3}"#)).unwrap();
        assert_eq!((ok.id.as_str(), ok.title.as_str(), ok.body.as_str(), ok.badge), ("a", "T", "B", Some(3)));
        assert!(request(&json(r#"{"title":"T"}"#)).is_ok(), "the empty identifier is refused by the registry, not here");
        let long = format!(r#"{{"id":"{}"}}"#, "x".repeat(MAX_NOTIF_ID + 1));
        assert!(request(&json(&long)).is_err());
        assert!(request(&json(r#"{"id":"a","title":3}"#)).is_err());
    }

    #[test]
    fn remove_ids_are_text_or_empty() {
        assert_eq!(string_list(None), Ok(Vec::new()));
        assert_eq!(string_list(Some(&json(r#"["a","b"]"#))), Ok(vec!["a".to_string(), "b".to_string()]));
        assert!(string_list(Some(&json(r#"[1]"#))).is_err());
        assert!(string_list(Some(&json(r#""a""#))).is_err());
    }
}
