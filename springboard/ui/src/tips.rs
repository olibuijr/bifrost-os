//! Contextual tips: short hints that point at a feature the person has not found yet.
//!
//! A tip has content (title, message, actions) and rules that say when it may show. A rule counts named events or
//! compares a named parameter. The `Datastore` keeps the event counts, the parameter values, which tips were
//! invalidated (acted on or dismissed), how often each was shown and when a tip last showed. It is written to a
//! text file with an atomic replace, so an interrupted save keeps the previous state. `DisplayFrequency` sets how
//! long after a shown tip another tip may show. A `TipGroup` presents one tip of several, either in order or the
//! first eligible one. Testing overrides force tips to show or hide. Nothing is drawn here: the caller passes
//! `now` in seconds, reads the answer and draws the tip.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::PathBuf;

/// Seconds since an epoch chosen by the caller (the shell passes the wall clock).
pub type Seconds = u64;

/// A button a tip offers, such as "Show Me".
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Action {
    pub id: String,
    pub title: String,
}

impl Action {
    pub fn new(id: &str, title: &str) -> Self {
        Action { id: id.to_string(), title: title.to_string() }
    }
}

/// A parameter value. Rules compare values for equality.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Bool(bool),
    Int(i64),
    Str(String),
}

/// A condition that must hold before a tip may show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rule {
    /// The named event has been donated at least `at_least` times.
    EventCount { event: String, at_least: u32 },
    /// The named parameter currently holds `equals`.
    Parameter { name: String, equals: Value },
}

/// The minimum gap between one shown tip and the next. Our names; the default is `Immediate`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplayFrequency {
    Immediate,
    Hourly,
    Daily,
    Weekly,
    Monthly,
}

impl DisplayFrequency {
    pub fn seconds(self) -> Seconds {
        match self {
            DisplayFrequency::Immediate => 0,
            DisplayFrequency::Hourly => 3_600,
            DisplayFrequency::Daily => 86_400,
            DisplayFrequency::Weekly => 604_800,
            DisplayFrequency::Monthly => 2_592_000,
        }
    }
}

/// A tip: content plus the rules that decide when it displays. Ids must be stable, since the datastore keys on them.
pub trait Tip {
    fn id(&self) -> &str;
    fn title(&self) -> &str;
    fn message(&self) -> &str {
        ""
    }
    fn actions(&self) -> Vec<Action> {
        Vec::new()
    }
    /// Every rule must hold; an empty list is always satisfied.
    fn rules(&self) -> Vec<Rule> {
        Vec::new()
    }
}

/// One setting for `configure`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfigOption {
    DisplayFrequency(DisplayFrequency),
    /// The file the datastore loads from and saves to. Without it the datastore lives in memory only.
    DatastorePath(PathBuf),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TipError {
    /// The datastore file could not be read or written; the message names the path.
    Datastore(String),
}

impl fmt::Display for TipError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TipError::Datastore(m) => write!(f, "tip datastore: {m}"),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct TipState {
    invalidated: bool,
    shown: u32,
}

/// Testing overrides. `hide` beats `show`, and `hide_all` beats `show_all`; overrides ignore rules and frequency.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Testing {
    show_all: bool,
    hide_all: bool,
    show: BTreeSet<String>,
    hide: BTreeSet<String>,
}

/// The persistent tip state of one app.
#[derive(Debug)]
pub struct Datastore {
    frequency: DisplayFrequency,
    path: Option<PathBuf>,
    events: BTreeMap<String, u32>,
    params: BTreeMap<String, Value>,
    tips: BTreeMap<String, TipState>,
    last_shown: Option<Seconds>,
    testing: Testing,
}

/// Loads the datastore for the given options. A missing file is an empty store; a file that exists but cannot be
/// read is an error. Malformed lines in the file are skipped.
pub fn configure(options: &[ConfigOption]) -> Result<Datastore, TipError> {
    let mut frequency = DisplayFrequency::Immediate;
    let mut path = None;
    for option in options {
        match option {
            ConfigOption::DisplayFrequency(f) => frequency = *f,
            ConfigOption::DatastorePath(p) => path = Some(p.clone()),
        }
    }
    let mut store = Datastore {
        frequency,
        path: path.clone(),
        events: BTreeMap::new(),
        params: BTreeMap::new(),
        tips: BTreeMap::new(),
        last_shown: None,
        testing: Testing::default(),
    };
    if let Some(p) = path {
        match std::fs::read_to_string(&p) {
            Ok(text) => store.parse(&text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(TipError::Datastore(format!("{}: {e}", p.display()))),
        }
    }
    Ok(store)
}

impl Datastore {
    /// Donates one occurrence of a user-defined event and saves.
    pub fn donate(&mut self, event: &str) -> Result<(), TipError> {
        *self.events.entry(event.to_string()).or_insert(0) += 1;
        self.save()
    }

    pub fn event_count(&self, event: &str) -> u32 {
        self.events.get(event).copied().unwrap_or(0)
    }

    /// Sets a parameter and saves.
    pub fn set_parameter(&mut self, name: &str, value: Value) -> Result<(), TipError> {
        self.params.insert(name.to_string(), value);
        self.save()
    }

    pub fn parameter(&self, name: &str) -> Option<&Value> {
        self.params.get(name)
    }

    /// Records that a tip was shown at `now`; it starts the display-frequency gap. Saves.
    pub fn mark_shown(&mut self, id: &str, now: Seconds) -> Result<(), TipError> {
        self.tips.entry(id.to_string()).or_default().shown += 1;
        self.last_shown = Some(now);
        self.save()
    }

    pub fn shown_count(&self, id: &str) -> u32 {
        self.tips.get(id).map_or(0, |t| t.shown)
    }

    /// The person acted on or dismissed the tip, so it never returns until the datastore is reset. Saves.
    pub fn invalidate(&mut self, id: &str) -> Result<(), TipError> {
        self.tips.entry(id.to_string()).or_default().invalidated = true;
        self.save()
    }

    pub fn is_invalidated(&self, id: &str) -> bool {
        self.tips.get(id).is_some_and(|t| t.invalidated)
    }

    /// Clears events, parameters, tip state and the display gap. Keeps the frequency, the path and the testing
    /// overrides. Saves.
    pub fn reset_datastore(&mut self) -> Result<(), TipError> {
        self.events.clear();
        self.params.clear();
        self.tips.clear();
        self.last_shown = None;
        self.save()
    }

    pub fn show_all_for_testing(&mut self) {
        self.testing.show_all = true;
    }

    pub fn hide_all_for_testing(&mut self) {
        self.testing.hide_all = true;
    }

    pub fn show_for_testing(&mut self, ids: &[&str]) {
        self.testing.show.extend(ids.iter().map(|s| s.to_string()));
    }

    pub fn hide_for_testing(&mut self, ids: &[&str]) {
        self.testing.hide.extend(ids.iter().map(|s| s.to_string()));
    }

    /// Removes every testing override.
    pub fn clear_testing(&mut self) {
        self.testing = Testing::default();
    }

    /// Whether a tip is hidden by a testing override.
    pub fn is_hidden(&self, id: &str) -> bool {
        self.testing.hide_all || self.testing.hide.contains(id)
    }

    fn is_forced_shown(&self, id: &str) -> bool {
        !self.is_hidden(id) && (self.testing.show_all || self.testing.show.contains(id))
    }

    /// A settled tip is one the person is done with: invalidated, or hidden for testing. A `TipGroup` in order moves
    /// past settled tips.
    pub fn is_settled(&self, id: &str) -> bool {
        self.is_invalidated(id) || self.is_hidden(id)
    }

    fn rule_holds(&self, rule: &Rule) -> bool {
        match rule {
            Rule::EventCount { event, at_least } => self.event_count(event) >= *at_least,
            Rule::Parameter { name, equals } => self.parameter(name) == Some(equals),
        }
    }

    /// Whether the tip may show at `now`: no hide override, not invalidated, every rule holds, and the display
    /// frequency has elapsed since the last shown tip. A show override skips all of these except the hide override.
    pub fn is_eligible(&self, tip: &dyn Tip, now: Seconds) -> bool {
        let id = tip.id();
        if self.is_hidden(id) {
            return false;
        }
        if self.is_forced_shown(id) {
            return true;
        }
        if self.is_invalidated(id) || !tip.rules().iter().all(|r| self.rule_holds(r)) {
            return false;
        }
        match self.last_shown {
            None => true,
            Some(t) => now.saturating_sub(t) >= self.frequency.seconds(),
        }
    }

    fn save(&self) -> Result<(), TipError> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let fail = |e: std::io::Error| TipError::Datastore(format!("{}: {e}", path.display()));
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(fail)?;
        }
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, self.to_text()).map_err(fail)?;
        std::fs::rename(&tmp, path).map_err(fail)
    }

    /// One tab-separated record per line; tabs, newlines and backslashes in fields are escaped.
    fn to_text(&self) -> String {
        let mut out = String::new();
        if let Some(t) = self.last_shown {
            push_line(&mut out, &["last_shown", &t.to_string()]);
        }
        for (name, n) in &self.events {
            push_line(&mut out, &["event", name, &n.to_string()]);
        }
        for (name, v) in &self.params {
            push_line(&mut out, &["param", name, &encode_value(v)]);
        }
        for (id, t) in &self.tips {
            push_line(&mut out, &["tip", id, if t.invalidated { "1" } else { "0" }, &t.shown.to_string()]);
        }
        out
    }

    fn parse(&mut self, text: &str) {
        for line in text.lines() {
            let f: Vec<String> = line.split('\t').map(unescape).collect();
            match f.first().map(String::as_str) {
                Some("last_shown") if f.len() == 2 => {
                    if let Ok(t) = f[1].parse() {
                        self.last_shown = Some(t);
                    }
                }
                Some("event") if f.len() == 3 => {
                    if let Ok(n) = f[2].parse() {
                        self.events.insert(f[1].clone(), n);
                    }
                }
                Some("param") if f.len() == 3 => {
                    if let Some(v) = decode_value(&f[2]) {
                        self.params.insert(f[1].clone(), v);
                    }
                }
                Some("tip") if f.len() == 4 => {
                    if let Ok(shown) = f[3].parse() {
                        self.tips.insert(f[1].clone(), TipState { invalidated: f[2] == "1", shown });
                    }
                }
                _ => {}
            }
        }
    }
}

/// Presentation order of a group (`TipGroup.Priority`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Priority {
    /// Shows the first tip in the list that is eligible now.
    FirstAvailable,
    /// Shows the first unsettled tip, and only once it is eligible. Later tips wait for it to be invalidated.
    Ordered,
}

/// A set of tips presented one at a time.
pub struct TipGroup<'a> {
    priority: Priority,
    tips: Vec<&'a dyn Tip>,
}

impl<'a> TipGroup<'a> {
    pub fn new(priority: Priority, tips: Vec<&'a dyn Tip>) -> Self {
        TipGroup { priority, tips }
    }

    /// Index of the tip to show now, or `None` when no tip may show.
    pub fn current(&self, store: &Datastore, now: Seconds) -> Option<usize> {
        match self.priority {
            Priority::FirstAvailable => self.tips.iter().position(|t| store.is_eligible(*t, now)),
            Priority::Ordered => {
                let i = self.tips.iter().position(|t| !store.is_settled(t.id()))?;
                store.is_eligible(self.tips[i], now).then_some(i)
            }
        }
    }
}

/// How an inline tip view is drawn. The default is `Mini`. Our names; the reference names are `TipViewStyle` with
/// `miniTip` and the standard style.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TipViewStyle {
    /// Title, message and every action.
    Standard,
    /// Title and message only. Actions are left to the surrounding view.
    #[default]
    Mini,
}

/// One block of a tip view, listed top to bottom.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TipSection {
    Title(String),
    Message(String),
    Actions(Vec<Action>),
}

/// The content of an inline tip view (`TipView`) for one tip. The view draws the sections it returns and draws
/// nothing when the tip may not show. Showing a tip is still the caller's job: call `Datastore::mark_shown` after
/// the view is drawn. The arrow and the anchor belong to the popover code, not to this model.
pub struct TipView<'a> {
    tip: &'a dyn Tip,
    style: TipViewStyle,
}

impl<'a> TipView<'a> {
    pub fn new(tip: &'a dyn Tip, style: TipViewStyle) -> Self {
        TipView { tip, style }
    }

    /// The sections to draw at `now`. Empty when the store says the tip may not show. An empty message is left out,
    /// and the actions are left out in the `Mini` style.
    pub fn sections(&self, store: &Datastore, now: Seconds) -> Vec<TipSection> {
        if !store.is_eligible(self.tip, now) {
            return Vec::new();
        }
        let mut out = vec![TipSection::Title(self.tip.title().to_string())];
        let message = self.tip.message();
        if !message.is_empty() {
            out.push(TipSection::Message(message.to_string()));
        }
        let actions = self.tip.actions();
        if self.style == TipViewStyle::Standard && !actions.is_empty() {
            out.push(TipSection::Actions(actions));
        }
        out
    }
}

fn push_line(out: &mut String, fields: &[&str]) {
    let escaped: Vec<String> = fields.iter().map(|f| escape(f)).collect();
    out.push_str(&escaped.join("\t"));
    out.push('\n');
}

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// Encodes a value as its kind letter, a colon and the text: `b:1`, `i:-4`, `s:Done`.
fn encode_value(v: &Value) -> String {
    match v {
        Value::Bool(b) => format!("b:{}", if *b { 1 } else { 0 }),
        Value::Int(n) => format!("i:{n}"),
        Value::Str(s) => format!("s:{s}"),
    }
}

fn decode_value(s: &str) -> Option<Value> {
    let rest = s.get(2..)?;
    match s.as_bytes().first()? {
        b'b' => match rest {
            "1" => Some(Value::Bool(true)),
            "0" => Some(Value::Bool(false)),
            _ => None,
        },
        b'i' => rest.parse().ok().map(Value::Int),
        b's' => Some(Value::Str(rest.to_string())),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Demo {
        id: &'static str,
        rules: Vec<Rule>,
    }

    impl Tip for Demo {
        fn id(&self) -> &str {
            self.id
        }
        fn title(&self) -> &str {
            "Demo"
        }
        fn rules(&self) -> Vec<Rule> {
            self.rules.clone()
        }
    }

    fn free(id: &'static str) -> Demo {
        Demo { id, rules: Vec::new() }
    }

    fn memory(freq: DisplayFrequency) -> Datastore {
        configure(&[ConfigOption::DisplayFrequency(freq)]).unwrap()
    }

    #[test]
    fn an_event_count_rule_needs_the_donations() {
        let mut store = memory(DisplayFrequency::Immediate);
        let tip = Demo { id: "export", rules: vec![Rule::EventCount { event: "saved".into(), at_least: 3 }] };
        store.donate("saved").unwrap();
        store.donate("saved").unwrap();
        assert!(!store.is_eligible(&tip, 0));
        store.donate("saved").unwrap();
        assert!(store.is_eligible(&tip, 0));
        assert_eq!(store.event_count("saved"), 3);
    }

    #[test]
    fn a_parameter_rule_compares_by_value() {
        let mut store = memory(DisplayFrequency::Immediate);
        let tip = Demo {
            id: "dark",
            rules: vec![Rule::Parameter { name: "mode".into(), equals: Value::Str("night".into()) }],
        };
        assert!(!store.is_eligible(&tip, 0));
        store.set_parameter("mode", Value::Str("day".into())).unwrap();
        assert!(!store.is_eligible(&tip, 0));
        store.set_parameter("mode", Value::Str("night".into())).unwrap();
        assert!(store.is_eligible(&tip, 0));
    }

    #[test]
    fn the_display_frequency_spaces_out_shown_tips() {
        let mut store = memory(DisplayFrequency::Daily);
        let a = free("a");
        let b = free("b");
        assert!(store.is_eligible(&a, 1_000));
        store.mark_shown("a", 1_000).unwrap();
        assert!(!store.is_eligible(&b, 1_100));
        assert!(store.is_eligible(&b, 1_000 + 86_400));
        assert_eq!(store.shown_count("a"), 1);
    }

    #[test]
    fn an_invalidated_tip_stays_away_until_the_datastore_is_reset() {
        let mut store = memory(DisplayFrequency::Immediate);
        let tip = free("tour");
        store.invalidate("tour").unwrap();
        assert!(store.is_invalidated("tour"));
        assert!(!store.is_eligible(&tip, 0));
        store.reset_datastore().unwrap();
        assert!(store.is_eligible(&tip, 0));
    }

    #[test]
    fn first_available_picks_the_first_eligible_tip() {
        let mut store = memory(DisplayFrequency::Immediate);
        let (a, b, c) = (
            Demo { id: "a", rules: vec![Rule::EventCount { event: "x".into(), at_least: 1 }] },
            free("b"),
            free("c"),
        );
        let group = TipGroup::new(Priority::FirstAvailable, vec![&a, &b, &c]);
        assert_eq!(group.current(&store, 0), Some(1));
        store.invalidate("b").unwrap();
        assert_eq!(group.current(&store, 0), Some(2));
        store.invalidate("c").unwrap();
        assert_eq!(group.current(&store, 0), None);
    }

    #[test]
    fn ordered_waits_for_the_earlier_tip_to_be_settled() {
        let mut store = memory(DisplayFrequency::Immediate);
        let first = Demo { id: "first", rules: vec![Rule::EventCount { event: "x".into(), at_least: 1 }] };
        let second = free("second");
        let group = TipGroup::new(Priority::Ordered, vec![&first, &second]);
        // The first tip is not eligible yet, and the second may not jump ahead of it.
        assert_eq!(group.current(&store, 0), None);
        store.donate("x").unwrap();
        assert_eq!(group.current(&store, 0), Some(0));
        store.invalidate("first").unwrap();
        assert_eq!(group.current(&store, 0), Some(1));
    }

    #[test]
    fn testing_overrides_force_tips_and_hide_wins() {
        let mut store = memory(DisplayFrequency::Daily);
        let locked = Demo { id: "locked", rules: vec![Rule::EventCount { event: "never".into(), at_least: 9 }] };
        let other = free("other");
        store.show_for_testing(&["locked"]);
        assert!(store.is_eligible(&locked, 0));
        store.hide_for_testing(&["locked"]);
        assert!(!store.is_eligible(&locked, 0));
        store.clear_testing();
        store.hide_all_for_testing();
        assert!(!store.is_eligible(&other, 0));
        store.clear_testing();
        store.show_all_for_testing();
        store.mark_shown("other", 0).unwrap();
        assert!(store.is_eligible(&other, 1), "show-all skips the display frequency gap");
    }

    #[test]
    fn the_datastore_round_trips_through_its_file() {
        let dir = std::env::temp_dir().join(format!("bifrost-tips-{}", std::process::id()));
        let path = dir.join("tips.txt");
        let _ = std::fs::remove_file(&path);
        {
            let mut store = configure(&[ConfigOption::DatastorePath(path.clone())]).unwrap();
            store.donate("opened\tfile").unwrap();
            store.set_parameter("note", Value::Str("two\nlines\\".into())).unwrap();
            store.set_parameter("count", Value::Int(-4)).unwrap();
            store.mark_shown("tour", 42).unwrap();
            store.invalidate("tour").unwrap();
        }
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, format!("{text}garbage line\nparam\tbad\tq:1\n")).unwrap();
        let store = configure(&[ConfigOption::DatastorePath(path.clone())]).unwrap();
        assert_eq!(store.event_count("opened\tfile"), 1);
        assert_eq!(store.parameter("note"), Some(&Value::Str("two\nlines\\".into())));
        assert_eq!(store.parameter("count"), Some(&Value::Int(-4)));
        assert_eq!(store.parameter("bad"), None);
        assert_eq!(store.shown_count("tour"), 1);
        assert!(store.is_invalidated("tour"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_file_is_an_empty_store_and_a_directory_is_an_error() {
        let dir = std::env::temp_dir().join(format!("bifrost-tips-dir-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let store = configure(&[ConfigOption::DatastorePath(dir.join("absent.txt"))]).unwrap();
        assert_eq!(store.event_count("anything"), 0);
        let err = configure(&[ConfigOption::DatastorePath(dir.clone())]);
        assert!(matches!(err, Err(TipError::Datastore(_))));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod tip_view_tests {
    use super::*;

    struct Full {
        message: &'static str,
        actions: Vec<Action>,
        rules: Vec<Rule>,
    }

    impl Tip for Full {
        fn id(&self) -> &str {
            "tip.full"
        }
        fn title(&self) -> &str {
            "Demo"
        }
        fn message(&self) -> &str {
            self.message
        }
        fn actions(&self) -> Vec<Action> {
            self.actions.clone()
        }
        fn rules(&self) -> Vec<Rule> {
            self.rules.clone()
        }
    }

    fn tip(message: &'static str) -> Full {
        Full { message, actions: vec![Action::new("show", "Show Me")], rules: Vec::new() }
    }

    #[test]
    fn the_standard_style_draws_title_message_and_actions_in_order() {
        let store = configure(&[]).unwrap();
        let t = tip("Try it");
        let view = TipView::new(&t, TipViewStyle::Standard);
        assert_eq!(
            view.sections(&store, 0),
            vec![
                TipSection::Title("Demo".to_string()),
                TipSection::Message("Try it".to_string()),
                TipSection::Actions(vec![Action::new("show", "Show Me")]),
            ]
        );
    }

    #[test]
    fn the_mini_style_is_the_default_and_leaves_out_the_actions() {
        assert_eq!(TipViewStyle::default(), TipViewStyle::Mini);
        let store = configure(&[]).unwrap();
        let t = tip("Try it");
        let sections = TipView::new(&t, TipViewStyle::default()).sections(&store, 0);
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[1], TipSection::Message("Try it".to_string()));
    }

    #[test]
    fn an_empty_message_and_no_actions_leave_only_the_title() {
        let store = configure(&[]).unwrap();
        let t = Full { message: "", actions: Vec::new(), rules: Vec::new() };
        let sections = TipView::new(&t, TipViewStyle::Standard).sections(&store, 0);
        assert_eq!(sections, vec![TipSection::Title("Demo".to_string())]);
    }

    #[test]
    fn the_view_is_empty_until_the_event_rule_holds() {
        let mut store = configure(&[]).unwrap();
        let t = Full {
            message: "Try it",
            actions: Vec::new(),
            rules: vec![Rule::EventCount { event: "open".to_string(), at_least: 2 }],
        };
        let view = TipView::new(&t, TipViewStyle::Standard);
        assert!(view.sections(&store, 0).is_empty());
        store.donate("open").unwrap();
        assert!(view.sections(&store, 0).is_empty());
        store.donate("open").unwrap();
        assert_eq!(view.sections(&store, 0).len(), 2);
    }

    #[test]
    fn a_shown_tip_hides_until_the_daily_gap_has_passed() {
        let mut store = configure(&[ConfigOption::DisplayFrequency(DisplayFrequency::Daily)]).unwrap();
        let t = tip("Try it");
        store.mark_shown("tip.full", 1_000).unwrap();
        let view = TipView::new(&t, TipViewStyle::Mini);
        assert!(view.sections(&store, 1_100).is_empty());
        assert_eq!(view.sections(&store, 1_000 + 86_400).len(), 2);
    }
}
