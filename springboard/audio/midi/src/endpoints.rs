//! MIDI object registry: clients, devices, entities, sources and destinations, and the connections from a client to
//! its sources.
//!
//! Pure: no hardware is probed and no port is opened. The caller adds the devices its own drivers found and records
//! the connections it makes. Virtual endpoints belong to the client that created them and are removed with it.
//! Hardware endpoints stay until the caller removes them. Object identifiers are never reused in one registry, so an
//! identifier that was looked up earlier cannot silently point at a different object. The vocabulary follows the
//! official framework's MIDI services (clients, devices, entities, endpoints, input ports and source connections).

use std::collections::{BTreeMap, BTreeSet};

/// The identifier of a client, device, entity or endpoint. Unique within one registry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ObjectId(pub u32);

/// A source produces events; a destination receives them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Source,
    Destination,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    pub id: ObjectId,
    pub name: String,
    /// True for a device plugged in from outside the machine, false for one built in.
    pub external: bool,
    pub entities: Vec<ObjectId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entity {
    pub id: ObjectId,
    pub name: String,
    pub device: ObjectId,
    pub sources: Vec<ObjectId>,
    pub destinations: Vec<ObjectId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Endpoint {
    pub id: ObjectId,
    pub name: String,
    pub direction: Direction,
    /// The entity that owns a hardware endpoint. `None` for a virtual endpoint.
    pub entity: Option<ObjectId>,
    /// The client that owns a virtual endpoint. `None` for a hardware endpoint.
    pub owner: Option<ObjectId>,
    /// Caller-defined context value, stored and returned unchanged.
    pub ref_con: u64,
}

impl Endpoint {
    pub fn is_virtual(&self) -> bool {
        self.owner.is_some()
    }
}

#[derive(Clone, Debug, Default)]
pub struct Registry {
    last_id: u32,
    clients: BTreeMap<ObjectId, String>,
    devices: BTreeMap<ObjectId, Device>,
    entities: BTreeMap<ObjectId, Entity>,
    endpoints: BTreeMap<ObjectId, Endpoint>,
    /// (client, source) pairs: the client's input port receives events from that source.
    connections: BTreeSet<(ObjectId, ObjectId)>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    fn allocate(&mut self) -> ObjectId {
        self.last_id += 1;
        ObjectId(self.last_id)
    }

    /// Creates a client. Its identifier is returned for use with the other calls.
    pub fn create_client(&mut self, name: &str) -> ObjectId {
        let id = self.allocate();
        self.clients.insert(id, name.to_string());
        id
    }

    pub fn client_name(&self, id: ObjectId) -> Option<&str> {
        self.clients.get(&id).map(|s| s.as_str())
    }

    /// Removes a client with its virtual endpoints and its connections. Returns `false` for an unknown client.
    pub fn dispose_client(&mut self, id: ObjectId) -> bool {
        if self.clients.remove(&id).is_none() {
            return false;
        }
        let owned: Vec<ObjectId> = self
            .endpoints
            .values()
            .filter(|e| e.owner == Some(id))
            .map(|e| e.id)
            .collect();
        for endpoint in owned {
            self.endpoints.remove(&endpoint);
        }
        let endpoints = &self.endpoints;
        self.connections
            .retain(|&(client, source)| client != id && endpoints.contains_key(&source));
        true
    }

    pub fn add_device(&mut self, name: &str, external: bool) -> ObjectId {
        let id = self.allocate();
        self.devices.insert(
            id,
            Device {
                id,
                name: name.to_string(),
                external,
                entities: Vec::new(),
            },
        );
        id
    }

    /// Adds an entity to a device. Returns `None` when the device is unknown.
    pub fn add_entity(&mut self, device: ObjectId, name: &str) -> Option<ObjectId> {
        if !self.devices.contains_key(&device) {
            return None;
        }
        let id = self.allocate();
        self.entities.insert(
            id,
            Entity {
                id,
                name: name.to_string(),
                device,
                sources: Vec::new(),
                destinations: Vec::new(),
            },
        );
        if let Some(d) = self.devices.get_mut(&device) {
            d.entities.push(id);
        }
        Some(id)
    }

    /// Adds a hardware endpoint to an entity. Returns `None` when the entity is unknown.
    pub fn add_endpoint(&mut self, entity: ObjectId, direction: Direction, name: &str) -> Option<ObjectId> {
        if !self.entities.contains_key(&entity) {
            return None;
        }
        let id = self.allocate();
        self.endpoints.insert(
            id,
            Endpoint {
                id,
                name: name.to_string(),
                direction,
                entity: Some(entity),
                owner: None,
                ref_con: 0,
            },
        );
        if let Some(e) = self.entities.get_mut(&entity) {
            match direction {
                Direction::Source => e.sources.push(id),
                Direction::Destination => e.destinations.push(id),
            }
        }
        Some(id)
    }

    /// Creates a virtual endpoint owned by a client. Returns `None` when the client is unknown.
    pub fn create_virtual_endpoint(&mut self, client: ObjectId, direction: Direction, name: &str) -> Option<ObjectId> {
        if !self.clients.contains_key(&client) {
            return None;
        }
        let id = self.allocate();
        self.endpoints.insert(
            id,
            Endpoint {
                id,
                name: name.to_string(),
                direction,
                entity: None,
                owner: Some(client),
                ref_con: 0,
            },
        );
        Some(id)
    }

    /// Disposes a virtual endpoint and its connections. Hardware endpoints cannot be disposed here, and `false` is
    /// returned for them.
    pub fn dispose_endpoint(&mut self, id: ObjectId) -> bool {
        match self.endpoints.get(&id) {
            Some(e) if e.is_virtual() => {}
            _ => return false,
        }
        self.endpoints.remove(&id);
        self.connections.retain(|&(_, source)| source != id);
        true
    }

    /// Removes an entity with its endpoints and every connection to those endpoints. The owning device keeps its other
    /// entities. Returns `false` for an unknown entity.
    pub fn remove_entity(&mut self, id: ObjectId) -> bool {
        let entity = match self.entities.remove(&id) {
            Some(e) => e,
            None => return false,
        };
        if let Some(d) = self.devices.get_mut(&entity.device) {
            d.entities.retain(|&e| e != id);
        }
        for endpoint in entity.sources.iter().chain(entity.destinations.iter()) {
            self.endpoints.remove(endpoint);
        }
        self.connections.retain(|&(_, source)| !entity.sources.contains(&source));
        true
    }

    /// Removes a device with all its entities, as the system does when a driver removes the device. Returns `false`
    /// for an unknown device.
    pub fn remove_device(&mut self, id: ObjectId) -> bool {
        let device = match self.devices.remove(&id) {
            Some(d) => d,
            None => return false,
        };
        for entity in device.entities {
            self.remove_entity(entity);
        }
        true
    }

    /// Removes an external device, as unplugging it does. Returns `false` when the device is unknown or built in.
    pub fn remove_external_device(&mut self, id: ObjectId) -> bool {
        let external = self.devices.get(&id).map_or(false, |d| d.external);
        if external {
            self.remove_device(id)
        } else {
            false
        }
    }

    /// Every client with its name, in identifier order.
    pub fn clients(&self) -> impl Iterator<Item = (ObjectId, &str)> {
        self.clients.iter().map(|(id, name)| (*id, name.as_str()))
    }

    /// Every device (built in and external), in identifier order.
    pub fn devices(&self) -> impl Iterator<Item = &Device> {
        self.devices.values()
    }

    /// Every entity, in identifier order.
    pub fn entities(&self) -> impl Iterator<Item = &Entity> {
        self.entities.values()
    }

    /// Every endpoint (hardware and virtual, sources and destinations), in identifier order.
    pub fn endpoints(&self) -> impl Iterator<Item = &Endpoint> {
        self.endpoints.values()
    }

    pub fn device(&self, id: ObjectId) -> Option<&Device> {
        self.devices.get(&id)
    }

    pub fn entity(&self, id: ObjectId) -> Option<&Entity> {
        self.entities.get(&id)
    }

    pub fn endpoint(&self, id: ObjectId) -> Option<&Endpoint> {
        self.endpoints.get(&id)
    }

    pub fn number_of_devices(&self) -> usize {
        self.devices.len()
    }

    pub fn number_of_external_devices(&self) -> usize {
        self.devices.values().filter(|d| d.external).count()
    }

    /// The external device at `index`, counted in identifier order.
    pub fn external_device_at(&self, index: usize) -> Option<ObjectId> {
        self.devices.values().filter(|d| d.external).nth(index).map(|d| d.id)
    }

    /// The number of endpoints with the given direction, hardware and virtual together.
    pub fn number_of_endpoints(&self, direction: Direction) -> usize {
        self.endpoints.values().filter(|e| e.direction == direction).count()
    }

    /// The endpoint at `index` among those with the given direction, counted in identifier order.
    pub fn endpoint_at(&self, direction: Direction, index: usize) -> Option<ObjectId> {
        self.endpoints
            .values()
            .filter(|e| e.direction == direction)
            .nth(index)
            .map(|e| e.id)
    }

    pub fn set_ref_con(&mut self, id: ObjectId, value: u64) -> bool {
        match self.endpoints.get_mut(&id) {
            Some(e) => {
                e.ref_con = value;
                true
            }
            None => false,
        }
    }

    pub fn ref_con(&self, id: ObjectId) -> Option<u64> {
        self.endpoints.get(&id).map(|e| e.ref_con)
    }

    /// Connects a client's input to a source. Returns `false` when the client or source is unknown, when the endpoint
    /// is not a source, or when the connection already exists.
    pub fn connect_source(&mut self, client: ObjectId, source: ObjectId) -> bool {
        let is_source = matches!(
            self.endpoints.get(&source),
            Some(e) if e.direction == Direction::Source
        );
        if !is_source || !self.clients.contains_key(&client) {
            return false;
        }
        self.connections.insert((client, source))
    }

    /// Removes a connection. Returns `false` when it did not exist.
    pub fn disconnect_source(&mut self, client: ObjectId, source: ObjectId) -> bool {
        self.connections.remove(&(client, source))
    }

    pub fn is_connected(&self, client: ObjectId, source: ObjectId) -> bool {
        self.connections.contains(&(client, source))
    }

    /// The sources a client listens to, in identifier order.
    pub fn sources_for(&self, client: ObjectId) -> Vec<ObjectId> {
        self.connections
            .iter()
            .filter(|&&(c, _)| c == client)
            .map(|&(_, s)| s)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_increase_and_are_never_reused() {
        let mut r = Registry::new();
        let a = r.create_client("first");
        let b = r.create_client("second");
        assert_eq!(a, ObjectId(1));
        assert_eq!(b, ObjectId(2));
        assert!(r.dispose_client(a));
        let c = r.create_client("third");
        assert_eq!(c, ObjectId(3));
        assert_eq!(r.client_name(a), None);
        assert_eq!(r.client_name(b), Some("second"));
    }

    #[test]
    fn hardware_endpoints_sit_in_entities_and_devices() {
        let mut r = Registry::new();
        let dev = r.add_device("keyboard", true);
        let ent = r.add_entity(dev, "keys").unwrap();
        let src = r.add_endpoint(ent, Direction::Source, "out").unwrap();
        let dst = r.add_endpoint(ent, Direction::Destination, "in").unwrap();
        assert_eq!(r.entity(ent).unwrap().sources, vec![src]);
        assert_eq!(r.entity(ent).unwrap().destinations, vec![dst]);
        assert_eq!(r.device(dev).unwrap().entities, vec![ent]);
        assert_eq!(r.number_of_external_devices(), 1);
        assert_eq!(r.external_device_at(0), Some(dev));
        assert_eq!(r.external_device_at(1), None);
        assert_eq!(r.number_of_endpoints(Direction::Source), 1);
        assert_eq!(r.endpoint(src).unwrap().entity, Some(ent));
        assert!(!r.endpoint(src).unwrap().is_virtual());
        // An entity or device that does not exist refuses new children.
        assert_eq!(r.add_entity(ObjectId(99), "ghost"), None);
        assert_eq!(r.add_endpoint(ObjectId(99), Direction::Source, "ghost"), None);
    }

    #[test]
    fn counts_and_indices_follow_direction_and_identifier_order() {
        let mut r = Registry::new();
        let client = r.create_client("app");
        let dev = r.add_device("synth", false);
        let ent = r.add_entity(dev, "main").unwrap();
        let hw_src = r.add_endpoint(ent, Direction::Source, "hw").unwrap();
        let v_dst = r.create_virtual_endpoint(client, Direction::Destination, "virtual").unwrap();
        let v_src = r.create_virtual_endpoint(client, Direction::Source, "virtual out").unwrap();
        assert_eq!(r.number_of_endpoints(Direction::Source), 2);
        assert_eq!(r.number_of_endpoints(Direction::Destination), 1);
        assert_eq!(r.endpoint_at(Direction::Source, 0), Some(hw_src));
        assert_eq!(r.endpoint_at(Direction::Source, 1), Some(v_src));
        assert_eq!(r.endpoint_at(Direction::Source, 2), None);
        assert_eq!(r.endpoint_at(Direction::Destination, 0), Some(v_dst));
        assert!(r.endpoint(v_src).unwrap().is_virtual());
        assert_eq!(r.number_of_external_devices(), 0);
    }

    #[test]
    fn connect_requires_a_source_and_a_known_client() {
        let mut r = Registry::new();
        let client = r.create_client("app");
        let src = r.create_virtual_endpoint(client, Direction::Source, "out").unwrap();
        let dst = r.create_virtual_endpoint(client, Direction::Destination, "in").unwrap();
        // A destination cannot be a connection source.
        assert!(!r.connect_source(client, dst));
        assert!(r.connect_source(client, src));
        // The same pair cannot be connected twice.
        assert!(!r.connect_source(client, src));
        assert!(r.is_connected(client, src));
        assert!(!r.connect_source(ObjectId(77), src));
        assert_eq!(r.sources_for(client), vec![src]);
        assert!(r.disconnect_source(client, src));
        assert!(!r.disconnect_source(client, src));
        assert!(r.sources_for(client).is_empty());
    }

    #[test]
    fn disposing_a_client_removes_its_virtual_endpoints_and_connections() {
        let mut r = Registry::new();
        let dev = r.add_device("controller", true);
        let ent = r.add_entity(dev, "pads").unwrap();
        let hw = r.add_endpoint(ent, Direction::Source, "pads out").unwrap();
        let app = r.create_client("app");
        let other = r.create_client("other");
        let v_src = r.create_virtual_endpoint(app, Direction::Source, "app out").unwrap();
        assert!(r.connect_source(app, hw));
        assert!(r.connect_source(other, hw));
        assert!(r.connect_source(other, v_src));
        assert!(r.dispose_client(app));
        assert!(r.endpoint(v_src).is_none());
        assert_eq!(r.number_of_endpoints(Direction::Source), 1);
        assert!(!r.is_connected(app, hw));
        // The other client keeps its connection to the hardware source, but loses the disposed virtual one.
        assert_eq!(r.sources_for(other), vec![hw]);
        assert!(!r.dispose_client(app));
    }

    #[test]
    fn only_virtual_endpoints_can_be_disposed_and_ref_cons_round_trip() {
        let mut r = Registry::new();
        let client = r.create_client("app");
        let dev = r.add_device("bank", true);
        let ent = r.add_entity(dev, "ch").unwrap();
        let hw = r.add_endpoint(ent, Direction::Destination, "in").unwrap();
        let v = r.create_virtual_endpoint(client, Direction::Source, "out").unwrap();
        assert!(!r.dispose_endpoint(hw));
        assert!(r.endpoint(hw).is_some());
        assert_eq!(r.ref_con(v), Some(0));
        assert!(r.set_ref_con(v, 0xDEAD_BEEF));
        assert_eq!(r.ref_con(v), Some(0xDEAD_BEEF));
        assert!(r.dispose_endpoint(v));
        assert!(!r.dispose_endpoint(v));
        assert!(!r.set_ref_con(v, 1));
    }

    #[test]
    fn removing_a_device_removes_its_entities_endpoints_and_connections() {
        let mut r = Registry::new();
        let app = r.create_client("app");
        let dev = r.add_device("keys", true);
        let ent = r.add_entity(dev, "ch 1").unwrap();
        let src = r.add_endpoint(ent, Direction::Source, "out").unwrap();
        let other = r.add_device("pad", true);
        let pad_ent = r.add_entity(other, "ch 1").unwrap();
        let pad = r.add_endpoint(pad_ent, Direction::Source, "out").unwrap();
        assert!(r.connect_source(app, src));
        assert!(r.connect_source(app, pad));
        assert!(r.remove_device(dev));
        assert!(r.device(dev).is_none());
        assert!(r.entity(ent).is_none());
        assert!(r.endpoint(src).is_none());
        assert!(!r.is_connected(app, src));
        assert_eq!(r.sources_for(app), vec![pad]);
        assert_eq!(r.number_of_devices(), 1);
        assert!(!r.remove_device(dev));
    }

    #[test]
    fn removing_one_entity_keeps_the_device_and_its_other_entities() {
        let mut r = Registry::new();
        let dev = r.add_device("bank", true);
        let a = r.add_entity(dev, "a").unwrap();
        let b = r.add_entity(dev, "b").unwrap();
        let ep = r.add_endpoint(b, Direction::Destination, "in").unwrap();
        assert!(r.remove_entity(a));
        assert_eq!(r.device(dev).unwrap().entities, vec![b]);
        assert!(r.endpoint(ep).is_some());
        assert!(r.remove_entity(b));
        assert!(r.endpoint(ep).is_none());
        assert_eq!(r.device(dev).unwrap().entities, Vec::<ObjectId>::new());
        assert!(!r.remove_entity(b));
    }

    #[test]
    fn only_an_external_device_can_be_removed_as_unplugged() {
        let mut r = Registry::new();
        let built_in = r.add_device("internal", false);
        let plugged = r.add_device("usb", true);
        assert!(!r.remove_external_device(built_in));
        assert_eq!(r.number_of_external_devices(), 1);
        assert!(r.remove_external_device(plugged));
        assert_eq!(r.number_of_external_devices(), 0);
        assert_eq!(r.number_of_devices(), 1);
        assert!(r.device(built_in).is_some());
        assert!(!r.remove_external_device(plugged));
    }

    #[test]
    fn identifiers_are_not_reused_after_a_removal() {
        let mut r = Registry::new();
        let dev = r.add_device("usb", true);
        assert!(r.remove_device(dev));
        let next = r.add_device("usb again", true);
        assert_eq!(dev, ObjectId(1));
        assert_eq!(next, ObjectId(2));
        assert_eq!(r.external_device_at(0), Some(next));
    }
}
