//! Bluetooth device events: an app registers for connect and disconnect events, of any device or of one address, and
//! reads the events queued for it. It can remove a registration at any time, and a removed registration gets no more
//! events.
//!
//! `DeviceEvents::register` returns a `Registration`. Each registration has its own queue of at most
//! `QUEUE_CAPACITY` events; when a queue is full, its oldest event is dropped and counted. `post` offers an event to
//! every registration whose filter matches and returns how many took it. `notice_for` and `notices_for_registration`
//! turn queued events into user notifications, only for an app whose authorization allows posting. This is the model
//! only: the shell handles its own connect and disconnect events, and nothing feeds this queue yet, so the matrix row
//! that uses it is partial.

use std::collections::{BTreeMap, VecDeque};

use bifrost_core::bt_address::BdAddr;

use crate::usernotif_model::AuthorizationStatus;

/// Most events one registration keeps before the oldest is dropped.
pub const QUEUE_CAPACITY: usize = 64;

/// Whether a device connected or disconnected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DeviceEventKind {
    Connected,
    Disconnected,
}

/// One connect or disconnect event for the device at `address`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeviceEvent {
    pub kind: DeviceEventKind,
    pub address: BdAddr,
}

/// Which events a registration wants: the kinds, and optionally one address (`None` is any device).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Filter {
    pub connected: bool,
    pub disconnected: bool,
    pub address: Option<BdAddr>,
}

impl Filter {
    /// Every connect and disconnect event of any device.
    pub fn all() -> Self {
        Filter { connected: true, disconnected: true, address: None }
    }

    /// True when `event` passes this filter.
    pub fn matches(&self, event: &DeviceEvent) -> bool {
        let kind_ok = match event.kind {
            DeviceEventKind::Connected => self.connected,
            DeviceEventKind::Disconnected => self.disconnected,
        };
        let address_ok = self.address.map_or(true, |a| a == event.address);
        kind_ok && address_ok
    }
}

/// The handle of one registration. Only `DeviceEvents` creates them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Registration(u64);

#[derive(Debug)]
struct Slot {
    filter: Filter,
    queue: VecDeque<DeviceEvent>,
    dropped: usize,
}

/// The registrations of one app and their queues.
#[derive(Debug, Default)]
pub struct DeviceEvents {
    next: u64,
    slots: BTreeMap<u64, Slot>,
}

impl DeviceEvents {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a registration with `filter` and returns its handle.
    pub fn register(&mut self, filter: Filter) -> Registration {
        self.next += 1;
        let id = self.next;
        self.slots.insert(id, Slot { filter, queue: VecDeque::new(), dropped: 0 });
        Registration(id)
    }

    /// Removes a registration. Returns false when it was not registered, or was removed already.
    pub fn unregister(&mut self, reg: Registration) -> bool {
        self.slots.remove(&reg.0).is_some()
    }

    /// True while the registration exists.
    pub fn is_registered(&self, reg: Registration) -> bool {
        self.slots.contains_key(&reg.0)
    }

    /// Queues `event` on every registration whose filter matches. Returns how many took it.
    pub fn post(&mut self, event: DeviceEvent) -> usize {
        let mut taken = 0;
        for slot in self.slots.values_mut() {
            if !slot.filter.matches(&event) {
                continue;
            }
            if slot.queue.len() == QUEUE_CAPACITY {
                slot.queue.pop_front();
                slot.dropped += 1;
            }
            slot.queue.push_back(event);
            taken += 1;
        }
        taken
    }

    /// Drains the queue of one registration, oldest first. Empty for an unknown registration.
    pub fn take(&mut self, reg: Registration) -> Vec<DeviceEvent> {
        match self.slots.get_mut(&reg.0) {
            Some(slot) => slot.queue.drain(..).collect(),
            None => Vec::new(),
        }
    }

    /// Events waiting in the queue of one registration (zero for an unknown one).
    pub fn pending(&self, reg: Registration) -> usize {
        self.slots.get(&reg.0).map_or(0, |s| s.queue.len())
    }

    /// Events dropped from the queue of one registration because it was full.
    pub fn dropped(&self, reg: Registration) -> usize {
        self.slots.get(&reg.0).map_or(0, |s| s.dropped)
    }
}

/// The user notification shown for one device event: a title and a body, both plain text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceNotice {
    pub title: String,
    pub body: String,
}

/// The notice for `event`. The body names the device when `name` is known and not blank, and shows its address otherwise.
pub fn notice_for(event: &DeviceEvent, name: Option<&str>) -> DeviceNotice {
    let verb = match event.kind {
        DeviceEventKind::Connected => "connected",
        DeviceEventKind::Disconnected => "disconnected",
    };
    let body = match name.map(str::trim).filter(|n| !n.is_empty()) {
        Some(known) => known.to_string(),
        None => event.address.to_string(),
    };
    DeviceNotice { title: format!("Bluetooth device {verb}"), body }
}

/// The notices for the queued `events` of one registration, oldest first. An app that may not post notifications gets
/// none; `name_of` supplies the known name of each device address.
pub fn notices_for_registration(
    events: Vec<DeviceEvent>,
    status: AuthorizationStatus,
    name_of: impl Fn(&BdAddr) -> Option<String>,
) -> Vec<DeviceNotice> {
    if !status.allows_posting() {
        return Vec::new();
    }
    events.iter().map(|event| notice_for(event, name_of(&event.address).as_deref())).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(last: u8) -> BdAddr {
        BdAddr([0, 0, 0, 0, 0, last])
    }

    fn event(kind: DeviceEventKind, last: u8) -> DeviceEvent {
        DeviceEvent { kind, address: addr(last) }
    }

    #[test]
    fn a_matching_event_is_queued_once_and_drained_once() {
        let mut ev = DeviceEvents::new();
        let r = ev.register(Filter::all());
        assert_eq!(ev.post(event(DeviceEventKind::Connected, 1)), 1);
        assert_eq!(ev.pending(r), 1);
        assert_eq!(ev.take(r), vec![event(DeviceEventKind::Connected, 1)]);
        assert!(ev.take(r).is_empty());
    }

    #[test]
    fn an_address_and_kind_filter_passes_only_its_events() {
        let mut ev = DeviceEvents::new();
        let r = ev.register(Filter { connected: true, disconnected: false, address: Some(addr(1)) });
        assert_eq!(ev.post(event(DeviceEventKind::Connected, 2)), 0);
        assert_eq!(ev.post(event(DeviceEventKind::Connected, 1)), 1);
        assert_eq!(ev.post(event(DeviceEventKind::Disconnected, 1)), 0);
        assert_eq!(ev.take(r), vec![event(DeviceEventKind::Connected, 1)]);
    }

    #[test]
    fn an_unregistered_handle_gets_nothing_and_cannot_be_removed_twice() {
        let mut ev = DeviceEvents::new();
        let r = ev.register(Filter::all());
        assert!(ev.is_registered(r));
        assert!(ev.unregister(r));
        assert!(!ev.is_registered(r));
        assert!(!ev.unregister(r));
        assert_eq!(ev.post(event(DeviceEventKind::Disconnected, 1)), 0);
        assert!(ev.take(r).is_empty());
        assert_eq!(ev.pending(r), 0);
    }

    #[test]
    fn a_full_queue_drops_the_oldest_event_and_counts_it() {
        let mut ev = DeviceEvents::new();
        let r = ev.register(Filter::all());
        for last in 0..70u8 {
            ev.post(event(DeviceEventKind::Connected, last));
        }
        assert_eq!(ev.pending(r), QUEUE_CAPACITY);
        assert_eq!(ev.dropped(r), 6);
        let queued = ev.take(r);
        assert_eq!(queued.len(), 64);
        assert_eq!(queued[0].address, addr(6));
        assert_eq!(queued[63].address, addr(69));
    }

    #[test]
    fn two_registrations_keep_separate_queues() {
        let mut ev = DeviceEvents::new();
        let all = ev.register(Filter::all());
        let down = ev.register(Filter { connected: false, disconnected: true, address: None });
        assert_eq!(ev.post(event(DeviceEventKind::Disconnected, 3)), 2);
        assert_eq!(ev.post(event(DeviceEventKind::Connected, 4)), 1);
        assert_eq!(ev.take(all).len(), 2);
        assert_eq!(ev.take(down), vec![event(DeviceEventKind::Disconnected, 3)]);
    }

    #[test]
    fn posting_with_no_registrations_takes_nothing() {
        let mut ev = DeviceEvents::new();
        assert_eq!(ev.post(event(DeviceEventKind::Connected, 9)), 0);
    }
}
