//! USB host model: standard descriptors, control requests, and the criteria that pick a device or an interface.
//!
//! Pure: no device is opened here. The caller reads the raw descriptor bytes (for example the device descriptor and
//! the full configuration block with `wTotalLength` bytes) and passes them in, and it sends the encoded control
//! requests itself. Field names follow the USB 2.0 and 3.2 specifications, which the official host framework
//! documentation also cites.

/// `bDescriptorType` values from the USB specification.
pub const DESC_DEVICE: u8 = 1;
pub const DESC_CONFIGURATION: u8 = 2;
pub const DESC_INTERFACE: u8 = 4;
pub const DESC_ENDPOINT: u8 = 5;

/// `bRequest` values for standard device requests.
pub mod request {
    pub const GET_STATUS: u8 = 0;
    pub const CLEAR_FEATURE: u8 = 1;
    pub const SET_FEATURE: u8 = 3;
    pub const SET_ADDRESS: u8 = 5;
    pub const GET_DESCRIPTOR: u8 = 6;
    pub const GET_CONFIGURATION: u8 = 8;
    pub const SET_CONFIGURATION: u8 = 9;
    pub const GET_INTERFACE: u8 = 10;
    pub const SET_INTERFACE: u8 = 11;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParseError {
    /// The bytes end before the descriptor they declare.
    Truncated,
    /// A descriptor declares a length that does not fit its type.
    BadLength,
    /// The first descriptor is not the type the parser expects.
    WrongType,
    /// An endpoint descriptor appears before any interface descriptor.
    Orphan,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeviceDescriptor {
    /// `bcdUSB`, for example `0x0200` for USB 2.0.
    pub usb_version: u16,
    pub class: u8,
    pub subclass: u8,
    pub protocol: u8,
    /// `bMaxPacketSize0`: the size of the control endpoint's packets.
    pub max_packet_size0: u8,
    pub vendor_id: u16,
    pub product_id: u16,
    /// `bcdDevice`: the device release number.
    pub device_version: u16,
    /// `bNumConfigurations`.
    pub configurations: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferType {
    Control,
    Isochronous,
    Bulk,
    Interrupt,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EndpointDescriptor {
    /// `bEndpointAddress`: the number in the low nibble, direction in bit 7.
    pub address: u8,
    /// `bmAttributes`: the transfer type in the low two bits.
    pub attributes: u8,
    /// `wMaxPacketSize`.
    pub max_packet_size: u16,
    /// `bInterval`: polling interval for interrupt and isochronous endpoints.
    pub interval: u8,
}

impl EndpointDescriptor {
    pub fn number(&self) -> u8 {
        self.address & 0x0f
    }

    /// Device-to-host (IN) when bit 7 of the address is set.
    pub fn is_in(&self) -> bool {
        self.address & 0x80 != 0
    }

    pub fn transfer_type(&self) -> TransferType {
        match self.attributes & 0x03 {
            0 => TransferType::Control,
            1 => TransferType::Isochronous,
            2 => TransferType::Bulk,
            _ => TransferType::Interrupt,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InterfaceDescriptor {
    /// `bInterfaceNumber`.
    pub number: u8,
    /// `bAlternateSetting`.
    pub alternate: u8,
    pub class: u8,
    pub subclass: u8,
    pub protocol: u8,
    pub endpoints: Vec<EndpointDescriptor>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigurationDescriptor {
    /// `bConfigurationValue`: the number `SET_CONFIGURATION` takes.
    pub value: u8,
    /// `bmAttributes`.
    pub attributes: u8,
    /// `bMaxPower` in milliamps (the field counts 2 mA units).
    pub max_power_ma: u16,
    pub interfaces: Vec<InterfaceDescriptor>,
}

impl ConfigurationDescriptor {
    /// The interface with this number and alternate setting, if the configuration has it.
    pub fn interface(&self, number: u8, alternate: u8) -> Option<&InterfaceDescriptor> {
        self.interfaces.iter().find(|i| i.number == number && i.alternate == alternate)
    }
}

fn u16_at(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

/// Parses the 18-byte device descriptor.
pub fn parse_device(bytes: &[u8]) -> Result<DeviceDescriptor, ParseError> {
    if bytes.len() < 18 {
        return Err(ParseError::Truncated);
    }
    if bytes[1] != DESC_DEVICE {
        return Err(ParseError::WrongType);
    }
    if bytes[0] != 18 {
        return Err(ParseError::BadLength);
    }
    Ok(DeviceDescriptor {
        usb_version: u16_at(bytes, 2),
        class: bytes[4],
        subclass: bytes[5],
        protocol: bytes[6],
        max_packet_size0: bytes[7],
        vendor_id: u16_at(bytes, 8),
        product_id: u16_at(bytes, 10),
        device_version: u16_at(bytes, 12),
        configurations: bytes[17],
    })
}

/// Parses a configuration block: the 9-byte configuration descriptor followed by its interface and endpoint
/// descriptors. The block must hold the `wTotalLength` bytes the configuration header declares. Descriptors of other
/// types (class-specific, for example) are skipped by their length.
pub fn parse_configuration(bytes: &[u8]) -> Result<ConfigurationDescriptor, ParseError> {
    if bytes.len() < 9 {
        return Err(ParseError::Truncated);
    }
    if bytes[1] != DESC_CONFIGURATION {
        return Err(ParseError::WrongType);
    }
    if bytes[0] != 9 {
        return Err(ParseError::BadLength);
    }
    let total = usize::from(u16_at(bytes, 2));
    if total < 9 || bytes.len() < total {
        return Err(ParseError::Truncated);
    }
    let block = &bytes[..total];
    let mut config = ConfigurationDescriptor {
        value: block[5],
        attributes: block[7],
        max_power_ma: u16::from(block[8]) * 2,
        interfaces: Vec::new(),
    };
    let mut at = usize::from(block[0]);
    while at < total {
        if at + 2 > total {
            return Err(ParseError::Truncated);
        }
        let len = usize::from(block[at]);
        if len < 2 {
            return Err(ParseError::BadLength);
        }
        if at + len > total {
            return Err(ParseError::Truncated);
        }
        let d = &block[at..at + len];
        match d[1] {
            DESC_INTERFACE => {
                if len < 9 {
                    return Err(ParseError::BadLength);
                }
                config.interfaces.push(InterfaceDescriptor {
                    number: d[2],
                    alternate: d[3],
                    class: d[5],
                    subclass: d[6],
                    protocol: d[7],
                    endpoints: Vec::new(),
                });
            }
            DESC_ENDPOINT => {
                if len < 7 {
                    return Err(ParseError::BadLength);
                }
                let iface = config.interfaces.last_mut().ok_or(ParseError::Orphan)?;
                iface.endpoints.push(EndpointDescriptor {
                    address: d[2],
                    attributes: d[3],
                    max_packet_size: u16_at(d, 4),
                    interval: d[6],
                });
            }
            _ => {}
        }
        at += len;
    }
    Ok(config)
}

/// Data direction of a control request, bit 7 of `bmRequestType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    HostToDevice,
    DeviceToHost,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestKind {
    Standard,
    Class,
    Vendor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Recipient {
    Device,
    Interface,
    Endpoint,
}

/// An 8-byte control setup packet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SetupPacket {
    pub direction: Direction,
    pub kind: RequestKind,
    pub recipient: Recipient,
    pub request: u8,
    pub value: u16,
    pub index: u16,
    pub length: u16,
}

impl SetupPacket {
    /// The `bmRequestType` byte.
    pub fn request_type(&self) -> u8 {
        let dir = match self.direction {
            Direction::HostToDevice => 0,
            Direction::DeviceToHost => 0x80,
        };
        let kind = match self.kind {
            RequestKind::Standard => 0,
            RequestKind::Class => 0x20,
            RequestKind::Vendor => 0x40,
        };
        let recipient = match self.recipient {
            Recipient::Device => 0,
            Recipient::Interface => 1,
            Recipient::Endpoint => 2,
        };
        dir | kind | recipient
    }

    /// The packet in wire order: request type, request, value, index, length, all little endian.
    pub fn encode(&self) -> [u8; 8] {
        let v = self.value.to_le_bytes();
        let i = self.index.to_le_bytes();
        let l = self.length.to_le_bytes();
        [self.request_type(), self.request, v[0], v[1], i[0], i[1], l[0], l[1]]
    }

    /// Reads a setup packet back from its wire form.
    pub fn decode(bytes: &[u8; 8]) -> SetupPacket {
        let rt = bytes[0];
        let direction = if rt & 0x80 != 0 { Direction::DeviceToHost } else { Direction::HostToDevice };
        let kind = match (rt >> 5) & 0x03 {
            1 => RequestKind::Class,
            2 => RequestKind::Vendor,
            _ => RequestKind::Standard,
        };
        let recipient = match rt & 0x1f {
            1 => Recipient::Interface,
            2 => Recipient::Endpoint,
            _ => Recipient::Device,
        };
        SetupPacket {
            direction,
            kind,
            recipient,
            request: bytes[1],
            value: u16_at(bytes, 2),
            index: u16_at(bytes, 4),
            length: u16_at(bytes, 6),
        }
    }
}

/// `GET_DESCRIPTOR` for a device-recipient standard request. `desc_type` is one of the `DESC_*` values, `index`
/// selects among descriptors of that type, and `language` is used for string descriptors only.
pub fn get_descriptor(desc_type: u8, index: u8, language: u16, length: u16) -> SetupPacket {
    SetupPacket {
        direction: Direction::DeviceToHost,
        kind: RequestKind::Standard,
        recipient: Recipient::Device,
        request: request::GET_DESCRIPTOR,
        value: (u16::from(desc_type) << 8) | u16::from(index),
        index: language,
        length,
    }
}

/// `SET_CONFIGURATION` to the given `bConfigurationValue`; 0 returns the device to its unconfigured state.
pub fn set_configuration(value: u8) -> SetupPacket {
    SetupPacket {
        direction: Direction::HostToDevice,
        kind: RequestKind::Standard,
        recipient: Recipient::Device,
        request: request::SET_CONFIGURATION,
        value: u16::from(value),
        index: 0,
        length: 0,
    }
}

/// `SET_INTERFACE`, which selects an alternate setting of an interface.
pub fn set_interface(number: u8, alternate: u8) -> SetupPacket {
    SetupPacket {
        direction: Direction::HostToDevice,
        kind: RequestKind::Standard,
        recipient: Recipient::Interface,
        request: request::SET_INTERFACE,
        value: u16::from(alternate),
        index: u16::from(number),
        length: 0,
    }
}

/// Whether the match needs every criterion set (`MatchAll`) or at least one (`MatchAny`), used when a device has
/// several interfaces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InterfaceMatchOption {
    MatchAll,
    MatchAny,
}

/// Device fields to match. A `None` field matches any value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DeviceMatch {
    pub vendor_id: Option<u16>,
    pub product_id: Option<u16>,
    pub class: Option<u8>,
    pub subclass: Option<u8>,
    pub protocol: Option<u8>,
}

impl DeviceMatch {
    pub fn matches(&self, d: &DeviceDescriptor) -> bool {
        self.vendor_id.is_none_or( |v| v == d.vendor_id)
            && self.product_id.is_none_or( |p| p == d.product_id)
            && self.class.is_none_or( |c| c == d.class)
            && self.subclass.is_none_or( |s| s == d.subclass)
            && self.protocol.is_none_or( |p| p == d.protocol)
    }
}

/// Interface fields to match. A `None` field matches any value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InterfaceMatch {
    pub class: Option<u8>,
    pub subclass: Option<u8>,
    pub protocol: Option<u8>,
}

impl InterfaceMatch {
    pub fn matches(&self, i: &InterfaceDescriptor) -> bool {
        self.class.is_none_or( |c| c == i.class)
            && self.subclass.is_none_or( |s| s == i.subclass)
            && self.protocol.is_none_or( |p| p == i.protocol)
    }
}

/// Accessory matching: a device must match `device`. If `interfaces` is empty the device match alone decides.
/// Otherwise the configuration's interfaces are checked against the list: with `MatchAny` one interface must match
/// one of the criteria, with `MatchAll` every criterion must match some interface.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MatchingCriteria {
    pub device: DeviceMatch,
    pub interfaces: Vec<InterfaceMatch>,
    pub option: Option<InterfaceMatchOption>,
}

impl MatchingCriteria {
    pub fn matches(&self, device: &DeviceDescriptor, config: &ConfigurationDescriptor) -> bool {
        if !self.device.matches(device) {
            return false;
        }
        if self.interfaces.is_empty() {
            return true;
        }
        match self.option.unwrap_or(InterfaceMatchOption::MatchAny) {
            InterfaceMatchOption::MatchAny => self
                .interfaces
                .iter()
                .any(|want| config.interfaces.iter().any(|i| want.matches(i))),
            InterfaceMatchOption::MatchAll => self
                .interfaces
                .iter()
                .all(|want| config.interfaces.iter().any(|i| want.matches(i))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device_bytes(vid: u16, pid: u16, class: u8) -> Vec<u8> {
        let mut b = vec![18, DESC_DEVICE, 0x00, 0x02, class, 0, 0, 64];
        b.extend_from_slice(&vid.to_le_bytes());
        b.extend_from_slice(&pid.to_le_bytes());
        b.extend_from_slice(&[0x00, 0x01, 0, 0, 0, 1]);
        b
    }

    /// One configuration with a HID-class interface (class 3) and a bulk-out and an interrupt-in endpoint, plus a
    /// class-specific descriptor that the parser must skip.
    fn config_bytes() -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(&[9, DESC_INTERFACE, 0, 0, 2, 3, 1, 1, 0]);
        body.extend_from_slice(&[3, 0x21, 0x11]); // class-specific, skipped
        body.extend_from_slice(&[7, DESC_ENDPOINT, 0x81, 0x03, 8, 0, 10]);
        body.extend_from_slice(&[9, DESC_INTERFACE, 1, 0, 1, 0xff, 0, 0, 0]);
        body.extend_from_slice(&[7, DESC_ENDPOINT, 0x02, 0x02, 0, 2, 0]);
        let total = (9 + body.len()) as u16;
        let mut b = vec![9, DESC_CONFIGURATION];
        b.extend_from_slice(&total.to_le_bytes());
        b.extend_from_slice(&[2, 1, 0, 0xa0, 50]);
        b.extend_from_slice(&body);
        b
    }

    #[test]
    fn parses_the_device_descriptor_fields() {
        let d = parse_device(&device_bytes(0x046d, 0xc08b, 0)).unwrap();
        assert_eq!(d.vendor_id, 0x046d);
        assert_eq!(d.product_id, 0xc08b);
        assert_eq!(d.usb_version, 0x0200);
        assert_eq!(d.max_packet_size0, 64);
        assert_eq!(d.configurations, 1);
        assert_eq!(parse_device(&device_bytes(1, 2, 0)[..10]), Err(ParseError::Truncated));
    }

    #[test]
    fn walks_interfaces_and_endpoints_and_skips_class_descriptors() {
        let c = parse_configuration(&config_bytes()).unwrap();
        assert_eq!(c.value, 1);
        assert_eq!(c.max_power_ma, 100);
        assert_eq!(c.interfaces.len(), 2);
        let hid = c.interface(0, 0).unwrap();
        assert_eq!(hid.class, 3);
        assert_eq!(hid.endpoints.len(), 1);
        let ep = hid.endpoints[0];
        assert!(ep.is_in());
        assert_eq!(ep.number(), 1);
        assert_eq!(ep.transfer_type(), TransferType::Interrupt);
        assert_eq!(ep.max_packet_size, 8);
        let vendor = c.interface(1, 0).unwrap();
        assert_eq!(vendor.endpoints[0].transfer_type(), TransferType::Bulk);
        assert!(!vendor.endpoints[0].is_in());
        assert!(c.interface(1, 1).is_none());
    }

    #[test]
    fn rejects_a_truncated_or_orphaned_configuration() {
        let full = config_bytes();
        assert_eq!(parse_configuration(&full[..full.len() - 1]), Err(ParseError::Truncated));
        // An endpoint descriptor placed before any interface is an error.
        let mut bad = vec![9, DESC_CONFIGURATION, 16, 0, 1, 1, 0, 0x80, 50];
        bad.extend_from_slice(&[7, DESC_ENDPOINT, 0x81, 0x03, 8, 0, 10]);
        assert_eq!(parse_configuration(&bad), Err(ParseError::Orphan));
    }

    #[test]
    fn encodes_a_setup_packet_in_wire_order_and_decodes_it_back() {
        let p = get_descriptor(DESC_CONFIGURATION, 0, 0, 0x0009);
        assert_eq!(p.encode(), [0x80, 6, 0x00, 0x02, 0, 0, 9, 0]);
        assert_eq!(SetupPacket::decode(&p.encode()), p);
        let s = set_configuration(1);
        assert_eq!(s.encode(), [0x00, 9, 1, 0, 0, 0, 0, 0]);
        let i = set_interface(2, 1);
        assert_eq!(i.request_type(), 0x01);
        assert_eq!(i.encode(), [0x01, 11, 1, 0, 2, 0, 0, 0]);
        let class = SetupPacket {
            direction: Direction::HostToDevice,
            kind: RequestKind::Class,
            recipient: Recipient::Interface,
            request: 0x0a,
            value: 0,
            index: 0,
            length: 0,
        };
        assert_eq!(class.request_type(), 0x21);
    }

    #[test]
    fn device_match_checks_only_the_fields_that_are_set() {
        let d = parse_device(&device_bytes(0x1234, 0x5678, 0)).unwrap();
        assert!(DeviceMatch::default().matches(&d));
        assert!(DeviceMatch { vendor_id: Some(0x1234), ..Default::default() }.matches(&d));
        assert!(!DeviceMatch { vendor_id: Some(0x1235), ..Default::default() }.matches(&d));
        assert!(!DeviceMatch { product_id: Some(0x0000), ..Default::default() }.matches(&d));
    }

    #[test]
    fn criteria_any_and_all_over_the_configuration_interfaces() {
        let d = parse_device(&device_bytes(0x1234, 0x5678, 0)).unwrap();
        let c = parse_configuration(&config_bytes()).unwrap();
        let hid = InterfaceMatch { class: Some(3), ..Default::default() };
        let vendor = InterfaceMatch { class: Some(0xff), ..Default::default() };
        let missing = InterfaceMatch { class: Some(7), ..Default::default() };

        let any = MatchingCriteria {
            device: DeviceMatch { vendor_id: Some(0x1234), ..Default::default() },
            interfaces: vec![hid, missing],
            option: Some(InterfaceMatchOption::MatchAny),
        };
        assert!(any.matches(&d, &c));

        let all = MatchingCriteria { option: Some(InterfaceMatchOption::MatchAll), ..any.clone() };
        assert!(!all.matches(&d, &c));
        let all_present = MatchingCriteria { interfaces: vec![hid, vendor], ..all.clone() };
        assert!(all_present.matches(&d, &c));

        let wrong_vendor = MatchingCriteria { device: DeviceMatch { vendor_id: Some(1), ..Default::default() }, ..any };
        assert!(!wrong_vendor.matches(&d, &c));
        let device_only = MatchingCriteria { device: DeviceMatch::default(), ..Default::default() };
        assert!(device_only.matches(&d, &c));
    }
}
