//! The routing core of the MIDI server: the endpoint registry plus a timestamped queue per listening client and per
//! destination. A virtual source and destination exist per app. Events keep their host-clock timestamp and their
//! arrival order: the queues are stable-sorted by timestamp, so two events with the same stamp leave in the order
//! they came in.
//!
//! Pure and allocation-friendly (this is the control path, not an audio thread); the daemon owns one `Hub`.

use crate::endpoints::{Direction, ObjectId, Registry};
use crate::ump::Ump;
use std::collections::BTreeMap;

/// One packet with its host-clock timestamp (nanoseconds) and the source it came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timed {
    pub host_ns: u64,
    pub source: ObjectId,
    pub packet: Ump,
}

#[derive(Default)]
pub struct Hub {
    pub registry: Registry,
    inboxes: BTreeMap<ObjectId, Vec<Timed>>,
    dest_queues: BTreeMap<ObjectId, Vec<Timed>>,
}

fn insert_stable(q: &mut Vec<Timed>, t: Timed) {
    let at = q.partition_point(|e| e.host_ns <= t.host_ns);
    q.insert(at, t);
}

impl Hub {
    pub fn new() -> Hub {
        Hub::default()
    }

    /// Registers an app and gives it a virtual source and a virtual destination named after it.
    pub fn create_app(&mut self, name: &str) -> (ObjectId, ObjectId, ObjectId) {
        let client = self.registry.create_client(name);
        let src = self
            .registry
            .create_virtual_endpoint(client, Direction::Source, name)
            .expect("the client was just created");
        let dst = self
            .registry
            .create_virtual_endpoint(client, Direction::Destination, name)
            .expect("the client was just created");
        self.inboxes.insert(client, Vec::new());
        self.dest_queues.insert(dst, Vec::new());
        (client, src, dst)
    }

    /// Registers a client with an input port (a listener inbox) and no endpoints. The caller adds virtual endpoints
    /// with `create_virtual_source` and `create_virtual_destination`.
    pub fn create_client(&mut self, name: &str) -> ObjectId {
        let client = self.registry.create_client(name);
        self.inboxes.insert(client, Vec::new());
        client
    }

    /// A virtual source owned by `client`; the client injects events into it with `send_from_source`.
    pub fn create_virtual_source(&mut self, client: ObjectId, name: &str) -> Option<ObjectId> {
        self.registry.create_virtual_endpoint(client, Direction::Source, name)
    }

    /// A virtual destination owned by `client`; events sent to it queue until `take_destination` drains them.
    pub fn create_virtual_destination(&mut self, client: ObjectId, name: &str) -> Option<ObjectId> {
        let dst = self.registry.create_virtual_endpoint(client, Direction::Destination, name)?;
        self.dest_queues.insert(dst, Vec::new());
        Some(dst)
    }

    /// Disposes a virtual endpoint with its connections and anything queued for it. `false` for hardware endpoints
    /// and unknown ids.
    pub fn dispose_endpoint(&mut self, id: ObjectId) -> bool {
        if !self.registry.dispose_endpoint(id) {
            return false;
        }
        self.dest_queues.remove(&id);
        true
    }

    /// Stops queueing for a hardware destination that went away.
    pub fn untrack_destination(&mut self, dst: ObjectId) {
        self.dest_queues.remove(&dst);
    }

    /// The destinations with a queue (virtual and hardware), in identifier order.
    pub fn tracked_destinations(&self) -> Vec<ObjectId> {
        self.dest_queues.keys().copied().collect()
    }

    /// The clients with an inbox, in identifier order.
    pub fn clients(&self) -> Vec<ObjectId> {
        self.inboxes.keys().copied().collect()
    }

    /// Removes an app with its virtual endpoints and anything still queued for it.
    pub fn dispose_app(&mut self, client: ObjectId) -> bool {
        let owned: Vec<ObjectId> = self.dest_queues.keys().copied().filter(|d| {
            self.registry.endpoint(*d).and_then(|e| e.owner) == Some(client)
        }).collect();
        if !self.registry.dispose_client(client) {
            return false;
        }
        self.inboxes.remove(&client);
        for d in owned {
            self.dest_queues.remove(&d);
        }
        true
    }

    /// Adds a hardware destination (for example a rawmidi output) so `send_to_destination` can queue for it.
    pub fn track_destination(&mut self, dst: ObjectId) {
        self.dest_queues.entry(dst).or_default();
    }

    /// An event arrives from a source. It is queued for every client connected to that source.
    /// Returns how many clients received it.
    pub fn send_from_source(&mut self, source: ObjectId, host_ns: u64, packet: Ump) -> usize {
        let listeners: Vec<ObjectId> = self
            .inboxes
            .keys()
            .copied()
            .filter(|c| self.registry.is_connected(*c, source))
            .collect();
        for c in &listeners {
            if let Some(q) = self.inboxes.get_mut(c) {
                insert_stable(q, Timed { host_ns, source, packet });
            }
        }
        listeners.len()
    }

    /// A client sends to a destination. The destination's driver (or owner) drains it with `take_destination`.
    /// `false` when the destination is not tracked.
    pub fn send_to_destination(&mut self, from: ObjectId, dst: ObjectId, host_ns: u64, packet: Ump) -> bool {
        match self.dest_queues.get_mut(&dst) {
            Some(q) => {
                insert_stable(q, Timed { host_ns, source: from, packet });
                true
            }
            None => false,
        }
    }

    /// The events of a client stamped at or before `until_ns`, in order. Later events stay queued.
    pub fn drain_inbox(&mut self, client: ObjectId, until_ns: u64) -> Vec<Timed> {
        Self::drain(self.inboxes.get_mut(&client), until_ns)
    }

    /// The events queued for a destination stamped at or before `until_ns`, in order.
    pub fn take_destination(&mut self, dst: ObjectId, until_ns: u64) -> Vec<Timed> {
        Self::drain(self.dest_queues.get_mut(&dst), until_ns)
    }

    fn drain(q: Option<&mut Vec<Timed>>, until_ns: u64) -> Vec<Timed> {
        match q {
            Some(q) => {
                let n = q.partition_point(|e| e.host_ns <= until_ns);
                q.drain(..n).collect()
            }
            None => Vec::new(),
        }
    }

    /// The earliest timestamp still queued for a live client or destination, so a daemon can sleep until then.
    /// Queues of objects that no longer exist are ignored.
    pub fn next_due_ns(&self) -> Option<u64> {
        let inboxes = self
            .inboxes
            .iter()
            .filter(|(c, _)| self.registry.client_name(**c).is_some())
            .filter_map(|(_, q)| q.first());
        let dests = self
            .dest_queues
            .iter()
            .filter(|(d, _)| self.registry.endpoint(**d).is_some())
            .filter_map(|(_, q)| q.first());
        inboxes.chain(dests).map(|t| t.host_ns).min()
    }

    /// The number of events waiting for a client.
    pub fn pending(&self, client: ObjectId) -> usize {
        self.inboxes.get(&client).map_or(0, Vec::len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::HostClock;
    use crate::events::MidiEvent;
    use crate::ump::{from_midi1, midi1_to_midi2};

    fn note(key: u8) -> Ump {
        from_midi1(0, MidiEvent::NoteOn { channel: 0, key, velocity: 100 })
    }

    #[test]
    fn loopback_keeps_order_and_timestamps() {
        let mut hub = Hub::new();
        let (_a, a_src, _a_dst) = hub.create_app("synth-out");
        let (b, _b_src, _b_dst) = hub.create_app("daw-in");
        assert!(hub.registry.connect_source(b, a_src));
        // Out of order arrival with equal stamps in the middle.
        hub.send_from_source(a_src, 3_000, note(3));
        hub.send_from_source(a_src, 1_000, note(1));
        hub.send_from_source(a_src, 2_000, note(2));
        hub.send_from_source(a_src, 2_000, note(22));
        let got = hub.drain_inbox(b, u64::MAX);
        let keys: Vec<(u64, u32)> = got.iter().map(|t| (t.host_ns, (t.packet.words[0] >> 8) & 0x7F)).collect();
        assert_eq!(keys, vec![(1_000, 1), (2_000, 2), (2_000, 22), (3_000, 3)]);
        assert!(got.iter().all(|t| t.source == a_src));
    }

    #[test]
    fn destination_queue_orders_by_timestamp_then_arrival() {
        let mut hub = Hub::new();
        let (_a, _, a_dst) = hub.create_app("a");
        let (_b, b_src, _) = hub.create_app("b");
        for (t, k) in [(50u64, 1u8), (10, 2), (50, 3), (u64::MAX, 4), (10, 5), (0, 6)] {
            assert!(hub.send_to_destination(b_src, a_dst, t, note(k)));
        }
        let got = hub.take_destination(a_dst, 50);
        let keys: Vec<(u64, u32)> = got.iter().map(|t| (t.host_ns, (t.packet.words[0] >> 8) & 0x7F)).collect();
        assert_eq!(keys, vec![(0, 6), (10, 2), (10, 5), (50, 1), (50, 3)]);
        // The event at u64::MAX stays until the horizon reaches it.
        assert_eq!(hub.take_destination(a_dst, u64::MAX - 1).len(), 0);
        assert_eq!(hub.take_destination(a_dst, u64::MAX).len(), 1);
    }

    #[test]
    fn future_events_stay_queued() {
        let mut hub = Hub::new();
        let (_a, a_src, _) = hub.create_app("a");
        let (b, _, _) = hub.create_app("b");
        hub.registry.connect_source(b, a_src);
        hub.send_from_source(a_src, 100, note(1));
        hub.send_from_source(a_src, 900, note(2));
        assert_eq!(hub.drain_inbox(b, 500).len(), 1);
        assert_eq!(hub.pending(b), 1);
        assert_eq!(hub.drain_inbox(b, 1000).len(), 1);
    }

    #[test]
    fn unconnected_client_hears_nothing() {
        let mut hub = Hub::new();
        let (_a, a_src, _) = hub.create_app("a");
        let (c, _, _) = hub.create_app("c");
        assert_eq!(hub.send_from_source(a_src, 1, note(1)), 0);
        assert_eq!(hub.pending(c), 0);
    }

    #[test]
    fn destination_gets_what_a_client_sends() {
        let mut hub = Hub::new();
        let (a, _, a_dst) = hub.create_app("a");
        let (b, b_src, _) = hub.create_app("b");
        assert!(hub.send_to_destination(b_src, a_dst, 10, note(5)));
        let got = hub.take_destination(a_dst, 10);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].source, b_src);
        assert!(!hub.send_to_destination(b_src, ObjectId(9999), 1, note(1)));
        let _ = (a, b);
    }

    #[test]
    fn disposing_an_app_drops_its_queues() {
        let mut hub = Hub::new();
        let (a, a_src, a_dst) = hub.create_app("a");
        let (b, _, _) = hub.create_app("b");
        hub.registry.connect_source(b, a_src);
        hub.send_to_destination(a_src, a_dst, 1, note(1));
        assert!(hub.dispose_app(a));
        assert!(hub.registry.endpoint(a_src).is_none());
        assert_eq!(hub.take_destination(a_dst, u64::MAX).len(), 0);
        assert!(!hub.dispose_app(a));
        // The listener is still there but its connection went with the source.
        assert_eq!(hub.send_from_source(a_src, 1, note(1)), 0);
        assert_eq!(hub.pending(b), 0);
    }

    #[test]
    fn sample_accurate_delivery_at_64_frames() {
        // A keyboard event stamped 144 samples into the timeline lands on frame 48 of the cycle 96..160.
        let clock = HostClock::new(0.0, 0, 48_000.0, 1.0);
        let mut hub = Hub::new();
        let (_k, k_src, _) = hub.create_app("keyboard");
        let (daw, _, _) = hub.create_app("daw");
        hub.registry.connect_source(daw, k_src);
        hub.send_from_source(k_src, clock.host_at(144.0), midi1_to_midi2(0, MidiEvent::NoteOn { channel: 0, key: 60, velocity: 100 }));
        let cycle_end_ns = clock.host_at(160.0);
        let evs = hub.drain_inbox(daw, cycle_end_ns);
        assert_eq!(evs.len(), 1);
        assert_eq!(clock.frame_in_cycle(evs[0].host_ns, 96.0, 64), Some(48));
    }

    #[test]
    fn next_due_is_the_earliest_queued_stamp_of_live_objects() {
        let mut hub = Hub::new();
        assert_eq!(hub.next_due_ns(), None);
        let (_a, a_src, a_dst) = hub.create_app("a");
        let (b, _, _) = hub.create_app("b");
        hub.registry.connect_source(b, a_src);
        hub.send_from_source(a_src, 700, note(1));
        hub.send_to_destination(a_src, a_dst, 300, note(2));
        assert_eq!(hub.next_due_ns(), Some(300));
        hub.take_destination(a_dst, 300);
        assert_eq!(hub.next_due_ns(), Some(700));
        // a queue whose destination left the registry no longer counts
        let dev = hub.registry.add_device("D", true);
        let ent = hub.registry.add_entity(dev, "E").unwrap();
        let hw = hub.registry.add_endpoint(ent, Direction::Destination, "in").unwrap();
        hub.track_destination(hw);
        hub.send_to_destination(a_src, hw, 5, note(3));
        assert_eq!(hub.next_due_ns(), Some(5));
        hub.registry.remove_external_device(dev);
        assert_eq!(hub.next_due_ns(), Some(700));
    }

    #[test]
    fn clients_and_virtual_endpoints_made_one_by_one_have_queues() {
        let mut hub = Hub::new();
        let c = hub.create_client("app");
        let d = hub.create_client("other");
        let src = hub.create_virtual_source(c, "out").unwrap();
        let dst = hub.create_virtual_destination(c, "in").unwrap();
        assert_eq!(hub.clients(), vec![c, d]);
        assert!(hub.registry.connect_source(d, src));
        assert_eq!(hub.send_from_source(src, 1, note(1)), 1);
        assert!(hub.send_to_destination(src, dst, 2, note(2)));
        assert_eq!(hub.tracked_destinations(), vec![dst]);
        assert!(hub.dispose_endpoint(dst));
        assert!(hub.tracked_destinations().is_empty());
        assert!(!hub.dispose_endpoint(dst));
        assert!(hub.create_virtual_source(ObjectId(999), "x").is_none());
    }
}
