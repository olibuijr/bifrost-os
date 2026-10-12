//! Peripheral role: the local GATT database a device publishes, its advertising payload, the centrals connected to
//! it, the notification subscriptions those centrals hold, and the Attribute Protocol requests they send.
//! Pure model, like `gatt`: there is no radio here. The transport reports the power state and the connections,
//! sends the `Update` values that `update_value` returns, and passes each incoming request to `on_read` or
//! `on_write`. A characteristic with a stored value answers at once. One without a value queues a `ServerRequest`
//! that the app answers with `respond`, and the transport sends the `Reply` values that `take_replies` returns.
use std::fmt;

use crate::gatt::{AttrPermissions, CharProperties, Descriptor, Uuid, WriteType, DEFAULT_MTU};

/// Largest value a single attribute write may carry (the protocol cap that `gatt` also uses).
pub const MAX_ATTRIBUTE_LEN: usize = 512;
/// Largest ATT MTU the model accepts.
const MAX_MTU: u16 = 517;
/// Legacy advertising carries 31 bytes of payload.
pub const LEGACY_ADVERTISING_LEN: usize = 31;
/// The flags structure (length, type, one flags byte) that a connectable device sends with legacy data.
const FLAGS_LEN: usize = 3;

/// Power and authorization state of the peripheral manager, as the transport reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ManagerState {
    Unknown,
    Resetting,
    Unsupported,
    Unauthorized,
    PoweredOff,
    PoweredOn,
}

/// Attribute Protocol error codes a request can finish with. The values are the ATT error codes of the Bluetooth
/// core specification.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttError {
    InvalidOffset,
    ReadNotPermitted,
    WriteNotPermitted,
    RequestNotSupported,
    InsufficientEncryption,
    AttributeNotFound,
    InvalidAttributeValueLength,
    UnlikelyError,
}

impl AttError {
    pub fn code(self) -> u8 {
        match self {
            AttError::ReadNotPermitted => 0x02,
            AttError::WriteNotPermitted => 0x03,
            AttError::RequestNotSupported => 0x06,
            AttError::InvalidOffset => 0x07,
            AttError::AttributeNotFound => 0x0A,
            AttError::UnlikelyError => 0x0E,
            AttError::InvalidAttributeValueLength => 0x0D,
            AttError::InsufficientEncryption => 0x0F,
        }
    }
}

impl fmt::Display for AttError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            AttError::InvalidOffset => "invalid offset",
            AttError::ReadNotPermitted => "read not permitted",
            AttError::WriteNotPermitted => "write not permitted",
            AttError::RequestNotSupported => "request not supported",
            AttError::InsufficientEncryption => "link must be encrypted",
            AttError::AttributeNotFound => "attribute not found",
            AttError::InvalidAttributeValueLength => "invalid attribute value length",
            AttError::UnlikelyError => "unlikely error",
        };
        write!(f, "{text} (ATT 0x{:02X})", self.code())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PeripheralError {
    /// Services, advertising and value updates need the powered-on state.
    NotPoweredOn(ManagerState),
    DuplicateService(Uuid),
    DuplicateCharacteristic(Uuid),
    ServiceNotFound,
    CharacteristicNotFound,
    PropertyNotSupported,
    ValueTooLong { max: usize },
    /// The data, with its flags structure, needs `size` bytes of the legacy payload.
    AdvertisingTooLarge { size: usize },
    /// The transmit queue is full. The app sends again after `set_ready(true)`.
    TransmitQueueFull,
    UnknownCentral(CentralId),
    NoPendingRequest(RequestId),
}

impl fmt::Display for PeripheralError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PeripheralError::NotPoweredOn(state) => write!(f, "peripheral manager is {state:?}, not powered on"),
            PeripheralError::DuplicateService(u) => write!(f, "service {} is already published", u.uuid_string()),
            PeripheralError::DuplicateCharacteristic(u) => {
                write!(f, "characteristic {} appears twice in one service", u.uuid_string())
            }
            PeripheralError::ServiceNotFound => write!(f, "service not published"),
            PeripheralError::CharacteristicNotFound => write!(f, "characteristic not found"),
            PeripheralError::PropertyNotSupported => write!(f, "operation not supported by the characteristic"),
            PeripheralError::ValueTooLong { max } => write!(f, "value longer than {max} bytes"),
            PeripheralError::AdvertisingTooLarge { size } => {
                write!(f, "advertising data needs {} of {} bytes", size, LEGACY_ADVERTISING_LEN)
            }
            PeripheralError::TransmitQueueFull => write!(f, "transmit queue full, wait for the ready signal"),
            PeripheralError::UnknownCentral(c) => write!(f, "central {} is not connected", c.0),
            PeripheralError::NoPendingRequest(r) => write!(f, "no pending request {}", r.0),
        }
    }
}

/// A connected central, identified by the transport.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CentralId(pub u32);

/// A request from a central that the app has to answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RequestId(pub u32);

/// Advertised fields. The local name and the service UUIDs are the two fields the model budgets.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AdvertisingData {
    pub local_name: Option<String>,
    pub service_uuids: Vec<Uuid>,
}

impl AdvertisingData {
    /// Bytes the advertised fields take, without the flags structure. Each group is one structure with a length
    /// byte and a type byte. 16-bit, 32-bit and 128-bit service UUIDs each go in their own list.
    pub fn payload_len(&self) -> usize {
        let mut len = 0;
        if let Some(name) = self.local_name.as_deref().filter(|n| !n.is_empty()) {
            len += 2 + name.len();
        }
        let (mut short16, mut short32, mut long128) = (0usize, 0usize, 0usize);
        for uuid in &self.service_uuids {
            match uuid.short() {
                Some(v) if v <= 0xFFFF => short16 += 1,
                Some(_) => short32 += 1,
                None => long128 += 1,
            }
        }
        if short16 > 0 {
            len += 2 + 2 * short16;
        }
        if short32 > 0 {
            len += 2 + 4 * short32;
        }
        if long128 > 0 {
            len += 2 + 16 * long128;
        }
        len
    }

    /// Whether the data plus the flags structure fits the legacy advertising payload.
    pub fn fits_legacy(&self) -> bool {
        self.payload_len() + FLAGS_LEN <= LEGACY_ADVERTISING_LEN
    }
}

/// A characteristic of the local database.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MutableCharacteristic {
    pub uuid: Uuid,
    pub properties: CharProperties,
    pub permissions: AttrPermissions,
    /// A stored value answers reads and writes directly. `None` leaves each request to the app.
    pub value: Option<Vec<u8>>,
    pub descriptors: Vec<Descriptor>,
}

impl MutableCharacteristic {
    pub fn new(
        uuid: Uuid,
        properties: CharProperties,
        permissions: AttrPermissions,
        value: Option<Vec<u8>>,
    ) -> MutableCharacteristic {
        MutableCharacteristic { uuid, properties, permissions, value, descriptors: Vec::new() }
    }
}

/// A service of the local database.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MutableService {
    pub uuid: Uuid,
    pub primary: bool,
    pub characteristics: Vec<MutableCharacteristic>,
}

impl MutableService {
    pub fn new(uuid: Uuid, primary: bool, characteristics: Vec<MutableCharacteristic>) -> MutableService {
        MutableService { uuid, primary, characteristics }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RequestKind {
    Read { offset: usize },
    Write { offset: usize, value: Vec<u8>, kind: WriteType },
}

/// A request from a connected central that waits for the app. Carries the central, the attribute and the offset,
/// and for a write the bytes written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerRequest {
    pub id: RequestId,
    pub central: CentralId,
    pub service: Uuid,
    pub characteristic: Uuid,
    pub kind: RequestKind,
}

/// The result of a request: a stored value answers now, otherwise the app answers the queued request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Dispatch {
    Answered(Vec<u8>),
    Queued(RequestId),
}

/// The response the transport sends for a request the app answered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reply {
    pub central: CentralId,
    pub request: RequestId,
    pub result: Result<Vec<u8>, AttError>,
}

/// A value to send to one subscribed central.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Update {
    pub central: CentralId,
    pub service: Uuid,
    pub characteristic: Uuid,
    pub value: Vec<u8>,
    /// True for an indication, false for a notification.
    pub indication: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Central {
    id: CentralId,
    mtu: u16,
    encrypted: bool,
    subscriptions: Vec<(Uuid, Uuid)>,
}

/// Payload bytes a notification or indication carries at this MTU (the MTU minus the 3-byte ATT header).
fn notify_payload(mtu: u16) -> usize {
    mtu as usize - 3
}

/// The local GATT database, advertising state and connected centrals of a peripheral.
pub struct PeripheralManager {
    state: ManagerState,
    services: Vec<MutableService>,
    advertising: Option<AdvertisingData>,
    centrals: Vec<Central>,
    pending: Vec<ServerRequest>,
    replies: Vec<Reply>,
    ready: bool,
    next_request: u32,
}

impl PeripheralManager {
    pub fn new() -> PeripheralManager {
        PeripheralManager {
            state: ManagerState::Unknown,
            services: Vec::new(),
            advertising: None,
            centrals: Vec::new(),
            pending: Vec::new(),
            replies: Vec::new(),
            ready: true,
            next_request: 1,
        }
    }

    pub fn state(&self) -> ManagerState {
        self.state
    }

    /// Records a state change from the transport. Leaving the powered-on state stops advertising, drops every
    /// connection and forgets the queued requests. The published services stay in the database.
    pub fn set_state(&mut self, state: ManagerState) {
        self.state = state;
        if state != ManagerState::PoweredOn {
            self.advertising = None;
            self.centrals.clear();
            self.pending.clear();
        }
    }

    /// False while the transport's transmit queue is full. `update_value` then fails until the transport sets ready.
    pub fn set_ready(&mut self, ready: bool) {
        self.ready = ready;
    }

    fn require_powered_on(&self) -> Result<(), PeripheralError> {
        if self.state == ManagerState::PoweredOn {
            Ok(())
        } else {
            Err(PeripheralError::NotPoweredOn(self.state))
        }
    }

    pub fn services(&self) -> &[MutableService] {
        &self.services
    }

    /// Publishes a service. Service UUIDs are unique, and so are the characteristic UUIDs within one service.
    pub fn add_service(&mut self, service: MutableService) -> Result<(), PeripheralError> {
        self.require_powered_on()?;
        if self.services.iter().any(|s| s.uuid == service.uuid) {
            return Err(PeripheralError::DuplicateService(service.uuid));
        }
        for (i, c) in service.characteristics.iter().enumerate() {
            if service.characteristics[..i].iter().any(|o| o.uuid == c.uuid) {
                return Err(PeripheralError::DuplicateCharacteristic(c.uuid));
            }
        }
        self.services.push(service);
        Ok(())
    }

    /// Removes a published service, its queued requests and the subscriptions to its characteristics.
    pub fn remove_service(&mut self, uuid: Uuid) -> Result<(), PeripheralError> {
        let before = self.services.len();
        self.services.retain(|s| s.uuid != uuid);
        if self.services.len() == before {
            return Err(PeripheralError::ServiceNotFound);
        }
        self.pending.retain(|r| r.service != uuid);
        for c in &mut self.centrals {
            c.subscriptions.retain(|(s, _)| *s != uuid);
        }
        Ok(())
    }

    pub fn remove_all_services(&mut self) {
        self.services.clear();
        self.pending.clear();
        for c in &mut self.centrals {
            c.subscriptions.clear();
        }
    }

    /// Sets the stored value of a characteristic. From then on, requests for it are answered from the stored value.
    pub fn set_value(&mut self, service: Uuid, characteristic: Uuid, value: Vec<u8>) -> Result<(), PeripheralError> {
        let ch = self.characteristic_mut(service, characteristic)?;
        ch.value = Some(value);
        Ok(())
    }

    fn characteristic(&self, service: Uuid, characteristic: Uuid) -> Result<&MutableCharacteristic, PeripheralError> {
        let svc = self
            .services
            .iter()
            .find(|s| s.uuid == service)
            .ok_or(PeripheralError::ServiceNotFound)?;
        svc.characteristics
            .iter()
            .find(|c| c.uuid == characteristic)
            .ok_or(PeripheralError::CharacteristicNotFound)
    }

    fn characteristic_mut(
        &mut self,
        service: Uuid,
        characteristic: Uuid,
    ) -> Result<&mut MutableCharacteristic, PeripheralError> {
        let svc = self
            .services
            .iter_mut()
            .find(|s| s.uuid == service)
            .ok_or(PeripheralError::ServiceNotFound)?;
        svc.characteristics
            .iter_mut()
            .find(|c| c.uuid == characteristic)
            .ok_or(PeripheralError::CharacteristicNotFound)
    }

    pub fn is_advertising(&self) -> bool {
        self.advertising.is_some()
    }

    /// Starts advertising. The data has to fit the legacy payload together with its flags structure.
    pub fn start_advertising(&mut self, data: AdvertisingData) -> Result<(), PeripheralError> {
        self.require_powered_on()?;
        if !data.fits_legacy() {
            let size = data.payload_len() + FLAGS_LEN;
            return Err(PeripheralError::AdvertisingTooLarge { size });
        }
        self.advertising = Some(data);
        Ok(())
    }

    pub fn stop_advertising(&mut self) {
        self.advertising = None;
    }

    /// A central connects, or reconnects with a new encryption state.
    pub fn connect(&mut self, central: CentralId, encrypted: bool) -> Result<(), PeripheralError> {
        self.require_powered_on()?;
        if let Some(pos) = self.centrals.iter().position(|c| c.id == central) {
            self.centrals[pos].encrypted = encrypted;
        } else {
            self.centrals.push(Central { id: central, mtu: DEFAULT_MTU, encrypted, subscriptions: Vec::new() });
        }
        Ok(())
    }

    /// A central disconnects. Its subscriptions and its queued requests go with it.
    pub fn disconnect(&mut self, central: CentralId) {
        self.centrals.retain(|c| c.id != central);
        self.pending.retain(|r| r.central != central);
    }

    /// The ATT MTU agreed with a central, clamped to 23..=517.
    pub fn set_mtu(&mut self, central: CentralId, mtu: u16) -> Result<(), PeripheralError> {
        let c = self
            .centrals
            .iter_mut()
            .find(|c| c.id == central)
            .ok_or(PeripheralError::UnknownCentral(central))?;
        c.mtu = mtu.clamp(DEFAULT_MTU, MAX_MTU);
        Ok(())
    }

    fn link_encrypted(&self, central: CentralId) -> Result<bool, AttError> {
        self.centrals
            .iter()
            .find(|c| c.id == central)
            .map(|c| c.encrypted)
            .ok_or(AttError::UnlikelyError)
    }

    /// Centrals subscribed to a characteristic, in connection order.
    pub fn subscribed_centrals(&self, service: Uuid, characteristic: Uuid) -> Vec<CentralId> {
        let key = (service, characteristic);
        self.centrals
            .iter()
            .filter(|c| c.subscriptions.contains(&key))
            .map(|c| c.id)
            .collect()
    }

    /// A central writes the client configuration of a notifying characteristic. Enabling needs the notify or the
    /// indicate property, plus the encryption that property asks for. Notify wins when both properties are set.
    pub fn set_subscription(
        &mut self,
        central: CentralId,
        service: Uuid,
        characteristic: Uuid,
        enable: bool,
    ) -> Result<(), AttError> {
        let link_encrypted = self.link_encrypted(central)?;
        let p = self
            .characteristic(service, characteristic)
            .map_err(|_| AttError::AttributeNotFound)?
            .properties;
        if enable {
            if !(p.contains(CharProperties::NOTIFY) || p.contains(CharProperties::INDICATE)) {
                return Err(AttError::RequestNotSupported);
            }
            let encryption_required = if p.contains(CharProperties::NOTIFY) {
                p.contains(CharProperties::NOTIFY_ENCRYPTION_REQUIRED)
            } else {
                p.contains(CharProperties::INDICATE_ENCRYPTION_REQUIRED)
            };
            if encryption_required && !link_encrypted {
                return Err(AttError::InsufficientEncryption);
            }
        }
        let pos = self
            .centrals
            .iter()
            .position(|c| c.id == central)
            .ok_or(AttError::UnlikelyError)?;
        let subs = &mut self.centrals[pos].subscriptions;
        let key = (service, characteristic);
        subs.retain(|k| *k != key);
        if enable {
            subs.push(key);
        }
        Ok(())
    }

    fn queue(&mut self, central: CentralId, service: Uuid, characteristic: Uuid, kind: RequestKind) -> RequestId {
        let id = RequestId(self.next_request);
        self.next_request = self.next_request.wrapping_add(1);
        self.pending.push(ServerRequest { id, central, service, characteristic, kind });
        id
    }

    /// A central reads a characteristic from `offset`. Needs the read property and the read permission, and the
    /// encryption the permission asks for. A stored value answers now; without one, the read waits for the app.
    pub fn on_read(
        &mut self,
        central: CentralId,
        service: Uuid,
        characteristic: Uuid,
        offset: usize,
    ) -> Result<Dispatch, AttError> {
        let encrypted = self.link_encrypted(central)?;
        let ch = self.characteristic(service, characteristic).map_err(|_| AttError::AttributeNotFound)?;
        if !ch.properties.contains(CharProperties::READ) {
            return Err(AttError::RequestNotSupported);
        }
        if !ch.permissions.contains(AttrPermissions::READABLE) {
            return Err(AttError::ReadNotPermitted);
        }
        if !ch.permissions.allows_read(encrypted) {
            return Err(AttError::InsufficientEncryption);
        }
        if let Some(value) = ch.value.as_ref() {
            if offset > value.len() {
                return Err(AttError::InvalidOffset);
            }
            return Ok(Dispatch::Answered(value[offset..].to_vec()));
        }
        let id = self.queue(central, service, characteristic, RequestKind::Read { offset });
        Ok(Dispatch::Queued(id))
    }

    /// A central writes a characteristic at `offset`. Needs the property for the write kind, the write permission
    /// and its encryption, and a total length within 512 bytes. A stored value is updated at once; without one, the
    /// write waits for the app.
    pub fn on_write(
        &mut self,
        central: CentralId,
        service: Uuid,
        characteristic: Uuid,
        offset: usize,
        value: Vec<u8>,
        kind: WriteType,
    ) -> Result<Dispatch, AttError> {
        let encrypted = self.link_encrypted(central)?;
        let ch = self.characteristic(service, characteristic).map_err(|_| AttError::AttributeNotFound)?;
        let needed = match kind {
            WriteType::WithResponse => CharProperties::WRITE,
            WriteType::WithoutResponse => CharProperties::WRITE_WITHOUT_RESPONSE,
        };
        if !ch.properties.contains(needed) {
            return Err(AttError::RequestNotSupported);
        }
        if !ch.permissions.contains(AttrPermissions::WRITEABLE) {
            return Err(AttError::WriteNotPermitted);
        }
        if !ch.permissions.allows_write(encrypted) {
            return Err(AttError::InsufficientEncryption);
        }
        if offset.saturating_add(value.len()) > MAX_ATTRIBUTE_LEN {
            return Err(AttError::InvalidAttributeValueLength);
        }
        if ch.value.is_none() {
            let id = self.queue(central, service, characteristic, RequestKind::Write { offset, value, kind });
            return Ok(Dispatch::Queued(id));
        }
        let ch = self.characteristic_mut(service, characteristic).map_err(|_| AttError::AttributeNotFound)?;
        let stored = match ch.value.as_mut() {
            Some(v) => v,
            None => return Err(AttError::UnlikelyError),
        };
        if offset > stored.len() {
            return Err(AttError::InvalidOffset);
        }
        let end = offset + value.len();
        if end > stored.len() {
            stored.resize(end, 0);
        }
        stored[offset..end].copy_from_slice(&value);
        Ok(Dispatch::Answered(Vec::new()))
    }

    /// Requests waiting for the app, oldest first.
    pub fn pending(&self) -> &[ServerRequest] {
        &self.pending
    }

    /// Answers a queued request. For a read, `Ok` carries the whole characteristic value, and the model sends the
    /// bytes from the request's offset. Reading past the end is an invalid offset. For a write, `Ok` means the
    /// write is accepted; the app stores the bytes with `set_value`.
    pub fn respond(&mut self, request: RequestId, result: Result<Vec<u8>, AttError>) -> Result<(), PeripheralError> {
        let pos = self
            .pending
            .iter()
            .position(|r| r.id == request)
            .ok_or(PeripheralError::NoPendingRequest(request))?;
        let req = self.pending.remove(pos);
        let outcome = match (&req.kind, result) {
            (RequestKind::Read { offset }, Ok(full)) => {
                if *offset > full.len() {
                    Err(AttError::InvalidOffset)
                } else {
                    Ok(full[*offset..].to_vec())
                }
            }
            (RequestKind::Read { .. }, Err(e)) => Err(e),
            (RequestKind::Write { .. }, Ok(_)) => Ok(Vec::new()),
            (RequestKind::Write { .. }, Err(e)) => Err(e),
        };
        self.replies.push(Reply { central: req.central, request: req.id, result: outcome });
        Ok(())
    }

    /// Replies the transport has to send, oldest first. Taking them empties the queue.
    pub fn take_replies(&mut self) -> Vec<Reply> {
        std::mem::take(&mut self.replies)
    }

    /// Sends a new value to subscribed centrals: all of them, or only the ones in `only`. A characteristic with the
    /// notify property sends notifications, and one with only the indicate property sends indications. The value
    /// must fit the smallest MTU among the targets. With no subscribers the call succeeds and sends nothing.
    pub fn update_value(
        &mut self,
        service: Uuid,
        characteristic: Uuid,
        value: &[u8],
        only: Option<&[CentralId]>,
    ) -> Result<Vec<Update>, PeripheralError> {
        self.require_powered_on()?;
        if !self.ready {
            return Err(PeripheralError::TransmitQueueFull);
        }
        let p = self.characteristic(service, characteristic)?.properties;
        if !(p.contains(CharProperties::NOTIFY) || p.contains(CharProperties::INDICATE)) {
            return Err(PeripheralError::PropertyNotSupported);
        }
        let indication = !p.contains(CharProperties::NOTIFY);
        let key = (service, characteristic);
        let targets: Vec<&Central> = self
            .centrals
            .iter()
            .filter(|c| c.subscriptions.contains(&key) && only.map_or(true, |list| list.contains(&c.id)))
            .collect();
        if let Some(max) = targets.iter().map(|c| notify_payload(c.mtu)).min() {
            if value.len() > max {
                return Err(PeripheralError::ValueTooLong { max });
            }
        }
        Ok(targets
            .iter()
            .map(|c| Update {
                central: c.id,
                service,
                characteristic,
                value: value.to_vec(),
                indication,
            })
            .collect())
    }
}

impl Default for PeripheralManager {
    fn default() -> Self {
        PeripheralManager::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uuid(s: &str) -> Uuid {
        Uuid::parse(s).unwrap()
    }

    /// One service (heart rate, 180D) with a notifying measurement (2A37) and a writable control point (2A39).
    fn powered_with_heart_rate() -> (PeripheralManager, Uuid, Uuid, Uuid) {
        let svc = uuid("180D");
        let measure = uuid("2A37");
        let control = uuid("2A39");
        let mut m = PeripheralManager::new();
        m.set_state(ManagerState::PoweredOn);
        m.add_service(MutableService::new(
            svc,
            true,
            vec![
                MutableCharacteristic::new(
                    measure,
                    CharProperties::NOTIFY,
                    AttrPermissions::READABLE,
                    None,
                ),
                MutableCharacteristic::new(
                    control,
                    CharProperties::WRITE | CharProperties::READ,
                    AttrPermissions::READABLE | AttrPermissions::WRITEABLE,
                    None,
                ),
            ],
        ))
        .unwrap();
        (m, svc, measure, control)
    }

    #[test]
    fn advertising_budget_counts_names_and_uuid_lists_plus_flags() {
        // "BifrOSt" is 7 bytes: 2 + 7 = 9. 180D is a 16-bit UUID: 2 + 2 = 4. Payload 13, with flags 16 <= 31.
        let small = AdvertisingData {
            local_name: Some("BifrOSt".to_string()),
            service_uuids: vec![uuid("180D")],
        };
        assert_eq!(small.payload_len(), 13);
        assert!(small.fits_legacy());
        // One 128-bit UUID: 2 + 16 = 18. Name 9 + 18 = 27 payload, 30 with flags: fits.
        let mut boundary = AdvertisingData {
            local_name: Some("BifrOSt".to_string()),
            service_uuids: vec![uuid("6E400001-B5A3-F393-E0A9-E50E24DCCA9E")],
        };
        assert_eq!(boundary.payload_len(), 27);
        assert!(boundary.fits_legacy());
        // A 16-bit UUID adds 4 more: payload 31, with flags 34 > 31.
        boundary.service_uuids.push(uuid("180D"));
        assert_eq!(boundary.payload_len(), 31);
        assert!(!boundary.fits_legacy());
        // Two 128-bit UUIDs share one structure: 2 + 32 = 34 > 31 on their own.
        let two = AdvertisingData {
            local_name: None,
            service_uuids: vec![uuid("6E400001-B5A3-F393-E0A9-E50E24DCCA9E"), uuid("6E400002-B5A3-F393-E0A9-E50E24DCCA9E")],
        };
        assert_eq!(two.payload_len(), 34);
        assert!(!two.fits_legacy());
    }

    #[test]
    fn services_need_power_and_unique_uuids() {
        let mut m = PeripheralManager::new();
        let svc = uuid("180F");
        let battery = MutableService::new(svc, true, vec![]);
        assert_eq!(m.add_service(battery.clone()), Err(PeripheralError::NotPoweredOn(ManagerState::Unknown)));
        m.set_state(ManagerState::PoweredOn);
        assert_eq!(m.add_service(battery.clone()), Ok(()));
        assert_eq!(m.add_service(battery), Err(PeripheralError::DuplicateService(svc)));
        let level = uuid("2A19");
        let twice = MutableService::new(
            uuid("180A"),
            true,
            vec![
                MutableCharacteristic::new(level, CharProperties::READ, AttrPermissions::READABLE, Some(vec![1])),
                MutableCharacteristic::new(level, CharProperties::READ, AttrPermissions::READABLE, Some(vec![2])),
            ],
        );
        assert_eq!(m.add_service(twice), Err(PeripheralError::DuplicateCharacteristic(level)));
        assert_eq!(m.services().len(), 1);
    }

    #[test]
    fn leaving_power_on_stops_advertising_and_drops_centrals() {
        let (mut m, _svc, measure, _) = powered_with_heart_rate();
        let name = AdvertisingData { local_name: Some("Midget".to_string()), service_uuids: vec![] };
        m.start_advertising(name.clone()).unwrap();
        assert!(m.is_advertising());
        m.connect(CentralId(7), false).unwrap();
        m.set_subscription(CentralId(7), uuid("180D"), measure, true).unwrap();
        m.set_state(ManagerState::PoweredOff);
        assert!(!m.is_advertising());
        assert_eq!(m.start_advertising(name), Err(PeripheralError::NotPoweredOn(ManagerState::PoweredOff)));
        m.set_state(ManagerState::PoweredOn);
        assert!(m.subscribed_centrals(uuid("180D"), measure).is_empty());
        assert_eq!(m.services().len(), 1);
    }

    #[test]
    fn stored_values_answer_reads_from_an_offset_and_refuse_bad_offsets() {
        let mut m = PeripheralManager::new();
        m.set_state(ManagerState::PoweredOn);
        let svc = uuid("180A");
        let name = uuid("2A00");
        let write_only = uuid("2A38");
        m.add_service(MutableService::new(
            svc,
            true,
            vec![
                MutableCharacteristic::new(
                    name,
                    CharProperties::READ,
                    AttrPermissions::READABLE,
                    Some(b"BifrOSt".to_vec()),
                ),
                MutableCharacteristic::new(
                    write_only,
                    CharProperties::WRITE,
                    AttrPermissions::READABLE | AttrPermissions::WRITEABLE,
                    Some(vec![0]),
                ),
            ],
        ))
        .unwrap();
        m.connect(CentralId(1), false).unwrap();
        // "BifrOSt": B0 i1 f2 r3 O4 S5 t6. Offset 2 gives "frOSt".
        assert_eq!(m.on_read(CentralId(1), svc, name, 2), Ok(Dispatch::Answered(b"frOSt".to_vec())));
        // Offset 7 is the end of the value: an empty answer. Offset 8 is past it.
        assert_eq!(m.on_read(CentralId(1), svc, name, 7), Ok(Dispatch::Answered(Vec::new())));
        assert_eq!(m.on_read(CentralId(1), svc, name, 8), Err(AttError::InvalidOffset));
        // A characteristic without the read property refuses the read.
        assert_eq!(m.on_read(CentralId(1), svc, write_only, 0), Err(AttError::RequestNotSupported));
        // An unknown central and an unknown attribute.
        assert_eq!(m.on_read(CentralId(9), svc, name, 0), Err(AttError::UnlikelyError));
        assert_eq!(m.on_read(CentralId(1), svc, uuid("2A01"), 0), Err(AttError::AttributeNotFound));
    }

    #[test]
    fn dynamic_reads_wait_for_the_app_and_respond_once() {
        let (mut m, svc, measure, control) = powered_with_heart_rate();
        m.connect(CentralId(3), false).unwrap();
        // The measurement is notify-only, so the read goes to the control point, which has the read property.
        assert_eq!(m.on_read(CentralId(3), svc, measure, 0), Err(AttError::RequestNotSupported));
        let queued = match m.on_read(CentralId(3), svc, control, 1) {
            Ok(Dispatch::Queued(id)) => id,
            other => panic!("expected a queued read, got {other:?}"),
        };
        assert_eq!(m.pending().len(), 1);
        assert_eq!(m.pending()[0].kind, RequestKind::Read { offset: 1 });
        // The app answers with the whole value. The model sends the bytes from offset 1.
        m.respond(queued, Ok(vec![0x06, 72, 75, 80])).unwrap();
        assert!(m.pending().is_empty());
        let replies = m.take_replies();
        assert_eq!(replies.len(), 1);
        assert_eq!(replies[0].central, CentralId(3));
        assert_eq!(replies[0].result, Ok(vec![72, 75, 80]));
        assert!(m.take_replies().is_empty());
        assert_eq!(m.respond(queued, Ok(vec![])), Err(PeripheralError::NoPendingRequest(queued)));
    }

    #[test]
    fn writes_check_property_permission_encryption_and_length() {
        let svc = uuid("180D");
        let control = uuid("2A39");
        let ro = uuid("2A3B");
        let mut m = PeripheralManager::new();
        m.set_state(ManagerState::PoweredOn);
        m.add_service(MutableService::new(
            svc,
            true,
            vec![
                MutableCharacteristic::new(
                    control,
                    CharProperties::WRITE,
                    AttrPermissions::WRITEABLE | AttrPermissions::WRITE_ENCRYPTION_REQUIRED,
                    None,
                ),
                MutableCharacteristic::new(ro, CharProperties::WRITE, AttrPermissions::READABLE, None),
            ],
        ))
        .unwrap();
        m.connect(CentralId(1), false).unwrap();
        // Permission is not writable on this one.
        assert_eq!(
            m.on_write(CentralId(1), svc, ro, 0, vec![1], WriteType::WithResponse),
            Err(AttError::WriteNotPermitted)
        );
        // Encryption is required and the link is not encrypted.
        assert_eq!(
            m.on_write(CentralId(1), svc, control, 0, vec![1], WriteType::WithResponse),
            Err(AttError::InsufficientEncryption)
        );
        // Without response is a different property, and this characteristic only has write-with-response.
        assert_eq!(
            m.on_write(CentralId(1), svc, control, 0, vec![1], WriteType::WithoutResponse),
            Err(AttError::RequestNotSupported)
        );
        m.connect(CentralId(1), true).unwrap();
        assert!(matches!(
            m.on_write(CentralId(1), svc, control, 0, vec![1, 2], WriteType::WithResponse),
            Ok(Dispatch::Queued(_))
        ));
        // A total length of 513 bytes is over the 512-byte cap, and 512 is allowed.
        assert_eq!(
            m.on_write(CentralId(1), svc, control, 1, vec![0; 512], WriteType::WithResponse),
            Err(AttError::InvalidAttributeValueLength)
        );
        assert!(m.on_write(CentralId(1), svc, control, 0, vec![0; 512], WriteType::WithResponse).is_ok());
    }

    #[test]
    fn stored_writes_update_the_value_and_grow_it_from_an_offset() {
        let svc = uuid("180D");
        let ch = uuid("2A3F");
        let mut m = PeripheralManager::new();
        m.set_state(ManagerState::PoweredOn);
        m.add_service(MutableService::new(
            svc,
            true,
            vec![MutableCharacteristic::new(
                ch,
                CharProperties::WRITE,
                AttrPermissions::WRITEABLE,
                Some(vec![1, 2, 3]),
            )],
        ))
        .unwrap();
        m.connect(CentralId(2), false).unwrap();
        assert_eq!(
            m.on_write(CentralId(2), svc, ch, 1, vec![9, 9, 9], WriteType::WithResponse),
            Ok(Dispatch::Answered(Vec::new()))
        );
        // [1, 2, 3] with 9 9 9 written at 1 gives [1, 9, 9, 9], which grew the value by one byte.
        assert_eq!(m.services()[0].characteristics[0].value, Some(vec![1, 9, 9, 9]));
        // The characteristic has no read property, and a write past the end of the value is an invalid offset.
        assert_eq!(m.on_read(CentralId(2), svc, ch, 0), Err(AttError::RequestNotSupported));
        assert_eq!(
            m.on_write(CentralId(2), svc, ch, 6, vec![5], WriteType::WithResponse),
            Err(AttError::InvalidOffset)
        );
    }

    #[test]
    fn subscriptions_gate_updates_by_mtu_queue_and_filter() {
        let (mut m, svc, measure, _) = powered_with_heart_rate();
        m.connect(CentralId(1), false).unwrap();
        m.connect(CentralId(2), false).unwrap();
        m.set_subscription(CentralId(1), svc, measure, true).unwrap();
        assert_eq!(m.subscribed_centrals(svc, measure), vec![CentralId(1)]);
        // Central 1 is at the default MTU of 23: a 20-byte payload fits, 21 does not.
        let ok = m.update_value(svc, measure, &[0u8; 20], None).unwrap();
        assert_eq!(ok.len(), 1);
        assert_eq!(ok[0].central, CentralId(1));
        assert!(!ok[0].indication);
        m.set_subscription(CentralId(2), svc, measure, true).unwrap();
        m.set_mtu(CentralId(2), 185).unwrap();
        // Central 2 would take 182 bytes, but the smallest MTU among the targets is 23 (central 1).
        assert_eq!(
            m.update_value(svc, measure, &[0u8; 21], None),
            Err(PeripheralError::ValueTooLong { max: 20 })
        );
        // The filter sends to central 2 alone, and its larger MTU then allows 100 bytes.
        let only_two = m.update_value(svc, measure, &[0u8; 100], Some(&[CentralId(2)][..])).unwrap();
        assert_eq!(only_two.len(), 1);
        assert_eq!(only_two[0].central, CentralId(2));
        m.set_ready(false);
        assert_eq!(m.update_value(svc, measure, &[1], None), Err(PeripheralError::TransmitQueueFull));
        m.set_ready(true);
        m.set_subscription(CentralId(1), svc, measure, false).unwrap();
        assert_eq!(m.subscribed_centrals(svc, measure), vec![CentralId(2)]);
    }

    #[test]
    fn indicate_only_characteristics_send_indications_and_check_encryption() {
        let svc = uuid("180D");
        let secure = uuid("2A3A");
        let mut m = PeripheralManager::new();
        m.set_state(ManagerState::PoweredOn);
        m.add_service(MutableService::new(
            svc,
            true,
            vec![MutableCharacteristic::new(
                secure,
                CharProperties::INDICATE | CharProperties::INDICATE_ENCRYPTION_REQUIRED,
                AttrPermissions::READABLE,
                None,
            )],
        ))
        .unwrap();
        m.connect(CentralId(4), false).unwrap();
        assert_eq!(m.set_subscription(CentralId(4), svc, secure, true), Err(AttError::InsufficientEncryption));
        m.connect(CentralId(4), true).unwrap();
        m.set_subscription(CentralId(4), svc, secure, true).unwrap();
        let sent = m.update_value(svc, secure, &[1, 2], None).unwrap();
        assert_eq!(sent.len(), 1);
        assert!(sent[0].indication);
        // A characteristic with neither property cannot be subscribed to.
        m.add_service(MutableService::new(
            uuid("180F"),
            true,
            vec![MutableCharacteristic::new(uuid("2A19"), CharProperties::READ, AttrPermissions::READABLE, Some(vec![1]))],
        ))
        .unwrap();
        assert_eq!(
            m.set_subscription(CentralId(4), uuid("180F"), uuid("2A19"), true),
            Err(AttError::RequestNotSupported)
        );
    }

    #[test]
    fn disconnect_drops_subscriptions_and_queued_requests() {
        let (mut m, svc, measure, control) = powered_with_heart_rate();
        m.connect(CentralId(5), false).unwrap();
        m.set_subscription(CentralId(5), svc, measure, true).unwrap();
        assert!(matches!(
            m.on_write(CentralId(5), svc, control, 0, vec![1], WriteType::WithResponse),
            Ok(Dispatch::Queued(_))
        ));
        assert_eq!(m.pending().len(), 1);
        m.disconnect(CentralId(5));
        assert!(m.subscribed_centrals(svc, measure).is_empty());
        assert!(m.pending().is_empty());
        assert_eq!(m.set_mtu(CentralId(5), 100), Err(PeripheralError::UnknownCentral(CentralId(5))));
    }
}
