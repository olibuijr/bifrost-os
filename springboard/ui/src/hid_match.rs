//! HID device matching: the criteria that filter the connected human interface devices, the device model the criteria
//! read, the usage-to-role table, and the difference between two snapshots of the connected devices.
//!
//! Pure: the caller (the shell's input scan) supplies the device snapshots. The model mirrors the platform HID device
//! manager: a list of matching criteria selects devices, and each criteria set is an AND of its fields. The list is
//! an OR over criteria sets. Two rules are BifrOSt's own choice: a device has a usage when it is the primary usage or
//! one of its device usages, and an empty criteria list matches no device.

/// A HID usage: the usage page and the usage ID within that page.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Usage {
    pub page: u16,
    pub usage: u16,
}

impl Usage {
    /// Generic Desktop, Pointer.
    pub const POINTER: Usage = Usage { page: 0x01, usage: 0x01 };
    /// Generic Desktop, Mouse.
    pub const MOUSE: Usage = Usage { page: 0x01, usage: 0x02 };
    /// Generic Desktop, Joystick.
    pub const JOYSTICK: Usage = Usage { page: 0x01, usage: 0x04 };
    /// Generic Desktop, Gamepad.
    pub const GAMEPAD: Usage = Usage { page: 0x01, usage: 0x05 };
    /// Generic Desktop, Keyboard.
    pub const KEYBOARD: Usage = Usage { page: 0x01, usage: 0x06 };
    /// Generic Desktop, Keypad.
    pub const KEYPAD: Usage = Usage { page: 0x01, usage: 0x07 };
    /// Generic Desktop, System Control.
    pub const SYSTEM_CONTROL: Usage = Usage { page: 0x01, usage: 0x80 };
    /// Consumer, Consumer Control.
    pub const CONSUMER_CONTROL: Usage = Usage { page: 0x0C, usage: 0x01 };
    /// Digitizers, Touch Screen.
    pub const TOUCH_SCREEN: Usage = Usage { page: 0x0D, usage: 0x04 };
    /// Digitizers, Touch Pad.
    pub const TOUCH_PAD: Usage = Usage { page: 0x0D, usage: 0x05 };

    /// The role this usage gives a device, if the table knows it.
    pub fn role(self) -> Option<Role> {
        match (self.page, self.usage) {
            (0x01, 0x01) => Some(Role::Pointer),
            (0x01, 0x02) => Some(Role::Mouse),
            (0x01, 0x04) => Some(Role::Joystick),
            (0x01, 0x05) => Some(Role::Gamepad),
            (0x01, 0x06) => Some(Role::Keyboard),
            (0x01, 0x07) => Some(Role::Keypad),
            (0x01, 0x80) => Some(Role::SystemControl),
            (0x0C, 0x01) => Some(Role::Consumer),
            (0x0D, 0x04) => Some(Role::TouchScreen),
            (0x0D, 0x05) => Some(Role::TouchPad),
            _ => None,
        }
    }
}

/// What a device is for, as the usage table classifies it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Pointer,
    Mouse,
    Joystick,
    Gamepad,
    Keyboard,
    Keypad,
    SystemControl,
    Consumer,
    TouchScreen,
    TouchPad,
}

/// The data transport of a device. The variants mirror the framework's transport cases.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Transport {
    Aid,
    Streaming,
    Bluetooth,
    BluetoothAccessory,
    BluetoothLowEnergy,
    Fifo,
    I2c,
    Iap,
    InductiveInBand,
    Serial,
    Spi,
    Spu,
    Usb,
    Virtual,
    /// A transport name this table does not list, kept as written.
    Unknown(String),
}

impl Transport {
    /// The lower-camel name of the transport.
    pub fn name(&self) -> &str {
        match self {
            Transport::Aid => "aid",
            Transport::Streaming => "streaming",
            Transport::Bluetooth => "bluetooth",
            Transport::BluetoothAccessory => "bluetoothAccessory",
            Transport::BluetoothLowEnergy => "bluetoothLowEnergy",
            Transport::Fifo => "fifo",
            Transport::I2c => "i2c",
            Transport::Iap => "iap",
            Transport::InductiveInBand => "inductiveInBand",
            Transport::Serial => "serial",
            Transport::Spi => "spi",
            Transport::Spu => "spu",
            Transport::Usb => "usb",
            Transport::Virtual => "virtual",
            Transport::Unknown(name) => name.as_str(),
        }
    }

    /// The transport for a name. A name the table does not list becomes `Unknown`.
    pub fn from_name(name: &str) -> Transport {
        match name {
            "aid" => Transport::Aid,
            "streaming" => Transport::Streaming,
            "bluetooth" => Transport::Bluetooth,
            "bluetoothAccessory" => Transport::BluetoothAccessory,
            "bluetoothLowEnergy" => Transport::BluetoothLowEnergy,
            "fifo" => Transport::Fifo,
            "i2c" => Transport::I2c,
            "iap" => Transport::Iap,
            "inductiveInBand" => Transport::InductiveInBand,
            "serial" => Transport::Serial,
            "spi" => Transport::Spi,
            "spu" => Transport::Spu,
            "usb" => Transport::Usb,
            "virtual" => Transport::Virtual,
            other => Transport::Unknown(other.to_owned()),
        }
    }
}

/// One connected HID device, as the system reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    /// Stable for the life of the connection; the key that snapshots are compared by.
    pub unique_id: String,
    pub product: String,
    pub manufacturer: String,
    pub serial: String,
    pub vendor_id: u32,
    pub product_id: u32,
    pub version: u32,
    pub transport: Transport,
    pub location_id: u32,
    pub built_in: bool,
    pub primary_usage: Usage,
    pub device_usages: Vec<Usage>,
}

impl Device {
    /// Whether the device has `usage`, as its primary usage or one of its device usages.
    pub fn has_usage(&self, usage: Usage) -> bool {
        self.primary_usage == usage || self.device_usages.contains(&usage)
    }

    /// Whether the device has a usage that gives it `role`.
    pub fn has_role(&self, role: Role) -> bool {
        self.primary_usage.role() == Some(role) || self.device_usages.iter().any(|u| u.role() == Some(role))
    }
}

/// One matching criteria set. Every field that is set must match; a field left as `None` (or an empty list) matches
/// any device.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Criteria {
    pub primary_usage: Option<Usage>,
    /// Every usage listed must be present on the device.
    pub device_usages: Vec<Usage>,
    pub vendor_id: Option<u32>,
    pub product_id: Option<u32>,
    pub transport: Option<Transport>,
    pub product: Option<String>,
    pub manufacturer: Option<String>,
    pub serial: Option<String>,
    pub unique_id: Option<String>,
    pub location_id: Option<u32>,
    pub version: Option<u32>,
    pub built_in: Option<bool>,
}

fn text_matches(want: &Option<String>, have: &str) -> bool {
    match want {
        Some(w) => w == have,
        None => true,
    }
}

impl Criteria {
    /// Whether `device` passes every field this set sets.
    pub fn matches(&self, device: &Device) -> bool {
        let usage_ok = match self.primary_usage {
            Some(u) => device.primary_usage == u,
            None => true,
        };
        usage_ok
            && self.device_usages.iter().all(|u| device.has_usage(*u))
            && self.vendor_id.map_or(true, |v| device.vendor_id == v)
            && self.product_id.map_or(true, |p| device.product_id == p)
            && self.transport.as_ref().map_or(true, |t| *t == device.transport)
            && text_matches(&self.product, &device.product)
            && text_matches(&self.manufacturer, &device.manufacturer)
            && text_matches(&self.serial, &device.serial)
            && text_matches(&self.unique_id, &device.unique_id)
            && self.location_id.map_or(true, |l| device.location_id == l)
            && self.version.map_or(true, |v| device.version == v)
            && self.built_in.map_or(true, |b| device.built_in == b)
    }
}

/// The devices that match any criteria set in `criteria`, in the order of `devices`. An empty list matches nothing.
pub fn matching<'a>(criteria: &[Criteria], devices: &'a [Device]) -> Vec<&'a Device> {
    devices.iter().filter(|d| criteria.iter().any(|c| c.matches(d))).collect()
}

/// The difference between two snapshots, for the devices the criteria select.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Changes<'a> {
    /// Devices that match now and did not match in the earlier snapshot.
    pub matched: Vec<&'a Device>,
    /// Devices that matched in the earlier snapshot and are gone now.
    pub removed: Vec<&'a Device>,
}

/// Compares two snapshots by `unique_id`. A device whose attributes change but whose ID stays is not reported.
pub fn changes<'a>(criteria: &[Criteria], before: &'a [Device], after: &'a [Device]) -> Changes<'a> {
    let was = matching(criteria, before);
    let now = matching(criteria, after);
    let matched = now
        .iter()
        .filter(|d| !was.iter().any(|w| w.unique_id == d.unique_id))
        .copied()
        .collect();
    let removed = was
        .iter()
        .filter(|w| !now.iter().any(|d| d.unique_id == w.unique_id))
        .copied()
        .collect();
    Changes { matched, removed }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(id: &str, primary: Usage, transport: Transport, vendor: u32, product: u32) -> Device {
        Device {
            unique_id: id.to_owned(),
            product: format!("product {id}"),
            manufacturer: "maker".to_owned(),
            serial: String::new(),
            vendor_id: vendor,
            product_id: product,
            version: 1,
            transport,
            location_id: 0x1400_0000,
            built_in: false,
            primary_usage: primary,
            device_usages: vec![primary],
        }
    }

    #[test]
    fn usage_table_classifies_known_pages_only() {
        assert_eq!(Usage::KEYBOARD.role(), Some(Role::Keyboard));
        assert_eq!(Usage::TOUCH_SCREEN.role(), Some(Role::TouchScreen));
        assert_eq!(Usage::CONSUMER_CONTROL.role(), Some(Role::Consumer));
        assert_eq!(Usage { page: 0x0F, usage: 0x01 }.role(), None);
        assert_eq!(Usage { page: 0x01, usage: 0x03 }.role(), None);
    }

    #[test]
    fn transport_names_round_trip_and_keep_unknown_names() {
        assert_eq!(Transport::from_name("usb"), Transport::Usb);
        assert_eq!(Transport::Usb.name(), "usb");
        assert_eq!(Transport::from_name("bluetoothLowEnergy").name(), "bluetoothLowEnergy");
        assert_eq!(Transport::from_name("nonsense"), Transport::Unknown("nonsense".to_owned()));
        assert_eq!(Transport::from_name("nonsense").name(), "nonsense");
    }

    #[test]
    fn fields_in_one_criteria_set_must_all_match() {
        let mouse = device("m1", Usage::MOUSE, Transport::Usb, 0x046d, 0xc077);
        let by_ids = Criteria { vendor_id: Some(0x046d), product_id: Some(0xc077), ..Default::default() };
        assert!(by_ids.matches(&mouse));
        let wrong_product = Criteria { vendor_id: Some(0x046d), product_id: Some(0xc078), ..Default::default() };
        assert!(!wrong_product.matches(&mouse));
        let empty = Criteria::default();
        assert!(empty.matches(&mouse));
    }

    #[test]
    fn every_listed_device_usage_must_be_present() {
        let keyboard = device("k1", Usage::KEYBOARD, Transport::Usb, 1, 1);
        let mut both = keyboard.clone();
        both.device_usages = vec![Usage::KEYBOARD, Usage::CONSUMER_CONTROL];
        let want_both = Criteria { device_usages: vec![Usage::KEYBOARD, Usage::CONSUMER_CONTROL], ..Default::default() };
        assert!(!want_both.matches(&keyboard));
        assert!(want_both.matches(&both));
        // The primary usage counts as present even when the device usage list omits it.
        let primary_only = Criteria { device_usages: vec![Usage::KEYBOARD], ..Default::default() };
        assert!(primary_only.matches(&keyboard));
    }

    #[test]
    fn criteria_list_is_an_or_and_an_empty_list_matches_nothing() {
        let kbd = device("k1", Usage::KEYBOARD, Transport::Usb, 1, 1);
        let bt_mouse = device("m2", Usage::MOUSE, Transport::Bluetooth, 2, 2);
        let usb_mouse = device("m3", Usage::MOUSE, Transport::Usb, 3, 3);
        let devices = vec![kbd, bt_mouse, usb_mouse];
        let list = vec![
            Criteria { primary_usage: Some(Usage::KEYBOARD), ..Default::default() },
            Criteria { transport: Some(Transport::Bluetooth), ..Default::default() },
        ];
        let got: Vec<&str> = matching(&list, &devices).iter().map(|d| d.unique_id.as_str()).collect();
        assert_eq!(got, vec!["k1", "m2"]);
        assert!(matching(&[], &devices).is_empty());
    }

    #[test]
    fn changes_report_new_matches_and_removed_matches_only() {
        let criteria = vec![Criteria { primary_usage: Some(Usage::MOUSE), ..Default::default() }];
        let before = vec![
            device("m-old", Usage::MOUSE, Transport::Usb, 1, 1),
            device("k1", Usage::KEYBOARD, Transport::Usb, 2, 2),
        ];
        let after = vec![
            device("m-new", Usage::MOUSE, Transport::Usb, 3, 3),
            device("k1", Usage::KEYBOARD, Transport::Usb, 2, 2),
        ];
        let c = changes(&criteria, &before, &after);
        let matched: Vec<&str> = c.matched.iter().map(|d| d.unique_id.as_str()).collect();
        let removed: Vec<&str> = c.removed.iter().map(|d| d.unique_id.as_str()).collect();
        assert_eq!(matched, vec!["m-new"]);
        assert_eq!(removed, vec!["m-old"]);
    }

    #[test]
    fn role_from_secondary_usage_and_built_in_filter() {
        let mut pad = device("p1", Usage::POINTER, Transport::Serial, 9, 9);
        pad.device_usages = vec![Usage::POINTER, Usage::KEYBOARD];
        assert!(pad.has_role(Role::Keyboard));
        assert!(pad.has_role(Role::Pointer));
        assert!(!pad.has_role(Role::Gamepad));
        pad.built_in = true;
        let external_only = Criteria { built_in: Some(false), ..Default::default() };
        assert!(!external_only.matches(&pad));
    }
}
