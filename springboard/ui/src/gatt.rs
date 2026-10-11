//! GATT client model for the central role: UUIDs, characteristic properties, attribute permissions, the discovered
//! service / characteristic / descriptor tree, and the attribute requests each operation produces.
//! Pure model: there is no radio and no transport here. A transport feeds discovery responses in with the `on_*`
//! methods, sends the `AttRequest` values that `read`, `write` and `set_notify` return, and reports back with
//! `on_value` and `confirm_notify`.
use std::fmt;
use std::ops::BitOr;

/// Bluetooth base UUID suffix (00000000-0000-1000-8000-00805F9B34FB) with the 32-bit prefix removed.
const BASE_LOW: u128 = 0x0000_1000_8000_0080_5f9b_34fb;
/// Client Characteristic Configuration descriptor (16-bit UUID 0x2902).
pub const CCCD_UUID: u16 = 0x2902;
/// Default ATT MTU before an exchange: a 23-byte payload, 3 bytes of which are the ATT header.
pub const DEFAULT_MTU: u16 = 23;
/// Largest attribute value the protocol allows, which bounds a write with response.
pub const MAX_ATTRIBUTE_LEN: usize = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Uuid(u128);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GattError {
    InvalidUuid(String),
    ServiceNotFound,
    CharacteristicNotFound,
    /// The layer below has not reported this level of the tree yet.
    NotDiscovered,
    PropertyNotSupported,
    EncryptionRequired,
    ValueTooLong { max: usize },
    /// Notifications need a Client Characteristic Configuration descriptor, and the discovered tree has none.
    NoCccd,
}

impl fmt::Display for GattError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GattError::InvalidUuid(s) => write!(f, "invalid UUID '{s}'"),
            GattError::ServiceNotFound => write!(f, "service not found"),
            GattError::CharacteristicNotFound => write!(f, "characteristic not found"),
            GattError::NotDiscovered => write!(f, "not discovered yet"),
            GattError::PropertyNotSupported => write!(f, "operation not supported by the characteristic"),
            GattError::EncryptionRequired => write!(f, "the link must be encrypted"),
            GattError::ValueTooLong { max } => write!(f, "value longer than {max} bytes"),
            GattError::NoCccd => write!(f, "characteristic has no client configuration descriptor"),
        }
    }
}

impl Uuid {
    /// Accepts a 16-bit ("180F"), 32-bit ("0000180F") or 128-bit UUID, with or without dashes.
    pub fn parse(s: &str) -> Result<Uuid, GattError> {
        let bad = || GattError::InvalidUuid(s.to_string());
        let hex = if s.len() == 36 {
            let dashes = [8usize, 13, 18, 23];
            for (i, c) in s.char_indices() {
                if dashes.contains(&i) != (c == '-') {
                    return Err(bad());
                }
            }
            s.replace('-', "")
        } else {
            s.to_string()
        };
        if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(bad());
        }
        match hex.len() {
            4 | 8 => {
                let short = u32::from_str_radix(&hex, 16).map_err(|_| bad())?;
                Ok(Uuid(((short as u128) << 96) | BASE_LOW))
            }
            32 => Ok(Uuid(u128::from_str_radix(&hex, 16).map_err(|_| bad())?)),
            _ => Err(bad()),
        }
    }

    pub fn from_16(short: u16) -> Uuid {
        Uuid(((short as u128) << 96) | BASE_LOW)
    }

    /// The 16-bit value when this UUID is a base-UUID alias of a 16- or 32-bit value.
    pub fn short(&self) -> Option<u32> {
        let low = self.0 & ((1u128 << 96) - 1);
        let high = (self.0 >> 96) as u32;
        if low == BASE_LOW {
            Some(high)
        } else {
            None
        }
    }

    /// Short form for base-UUID aliases ("180F", "0000180F" for 32-bit), dashed upper-case 128-bit otherwise.
    pub fn uuid_string(&self) -> String {
        match self.short() {
            Some(v) if v <= 0xFFFF => format!("{v:04X}"),
            Some(v) => format!("{v:08X}"),
            None => {
                let v = self.0;
                format!(
                    "{:08X}-{:04X}-{:04X}-{:04X}-{:012X}",
                    (v >> 96) as u32,
                    (v >> 80) & 0xFFFF,
                    (v >> 64) & 0xFFFF,
                    (v >> 48) & 0xFFFF,
                    v & 0xFFFF_FFFF_FFFF
                )
            }
        }
    }

    pub fn to_bytes_le(&self) -> [u8; 16] {
        self.0.to_le_bytes()
    }
}

/// Characteristic properties: which operations the remote characteristic allows. Bit values follow the
/// characteristic property field of the Bluetooth core specification.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CharProperties(u16);

impl CharProperties {
    pub const BROADCAST: CharProperties = CharProperties(0x001);
    pub const READ: CharProperties = CharProperties(0x002);
    pub const WRITE_WITHOUT_RESPONSE: CharProperties = CharProperties(0x004);
    pub const WRITE: CharProperties = CharProperties(0x008);
    pub const NOTIFY: CharProperties = CharProperties(0x010);
    pub const INDICATE: CharProperties = CharProperties(0x020);
    pub const AUTHENTICATED_SIGNED_WRITES: CharProperties = CharProperties(0x040);
    pub const EXTENDED_PROPERTIES: CharProperties = CharProperties(0x080);
    pub const NOTIFY_ENCRYPTION_REQUIRED: CharProperties = CharProperties(0x100);
    pub const INDICATE_ENCRYPTION_REQUIRED: CharProperties = CharProperties(0x200);

    pub fn from_raw(raw: u16) -> CharProperties {
        CharProperties(raw)
    }

    pub fn raw(&self) -> u16 {
        self.0
    }

    /// True when every flag in `other` is set here.
    pub fn contains(&self, other: CharProperties) -> bool {
        self.0 & other.0 == other.0
    }
}

impl BitOr for CharProperties {
    type Output = CharProperties;
    fn bitor(self, rhs: CharProperties) -> CharProperties {
        CharProperties(self.0 | rhs.0)
    }
}

/// Read and write permissions and encryption requirements of an attribute value, for a local attribute table.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AttrPermissions(u8);

impl AttrPermissions {
    pub const READABLE: AttrPermissions = AttrPermissions(0x01);
    pub const WRITEABLE: AttrPermissions = AttrPermissions(0x02);
    pub const READ_ENCRYPTION_REQUIRED: AttrPermissions = AttrPermissions(0x04);
    pub const WRITE_ENCRYPTION_REQUIRED: AttrPermissions = AttrPermissions(0x08);

    pub fn from_raw(raw: u8) -> AttrPermissions {
        AttrPermissions(raw & 0x0F)
    }

    pub fn raw(&self) -> u8 {
        self.0
    }

    pub fn contains(&self, other: AttrPermissions) -> bool {
        self.0 & other.0 == other.0
    }

    /// Whether a read from a peer is allowed on a link with the given encryption state.
    pub fn allows_read(&self, link_encrypted: bool) -> bool {
        self.contains(AttrPermissions::READABLE)
            && (link_encrypted || !self.contains(AttrPermissions::READ_ENCRYPTION_REQUIRED))
    }

    /// Whether a write from a peer is allowed on a link with the given encryption state.
    pub fn allows_write(&self, link_encrypted: bool) -> bool {
        self.contains(AttrPermissions::WRITEABLE)
            && (link_encrypted || !self.contains(AttrPermissions::WRITE_ENCRYPTION_REQUIRED))
    }
}

impl BitOr for AttrPermissions {
    type Output = AttrPermissions;
    fn bitor(self, rhs: AttrPermissions) -> AttrPermissions {
        AttrPermissions(self.0 | rhs.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteType {
    WithResponse,
    WithoutResponse,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Descriptor {
    pub uuid: Uuid,
    pub value: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Characteristic {
    pub uuid: Uuid,
    pub properties: CharProperties,
    /// `None` until descriptor discovery has run for this characteristic.
    pub descriptors: Option<Vec<Descriptor>>,
    pub value: Option<Vec<u8>>,
    pub notifying: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Service {
    pub uuid: Uuid,
    pub primary: bool,
    /// `None` until characteristic discovery has run for this service.
    pub characteristics: Option<Vec<Characteristic>>,
}

/// An attribute request for the transport to send. The transport maps the UUIDs to attribute handles.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AttRequest {
    ReadCharacteristic { service: Uuid, characteristic: Uuid },
    ReadDescriptor { service: Uuid, characteristic: Uuid, descriptor: Uuid },
    WriteCharacteristic { service: Uuid, characteristic: Uuid, value: Vec<u8>, kind: WriteType },
    WriteDescriptor { service: Uuid, characteristic: Uuid, descriptor: Uuid, value: Vec<u8> },
}

/// The remote tree held by a central, filled in as discovery responses arrive.
#[derive(Clone, Debug)]
pub struct GattClient {
    services: Vec<Service>,
    mtu: u16,
}

impl GattClient {
    pub fn new() -> GattClient {
        GattClient { services: Vec::new(), mtu: DEFAULT_MTU }
    }

    /// The negotiated ATT MTU, clamped to the protocol range 23..=517.
    pub fn mtu(&self) -> u16 {
        self.mtu
    }

    pub fn set_mtu(&mut self, mtu: u16) {
        self.mtu = mtu.clamp(DEFAULT_MTU, 517);
    }

    /// Largest value a single write of this kind carries. Without response it is the ATT payload (MTU minus the
    /// 3-byte header). With response the protocol cap of 512 bytes applies (prepared writes can exceed the MTU).
    pub fn max_write_length(&self, kind: WriteType) -> usize {
        match kind {
            WriteType::WithoutResponse => (self.mtu - 3) as usize,
            WriteType::WithResponse => MAX_ATTRIBUTE_LEN,
        }
    }

    pub fn services(&self) -> &[Service] {
        &self.services
    }

    /// Records the primary or included services a discovery response returned. Their characteristics reset to
    /// not-yet-discovered.
    pub fn on_services(&mut self, found: Vec<(Uuid, bool)>) {
        self.services = found
            .into_iter()
            .map(|(uuid, primary)| Service { uuid, primary, characteristics: None })
            .collect();
    }

    pub fn on_characteristics(
        &mut self,
        service: Uuid,
        found: Vec<(Uuid, CharProperties)>,
    ) -> Result<(), GattError> {
        let svc = self.service_mut(service)?;
        svc.characteristics = Some(
            found
                .into_iter()
                .map(|(uuid, properties)| Characteristic {
                    uuid,
                    properties,
                    descriptors: None,
                    value: None,
                    notifying: false,
                })
                .collect(),
        );
        Ok(())
    }

    pub fn on_descriptors(
        &mut self,
        service: Uuid,
        characteristic: Uuid,
        found: Vec<(Uuid, Vec<u8>)>,
    ) -> Result<(), GattError> {
        let ch = self.characteristic_mut(service, characteristic)?;
        ch.descriptors = Some(found.into_iter().map(|(uuid, value)| Descriptor { uuid, value }).collect());
        Ok(())
    }

    /// Stores a value from a read response, a write response or an incoming notification or indication.
    pub fn on_value(&mut self, service: Uuid, characteristic: Uuid, value: Vec<u8>) -> Result<(), GattError> {
        self.characteristic_mut(service, characteristic)?.value = Some(value);
        Ok(())
    }

    pub fn characteristics(&self, service: Uuid) -> Result<&[Characteristic], GattError> {
        let svc = self.service(service)?;
        svc.characteristics.as_deref().ok_or(GattError::NotDiscovered)
    }

    pub fn characteristic(&self, service: Uuid, characteristic: Uuid) -> Result<&Characteristic, GattError> {
        self.characteristics(service)?
            .iter()
            .find(|c| c.uuid == characteristic)
            .ok_or(GattError::CharacteristicNotFound)
    }

    pub fn value(&self, service: Uuid, characteristic: Uuid) -> Option<&[u8]> {
        self.characteristic(service, characteristic).ok()?.value.as_deref()
    }

    /// A read request. Needs the read property.
    pub fn read(&self, service: Uuid, characteristic: Uuid) -> Result<AttRequest, GattError> {
        let ch = self.characteristic(service, characteristic)?;
        if !ch.properties.contains(CharProperties::READ) {
            return Err(GattError::PropertyNotSupported);
        }
        Ok(AttRequest::ReadCharacteristic { service, characteristic })
    }

    /// A descriptor read request. Descriptors must have been discovered.
    pub fn read_descriptor(&self, service: Uuid, characteristic: Uuid, descriptor: Uuid) -> Result<AttRequest, GattError> {
        let ch = self.characteristic(service, characteristic)?;
        let found = ch.descriptors.as_ref().ok_or(GattError::NotDiscovered)?;
        if !found.iter().any(|d| d.uuid == descriptor) {
            return Err(GattError::CharacteristicNotFound);
        }
        Ok(AttRequest::ReadDescriptor { service, characteristic, descriptor })
    }

    /// A write request. Checks the property for the write kind and the length limit for the current MTU.
    pub fn write(
        &self,
        service: Uuid,
        characteristic: Uuid,
        value: &[u8],
        kind: WriteType,
    ) -> Result<AttRequest, GattError> {
        let ch = self.characteristic(service, characteristic)?;
        let needed = match kind {
            WriteType::WithResponse => CharProperties::WRITE,
            WriteType::WithoutResponse => CharProperties::WRITE_WITHOUT_RESPONSE,
        };
        if !ch.properties.contains(needed) {
            return Err(GattError::PropertyNotSupported);
        }
        let max = self.max_write_length(kind);
        if value.len() > max {
            return Err(GattError::ValueTooLong { max });
        }
        Ok(AttRequest::WriteCharacteristic { service, characteristic, value: value.to_vec(), kind })
    }

    /// Turns notifications or indications on or off. Notifications use the notify property when the characteristic has
    /// it, and indications otherwise. Writes the Client Characteristic Configuration descriptor (value 0x0001
    /// notify, 0x0002 indicate, 0x0000 off). When the descriptors are known and none is the configuration descriptor,
    /// the call fails with `NoCccd`. When the property needs encryption, the link must be encrypted.
    pub fn set_notify(
        &self,
        service: Uuid,
        characteristic: Uuid,
        enable: bool,
        link_encrypted: bool,
    ) -> Result<AttRequest, GattError> {
        let ch = self.characteristic(service, characteristic)?;
        let p = ch.properties;
        let bits: u16 = if !enable {
            0
        } else if p.contains(CharProperties::NOTIFY) {
            if p.contains(CharProperties::NOTIFY_ENCRYPTION_REQUIRED) && !link_encrypted {
                return Err(GattError::EncryptionRequired);
            }
            0x0001
        } else if p.contains(CharProperties::INDICATE) {
            if p.contains(CharProperties::INDICATE_ENCRYPTION_REQUIRED) && !link_encrypted {
                return Err(GattError::EncryptionRequired);
            }
            0x0002
        } else {
            return Err(GattError::PropertyNotSupported);
        };
        let cccd = Uuid::from_16(CCCD_UUID);
        if let Some(found) = &ch.descriptors {
            if !found.iter().any(|d| d.uuid == cccd) {
                return Err(GattError::NoCccd);
            }
        }
        Ok(AttRequest::WriteDescriptor {
            service,
            characteristic,
            descriptor: cccd,
            value: bits.to_le_bytes().to_vec(),
        })
    }

    /// Records the outcome of a notify request, after the descriptor write is acknowledged.
    pub fn confirm_notify(&mut self, service: Uuid, characteristic: Uuid, enabled: bool) -> Result<(), GattError> {
        self.characteristic_mut(service, characteristic)?.notifying = enabled;
        Ok(())
    }

    fn service(&self, uuid: Uuid) -> Result<&Service, GattError> {
        self.services.iter().find(|s| s.uuid == uuid).ok_or(GattError::ServiceNotFound)
    }

    fn service_mut(&mut self, uuid: Uuid) -> Result<&mut Service, GattError> {
        self.services.iter_mut().find(|s| s.uuid == uuid).ok_or(GattError::ServiceNotFound)
    }

    fn characteristic_mut(&mut self, service: Uuid, characteristic: Uuid) -> Result<&mut Characteristic, GattError> {
        let chars = self
            .service_mut(service)?
            .characteristics
            .as_mut()
            .ok_or(GattError::NotDiscovered)?;
        chars.iter_mut().find(|c| c.uuid == characteristic).ok_or(GattError::CharacteristicNotFound)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hr_client() -> (GattClient, Uuid, Uuid) {
        let svc = Uuid::parse("180D").unwrap();
        let measure = Uuid::parse("2A37").unwrap();
        let mut c = GattClient::new();
        c.on_services(vec![(svc, true)]);
        c.on_characteristics(
            svc,
            vec![(measure, CharProperties::NOTIFY | CharProperties::READ)],
        )
        .unwrap();
        (c, svc, measure)
    }

    #[test]
    fn uuid_parse_expands_short_forms_and_round_trips() {
        let short = Uuid::parse("180F").unwrap();
        assert_eq!(short, Uuid::from_16(0x180F));
        assert_eq!(short.uuid_string(), "180F");
        assert_eq!(Uuid::parse("0000180f").unwrap(), short);
        let long = Uuid::parse("6E400001-B5A3-F393-E0A9-E50E24DCCA9E").unwrap();
        assert_eq!(long.short(), None);
        assert_eq!(long.uuid_string(), "6E400001-B5A3-F393-E0A9-E50E24DCCA9E");
        assert_eq!(Uuid::parse("6e400001b5a3f393e0a9e50e24dcca9e").unwrap(), long);
    }

    #[test]
    fn uuid_parse_rejects_malformed_input() {
        assert!(Uuid::parse("18").is_err());
        assert!(Uuid::parse("GGGG").is_err());
        assert!(Uuid::parse("6E400001B5A3-F393-E0A9-E50E24DCCA9E").is_err());
    }

    #[test]
    fn properties_and_permissions_combine_and_check_flags() {
        let p = CharProperties::from_raw(0x12);
        assert!(p.contains(CharProperties::READ) && p.contains(CharProperties::NOTIFY));
        assert!(!p.contains(CharProperties::WRITE));
        let perms = AttrPermissions::READABLE | AttrPermissions::WRITEABLE | AttrPermissions::WRITE_ENCRYPTION_REQUIRED;
        assert!(perms.allows_read(false));
        assert!(!perms.allows_write(false));
        assert!(perms.allows_write(true));
    }

    #[test]
    fn hierarchy_is_unavailable_until_discovered() {
        let svc = Uuid::parse("180D").unwrap();
        let mut c = GattClient::new();
        assert_eq!(c.characteristics(svc), Err(GattError::ServiceNotFound));
        c.on_services(vec![(svc, true)]);
        assert_eq!(c.characteristics(svc), Err(GattError::NotDiscovered));
        c.on_characteristics(svc, vec![]).unwrap();
        assert_eq!(c.characteristics(svc).unwrap().len(), 0);
    }

    #[test]
    fn read_needs_the_read_property_and_stores_the_response() {
        let (mut c, svc, measure) = hr_client();
        assert_eq!(
            c.read(svc, measure),
            Ok(AttRequest::ReadCharacteristic { service: svc, characteristic: measure })
        );
        let write_only = Uuid::parse("2A38").unwrap();
        c.on_characteristics(svc, vec![(write_only, CharProperties::WRITE)]).unwrap();
        assert_eq!(c.read(svc, write_only), Err(GattError::PropertyNotSupported));
        c.on_characteristics(svc, vec![(measure, CharProperties::READ)]).unwrap();
        c.on_value(svc, measure, vec![0x06, 72]).unwrap();
        assert_eq!(c.value(svc, measure), Some(&[0x06u8, 72][..]));
    }

    #[test]
    fn write_checks_property_kind_and_mtu_length() {
        let svc = Uuid::parse("180D").unwrap();
        let ctl = Uuid::parse("2A39").unwrap();
        let mut c = GattClient::new();
        c.on_services(vec![(svc, true)]);
        c.on_characteristics(svc, vec![(ctl, CharProperties::WRITE_WITHOUT_RESPONSE)]).unwrap();
        assert_eq!(
            c.write(svc, ctl, &[1], WriteType::WithResponse),
            Err(GattError::PropertyNotSupported)
        );
        assert_eq!(c.max_write_length(WriteType::WithoutResponse), 20);
        assert!(c.write(svc, ctl, &[0u8; 20], WriteType::WithoutResponse).is_ok());
        assert_eq!(
            c.write(svc, ctl, &[0u8; 21], WriteType::WithoutResponse),
            Err(GattError::ValueTooLong { max: 20 })
        );
        c.set_mtu(247);
        assert_eq!(c.max_write_length(WriteType::WithoutResponse), 244);
        c.set_mtu(9999);
        assert_eq!(c.mtu(), 517);
    }

    #[test]
    fn notify_writes_the_configuration_descriptor_and_tracks_state() {
        let (mut c, svc, measure) = hr_client();
        let on = c.set_notify(svc, measure, true, false).unwrap();
        match on {
            AttRequest::WriteDescriptor { descriptor, value, .. } => {
                assert_eq!(descriptor, Uuid::from_16(CCCD_UUID));
                assert_eq!(value, vec![0x01, 0x00]);
            }
            other => panic!("unexpected request {other:?}"),
        }
        assert!(!c.characteristic(svc, measure).unwrap().notifying);
        c.confirm_notify(svc, measure, true).unwrap();
        assert!(c.characteristic(svc, measure).unwrap().notifying);
        let off = c.set_notify(svc, measure, false, false).unwrap();
        assert!(matches!(off, AttRequest::WriteDescriptor { ref value, .. } if value == &vec![0, 0]));
    }

    #[test]
    fn notify_needs_an_encrypted_link_when_the_property_says_so() {
        let svc = Uuid::parse("180D").unwrap();
        let secure = Uuid::parse("2A3A").unwrap();
        let mut c = GattClient::new();
        c.on_services(vec![(svc, true)]);
        c.on_characteristics(
            svc,
            vec![(secure, CharProperties::INDICATE | CharProperties::INDICATE_ENCRYPTION_REQUIRED)],
        )
        .unwrap();
        assert_eq!(c.set_notify(svc, secure, true, false), Err(GattError::EncryptionRequired));
        match c.set_notify(svc, secure, true, true).unwrap() {
            AttRequest::WriteDescriptor { value, .. } => assert_eq!(value, vec![0x02, 0x00]),
            other => panic!("unexpected request {other:?}"),
        }
    }

    #[test]
    fn notify_refuses_a_characteristic_without_the_configuration_descriptor() {
        let (mut c, svc, measure) = hr_client();
        c.on_descriptors(svc, measure, vec![(Uuid::parse("2901").unwrap(), b"Heart rate".to_vec())]).unwrap();
        assert_eq!(c.set_notify(svc, measure, true, false), Err(GattError::NoCccd));
        c.on_descriptors(svc, measure, vec![(Uuid::from_16(CCCD_UUID), vec![])]).unwrap();
        assert!(c.set_notify(svc, measure, true, false).is_ok());
    }
}
