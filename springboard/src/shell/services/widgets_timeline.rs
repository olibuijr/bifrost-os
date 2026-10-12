//! The `widgets_timeline` service: apps give the shell widget timelines (dated entries with a reload policy) and start
//! live activities. The shell keeps them, shows the entry on screen in Notification Centre's widget area and asks the
//! owning app for a new timeline when its reload date passes (`widgets.reload` event). Models: `bifrost_ui::timeline`
//! (entries and reload policy) and `bifrost_ui::live_activity` (the activity life cycle).
//!
//! Methods (JSON in, JSON out):
//! - `reload {kind, entries:[{date,title,text}], policy:"at_end"|"never"|"after", reload_at?}` replaces the app's
//!   timeline for `kind`; returns the entry count.
//!   An optional `relevance` object ranks the widget in the stack: `{score, duration}` (eligible `duration` seconds from
//!   the entry's date) or `{score, at}` / `{score, from, to, closed?}` (eligible while a time clue is active); `kind`
//!   ("default"|"informational"|"scheduled") moves the score. Without it the widget keeps its place in the stack.
//! - `remove {kind}` drops it; returns whether one was there.
//! - `current {kind}` returns `{date,title,text}` of the entry on screen now, or null.
//! - `search {kind, description?, placeholder?, results:[string]}` sets the widget's search view (the reference
//!   `NCWidgetSearchViewController` results); returns the result count.
//! - `search_rows {kind}` returns `{rows:[string], placeholder, description}` for the search view, or the empty view.
//! - `start_activity {title,text,stale_at?,start_at?}` returns the activity id.
//! - `update_activity {id,title,text,stale_at?}`, `end_activity {id,title?,text?,dismissal:"default"|"immediate"|"after",after?}`.
//! - `activities {}` lists the app's activities on screen: `[{id,state,title,text}]`.
//!
//! The store is process-wide because Notification Centre draws from it outside a service call; the logic lives in
//! [`Store`], which the tests drive directly.
use super::{Event, Shell, SystemService};
use bifrost_ui::live_activity::{Content, Dismissal, Error, Registry, State};
use bifrost_ui::relevance::{self, ClueError, DateKind, Item, Relevance, TimeClue};
use bifrost_ui::timeline::{Entry, ReloadPolicy, Timeline};
use bifrost_ui::widget_search::WidgetSearch;
use bifrost_ui::widget_config::{ConfigError, Configuration, Parameter, Value};
use bifrost_ui::widget_link::Link;
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
    /// the widget is set to Large in the Edit Widgets gallery: its row is taller
    pub large: bool,
    /// the (app, kind) of a widget row; None for a live activity
    pub widget: Option<(String, String)>,
}

/// One widget in the Edit Widgets gallery: every widget an app has reloaded, placed in the column or not.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listing {
    pub app: String,
    pub kind: String,
    /// the title of the entry on screen, or the kind when there is none
    pub title: String,
    /// the search view's description, when the app set one
    pub description: Option<String>,
    pub placed: bool,
    pub large: bool,
    /// the widget has user-configurable options (`configure`); Edit Widgets offers Options for it
    pub configurable: bool,
}

/// A widget's relevance hint: how the stack ranks its entry (`relevance::rank`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Hint {
    /// Eligible for `duration` seconds from the entry's date, with `score`.
    Entry { score: f32, duration: f64 },
    /// Eligible while `clue` is active; the clue's hint moves `score`.
    Clue { score: f32, clue: TimeClue },
}

impl Hint {
    /// The rank item for an entry dated `date`, as seen at `now`.
    fn item(&self, date: f64, now: f64) -> Item {
        match *self {
            Hint::Entry { score, duration } => Item { date, relevance: Some(Relevance { score, duration }) },
            Hint::Clue { score, clue } => {
                // While the clue is active its score is final, so the item stays eligible until the clue ends.
                let relevance = clue.score_at(score, now).map(|s| Relevance { score: s, duration: f64::INFINITY });
                Item { date: clue.window().0, relevance }
            }
        }
    }
}

struct Widget {
    app: String,
    kind: String,
    timeline: Timeline<Text>,
    /// the reload event was already sent for this timeline
    requested: bool,
    /// how the stack ranks this widget; None keeps the widget in its given place
    hint: Option<Hint>,
    /// shown in the column (Edit Widgets adds and removes it); kept across reloads
    placed: bool,
    /// set to Large in the gallery; kept across reloads
    large: bool,
    /// the link a tap on the widget opens; set by each reload (`link`)
    link: Option<Link>,
    /// the options the app defined and the person's values; kept across reloads
    config: Option<Configuration>,
}

/// An app's search view for one widget kind.
struct Search {
    app: String,
    kind: String,
    view: WidgetSearch<String>,
}

/// The string a search result shows in the list.
fn result_title(s: &String) -> &str {
    s.as_str()
}

/// The most results one search view may hold.
const MAX_RESULTS: usize = 64;

/// Every app's widget timelines, search views and live activities.
pub struct Store {
    widgets: Vec<Widget>,
    searches: Vec<Search>,
    activities: Registry<Text>,
    /// the app that started each activity, by id
    owners: Vec<(u32, String)>,
}

impl Default for Store {
    fn default() -> Self {
        Store { widgets: Vec::new(), searches: Vec::new(), activities: Registry::new(true), owners: Vec::new() }
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

/// The Edit Widgets gallery at `now` (unix seconds): every widget an app has reloaded.
pub fn gallery(now: i64) -> Vec<Listing> {
    with(|s| s.listing(now))
}

/// The link a tap on `app`'s widget `kind` opens, if the app set one.
pub fn link(app: &str, kind: &str) -> Option<Link> {
    with(|s| s.link(app, kind).cloned())
}

/// The options of `app`'s widget `kind` with the values in effect, if the app defined any.
pub fn configuration(app: &str, kind: &str) -> Option<Configuration> {
    with(|s| s.configuration(app, kind).cloned())
}

/// Moves one option of `app`'s widget `kind` by `dir` (see [`Store::step_option`]). False when it is not there.
pub fn step_option(app: &str, kind: &str, key: &str, dir: i64) -> bool {
    with(|s| s.step_option(app, kind, key, dir))
}

/// Adds `app`'s widget `kind` to the column, or takes it off. False when the app has not reloaded that kind.
pub fn place(app: &str, kind: &str, placed: bool) -> bool {
    with(|s| s.set_placed(app, kind, placed))
}

/// Sets `app`'s widget `kind` to Large (`true`) or Small in the column. False when the app has not reloaded that kind.
pub fn resize(app: &str, kind: &str, large: bool) -> bool {
    with(|s| s.set_large(app, kind, large))
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
    /// Replaces the timeline `kind` of `app`, with no relevance hint. The kind must be 1 to 64 bytes and the entries
    /// at most 64.
    #[cfg(test)]
    pub fn reload(&mut self, app: &str, kind: &str, entries: Vec<Entry<Text>>, policy: ReloadPolicy) -> Result<usize, String> {
        self.reload_ranked(app, kind, entries, policy, None)
    }

    /// [`Store::reload`] with the widget's relevance hint (None for no ranking).
    pub fn reload_ranked(
        &mut self,
        app: &str,
        kind: &str,
        entries: Vec<Entry<Text>>,
        policy: ReloadPolicy,
        hint: Option<Hint>,
    ) -> Result<usize, String> {
        check_kind(kind)?;
        if entries.len() > MAX_ENTRIES {
            return Err(format!("a timeline holds at most {MAX_ENTRIES} entries"));
        }
        let n = entries.len();
        // a reload keeps the person's choices in the gallery; a new widget starts placed, at Small
        let (placed, large, config) = self.widgets.iter().find(|w| w.app == app && w.kind == kind).map_or((true, false, None), |w| (w.placed, w.large, w.config.clone()));
        self.widgets.retain(|w| !(w.app == app && w.kind == kind));
        self.widgets.push(Widget { app: app.into(), kind: kind.into(), timeline: Timeline::new(entries, policy), requested: false, hint, placed, large, link: None, config });
        Ok(n)
    }

    /// Adds the widget to the column (`placed`) or takes it off; false when the app has not reloaded that kind.
    pub fn set_placed(&mut self, app: &str, kind: &str, placed: bool) -> bool {
        self.widgets.iter_mut().find(|w| w.app == app && w.kind == kind).map(|w| w.placed = placed).is_some()
    }

    /// Sets the widget's size in the column: Large is the taller row. False when the app has not reloaded that kind.
    pub fn set_large(&mut self, app: &str, kind: &str, large: bool) -> bool {
        self.widgets.iter_mut().find(|w| w.app == app && w.kind == kind).map(|w| w.large = large).is_some()
    }

    /// Sets the link a tap on the widget opens (None for no link). False when the app has not reloaded that kind.
    pub fn set_link(&mut self, app: &str, kind: &str, link: Option<Link>) -> bool {
        self.widgets.iter_mut().find(|w| w.app == app && w.kind == kind).map(|w| w.link = link).is_some()
    }

    /// The link a tap on the widget opens, if the app set one.
    pub fn link(&self, app: &str, kind: &str) -> Option<&Link> {
        self.widgets.iter().find(|w| w.app == app && w.kind == kind).and_then(|w| w.link.as_ref())
    }

    /// Defines the widget's options (`params`). Defining the same options again keeps the person's values; new options
    /// start at their defaults. Returns the option count. The widget must have been reloaded.
    pub fn configure(&mut self, app: &str, kind: &str, params: Vec<Parameter>) -> Result<usize, String> {
        check_kind(kind)?;
        let config = Configuration::new(params).map_err(config_error)?;
        let n = config.params().len();
        let w = self.widgets.iter_mut().find(|w| w.app == app && w.kind == kind).ok_or_else(|| format!("no widget {kind}"))?;
        if w.config.as_ref().map_or(true, |c| c.params() != config.params()) {
            w.config = Some(config);
        }
        Ok(n)
    }

    /// The widget's options with the values in effect, if the app defined any.
    pub fn configuration(&self, app: &str, kind: &str) -> Option<&Configuration> {
        self.widgets.iter().find(|w| w.app == app && w.kind == kind).and_then(|w| w.config.as_ref())
    }

    /// Sets one option of the widget. The value must match the option's kind and range.
    pub fn set_option(&mut self, app: &str, kind: &str, key: &str, value: Value) -> Result<(), String> {
        let c = self.widgets.iter_mut().find(|w| w.app == app && w.kind == kind).and_then(|w| w.config.as_mut()).ok_or_else(|| format!("{kind} has no options"))?;
        c.set(key, value).map_err(config_error)
    }

    /// Moves one option by `dir`: a choice to the next (or previous) name, wrapping; a number by one, kept in its
    /// range; a flag flips. False when the widget or option is not there.
    pub fn step_option(&mut self, app: &str, kind: &str, key: &str, dir: i64) -> bool {
        let Some(c) = self.widgets.iter_mut().find(|w| w.app == app && w.kind == kind).and_then(|w| w.config.as_mut()) else { return false };
        let Some(param) = c.params().iter().find(|p| p.key() == key).cloned() else { return false };
        let next = match (param, c.value(key)) {
            (Parameter::Choice { options, .. }, Some(Value::Choice(i))) => Value::Choice((i as i64 + dir).rem_euclid(options.len() as i64) as usize),
            (Parameter::Flag { .. }, Some(Value::Flag(b))) => Value::Flag(!b),
            (Parameter::Number { min, max, .. }, Some(Value::Number(n))) => Value::Number((n + dir).clamp(min, max)),
            _ => return false,
        };
        c.set(key, next).is_ok()
    }

    /// The Edit Widgets gallery at `now`: every widget, in the order the apps reloaded them.
    pub fn listing(&self, now: i64) -> Vec<Listing> {
        self.widgets
            .iter()
            .map(|w| Listing {
                app: w.app.clone(),
                kind: w.kind.clone(),
                title: w.timeline.entry_at(now as f64).map(|e| e.value.title.clone()).filter(|t| !t.is_empty()).unwrap_or_else(|| w.kind.clone()),
                description: self.search_view(&w.app, &w.kind).and_then(|v| v.description()).map(str::to_string),
                placed: w.placed,
                large: w.large,
                configurable: w.config.as_ref().is_some_and(|c| !c.params().is_empty()),
            })
            .collect()
    }

    /// Replaces the search view of `kind` for `app` with `results`, in their order. Blank `description` or
    /// `placeholder` text is no text. Returns the result count.
    pub fn set_search(&mut self, app: &str, kind: &str, description: &str, placeholder: &str, results: Vec<String>) -> Result<usize, String> {
        check_kind(kind)?;
        if results.len() > MAX_RESULTS {
            return Err(format!("a search view holds at most {MAX_RESULTS} results"));
        }
        let n = results.len();
        let mut view = WidgetSearch::new(result_title).with_description(description).with_placeholder(placeholder);
        view.set_results(results);
        self.searches.retain(|s| !(s.app == app && s.kind == kind));
        self.searches.push(Search { app: app.into(), kind: kind.into(), view });
        Ok(n)
    }

    /// The search view of `kind` for `app`, if the app set one.
    pub fn search_view(&self, app: &str, kind: &str) -> Option<&WidgetSearch<String>> {
        self.searches.iter().find(|s| s.app == app && s.kind == kind).map(|s| &s.view)
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
        let now_f = now as f64;
        // each widget's entry on screen, paired with its rank item; widgets without a hint rank last, in their order
        // a widget taken off in the gallery is not in the column
        let shown: Vec<(&Widget, &Entry<Text>)> = self.widgets.iter().filter(|w| w.placed).filter_map(|w| w.timeline.entry_at(now_f).map(|e| (w, e))).collect();
        let items: Vec<Item> = shown
            .iter()
            .map(|(w, e)| match w.hint {
                Some(h) => h.item(e.date, now_f),
                None => Item { date: e.date, relevance: None },
            })
            .collect();
        let mut rows: Vec<Row> = relevance::rank(&items, now_f)
            .into_iter()
            .map(|i| {
                let (w, e) = shown[i];
                Row { title: e.value.title.clone(), text: e.value.text.clone(), large: w.large, widget: Some((w.app.clone(), w.kind.clone())) }
            })
            .collect();
        for (id, _) in &self.owners {
            if let Some(c) = self.activities.content(*id, unix(now)) {
                rows.push(Row { title: c.state.title.clone(), text: c.state.text.clone(), large: false, widget: None });
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
                let hint = hint_of(p)?;
                let link = match p.str_field("link") {
                    Some(u) => Some(Link::parse(&u).map_err(|e| format!("a widget link is bifrost-app://app/path, not {u:?} ({e:?})"))?),
                    None => None,
                };
                let n = self.reload_ranked(app, &kind, entries, policy, hint)?;
                self.set_link(app, &kind, link);
                Ok(Json::Num(n as f64))
            }
            "configure" => {
                let kind = p.str_field("kind").ok_or("configure needs a kind")?;
                let mut params = Vec::new();
                for j in p.get("parameters").and_then(|j| j.as_array()).unwrap_or_default() {
                    params.push(parameter_of(j)?);
                }
                Ok(Json::Num(self.configure(app, &kind, params)? as f64))
            }
            "set_option" => {
                let kind = p.str_field("kind").ok_or("set_option needs a kind")?;
                let key = p.str_field("key").ok_or("set_option needs a key")?;
                let param = self
                    .configuration(app, &kind)
                    .and_then(|c| c.params().iter().find(|q| q.key() == key).cloned())
                    .ok_or_else(|| format!("no option {key}"))?;
                let value = value_of(&param, p.get("value").ok_or("set_option needs a value")?)?;
                self.set_option(app, &kind, &key, value)?;
                Ok(Json::Bool(true))
            }
            "options" => {
                let kind = p.str_field("kind").ok_or("options needs a kind")?;
                let Some(c) = self.configuration(app, &kind) else { return Ok(Json::Arr(Vec::new())) };
                let mut out = Vec::new();
                for param in c.params() {
                    let key = param.key();
                    let value = match c.value(key) {
                        Some(Value::Choice(_)) => Json::Str(c.choice_label(key).unwrap_or_default().to_string()),
                        Some(Value::Flag(b)) => Json::Bool(b),
                        Some(Value::Number(n)) => Json::Num(n as f64),
                        None => Json::Null,
                    };
                    out.push(Json::Obj(vec![("key".into(), Json::Str(key.into())), ("value".into(), value)]));
                }
                Ok(Json::Arr(out))
            }
            "search" => {
                let kind = p.str_field("kind").ok_or("search needs a kind")?;
                let description = p.str_field("description").unwrap_or_default();
                let placeholder = p.str_field("placeholder").unwrap_or_default();
                let results: Vec<String> = p
                    .get("results")
                    .and_then(|j| j.as_array())
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|r| r.as_str().map(|s| s.to_string()))
                    .collect();
                let n = self.set_search(app, &kind, &description, &placeholder, results)?;
                Ok(Json::Num(n as f64))
            }
            "search_rows" => {
                let kind = p.str_field("kind").ok_or("search_rows needs a kind")?;
                let view = self.search_view(app, &kind);
                let rows: Vec<Json> = view.map(|v| v.rows().into_iter().map(|r| Json::Str(r.to_string())).collect()).unwrap_or_default();
                let placeholder = view.and_then(|v| v.placeholder()).map_or(Json::Null, |s| Json::Str(s.to_string()));
                let description = view.and_then(|v| v.description()).map_or(Json::Null, |s| Json::Str(s.to_string()));
                Ok(Json::Obj(vec![
                    ("rows".into(), Json::Arr(rows)),
                    ("placeholder".into(), placeholder),
                    ("description".into(), description),
                ]))
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

fn config_error(e: ConfigError) -> String {
    match e {
        ConfigError::EmptyKey => "an option needs a key".into(),
        ConfigError::DuplicateKey(k) => format!("option {k} is listed twice"),
        ConfigError::NoOptions(k) => format!("choice {k} lists no names"),
        ConfigError::BadDefault(k) => format!("the default of {k} is out of range"),
        ConfigError::BadRange(k) => format!("option {k} has min above max"),
        ConfigError::UnknownKey(k) => format!("no option {k}"),
        ConfigError::WrongType(k) => format!("the value of {k} has the wrong kind"),
        ConfigError::OutOfRange(k) => format!("the value of {k} is out of range"),
    }
}

/// One option of a `configure` call: `{type:"choice", key, options:[names], default?}`, `{type:"flag", key, default?}`
/// or `{type:"number", key, min, max, default?}` (the default is the minimum when left out).
fn parameter_of(j: &Json) -> Result<Parameter, String> {
    let key = j.str_field("key").ok_or("a parameter needs a key")?;
    let num = |name: &str| j.get(name).and_then(Json::number);
    match j.str_field("type").as_deref() {
        Some("choice") => {
            let options: Vec<String> = j.get("options").and_then(|o| o.as_array()).unwrap_or_default().iter().filter_map(|o| o.as_str().map(str::to_string)).collect();
            Ok(Parameter::Choice { key, options, default: num("default").unwrap_or(0.0).max(0.0) as usize })
        }
        Some("flag") => Ok(Parameter::Flag { key, default: matches!(j.get("default"), Some(Json::Bool(true))) }),
        Some("number") => {
            let min = num("min").unwrap_or(0.0) as i64;
            let max = num("max").unwrap_or(min as f64) as i64;
            let default = num("default").map_or(min, |d| d as i64);
            Ok(Parameter::Number { key, min, max, default })
        }
        Some(other) => Err(format!("no option type {other}")),
        None => Err("a parameter needs a type".into()),
    }
}

/// The value a `set_option` call sends for `param`: a choice index, a flag, or a whole number.
fn value_of(param: &Parameter, j: &Json) -> Result<Value, String> {
    let whole = |what: &str| Json::number(j).filter(|n| n.is_finite() && n.fract() == 0.0).ok_or(format!("{what} takes a whole number"));
    match param {
        Parameter::Choice { .. } => {
            let n = whole("a choice")?;
            if n < 0.0 {
                return Err("a choice index is 0 or more".into());
            }
            Ok(Value::Choice(n as usize))
        }
        Parameter::Flag { .. } => match j {
            Json::Bool(b) => Ok(Value::Flag(*b)),
            _ => Err("a flag takes true or false".into()),
        },
        Parameter::Number { .. } => Ok(Value::Number(whole("a number")? as i64)),
    }
}

fn id_of(p: &Json) -> Result<u32, String> {
    let id = p.get("id").and_then(Json::as_u64).ok_or("needs an activity id")?;
    u32::try_from(id).map_err(|_| "no such activity".to_string())
}

/// A widget kind must be 1 to `MAX_KIND` bytes.
fn check_kind(kind: &str) -> Result<(), String> {
    if kind.is_empty() || kind.len() > MAX_KIND {
        return Err(format!("a widget kind is 1 to {MAX_KIND} bytes"));
    }
    Ok(())
}

/// The relevance hint of a `reload` call: `{score, duration}`, or a time clue `{score, at}` / `{score, from, to,
/// closed?}`, with an optional `kind`. None when the call has no `relevance`.
fn hint_of(p: &Json) -> Result<Option<Hint>, String> {
    let Some(r) = p.get("relevance") else { return Ok(None) };
    let score: f64 = r.get("score").and_then(Json::number).ok_or("relevance needs a score")?;
    if !score.is_finite() {
        return Err("a relevance score is a number".into());
    }
    let score = score as f32;
    let kind = match r.get("kind").and_then(Json::as_str).unwrap_or("default") {
        "default" => DateKind::Default,
        "informational" => DateKind::Informational,
        "scheduled" => DateKind::Scheduled,
        other => return Err(format!("no relevance kind {other}")),
    };
    if let Some(duration) = r.get("duration").and_then(Json::number) {
        return Ok(Some(Hint::Entry { score: score + kind.score_offset(), duration }));
    }
    let clue = if let Some(at) = r.get("at").and_then(Json::number) {
        TimeClue::at(at, kind)
    } else if let (Some(from), Some(to)) = (r.get("from").and_then(Json::number), r.get("to").and_then(Json::number)) {
        if matches!(r.get("closed"), Some(Json::Bool(true))) {
            TimeClue::range(from, to, kind)
        } else {
            TimeClue::interval(from, to, kind)
        }
    } else {
        return Err("relevance needs a duration, an at date, or from and to".into());
    };
    let clue = clue.map_err(|e| match e {
        ClueError::NotFinite => "a clue date is a number".to_string(),
        ClueError::EmptyWindow => "a clue window is not empty".to_string(),
    })?;
    Ok(Some(Hint::Clue { score, clue }))
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
        assert_eq!(s.rows(150), vec![Row { title: "Next".into(), text: "first".into(), large: false, widget: Some(("is.a".into(), "next".into())) }]);
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
        assert_eq!(s.rows(110), vec![Row { title: "Ride".into(), text: "5 km".into(), large: false, widget: None }]);
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

    fn ranked(s: &mut Store, app: &str, kind: &str, date: f64, text: &str, hint: Option<Hint>) {
        s.reload_ranked(app, kind, vec![entry(date, "W", text)], ReloadPolicy::Never, hint).unwrap();
    }

    fn texts(rows: &[Row]) -> Vec<&str> {
        rows.iter().map(|r| r.text.as_str()).collect()
    }

    #[test]
    fn widgets_without_a_hint_keep_their_stack_order() {
        let mut s = Store::default();
        ranked(&mut s, "is.a", "one", 100.0, "a", None);
        ranked(&mut s, "is.a", "two", 100.0, "b", None);
        assert_eq!(texts(&s.rows(150)), ["a", "b"]);
    }

    #[test]
    fn a_higher_score_comes_first_and_an_expired_entry_falls_behind() {
        let mut s = Store::default();
        // A: score 1, eligible 100..1100. B: score 3, eligible 100..1100. C: no hint.
        ranked(&mut s, "is.a", "a", 100.0, "a", Some(Hint::Entry { score: 1.0, duration: 1000.0 }));
        ranked(&mut s, "is.a", "b", 100.0, "b", Some(Hint::Entry { score: 3.0, duration: 1000.0 }));
        ranked(&mut s, "is.a", "c", 100.0, "c", None);
        assert_eq!(texts(&s.rows(150)), ["b", "a", "c"]);
        // B is re-added with duration 20, so it has expired at 150. The stack is now [A, C, B]: A is eligible, then
        // C and B keep their stack order.
        ranked(&mut s, "is.a", "b", 100.0, "b", Some(Hint::Entry { score: 3.0, duration: 20.0 }));
        assert_eq!(texts(&s.rows(150)), ["a", "c", "b"]);
    }

    #[test]
    fn a_time_clue_scores_by_its_hint_and_drops_out_when_it_ends() {
        let mut s = Store::default();
        // D: a scheduled clue over 0..200 with base 1.0, so 1.0 + 1.0 = 2.0 while active.
        let clue = TimeClue::interval(0.0, 200.0, DateKind::Scheduled).unwrap();
        ranked(&mut s, "is.a", "d", 0.0, "d", Some(Hint::Clue { score: 1.0, clue }));
        // E: entry-based score 1.5 for 1000 s from 100.
        ranked(&mut s, "is.a", "e", 100.0, "e", Some(Hint::Entry { score: 1.5, duration: 1000.0 }));
        assert_eq!(texts(&s.rows(150)), ["d", "e"], "2.0 beats 1.5");
        assert_eq!(texts(&s.rows(250)), ["e", "d"], "the clue ended at 200, so D has no score");
    }

    #[test]
    fn a_relevance_object_in_the_reload_call_builds_the_hint() {
        let mut s = Store::default();
        let body = |relevance: Json| {
            obj(&[
                ("kind", Json::Str("k".into())),
                ("relevance", relevance),
                ("entries", Json::Arr(vec![obj(&[("date", Json::Num(0.0)), ("text", Json::Str("v".into()))])])),
            ])
        };
        let clue = obj(&[("score", Json::Num(1.0)), ("kind", Json::Str("scheduled".into())), ("from", Json::Num(0.0)), ("to", Json::Num(200.0))]);
        assert_eq!(s.call("is.a", "reload", &body(clue), 10), Ok(Json::Num(1.0)));
        let plain = obj(&[("score", Json::Num(2.0)), ("duration", Json::Num(60.0))]);
        assert_eq!(hint_of(&body(plain)), Ok(Some(Hint::Entry { score: 2.0, duration: 60.0 })));
        let scheduled = obj(&[("score", Json::Num(1.0)), ("kind", Json::Str("scheduled".into())), ("duration", Json::Num(60.0))]);
        assert_eq!(hint_of(&body(scheduled)), Ok(Some(Hint::Entry { score: 2.0, duration: 60.0 })), "scheduled adds 1.0");
        assert!(hint_of(&body(obj(&[("score", Json::Num(1.0))]))).is_err(), "no duration and no clue dates");
        assert!(hint_of(&body(obj(&[("duration", Json::Num(1.0))]))).is_err(), "a score is required");
        let empty = obj(&[("score", Json::Num(1.0)), ("from", Json::Num(5.0)), ("to", Json::Num(5.0))]);
        assert!(hint_of(&body(empty)).is_err(), "an interval with no length is refused");
        assert_eq!(hint_of(&obj(&[("kind", Json::Str("k".into()))])), Ok(None), "no relevance object, no hint");
    }

    #[test]
    fn a_search_view_holds_its_results_and_shows_the_placeholder_when_empty() {
        let mut s = Store::default();
        let set = |results: Vec<Json>| {
            obj(&[
                ("kind", Json::Str("notes".into())),
                ("description", Json::Str(" Notes ".into())),
                ("placeholder", Json::Str("No notes match".into())),
                ("results", Json::Arr(results)),
            ])
        };
        assert_eq!(s.call("is.a", "search", &set(vec![Json::Str("Groceries".into()), Json::Str("Plan".into())]), 0), Ok(Json::Num(2.0)));
        let view = s.call("is.a", "search_rows", &obj(&[("kind", Json::Str("notes".into()))]), 0).unwrap();
        assert_eq!(view.get("description").and_then(Json::as_str), Some("Notes"), "trimmed");
        assert_eq!(view.get("placeholder"), Some(&Json::Null), "results are shown, so no placeholder");
        assert_eq!(view.get("rows").and_then(|j| j.as_array()).map(|a| a.len()), Some(2));
        // A new search with no results shows the placeholder in place of the list.
        assert_eq!(s.call("is.a", "search", &set(vec![]), 0), Ok(Json::Num(0.0)));
        let empty = s.call("is.a", "search_rows", &obj(&[("kind", Json::Str("notes".into()))]), 0).unwrap();
        assert_eq!(empty.get("placeholder").and_then(Json::as_str), Some("No notes match"));
        assert_eq!(empty.get("rows").and_then(|j| j.as_array()).map(|a| a.len()), Some(0));
        let other = s.call("is.b", "search_rows", &obj(&[("kind", Json::Str("notes".into()))]), 0).unwrap();
        assert_eq!(other.get("placeholder"), Some(&Json::Null), "another app has no search view");
    }

    #[test]
    fn a_widget_taken_off_in_the_gallery_leaves_the_column_and_comes_back() {
        let mut s = Store::default();
        ranked(&mut s, "is.a", "one", 100.0, "a", None);
        ranked(&mut s, "is.a", "two", 100.0, "b", None);
        assert!(s.set_placed("is.a", "one", false));
        assert_eq!(texts(&s.rows(150)), ["b"], "the widget taken off is not drawn");
        let g = s.listing(150);
        assert_eq!(g.len(), 2, "the gallery still lists it, so it can be added back");
        assert!(!g[0].placed && g[1].placed);
        assert!(s.set_placed("is.a", "one", true));
        assert_eq!(texts(&s.rows(150)), ["a", "b"]);
        assert!(!s.set_placed("is.b", "one", false), "no such widget for another app");
    }

    #[test]
    fn a_reload_keeps_the_choice_made_in_the_gallery() {
        let mut s = Store::default();
        ranked(&mut s, "is.a", "one", 100.0, "a", None);
        s.set_placed("is.a", "one", false);
        s.set_large("is.a", "one", true);
        ranked(&mut s, "is.a", "one", 100.0, "a2", None);
        let g = s.listing(150);
        assert!(!g[0].placed && g[0].large, "the app's reload does not undo the person's choice");
        assert!(s.rows(150).is_empty());
    }

    #[test]
    fn a_large_widget_is_a_taller_row_and_the_gallery_shows_the_search_description() {
        let mut s = Store::default();
        ranked(&mut s, "is.a", "one", 100.0, "a", None);
        assert!(s.set_large("is.a", "one", true));
        assert_eq!(s.rows(150), vec![Row { title: "W".into(), text: "a".into(), large: true, widget: Some(("is.a".into(), "one".into())) }]);
        assert_eq!(s.listing(150)[0].title, "W", "the title of the entry on screen");
        assert_eq!(s.listing(150)[0].description, None);
        s.set_search("is.a", "one", "Notes", "", vec![]).unwrap();
        assert_eq!(s.listing(150)[0].description.as_deref(), Some("Notes"));
    }

    #[test]
    fn a_gallery_row_without_an_entry_on_screen_is_named_by_its_kind() {
        let mut s = Store::default();
        s.reload("is.a", "later", vec![], ReloadPolicy::Never).unwrap();
        let g = s.listing(150);
        assert_eq!(g[0].title, "later", "an empty timeline has no entry on screen");
        assert_eq!(g[0].kind, "later");
    }

    #[test]
    fn a_widget_link_is_read_on_reload_and_a_link_outside_the_scheme_is_refused() {
        let mut s = Store::default();
        let p = obj(&[
            ("kind", Json::Str("next".into())),
            ("entries", Json::Arr(vec![obj(&[("date", Json::Num(100.0)), ("title", Json::Str("Next".into()))])])),
            ("link", Json::Str("bifrost-app://is.a/trips/today".into())),
        ]);
        assert_eq!(s.call("is.a", "reload", &p, 90), Ok(Json::Num(1.0)));
        assert_eq!(s.link("is.a", "next").map(|l| l.to_url()), Some("bifrost-app://is.a/trips/today".to_string()));
        let bad = obj(&[("kind", Json::Str("next".into())), ("entries", Json::Arr(vec![])), ("link", Json::Str("https://example.com".into()))]);
        assert!(s.call("is.a", "reload", &bad, 90).is_err(), "only bifrost-app:// links are widget links");
        assert!(s.link("is.a", "next").is_some(), "a refused reload changes nothing");
        // a reload without a link clears it: the widget has no tap target until the app sends one again
        s.reload("is.a", "next", vec![], ReloadPolicy::Never).unwrap();
        assert_eq!(s.link("is.a", "next"), None);
    }

    #[test]
    fn a_widget_offers_options_once_it_defines_them_and_the_values_survive_a_reload() {
        let mut s = Store::default();
        s.reload("is.a", "next", vec![entry(100.0, "Next", "")], ReloadPolicy::Never).unwrap();
        assert_eq!(s.configure("is.b", "next", vec![]), Err("no widget next".to_string()), "an app configures only its own widget");
        let city = obj(&[("type", Json::Str("choice".into())), ("key", Json::Str("city".into())), ("options", Json::Arr(vec![Json::Str("Reykjavik".into()), Json::Str("Akureyri".into())]))]);
        let metric = obj(&[("type", Json::Str("flag".into())), ("key", Json::Str("metric".into())), ("default", Json::Bool(true))]);
        let days = obj(&[("type", Json::Str("number".into())), ("key", Json::Str("days".into())), ("min", Json::Num(1.0)), ("max", Json::Num(5.0))]);
        let p = obj(&[("kind", Json::Str("next".into())), ("parameters", Json::Arr(vec![city, metric, days]))]);
        assert_eq!(s.call("is.a", "configure", &p, 90), Ok(Json::Num(3.0)));
        assert!(s.listing(150)[0].configurable, "the gallery offers Options for it");
        let options = Json::Arr(vec![
            obj(&[("key", Json::Str("city".into())), ("value", Json::Str("Reykjavik".into()))]),
            obj(&[("key", Json::Str("metric".into())), ("value", Json::Bool(true))]),
            obj(&[("key", Json::Str("days".into())), ("value", Json::Num(1.0))]),
        ]);
        assert_eq!(s.call("is.a", "options", &obj(&[("kind", Json::Str("next".into()))]), 150), Ok(options));
        // one step each: the city wraps to Akureyri, the flag flips, the number goes up by one
        assert!(s.step_option("is.a", "next", "city", 1));
        assert!(s.step_option("is.a", "next", "metric", 1));
        assert!(s.step_option("is.a", "next", "days", 1));
        assert_eq!(s.configuration("is.a", "next").and_then(|c| c.choice_label("city").map(str::to_string)), Some("Akureyri".to_string()));
        assert_eq!(s.configuration("is.a", "next").and_then(|c| c.value("metric")), Some(Value::Flag(false)));
        assert_eq!(s.configuration("is.a", "next").and_then(|c| c.value("days")), Some(Value::Number(2)));
        // a number is kept in its range, and a value out of range is refused
        assert!(s.step_option("is.a", "next", "days", 10));
        assert_eq!(s.configuration("is.a", "next").and_then(|c| c.value("days")), Some(Value::Number(5)));
        assert!(s.set_option("is.a", "next", "days", Value::Number(9)).is_err());
        assert_eq!(s.call("is.a", "set_option", &obj(&[("kind", Json::Str("next".into())), ("key", Json::Str("days".into())), ("value", Json::Num(4.0))]), 150), Ok(Json::Bool(true)));
        // a reload keeps the person's values
        s.reload("is.a", "next", vec![entry(100.0, "Next", "")], ReloadPolicy::Never).unwrap();
        assert_eq!(s.configuration("is.a", "next").and_then(|c| c.value("days")), Some(Value::Number(4)));
        // defining the same options again keeps the values; a changed list starts over
        assert_eq!(s.configure("is.a", "next", vec![Parameter::Flag { key: "metric".into(), default: true }]), Ok(1));
        assert_eq!(s.configuration("is.a", "next").and_then(|c| c.value("days")), None, "a new list has only its own options");
    }
}
