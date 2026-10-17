//! Property model for a camera device. A device exposes named properties, each addressed by a selector, a scope and an
//! element, the way a media device framework lists them. Readers get a value, writers set one, and every real change is
//! queued for the listeners. Only the properties a device has are answered: an unknown address is refused, never guessed.
//! The identity fields (name, maker, unique id, transport) and the stream count are read-only; only `RUNNING` (start or
//! stop the capture) can be written. The model does not open a camera; the capture code reads the device's state from it.

/// The device's display name. Read-only text.
pub const NAME: u32 = 1;
/// The maker of the device. Read-only text.
pub const MANUFACTURER: u32 = 2;
/// The stable unique id of the device. Read-only text.
pub const UID: u32 = 3;
/// How the device is attached, as text: "usb", "network" or "other". Read-only.
pub const TRANSPORT: u32 = 4;
/// Whether capture is running. The one writable property: a flag.
pub const RUNNING: u32 = 5;
/// The number of input streams the device has. Read-only number.
pub const STREAM_COUNT: u32 = 6;

/// The part of the device a property belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Scope {
    /// The device as a whole.
    Global,
    /// The device's capture side.
    Input,
}

/// Where a property sits: its selector, the scope it belongs to, and the element (channel) within that scope.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Address {
    pub selector: u32,
    pub scope: Scope,
    pub element: u32,
}

impl Address {
    /// A global-scope address on element 0, which is where the device's own properties live.
    pub fn global(selector: u32) -> Self {
        Address { selector, scope: Scope::Global, element: 0 }
    }
}

/// A property value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Text(String),
    Number(u32),
    Flag(bool),
}

/// Why a property call was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PropertyError {
    /// The device has no property at that address.
    Unknown,
    /// The property cannot be written.
    ReadOnly,
    /// The value is not the kind the property holds (text, number or flag).
    WrongType,
    /// A text identity is empty.
    Empty,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Entry {
    address: Address,
    value: Value,
    writable: bool,
}

/// The properties of one camera device, with the changes not yet taken by the listeners.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CameraProperties {
    entries: Vec<Entry>,
    changes: Vec<(Address, Value)>,
}

impl CameraProperties {
    /// The properties of a device with the given identity, stopped, with no input streams. A name, a unique id and a
    /// transport must not be empty; the maker may be empty.
    pub fn new(name: &str, manufacturer: &str, uid: &str, transport: &str) -> Result<Self, PropertyError> {
        if name.trim().is_empty() || uid.trim().is_empty() || transport.trim().is_empty() {
            return Err(PropertyError::Empty);
        }
        let entries = vec![
            Entry { address: Address::global(NAME), value: Value::Text(name.to_string()), writable: false },
            Entry { address: Address::global(MANUFACTURER), value: Value::Text(manufacturer.to_string()), writable: false },
            Entry { address: Address::global(UID), value: Value::Text(uid.to_string()), writable: false },
            Entry { address: Address::global(TRANSPORT), value: Value::Text(transport.to_string()), writable: false },
            Entry { address: Address::global(RUNNING), value: Value::Flag(false), writable: true },
            Entry { address: Address { selector: STREAM_COUNT, scope: Scope::Input, element: 0 }, value: Value::Number(0), writable: false },
        ];
        Ok(CameraProperties { entries, changes: Vec::new() })
    }

    /// The addresses of every property the device has, in the order they were added.
    pub fn addresses(&self) -> Vec<Address> {
        self.entries.iter().map(|e| e.address).collect()
    }

    /// The value at `address`, or `Unknown` when the device has no such property.
    pub fn get(&self, address: Address) -> Result<&Value, PropertyError> {
        self.entry(address).map(|e| &e.value).ok_or(PropertyError::Unknown)
    }

    /// Writes a property. A value equal to the current one is accepted and queues no change. Read-only properties
    /// and values of the wrong kind are refused.
    pub fn set(&mut self, address: Address, value: Value) -> Result<(), PropertyError> {
        let entry = self.entries.iter_mut().find(|e| e.address == address).ok_or(PropertyError::Unknown)?;
        if !entry.writable {
            return Err(PropertyError::ReadOnly);
        }
        if std::mem::discriminant(&entry.value) != std::mem::discriminant(&value) {
            return Err(PropertyError::WrongType);
        }
        if entry.value != value {
            entry.value = value.clone();
            self.changes.push((address, value));
        }
        Ok(())
    }

    /// Records how many input streams the device has. The count is set by the capture code, which knows the device.
    pub fn set_stream_count(&mut self, count: u32) {
        let address = Address { selector: STREAM_COUNT, scope: Scope::Input, element: 0 };
        if let Some(entry) = self.entries.iter_mut().find(|e| e.address == address) {
            entry.value = Value::Number(count);
        }
    }

    /// The changes made since the last call, oldest first. The listeners read them and the queue is cleared.
    pub fn take_changes(&mut self) -> Vec<(Address, Value)> {
        std::mem::take(&mut self.changes)
    }

    fn entry(&self, address: Address) -> Option<&Entry> {
        self.entries.iter().find(|e| e.address == address)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn camera() -> CameraProperties {
        CameraProperties::new("Webcam", "Acme", "cam-0001", "usb").unwrap()
    }

    #[test]
    fn identity_is_read_back_as_text() {
        let cam = camera();
        assert_eq!(cam.get(Address::global(NAME)), Ok(&Value::Text("Webcam".to_string())));
        assert_eq!(cam.get(Address::global(MANUFACTURER)), Ok(&Value::Text("Acme".to_string())));
        assert_eq!(cam.get(Address::global(UID)), Ok(&Value::Text("cam-0001".to_string())));
        assert_eq!(cam.get(Address::global(TRANSPORT)), Ok(&Value::Text("usb".to_string())));
    }

    #[test]
    fn a_new_device_is_stopped_with_no_streams() {
        let mut cam = camera();
        assert_eq!(cam.get(Address::global(RUNNING)), Ok(&Value::Flag(false)));
        assert_eq!(cam.get(Address { selector: STREAM_COUNT, scope: Scope::Input, element: 0 }), Ok(&Value::Number(0)));
        cam.set_stream_count(2);
        assert_eq!(cam.get(Address { selector: STREAM_COUNT, scope: Scope::Input, element: 0 }), Ok(&Value::Number(2)));
    }

    #[test]
    fn an_empty_identity_is_refused() {
        assert_eq!(CameraProperties::new("", "Acme", "cam", "usb"), Err(PropertyError::Empty));
        assert_eq!(CameraProperties::new("Cam", "Acme", "  ", "usb"), Err(PropertyError::Empty));
        assert!(CameraProperties::new("Cam", "", "cam", "usb").is_ok(), "the maker may be empty");
    }

    #[test]
    fn only_running_can_be_written_and_each_change_is_queued_once() {
        let mut cam = camera();
        assert_eq!(cam.set(Address::global(NAME), Value::Text("Other".into())), Err(PropertyError::ReadOnly));
        cam.set(Address::global(RUNNING), Value::Flag(true)).unwrap();
        cam.set(Address::global(RUNNING), Value::Flag(true)).unwrap();
        let changes = cam.take_changes();
        assert_eq!(changes, vec![(Address::global(RUNNING), Value::Flag(true))]);
        assert!(cam.take_changes().is_empty());
    }

    #[test]
    fn a_value_of_the_wrong_kind_is_refused() {
        let mut cam = camera();
        assert_eq!(cam.set(Address::global(RUNNING), Value::Number(1)), Err(PropertyError::WrongType));
        assert_eq!(cam.get(Address::global(RUNNING)), Ok(&Value::Flag(false)));
    }

    #[test]
    fn an_address_the_device_does_not_have_is_unknown() {
        let mut cam = camera();
        let missing = Address::global(99);
        assert_eq!(cam.get(missing), Err(PropertyError::Unknown));
        assert_eq!(cam.set(missing, Value::Flag(true)), Err(PropertyError::Unknown));
        assert_eq!(cam.get(Address { selector: RUNNING, scope: Scope::Input, element: 0 }), Err(PropertyError::Unknown));
    }

    #[test]
    fn addresses_list_every_property_in_order() {
        let cam = camera();
        let selectors: Vec<u32> = cam.addresses().iter().map(|a| a.selector).collect();
        assert_eq!(selectors, vec![NAME, MANUFACTURER, UID, TRANSPORT, RUNNING, STREAM_COUNT]);
    }
}
