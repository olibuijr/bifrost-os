//! Accessory manager: the connected hardware accessories, the listeners that follow them, and the exclusive open
//! and protocol-session rules that sit on top.
//!
//! Pure: no bus, no thread, no file descriptor. The caller reports attach and detach with the descriptors it read
//! (USB device and configuration descriptors come from `usb_descriptors`), and it drains each listener's events in
//! its own loop. Listeners are keyed by a number the caller chooses, which stands for the listener object.
//!
//! Model, in the order the rules apply:
//! - An accessory gets a connection id at attach. The id is never reused, and a detached accessory keeps its record
//!   so an app can still read its identity. Its connection id reads as `None` once it is gone.
//! - A listener registers with matching criteria. The register call returns the accessories already connected that
//!   match, and later attach and detach changes are queued for it as events. A detach reaches only the listeners
//!   that were told about the attach.
//! - Opening an accessory is exclusive: one owner at a time. Closing needs the owner that opened it.
//! - A protocol session is allowed only for a connected accessory that declares the protocol string.
//!
//! The model mirrors the USB accessory manager (shared manager, listener registration with criteria, attach and
//! detach events, exclusive open) and the per-accessory identity and protocol matching of the older accessory
//! framework. Session data streams are not modelled. Unplugging a device in the kernel is the caller's job to
//! report with `detach`.

use crate::usb_descriptors::{ConfigurationDescriptor, DeviceDescriptor, MatchingCriteria};

/// Error values for accessory operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The listener key is already registered.
    ListenerAlreadyRegistered,
    /// The accessory is disconnected, or another owner has it open.
    NotAccessible,
    /// The accessory is not in the state the call needs: the same owner opens twice, a close comes from a
    /// non-owner, or a session asks for a protocol the accessory does not declare.
    InvalidState,
    /// The listener key was never registered. No framework code is named for this case; the model reports it here.
    Internal,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::ListenerAlreadyRegistered => write!(f, "A listener is already registered under this key"),
            Error::NotAccessible => write!(f, "The accessory is disconnected or open for another owner"),
            Error::InvalidState => write!(f, "The accessory is not in the state this call needs"),
            Error::Internal => write!(f, "The listener is not registered"),
        }
    }
}

/// A connection change for one accessory, carrying its connection id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Connected(u32),
    Disconnected(u32),
}

/// One accessory: the bus identity, its descriptors, the manufacturer strings and the protocols it declares.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Accessory {
    /// The connection id. Zero until the manager assigns one at attach.
    pub id: u32,
    /// The bus identity the kernel reports for the device.
    pub registry_id: u64,
    pub device: DeviceDescriptor,
    /// The selected configuration, when the caller has read it.
    pub configuration: Option<ConfigurationDescriptor>,
    /// The display name, manufacturer, model, serial, firmware and hardware strings, as the device reports them.
    pub name: String,
    pub manufacturer: String,
    pub model_number: String,
    pub serial_number: String,
    pub firmware_revision: String,
    pub hardware_revision: String,
    /// The protocol strings the device declares. A session can only use one of these.
    pub protocol_strings: Vec<String>,
    connected: bool,
    opened_by: Option<u32>,
}

impl Accessory {
    /// An accessory with empty strings and no protocols. The manager assigns the connection id at attach.
    pub fn new(registry_id: u64, device: DeviceDescriptor) -> Self {
        Accessory {
            id: 0,
            registry_id,
            device,
            configuration: None,
            name: String::new(),
            manufacturer: String::new(),
            model_number: String::new(),
            serial_number: String::new(),
            firmware_revision: String::new(),
            hardware_revision: String::new(),
            protocol_strings: Vec::new(),
            connected: false,
            opened_by: None,
        }
    }

    pub fn is_connected(&self) -> bool {
        self.connected
    }

    /// The connection id while connected. A disconnected accessory has none.
    pub fn connection_id(&self) -> Option<u32> {
        if self.connected {
            Some(self.id)
        } else {
            None
        }
    }

    /// The owner that holds the accessory open, if any.
    pub fn opened_by(&self) -> Option<u32> {
        self.opened_by
    }

    pub fn supports_protocol(&self, protocol: &str) -> bool {
        self.protocol_strings.iter().any(|p| p == protocol)
    }

    /// Whether the accessory's device and configuration match the criteria. A missing configuration counts as one
    /// with no interfaces, so criteria that name an interface do not match it.
    fn matches(&self, criteria: &MatchingCriteria) -> bool {
        match &self.configuration {
            Some(config) => criteria.matches(&self.device, config),
            None => criteria.matches(&self.device, &empty_configuration()),
        }
    }
}

/// A session between the app and one protocol of one accessory. The data streams are not part of this model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Session {
    pub accessory: u32,
    pub protocol: String,
}

struct Listener {
    key: u32,
    criteria: MatchingCriteria,
    /// The connection ids this listener has been told are connected.
    matched: Vec<u32>,
    pending: Vec<Event>,
}

/// The accessories and listeners of one process.
#[derive(Debug, Default)]
pub struct AccessoryManager {
    last_id: u32,
    accessories: Vec<Accessory>,
    listeners: Vec<Listener>,
}

impl std::fmt::Debug for Listener {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Listener").field("key", &self.key).field("matched", &self.matched).finish()
    }
}

fn empty_configuration() -> ConfigurationDescriptor {
    ConfigurationDescriptor { value: 0, attributes: 0, max_power_ma: 0, interfaces: Vec::new() }
}

impl AccessoryManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records an attach and returns the connection id. A registry id that is already connected returns its
    /// existing id and changes nothing. Otherwise the accessory gets the next id, and every listener whose criteria
    /// match it is queued a `Connected` event.
    pub fn attach(&mut self, mut accessory: Accessory) -> u32 {
        if let Some(existing) = self.accessories.iter().find(|a| a.connected && a.registry_id == accessory.registry_id) {
            return existing.id;
        }
        self.last_id += 1;
        let id = self.last_id;
        accessory.id = id;
        accessory.connected = true;
        accessory.opened_by = None;
        for listener in self.listeners.iter_mut() {
            if !listener.matched.contains(&id) && accessory.matches(&listener.criteria) {
                listener.matched.push(id);
                listener.pending.push(Event::Connected(id));
            }
        }
        self.accessories.push(accessory);
        id
    }

    /// Records a detach by registry id and returns the connection id that went away. The record stays, marked
    /// disconnected and not open. Only the listeners that were told about the attach hear about the detach.
    pub fn detach(&mut self, registry_id: u64) -> Option<u32> {
        let accessory = self.accessories.iter_mut().find(|a| a.connected && a.registry_id == registry_id)?;
        accessory.connected = false;
        accessory.opened_by = None;
        let id = accessory.id;
        for listener in self.listeners.iter_mut() {
            if let Some(at) = listener.matched.iter().position(|&m| m == id) {
                listener.matched.remove(at);
                listener.pending.push(Event::Disconnected(id));
            }
        }
        Some(id)
    }

    /// Registers a listener under `key`. Returns the connected accessories that already match, so the caller gets the
    /// current set before any change. Registering the same key again is an error.
    pub fn register(&mut self, key: u32, criteria: MatchingCriteria) -> Result<Vec<u32>, Error> {
        if self.listeners.iter().any(|l| l.key == key) {
            return Err(Error::ListenerAlreadyRegistered);
        }
        let matched: Vec<u32> =
            self.accessories.iter().filter(|a| a.connected && a.matches(&criteria)).map(|a| a.id).collect();
        self.listeners.push(Listener { key, criteria, matched: matched.clone(), pending: Vec::new() });
        Ok(matched)
    }

    /// Removes a listener. Its queued events are dropped with it. An unknown key is `Error::Internal`.
    pub fn unregister(&mut self, key: u32) -> Result<(), Error> {
        let at = self.listeners.iter().position(|l| l.key == key).ok_or(Error::Internal)?;
        self.listeners.remove(at);
        Ok(())
    }

    /// Drains the events queued for a listener, oldest first. An unknown key has no events.
    pub fn take_events(&mut self, key: u32) -> Vec<Event> {
        match self.listeners.iter_mut().find(|l| l.key == key) {
            Some(listener) => std::mem::take(&mut listener.pending),
            None => Vec::new(),
        }
    }

    /// Opens a connected accessory for `owner`. The open is exclusive: another owner gets `NotAccessible`, and the
    /// same owner opening twice gets `InvalidState`.
    pub fn open(&mut self, id: u32, owner: u32) -> Result<(), Error> {
        let accessory = self.accessories.iter_mut().find(|a| a.id == id).ok_or(Error::NotAccessible)?;
        if !accessory.connected {
            return Err(Error::NotAccessible);
        }
        match accessory.opened_by {
            None => {
                accessory.opened_by = Some(owner);
                Ok(())
            }
            Some(holder) if holder == owner => Err(Error::InvalidState),
            Some(_) => Err(Error::NotAccessible),
        }
    }

    /// Closes the accessory for its owner. Any other caller, or a close with nothing open, gets `InvalidState`.
    pub fn close(&mut self, id: u32, owner: u32) -> Result<(), Error> {
        let accessory = self.accessories.iter_mut().find(|a| a.id == id).ok_or(Error::NotAccessible)?;
        if accessory.opened_by == Some(owner) {
            accessory.opened_by = None;
            Ok(())
        } else {
            Err(Error::InvalidState)
        }
    }

    /// The record for a connection id, connected or not.
    pub fn accessory(&self, id: u32) -> Option<&Accessory> {
        self.accessories.iter().find(|a| a.id == id)
    }

    /// The connected accessories in the order they arrived.
    pub fn connected_accessories(&self) -> Vec<&Accessory> {
        self.accessories.iter().filter(|a| a.connected).collect()
    }

    /// Starts a session with a connected accessory on a protocol it declares. A disconnected accessory gives
    /// `NotAccessible`; an undeclared protocol gives `InvalidState`.
    pub fn session(&self, id: u32, protocol: &str) -> Result<Session, Error> {
        let accessory = self.accessory(id).filter(|a| a.connected).ok_or(Error::NotAccessible)?;
        if !accessory.supports_protocol(protocol) {
            return Err(Error::InvalidState);
        }
        Ok(Session { accessory: id, protocol: protocol.to_string() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usb_descriptors::{DeviceMatch, InterfaceDescriptor, InterfaceMatch};

    fn device(vid: u16, pid: u16) -> DeviceDescriptor {
        DeviceDescriptor {
            usb_version: 0x0200,
            class: 0,
            subclass: 0,
            protocol: 0,
            max_packet_size0: 64,
            vendor_id: vid,
            product_id: pid,
            device_version: 0x0100,
            configurations: 1,
        }
    }

    /// A configuration with one interface of the given class.
    fn config_with_class(class: u8) -> ConfigurationDescriptor {
        let interface = InterfaceDescriptor {
            number: 0,
            alternate: 0,
            class,
            subclass: 0,
            protocol: 0,
            endpoints: Vec::new(),
        };
        ConfigurationDescriptor { value: 1, attributes: 0x80, max_power_ma: 500, interfaces: vec![interface] }
    }

    fn accessory(registry_id: u64, vid: u16) -> Accessory {
        Accessory::new(registry_id, device(vid, 0x0001))
    }

    fn by_vendor(vid: u16) -> MatchingCriteria {
        MatchingCriteria { device: DeviceMatch { vendor_id: Some(vid), ..Default::default() }, ..Default::default() }
    }

    #[test]
    fn attach_assigns_increasing_ids_and_a_repeat_attach_keeps_its_id() {
        let mut m = AccessoryManager::new();
        let first = m.attach(accessory(100, 0x1234));
        let second = m.attach(accessory(101, 0x1234));
        assert_eq!((first, second), (1, 2));
        // the same registry id while connected is the same accessory
        assert_eq!(m.attach(accessory(100, 0x1234)), 1);
        assert_eq!(m.connected_accessories().len(), 2);
        assert_eq!(m.accessory(1).and_then(|a| a.connection_id()), Some(1));
    }

    #[test]
    fn a_detached_accessory_keeps_its_record_but_loses_its_connection_id() {
        let mut m = AccessoryManager::new();
        let id = m.attach(accessory(7, 0x1234));
        assert_eq!(m.detach(7), Some(id));
        let record = m.accessory(id).expect("record kept");
        assert!(!record.is_connected());
        assert_eq!(record.connection_id(), None);
        assert_eq!(m.detach(7), None, "a second detach of the same registry id changes nothing");
        // reattaching the same device gets a new id; ids are never reused
        let again = m.attach(accessory(7, 0x1234));
        assert_eq!(again, 2);
    }

    #[test]
    fn register_returns_the_connected_matches_and_later_changes_reach_only_matching_listeners() {
        let mut m = AccessoryManager::new();
        let a = m.attach(accessory(1, 0x1234));
        let _other = m.attach(accessory(2, 0x9999));
        // the listener registered late sees the one connected match in the completion list
        let current = m.register(10, by_vendor(0x1234)).unwrap();
        assert_eq!(current, vec![a]);
        // a new matching attach is queued; a non-matching one is not
        let b = m.attach(accessory(3, 0x1234));
        m.attach(accessory(4, 0x5555));
        assert_eq!(m.take_events(10), vec![Event::Connected(b)]);
        assert!(m.take_events(10).is_empty(), "take_events drains the queue");
    }

    #[test]
    fn detach_reaches_only_listeners_that_were_told_about_the_attach() {
        let mut m = AccessoryManager::new();
        m.register(1, by_vendor(0x1234)).unwrap();
        m.register(2, by_vendor(0x9999)).unwrap();
        let id = m.attach(accessory(5, 0x1234));
        assert_eq!(m.detach(5), Some(id));
        assert_eq!(m.take_events(1), vec![Event::Connected(id), Event::Disconnected(id)]);
        assert!(m.take_events(2).is_empty());
    }

    #[test]
    fn criteria_checks_the_configuration_interfaces_when_they_are_named() {
        let mut m = AccessoryManager::new();
        m.register(
            1,
            MatchingCriteria {
                device: DeviceMatch::default(),
                interfaces: vec![InterfaceMatch { class: Some(0x03), subclass: None, protocol: None }],
                option: None,
            },
        )
        .unwrap();
        let mut with_hid = accessory(9, 0x1234);
        with_hid.configuration = Some(config_with_class(0x03));
        let mut with_storage = accessory(10, 0x1234);
        with_storage.configuration = Some(config_with_class(0x08));
        // a device with no configuration read yet does not match an interface criterion
        let without_config = accessory(11, 0x1234);
        let hid = m.attach(with_hid);
        m.attach(with_storage);
        m.attach(without_config);
        assert_eq!(m.take_events(1), vec![Event::Connected(hid)]);
    }

    #[test]
    fn open_is_exclusive_and_close_needs_the_owner() {
        let mut m = AccessoryManager::new();
        m.register(1, by_vendor(0x1234)).unwrap();
        let id = m.attach(accessory(20, 0x1234));
        assert_eq!(m.open(id, 100), Ok(()));
        assert_eq!(m.open(id, 200), Err(Error::NotAccessible), "another owner is refused");
        assert_eq!(m.open(id, 100), Err(Error::InvalidState), "the owner cannot open twice");
        assert_eq!(m.close(id, 200), Err(Error::InvalidState), "only the owner closes");
        assert_eq!(m.close(id, 100), Ok(()));
        assert_eq!(m.close(id, 100), Err(Error::InvalidState), "nothing is open now");
        assert_eq!(m.open(id, 200), Ok(()));
    }

    #[test]
    fn a_detach_drops_the_open_and_a_disconnected_accessory_cannot_be_opened() {
        let mut m = AccessoryManager::new();
        let id = m.attach(accessory(30, 0x1234));
        m.open(id, 1).unwrap();
        m.detach(30);
        assert_eq!(m.accessory(id).and_then(|a| a.opened_by()), None);
        assert_eq!(m.open(id, 1), Err(Error::NotAccessible));
    }

    #[test]
    fn register_twice_and_unregister_an_unknown_key_are_errors() {
        let mut m = AccessoryManager::new();
        m.register(1, MatchingCriteria::default()).unwrap();
        assert_eq!(m.register(1, MatchingCriteria::default()), Err(Error::ListenerAlreadyRegistered));
        assert_eq!(m.unregister(2), Err(Error::Internal));
        m.unregister(1).unwrap();
        // a removed listener gets nothing queued from later attaches
        m.attach(accessory(40, 0x1234));
        assert!(m.take_events(1).is_empty());
    }

    #[test]
    fn a_session_needs_a_connected_accessory_and_a_declared_protocol() {
        let mut m = AccessoryManager::new();
        let mut acc = accessory(50, 0x1234);
        acc.protocol_strings = vec!["com.example.pad".to_string()];
        let id = m.attach(acc);
        let session = m.session(id, "com.example.pad").unwrap();
        assert_eq!(session, Session { accessory: id, protocol: "com.example.pad".to_string() });
        assert_eq!(m.session(id, "com.example.other"), Err(Error::InvalidState));
        m.detach(50);
        assert_eq!(m.session(id, "com.example.pad"), Err(Error::NotAccessible));
        assert_eq!(m.session(99, "com.example.pad"), Err(Error::NotAccessible));
    }

    #[test]
    fn every_error_has_a_readable_message() {
        let all = [
            Error::ListenerAlreadyRegistered,
            Error::NotAccessible,
            Error::InvalidState,
            Error::Internal,
        ];
        for error in all {
            assert!(!error.to_string().is_empty());
        }
        assert_eq!(Error::NotAccessible.to_string(), "The accessory is disconnected or open for another owner");
    }
}
