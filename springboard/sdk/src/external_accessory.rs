//! Accessory connections for apps: the registry of connected hardware accessories, connect and disconnect
//! notifications, and data sessions over the protocols an accessory declares.
//!
//! This is a pure model with no device access. The system layer calls `AccessoryManager::attach` and `detach` when a
//! device appears or goes away, and passes received bytes to `Session::receive`. Bytes an app writes wait in the
//! session's output stream, and the transport takes them with `Session::take_output`. Notifications are queued only
//! while the app is registered, as delivery is in the system's accessory manager.
//!
//! On the desktop the bus is the shell's `usb` service (`crate::usb_access`): [`AccessoryManager::apply_usb_event`] and
//! [`AccessoryManager::sync_usb_devices`] turn its events and device lists into attach and detach, and [`UsbTransport`]
//! moves a session's bytes over the bulk or interrupt endpoints of the opened device. The person grants each device
//! first (the shell's sheet), so an app only ever sees accessories it was allowed to use. A USB accessory has no
//! declared protocol strings on the wire; [`usb_protocol`] names the one protocol each model gets, so a session can be
//! opened with the same call as for any accessory.

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

    /// The app view of an accessory that the USB accessory manager holds (springboard/core/src/accessory_manager.rs).
    /// The connection id is the manager's id, the identity strings and protocol strings are copied, and the connected
    /// state follows the manager. Revisions are not part of the app view.
    pub fn from_core(core: &bifrost_core::accessory_manager::Accessory) -> Self {
        let protocols: Vec<&str> = core.protocol_strings.iter().map(String::as_str).collect();
        let mut accessory = Accessory::new(
            u64::from(core.id),
            &core.name,
            &core.manufacturer,
            &core.model_number,
            &core.serial_number,
            &protocols,
        );
        accessory.connected = core.is_connected();
        accessory
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

/// The protocol string of a USB accessory model, `usb.vvvv.pppp` in lower-case hex.
pub fn usb_protocol(vendor_id: u16, product_id: u16) -> String {
    format!("usb.{vendor_id:04x}.{product_id:04x}")
}

/// The connection id of a granted USB device: a 64-bit FNV-1a hash of the device id, stable while the device keeps its
/// serial number, so a re-plug of the same accessory is the same connection id.
pub fn usb_connection_id(device_id: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in device_id.bytes() {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

/// The app view of a granted USB device. The serial number is the last part of the device id (`-` means none).
pub fn accessory_from_usb(device: &crate::usb_access::Device) -> Accessory {
    let serial = device.id.rsplit(':').next().filter(|s| *s != "-").unwrap_or("");
    Accessory::new(usb_connection_id(&device.id), &device.name, "", &device.name, serial, &[usb_protocol(device.vendor_id, device.product_id).as_str()])
}

impl AccessoryManager {
    /// Applies one event of the `usb` service: a connect (or a granted answer that names a connected device) attaches
    /// the accessory, a disconnect detaches it. Returns whether the registry changed.
    pub fn apply_usb_event(&mut self, event: &crate::usb_access::Event) -> bool {
        use crate::usb_access::Event;
        match event {
            Event::Connected(device) | Event::Access { granted: true, device: Some(device), .. } => self.attach(accessory_from_usb(device)),
            Event::Disconnected { id } => self.detach(usb_connection_id(id)),
            _ => false,
        }
    }

    /// Brings the registry in line with the granted devices the service listed (a `devices` reply): attaches the ones
    /// missing and detaches the ones that are gone, queueing the notifications of a registered app.
    pub fn sync_usb_devices(&mut self, devices: &[crate::usb_access::Device]) {
        let wanted: Vec<u64> = devices.iter().map(|d| usb_connection_id(&d.id)).collect();
        let gone: Vec<u64> = self.accessories.iter().map(|a| a.connection_id).filter(|id| !wanted.contains(id)).collect();
        for id in gone {
            self.detach(id);
        }
        for device in devices {
            self.attach(accessory_from_usb(device));
        }
    }
}

/// What a [`UsbTransport`] asks the app to send to the `usb` service next.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TransportAction {
    /// `usb_access::read(ctx, handle, endpoint, length, timeout)`.
    Read { endpoint: u8, length: usize },
    /// `usb_access::write(ctx, handle, endpoint, &data, timeout)`.
    Write { endpoint: u8, data: Vec<u8> },
}

/// Moves a session's bytes over one opened, claimed USB device: a bulk or interrupt IN endpoint feeds the session's
/// input stream and a bulk or interrupt OUT endpoint carries its output stream. One read and one write are in flight at
/// a time. A read that times out is normal (the accessory had nothing to say) and is issued again; any other failure
/// stops the transport and is reported by [`UsbTransport::error`].
///
/// The app calls [`UsbTransport::poll`] after each tick and sends what it returns with `usb_access`, and hands every
/// `usb_access::Event::Done` of the handle to [`UsbTransport::complete`].
#[derive(Debug)]
pub struct UsbTransport {
    in_endpoint: u8,
    out_endpoint: u8,
    chunk: usize,
    reading: bool,
    writing: bool,
    pending: Vec<u8>,
    failed: Option<String>,
}

impl UsbTransport {
    /// The most bytes one request moves by default.
    pub const CHUNK: usize = 512;

    /// A transport over the endpoints `in_endpoint` (with the direction bit, `0x81`) and `out_endpoint` (`0x02`).
    pub fn new(in_endpoint: u8, out_endpoint: u8) -> Self {
        UsbTransport { in_endpoint, out_endpoint, chunk: Self::CHUNK, reading: false, writing: false, pending: Vec::new(), failed: None }
    }

    /// Changes the size of one request, from 1 to `usb_access::MAX_BYTES`.
    pub fn with_chunk(mut self, chunk: usize) -> Self {
        self.chunk = chunk.clamp(1, crate::usb_access::MAX_BYTES);
        self
    }

    /// The requests to send now: a read when none is in flight, and the next chunk of the session's output when no
    /// write is in flight. Nothing once the transport failed.
    pub fn poll(&mut self, session: &mut Session) -> Vec<TransportAction> {
        let mut actions = Vec::new();
        if self.failed.is_some() {
            return actions;
        }
        self.pending.extend(session.take_output());
        if !self.reading {
            self.reading = true;
            actions.push(TransportAction::Read { endpoint: self.in_endpoint, length: self.chunk });
        }
        if !self.writing && !self.pending.is_empty() {
            let n = self.pending.len().min(self.chunk);
            let data: Vec<u8> = self.pending.drain(..n).collect();
            self.writing = true;
            actions.push(TransportAction::Write { endpoint: self.out_endpoint, data });
        }
        actions
    }

    /// A finished request of the handle. Data from a read goes to the session's input stream.
    pub fn complete(&mut self, done: &crate::usb_access::Done, session: &mut Session) {
        match done.op.as_str() {
            "read" => {
                self.reading = false;
                match (&done.error, done.failure.as_deref()) {
                    (None, _) => {
                        if let Some(bytes) = &done.data {
                            if !bytes.is_empty() {
                                session.receive(bytes);
                            }
                        }
                    }
                    (Some(_), Some("timeout")) => {}
                    (Some(e), _) => self.failed = Some(e.clone()),
                }
            }
            "write" => {
                self.writing = false;
                if let Some(e) = &done.error {
                    self.failed = Some(e.clone());
                }
            }
            _ => {}
        }
    }

    /// Why the transport stopped, if it did.
    pub fn error(&self) -> Option<&str> {
        self.failed.as_deref()
    }

    /// Bytes of the session's output taken but not yet sent.
    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_view_copies_identity_and_connected_state_from_the_usb_manager() {
        let mut core = bifrost_core::accessory_manager::Accessory::new(
            5,
            bifrost_core::usb_descriptors::DeviceDescriptor {
                usb_version: 0x0200,
                class: 0,
                subclass: 0,
                protocol: 0,
                max_packet_size0: 64,
                vendor_id: 0x1234,
                product_id: 0x0007,
                device_version: 0x0100,
                configurations: 1,
            },
        );
        core.name = "Meter".to_string();
        core.manufacturer = "Acme".to_string();
        core.model_number = "M-100".to_string();
        core.serial_number = "SN-1".to_string();
        core.protocol_strings = vec!["com.acme.meter".to_string()];
        let mut manager = bifrost_core::accessory_manager::AccessoryManager::new();
        let id = manager.attach(core);

        let view = Accessory::from_core(manager.accessory(id).unwrap());
        assert_eq!(view.connection_id(), u64::from(id));
        assert_eq!(view.name, "Meter");
        assert_eq!(view.manufacturer, "Acme");
        assert!(view.supports_protocol("com.acme.meter"));
        assert!(view.is_connected());

        manager.detach(5);
        let gone = Accessory::from_core(manager.accessory(id).unwrap());
        assert!(!gone.is_connected());
    }
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
    fn usb_device(id: &str, name: &str) -> crate::usb_access::Device {
        crate::usb_access::Device {
            id: id.to_string(),
            name: name.to_string(),
            vendor_id: 0x0403,
            product_id: 0x6001,
            kind: crate::usb_access::DeviceKind::Usb,
            device_class: 0,
            interface_classes: vec![0xff],
            usages: Vec::new(),
        }
    }

    fn done(op: &str, data: Option<Vec<u8>>, error: Option<(&str, &str)>) -> crate::usb_access::Done {
        crate::usb_access::Done {
            handle: 1,
            token: 1,
            op: op.to_string(),
            data,
            count: None,
            error: error.map(|(e, _)| e.to_string()),
            failure: error.map(|(_, f)| f.to_string()),
        }
    }

    #[test]
    fn a_granted_usb_device_becomes_an_accessory_with_one_protocol_and_a_stable_id() {
        let a = accessory_from_usb(&usb_device("0403:6001:A1", "Logic Probe"));
        assert_eq!(a.name, "Logic Probe");
        assert_eq!(a.serial_number, "A1");
        assert!(a.supports_protocol("usb.0403.6001"));
        assert_eq!(a.connection_id(), usb_connection_id("0403:6001:A1"));
        assert_ne!(usb_connection_id("0403:6001:A1"), usb_connection_id("0403:6001:B2"));
        assert_eq!(accessory_from_usb(&usb_device("0403:6001:-", "x")).serial_number, "", "no serial");
    }

    #[test]
    fn usb_events_attach_and_detach_and_notify_a_registered_app() {
        use crate::usb_access::Event;
        let mut m = AccessoryManager::new();
        m.register_for_notifications();
        let dev = usb_device("0403:6001:A1", "Logic Probe");
        assert!(m.apply_usb_event(&Event::Connected(dev.clone())));
        assert!(!m.apply_usb_event(&Event::Connected(dev.clone())), "the same device attaches once");
        assert!(!m.apply_usb_event(&Event::Access { id: dev.id.clone(), granted: false, device: None }));
        assert_eq!(m.connected_accessories().len(), 1);
        assert!(m.session(usb_connection_id(&dev.id), "usb.0403.6001").is_ok());
        assert!(m.apply_usb_event(&Event::Disconnected { id: dev.id.clone() }));
        assert!(!m.apply_usb_event(&Event::Disconnected { id: dev.id.clone() }));
        let events = m.take_events();
        assert!(matches!(events[0], AccessoryEvent::Connected(_)) && matches!(events[1], AccessoryEvent::Disconnected(_)));
        assert!(!m.apply_usb_event(&Event::Closed { handle: 1, reason: String::new() }));
    }

    #[test]
    fn a_granted_answer_that_names_the_device_attaches_it() {
        use crate::usb_access::Event;
        let mut m = AccessoryManager::new();
        let dev = usb_device("0403:6001:A1", "Logic Probe");
        assert!(m.apply_usb_event(&Event::Access { id: dev.id.clone(), granted: true, device: Some(dev) }));
        assert_eq!(m.connected_accessories().len(), 1);
    }

    #[test]
    fn a_device_list_syncs_the_registry() {
        let mut m = AccessoryManager::new();
        m.register_for_notifications();
        let a = usb_device("0403:6001:A1", "A");
        let b = usb_device("0403:6001:B2", "B");
        m.sync_usb_devices(&[a.clone(), b.clone()]);
        assert_eq!(m.connected_accessories().len(), 2);
        m.sync_usb_devices(&[b.clone()]);
        let names: Vec<&str> = m.connected_accessories().iter().map(|x| x.name.as_str()).collect();
        assert_eq!(names, ["B"]);
        m.sync_usb_devices(&[]);
        assert!(m.connected_accessories().is_empty());
        assert_eq!(m.take_events().len(), 4, "two connects and two disconnects");
    }

    #[test]
    fn the_transport_reads_continuously_and_sends_output_in_chunks() {
        let mut m = AccessoryManager::new();
        m.attach(meter());
        let mut s = m.session(7, "com.acme.meter").unwrap();
        s.output_stream().open();
        s.input_stream().open();
        let mut t = UsbTransport::new(0x81, 0x02).with_chunk(4);
        assert_eq!(t.poll(&mut s), vec![TransportAction::Read { endpoint: 0x81, length: 4 }]);
        assert!(t.poll(&mut s).is_empty(), "one read in flight");
        s.output_stream().write(b"abcdefghij").unwrap();
        assert_eq!(t.poll(&mut s), vec![TransportAction::Write { endpoint: 0x02, data: b"abcd".to_vec() }]);
        assert_eq!(t.pending_len(), 6);
        assert!(t.poll(&mut s).is_empty(), "one write in flight");
        t.complete(&done("write", None, None), &mut s);
        assert_eq!(t.poll(&mut s), vec![TransportAction::Write { endpoint: 0x02, data: b"efgh".to_vec() }]);
        t.complete(&done("write", None, None), &mut s);
        assert_eq!(t.poll(&mut s), vec![TransportAction::Write { endpoint: 0x02, data: b"ij".to_vec() }]);
        // data from the accessory reaches the input stream, and the next read is issued
        t.complete(&done("read", Some(b"hi".to_vec()), None), &mut s);
        let mut buf = [0u8; 8];
        assert_eq!(s.input_stream().read(&mut buf), Ok(2));
        assert_eq!(&buf[..2], b"hi");
        assert_eq!(t.poll(&mut s), vec![TransportAction::Read { endpoint: 0x81, length: 4 }]);
        assert!(t.error().is_none());
    }

    #[test]
    fn a_read_timeout_is_normal_and_any_other_failure_stops_the_transport() {
        let mut m = AccessoryManager::new();
        m.attach(meter());
        let mut s = m.session(7, "com.acme.meter").unwrap();
        let mut t = UsbTransport::new(0x81, 0x02);
        t.poll(&mut s);
        t.complete(&done("read", None, Some(("timed out", "timeout"))), &mut s);
        assert!(t.error().is_none());
        assert_eq!(t.poll(&mut s).len(), 1, "the read is issued again");
        t.complete(&done("read", None, Some(("device gone", "disconnected"))), &mut s);
        assert_eq!(t.error(), Some("device gone"));
        assert!(t.poll(&mut s).is_empty(), "a failed transport sends nothing more");
    }

    #[test]
    fn a_failed_write_stops_the_transport_and_unrelated_results_are_ignored() {
        let mut m = AccessoryManager::new();
        m.attach(meter());
        let mut s = m.session(7, "com.acme.meter").unwrap();
        s.output_stream().open();
        let mut t = UsbTransport::new(0x81, 0x02);
        s.output_stream().write(b"x").unwrap();
        assert_eq!(t.poll(&mut s).len(), 2);
        t.complete(&done("claim", None, None), &mut s);
        assert!(t.error().is_none());
        t.complete(&done("write", None, Some(("endpoint stalled", "stall"))), &mut s);
        assert_eq!(t.error(), Some("endpoint stalled"));
    }

    #[test]
    fn the_chunk_is_kept_inside_what_the_service_moves() {
        let big = UsbTransport::new(0x81, 0x02).with_chunk(1 << 20);
        assert_eq!(big.chunk, crate::usb_access::MAX_BYTES);
        assert_eq!(UsbTransport::new(0x81, 0x02).with_chunk(0).chunk, 1);
        assert_eq!(usb_protocol(0x1234, 0xabc), "usb.1234.0abc");
    }
}
