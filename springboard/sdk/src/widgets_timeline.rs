//! Client for the shell's `widgets_timeline` service (`springboard/src/shell/services/widgets_timeline.rs`): an app
//! gives the shell a widget timeline (dated entries and a reload policy), reads the entry on screen, and starts and
//! ends live activities. The shell draws the current entry in Notification Centre and sends `reload` when the
//! timeline's reload date passes.
//!
//! ```ignore
//! use bifrost_app::widgets_timeline::{self as wt, Entry, ReloadPolicy};
//! wt::reload(ctx, "next-event", &[Entry { date: start, title: "Next".into(), text: "Standup".into() }], ReloadPolicy::AtEnd);
//! // in App::service_event:
//! //   if let Some(wt::Event::Reload { kind }) = wt::parse_event(service, event, body) { /* build and reload again */ }
//! ```
//! Requests return the request id the reply carries (see [`crate::service`]). Dates are unix seconds.
use crate::controls::live_activity::Dismissal;
pub use crate::controls::timeline::ReloadPolicy;
use crate::runtime::Ctx;
use crate::service;
use bifrost_core::json::Json;

/// The service name the shell registers.
pub const SERVICE: &str = "widgets_timeline";
/// The event the shell sends when a timeline's reload date has passed. Its body is `{"kind": "..."}`.
pub const EVENT_RELOAD: &str = "reload";
/// The event the shell sends to an app that observes a widget (see [`observe`]) when the person adds, removes or
/// resizes it. Its body is `{"kind": "...", "on_screen": bool, "large": bool}`.
pub const EVENT_VISIBILITY: &str = "visibility";
/// The event the shell sends when the person opens the app from one of its widgets. Its body is
/// `{"kind": "...", "family": "small"|"medium"|"large"}`; a widget link arrives as `Open` just before it.
pub const EVENT_OPENED: &str = "opened";

/// One dated entry of a widget timeline: shown from `date` until the next entry's date.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub date: f64,
    pub title: String,
    pub text: String,
}

/// What a widget event means to the app.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// The shell asks for a new timeline for `kind`.
    Reload { kind: String },
    /// The person changed an observed widget: it is `on_screen` (in the column with an entry for now) and `large` or not.
    Visibility { kind: String, on_screen: bool, large: bool },
    /// The person opened the app from its widget `kind`, shown at `family` (the reference widget kind and family keys
    /// of the activity that opens the app).
    Opened { kind: String, family: Family },
}

/// Decodes a service event: `Some` for this service's events the SDK knows, `None` for any other.
pub fn parse_event(service: &str, event: &str, body: &Json) -> Option<Event> {
    if service != SERVICE {
        return None;
    }
    let kind = body.str_field("kind")?;
    match event {
        EVENT_RELOAD => Some(Event::Reload { kind }),
        EVENT_VISIBILITY => {
            let flag = |key: &str| match body.get(key) {
                Some(Json::Bool(b)) => Some(*b),
                _ => None,
            };
            Some(Event::Visibility { kind, on_screen: flag("on_screen")?, large: flag("large")? })
        }
        EVENT_OPENED => Some(Event::Opened { kind, family: body.get("family").and_then(Json::as_str).and_then(Family::from_key)? }),
        _ => None,
    }
}

/// Replaces the timeline `kind` with `entries` under `policy`.
pub fn reload(ctx: &Ctx, kind: &str, entries: &[Entry], policy: ReloadPolicy) -> u32 {
    let (method, body) = reload_call(kind, entries, policy);
    service::call(ctx, SERVICE, method, &body)
}

/// How the date in an entry's clock is written: `Relative` ("in 5 minutes") or `Timer` ("4:59").
pub use crate::controls::widget_date::Style as ClockStyle;
/// The sizes a widget kind can take.
pub use crate::controls::widget_kind::Family;
/// The system colours a widget can fill its tile with (the reference `containerBackground` given a colour).
pub use crate::controls::widget_render::ContainerTint;

/// A button or toggle on a widget entry. A tap runs the app's own `[[intent]]` `intent` with `params` over the app's
/// control socket (the app must be running). A toggle (`toggle` is `Some(shown state)`) also passes its new state as
/// the parameter `on` (`"true"` or `"false"`, read with [`crate::controls::control_widget::toggle_param`]); the widget
/// shows the new state at once and puts the old one back when the intent fails. At most three per entry.
#[derive(Clone, Debug, PartialEq)]
pub struct Button {
    pub id: String,
    pub label: String,
    pub intent: String,
    pub params: Vec<(String, String)>,
    pub toggle: Option<bool>,
}

impl Button {
    /// A button that runs `intent`.
    pub fn new(id: &str, label: &str, intent: &str) -> Self {
        Button { id: id.into(), label: label.into(), intent: intent.into(), params: Vec::new(), toggle: None }
    }

    /// A toggle that shows `on` and runs `intent` with the new state.
    pub fn toggle(id: &str, label: &str, intent: &str, on: bool) -> Self {
        Button { toggle: Some(on), ..Button::new(id, label, intent) }
    }

    /// Adds a string parameter the intent receives.
    pub fn param(mut self, name: &str, value: &str) -> Self {
        self.params.push((name.into(), value.into()));
        self
    }
}

/// What one widget size shows instead of the entry's title, text or detail (the reference view's switch on
/// `widgetFamily`). A part left as None keeps the entry's.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Variant {
    pub title: Option<String>,
    pub text: Option<String>,
    pub detail: Option<String>,
}

/// An [`Entry`] with the richer parts of a widget: live date text, buttons, a detail line and per-size variants.
#[derive(Clone, Debug, PartialEq)]
pub struct Rich {
    pub entry: Entry,
    /// live date text: `{}` in the entry's text marks where it goes, else it follows the text; it keeps counting while
    /// the app does not run
    pub clock: Option<(ClockStyle, f64)>,
    pub buttons: Vec<Button>,
    /// a third line under the text, shown by the Medium and Large sizes
    pub detail: Option<String>,
    /// what each size shows instead of the entry's own parts
    pub variants: Vec<(Family, Variant)>,
    /// the system colour the widget's tile is filled with; None keeps the glass tile
    pub background: Option<ContainerTint>,
}

impl Rich {
    pub fn new(entry: Entry) -> Self {
        Rich { entry, clock: None, buttons: Vec::new(), detail: None, variants: Vec::new(), background: None }
    }

    /// Fills the widget's tile with a system colour; its text turns white (dark on yellow and mint).
    pub fn with_background(mut self, tint: ContainerTint) -> Self {
        self.background = Some(tint);
        self
    }

    /// Adds the detail line the Medium and Large sizes show under the text.
    pub fn with_detail(mut self, detail: &str) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// Gives `family` its own title, text or detail; a later call for the same size replaces it.
    pub fn with_variant(mut self, family: Family, variant: Variant) -> Self {
        self.variants.retain(|(f, _)| *f != family);
        self.variants.push((family, variant));
        self
    }

    /// Shows the distance to `target` (unix seconds) as live text in `style`.
    pub fn with_clock(mut self, style: ClockStyle, target: f64) -> Self {
        self.clock = Some((style, target));
        self
    }

    pub fn with_button(mut self, button: Button) -> Self {
        self.buttons.push(button);
        self
    }
}

/// A widget kind the app offers: its name and description in the Edit Widgets gallery and the sizes it supports.
#[derive(Clone, Debug, PartialEq)]
pub struct Kind {
    pub id: String,
    pub name: String,
    pub description: String,
    pub families: Vec<Family>,
    pub configurable: bool,
}

impl Kind {
    /// A kind offered at Small and Medium.
    pub fn new(id: &str, name: &str) -> Self {
        Kind { id: id.into(), name: name.into(), description: String::new(), families: vec![Family::Small, Family::Medium], configurable: false }
    }

    pub fn with_description(mut self, description: &str) -> Self {
        self.description = description.into();
        self
    }

    /// The sizes the kind supports (replaces Small and Medium).
    pub fn with_families(mut self, families: &[Family]) -> Self {
        self.families = families.to_vec();
        self
    }
}

/// Registers the widget kinds the app offers, so the Edit Widgets gallery lists them (with their sizes) before a first
/// [`reload`]. The reply is the kind count. Apps can also declare them as `[[widget]]` tables in `manifest.toml`.
pub fn register(ctx: &Ctx, kinds: &[Kind]) -> u32 {
    service::call(ctx, SERVICE, "register", &register_body(kinds))
}

/// [`reload`] with live date text and buttons on the entries, and an optional `bifrost-app://app/path` link a tap on
/// the widget opens.
pub fn reload_rich(ctx: &Ctx, kind: &str, entries: &[Rich], policy: ReloadPolicy, link: Option<&str>) -> u32 {
    service::call(ctx, SERVICE, "reload", &rich_reload_body(kind, entries, policy, link))
}

/// Asks the shell for a new timeline for each of this app's widgets of `kind` in the column (the reference
/// `WidgetCenter` `reloadTimelines(ofKind:)`): the shell answers with one [`Event::Reload`] per widget on its next
/// tick, however often the app asked before then. The reply is how many widgets will be asked.
pub fn reload_timelines(ctx: &Ctx, kind: &str) -> u32 {
    service::call(ctx, SERVICE, "reload_timelines", &obj(&[("kind", Json::Str(kind.into()))]))
}

/// [`reload_timelines`] for every kind this app offers (the reference `reloadAllTimelines()`).
pub fn reload_all_timelines(ctx: &Ctx) -> u32 {
    service::call(ctx, SERVICE, "reload_all_timelines", &Json::Null)
}

/// Asks which of this app's widgets the person has in the column (the reference `getCurrentConfigurations`). The reply
/// is `[{kind, family, configuration: {key: value}}]`; read it with [`parse_configurations`].
pub fn current_configurations(ctx: &Ctx) -> u32 {
    service::call(ctx, SERVICE, "current_configurations", &Json::Null)
}

/// One widget the person has placed (the reference `WidgetInfo`): its kind, its size and its option values, a choice
/// by its name.
#[derive(Clone, Debug, PartialEq)]
pub struct WidgetInfo {
    pub kind: String,
    pub family: Family,
    pub configuration: Vec<(String, Json)>,
}

/// Decodes the reply to [`current_configurations`]. Rows without a kind or with an unknown size are skipped.
pub fn parse_configurations(reply: &Json) -> Vec<WidgetInfo> {
    reply
        .as_array()
        .unwrap_or_default()
        .iter()
        .filter_map(|r| {
            let kind = r.str_field("kind")?;
            let family = r.get("family").and_then(Json::as_str).and_then(Family::from_key)?;
            let configuration = match r.get("configuration") {
                Some(Json::Obj(pairs)) => pairs.clone(),
                _ => Vec::new(),
            };
            Some(WidgetInfo { kind, family, configuration })
        })
        .collect()
}

/// Drops the timeline `kind`.
pub fn remove(ctx: &Ctx, kind: &str) -> u32 {
    service::call(ctx, SERVICE, "remove", &obj(&[("kind", Json::Str(kind.into()))]))
}

/// Asks for the entry on screen for `kind` now. The reply is `{date, title, text}` or null.
pub fn current(ctx: &Ctx, kind: &str) -> u32 {
    service::call(ctx, SERVICE, "current", &obj(&[("kind", Json::Str(kind.into()))]))
}

/// Starts a live activity showing `title` and `text`. `stale_at` marks when its content is out of date; `start_at`
/// makes it pending until then. The reply is the activity id.
pub fn start_activity(ctx: &Ctx, title: &str, text: &str, stale_at: Option<u64>, start_at: Option<u64>) -> u32 {
    let mut pairs = content_pairs(title, text, stale_at);
    if let Some(t) = start_at {
        pairs.push(("start_at".into(), Json::Num(t as f64)));
    }
    service::call(ctx, SERVICE, "start_activity", &Json::Obj(pairs))
}

/// Replaces the content of a live activity this app started.
pub fn update_activity(ctx: &Ctx, id: u32, title: &str, text: &str, stale_at: Option<u64>) -> u32 {
    let mut pairs = content_pairs(title, text, stale_at);
    pairs.insert(0, ("id".into(), Json::Num(id as f64)));
    service::call(ctx, SERVICE, "update_activity", &Json::Obj(pairs))
}

/// Ends a live activity this app started. `final_content` replaces its content first; `dismissal` says when it leaves
/// the screen.
pub fn end_activity(ctx: &Ctx, id: u32, final_content: Option<(&str, &str)>, dismissal: Dismissal) -> u32 {
    service::call(ctx, SERVICE, "end_activity", &end_body(id, final_content, dismissal))
}

/// Lists this app's live activities on screen: `[{id, state, title, text}]`.
pub fn activities(ctx: &Ctx) -> u32 {
    service::call(ctx, SERVICE, "activities", &Json::Null)
}

/// Lists this app's widgets as the person arranged them: `[{kind, on_screen, large, configurable, title?}]`.
pub fn widgets(ctx: &Ctx) -> u32 {
    service::call(ctx, SERVICE, "widgets", &Json::Null)
}

/// Starts (`on`) or stops observing the widget `kind`: the shell then sends a [`Event::Visibility`] each time the
/// person adds, removes or resizes it.
pub fn observe(ctx: &Ctx, kind: &str, on: bool) -> u32 {
    service::call(ctx, SERVICE, "observe", &obj(&[("kind", Json::Str(kind.into())), ("on", Json::Bool(on))]))
}

/// The method name and JSON body of a `reload` call. Split out so the encoding can be checked without a context.
fn reload_call(kind: &str, entries: &[Entry], policy: ReloadPolicy) -> (&'static str, Json) {
    let (name, reload_at) = match policy {
        ReloadPolicy::AtEnd => ("at_end", None),
        ReloadPolicy::Never => ("never", None),
        ReloadPolicy::After(t) => ("after", Some(t)),
    };
    let mut pairs = vec![
        ("kind".into(), Json::Str(kind.into())),
        ("policy".into(), Json::Str(name.into())),
        ("entries".into(), Json::Arr(entries.iter().map(entry_json).collect())),
    ];
    if let Some(t) = reload_at {
        pairs.push(("reload_at".into(), Json::Num(t)));
    }
    ("reload", Json::Obj(pairs))
}

/// The JSON body of an `end_activity` call.
fn end_body(id: u32, final_content: Option<(&str, &str)>, dismissal: Dismissal) -> Json {
    let mut pairs = vec![("id".into(), Json::Num(id as f64))];
    if let Some((title, text)) = final_content {
        pairs.push(("title".into(), Json::Str(title.into())));
        pairs.push(("text".into(), Json::Str(text.into())));
    }
    let (name, after) = match dismissal {
        Dismissal::Default => ("default", None),
        Dismissal::Immediate => ("immediate", None),
        Dismissal::After(t) => ("after", Some(t)),
    };
    pairs.push(("dismissal".into(), Json::Str(name.into())));
    if let Some(t) = after {
        pairs.push(("after".into(), Json::Num(t as f64)));
    }
    Json::Obj(pairs)
}

/// The JSON body of a `register` call.
fn register_body(kinds: &[Kind]) -> Json {
    let kind = |k: &Kind| {
        obj(&[
            ("id", Json::Str(k.id.clone())),
            ("name", Json::Str(k.name.clone())),
            ("description", Json::Str(k.description.clone())),
            ("families", Json::Arr(k.families.iter().map(|f| Json::Str(f.key().into())).collect())),
            ("configurable", Json::Bool(k.configurable)),
        ])
    };
    obj(&[("kinds", Json::Arr(kinds.iter().map(kind).collect()))])
}

/// The JSON body of a rich `reload` call: the plain call's fields, `clock` and `actions` on each entry, and `link`.
fn rich_reload_body(kind: &str, entries: &[Rich], policy: ReloadPolicy, link: Option<&str>) -> Json {
    let plain: Vec<Entry> = entries.iter().map(|r| r.entry.clone()).collect();
    let (_, mut body) = reload_call(kind, &plain, policy);
    if let Json::Obj(pairs) = &mut body {
        for (k, v) in pairs.iter_mut() {
            if k.as_str() == "entries" {
                let rich: Vec<Json> = entries.iter().map(rich_entry_json).collect();
                *v = Json::Arr(rich);
            }
        }
        if let Some(l) = link {
            pairs.push(("link".into(), Json::Str(l.into())));
        }
    }
    body
}

fn rich_entry_json(r: &Rich) -> Json {
    let mut pairs = match entry_json(&r.entry) {
        Json::Obj(p) => p,
        _ => Vec::new(),
    };
    if let Some((style, target)) = r.clock {
        let key = match style {
            ClockStyle::Relative => "relative",
            ClockStyle::Timer => "timer",
        };
        pairs.push(("clock".into(), obj(&[("style", Json::Str(key.into())), ("target", Json::Num(target))])));
    }
    if !r.buttons.is_empty() {
        pairs.push(("actions".into(), Json::Arr(r.buttons.iter().map(button_json).collect())));
    }
    if let Some(d) = &r.detail {
        pairs.push(("detail".into(), Json::Str(d.clone())));
    }
    if let Some(t) = r.background {
        pairs.push(("background".into(), Json::Str(t.key().into())));
    }
    if !r.variants.is_empty() {
        let sizes = r
            .variants
            .iter()
            .map(|(f, v)| {
                let parts = [("title", &v.title), ("text", &v.text), ("detail", &v.detail)];
                let fields = parts.iter().filter_map(|(k, p)| p.as_ref().map(|t| (k.to_string(), Json::Str(t.clone())))).collect();
                (f.key().to_string(), Json::Obj(fields))
            })
            .collect();
        pairs.push(("families".into(), Json::Obj(sizes)));
    }
    Json::Obj(pairs)
}

fn button_json(b: &Button) -> Json {
    let mut pairs = vec![
        ("id".to_string(), Json::Str(b.id.clone())),
        ("label".to_string(), Json::Str(b.label.clone())),
        ("intent".to_string(), Json::Str(b.intent.clone())),
    ];
    if !b.params.is_empty() {
        pairs.push(("params".into(), Json::Obj(b.params.iter().map(|(k, v)| (k.clone(), Json::Str(v.clone()))).collect())));
    }
    if let Some(on) = b.toggle {
        pairs.push(("toggle".into(), Json::Bool(on)));
    }
    Json::Obj(pairs)
}

fn entry_json(e: &Entry) -> Json {
    obj(&[("date", Json::Num(e.date)), ("title", Json::Str(e.title.clone())), ("text", Json::Str(e.text.clone()))])
}

fn content_pairs(title: &str, text: &str, stale_at: Option<u64>) -> Vec<(String, Json)> {
    let mut pairs = vec![("title".into(), Json::Str(title.into())), ("text".into(), Json::Str(text.into()))];
    if let Some(t) = stale_at {
        pairs.push(("stale_at".into(), Json::Num(t as f64)));
    }
    pairs
}

fn obj(pairs: &[(&str, Json)]) -> Json {
    Json::Obj(pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reload_call_encodes_the_entries_and_the_policy() {
        let e = Entry { date: 10.0, title: "Next".into(), text: "Standup".into() };
        let (method, body) = reload_call("next", std::slice::from_ref(&e), ReloadPolicy::After(300.0));
        assert_eq!(method, "reload");
        assert_eq!(body.str_field("kind").as_deref(), Some("next"));
        assert_eq!(body.str_field("policy").as_deref(), Some("after"));
        assert_eq!(body.get("reload_at").and_then(Json::number), Some(300.0));
        let first = &body.get("entries").and_then(Json::array).unwrap()[0];
        assert_eq!(first.get("date").and_then(Json::number), Some(10.0));
        assert_eq!(first.str_field("text").as_deref(), Some("Standup"));
    }

    #[test]
    fn a_rich_reload_adds_the_clock_buttons_and_link_to_the_plain_call() {
        let e = Entry { date: 10.0, title: "Next".into(), text: "starts {}".into() };
        let rich = Rich::new(e)
            .with_clock(ClockStyle::Relative, 130.0)
            .with_button(Button::new("done", "Done", "mark-done").param("item", "7"))
            .with_button(Button::toggle("mute", "Mute", "set-mute", false));
        let body = rich_reload_body("next", std::slice::from_ref(&rich), ReloadPolicy::After(300.0), Some("bifrost-app://is.x/next"));
        assert_eq!(body.str_field("kind").as_deref(), Some("next"));
        assert_eq!(body.str_field("policy").as_deref(), Some("after"), "the plain call's fields stay");
        assert_eq!(body.get("reload_at").and_then(Json::number), Some(300.0));
        assert_eq!(body.str_field("link").as_deref(), Some("bifrost-app://is.x/next"));
        let first = &body.get("entries").and_then(Json::array).unwrap()[0];
        assert_eq!(first.str_field("text").as_deref(), Some("starts {}"));
        let clock = first.get("clock").unwrap();
        assert_eq!((clock.str_field("style").as_deref(), clock.get("target").and_then(Json::number)), (Some("relative"), Some(130.0)));
        let actions = first.get("actions").and_then(Json::array).unwrap();
        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0].str_field("intent").as_deref(), Some("mark-done"));
        assert_eq!(actions[0].get("params").and_then(|p| p.str_field("item")).as_deref(), Some("7"));
        assert!(actions[0].get("toggle").is_none(), "a button has no toggle state");
        assert_eq!(actions[1].get("toggle"), Some(&Json::Bool(false)));
    }

    #[test]
    fn a_rich_entry_carries_its_detail_and_one_variant_per_size() {
        let e = Entry { date: 0.0, title: "Next".into(), text: "Standup 9:00".into() };
        let rich = Rich::new(e)
            .with_detail("Room 4")
            .with_variant(Family::Small, Variant { text: Some("old".into()), ..Variant::default() })
            .with_variant(Family::Small, Variant { text: Some("9:00".into()), ..Variant::default() })
            .with_variant(Family::Large, Variant { title: Some("Today".into()), ..Variant::default() });
        let j = rich_entry_json(&rich);
        assert_eq!(j.str_field("detail").as_deref(), Some("Room 4"));
        let sizes = j.get("families").unwrap();
        assert_eq!(sizes.get("small").and_then(|v| v.str_field("text")).as_deref(), Some("9:00"), "the later variant replaces the first");
        assert!(sizes.get("small").and_then(|v| v.get("title")).is_none(), "a part left out is not sent");
        assert_eq!(sizes.get("large").and_then(|v| v.str_field("title")).as_deref(), Some("Today"));
        assert!(j.get("background").is_none(), "no background unless asked");
        let tinted = rich_entry_json(&rich.with_background(ContainerTint::Teal));
        assert_eq!(tinted.str_field("background").as_deref(), Some("teal"));
    }

    #[test]
    fn a_rich_entry_without_extras_encodes_like_a_plain_one_and_has_no_link() {
        let e = Entry { date: 1.0, title: "t".into(), text: "x".into() };
        let body = rich_reload_body("k", &[Rich::new(e.clone())], ReloadPolicy::Never, None);
        let first = &body.get("entries").and_then(Json::array).unwrap()[0];
        assert_eq!(first, &entry_json(&e));
        assert!(body.get("link").is_none());
    }

    #[test]
    fn a_register_call_names_each_kind_with_its_sizes() {
        let kinds = [
            Kind::new("next", "Next Event").with_description("The next one.").with_families(&[Family::Small, Family::Large]),
            Kind::new("plain", "Plain"),
        ];
        let body = register_body(&kinds);
        let list = body.get("kinds").and_then(Json::array).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].str_field("name").as_deref(), Some("Next Event"));
        assert_eq!(list[0].str_field("description").as_deref(), Some("The next one."));
        let sizes: Vec<String> = list[0].get("families").and_then(Json::array).unwrap().iter().filter_map(|f| f.as_str().map(str::to_string)).collect();
        assert_eq!(sizes, vec!["small".to_string(), "large".to_string()]);
        let default_sizes: Vec<&str> = list[1].get("families").and_then(Json::array).unwrap().iter().filter_map(Json::as_str).collect();
        assert_eq!(default_sizes, vec!["small", "medium"]);
    }

    #[test]
    fn the_current_configurations_reply_decodes_into_widget_infos() {
        let reply = Json::Arr(vec![
            obj(&[("kind", Json::Str("next".into())), ("family", Json::Str("medium".into())), ("configuration", obj(&[("city", Json::Str("Oslo".into()))]))]),
            obj(&[("kind", Json::Str("bad".into())), ("family", Json::Str("huge".into()))]),
            obj(&[("family", Json::Str("small".into()))]),
        ]);
        let infos = parse_configurations(&reply);
        assert_eq!(infos.len(), 1, "an unknown size or a missing kind is skipped");
        assert_eq!((infos[0].kind.as_str(), infos[0].family), ("next", Family::Medium));
        assert_eq!(infos[0].configuration, vec![("city".to_string(), Json::Str("Oslo".into()))]);
        assert!(parse_configurations(&Json::Null).is_empty());
    }

    #[test]
    fn the_end_of_timeline_and_never_policies_send_no_reload_date() {
        let (_, body) = reload_call("k", &[], ReloadPolicy::AtEnd);
        assert_eq!(body.str_field("policy").as_deref(), Some("at_end"));
        assert!(body.get("reload_at").is_none());
        let (_, body) = reload_call("k", &[], ReloadPolicy::Never);
        assert_eq!(body.str_field("policy").as_deref(), Some("never"));
    }

    #[test]
    fn the_reload_event_is_decoded_for_this_service_only() {
        let body = obj(&[("kind", Json::Str("next".into()))]);
        assert_eq!(parse_event(SERVICE, EVENT_RELOAD, &body), Some(Event::Reload { kind: "next".into() }));
        assert_eq!(parse_event("dock", EVENT_RELOAD, &body), None, "another service's event is not ours");
        assert_eq!(parse_event(SERVICE, "other", &body), None);
        assert_eq!(parse_event(SERVICE, EVENT_RELOAD, &Json::Null), None, "a reload event names its kind");
    }

    #[test]
    fn the_visibility_event_carries_the_widget_state_and_needs_all_of_it() {
        let body = Json::Obj(vec![
            ("kind".into(), Json::Str("next".into())),
            ("on_screen".into(), Json::Bool(true)),
            ("large".into(), Json::Bool(false)),
        ]);
        assert_eq!(parse_event(SERVICE, EVENT_VISIBILITY, &body), Some(Event::Visibility { kind: "next".into(), on_screen: true, large: false }));
        let partial = obj(&[("kind", Json::Str("next".into()))]);
        assert_eq!(parse_event(SERVICE, EVENT_VISIBILITY, &partial), None);
        assert_eq!(parse_event("dock", EVENT_VISIBILITY, &body), None);
    }

    #[test]
    fn the_opened_event_names_the_widget_and_its_size() {
        let body = obj(&[("kind", Json::Str("next".into())), ("family", Json::Str("medium".into()))]);
        assert_eq!(parse_event(SERVICE, EVENT_OPENED, &body), Some(Event::Opened { kind: "next".into(), family: Family::Medium }));
        assert_eq!(parse_event(SERVICE, EVENT_OPENED, &obj(&[("kind", Json::Str("next".into()))])), None, "it names a size");
    }

    #[test]
    fn an_end_call_carries_the_final_content_and_the_dismissal_time() {
        let body = end_body(7, Some(("Ride", "Done")), Dismissal::After(42));
        assert_eq!(body.get("id").and_then(Json::number), Some(7.0));
        assert_eq!(body.str_field("title").as_deref(), Some("Ride"));
        assert_eq!(body.str_field("dismissal").as_deref(), Some("after"));
        assert_eq!(body.get("after").and_then(Json::number), Some(42.0));
        let plain = end_body(8, None, Dismissal::Immediate);
        assert_eq!(plain.str_field("dismissal").as_deref(), Some("immediate"));
        assert!(plain.get("title").is_none() && plain.get("after").is_none());
    }
}
