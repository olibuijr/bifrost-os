//! Low-level input events and event taps. This module is the model: the event record (kind, location, flags,
//! timestamp, key and button fields, scroll deltas, source identity), the event sources with their state tables
//! (which keys and buttons are down, the modifier flags, how long since the last event of a kind), the tap list that
//! filters the stream at session level, and declarative filter rules. The shell owns one tap list and runs every input
//! event it receives through it (`springboard/src/shell/services/event_services.rs`); apps reach it through the SDK client.
//!
//! A tap watches the kinds in its mask, in list order, and answers each event with a decision: pass it on, drop it, or
//! replace it with another event. A listen-only tap sees events but cannot change them. A tap can be switched off and
//! on without losing its place. Posting needs the accessibility permission; the shell checks it, this model does not.

use crate::wire_json::*;
use bifrost_core::json::Json;

/// The kinds of input event.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EventType {
    Null,
    LeftMouseDown,
    LeftMouseUp,
    RightMouseDown,
    RightMouseUp,
    MouseMoved,
    LeftMouseDragged,
    RightMouseDragged,
    KeyDown,
    KeyUp,
    FlagsChanged,
    ScrollWheel,
    OtherMouseDown,
    OtherMouseUp,
    OtherMouseDragged,
}

/// Every event kind, in mask-bit order.
pub const ALL_TYPES: [EventType; 15] = [
    EventType::Null,
    EventType::LeftMouseDown,
    EventType::LeftMouseUp,
    EventType::RightMouseDown,
    EventType::RightMouseUp,
    EventType::MouseMoved,
    EventType::LeftMouseDragged,
    EventType::RightMouseDragged,
    EventType::KeyDown,
    EventType::KeyUp,
    EventType::FlagsChanged,
    EventType::ScrollWheel,
    EventType::OtherMouseDown,
    EventType::OtherMouseUp,
    EventType::OtherMouseDragged,
];

impl EventType {
    /// Whether this kind is a mouse event (moves, drags and button changes; not scrolling).
    pub fn is_mouse(self) -> bool {
        !matches!(self, EventType::Null | EventType::KeyDown | EventType::KeyUp | EventType::FlagsChanged | EventType::ScrollWheel)
    }

    pub fn is_key(self) -> bool {
        matches!(self, EventType::KeyDown | EventType::KeyUp)
    }

    /// Whether the event carries a pointer location.
    pub fn has_location(self) -> bool {
        self.is_mouse() || self == EventType::ScrollWheel
    }

    /// The button a mouse kind concerns: 0 left, 1 right, 2 other.
    pub fn button(self) -> Option<u8> {
        match self {
            EventType::LeftMouseDown | EventType::LeftMouseUp | EventType::LeftMouseDragged => Some(0),
            EventType::RightMouseDown | EventType::RightMouseUp | EventType::RightMouseDragged => Some(1),
            EventType::OtherMouseDown | EventType::OtherMouseUp | EventType::OtherMouseDragged => Some(2),
            _ => None,
        }
    }

    /// Whether this kind presses a button.
    pub fn is_button_down(self) -> bool {
        matches!(self, EventType::LeftMouseDown | EventType::RightMouseDown | EventType::OtherMouseDown)
    }

    pub fn is_button_up(self) -> bool {
        matches!(self, EventType::LeftMouseUp | EventType::RightMouseUp | EventType::OtherMouseUp)
    }

    /// The bit this kind takes in an [`EventMask`]: one bit per kind, in declaration order.
    fn bit(self) -> u64 {
        1u64 << (self as u32)
    }

    pub fn name(self) -> &'static str {
        match self {
            EventType::Null => "null",
            EventType::LeftMouseDown => "left_mouse_down",
            EventType::LeftMouseUp => "left_mouse_up",
            EventType::RightMouseDown => "right_mouse_down",
            EventType::RightMouseUp => "right_mouse_up",
            EventType::MouseMoved => "mouse_moved",
            EventType::LeftMouseDragged => "left_mouse_dragged",
            EventType::RightMouseDragged => "right_mouse_dragged",
            EventType::KeyDown => "key_down",
            EventType::KeyUp => "key_up",
            EventType::FlagsChanged => "flags_changed",
            EventType::ScrollWheel => "scroll_wheel",
            EventType::OtherMouseDown => "other_mouse_down",
            EventType::OtherMouseUp => "other_mouse_up",
            EventType::OtherMouseDragged => "other_mouse_dragged",
        }
    }

    pub fn from_name(name: &str) -> Option<EventType> {
        ALL_TYPES.iter().copied().find(|t| t.name() == name)
    }
}

/// A set of event kinds a tap watches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EventMask(u64);

impl EventMask {
    /// The empty mask: no kinds.
    pub const NONE: EventMask = EventMask(0);

    /// The mask of the listed kinds.
    pub fn of(types: &[EventType]) -> EventMask {
        EventMask(types.iter().fold(0u64, |m, t| m | t.bit()))
    }

    /// The mask of every kind.
    pub fn all() -> EventMask {
        EventMask::of(&ALL_TYPES)
    }

    /// Whether the mask watches this kind.
    pub fn contains(self, kind: EventType) -> bool {
        self.0 & kind.bit() != 0
    }

    /// The kinds of both masks.
    pub fn union(self, other: EventMask) -> EventMask {
        EventMask(self.0 | other.0)
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// The kinds in the mask.
    pub fn kinds(self) -> Vec<EventType> {
        ALL_TYPES.iter().copied().filter(|t| self.contains(*t)).collect()
    }

    pub fn to_json(self) -> Json {
        Json::Arr(self.kinds().iter().map(|k| text(k.name())).collect())
    }

    /// Reads a list of kind names; an unknown name makes the whole mask `None`.
    pub fn from_json(j: &Json) -> Option<EventMask> {
        let mut kinds = Vec::new();
        for item in j.as_array()? {
            kinds.push(EventType::from_name(item.as_str()?)?);
        }
        Some(EventMask::of(&kinds))
    }
}

/// The modifier flags of an event.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EventFlags(pub u32);

impl EventFlags {
    pub const NONE: EventFlags = EventFlags(0);
    pub const CAPS_LOCK: u32 = 1 << 16;
    pub const SHIFT: u32 = 1 << 17;
    pub const CONTROL: u32 = 1 << 18;
    pub const ALT: u32 = 1 << 19;
    pub const COMMAND: u32 = 1 << 20;
    pub const NUMERIC_PAD: u32 = 1 << 21;
    pub const HELP: u32 = 1 << 22;
    pub const FUNCTION: u32 = 1 << 23;

    pub fn contains(self, bit: u32) -> bool {
        self.0 & bit == bit
    }

    pub fn with(self, bit: u32) -> EventFlags {
        EventFlags(self.0 | bit)
    }

    pub fn without(self, bit: u32) -> EventFlags {
        EventFlags(self.0 & !bit)
    }

    /// The modifier bit a key code toggles, for the Linux key codes of the modifier keys (shift, control, alt, super,
    /// caps lock).
    pub fn bit_of_key(code: u16) -> Option<u32> {
        match code {
            42 | 54 => Some(EventFlags::SHIFT),
            29 | 97 => Some(EventFlags::CONTROL),
            56 | 100 => Some(EventFlags::ALT),
            125 | 126 => Some(EventFlags::COMMAND),
            58 => Some(EventFlags::CAPS_LOCK),
            _ => None,
        }
    }
}

/// Where an event came from: the hardware stream, the login session's stream, or a private source that does not touch
/// the shared state tables.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SourceState {
    Private,
    Session,
    Hid,
}

impl SourceState {
    pub fn code(self) -> u32 {
        match self {
            SourceState::Private => 0,
            SourceState::Session => 1,
            SourceState::Hid => 2,
        }
    }

    pub fn from_code(c: u32) -> SourceState {
        match c {
            1 => SourceState::Session,
            2 => SourceState::Hid,
            _ => SourceState::Private,
        }
    }
}

/// One input event. `location` is in desktop points and meaningful for the kinds that have one; `key_code` is a key
/// event's hardware key code (Linux numbering); `text` the character a key event types, when it types one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Event {
    pub kind: EventType,
    pub timestamp_ns: u64,
    pub location: (f32, f32),
    pub flags: EventFlags,
    pub key_code: u16,
    pub text: Option<char>,
    pub autorepeat: bool,
    pub keyboard_type: u32,
    /// Mouse button number: 0 left, 1 right, 2 and up other.
    pub button: u8,
    pub click_state: u32,
    pub pressure: f32,
    /// Pointer movement since the previous mouse event.
    pub delta: (f32, f32),
    /// Scroll wheel movement in lines: (vertical, horizontal).
    pub scroll_lines: (f32, f32),
    /// Scroll movement in points: (vertical, horizontal). Zero for a notched wheel.
    pub scroll_points: (f32, f32),
    pub source: SourceState,
    /// A value the source attaches, which taps can read back (to tell their own events from others).
    pub user_data: i64,
    pub source_pid: u32,
    /// The process the event is addressed to; 0 for the focused one.
    pub target_pid: u32,
}

impl Event {
    fn base(kind: EventType, timestamp_ns: u64) -> Event {
        Event {
            kind,
            timestamp_ns,
            location: (0.0, 0.0),
            flags: EventFlags::NONE,
            key_code: 0,
            text: None,
            autorepeat: false,
            keyboard_type: 0,
            button: 0,
            click_state: 0,
            pressure: 0.0,
            delta: (0.0, 0.0),
            scroll_lines: (0.0, 0.0),
            scroll_points: (0.0, 0.0),
            source: SourceState::Private,
            user_data: 0,
            source_pid: 0,
            target_pid: 0,
        }
    }

    /// An event that does nothing, used to wake a tap.
    pub fn null(timestamp_ns: u64) -> Event {
        Event::base(EventType::Null, timestamp_ns)
    }

    /// A synthetic mouse event. `None` when `kind` is not a mouse kind. The button follows the kind.
    pub fn mouse(kind: EventType, location: (f32, f32), timestamp_ns: u64) -> Option<Event> {
        let button = kind.button().or(if kind == EventType::MouseMoved { Some(0) } else { None })?;
        let mut e = Event::base(kind, timestamp_ns);
        e.location = location;
        e.button = button;
        if kind.is_button_down() {
            e.click_state = 1;
            e.pressure = 1.0;
        }
        Some(e)
    }

    /// A synthetic key event for a hardware key code.
    pub fn key(key_code: u16, down: bool, timestamp_ns: u64) -> Event {
        let mut e = Event::base(if down { EventType::KeyDown } else { EventType::KeyUp }, timestamp_ns);
        e.key_code = key_code;
        e
    }

    /// A key-down event for a key combination, as the key and the modifier flags of the combination.
    pub fn key_down(combo: crate::keys::KeyCombo, timestamp_ns: u64) -> Event {
        let mut e = Event::key(0, true, timestamp_ns);
        e.text = match combo.key {
            crate::keys::Key::Char(c) => Some(c),
            crate::keys::Key::Space => Some(' '),
            _ => None,
        };
        let mut flags = EventFlags::NONE;
        for (on, bit) in [(combo.cmd, EventFlags::COMMAND), (combo.ctrl, EventFlags::CONTROL), (combo.alt, EventFlags::ALT), (combo.shift, EventFlags::SHIFT)] {
            if on {
                flags = flags.with(bit);
            }
        }
        e.flags = flags;
        e
    }

    /// A scroll wheel event moving `vertical` and `horizontal` lines (positive scrolls up and left).
    pub fn scroll(vertical: f32, horizontal: f32, location: (f32, f32), timestamp_ns: u64) -> Event {
        let mut e = Event::base(EventType::ScrollWheel, timestamp_ns);
        e.scroll_lines = (vertical, horizontal);
        e.location = location;
        e
    }

    /// A modifier change: the new flag state.
    pub fn flags_changed(flags: EventFlags, timestamp_ns: u64) -> Event {
        let mut e = Event::base(EventType::FlagsChanged, timestamp_ns);
        e.flags = flags;
        e
    }

    pub fn with_flags(mut self, flags: EventFlags) -> Event {
        self.flags = flags;
        self
    }

    pub fn with_user_data(mut self, user_data: i64) -> Event {
        self.user_data = user_data;
        self
    }

    pub fn at(mut self, location: (f32, f32)) -> Event {
        self.location = location;
        self
    }

    /// Reads one numeric field as an integer.
    pub fn int_field(&self, field: EventField) -> i64 {
        match field {
            EventField::MouseButton => self.button as i64,
            EventField::MouseClickState => self.click_state as i64,
            EventField::KeyCode => self.key_code as i64,
            EventField::KeyAutorepeat => self.autorepeat as i64,
            EventField::KeyboardType => self.keyboard_type as i64,
            EventField::ScrollLines1 => self.scroll_lines.0 as i64,
            EventField::ScrollLines2 => self.scroll_lines.1 as i64,
            EventField::ScrollPoints1 => self.scroll_points.0 as i64,
            EventField::ScrollPoints2 => self.scroll_points.1 as i64,
            EventField::SourceUserData => self.user_data,
            EventField::SourceState => self.source.code() as i64,
            EventField::SourcePid => self.source_pid as i64,
            EventField::TargetPid => self.target_pid as i64,
            EventField::MousePressure => self.pressure as i64,
            EventField::MouseDeltaX => self.delta.0 as i64,
            EventField::MouseDeltaY => self.delta.1 as i64,
        }
    }

    /// Writes one numeric field from an integer.
    pub fn set_int_field(&mut self, field: EventField, v: i64) {
        match field {
            EventField::MouseButton => self.button = v.clamp(0, 31) as u8,
            EventField::MouseClickState => self.click_state = v.max(0) as u32,
            EventField::KeyCode => self.key_code = v.clamp(0, u16::MAX as i64) as u16,
            EventField::KeyAutorepeat => self.autorepeat = v != 0,
            EventField::KeyboardType => self.keyboard_type = v.max(0) as u32,
            EventField::ScrollLines1 => self.scroll_lines.0 = v as f32,
            EventField::ScrollLines2 => self.scroll_lines.1 = v as f32,
            EventField::ScrollPoints1 => self.scroll_points.0 = v as f32,
            EventField::ScrollPoints2 => self.scroll_points.1 = v as f32,
            EventField::SourceUserData => self.user_data = v,
            EventField::SourceState => self.source = SourceState::from_code(v.max(0) as u32),
            EventField::SourcePid => self.source_pid = v.max(0) as u32,
            EventField::TargetPid => self.target_pid = v.max(0) as u32,
            EventField::MousePressure => self.pressure = v as f32,
            EventField::MouseDeltaX => self.delta.0 = v as f32,
            EventField::MouseDeltaY => self.delta.1 = v as f32,
        }
    }

    /// Reads one numeric field as a real number.
    pub fn double_field(&self, field: EventField) -> f64 {
        match field {
            EventField::MousePressure => self.pressure as f64,
            EventField::MouseDeltaX => self.delta.0 as f64,
            EventField::MouseDeltaY => self.delta.1 as f64,
            EventField::ScrollLines1 => self.scroll_lines.0 as f64,
            EventField::ScrollLines2 => self.scroll_lines.1 as f64,
            EventField::ScrollPoints1 => self.scroll_points.0 as f64,
            EventField::ScrollPoints2 => self.scroll_points.1 as f64,
            other => self.int_field(other) as f64,
        }
    }

    /// Writes one numeric field from a real number.
    pub fn set_double_field(&mut self, field: EventField, v: f64) {
        match field {
            EventField::MousePressure => self.pressure = v as f32,
            EventField::MouseDeltaX => self.delta.0 = v as f32,
            EventField::MouseDeltaY => self.delta.1 = v as f32,
            EventField::ScrollLines1 => self.scroll_lines.0 = v as f32,
            EventField::ScrollLines2 => self.scroll_lines.1 = v as f32,
            EventField::ScrollPoints1 => self.scroll_points.0 = v as f32,
            EventField::ScrollPoints2 => self.scroll_points.1 = v as f32,
            other => self.set_int_field(other, v as i64),
        }
    }

    pub fn to_json(&self) -> Json {
        obj(vec![
            ("kind", text(self.kind.name())),
            ("timestamp_ns", num(self.timestamp_ns as f64)),
            ("x", num(self.location.0)),
            ("y", num(self.location.1)),
            ("flags", num(self.flags.0)),
            ("key_code", num(self.key_code)),
            ("text", self.text.map_or(Json::Null, |c| text(&c.to_string()))),
            ("autorepeat", Json::Bool(self.autorepeat)),
            ("keyboard_type", num(self.keyboard_type)),
            ("button", num(self.button)),
            ("click_state", num(self.click_state)),
            ("pressure", num(self.pressure)),
            ("dx", num(self.delta.0)),
            ("dy", num(self.delta.1)),
            ("scroll_lines_v", num(self.scroll_lines.0)),
            ("scroll_lines_h", num(self.scroll_lines.1)),
            ("scroll_points_v", num(self.scroll_points.0)),
            ("scroll_points_h", num(self.scroll_points.1)),
            ("source", num(self.source.code())),
            ("user_data", num(self.user_data as f64)),
            ("source_pid", num(self.source_pid)),
            ("target_pid", num(self.target_pid)),
        ])
    }

    /// Reads an event; only `kind` is required, the other fields default to zero.
    pub fn from_json(j: &Json) -> Option<Event> {
        let kind = EventType::from_name(&str_of(j, "kind")?)?;
        let mut e = Event::base(kind, f64_of(j, "timestamp_ns").unwrap_or(0.0).max(0.0) as u64);
        e.location = (f32_of(j, "x").unwrap_or(0.0), f32_of(j, "y").unwrap_or(0.0));
        e.flags = EventFlags(u32_of(j, "flags").unwrap_or(0));
        e.key_code = u32_of(j, "key_code").unwrap_or(0).min(u16::MAX as u32) as u16;
        e.text = str_of(j, "text").and_then(|s| s.chars().next());
        e.autorepeat = bool_of(j, "autorepeat").unwrap_or(false);
        e.keyboard_type = u32_of(j, "keyboard_type").unwrap_or(0);
        e.button = u32_of(j, "button").unwrap_or(0).min(31) as u8;
        e.click_state = u32_of(j, "click_state").unwrap_or(0);
        e.pressure = f32_of(j, "pressure").unwrap_or(0.0);
        e.delta = (f32_of(j, "dx").unwrap_or(0.0), f32_of(j, "dy").unwrap_or(0.0));
        e.scroll_lines = (f32_of(j, "scroll_lines_v").unwrap_or(0.0), f32_of(j, "scroll_lines_h").unwrap_or(0.0));
        e.scroll_points = (f32_of(j, "scroll_points_v").unwrap_or(0.0), f32_of(j, "scroll_points_h").unwrap_or(0.0));
        e.source = SourceState::from_code(u32_of(j, "source").unwrap_or(0));
        e.user_data = i64_of(j, "user_data").unwrap_or(0);
        e.source_pid = u32_of(j, "source_pid").unwrap_or(0);
        e.target_pid = u32_of(j, "target_pid").unwrap_or(0);
        Some(e)
    }
}

/// The numeric fields of an event that can be read and written by field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventField {
    MouseButton,
    MouseClickState,
    MousePressure,
    MouseDeltaX,
    MouseDeltaY,
    KeyCode,
    KeyAutorepeat,
    KeyboardType,
    ScrollLines1,
    ScrollLines2,
    ScrollPoints1,
    ScrollPoints2,
    SourceUserData,
    SourceState,
    SourcePid,
    TargetPid,
}

// ---- event sources ----

/// The state tables of the event sources. The hardware table follows the real devices; the session table follows what
/// reached the login session, including posted events. A private source has no table. Feeding an event updates the
/// table of its source state, so a client can ask which keys and buttons are down and how long ago the last event
/// of a kind was.
#[derive(Clone, Debug, Default)]
pub struct SourceTables {
    hid: Table,
    session: Table,
}

#[derive(Clone, Debug, Default)]
struct Table {
    flags: EventFlags,
    keys_down: Vec<u16>,
    buttons: u32,
    last_ns: [Option<u64>; 15],
    counter: [u64; 15],
}

impl SourceTables {
    pub fn new() -> Self {
        SourceTables::default()
    }

    fn table(&self, state: SourceState) -> Option<&Table> {
        match state {
            SourceState::Hid => Some(&self.hid),
            SourceState::Session => Some(&self.session),
            SourceState::Private => None,
        }
    }

    fn table_mut(&mut self, state: SourceState) -> Option<&mut Table> {
        match state {
            SourceState::Hid => Some(&mut self.hid),
            SourceState::Session => Some(&mut self.session),
            SourceState::Private => None,
        }
    }

    /// Records an event in the table of `state` (nothing for a private source).
    pub fn observe(&mut self, state: SourceState, e: &Event) {
        let Some(t) = self.table_mut(state) else { return };
        let i = e.kind as usize;
        t.last_ns[i] = Some(e.timestamp_ns);
        t.counter[i] += 1;
        match e.kind {
            EventType::KeyDown => {
                if !t.keys_down.contains(&e.key_code) {
                    t.keys_down.push(e.key_code);
                }
                if let Some(bit) = EventFlags::bit_of_key(e.key_code) {
                    t.flags = if bit == EventFlags::CAPS_LOCK && t.flags.contains(bit) { t.flags.without(bit) } else { t.flags.with(bit) };
                }
            }
            EventType::KeyUp => {
                t.keys_down.retain(|k| *k != e.key_code);
                if let Some(bit) = EventFlags::bit_of_key(e.key_code).filter(|b| *b != EventFlags::CAPS_LOCK) {
                    t.flags = t.flags.without(bit);
                }
            }
            EventType::FlagsChanged => t.flags = e.flags,
            k if k.is_button_down() => t.buttons |= 1 << e.button.min(31),
            k if k.is_button_up() => t.buttons &= !(1 << e.button.min(31)),
            _ => {}
        }
    }

    /// The modifier flags now held in the table of `state`.
    pub fn flags_state(&self, state: SourceState) -> EventFlags {
        self.table(state).map_or(EventFlags::NONE, |t| t.flags)
    }

    pub fn key_state(&self, state: SourceState, key_code: u16) -> bool {
        self.table(state).is_some_and(|t| t.keys_down.contains(&key_code))
    }

    pub fn button_state(&self, state: SourceState, button: u8) -> bool {
        self.table(state).is_some_and(|t| t.buttons & (1 << button.min(31)) != 0)
    }

    /// Seconds from the last event of `kind` to `now_ns`; `None` when there has been none (or for a private source).
    pub fn seconds_since_last(&self, state: SourceState, kind: EventType, now_ns: u64) -> Option<f64> {
        let last = self.table(state)?.last_ns[kind as usize]?;
        Some(now_ns.saturating_sub(last) as f64 / 1e9)
    }

    /// How many events of `kind` the table has seen.
    pub fn counter(&self, state: SourceState, kind: EventType) -> u64 {
        self.table(state).map_or(0, |t| t.counter[kind as usize])
    }
}

/// A source that stamps the events built from it: the state it draws from, a user data value and the process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EventSource {
    pub state: SourceState,
    pub user_data: i64,
    pub pid: u32,
}

impl EventSource {
    pub fn new(state: SourceState, pid: u32) -> Self {
        EventSource { state, user_data: 0, pid }
    }

    /// Stamps an event with this source's identity.
    pub fn stamp(&self, mut e: Event) -> Event {
        e.source = self.state;
        e.user_data = self.user_data;
        e.source_pid = self.pid;
        e
    }
}

// ---- taps ----

/// Whether a tap sees the events in its mask as a default filter or only listens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TapOptions {
    /// The tap may drop or replace events.
    DefaultTap,
    /// The tap sees events, and its decisions are ignored.
    ListenOnly,
}

impl TapOptions {
    pub fn name(self) -> &'static str {
        match self {
            TapOptions::DefaultTap => "filter",
            TapOptions::ListenOnly => "listen",
        }
    }

    pub fn from_name(name: &str) -> Option<TapOptions> {
        match name {
            "filter" => Some(TapOptions::DefaultTap),
            "listen" => Some(TapOptions::ListenOnly),
            _ => None,
        }
    }
}

/// Where a new tap goes in the list: before every existing tap, or after them all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TapPlacement {
    HeadInsert,
    TailAppend,
}

impl TapPlacement {
    pub fn from_name(name: &str) -> Option<TapPlacement> {
        match name {
            "head" => Some(TapPlacement::HeadInsert),
            "tail" => Some(TapPlacement::TailAppend),
            _ => None,
        }
    }
}

/// A tap's answer to one event.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Decision {
    /// Let the event go on unchanged.
    Pass,
    /// Remove the event from the stream.
    Drop,
    /// Send this event on in place of the one received.
    Replace(Event),
}

struct EventTap {
    id: u32,
    mask: EventMask,
    options: TapOptions,
    enabled: bool,
    filter: Box<dyn FnMut(&Event) -> Decision>,
}

/// The installed taps, in the order events visit them.
pub struct EventTapList {
    taps: Vec<EventTap>,
    next_id: u32,
}

impl Default for EventTapList {
    fn default() -> Self {
        Self::new()
    }
}

impl EventTapList {
    /// An empty list. Tap ids start at 1.
    pub fn new() -> Self {
        EventTapList { taps: Vec::new(), next_id: 1 }
    }

    /// Whether no tap is installed (the common case: the input path skips the list).
    pub fn is_empty(&self) -> bool {
        self.taps.is_empty()
    }

    /// Installs a tap that watches `mask` and asks `filter` about each event it sees. Returns the tap's id.
    pub fn create(
        &mut self,
        placement: TapPlacement,
        options: TapOptions,
        mask: EventMask,
        filter: impl FnMut(&Event) -> Decision + 'static,
    ) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        let tap = EventTap { id, mask, options, enabled: true, filter: Box::new(filter) };
        match placement {
            TapPlacement::HeadInsert => self.taps.insert(0, tap),
            TapPlacement::TailAppend => self.taps.push(tap),
        }
        id
    }

    /// The ids of the installed taps, in visiting order.
    pub fn ids(&self) -> Vec<u32> {
        self.taps.iter().map(|t| t.id).collect()
    }

    /// Enables or disables a tap. `false` when no tap has the id.
    pub fn set_enabled(&mut self, id: u32, enabled: bool) -> bool {
        match self.taps.iter_mut().find(|t| t.id == id) {
            Some(t) => {
                t.enabled = enabled;
                true
            }
            None => false,
        }
    }

    /// Whether a tap is enabled, or `None` when no tap has the id.
    pub fn is_enabled(&self, id: u32) -> Option<bool> {
        self.taps.iter().find(|t| t.id == id).map(|t| t.enabled)
    }

    /// The options a tap was installed with.
    pub fn options_of(&self, id: u32) -> Option<TapOptions> {
        self.taps.iter().find(|t| t.id == id).map(|t| t.options)
    }

    /// Removes a tap. `false` when no tap has the id.
    pub fn remove(&mut self, id: u32) -> bool {
        let before = self.taps.len();
        self.taps.retain(|t| t.id != id);
        self.taps.len() != before
    }

    /// Runs an event through the enabled taps whose masks watch its kind, in order. A default tap can drop the event,
    /// which ends the run with `None`, or replace it, and later taps see the replacement. Returns the event that
    /// reaches the stream.
    pub fn post(&mut self, event: Event) -> Option<Event> {
        let mut current = event;
        for tap in self.taps.iter_mut() {
            if !tap.enabled || !tap.mask.contains(current.kind) {
                continue;
            }
            let decision = (tap.filter)(&current);
            if tap.options == TapOptions::ListenOnly {
                continue;
            }
            match decision {
                Decision::Pass => {}
                Decision::Drop => return None,
                Decision::Replace(e) => current = e,
            }
        }
        Some(current)
    }
}

// ---- declarative filters ----

/// What a rule does to the events it matches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuleAction {
    Drop,
    Pass,
    /// Send a key event with this key code instead (the flags stay).
    ReplaceKey(u16),
}

/// One filter rule: the events it matches and what it does to them. A field left `None` matches anything.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FilterRule {
    pub kind: Option<EventType>,
    pub key_code: Option<u16>,
    /// The flags that must be set (extra flags may be set too).
    pub flags: Option<EventFlags>,
    pub button: Option<u8>,
    pub action: RuleAction,
}

impl FilterRule {
    pub fn matches(&self, e: &Event) -> bool {
        self.kind.map_or(true, |k| k == e.kind)
            && self.key_code.map_or(true, |c| c == e.key_code)
            && self.flags.map_or(true, |f| e.flags.contains(f.0))
            && self.button.map_or(true, |b| b == e.button)
    }

    pub fn to_json(&self) -> Json {
        let action = match self.action {
            RuleAction::Drop => "drop".to_string(),
            RuleAction::Pass => "pass".to_string(),
            RuleAction::ReplaceKey(c) => format!("replace_key:{c}"),
        };
        obj(vec![
            ("kind", self.kind.map_or(Json::Null, |k| text(k.name()))),
            ("key_code", self.key_code.map_or(Json::Null, num)),
            ("flags", self.flags.map_or(Json::Null, |f| num(f.0))),
            ("button", self.button.map_or(Json::Null, num)),
            ("action", text(&action)),
        ])
    }

    pub fn from_json(j: &Json) -> Option<FilterRule> {
        let action = match str_of(j, "action")?.as_str() {
            "drop" => RuleAction::Drop,
            "pass" => RuleAction::Pass,
            other => RuleAction::ReplaceKey(other.strip_prefix("replace_key:")?.parse().ok()?),
        };
        let kind = match j.get("kind") {
            None | Some(Json::Null) => None,
            Some(k) => Some(EventType::from_name(k.as_str()?)?),
        };
        Some(FilterRule {
            kind,
            key_code: u32_of(j, "key_code").map(|c| c.min(u16::MAX as u32) as u16),
            flags: u32_of(j, "flags").map(EventFlags),
            button: u32_of(j, "button").map(|b| b.min(31) as u8),
            action,
        })
    }
}

/// The decision of a rule list for one event: the action of the first rule that matches, or pass.
pub fn decide(rules: &[FilterRule], e: &Event) -> Decision {
    match rules.iter().find(|r| r.matches(e)).map(|r| r.action) {
        None | Some(RuleAction::Pass) => Decision::Pass,
        Some(RuleAction::Drop) => Decision::Drop,
        Some(RuleAction::ReplaceKey(code)) => {
            let mut replaced = *e;
            replaced.key_code = code;
            replaced.text = None;
            Decision::Replace(replaced)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::KeyCombo;
    use std::cell::Cell;
    use std::rc::Rc;

    fn key(s: &str) -> KeyCombo {
        KeyCombo::parse(s).unwrap()
    }

    #[test]
    fn mask_watches_only_the_listed_kinds() {
        let m = EventMask::of(&[EventType::LeftMouseDown, EventType::KeyDown]);
        assert!(m.contains(EventType::LeftMouseDown));
        assert!(m.contains(EventType::KeyDown));
        assert!(!m.contains(EventType::MouseMoved));
        let u = m.union(EventMask::of(&[EventType::MouseMoved]));
        assert!(u.contains(EventType::MouseMoved));
        assert!(!EventMask::NONE.contains(EventType::KeyDown));
        assert!(EventMask::all().contains(EventType::Null));
        assert!(EventMask::NONE.is_empty());
        assert_eq!(EventMask::all().kinds().len(), ALL_TYPES.len());
    }

    #[test]
    fn a_mask_survives_the_wire_and_refuses_unknown_names() {
        let m = EventMask::of(&[EventType::ScrollWheel, EventType::KeyUp]);
        assert_eq!(EventMask::from_json(&m.to_json()), Some(m));
        assert_eq!(EventMask::from_json(&Json::Arr(vec![text("bogus")])), None);
    }

    #[test]
    fn mouse_constructor_refuses_a_key_kind_and_takes_the_button_from_the_kind() {
        assert!(Event::mouse(EventType::KeyDown, (1.0, 2.0), 5).is_none());
        assert!(Event::mouse(EventType::ScrollWheel, (1.0, 2.0), 5).is_none());
        let e = Event::mouse(EventType::MouseMoved, (1.0, 2.0), 5).unwrap();
        assert_eq!(e.location, (1.0, 2.0));
        let r = Event::mouse(EventType::RightMouseDown, (0.0, 0.0), 6).unwrap();
        assert_eq!(r.button, 1);
        assert_eq!(r.click_state, 1);
    }

    #[test]
    fn creation_helpers_fill_the_fields() {
        let k = Event::key(30, true, 9);
        assert_eq!((k.kind, k.key_code), (EventType::KeyDown, 30));
        assert_eq!(Event::key(30, false, 9).kind, EventType::KeyUp);
        let c = Event::key_down(key("cmd+shift+s"), 3);
        assert_eq!(c.text, Some('s'));
        assert!(c.flags.contains(EventFlags::COMMAND | EventFlags::SHIFT));
        assert!(!c.flags.contains(EventFlags::ALT));
        let s = Event::scroll(-3.0, 1.0, (4.0, 5.0), 8);
        assert_eq!((s.scroll_lines, s.location), ((-3.0, 1.0), (4.0, 5.0)));
        assert_eq!(Event::flags_changed(EventFlags(EventFlags::SHIFT), 1).flags.0, EventFlags::SHIFT);
        assert_eq!(Event::null(2).kind, EventType::Null);
    }

    #[test]
    fn fields_read_and_write_by_name() {
        let mut e = Event::mouse(EventType::LeftMouseDown, (0.0, 0.0), 1).unwrap();
        e.set_int_field(EventField::MouseClickState, 2);
        e.set_double_field(EventField::MousePressure, 0.5);
        e.set_int_field(EventField::SourceUserData, -9);
        e.set_int_field(EventField::KeyCode, 70000);
        assert_eq!(e.int_field(EventField::MouseClickState), 2);
        assert_eq!(e.double_field(EventField::MousePressure), 0.5);
        assert_eq!(e.int_field(EventField::SourceUserData), -9);
        assert_eq!(e.key_code, u16::MAX);
        e.set_double_field(EventField::ScrollLines1, 2.5);
        assert_eq!(e.double_field(EventField::ScrollLines1), 2.5);
        e.set_int_field(EventField::SourceState, 2);
        assert_eq!(e.source, SourceState::Hid);
    }

    #[test]
    fn an_event_survives_the_wire() {
        let mut e = Event::key_down(key("ctrl+a"), 123_456_789).with_user_data(77);
        e.key_code = 30;
        e.source = SourceState::Session;
        e.source_pid = 41;
        assert_eq!(Event::from_json(&e.to_json()), Some(e));
        let m = Event::mouse(EventType::MouseMoved, (3.5, 4.5), 9).unwrap();
        assert_eq!(Event::from_json(&m.to_json()), Some(m));
        assert_eq!(Event::from_json(&obj(vec![("kind", text("bogus"))])), None);
        assert_eq!(Event::from_json(&obj(vec![("kind", text("null"))])).map(|e| e.timestamp_ns), Some(0));
    }

    #[test]
    fn source_tables_follow_keys_buttons_and_modifiers() {
        let mut t = SourceTables::new();
        t.observe(SourceState::Hid, &Event::key(42, true, 1_000_000_000));
        t.observe(SourceState::Hid, &Event::key(30, true, 2_000_000_000));
        assert!(t.key_state(SourceState::Hid, 30));
        assert!(!t.key_state(SourceState::Session, 30));
        assert!(t.flags_state(SourceState::Hid).contains(EventFlags::SHIFT));
        t.observe(SourceState::Hid, &Event::key(30, false, 3_000_000_000));
        t.observe(SourceState::Hid, &Event::key(42, false, 3_000_000_000));
        assert!(!t.key_state(SourceState::Hid, 30));
        assert_eq!(t.flags_state(SourceState::Hid), EventFlags::NONE);
        t.observe(SourceState::Session, &Event::mouse(EventType::RightMouseDown, (0.0, 0.0), 5).unwrap());
        assert!(t.button_state(SourceState::Session, 1));
        t.observe(SourceState::Session, &Event::mouse(EventType::RightMouseUp, (0.0, 0.0), 6).unwrap());
        assert!(!t.button_state(SourceState::Session, 1));
        assert_eq!(t.counter(SourceState::Hid, EventType::KeyDown), 2);
        assert_eq!(t.seconds_since_last(SourceState::Hid, EventType::KeyDown, 5_000_000_000), Some(3.0));
        assert_eq!(t.seconds_since_last(SourceState::Hid, EventType::ScrollWheel, 5), None);
        t.observe(SourceState::Private, &Event::key(30, true, 1));
        assert_eq!(t.counter(SourceState::Private, EventType::KeyDown), 0);
    }

    #[test]
    fn caps_lock_toggles_on_each_press() {
        let mut t = SourceTables::new();
        t.observe(SourceState::Session, &Event::key(58, true, 1));
        t.observe(SourceState::Session, &Event::key(58, false, 2));
        assert!(t.flags_state(SourceState::Session).contains(EventFlags::CAPS_LOCK));
        t.observe(SourceState::Session, &Event::key(58, true, 3));
        t.observe(SourceState::Session, &Event::key(58, false, 4));
        assert!(!t.flags_state(SourceState::Session).contains(EventFlags::CAPS_LOCK));
    }

    #[test]
    fn a_source_stamps_its_identity() {
        let mut s = EventSource::new(SourceState::Session, 12);
        s.user_data = 5;
        let e = s.stamp(Event::null(1));
        assert_eq!((e.source, e.user_data, e.source_pid), (SourceState::Session, 5, 12));
    }

    #[test]
    fn an_empty_list_passes_the_event_through_unchanged() {
        let mut list = EventTapList::new();
        assert!(list.is_empty());
        let e = Event::key_down(key("cmd+s"), 10);
        assert_eq!(list.post(e), Some(e));
    }

    #[test]
    fn a_drop_stops_the_event_before_later_taps() {
        let mut list = EventTapList::new();
        list.create(TapPlacement::TailAppend, TapOptions::DefaultTap, EventMask::all(), |_| Decision::Drop);
        let seen = Rc::new(Cell::new(0u32));
        let s = seen.clone();
        list.create(TapPlacement::TailAppend, TapOptions::DefaultTap, EventMask::all(), move |_| {
            s.set(s.get() + 1);
            Decision::Pass
        });
        assert_eq!(list.post(Event::key_down(key("a"), 1)), None);
        assert_eq!(seen.get(), 0);
    }

    #[test]
    fn a_replacement_is_what_later_taps_see() {
        let mut list = EventTapList::new();
        let replacement = Event::mouse(EventType::MouseMoved, (3.0, 4.0), 7).unwrap();
        list.create(TapPlacement::HeadInsert, TapOptions::DefaultTap, EventMask::all(), move |_| {
            Decision::Replace(replacement)
        });
        let seen = Rc::new(Cell::new(0u32));
        let s = seen.clone();
        list.create(TapPlacement::TailAppend, TapOptions::DefaultTap, EventMask::of(&[EventType::MouseMoved]), move |e| {
            if e.location == (3.0, 4.0) {
                s.set(s.get() + 1);
            }
            Decision::Pass
        });
        assert_eq!(list.post(Event::key_down(key("a"), 1)), Some(replacement));
        assert_eq!(seen.get(), 1);
    }

    #[test]
    fn a_listen_only_tap_observes_but_cannot_change_the_event() {
        let mut list = EventTapList::new();
        let seen = Rc::new(Cell::new(0u32));
        let s = seen.clone();
        list.create(TapPlacement::HeadInsert, TapOptions::ListenOnly, EventMask::all(), move |_| {
            s.set(s.get() + 1);
            Decision::Drop
        });
        let e = Event::key_down(key("cmd+z"), 2);
        assert_eq!(list.post(e), Some(e));
        assert_eq!(seen.get(), 1);
    }

    #[test]
    fn a_disabled_tap_is_skipped_until_enabled_again() {
        let mut list = EventTapList::new();
        let id = list.create(TapPlacement::TailAppend, TapOptions::DefaultTap, EventMask::all(), |_| Decision::Drop);
        let e = Event::key_down(key("a"), 3);
        assert!(list.set_enabled(id, false));
        assert_eq!(list.is_enabled(id), Some(false));
        assert_eq!(list.post(e), Some(e));
        assert!(list.set_enabled(id, true));
        assert_eq!(list.post(e), None);
        assert!(!list.set_enabled(99, true));
        assert_eq!(list.options_of(id), Some(TapOptions::DefaultTap));
    }

    #[test]
    fn a_tap_only_sees_kinds_in_its_mask_and_head_insert_runs_first() {
        let mut list = EventTapList::new();
        let mouse_calls = Rc::new(Cell::new(0u32));
        let m = mouse_calls.clone();
        let tail = list.create(TapPlacement::TailAppend, TapOptions::DefaultTap, EventMask::of(&[EventType::LeftMouseDown]), move |_| {
            m.set(m.get() + 1);
            Decision::Pass
        });
        let head = list.create(TapPlacement::HeadInsert, TapOptions::DefaultTap, EventMask::all(), |_| Decision::Pass);
        assert_eq!(list.ids(), vec![head, tail]);
        list.post(Event::key_down(key("a"), 4));
        assert_eq!(mouse_calls.get(), 0);
        let click = Event::mouse(EventType::LeftMouseDown, (5.0, 6.0), 8).unwrap();
        list.post(click);
        assert_eq!(mouse_calls.get(), 1);
        assert!(list.remove(tail));
        assert!(!list.remove(tail));
        assert_eq!(list.ids(), vec![head]);
    }

    #[test]
    fn rules_drop_replace_and_pass_by_first_match() {
        let rules = [
            FilterRule { kind: Some(EventType::KeyDown), key_code: Some(30), flags: Some(EventFlags(EventFlags::COMMAND)), button: None, action: RuleAction::Drop },
            FilterRule { kind: Some(EventType::KeyDown), key_code: Some(30), flags: None, button: None, action: RuleAction::ReplaceKey(48) },
            FilterRule { kind: Some(EventType::LeftMouseDown), key_code: None, flags: None, button: Some(0), action: RuleAction::Drop },
        ];
        let mut cmd_a = Event::key(30, true, 1);
        cmd_a.flags = EventFlags(EventFlags::COMMAND | EventFlags::SHIFT);
        assert_eq!(decide(&rules, &cmd_a), Decision::Drop);
        match decide(&rules, &Event::key(30, true, 2)) {
            Decision::Replace(e) => assert_eq!(e.key_code, 48),
            other => panic!("expected a replacement, got {other:?}"),
        }
        assert_eq!(decide(&rules, &Event::key(31, true, 3)), Decision::Pass);
        assert_eq!(decide(&rules, &Event::mouse(EventType::LeftMouseDown, (0.0, 0.0), 4).unwrap()), Decision::Drop);
        assert_eq!(decide(&rules, &Event::mouse(EventType::RightMouseDown, (0.0, 0.0), 5).unwrap()), Decision::Pass);
    }

    #[test]
    fn a_rule_survives_the_wire() {
        for r in [
            FilterRule { kind: Some(EventType::KeyDown), key_code: Some(30), flags: Some(EventFlags(EventFlags::ALT)), button: None, action: RuleAction::ReplaceKey(5) },
            FilterRule { kind: None, key_code: None, flags: None, button: Some(2), action: RuleAction::Pass },
        ] {
            assert_eq!(FilterRule::from_json(&r.to_json()), Some(r));
        }
        assert_eq!(FilterRule::from_json(&obj(vec![("action", text("explode"))])), None);
    }

    #[test]
    fn tap_names_parse() {
        assert_eq!(TapOptions::from_name("listen"), Some(TapOptions::ListenOnly));
        assert_eq!(TapOptions::from_name(TapOptions::DefaultTap.name()), Some(TapOptions::DefaultTap));
        assert_eq!(TapOptions::from_name("x"), None);
        assert_eq!(TapPlacement::from_name("head"), Some(TapPlacement::HeadInsert));
        assert_eq!(TapPlacement::from_name("x"), None);
    }
}
