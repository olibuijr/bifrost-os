//! Accessory connections for apps: the registry of connected hardware accessories, connect and disconnect
//! notifications, and data sessions over the protocols an accessory declares.
//!
//! This is a pure model with no device access. The system layer calls `AccessoryManager::attach` and `detach` when a
//! device appears or goes away, and passes received bytes to `Session::receive`. Bytes an app writes wait in the
//! session's output stream, and the transport takes them with `Session::take_output`. Notifications are queued only
//! while the app is registered, as delivery is in the system's accessory manager.

use crate::stream::{InputStream, OutputStream, StreamStatus};

/// Why a session could not be opened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessoryError {
    /// No connected accessory has the given connection id.
    NotConnected,
    /// The accessory does not declare the requested protocol string.
    ProtocolNotSupported,
}

impl std::fmt::Display for AccessoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AccessoryError::NotConnected => write!(f, "The accessory is not connected"),
            AccessoryError::ProtocolNotSupported => write!(f, "The accessory does not support this protocol"),
        }
    }
}

/// A connected hardware accessory: its identity and the protocol strings it declares.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Accessory {
    pub name: String,
    pub manufacturer: String,
    pub model_number: String,
    pub serial_number: String,
    pub protocol_strings: Vec<String>,
    connection_id: u64,
    connected: bool,
}

impl Accessory {
    /// An accessory that is not connected yet. The manager sets the connected state on `attach`.
    pub fn new(
        connection_id: u64,
        name: &str,
        manufacturer: &str,
        model_number: &str,
        serial_number: &str,
        protocol_strings: &[&str],
    ) -> Self {
        Accessory {
            name: name.to_string(),
            manufacturer: manufacturer.to_string(),
            model_number: model_number.to_string(),
            serial_number: serial_number.to_string(),
            protocol_strings: protocol_strings.iter().map(|p| p.to_string()).collect(),
            connection_id,
            connected: false,
        }
    }

    /// The id the system gave this connection. It is unique among the connected accessories.
    pub fn connection_id(&self) -> u64 {
        self.connection_id
    }

    /// `isConnected`: true while the manager holds this accessory as attached.
    pub fn is_connected(&self) -> bool {
        self.connected
    }

    /// True when the accessory declares `protocol` (exact string match).
    pub fn supports_protocol(&self, protocol: &str) -> bool {
        self.protocol_strings.iter().any(|p| p == protocol)
    }
}

/// A connect or disconnect notification. The accessory is copied as it was at the change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AccessoryEvent {
    Connected(Accessory),
    Disconnected(Accessory),
}

/// The registry of connected accessories and the app's notification queue.
#[derive(Debug, Default)]
pub struct AccessoryManager {
    accessories: Vec<Accessory>,
    registered: bool,
    events: Vec<AccessoryEvent>,
}

impl AccessoryManager {
    /// An empty manager with no accessories and no registration.
    pub fn new() -> Self {
        Self::default()
    }

    /// `registerForLocalNotifications`: from now on, attach and detach changes are queued as events.
    pub fn register_for_notifications(&mut self) {
        self.registered = true;
    }

    /// `unregisterForLocalNotifications`: stops queueing and drops events not yet taken.
    pub fn unregister_for_notifications(&mut self) {
        self.registered = false;
        self.events.clear();
    }

    /// True between `register_for_notifications` and `unregister_for_notifications`.
    pub fn is_registered(&self) -> bool {
        self.registered
    }

    /// Called by the system when an accessory connects. Returns false, and changes nothing, when an accessory with
    /// the same connection id is already attached.
    pub fn attach(&mut self, mut accessory: Accessory) -> bool {
        if self.accessories.iter().any(|a| a.connection_id == accessory.connection_id) {
            return false;
        }
        accessory.connected = true;
        self.accessories.push(accessory.clone());
        if self.registered {
            self.events.push(AccessoryEvent::Connected(accessory));
        }
        true
    }

    /// Called by the system when an accessory disconnects. Returns false when no attached accessory has the id.
    pub fn detach(&mut self, connection_id: u64) -> bool {
        let index = match self.accessories.iter().position(|a| a.connection_id == connection_id) {
            Some(i) => i,
            None => return false,
        };
        let mut accessory = self.accessories.remove(index);
        accessory.connected = false;
        if self.registered {
            self.events.push(AccessoryEvent::Disconnected(accessory));
        }
        true
    }

    /// `connectedAccessories`: every attached accessory, in the order they attached.
    pub fn connected_accessories(&self) -> &[Accessory] {
        &self.accessories
    }

    /// The attached accessories that declare `protocol`, in attach order.
    pub fn accessories_for_protocol(&self, protocol: &str) -> Vec<&Accessory> {
        self.accessories.iter().filter(|a| a.supports_protocol(protocol)).collect()
    }

    /// Returns the queued notifications in order and empties the queue.
    pub fn take_events(&mut self) -> Vec<AccessoryEvent> {
        std::mem::take(&mut self.events)
    }

    /// `init(accessory:forProtocol:)`: a closed session to an attached accessory over a protocol it declares.
    pub fn session(&self, connection_id: u64, protocol: &str) -> Result<Session, AccessoryError> {
        let accessory = match self.accessories.iter().find(|a| a.connection_id == connection_id) {
            Some(a) => a,
            None => return Err(AccessoryError::NotConnected),
        };
        if !accessory.supports_protocol(protocol) {
            return Err(AccessoryError::ProtocolNotSupported);
        }
        Ok(Session {
            accessory: accessory.clone(),
            protocol: protocol.to_string(),
            input: InputStream::with_data(Vec::new()),
            output: OutputStream::to_memory(),
        })
    }
}

/// A data session with one accessory over one protocol. The streams start closed and the app opens them.
#[derive(Debug)]
pub struct Session {
    accessory: Accessory,
    protocol: String,
    input: InputStream,
    output: OutputStream,
}

impl Session {
    /// `accessory`: the accessory this session talks to.
    pub fn accessory(&self) -> &Accessory {
        &self.accessory
    }

    /// `protocolString`: the protocol this session uses.
    pub fn protocol_string(&self) -> &str {
        &self.protocol
    }

    /// `inputStream`: bytes the accessory sent.
    pub fn input_stream(&mut self) -> &mut InputStream {
        &mut self.input
    }

    /// `outputStream`: bytes the app sends to the accessory.
    pub fn output_stream(&mut self) -> &mut OutputStream {
        &mut self.output
    }

    /// Called by the transport with bytes that arrived from the accessory.
    pub fn receive(&mut self, bytes: &[u8]) {
        self.input.append(bytes);
    }

    /// Called by the transport: removes the bytes the app wrote, in order, for sending to the accessory.
    pub fn take_output(&mut self) -> Vec<u8> {
        self.output.take_written()
    }

    /// Closes both streams. A closed session refuses reads and writes.
    pub fn close(&mut self) {
        self.input.close();
        self.output.close();
    }

    /// The status of the output stream, for callers that check before writing.
    pub fn output_status(&self) -> StreamStatus {
        self.output.status()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream::StreamError;

    fn meter() -> Accessory {
        Accessory::new(7, "Meter", "Acme", "M-100", "SN-1", &["com.acme.meter", "com.acme.log"])
    }

    fn scale() -> Accessory {
        Accessory::new(9, "Scale", "Acme", "S-2", "SN-2", &["com.acme.log"])
    }

    #[test]
    fn attach_and_detach_queue_events_in_order_while_registered() {
        let mut m = AccessoryManager::new();
        m.register_for_notifications();
        assert!(m.attach(meter()));
        assert!(m.detach(7));
        let events = m.take_events();
        assert_eq!(events.len(), 2);
        match &events[0] {
            AccessoryEvent::Connected(a) => {
                assert_eq!(a.connection_id(), 7);
                assert!(a.is_connected());
            }
            other => panic!("expected Connected, got {:?}", other),
        }
        match &events[1] {
            AccessoryEvent::Disconnected(a) => assert!(!a.is_connected()),
            other => panic!("expected Disconnected, got {:?}", other),
        }
        assert!(m.take_events().is_empty());
        assert!(m.connected_accessories().is_empty());
    }

    #[test]
    fn changes_before_registration_are_not_queued() {
        let mut m = AccessoryManager::new();
        assert!(m.attach(meter()));
        assert!(m.take_events().is_empty());
        assert_eq!(m.connected_accessories().len(), 1);
        m.register_for_notifications();
        assert!(m.detach(7));
        assert_eq!(m.take_events().len(), 1);
        m.register_for_notifications();
        assert!(m.attach(scale()));
        m.unregister_for_notifications();
        assert!(m.take_events().is_empty());
        assert!(!m.is_registered());
    }

    #[test]
    fn duplicate_attach_and_unknown_detach_change_nothing() {
        let mut m = AccessoryManager::new();
        m.register_for_notifications();
        assert!(m.attach(meter()));
        assert!(!m.attach(meter()));
        assert_eq!(m.connected_accessories().len(), 1);
        assert_eq!(m.take_events().len(), 1);
        assert!(!m.detach(99));
        assert!(m.take_events().is_empty());
    }

    #[test]
    fn protocol_match_lists_only_accessories_that_declare_it() {
        let mut m = AccessoryManager::new();
        m.attach(meter());
        m.attach(scale());
        let log: Vec<u64> = m.accessories_for_protocol("com.acme.log").iter().map(|a| a.connection_id()).collect();
        assert_eq!(log, vec![7, 9]);
        let meter_only: Vec<String> =
            m.accessories_for_protocol("com.acme.meter").iter().map(|a| a.serial_number.clone()).collect();
        assert_eq!(meter_only, vec!["SN-1".to_string()]);
        assert!(m.accessories_for_protocol("com.acme.other").is_empty());
    }

    #[test]
    fn session_needs_a_connected_accessory_with_the_protocol() {
        let mut m = AccessoryManager::new();
        m.attach(scale());
        assert_eq!(m.session(7, "com.acme.log").err(), Some(AccessoryError::NotConnected));
        assert_eq!(m.session(9, "com.acme.meter").err(), Some(AccessoryError::ProtocolNotSupported));
        let s = m.session(9, "com.acme.log").expect("supported protocol opens");
        assert_eq!(s.protocol_string(), "com.acme.log");
        assert_eq!(s.accessory().name, "Scale");
        m.detach(9);
        assert_eq!(m.session(9, "com.acme.log").err(), Some(AccessoryError::NotConnected));
    }

    #[test]
    fn session_streams_carry_bytes_both_ways() {
        let mut m = AccessoryManager::new();
        m.attach(meter());
        let mut s = m.session(7, "com.acme.log").unwrap();
        assert_eq!(s.output_status(), StreamStatus::NotOpen);
        s.input_stream().open();
        s.output_stream().open();

        s.receive(b"ab");
        let mut buf = [0u8; 4];
        assert_eq!(s.input_stream().read(&mut buf), Ok(2));
        assert_eq!(&buf[..2], b"ab");
        assert_eq!(s.input_stream().status(), StreamStatus::AtEnd);

        // Bytes arriving after the end are read on the next call.
        s.receive(b"cde");
        assert_eq!(s.input_stream().read(&mut buf), Ok(3));
        assert_eq!(&buf[..3], b"cde");

        assert_eq!(s.output_stream().write(b"ping"), Ok(4));
        assert_eq!(s.output_stream().write(b"!"), Ok(1));
        assert_eq!(s.take_output(), b"ping!".to_vec());
        assert!(s.take_output().is_empty());
    }

    #[test]
    fn closed_session_refuses_writes() {
        let mut m = AccessoryManager::new();
        m.attach(meter());
        let mut s = m.session(7, "com.acme.meter").unwrap();
        s.output_stream().open();
        s.close();
        assert_eq!(s.output_stream().write(b"x"), Err(StreamError::Closed));
        assert_eq!(s.input_stream().status(), StreamStatus::Closed);
    }

    #[test]
    fn errors_have_readable_messages() {
        assert_eq!(AccessoryError::NotConnected.to_string(), "The accessory is not connected");
        assert_eq!(
            AccessoryError::ProtocolNotSupported.to_string(),
            "The accessory does not support this protocol"
        );
    }
}
