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
