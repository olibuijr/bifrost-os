//! The `widgets_timeline` service: apps give the shell widget timelines (dated entries with a reload policy) and start
//! live activities. The shell keeps them, shows the entry on screen in Notification Centre's widget area and asks the
//! owning app for a new timeline when its reload date passes (`widgets.reload` event). Models: `bifrost_ui::timeline`
//! (entries and reload policy) and `bifrost_ui::live_activity` (the activity life cycle).
//!
//! Methods (JSON in, JSON out):
//! - `reload {kind, entries:[{date,title,text}], policy:"at_end"|"never"|"after", reload_at?}` replaces the app's
//!   timeline for `kind`; returns the entry count.
//! - `remove {kind}` drops it; returns whether one was there.
//! - `current {kind}` returns `{date,title,text}` of the entry on screen now, or null.
//! - `start_activity {title,text,stale_at?,start_at?}` returns the activity id.
//! - `update_activity {id,title,text,stale_at?}`, `end_activity {id,title?,text?,dismissal:"default"|"immediate"|"after",after?}`.
//! - `activities {}` lists the app's activities on screen: `[{id,state,title,text}]`.
//!
//! The store is process-wide because Notification Centre draws from it outside a service call; the logic lives in
//! [`Store`], which the tests drive directly.
use super::{Event, Shell, SystemService};
use bifrost_ui::live_activity::{Content, Dismissal, Error, Registry, State};
use bifrost_ui::timeline::{Entry, ReloadPolicy, Timeline};
use bifrost_core::json::Json;
use std::sync::Mutex;

/// The longest kind id an app may register.
const MAX_KIND: usize = 64;
/// The most entries one timeline may hold.
const MAX_ENTRIES: usize = 64;

/// The text a widget entry or an activity shows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Text {
    pub title: String,
    pub text: String,
}

impl Text {
    fn of(j: &Json) -> Self {
        Text { title: j.str_field("title").unwrap_or_default(), text: j.str_field("text").unwrap_or_default() }
    }
}

/// One line the Notification Centre shows for a widget or an activity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub title: String,
    pub text: String,
}

struct Widget {
    app: String,
    kind: String,
    timeline: Timeline<Text>,
    /// the reload event was already sent for this timeline
    requested: bool,
}

/// Every app's widget timelines and live activities.
pub struct Store {
    widgets: Vec<Widget>,
    activities: Registry<Text>,
    /// the app that started each activity, by id
    owners: Vec<(u32, String)>,
}

impl Default for Store {
    fn default() -> Self {
        Store { widgets: Vec::new(), activities: Registry::new(true), owners: Vec::new() }
    }
}

static STORE: Mutex<Option<Store>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut Store) -> R) -> R {
    let mut g = STORE.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(Store::default))
}

/// The rows Notification Centre shows at `now` (unix seconds): each widget's entry on screen, then every live
/// activity that is still on screen.
pub fn rows(now: i64) -> Vec<Row> {
    with(|s| s.rows(now))
}

fn error_text(e: Error) -> String {
    match e {
        Error::Disabled => "live activities are off".into(),
        Error::NotFound => "no such activity".into(),
        Error::NotLive => "the activity has ended".into(),
    }
}

fn state_name(s: State) -> &'static str {
    match s {
        State::Pending => "pending",
        State::Active => "active",
        State::Stale => "stale",
        State::Ended => "ended",
        State::Dismissed => "dismissed",
    }
}

fn unix(now: i64) -> u64 {
    now.max(0) as u64
}

impl Store {
    /// Replaces the timeline `kind` of `app`. The kind must be 1 to 64 bytes and the entries at most 64.
    pub fn reload(&mut self, app: &str, kind: &str, entries: Vec<Entry<Text>>, policy: ReloadPolicy) -> Result<usize, String> {
        if kind.is_empty() || kind.len() > MAX_KIND {
            return Err(format!("a widget kind is 1 to {MAX_KIND} bytes"));
        }
        if entries.len() > MAX_ENTRIES {
            return Err(format!("a timeline holds at most {MAX_ENTRIES} entries"));
        }
        let n = entries.len();
        self.widgets.retain(|w| !(w.app == app && w.kind == kind));
        self.widgets.push(Widget { app: app.into(), kind: kind.into(), timeline: Timeline::new(entries, policy), requested: false });
        Ok(n)
    }

    pub fn remove(&mut self, app: &str, kind: &str) -> bool {
        let before = self.widgets.len();
        self.widgets.retain(|w| !(w.app == app && w.kind == kind));
        self.widgets.len() != before
    }

    /// The entry on screen for `kind` at `now`, if any.
    pub fn current(&self, app: &str, kind: &str, now: i64) -> Option<Entry<Text>> {
        let w = self.widgets.iter().find(|w| w.app == app && w.kind == kind)?;
        w.timeline.entry_at(now as f64).cloned()
    }

    /// The timelines whose reload date has passed at `now`. Each is returned once until its timeline is replaced.
    pub fn due(&mut self, now: i64) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for w in self.widgets.iter_mut() {
            // the date itself, not `next_reload(now)`, which clamps to now and would always be due
            let at = w.timeline.next_reload(f64::NEG_INFINITY);
            if !w.requested && at.map_or(false, |d| d <= now as f64) {
                w.requested = true;
                out.push((w.app.clone(), w.kind.clone()));
            }
        }
        // drop activities the shell has removed, so the owner list stays short
        let now_u = unix(now);
        let activities = &self.activities;
        self.owners.retain(|(id, _)| activities.state(*id, now_u).map_or(false, |s| s != State::Dismissed));
        out
    }

    pub fn rows(&self, now: i64) -> Vec<Row> {
        let mut rows: Vec<Row> = self
            .widgets
            .iter()
            .filter_map(|w| w.timeline.entry_at(now as f64))
            .map(|e| Row { title: e.value.title.clone(), text: e.value.text.clone() })
            .collect();
        for (id, _) in &self.owners {
            if let Some(c) = self.activities.content(*id, unix(now)) {
                rows.push(Row { title: c.state.title.clone(), text: c.state.text.clone() });
            }
        }
        rows
    }

    fn owned(&self, app: &str, id: u32, now: i64) -> Result<(), String> {
        let mine = self.owners.iter().any(|(i, a)| *i == id && a == app);
        if !mine || self.activities.state(id, unix(now)).map_or(true, |s| s == State::Dismissed) {
            return Err(format!("no activity {id}"));
        }
        Ok(())
    }

    /// Runs one service method for `app` at `now` (unix seconds).
    pub fn call(&mut self, app: &str, method: &str, p: &Json, now: i64) -> Result<Json, String> {
        match method {
            "reload" => {
                let kind = p.str_field("kind").ok_or("reload needs a kind")?;
                let policy = match p.get("policy").and_then(Json::as_str).unwrap_or("at_end") {
                    "at_end" => ReloadPolicy::AtEnd,
                    "never" => ReloadPolicy::Never,
                    "after" => ReloadPolicy::After(p.get("reload_at").and_then(Json::number).ok_or("policy after needs reload_at")?),
                    other => return Err(format!("no reload policy {other}")),
                };
                let mut entries = Vec::new();
                for e in p.get("entries").and_then(|j| j.as_array()).unwrap_or_default() {
                    let date = e.get("date").and_then(Json::number).ok_or("an entry needs a date")?;
                    entries.push(Entry { date, value: Text::of(e) });
                }
                let n = self.reload(app, &kind, entries, policy)?;
                Ok(Json::Num(n as f64))
            }
            "remove" => {
                let kind = p.str_field("kind").ok_or("remove needs a kind")?;
                Ok(Json::Bool(self.remove(app, &kind)))
            }
            "current" => {
                let kind = p.str_field("kind").ok_or("current needs a kind")?;
                Ok(match self.current(app, &kind, now) {
                    Some(e) => Json::Obj(vec![
                        ("date".into(), Json::Num(e.date)),
                        ("title".into(), Json::Str(e.value.title)),
                        ("text".into(), Json::Str(e.value.text)),
                    ]),
                    None => Json::Null,
                })
            }
            "start_activity" => {
                let content = content_of(p);
                let start = p.get("start_at").and_then(Json::as_u64);
                let id = self.activities.request(app, content, start, unix(now)).map_err(error_text)?;
                self.owners.push((id, app.into()));
                Ok(Json::Num(id as f64))
            }
            "update_activity" => {
                let id = id_of(p)?;
                self.owned(app, id, now)?;
                self.activities.update(id, content_of(p), unix(now)).map_err(error_text)?;
                Ok(Json::Bool(true))
            }
            "end_activity" => {
                let id = id_of(p)?;
                self.owned(app, id, now)?;
                let dismissal = match p.get("dismissal").and_then(Json::as_str).unwrap_or("default") {
                    "default" => Dismissal::Default,
                    "immediate" => Dismissal::Immediate,
                    "after" => Dismissal::After(p.get("after").and_then(Json::as_u64).ok_or("dismissal after needs after")?),
                    other => return Err(format!("no dismissal {other}")),
                };
                let final_content = if p.get("title").is_some() || p.get("text").is_some() { Some(content_of(p)) } else { None };
                self.activities.end(id, final_content, dismissal, unix(now)).map_err(error_text)?;
                Ok(Json::Bool(true))
            }
            "activities" => {
                let mut list = Vec::new();
                for (id, a) in &self.owners {
                    if a != app {
                        continue;
                    }
                    let (Some(state), Some(c)) = (self.activities.state(*id, unix(now)), self.activities.content(*id, unix(now))) else { continue };
                    list.push(Json::Obj(vec![
                        ("id".into(), Json::Num(*id as f64)),
                        ("state".into(), Json::Str(state_name(state).into())),
                        ("title".into(), Json::Str(c.state.title.clone())),
                        ("text".into(), Json::Str(c.state.text.clone())),
                    ]));
                }
                Ok(Json::Arr(list))
            }
            _ => Err(format!("widgets_timeline has no method {method}")),
        }
    }
}

fn id_of(p: &Json) -> Result<u32, String> {
    let id = p.get("id").and_then(Json::as_u64).ok_or("needs an activity id")?;
    u32::try_from(id).map_err(|_| "no such activity".to_string())
}

fn content_of(p: &Json) -> Content<Text> {
    let c = Content::new(Text::of(p));
    match p.get("stale_at").and_then(Json::as_u64) {
        Some(t) => c.stale_at(t),
        None => c,
    }
}

/// The `widgets_timeline` service, registered in [`super::all`].
pub struct WidgetsTimeline;

impl SystemService for WidgetsTimeline {
    fn name(&self) -> &'static str {
        "widgets_timeline"
    }

    fn call(&mut self, shell: &mut Shell, app_id: &str, method: &str, params: &Json, _t: f64) -> Result<Json, String> {
        let now = shell.notify.now();
        with(|s| s.call(app_id, method, params, now))
    }

    fn tick(&mut self, shell: &mut Shell, _t: f64) -> Vec<Event> {
        let now = shell.notify.now();
        with(|s| s.due(now))
            .into_iter()
            .map(|(app, kind)| Event {
                app: Some(app),
                event: "reload".into(),
                body: Json::Obj(vec![("kind".into(), Json::Str(kind))]),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(date: f64, title: &str, text: &str) -> Entry<Text> {
        Entry { date, value: Text { title: title.into(), text: text.into() } }
    }

    fn obj(pairs: &[(&str, Json)]) -> Json {
        Json::Obj(pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect())
    }

    #[test]
    fn the_entry_on_screen_follows_the_clock_and_is_per_app_and_kind() {
        let mut s = Store::default();
        s.reload("is.a", "next", vec![entry(100.0, "Next", "first"), entry(200.0, "Next", "second")], ReloadPolicy::Never).unwrap();
        assert_eq!(s.current("is.a", "next", 150).map(|e| e.value.text), Some("first".into()));
        assert_eq!(s.current("is.a", "next", 250).map(|e| e.value.text), Some("second".into()));
        assert_eq!(s.current("is.b", "next", 250), None, "another app cannot read the timeline");
        assert_eq!(s.rows(150), vec![Row { title: "Next".into(), text: "first".into() }]);
    }

    #[test]
    fn a_reload_replaces_the_timeline_and_remove_drops_it() {
        let mut s = Store::default();
        s.reload("is.a", "k", vec![entry(1.0, "t", "old")], ReloadPolicy::Never).unwrap();
        s.reload("is.a", "k", vec![entry(1.0, "t", "new")], ReloadPolicy::Never).unwrap();
        assert_eq!(s.rows(5).len(), 1);
        assert_eq!(s.rows(5)[0].text, "new");
        assert!(s.remove("is.a", "k"));
        assert!(!s.remove("is.a", "k"));
        assert!(s.rows(5).is_empty());
    }

    #[test]
    fn a_reload_is_due_once_at_its_date_and_at_end_waits_for_the_last_entry() {
        let mut s = Store::default();
        s.reload("is.a", "fixed", vec![entry(10.0, "t", "x")], ReloadPolicy::After(300.0)).unwrap();
        s.reload("is.a", "end", vec![entry(10.0, "t", "a"), entry(50.0, "t", "b")], ReloadPolicy::AtEnd).unwrap();
        assert!(s.due(40).is_empty(), "neither reload date has passed");
        assert_eq!(s.due(60), vec![("is.a".to_string(), "end".to_string())], "the last entry's date passed");
        assert!(s.due(60).is_empty(), "each reload is requested once");
        assert_eq!(s.due(300), vec![("is.a".to_string(), "fixed".to_string())]);
    }

    #[test]
    fn a_never_timeline_is_not_asked_for_a_reload() {
        let mut s = Store::default();
        s.reload("is.a", "m", vec![entry(10.0, "t", "x")], ReloadPolicy::Never).unwrap();
        assert!(s.due(1_000_000).is_empty());
    }

    #[test]
    fn reload_validates_the_kind_and_the_entry_count() {
        let mut s = Store::default();
        assert!(s.reload("is.a", "", vec![], ReloadPolicy::Never).is_err());
        assert!(s.reload("is.a", &"k".repeat(MAX_KIND + 1), vec![], ReloadPolicy::Never).is_err());
        let many: Vec<_> = (0..=MAX_ENTRIES).map(|i| entry(i as f64, "t", "x")).collect();
        assert!(s.reload("is.a", "k", many, ReloadPolicy::Never).is_err());
    }

    #[test]
    fn the_reload_call_reads_entries_and_policy_from_json() {
        let mut s = Store::default();
        let body = obj(&[
            ("kind", Json::Str("next".into())),
            ("policy", Json::Str("after".into())),
            ("reload_at", Json::Num(500.0)),
            ("entries", Json::Arr(vec![obj(&[("date", Json::Num(10.0)), ("title", Json::Str("T".into())), ("text", Json::Str("v".into()))])])),
        ]);
        assert_eq!(s.call("is.a", "reload", &body, 20), Ok(Json::Num(1.0)));
        let cur = s.call("is.a", "current", &obj(&[("kind", Json::Str("next".into()))]), 20).unwrap();
        assert_eq!(cur.get("text").and_then(Json::as_str), Some("v"));
        assert!(s.call("is.a", "reload", &obj(&[("kind", Json::Str("x".into())), ("policy", Json::Str("soon".into()))]), 20).is_err());
        assert!(s.call("is.a", "reload", &obj(&[("kind", Json::Str("x".into())), ("entries", Json::Arr(vec![obj(&[])]))]), 20).is_err(), "an entry needs a date");
    }

    #[test]
    fn a_live_activity_is_started_updated_and_ended_by_its_owner_only() {
        let mut s = Store::default();
        let id = s.call("is.a", "start_activity", &obj(&[("title", Json::Str("Ride".into())), ("text", Json::Str("2 km".into()))]), 100).unwrap();
        let Json::Num(n) = id else { panic!("an id") };
        let id = n as u64;
        let p = obj(&[("id", Json::Num(id as f64)), ("title", Json::Str("Ride".into())), ("text", Json::Str("5 km".into()))]);
        assert!(s.call("is.b", "update_activity", &p, 110).is_err(), "another app cannot update it");
        assert_eq!(s.call("is.a", "update_activity", &p, 110), Ok(Json::Bool(true)));
        assert_eq!(s.rows(110), vec![Row { title: "Ride".into(), text: "5 km".into() }]);
        let listed = s.call("is.a", "activities", &Json::Null, 110).unwrap();
        assert_eq!(listed.as_array().map(|a| a.len()), Some(1));
        let end = obj(&[("id", Json::Num(id as f64)), ("dismissal", Json::Str("immediate".into()))]);
        assert_eq!(s.call("is.a", "end_activity", &end, 120), Ok(Json::Bool(true)));
        assert!(s.call("is.a", "update_activity", &p, 121).is_err(), "an ended activity takes no updates");
        assert!(s.rows(121).is_empty(), "an immediately dismissed activity leaves the screen");
        assert_eq!(s.call("is.a", "activities", &Json::Null, 10_000).unwrap(), Json::Arr(vec![]), "removed activities are not listed");
    }

    #[test]
    fn live_activities_can_be_switched_off_by_the_person() {
        let mut s = Store::default();
        s.activities.set_enabled(false);
        let err = s.call("is.a", "start_activity", &obj(&[("title", Json::Str("x".into()))]), 1);
        assert_eq!(err, Err("live activities are off".into()));
    }

    #[test]
    fn unknown_methods_are_errors() {
        let mut s = Store::default();
        assert!(s.call("is.a", "nope", &Json::Null, 0).is_err());
    }
}
