//! GATT server model for the peripheral role: a local attribute table of services, characteristics and
//! descriptors with handles, the permission and encryption checks an attribute server applies to reads and writes,
//! and the client configuration state that decides whether a characteristic notifies.
//! Pure model: there is no radio and no transport here. A transport maps incoming ATT requests to `read` and
//! `write`, sends the returned bytes back, and uses `is_notifying` to decide when to send a notification.
use crate::gatt::{AttrPermissions, CharProperties, Uuid, CCCD_UUID, MAX_ATTRIBUTE_LEN};


#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttError {
    /// No attribute has this handle, or the handle cannot take the requested child.
    InvalidHandle,
    ReadNotPermitted,
    WriteNotPermitted,
    /// The attribute needs an encrypted link and this one is not encrypted.
    EncryptionRequired,
    /// The value is longer than the protocol allows, or a configuration value has the wrong size.
    InvalidLength,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttrKind {
    /// Service declaration. `uuid` is the service UUID.
    Service { primary: bool },
    /// Characteristic declaration. `uuid` is the characteristic UUID, and the value is its property byte followed by
    /// the little-endian handle of its value attribute.
    Characteristic,
    /// Characteristic value. `uuid` is the characteristic UUID.
    Value { properties: CharProperties },
    /// Descriptor, a child of the characteristic value that comes before the next declaration.
    Descriptor,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attribute {
    pub handle: u16,
    pub uuid: Uuid,
    pub kind: AttrKind,
    pub perms: AttrPermissions,
    pub value: Vec<u8>,
}

/// Local attribute table. Handles start at 1 and are assigned in the order the attributes are added, so the table
/// stays sorted by handle and lookups are binary searches.
#[derive(Clone, Debug)]
pub struct GattServer {
    attrs: Vec<Attribute>,
    next_handle: u16,
}

impl GattServer {
    pub fn new() -> GattServer {
        GattServer { attrs: Vec::new(), next_handle: 1 }
    }

    /// Every attribute in handle order.
    pub fn attributes(&self) -> &[Attribute] {
        &self.attrs
    }

    /// Adds a service declaration and returns its handle.
    pub fn add_service(&mut self, uuid: Uuid, primary: bool) -> Result<u16, AttError> {
        self.push(uuid, AttrKind::Service { primary }, AttrPermissions::READABLE, Vec::new())
    }

    /// Adds a characteristic under a service handle. Returns the handle of the value attribute, which is the handle
    /// that reads, writes and descriptors refer to. The declaration takes the handle before it.
    pub fn add_characteristic(
        &mut self,
        service: u16,
        uuid: Uuid,
        properties: CharProperties,
        perms: AttrPermissions,
        value: Vec<u8>,
    ) -> Result<u16, AttError> {
        match self.index(service).map(|i| self.attrs[i].kind) {
            Some(AttrKind::Service { .. }) => {}
            _ => return Err(AttError::InvalidHandle),
        }
        if value.len() > MAX_ATTRIBUTE_LEN {
            return Err(AttError::InvalidLength);
        }
        if self.next_handle >= u16::MAX - 1 {
            return Err(AttError::InvalidHandle);
        }
        let value_handle = self.next_handle + 1;
        let [lo, hi] = value_handle.to_le_bytes();
        // The declaration value carries the property byte; the upper byte of the property field is not representable
        // in the declaration and is dropped here.
        self.push(
            uuid,
            AttrKind::Characteristic,
            AttrPermissions::READABLE,
            vec![properties.raw() as u8, lo, hi],
        )?;
        self.push(uuid, AttrKind::Value { properties }, perms, value)?;
        Ok(value_handle)
    }

    /// Adds a descriptor to a characteristic value. Only the most recently added characteristic can take one, since
    /// its descriptors must directly follow its value in the table.
    pub fn add_descriptor(
        &mut self,
        value_handle: u16,
        uuid: Uuid,
        perms: AttrPermissions,
        value: Vec<u8>,
    ) -> Result<u16, AttError> {
        let i = self.index(value_handle).ok_or(AttError::InvalidHandle)?;
        if !matches!(self.attrs[i].kind, AttrKind::Value { .. }) {
            return Err(AttError::InvalidHandle);
        }
        if self.attrs[i + 1..].iter().any(|a| a.kind != AttrKind::Descriptor) {
            return Err(AttError::InvalidHandle);
        }
        if value.len() > MAX_ATTRIBUTE_LEN {
            return Err(AttError::InvalidLength);
        }
        self.push(uuid, AttrKind::Descriptor, perms, value)
    }

    /// A read from a peer. Declarations are always readable; values and descriptors go through the permission checks.
    pub fn read(&self, handle: u16, link_encrypted: bool) -> Result<&[u8], AttError> {
        let a = &self.attrs[self.index(handle).ok_or(AttError::InvalidHandle)?];
        gate(a, link_encrypted, false)?;
        Ok(&a.value)
    }

    /// A write from a peer. Declarations are never writable. A client configuration descriptor must be exactly two
    /// bytes long, and every value must fit in `MAX_ATTRIBUTE_LEN`.
    pub fn write(&mut self, handle: u16, value: &[u8], link_encrypted: bool) -> Result<(), AttError> {
        let i = self.index(handle).ok_or(AttError::InvalidHandle)?;
        gate(&self.attrs[i], link_encrypted, true)?;
        if value.len() > MAX_ATTRIBUTE_LEN {
            return Err(AttError::InvalidLength);
        }
        let is_cccd = self.attrs[i].kind == AttrKind::Descriptor && self.attrs[i].uuid == cccd_uuid();
        if is_cccd && value.len() != 2 {
            return Err(AttError::InvalidLength);
        }
        self.attrs[i].value = value.to_vec();
        Ok(())
    }

    /// Local update of a value by the server itself, outside any peer request, so no permission check applies.
    pub fn set_value(&mut self, handle: u16, value: &[u8]) -> Result<(), AttError> {
        let i = self.index(handle).ok_or(AttError::InvalidHandle)?;
        if value.len() > MAX_ATTRIBUTE_LEN {
            return Err(AttError::InvalidLength);
        }
        self.attrs[i].value = value.to_vec();
        Ok(())
    }

    /// The stored value of an attribute, without any permission check.
    pub fn value(&self, handle: u16) -> Option<&[u8]> {
        self.index(handle).map(|i| self.attrs[i].value.as_slice())
    }

    /// True when a peer has enabled notifications (`NOTIFY`) or indications (`INDICATE`) on this value, through its
    /// client configuration descriptor, and the characteristic allows that operation.
    pub fn is_notifying(&self, value_handle: u16) -> bool {
        let Some(i) = self.index(value_handle) else {
            return false;
        };
        let AttrKind::Value { properties } = self.attrs[i].kind else {
            return false;
        };
        let cccd = cccd_uuid();
        let bits = self.attrs[i + 1..]
            .iter()
            .take_while(|a| a.kind == AttrKind::Descriptor)
            .find(|a| a.uuid == cccd)
            .and_then(|a| if a.value.len() == 2 { Some(u16::from_le_bytes([a.value[0], a.value[1]])) } else { None })
            .unwrap_or(0);
        (properties.contains(CharProperties::NOTIFY) && bits & 0x0001 != 0)
            || (properties.contains(CharProperties::INDICATE) && bits & 0x0002 != 0)
    }

    fn index(&self, handle: u16) -> Option<usize> {
        self.attrs.binary_search_by_key(&handle, |a| a.handle).ok()
    }

    fn push(&mut self, uuid: Uuid, kind: AttrKind, perms: AttrPermissions, value: Vec<u8>) -> Result<u16, AttError> {
        if self.next_handle == u16::MAX {
            return Err(AttError::InvalidHandle);
        }
        let handle = self.next_handle;
        self.next_handle += 1;
        self.attrs.push(Attribute { handle, uuid, kind, perms, value });
        Ok(handle)
    }
}

fn cccd_uuid() -> Uuid {
    Uuid::from_16(CCCD_UUID)
}

/// The access rules for one attribute. Declarations are public and read-only. Values also need the matching
/// characteristic property. Every attribute needs its permission bit, and encrypted-only attributes need an encrypted
/// link.
fn gate(a: &Attribute, link_encrypted: bool, write: bool) -> Result<(), AttError> {
    let deny = if write { AttError::WriteNotPermitted } else { AttError::ReadNotPermitted };
    match a.kind {
        AttrKind::Service { .. } | AttrKind::Characteristic => {
            if write {
                Err(deny)
            } else {
                Ok(())
            }
        }
        AttrKind::Value { properties } => {
            let property_ok = if write {
                properties.contains(CharProperties::WRITE) || properties.contains(CharProperties::WRITE_WITHOUT_RESPONSE)
            } else {
                properties.contains(CharProperties::READ)
            };
            if !property_ok {
                return Err(deny);
            }
            permission_gate(a.perms, link_encrypted, write)
        }
        AttrKind::Descriptor => permission_gate(a.perms, link_encrypted, write),
    }
}

fn permission_gate(perms: AttrPermissions, link_encrypted: bool, write: bool) -> Result<(), AttError> {
    let (allowed, encrypted_only) = if write {
        (AttrPermissions::WRITEABLE, AttrPermissions::WRITE_ENCRYPTION_REQUIRED)
    } else {
        (AttrPermissions::READABLE, AttrPermissions::READ_ENCRYPTION_REQUIRED)
    };
    if !perms.contains(allowed) {
        return Err(if write { AttError::WriteNotPermitted } else { AttError::ReadNotPermitted });
    }
    if perms.contains(encrypted_only) && !link_encrypted {
        return Err(AttError::EncryptionRequired);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uuid(s: &str) -> Uuid {
        Uuid::parse(s).unwrap()
    }

    #[test]
    fn handles_are_assigned_in_order() {
        let mut s = GattServer::new();
        let svc = s.add_service(uuid("180D"), true).unwrap();
        assert_eq!(svc, 1);
        // The declaration takes handle 2 and the value handle 3.
        let v = s
            .add_characteristic(svc, uuid("2A37"), CharProperties::NOTIFY | CharProperties::READ, AttrPermissions::READABLE, vec![0, 72])
            .unwrap();
        assert_eq!(v, 3);
        let cccd = s
            .add_descriptor(v, Uuid::from_16(CCCD_UUID), AttrPermissions::READABLE | AttrPermissions::WRITEABLE, vec![0, 0])
            .unwrap();
        assert_eq!(cccd, 4);
        assert_eq!(s.add_service(uuid("180F"), true).unwrap(), 5);
        let handles: Vec<u16> = s.attributes().iter().map(|a| a.handle).collect();
        assert_eq!(handles, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn reads_follow_the_property_and_permission_bits() {
        let mut s = GattServer::new();
        let svc = s.add_service(uuid("180D"), true).unwrap();
        let readable = s
            .add_characteristic(svc, uuid("2A37"), CharProperties::READ, AttrPermissions::READABLE, vec![0, 72])
            .unwrap();
        let write_only = s
            .add_characteristic(svc, uuid("2A38"), CharProperties::WRITE, AttrPermissions::READABLE | AttrPermissions::WRITEABLE, vec![9])
            .unwrap();
        assert_eq!(s.read(readable, false), Ok(&[0u8, 72][..]));
        assert_eq!(s.read(write_only, false), Err(AttError::ReadNotPermitted));
        assert_eq!(s.read(99, false), Err(AttError::InvalidHandle));
        // The declaration of the first characteristic holds its property byte (READ = 0x02) and value handle 3.
        assert_eq!(s.read(2, false), Ok(&[0x02u8, 3, 0][..]));
    }

    #[test]
    fn encrypted_only_attributes_need_an_encrypted_link() {
        let mut s = GattServer::new();
        let svc = s.add_service(uuid("180D"), true).unwrap();
        let secret = s
            .add_characteristic(
                svc,
                uuid("2A37"),
                CharProperties::READ,
                AttrPermissions::READABLE | AttrPermissions::READ_ENCRYPTION_REQUIRED,
                vec![42],
            )
            .unwrap();
        assert_eq!(s.read(secret, false), Err(AttError::EncryptionRequired));
        assert_eq!(s.read(secret, true), Ok(&[42u8][..]));
    }

    #[test]
    fn writes_check_length_and_declarations() {
        let mut s = GattServer::new();
        let svc = s.add_service(uuid("180D"), true).unwrap();
        let v = s
            .add_characteristic(
                svc,
                uuid("2A38"),
                CharProperties::READ | CharProperties::WRITE,
                AttrPermissions::READABLE | AttrPermissions::WRITEABLE,
                vec![0],
            )
            .unwrap();
        s.write(v, &[1, 2, 3], false).unwrap();
        assert_eq!(s.value(v), Some(&[1u8, 2, 3][..]));
        assert_eq!(s.write(v, &[0u8; 513], false), Err(AttError::InvalidLength));
        s.write(v, &[7u8; 512], false).unwrap();
        assert_eq!(s.value(v).map(|b| b.len()), Some(512));
        // The declaration of a characteristic is never writable.
        assert_eq!(s.write(v - 1, &[1], false), Err(AttError::WriteNotPermitted));
        // A local update is not limited by the peer's permissions.
        s.set_value(v, &[5]).unwrap();
        assert_eq!(s.value(v), Some(&[5u8][..]));
    }

    #[test]
    fn notifications_follow_the_client_configuration_descriptor() {
        let mut s = GattServer::new();
        let svc = s.add_service(uuid("180D"), true).unwrap();
        let v = s
            .add_characteristic(svc, uuid("2A37"), CharProperties::NOTIFY | CharProperties::READ, AttrPermissions::READABLE, vec![0])
            .unwrap();
        let cccd = s
            .add_descriptor(v, Uuid::from_16(CCCD_UUID), AttrPermissions::READABLE | AttrPermissions::WRITEABLE, vec![0, 0])
            .unwrap();
        assert!(!s.is_notifying(v));
        s.write(cccd, &[1, 0], false).unwrap();
        assert!(s.is_notifying(v));
        // Indications are not allowed on a NOTIFY-only characteristic, even when the descriptor asks for them.
        s.write(cccd, &[2, 0], false).unwrap();
        assert!(!s.is_notifying(v));
        assert_eq!(s.write(cccd, &[1], false), Err(AttError::InvalidLength));
    }

    #[test]
    fn descriptors_cannot_attach_to_an_earlier_characteristic() {
        let mut s = GattServer::new();
        let svc = s.add_service(uuid("180D"), true).unwrap();
        let first = s
            .add_characteristic(svc, uuid("2A37"), CharProperties::READ, AttrPermissions::READABLE, vec![0])
            .unwrap();
        s.add_characteristic(svc, uuid("2A38"), CharProperties::READ, AttrPermissions::READABLE, vec![1])
            .unwrap();
        assert_eq!(
            s.add_descriptor(first, uuid("2901"), AttrPermissions::READABLE, vec![]),
            Err(AttError::InvalidHandle)
        );
        // A characteristic must sit under a service, not under another characteristic.
        assert_eq!(
            s.add_characteristic(first, uuid("2A39"), CharProperties::READ, AttrPermissions::READABLE, vec![]),
            Err(AttError::InvalidHandle)
        );
    }
}
