//! The `events` service: input events for apps that hold the `accessibility` permission. It posts synthetic events
//! into the shell's input path and installs session-level event taps on it, and it answers questions about the
//! input state tables. Every input event the shell receives (pointer, hover and key) runs through [`InputTaps`] before
//! the shell acts on it; with no tap installed that costs a state-table update and nothing else. The model is
//! `bifrost_ui::event_tap`.
//!
//! Methods (JSON parameters, all need the `accessibility` permission declared in the manifest and switched on by the
//! person in Settings > Privacy & Security > App Permissions, otherwise the call fails; nothing is posted while a
//! system sheet or alert is up, so an app can never answer a permission or security sheet):
//! - `post {event}` posts an event into the session stream. It runs through the taps like a real one (a tap can drop
//!   it) and then reaches the shell. Pointer kinds act as a touch or a hover at the event's location (points); a
//!   right-button press opens the context menu; keys go through the shell's key path with the modifiers of the event
//!   pressed around them; a scroll wheel event becomes a short drag. Nothing is posted while the screen is locked.
//! - `tap_create {mask, options, placement, rules}` installs a tap. `mask` is a list of event kind names, `options`
//!   `"filter"` or `"listen"`, `placement` `"head"` or `"tail"`, `rules` the filter rules of a filtering tap (first match
//!   wins; no rule passes). The shell cannot wait for an app's answer in the middle of an input event, so a filtering
//!   tap decides by its rules and still tells the app what it saw. Answers `{tap}`.
//! - `tap_enable {tap, enabled}`, `tap_remove {tap}`, `tap_list {}` manage the caller's own taps.
//! - `state {source, query, ...}` reads a state table: `query` is `flags`, `key {key_code}`, `button {button}`,
//!   `since {kind}` (seconds since the last event of that kind) or `counter {kind}`.
//! - `location {}` answers the last pointer location in points.
//!
//! Every event a tap sees is pushed to its owner as the `tap` event `{tap, action, event}` on the next tick, where
//! `action` is `listen`, `pass`, `drop` or `replace`. A tap goes away with its app. The power, volume and wake keys and
//! everything typed while the screen is locked never reach a tap.
use super::{app_has_permission, app_pid, Event as ServiceEvent, Shell, SystemService};
use crate::shell::apphost;
use crate::shell::evkeys;
use bifrost_core::json::Json;
use bifrost_ui::event_tap::*;
use bifrost_ui::wire_json::*;
use bifrost_ui::Event as UiEvent;
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;

pub const SERVICE: &str = "events";
/// The manifest permission that allows posting events and installing taps.
pub const PERMISSION: &str = "accessibility";
/// The event a tap's owner receives for each event the tap saw.
pub const TAP_EVENT: &str = "tap";

const MAX_NOTICES: usize = 512;
const MAX_NOTICES_PER_TICK: usize = 64;
const MAX_TAPS_PER_APP: usize = 8;
const MAX_POSTED: usize = 512;
/// Lines of scroll that equal one point of drag, times the pixel size of a line.
const SCROLL_LINE_POINTS: f32 = 40.0;
/// Keys the shell keeps for itself: power, volume and wake.
const RESERVED_KEYS: [u16; 4] = [114, 115, 116, 143];
const MODIFIER_KEYS: [(u32, u16); 4] = [
    (EventFlags::SHIFT, 42),
    (EventFlags::CONTROL, 29),
    (EventFlags::ALT, 56),
    (EventFlags::COMMAND, 125),
];

/// What a posted event asks the shell to do next; the main loop feeds each into the real input path.
#[derive(Clone, Debug, PartialEq)]
pub enum Posted {
    Key(u16, bool),
    Pointer(UiEvent),
    Hover(f32, f32),
    Context(f32, f32),
}

struct Notice {
    app: String,
    tap: u32,
    action: &'static str,
    event: Event,
}

struct TapInfo {
    id: u32,
    app: String,
    mask: EventMask,
    options: TapOptions,
}

/// The taps, the state tables and the queue of posted input of one shell.
pub struct InputTaps {
    list: EventTapList,
    info: Vec<TapInfo>,
    outbox: Rc<RefCell<VecDeque<Notice>>>,
    tables: SourceTables,
    location: (f32, f32),
    dragging: bool,
    posted: VecDeque<Posted>,
    /// Set while a posted event enters the shell, so it does not pass through the taps a second time.
    bypass: bool,
    epoch: std::time::Instant,
}

impl Default for InputTaps {
    fn default() -> Self {
        InputTaps {
            list: EventTapList::new(),
            info: Vec::new(),
            outbox: Rc::new(RefCell::new(VecDeque::new())),
            tables: SourceTables::new(),
            location: (0.0, 0.0),
            dragging: false,
            posted: VecDeque::new(),
            bypass: false,
            epoch: std::time::Instant::now(),
        }
    }
}

impl InputTaps {
    /// Nanoseconds since the shell started: the clock of every event the shell creates.
    pub fn now_ns(&self) -> u64 {
        self.epoch.elapsed().as_nanos() as u64
    }

    /// Runs an event of source `state` through the taps. The hardware table sees every real event; the session table
    /// sees what survives the taps. `None` when a tap dropped the event.
    pub fn process(&mut self, mut e: Event, state: SourceState) -> Option<Event> {
        e.source = state;
        if state == SourceState::Hid {
            self.tables.observe(SourceState::Hid, &e);
        }
        if e.kind.has_location() {
            self.location = e.location;
        }
        let out = if self.list.is_empty() { Some(e) } else { self.list.post(e) };
        if let Some(survivor) = &out {
            self.tables.observe(SourceState::Session, survivor);
        }
        out
    }

    /// Installs a tap for `app`. At most [`MAX_TAPS_PER_APP`] per app.
    pub fn create(&mut self, app: &str, mask: EventMask, options: TapOptions, placement: TapPlacement, rules: Vec<FilterRule>) -> Result<u32, String> {
        if self.info.iter().filter(|t| t.app == app).count() >= MAX_TAPS_PER_APP {
            return Err(format!("an app may have at most {MAX_TAPS_PER_APP} event taps"));
        }
        let out = self.outbox.clone();
        let owner = app.to_string();
        let cell = Rc::new(Cell::new(0u32));
        let id_cell = cell.clone();
        let id = self.list.create(placement, options, mask, move |e| {
            let d = decide(&rules, e);
            let action = match (options, d) {
                (TapOptions::ListenOnly, _) => "listen",
                (_, Decision::Pass) => "pass",
                (_, Decision::Drop) => "drop",
                (_, Decision::Replace(_)) => "replace",
            };
            let mut q = out.borrow_mut();
            if q.len() >= MAX_NOTICES {
                q.pop_front();
            }
            q.push_back(Notice { app: owner.clone(), tap: id_cell.get(), action, event: *e });
            d
        });
        cell.set(id);
        self.info.push(TapInfo { id, app: app.to_string(), mask, options });
        Ok(id)
    }

    fn owns(&self, app: &str, tap: u32) -> bool {
        self.info.iter().any(|t| t.id == tap && t.app == app)
    }

    pub fn remove(&mut self, app: &str, tap: u32) -> bool {
        if !self.owns(app, tap) {
            return false;
        }
        self.info.retain(|t| t.id != tap);
        self.list.remove(tap)
    }

    pub fn set_enabled(&mut self, app: &str, tap: u32, enabled: bool) -> bool {
        self.owns(app, tap) && self.list.set_enabled(tap, enabled)
    }

    /// Removes every tap of an app that is gone.
    pub fn remove_app(&mut self, app: &str) {
        let ids: Vec<u32> = self.info.iter().filter(|t| t.app == app).map(|t| t.id).collect();
        for id in ids {
            self.list.remove(id);
        }
        self.info.retain(|t| t.app != app);
    }

    fn apps_with_taps(&self) -> Vec<String> {
        let mut apps: Vec<String> = self.info.iter().map(|t| t.app.clone()).collect();
        apps.sort();
        apps.dedup();
        apps
    }

    fn push_posted(&mut self, p: Posted) -> Result<(), String> {
        if self.posted.len() >= MAX_POSTED {
            return Err("too many posted events are waiting".into());
        }
        self.posted.push_back(p);
        Ok(())
    }

    /// Queues the input that an event stands for. `before` is the session modifier state before the event, which a
    /// modifier change is compared with. `pt` is pixels per point. Returns how many input items were queued.
    pub fn deliver(&mut self, e: &Event, before: EventFlags, pt: f32) -> Result<usize, String> {
        let start = self.posted.len();
        let (x, y) = (e.location.0 * pt, e.location.1 * pt);
        match e.kind {
            EventType::Null => {}
            EventType::LeftMouseDown => self.push_posted(Posted::Pointer(UiEvent::Down(x, y)))?,
            EventType::LeftMouseUp => self.push_posted(Posted::Pointer(UiEvent::Up(x, y)))?,
            EventType::LeftMouseDragged => self.push_posted(Posted::Pointer(UiEvent::Move(x, y)))?,
            EventType::MouseMoved => self.push_posted(Posted::Hover(x, y))?,
            EventType::RightMouseDown => self.push_posted(Posted::Context(x, y))?,
            EventType::RightMouseUp | EventType::RightMouseDragged => {}
            EventType::OtherMouseDown | EventType::OtherMouseUp | EventType::OtherMouseDragged => {
                return Err("this shell has no use for the other mouse buttons".into());
            }
            EventType::KeyDown | EventType::KeyUp => {
                let down = e.kind == EventType::KeyDown;
                let (code, needs_shift) = resolve_key(e)?;
                if RESERVED_KEYS.contains(&code) {
                    return Err("the power, volume and wake keys cannot be posted".into());
                }
                let flags = if needs_shift { e.flags.with(EventFlags::SHIFT) } else { e.flags };
                let mods: Vec<u16> = MODIFIER_KEYS.iter().filter(|(bit, _)| flags.contains(*bit)).map(|(_, c)| *c).collect();
                if down {
                    for m in &mods {
                        self.push_posted(Posted::Key(*m, true))?;
                    }
                    self.push_posted(Posted::Key(code, true))?;
                } else {
                    self.push_posted(Posted::Key(code, false))?;
                    for m in mods.iter().rev() {
                        self.push_posted(Posted::Key(*m, false))?;
                    }
                }
            }
            EventType::FlagsChanged => {
                for (bit, code) in MODIFIER_KEYS {
                    let (was, now) = (before.contains(bit), e.flags.contains(bit));
                    if was != now {
                        self.push_posted(Posted::Key(code, now))?;
                    }
                }
            }
            EventType::ScrollWheel => {
                let (v, h) = e.scroll_lines;
                let (dy, dx) = (v * SCROLL_LINE_POINTS * pt, h * SCROLL_LINE_POINTS * pt);
                self.push_posted(Posted::Pointer(UiEvent::Down(x, y)))?;
                for step in 1..=4 {
                    let k = step as f32 / 4.0;
                    self.push_posted(Posted::Pointer(UiEvent::Move(x + dx * k, y + dy * k)))?;
                }
                self.push_posted(Posted::Pointer(UiEvent::Up(x + dx, y + dy)))?;
            }
        }
        Ok(self.posted.len() - start)
    }
}

/// The hardware key code of a key event, and whether typing its character needs Shift. An event without a code but
/// with a character is looked up on the keyboard layout the shell types with.
fn resolve_key(e: &Event) -> Result<(u16, bool), String> {
    if e.key_code != 0 {
        return Ok((e.key_code, false));
    }
    let c = e.text.ok_or("a key event needs a key code or a character")?;
    (1..=255u16)
        .find(|k| evkeys::char_for(*k, false) == Some(c))
        .map(|k| (k, false))
        .or_else(|| (1..=255u16).find(|k| evkeys::char_for(*k, true) == Some(c)).map(|k| (k, true)))
        .ok_or_else(|| format!("no key types {c:?}"))
}

impl Shell {
    /// A pointer event of the touch path, through the taps. `None` when a tap dropped it.
    pub(crate) fn tap_pointer_event(&mut self, ev: UiEvent) -> Option<UiEvent> {
        let (down, x, y) = match ev {
            UiEvent::Down(x, y) => (Some(true), x, y),
            UiEvent::Move(x, y) => (None, x, y),
            UiEvent::Up(x, y) => (Some(false), x, y),
            _ => return Some(ev),
        };
        let was_dragging = self.input_taps.dragging;
        if let Some(d) = down {
            self.input_taps.dragging = d;
        }
        if self.input_taps.bypass || self.is_locked() {
            return Some(ev);
        }
        let pt = self.ui.pt(1.0).max(0.01);
        let kind = match ev {
            UiEvent::Down(..) => EventType::LeftMouseDown,
            UiEvent::Up(..) => EventType::LeftMouseUp,
            _ if was_dragging => EventType::LeftMouseDragged,
            _ => EventType::MouseMoved,
        };
        let now = self.input_taps.now_ns();
        let mut e = Event::mouse(kind, (x / pt, y / pt), now)?;
        e.delta = (e.location.0 - self.input_taps.location.0, e.location.1 - self.input_taps.location.1);
        let out = self.input_taps.process(e, SourceState::Hid)?;
        let (nx, ny) = (out.location.0 * pt, out.location.1 * pt);
        Some(match out.kind {
            EventType::LeftMouseDown => UiEvent::Down(nx, ny),
            EventType::LeftMouseUp => UiEvent::Up(nx, ny),
            EventType::LeftMouseDragged | EventType::MouseMoved => UiEvent::Move(nx, ny),
            _ => ev,
        })
    }

    /// A hover (the pointer moved with no button down), through the taps. The position it returns may differ from the
    /// one given when a tap replaced the event; `None` when a tap dropped it.
    pub fn tap_hover(&mut self, x: f32, y: f32) -> Option<(f32, f32)> {
        if self.input_taps.bypass || self.is_locked() {
            return Some((x, y));
        }
        let pt = self.ui.pt(1.0).max(0.01);
        let now = self.input_taps.now_ns();
        let mut e = Event::mouse(EventType::MouseMoved, (x / pt, y / pt), now)?;
        e.delta = (e.location.0 - self.input_taps.location.0, e.location.1 - self.input_taps.location.1);
        let out = self.input_taps.process(e, SourceState::Hid)?;
        Some((out.location.0 * pt, out.location.1 * pt))
    }

    /// A raw key (Linux key code), through the taps. Returns the key code to act on, which a tap may have replaced;
    /// `None` when a tap dropped the event. Reserved keys and everything typed while locked skip the taps.
    pub fn tap_key(&mut self, code: u16, down: bool) -> Option<u16> {
        // a password being typed (a password sheet, a secure field, a Terminal with Secure Keyboard Entry on) is seen by no tap
        if RESERVED_KEYS.contains(&code) || self.input_taps.bypass || self.is_locked() || self.secure_entry_up() {
            return Some(code);
        }
        let now = self.input_taps.now_ns();
        let out = self.input_taps.process(Event::key(code, down, now), SourceState::Hid)?;
        Some(out.key_code)
    }

    /// The input that posted events are waiting to feed into the shell, oldest first.
    pub fn take_posted_input(&mut self) -> Vec<Posted> {
        self.input_taps.posted.drain(..).collect()
    }

    /// Feeds a pointer event that a post produced into the shell without running the taps again.
    pub fn event_untapped(&mut self, ev: UiEvent, t: f64) -> Vec<super::super::Effect> {
        self.input_taps.bypass = true;
        let fx = self.event(ev, t);
        self.input_taps.bypass = false;
        fx
    }
}

pub struct EventServices;

impl SystemService for EventServices {
    fn name(&self) -> &'static str {
        SERVICE
    }

    fn call(&mut self, shell: &mut Shell, app_id: &str, method: &str, params: &Json, _t: f64) -> Result<Json, String> {
        if !app_has_permission(app_id, PERMISSION) {
            return Err(format!("events: the app needs the {PERMISSION} permission"));
        }
        // declaring the permission is not enough: the person turns it on for the app in Settings > Privacy & Security >
        // App Permissions, and until then every call is refused
        if !super::privacy_permissions::accessibility_allowed(app_id) {
            return Err(format!("events: the {PERMISSION} permission has not been allowed in Settings"));
        }
        match method {
            "post" => post(shell, app_id, params),
            "tap_create" => tap_create(shell, app_id, params),
            "tap_enable" => {
                let tap = u32_of(params, "tap").ok_or("tap_enable needs a tap")?;
                let on = bool_of(params, "enabled").ok_or("tap_enable needs enabled")?;
                Ok(obj(vec![("ok", Json::Bool(shell.input_taps.set_enabled(app_id, tap, on)))]))
            }
            "tap_remove" => {
                let tap = u32_of(params, "tap").ok_or("tap_remove needs a tap")?;
                Ok(obj(vec![("ok", Json::Bool(shell.input_taps.remove(app_id, tap)))]))
            }
            "tap_list" => Ok(Json::Arr(
                shell
                    .input_taps
                    .info
                    .iter()
                    .filter(|t| t.app == app_id)
                    .map(|t| {
                        obj(vec![
                            ("tap", num(t.id)),
                            ("enabled", Json::Bool(shell.input_taps.list.is_enabled(t.id).unwrap_or(false))),
                            ("options", text(t.options.name())),
                            ("mask", t.mask.to_json()),
                        ])
                    })
                    .collect(),
            )),
            "state" => state(shell, params),
            "location" => {
                let (x, y) = shell.input_taps.location;
                Ok(obj(vec![("x", num(x)), ("y", num(y))]))
            }
            _ => Err(format!("events has no method {method}")),
        }
    }

    fn tick(&mut self, shell: &mut Shell, _t: f64) -> Vec<ServiceEvent> {
        for app in shell.input_taps.apps_with_taps() {
            let alive = apphost::slot_of(&app).is_some_and(|slot| matches!(shell.ext.get(slot), Some(Some(_))));
            if !alive || !super::privacy_permissions::accessibility_allowed(&app) {
                shell.input_taps.remove_app(&app);
            }
        }
        let mut q = shell.input_taps.outbox.borrow_mut();
        let n = q.len().min(MAX_NOTICES_PER_TICK);
        let out: Vec<ServiceEvent> = q
            .drain(..n)
            .map(|note| ServiceEvent {
                app: Some(note.app),
                event: TAP_EVENT.into(),
                body: obj(vec![("tap", num(note.tap)), ("action", text(note.action)), ("event", note.event.to_json())]),
            })
            .collect();
        out
    }
}

fn post(shell: &mut Shell, app_id: &str, params: &Json) -> Result<Json, String> {
    if shell.is_locked() {
        return Err("events: nothing can be posted while the screen is locked".into());
    }
    if shell.modal_up() {
        return Err("events: nothing can be posted while a system sheet or alert is up".into());
    }
    let mut e = params.get("event").and_then(Event::from_json).ok_or("post needs a readable event")?;
    e.source_pid = app_pid(shell, app_id).unwrap_or(0);
    if e.timestamp_ns == 0 {
        e.timestamp_ns = shell.input_taps.now_ns();
    }
    let before = shell.input_taps.tables.flags_state(SourceState::Session);
    let pt = shell.ui.pt(1.0).max(0.01);
    let Some(out) = shell.input_taps.process(e, SourceState::Session) else {
        return Ok(obj(vec![("delivered", Json::Bool(false)), ("dropped", Json::Bool(true))]));
    };
    let n = shell.input_taps.deliver(&out, before, pt)?;
    shell.dirty = true;
    Ok(obj(vec![("delivered", Json::Bool(true)), ("dropped", Json::Bool(false)), ("items", num(n as f64))]))
}

fn tap_create(shell: &mut Shell, app_id: &str, params: &Json) -> Result<Json, String> {
    let mask = params.get("mask").and_then(EventMask::from_json).ok_or("tap_create needs a mask of known event kinds")?;
    if mask.is_empty() {
        return Err("tap_create needs at least one event kind".into());
    }
    let options = TapOptions::from_name(&str_of(params, "options").unwrap_or_else(|| "listen".into())).ok_or("options is filter or listen")?;
    let placement = TapPlacement::from_name(&str_of(params, "placement").unwrap_or_else(|| "tail".into())).ok_or("placement is head or tail")?;
    let mut rules = Vec::new();
    if let Some(list) = params.get("rules").and_then(|r| r.as_array()) {
        for r in list {
            rules.push(FilterRule::from_json(r).ok_or("unreadable filter rule")?);
        }
    }
    if options == TapOptions::ListenOnly && !rules.is_empty() {
        return Err("a listen-only tap has no rules".into());
    }
    let tap = shell.input_taps.create(app_id, mask, options, placement, rules)?;
    Ok(obj(vec![("tap", num(tap))]))
}

fn state(shell: &Shell, params: &Json) -> Result<Json, String> {
    let source = match str_of(params, "source").as_deref().unwrap_or("session") {
        "hid" => SourceState::Hid,
        "session" => SourceState::Session,
        "private" => SourceState::Private,
        other => return Err(format!("unknown source {other}")),
    };
    let kind = || str_of(params, "kind").and_then(|k| EventType::from_name(&k)).ok_or_else(|| "a known event kind is needed".to_string());
    let t = &shell.input_taps.tables;
    match str_of(params, "query").ok_or("state needs a query")?.as_str() {
        "flags" => Ok(obj(vec![("flags", num(t.flags_state(source).0))])),
        "key" => Ok(obj(vec![("down", Json::Bool(t.key_state(source, u32_of(params, "key_code").ok_or("key needs key_code")?.min(u16::MAX as u32) as u16)))])),
        "button" => Ok(obj(vec![("down", Json::Bool(t.button_state(source, u32_of(params, "button").ok_or("button needs button")?.min(31) as u8)))])),
        "since" => {
            let now = shell.input_taps.now_ns();
            Ok(obj(vec![("seconds", t.seconds_since_last(source, kind()?, now).map_or(Json::Null, num))]))
        }
        "counter" => Ok(obj(vec![("count", num(t.counter(source, kind()?) as f64))])),
        other => Err(format!("unknown query {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn taps_with(rules: Vec<FilterRule>, options: TapOptions) -> InputTaps {
        let mut t = InputTaps::default();
        t.create("app", EventMask::of(&[EventType::KeyDown, EventType::LeftMouseDown]), options, TapPlacement::TailAppend, rules).unwrap();
        t
    }

    fn drop_key(code: u16) -> FilterRule {
        FilterRule { kind: Some(EventType::KeyDown), key_code: Some(code), flags: None, button: None, action: RuleAction::Drop }
    }

    #[test]
    fn with_no_tap_an_event_only_updates_the_tables() {
        let mut t = InputTaps::default();
        let e = Event::key(30, true, 5);
        assert_eq!(t.process(e, SourceState::Hid).map(|e| e.key_code), Some(30));
        assert!(t.tables.key_state(SourceState::Hid, 30));
        assert!(t.tables.key_state(SourceState::Session, 30));
    }

    #[test]
    fn a_dropped_event_reaches_the_hardware_table_but_not_the_session_table() {
        let mut t = taps_with(vec![drop_key(30)], TapOptions::DefaultTap);
        assert_eq!(t.process(Event::key(30, true, 5), SourceState::Hid), None);
        assert!(t.tables.key_state(SourceState::Hid, 30));
        assert!(!t.tables.key_state(SourceState::Session, 30));
        assert!(t.process(Event::key(31, true, 6), SourceState::Hid).is_some());
    }

    #[test]
    fn a_listen_tap_reports_what_it_saw_and_cannot_drop() {
        let mut t = taps_with(vec![], TapOptions::ListenOnly);
        assert!(t.process(Event::key(30, true, 5), SourceState::Hid).is_some());
        let n = t.outbox.borrow_mut().pop_front().unwrap();
        assert_eq!((n.app.as_str(), n.action, n.tap, n.event.key_code), ("app", "listen", 1, 30));
    }

    #[test]
    fn a_filter_tap_reports_its_decision() {
        let mut t = taps_with(vec![drop_key(30)], TapOptions::DefaultTap);
        t.process(Event::key(30, true, 5), SourceState::Hid);
        t.process(Event::key(31, true, 6), SourceState::Hid);
        let actions: Vec<&str> = t.outbox.borrow().iter().map(|n| n.action).collect();
        assert_eq!(actions, vec!["drop", "pass"]);
    }

    #[test]
    fn taps_belong_to_their_app_and_are_limited() {
        let mut t = InputTaps::default();
        let id = t.create("a", EventMask::all(), TapOptions::ListenOnly, TapPlacement::TailAppend, vec![]).unwrap();
        assert!(!t.set_enabled("b", id, false));
        assert!(!t.remove("b", id));
        assert!(t.set_enabled("a", id, false));
        for _ in 1..MAX_TAPS_PER_APP {
            t.create("a", EventMask::all(), TapOptions::ListenOnly, TapPlacement::TailAppend, vec![]).unwrap();
        }
        assert!(t.create("a", EventMask::all(), TapOptions::ListenOnly, TapPlacement::TailAppend, vec![]).is_err());
        t.remove_app("a");
        assert!(t.list.is_empty());
        assert!(t.apps_with_taps().is_empty());
    }

    #[test]
    fn a_disabled_tap_sees_nothing() {
        let mut t = taps_with(vec![drop_key(30)], TapOptions::DefaultTap);
        assert!(t.set_enabled("app", 1, false));
        assert!(t.process(Event::key(30, true, 5), SourceState::Hid).is_some());
        assert!(t.outbox.borrow().is_empty());
    }

    #[test]
    fn a_key_post_presses_its_modifiers_around_the_key() {
        let mut t = InputTaps::default();
        let mut e = Event::key(30, true, 1);
        e.flags = EventFlags(EventFlags::COMMAND | EventFlags::SHIFT);
        assert_eq!(t.deliver(&e, EventFlags::NONE, 1.0), Ok(3));
        let down: Vec<Posted> = t.posted.drain(..).collect();
        assert_eq!(down, vec![Posted::Key(42, true), Posted::Key(125, true), Posted::Key(30, true)]);
        e.kind = EventType::KeyUp;
        t.deliver(&e, EventFlags::NONE, 1.0).unwrap();
        let up: Vec<Posted> = t.posted.drain(..).collect();
        assert_eq!(up, vec![Posted::Key(30, false), Posted::Key(125, false), Posted::Key(42, false)]);
    }

    #[test]
    fn a_character_without_a_code_finds_its_key() {
        let mut e = Event::key(0, true, 1);
        e.text = Some('a');
        assert_eq!(resolve_key(&e), Ok((30, false)));
        e.text = Some('A');
        assert_eq!(resolve_key(&e), Ok((30, true)));
        e.text = Some('\u{1F600}');
        assert!(resolve_key(&e).is_err());
        e.text = None;
        assert!(resolve_key(&e).is_err());
    }

    #[test]
    fn reserved_keys_cannot_be_posted() {
        let mut t = InputTaps::default();
        assert!(t.deliver(&Event::key(116, true, 1), EventFlags::NONE, 1.0).is_err());
        assert!(t.posted.is_empty());
    }

    #[test]
    fn a_modifier_change_presses_only_what_changed() {
        let mut t = InputTaps::default();
        let e = Event::flags_changed(EventFlags(EventFlags::SHIFT | EventFlags::ALT), 1);
        assert_eq!(t.deliver(&e, EventFlags(EventFlags::SHIFT), 1.0), Ok(1));
        assert_eq!(t.posted.pop_front(), Some(Posted::Key(56, true)));
        let off = Event::flags_changed(EventFlags::NONE, 2);
        t.deliver(&off, EventFlags(EventFlags::SHIFT), 1.0).unwrap();
        assert_eq!(t.posted.pop_front(), Some(Posted::Key(42, false)));
    }

    #[test]
    fn pointer_kinds_scale_points_to_pixels() {
        let mut t = InputTaps::default();
        let e = Event::mouse(EventType::LeftMouseDown, (10.0, 20.0), 1).unwrap();
        t.deliver(&e, EventFlags::NONE, 2.0).unwrap();
        assert_eq!(t.posted.pop_front(), Some(Posted::Pointer(UiEvent::Down(20.0, 40.0))));
        let h = Event::mouse(EventType::MouseMoved, (1.0, 2.0), 1).unwrap();
        t.deliver(&h, EventFlags::NONE, 2.0).unwrap();
        assert_eq!(t.posted.pop_front(), Some(Posted::Hover(2.0, 4.0)));
        let r = Event::mouse(EventType::RightMouseDown, (3.0, 4.0), 1).unwrap();
        t.deliver(&r, EventFlags::NONE, 1.0).unwrap();
        assert_eq!(t.posted.pop_front(), Some(Posted::Context(3.0, 4.0)));
        let other = Event::mouse(EventType::OtherMouseDown, (3.0, 4.0), 1).unwrap();
        assert!(t.deliver(&other, EventFlags::NONE, 1.0).is_err());
    }

    #[test]
    fn a_scroll_wheel_becomes_a_drag_there_and_back() {
        let mut t = InputTaps::default();
        let e = Event::scroll(1.0, 0.0, (100.0, 100.0), 1);
        assert_eq!(t.deliver(&e, EventFlags::NONE, 1.0), Ok(6));
        let items: Vec<Posted> = t.posted.drain(..).collect();
        assert_eq!(items.first(), Some(&Posted::Pointer(UiEvent::Down(100.0, 100.0))));
        assert_eq!(items.last(), Some(&Posted::Pointer(UiEvent::Up(100.0, 140.0))));
    }

    #[test]
    fn the_queue_of_posted_input_is_bounded() {
        let mut t = InputTaps::default();
        let e = Event::mouse(EventType::MouseMoved, (0.0, 0.0), 1).unwrap();
        for _ in 0..MAX_POSTED {
            t.deliver(&e, EventFlags::NONE, 1.0).unwrap();
        }
        assert!(t.deliver(&e, EventFlags::NONE, 1.0).is_err());
    }

    #[test]
    fn no_tap_sees_a_key_typed_under_secure_keyboard_entry() {
        use crate::shell::{AppId, Screen};
        let mut sh = crate::shell::tests::desktop_shell();
        sh.screen = Screen::App(AppId::Terminal);
        sh.anim = None;
        sh.input_taps.create("app", EventMask::of(&[EventType::KeyDown]), TapOptions::DefaultTap, TapPlacement::TailAppend, vec![drop_key(30)]).unwrap();
        assert_eq!(sh.tap_key(30, true), None, "an ordinary key goes through the tap, which drops it");
        let seen = sh.input_taps.outbox.borrow().len();
        sh.terminal_ui.secure = true;
        assert_eq!(sh.tap_key(30, true), Some(30), "under Secure Keyboard Entry the tap is bypassed");
        assert_eq!(sh.input_taps.outbox.borrow().len(), seen, "and was told nothing");
    }

    #[test]
    fn a_location_is_remembered_from_events_that_have_one() {
        let mut t = InputTaps::default();
        t.process(Event::mouse(EventType::MouseMoved, (7.0, 8.0), 1).unwrap(), SourceState::Hid);
        t.process(Event::key(30, true, 2), SourceState::Hid);
        assert_eq!(t.location, (7.0, 8.0));
    }
}
