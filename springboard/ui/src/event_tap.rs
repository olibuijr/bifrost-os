//! Event taps: a pure model of the filter chain for low-level input events. A tap watches the event kinds in its mask,
//! in list order, and answers each event with a decision: pass it on, drop it, or replace it with another event. A
//! listen-only tap sees events but cannot change them. An event posted to the list runs through the enabled taps in
//! order, and it reaches the stream only when no default tap dropped it. The model reads no hardware and posts
//! nothing to a live input stream: callers hand it synthetic events and read back what survives.

use crate::keys::KeyCombo;

/// The kinds of input event a tap can watch. Mouse kinds carry a location; the key kind carries a key combination.
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
}

/// Every event kind, in mask-bit order.
const ALL_TYPES: [EventType; 9] = [
    EventType::Null,
    EventType::LeftMouseDown,
    EventType::LeftMouseUp,
    EventType::RightMouseDown,
    EventType::RightMouseUp,
    EventType::MouseMoved,
    EventType::LeftMouseDragged,
    EventType::RightMouseDragged,
    EventType::KeyDown,
];

impl EventType {
    /// Whether this kind is a mouse event and so carries a location.
    pub fn is_mouse(self) -> bool {
        !matches!(self, EventType::Null | EventType::KeyDown)
    }

    /// The bit this kind takes in an [`EventMask`]: one bit per kind, in declaration order.
    fn bit(self) -> u64 {
        1u64 << (self as u32)
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
}

/// One input event. `location` is in desktop points and only meaningful for mouse kinds; `key` is set for key kinds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Event {
    pub kind: EventType,
    pub timestamp_ns: u64,
    pub location: (f32, f32),
    pub key: Option<KeyCombo>,
}

impl Event {
    /// A synthetic mouse event. `None` when `kind` is not a mouse kind.
    pub fn mouse(kind: EventType, location: (f32, f32), timestamp_ns: u64) -> Option<Event> {
        if kind.is_mouse() {
            Some(Event { kind, timestamp_ns, location, key: None })
        } else {
            None
        }
    }

    /// A synthetic key-down event for a key combination.
    pub fn key_down(combo: KeyCombo, timestamp_ns: u64) -> Event {
        Event { kind: EventType::KeyDown, timestamp_ns, location: (0.0, 0.0), key: Some(combo) }
    }
}

/// Whether a tap sees the events in its mask as a default filter or only listens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TapOptions {
    /// The tap may drop or replace events.
    DefaultTap,
    /// The tap sees events, and its decisions are ignored.
    ListenOnly,
}

/// Where a new tap goes in the list: before every existing tap, or after them all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TapPlacement {
    HeadInsert,
    TailAppend,
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

#[cfg(test)]
mod tests {
    use super::*;
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
    }

    #[test]
    fn mouse_constructor_refuses_a_key_kind() {
        assert!(Event::mouse(EventType::KeyDown, (1.0, 2.0), 5).is_none());
        let e = Event::mouse(EventType::MouseMoved, (1.0, 2.0), 5).unwrap();
        assert_eq!(e.location, (1.0, 2.0));
        assert_eq!(e.key, None);
    }

    #[test]
    fn an_empty_list_passes_the_event_through_unchanged() {
        let mut list = EventTapList::new();
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
}
