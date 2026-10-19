//! The server side of the client protocol, without a socket: one `Broker` owns the routing `Hub` and one small state
//! per connection. The daemon feeds it the requests it decoded (`handle`), asks it for the events to push
//! (`pump`) and for what must go out to hardware (`hardware_output`). Because no socket is touched, the whole
//! protocol is testable on the host and the SDK client is tested against it.
//!
//! One client has one input port: it hears every source it connected, in host-time order, as `Received`. Packets sent
//! to a virtual destination it owns come back as `Delivered`. Timestamps are host-clock nanoseconds; 0 means now.

use crate::endpoints::{Direction, ObjectId};
use crate::events::MidiEvent;
use crate::hub::{Hub, Timed};
use crate::thru::{ControlTransform, ThruConnection, ThruEvent, ThruParams, ValueMap};
use crate::ump::{from_midi1, midi1_to_midi2, midi2_to_midi1, to_midi1, Ump};
use crate::wire::{DeviceInfo, EndpointInfo, EntityInfo, Message, Refusal, Request, Setup, ThruSpec, VERSION};
use std::collections::BTreeMap;

/// A connection of the daemon's socket, numbered by the daemon.
pub type ConnId = u64;

/// The most virtual endpoints one client may own, so a runaway app cannot fill the registry.
pub const MAX_ENDPOINTS_PER_CLIENT: usize = 128;
/// The most thru connections one client may own.
pub const MAX_THRUS_PER_CLIENT: usize = 32;

struct Conn {
    client: Option<ObjectId>,
    /// The setup generation the client was last told about.
    seen_generation: u32,
}

/// A play-through from a source to a destination, run inside the server.
struct Thru {
    owner: ObjectId,
    source: ObjectId,
    destination: ObjectId,
    conn: ThruConnection,
}

pub struct Broker {
    pub hub: Hub,
    conns: BTreeMap<ConnId, Conn>,
    next_conn: ConnId,
    generation: u32,
    thrus: BTreeMap<u32, Thru>,
    next_thru: u32,
}

/// A value table from the wire: empty is the identity.
fn map_from(table: &[u8]) -> Option<ValueMap> {
    if table.is_empty() {
        return Some(ValueMap::identity());
    }
    let t: [u8; 128] = table.try_into().ok()?;
    ValueMap::from_table(t)
}

/// The thru parameters a wire spec describes; `None` when a table is malformed.
fn params_from(spec: &ThruSpec) -> Option<ThruParams> {
    let mut controls = Vec::with_capacity(spec.controls.len());
    for (from, to, table) in &spec.controls {
        controls.push(ControlTransform { from: *from, to: *to, value: map_from(table)? });
    }
    Some(ThruParams { channel: spec.channel, note: map_from(&spec.note)?, velocity: map_from(&spec.velocity)?, controls })
}

fn channel_of(e: &MidiEvent) -> u8 {
    match *e {
        MidiEvent::NoteOff { channel, .. }
        | MidiEvent::NoteOn { channel, .. }
        | MidiEvent::PolyPressure { channel, .. }
        | MidiEvent::ControlChange { channel, .. }
        | MidiEvent::ProgramChange { channel, .. }
        | MidiEvent::ChannelPressure { channel, .. }
        | MidiEvent::PitchBend { channel, .. } => channel,
    }
}

/// What a thru connection forwards for one packet. Notes and control changes go through its maps and transforms,
/// other channel messages pass when the channel filter allows, and system messages always pass (when enabled).
fn thru_packet(conn: &ThruConnection, p: Ump) -> Option<Ump> {
    if !conn.enabled() {
        return None;
    }
    let mt = p.message_type();
    let ev = match mt {
        2 => to_midi1(&p),
        4 => midi2_to_midi1(&p),
        _ => None,
    };
    let Some(ev) = ev else { return Some(p) };
    let te = match ev {
        MidiEvent::NoteOn { channel, key, velocity } => ThruEvent::NoteOn { channel, note: key, velocity },
        MidiEvent::NoteOff { channel, key, velocity } => ThruEvent::NoteOff { channel, note: key, velocity },
        MidiEvent::ControlChange { channel, controller, value } => ThruEvent::ControlChange { channel, control: controller, value },
        other => {
            let pass = conn.params().channel.map_or(true, |f| f == channel_of(&other));
            return if pass { Some(p) } else { None };
        }
    };
    let out = match conn.process(te)? {
        ThruEvent::NoteOn { channel, note, velocity } => MidiEvent::NoteOn { channel, key: note, velocity },
        ThruEvent::NoteOff { channel, note, velocity } => MidiEvent::NoteOff { channel, key: note, velocity },
        ThruEvent::ControlChange { channel, control, value } => {
            MidiEvent::ControlChange { channel, controller: control, value }
        }
    };
    Some(if mt == 2 { from_midi1(p.group(), out) } else { midi1_to_midi2(p.group(), out) })
}

impl Broker {
    pub fn new(hub: Hub) -> Broker {
        Broker { hub, conns: BTreeMap::new(), next_conn: 1, generation: 0, thrus: BTreeMap::new(), next_thru: 1 }
    }

    /// A new connection (no client until it says hello).
    pub fn open(&mut self) -> ConnId {
        let id = self.next_conn;
        self.next_conn += 1;
        self.conns.insert(id, Conn { client: None, seen_generation: self.generation });
        id
    }

    /// The connection went away: its client, virtual endpoints and queues go with it.
    pub fn close(&mut self, conn: ConnId) {
        if let Some(c) = self.conns.remove(&conn) {
            if let Some(client) = c.client {
                self.thrus.retain(|_, t| t.owner != client);
                if self.hub.dispose_app(client) {
                    self.generation += 1;
                }
                self.prune_thrus();
            }
        }
    }

    /// The number of open connections.
    pub fn connections(&self) -> usize {
        self.conns.len()
    }

    /// The daemon calls this when it added or removed a hardware device, entity or endpoint: every client is told.
    pub fn setup_changed(&mut self) {
        self.generation += 1;
        self.prune_thrus();
    }

    /// Drops the thru connections whose source or destination no longer exists.
    fn prune_thrus(&mut self) {
        let reg = &self.hub.registry;
        self.thrus.retain(|_, t| reg.endpoint(t.source).is_some() && reg.endpoint(t.destination).is_some());
    }

    /// A packet arrives at a source (hardware read, or a client's injection): it is queued for every client
    /// connected to the source and run through the thru connections that start there. The daemon calls this instead
    /// of `Hub::send_from_source`. Returns how many clients were connected.
    pub fn route(&mut self, source: ObjectId, host_ns: u64, packet: Ump) -> usize {
        let heard = self.hub.send_from_source(source, host_ns, packet);
        let out: Vec<(ObjectId, Ump)> = self
            .thrus
            .values()
            .filter(|t| t.source == source)
            .filter_map(|t| thru_packet(&t.conn, packet).map(|p| (t.destination, p)))
            .collect();
        for (dst, p) in out {
            self.hub.send_to_destination(source, dst, host_ns, p);
        }
        heard
    }

    /// The number of thru connections.
    pub fn thru_count(&self) -> usize {
        self.thrus.len()
    }

    pub fn generation(&self) -> u32 {
        self.generation
    }

    /// The client of a connection, once it has said hello.
    pub fn client_of(&self, conn: ConnId) -> Option<ObjectId> {
        self.conns.get(&conn).and_then(|c| c.client)
    }

    /// The setup as one client sees it (its own connections marked).
    pub fn setup_for(&self, client: Option<ObjectId>) -> Setup {
        let reg = &self.hub.registry;
        let id = |o: ObjectId| o.0;
        Setup {
            generation: self.generation,
            devices: reg
                .devices()
                .map(|d| DeviceInfo {
                    id: d.id.0,
                    name: d.name.clone(),
                    external: d.external,
                    entities: d.entities.iter().map(|e| id(*e)).collect(),
                })
                .collect(),
            entities: reg
                .entities()
                .map(|e| EntityInfo {
                    id: e.id.0,
                    name: e.name.clone(),
                    device: e.device.0,
                    sources: e.sources.iter().map(|s| id(*s)).collect(),
                    destinations: e.destinations.iter().map(|s| id(*s)).collect(),
                })
                .collect(),
            endpoints: reg
                .endpoints()
                .map(|p| EndpointInfo {
                    id: p.id.0,
                    name: p.name.clone(),
                    source: p.direction == Direction::Source,
                    entity: p.entity.map_or(0, id),
                    owner: p.owner.map_or(0, id),
                    ref_con: p.ref_con,
                    connected: p.direction == Direction::Source && client.is_some_and(|c| reg.is_connected(c, p.id)),
                })
                .collect(),
        }
    }

    fn owned_endpoints(&self, client: ObjectId) -> usize {
        self.hub.registry.endpoints().filter(|e| e.owner == Some(client)).count()
    }

    /// The first virtual source a client owns (the sender recorded for what it sends), or id 0.
    fn sender_of(&self, client: ObjectId) -> ObjectId {
        self.hub
            .registry
            .endpoints()
            .find(|e| e.owner == Some(client) && e.direction == Direction::Source)
            .map_or(ObjectId(0), |e| e.id)
    }

    /// Applies one request from a connection and returns the replies, in order.
    pub fn handle(&mut self, conn: ConnId, req: Request, now_ns: u64) -> Vec<Message> {
        let client = match self.conns.get(&conn) {
            Some(c) => c.client,
            None => return Vec::new(),
        };
        // Requests that need no client.
        match &req {
            Request::Ping => return vec![Message::Pong { host_ns: now_ns }],
            Request::Hello { name, version } => {
                if client.is_some() {
                    return vec![Message::Failed(Refusal::NoClient)];
                }
                if *version != VERSION {
                    return vec![Message::Failed(Refusal::BadVersion)];
                }
                let name = if name.trim().is_empty() { "client" } else { name.as_str() };
                let id = self.hub.create_client(name);
                if let Some(c) = self.conns.get_mut(&conn) {
                    c.client = Some(id);
                    c.seen_generation = self.generation;
                }
                return vec![Message::Welcome { client: id.0, version: VERSION }];
            }
            _ => {}
        }
        let Some(client) = client else {
            return vec![match req {
                Request::Send { .. } | Request::Inject { .. } => Message::SendRefused(Refusal::NoClient),
                _ => Message::Failed(Refusal::NoClient),
            }];
        };
        match req {
            Request::Ping | Request::Hello { .. } => Vec::new(),
            Request::List => {
                if let Some(c) = self.conns.get_mut(&conn) {
                    c.seen_generation = self.generation;
                }
                vec![Message::Setup(self.setup_for(Some(client)))]
            }
            Request::CreateSource { name } => {
                if self.owned_endpoints(client) >= MAX_ENDPOINTS_PER_CLIENT {
                    return vec![Message::Failed(Refusal::NotApplicable)];
                }
                match self.hub.create_virtual_source(client, &name) {
                    Some(id) => {
                        self.generation += 1;
                        vec![Message::Created { id: id.0 }]
                    }
                    None => vec![Message::Failed(Refusal::BadObject)],
                }
            }
            Request::CreateDestination { name } => {
                if self.owned_endpoints(client) >= MAX_ENDPOINTS_PER_CLIENT {
                    return vec![Message::Failed(Refusal::NotApplicable)];
                }
                match self.hub.create_virtual_destination(client, &name) {
                    Some(id) => {
                        self.generation += 1;
                        vec![Message::Created { id: id.0 }]
                    }
                    None => vec![Message::Failed(Refusal::BadObject)],
                }
            }
            Request::Dispose { id } => {
                let id = ObjectId(id);
                let mine = self.hub.registry.endpoint(id).is_some_and(|e| e.owner == Some(client));
                if mine && self.hub.dispose_endpoint(id) {
                    self.generation += 1;
                    self.prune_thrus();
                    vec![Message::Done]
                } else {
                    vec![Message::Failed(Refusal::BadObject)]
                }
            }
            Request::Connect { source } => {
                let source = ObjectId(source);
                match self.hub.registry.endpoint(source).map(|e| e.direction) {
                    None => vec![Message::Failed(Refusal::BadObject)],
                    Some(Direction::Destination) => vec![Message::Failed(Refusal::WrongDirection)],
                    Some(Direction::Source) => {
                        if self.hub.registry.connect_source(client, source) {
                            vec![Message::Done]
                        } else {
                            vec![Message::Failed(Refusal::NotApplicable)]
                        }
                    }
                }
            }
            Request::Disconnect { source } => {
                if self.hub.registry.disconnect_source(client, ObjectId(source)) {
                    vec![Message::Done]
                } else {
                    vec![Message::Failed(Refusal::NotApplicable)]
                }
            }
            Request::SetRefCon { id, value } => {
                let id = ObjectId(id);
                let mine = self.hub.registry.endpoint(id).is_some_and(|e| e.owner == Some(client));
                if mine && self.hub.registry.set_ref_con(id, value) {
                    vec![Message::Done]
                } else {
                    vec![Message::Failed(Refusal::BadObject)]
                }
            }
            Request::Send { destination, host_ns, packets } => {
                let dst = ObjectId(destination);
                match self.hub.registry.endpoint(dst).map(|e| e.direction) {
                    None => return vec![Message::SendRefused(Refusal::BadObject)],
                    Some(Direction::Source) => return vec![Message::SendRefused(Refusal::WrongDirection)],
                    Some(Direction::Destination) => {}
                }
                let from = self.sender_of(client);
                let stamp = if host_ns == 0 { now_ns } else { host_ns };
                for p in packets {
                    if !self.hub.send_to_destination(from, dst, stamp, p) {
                        return vec![Message::SendRefused(Refusal::BadObject)];
                    }
                }
                Vec::new()
            }
            Request::Inject { source, host_ns, packets } => {
                let src = ObjectId(source);
                match self.hub.registry.endpoint(src) {
                    None => return vec![Message::SendRefused(Refusal::BadObject)],
                    Some(e) if e.direction != Direction::Source => return vec![Message::SendRefused(Refusal::WrongDirection)],
                    Some(e) if e.owner != Some(client) => return vec![Message::SendRefused(Refusal::BadObject)],
                    Some(_) => {}
                }
                let stamp = if host_ns == 0 { now_ns } else { host_ns };
                for p in packets {
                    self.route(src, stamp, p);
                }
                Vec::new()
            }
            Request::ThruCreate { source, destination, spec } => {
                let (src, dst) = (ObjectId(source), ObjectId(destination));
                let dir = |id: ObjectId| self.hub.registry.endpoint(id).map(|e| e.direction);
                match (dir(src), dir(dst)) {
                    (None, _) | (_, None) => return vec![Message::Failed(Refusal::BadObject)],
                    (Some(Direction::Source), Some(Direction::Destination)) => {}
                    _ => return vec![Message::Failed(Refusal::WrongDirection)],
                }
                if self.thrus.values().filter(|t| t.owner == client).count() >= MAX_THRUS_PER_CLIENT {
                    return vec![Message::Failed(Refusal::NotApplicable)];
                }
                let Some(params) = params_from(&spec) else { return vec![Message::Failed(Refusal::NotApplicable)] };
                let id = self.next_thru;
                self.next_thru += 1;
                self.thrus.insert(id, Thru { owner: client, source: src, destination: dst, conn: ThruConnection::new(params) });
                vec![Message::Created { id }]
            }
            Request::ThruEnable { id, enabled } => match self.thrus.get_mut(&id) {
                Some(t) if t.owner == client => {
                    t.conn.set_enabled(enabled);
                    vec![Message::Done]
                }
                _ => vec![Message::Failed(Refusal::BadObject)],
            },
            Request::ThruDispose { id } => {
                if self.thrus.get(&id).is_some_and(|t| t.owner == client) {
                    self.thrus.remove(&id);
                    vec![Message::Done]
                } else {
                    vec![Message::Failed(Refusal::BadObject)]
                }
            }
        }
    }

    /// The events to push now: packets from connected sources and to virtual destinations that are due at
    /// `now_ns`, and a `SetupChanged` for each client that has not seen the latest setup.
    pub fn pump(&mut self, now_ns: u64) -> Vec<(ConnId, Message)> {
        let mut out = Vec::new();
        let generation = self.generation;
        let ids: Vec<ConnId> = self.conns.keys().copied().collect();
        for conn in ids {
            let Some(client) = self.conns.get(&conn).and_then(|c| c.client) else { continue };
            if let Some(c) = self.conns.get_mut(&conn) {
                if c.seen_generation != generation {
                    c.seen_generation = generation;
                    out.push((conn, Message::SetupChanged { generation }));
                }
            }
            for t in self.hub.drain_inbox(client, now_ns) {
                out.push((conn, Message::Received { source: t.source.0, host_ns: t.host_ns, packet: t.packet }));
            }
            let owned: Vec<ObjectId> = self
                .hub
                .tracked_destinations()
                .into_iter()
                .filter(|d| self.hub.registry.endpoint(*d).is_some_and(|e| e.owner == Some(client)))
                .collect();
            for dst in owned {
                for t in self.hub.take_destination(dst, now_ns) {
                    out.push((
                        conn,
                        Message::Delivered { destination: dst.0, from: t.source.0, host_ns: t.host_ns, packet: t.packet },
                    ));
                }
            }
        }
        out
    }

    /// What is due for the hardware destinations (the tracked destinations no client owns), in order per
    /// destination: the daemon encodes each packet to bytes and writes it to the port.
    pub fn hardware_output(&mut self, now_ns: u64) -> Vec<(ObjectId, Timed)> {
        let hardware: Vec<ObjectId> = self
            .hub
            .tracked_destinations()
            .into_iter()
            .filter(|d| self.hub.registry.endpoint(*d).is_some_and(|e| e.owner.is_none()))
            .collect();
        let mut out = Vec::new();
        for dst in hardware {
            for t in self.hub.take_destination(dst, now_ns) {
                out.push((dst, t));
            }
        }
        out
    }
}

/// A convenience for tests and callers: packets of one `Message::Received` batch for a source.
pub fn received_for(msgs: &[(ConnId, Message)], conn: ConnId) -> Vec<(u32, u64, Ump)> {
    msgs.iter()
        .filter(|(c, _)| *c == conn)
        .filter_map(|(_, m)| match m {
            Message::Received { source, host_ns, packet } => Some((*source, *host_ns, *packet)),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::MidiEvent;
    use crate::ump::from_midi1;

    fn note(key: u8) -> Ump {
        from_midi1(0, MidiEvent::NoteOn { channel: 0, key, velocity: 100 })
    }

    fn hello(b: &mut Broker, name: &str) -> (ConnId, u32) {
        let c = b.open();
        match b.handle(c, Request::Hello { name: name.into(), version: VERSION }, 0).as_slice() {
            [Message::Welcome { client, .. }] => (c, *client),
            other => panic!("{other:?}"),
        }
    }

    fn created(m: Vec<Message>) -> u32 {
        match m.as_slice() {
            [Message::Created { id }] => *id,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn nothing_works_before_hello_but_ping() {
        let mut b = Broker::new(Hub::new());
        let c = b.open();
        assert_eq!(b.handle(c, Request::List, 0), vec![Message::Failed(Refusal::NoClient)]);
        assert_eq!(b.handle(c, Request::CreateSource { name: "x".into() }, 0), vec![Message::Failed(Refusal::NoClient)]);
        assert_eq!(b.handle(c, Request::Send { destination: 1, host_ns: 0, packets: vec![note(1)] }, 0), vec![Message::SendRefused(Refusal::NoClient)]);
        assert_eq!(b.handle(c, Request::Ping, 77), vec![Message::Pong { host_ns: 77 }]);
        assert_eq!(b.handle(99, Request::Ping, 0), Vec::<Message>::new());
    }

    #[test]
    fn hello_twice_and_wrong_version_are_refused() {
        let mut b = Broker::new(Hub::new());
        let (c, _) = hello(&mut b, "a");
        assert_eq!(b.handle(c, Request::Hello { name: "a".into(), version: VERSION }, 0), vec![Message::Failed(Refusal::NoClient)]);
        let d = b.open();
        assert_eq!(b.handle(d, Request::Hello { name: "b".into(), version: VERSION + 1 }, 0), vec![Message::Failed(Refusal::BadVersion)]);
    }

    #[test]
    fn a_source_injected_by_one_client_reaches_the_connected_client_in_order() {
        let mut b = Broker::new(Hub::new());
        let (a, _) = hello(&mut b, "synth");
        let (d, _) = hello(&mut b, "daw");
        let src = created(b.handle(a, Request::CreateSource { name: "synth out".into() }, 0));
        assert_eq!(b.handle(d, Request::Connect { source: src }, 0), vec![Message::Done]);
        // a duplicate connection does not apply
        assert_eq!(b.handle(d, Request::Connect { source: src }, 0), vec![Message::Failed(Refusal::NotApplicable)]);
        assert!(b.handle(a, Request::Inject { source: src, host_ns: 300, packets: vec![note(3)] }, 1000).is_empty());
        assert!(b.handle(a, Request::Inject { source: src, host_ns: 100, packets: vec![note(1), note(2)] }, 1000).is_empty());
        let got = received_for(&b.pump(10_000), d);
        let keys: Vec<(u64, u32)> = got.iter().map(|(_, t, p)| (*t, (p.words[0] >> 8) & 0x7F)).collect();
        assert_eq!(keys, vec![(100, 1), (100, 2), (300, 3)]);
        assert!(got.iter().all(|(s, _, _)| *s == src));
        // the injecting client is not connected to its own source: it hears nothing
        assert!(received_for(&b.pump(10_000), a).is_empty());
    }

    #[test]
    fn injection_stamped_in_the_future_waits_and_zero_means_now() {
        let mut b = Broker::new(Hub::new());
        let (a, _) = hello(&mut b, "a");
        let (d, _) = hello(&mut b, "d");
        let src = created(b.handle(a, Request::CreateSource { name: "o".into() }, 0));
        b.handle(d, Request::Connect { source: src }, 0);
        b.handle(a, Request::Inject { source: src, host_ns: 5_000, packets: vec![note(1)] }, 100);
        b.handle(a, Request::Inject { source: src, host_ns: 0, packets: vec![note(2)] }, 100);
        let first = received_for(&b.pump(100), d);
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].1, 100);
        let second = received_for(&b.pump(5_000), d);
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].1, 5_000);
    }

    #[test]
    fn a_client_may_only_inject_into_its_own_source() {
        let mut b = Broker::new(Hub::new());
        let (a, _) = hello(&mut b, "a");
        let (d, _) = hello(&mut b, "d");
        let src = created(b.handle(a, Request::CreateSource { name: "o".into() }, 0));
        let dst = created(b.handle(d, Request::CreateDestination { name: "i".into() }, 0));
        assert_eq!(b.handle(d, Request::Inject { source: src, host_ns: 0, packets: vec![note(1)] }, 0), vec![Message::SendRefused(Refusal::BadObject)]);
        assert_eq!(b.handle(a, Request::Inject { source: dst, host_ns: 0, packets: vec![note(1)] }, 0), vec![Message::SendRefused(Refusal::WrongDirection)]);
        assert_eq!(b.handle(a, Request::Inject { source: 999, host_ns: 0, packets: vec![note(1)] }, 0), vec![Message::SendRefused(Refusal::BadObject)]);
    }

    #[test]
    fn packets_sent_to_a_virtual_destination_are_delivered_to_its_owner() {
        let mut b = Broker::new(Hub::new());
        let (a, _) = hello(&mut b, "sender");
        let (d, _) = hello(&mut b, "synth");
        let own = created(b.handle(a, Request::CreateSource { name: "s".into() }, 0));
        let dst = created(b.handle(d, Request::CreateDestination { name: "synth in".into() }, 0));
        assert!(b.handle(a, Request::Send { destination: dst, host_ns: 50, packets: vec![note(7), note(8)] }, 10).is_empty());
        let out = b.pump(60);
        let mine: Vec<&Message> = out.iter().filter(|(c, _)| *c == d).map(|(_, m)| m).collect();
        let delivered: Vec<(u32, u32, u64)> = mine
            .iter()
            .filter_map(|m| match m {
                Message::Delivered { destination, from, host_ns, .. } => Some((*destination, *from, *host_ns)),
                _ => None,
            })
            .collect();
        // the sender was recorded as the source of its first virtual endpoint
        assert_eq!(delivered, vec![(dst, own, 50), (dst, own, 50)]);
        assert!(out.iter().all(|(c, m)| *c != a || !matches!(m, Message::Delivered { .. })));
        assert!(b.pump(60).iter().all(|(_, m)| !matches!(m, Message::Delivered { .. })));
    }

    #[test]
    fn send_checks_the_destination() {
        let mut b = Broker::new(Hub::new());
        let (a, _) = hello(&mut b, "a");
        let src = created(b.handle(a, Request::CreateSource { name: "s".into() }, 0));
        assert_eq!(b.handle(a, Request::Send { destination: src, host_ns: 0, packets: vec![note(1)] }, 0), vec![Message::SendRefused(Refusal::WrongDirection)]);
        assert_eq!(b.handle(a, Request::Send { destination: 555, host_ns: 0, packets: vec![note(1)] }, 0), vec![Message::SendRefused(Refusal::BadObject)]);
    }

    #[test]
    fn hardware_destinations_collect_what_is_due() {
        let mut hub = Hub::new();
        let dev = hub.registry.add_device("Synth", true);
        let ent = hub.registry.add_entity(dev, "Port").unwrap();
        let out_ep = hub.registry.add_endpoint(ent, Direction::Destination, "Synth in").unwrap();
        hub.track_destination(out_ep);
        let mut b = Broker::new(hub);
        let (a, _) = hello(&mut b, "a");
        b.handle(a, Request::Send { destination: out_ep.0, host_ns: 0, packets: vec![note(60)] }, 500);
        b.handle(a, Request::Send { destination: out_ep.0, host_ns: 900, packets: vec![note(61)] }, 500);
        let due = b.hardware_output(500);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].0, out_ep);
        assert_eq!(due[0].1.host_ns, 500);
        assert!(b.hardware_output(899).is_empty());
        assert_eq!(b.hardware_output(900).len(), 1);
    }

    #[test]
    fn the_setup_lists_hardware_and_virtual_endpoints_with_connections() {
        let mut hub = Hub::new();
        let dev = hub.registry.add_device("Keys", true);
        let ent = hub.registry.add_entity(dev, "Port 1").unwrap();
        let kb = hub.registry.add_endpoint(ent, Direction::Source, "Keys out").unwrap();
        let mut b = Broker::new(hub);
        let (a, client) = hello(&mut b, "app");
        assert_eq!(b.handle(a, Request::Connect { source: kb.0 }, 0), vec![Message::Done]);
        let v = created(b.handle(a, Request::CreateDestination { name: "app in".into() }, 0));
        let setup = match b.handle(a, Request::List, 0).pop() {
            Some(Message::Setup(s)) => s,
            other => panic!("{other:?}"),
        };
        assert_eq!(setup.devices.len(), 1);
        assert_eq!(setup.devices[0].name, "Keys");
        assert_eq!(setup.entities[0].sources, vec![kb.0]);
        let hw = setup.endpoints.iter().find(|e| e.id == kb.0).unwrap();
        assert!(hw.source && hw.connected && hw.owner == 0 && hw.entity == ent.0);
        let virt = setup.endpoints.iter().find(|e| e.id == v).unwrap();
        assert!(!virt.source && virt.owner == client && virt.entity == 0);
    }

    #[test]
    fn setup_changes_are_pushed_once_to_every_client_that_has_not_seen_them() {
        let mut b = Broker::new(Hub::new());
        let (a, _) = hello(&mut b, "a");
        let (d, _) = hello(&mut b, "d");
        assert!(b.pump(0).is_empty());
        // hardware appears
        let dev = b.hub.registry.add_device("Pad", true);
        b.hub.registry.add_entity(dev, "p");
        b.setup_changed();
        let out = b.pump(0);
        assert_eq!(out.iter().filter(|(_, m)| matches!(m, Message::SetupChanged { .. })).count(), 2);
        assert!(b.pump(0).is_empty());
        // asking for the list counts as seeing it
        b.setup_changed();
        b.handle(a, Request::List, 0);
        let out = b.pump(0);
        assert!(out.iter().all(|(c, _)| *c == d), "{out:?}");
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn closing_a_connection_removes_its_client_and_endpoints() {
        let mut b = Broker::new(Hub::new());
        let (a, _) = hello(&mut b, "a");
        let (d, _) = hello(&mut b, "d");
        let src = created(b.handle(a, Request::CreateSource { name: "o".into() }, 0));
        let dst = created(b.handle(a, Request::CreateDestination { name: "i".into() }, 0));
        b.handle(d, Request::Connect { source: src }, 0);
        b.pump(0);
        b.close(a);
        assert_eq!(b.connections(), 1);
        assert!(b.hub.registry.endpoint(ObjectId(src)).is_none());
        assert!(b.hub.registry.endpoint(ObjectId(dst)).is_none());
        assert_eq!(b.handle(d, Request::Send { destination: dst, host_ns: 0, packets: vec![note(1)] }, 0), vec![Message::SendRefused(Refusal::BadObject)]);
        // the survivor is told the setup changed
        assert!(b.pump(0).iter().any(|(c, m)| *c == d && matches!(m, Message::SetupChanged { .. })));
    }

    #[test]
    fn dispose_only_works_on_own_virtual_endpoints_and_ref_con_sticks() {
        let mut hub = Hub::new();
        let dev = hub.registry.add_device("Keys", true);
        let ent = hub.registry.add_entity(dev, "p").unwrap();
        let kb = hub.registry.add_endpoint(ent, Direction::Source, "k").unwrap();
        let mut b = Broker::new(hub);
        let (a, _) = hello(&mut b, "a");
        let (d, _) = hello(&mut b, "d");
        let src = created(b.handle(a, Request::CreateSource { name: "o".into() }, 0));
        assert_eq!(b.handle(d, Request::Dispose { id: src }, 0), vec![Message::Failed(Refusal::BadObject)]);
        assert_eq!(b.handle(a, Request::Dispose { id: kb.0 }, 0), vec![Message::Failed(Refusal::BadObject)]);
        assert_eq!(b.handle(a, Request::SetRefCon { id: src, value: 42 }, 0), vec![Message::Done]);
        assert_eq!(b.hub.registry.ref_con(ObjectId(src)), Some(42));
        assert_eq!(b.handle(a, Request::Dispose { id: src }, 0), vec![Message::Done]);
        assert!(b.hub.registry.endpoint(ObjectId(src)).is_none());
    }

    #[test]
    fn a_client_cannot_own_unlimited_endpoints() {
        let mut b = Broker::new(Hub::new());
        let (a, _) = hello(&mut b, "a");
        for i in 0..MAX_ENDPOINTS_PER_CLIENT {
            created(b.handle(a, Request::CreateSource { name: format!("s{i}") }, 0));
        }
        assert_eq!(b.handle(a, Request::CreateSource { name: "one more".into() }, 0), vec![Message::Failed(Refusal::NotApplicable)]);
    }
}
