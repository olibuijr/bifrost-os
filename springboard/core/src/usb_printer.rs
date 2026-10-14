//! USB printer class: recognise printer interfaces, build the class control requests, decode the port status byte and
//! parse the IEEE 1284 device ID string.
//!
//! Pure, like the rest of the USB model: no device is opened and no request is sent. The caller reads the descriptors
//! with `usb_descriptors`, finds a printer interface with [`is_printer_interface`], sends the [`SetupPacket`]s built
//! here through its own transport, and moves the print data over the bulk endpoints named by [`bulk_endpoints`].
//!
//! Follows the USB printer class (class code 7, subclass 1). The protocol byte says whether the interface is
//! unidirectional, bidirectional or IEEE 1284.4. Three class requests are modelled: `GET_DEVICE_ID`,
//! `GET_PORT_STATUS` and `SOFT_RESET`. The device ID is the IEEE 1284 text block: a two-byte big-endian length that
//! counts itself, then `KEY:VALUE;` pairs.

use crate::usb_descriptors::{Direction, InterfaceDescriptor, Recipient, RequestKind, SetupPacket, TransferType};

/// `bInterfaceClass` of the printer class.
pub const CLASS_PRINTER: u8 = 7;
/// `bInterfaceSubClass` of the printer class.
pub const SUBCLASS_PRINTER: u8 = 1;

/// The class request numbers (`bRequest`).
pub mod request {
    pub const GET_DEVICE_ID: u8 = 0;
    pub const GET_PORT_STATUS: u8 = 1;
    pub const SOFT_RESET: u8 = 2;
}

/// The protocol of a printer interface (`bInterfaceProtocol`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Protocol {
    /// Host to printer only (protocol 1).
    Unidirectional,
    /// Host and printer both send data (protocol 2).
    Bidirectional,
    /// IEEE 1284.4 packet protocol, for multi-function devices (protocol 3).
    Ieee1284Dot4,
}

/// The printer protocol of an interface, or `None` when it is not a printer interface.
pub fn protocol(iface: &InterfaceDescriptor) -> Option<Protocol> {
    if iface.class != CLASS_PRINTER || iface.subclass != SUBCLASS_PRINTER {
        return None;
    }
    match iface.protocol {
        1 => Some(Protocol::Unidirectional),
        2 => Some(Protocol::Bidirectional),
        3 => Some(Protocol::Ieee1284Dot4),
        _ => None,
    }
}

/// Whether the interface is a printer interface.
pub fn is_printer_interface(iface: &InterfaceDescriptor) -> bool {
    protocol(iface).is_some()
}

/// The bulk endpoint addresses of an interface, as `(out, in)`. The first bulk endpoint of each direction wins. An
/// interrupt endpoint is not a print data path and is ignored here.
pub fn bulk_endpoints(iface: &InterfaceDescriptor) -> (Option<u8>, Option<u8>) {
    let mut out = None;
    let mut input = None;
    for ep in &iface.endpoints {
        if ep.transfer_type() != TransferType::Bulk {
            continue;
        }
        if ep.is_in() {
            if input.is_none() {
                input = Some(ep.address);
            }
        } else if out.is_none() {
            out = Some(ep.address);
        }
    }
    (out, input)
}

/// Whether the interface can carry print data: it is a printer interface with a bulk-out endpoint. A printer without
/// one can only report its status, so it is not offered for printing.
pub fn can_print(iface: &InterfaceDescriptor) -> bool {
    is_printer_interface(iface) && bulk_endpoints(iface).0.is_some()
}

/// `GET_DEVICE_ID` to the interface. The reply starts with the two-byte length, so `length` should be the buffer size.
pub fn get_device_id(interface: u8, length: u16) -> SetupPacket {
    class_request(Direction::DeviceToHost, request::GET_DEVICE_ID, interface, length)
}

/// `GET_PORT_STATUS` to the interface. The reply is one byte, read with [`PortStatus`].
pub fn get_port_status(interface: u8) -> SetupPacket {
    class_request(Direction::DeviceToHost, request::GET_PORT_STATUS, interface, 1)
}

/// `SOFT_RESET` to the interface, which clears the printer's buffers and the bulk pipes.
pub fn soft_reset(interface: u8) -> SetupPacket {
    class_request(Direction::HostToDevice, request::SOFT_RESET, interface, 0)
}

fn class_request(direction: Direction, request: u8, interface: u8, length: u16) -> SetupPacket {
    SetupPacket {
        direction,
        kind: RequestKind::Class,
        recipient: Recipient::Interface,
        request,
        value: 0,
        index: u16::from(interface),
        length,
    }
}

/// The one-byte port status reply. Bits 0 to 2 and 6 to 7 are reserved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PortStatus(pub u8);

impl PortStatus {
    /// Bit 5: the paper tray is empty.
    pub fn paper_empty(self) -> bool {
        self.0 & 0x20 != 0
    }

    /// Bit 4: the printer is selected (online).
    pub fn selected(self) -> bool {
        self.0 & 0x10 != 0
    }

    /// Bit 3: no printer error. The bit is set when the printer is fine.
    pub fn no_error(self) -> bool {
        self.0 & 0x08 != 0
    }

    /// Selected, without an error and with paper: the printer will take print data.
    pub fn ready(self) -> bool {
        self.selected() && self.no_error() && !self.paper_empty()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceIdError {
    /// Fewer than two bytes, or a declared length below two.
    TooShort,
    /// The declared length is more than the bytes that were read.
    LengthPastEnd,
    /// The text after the length is not UTF-8.
    NotText,
}

/// The parsed IEEE 1284 device ID: its `KEY:VALUE` pairs in the order they were sent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceId {
    pairs: Vec<(String, String)>,
}

/// Parse a `GET_DEVICE_ID` reply. The declared length includes its own two bytes; bytes after it are ignored, and
/// NUL and whitespace around a pair are trimmed. A segment without a colon is skipped.
pub fn parse_device_id(bytes: &[u8]) -> Result<DeviceId, DeviceIdError> {
    if bytes.len() < 2 {
        return Err(DeviceIdError::TooShort);
    }
    let declared = usize::from(u16::from_be_bytes([bytes[0], bytes[1]]));
    if declared < 2 {
        return Err(DeviceIdError::TooShort);
    }
    if declared > bytes.len() {
        return Err(DeviceIdError::LengthPastEnd);
    }
    let body = std::str::from_utf8(&bytes[2..declared]).map_err(|_| DeviceIdError::NotText)?;
    let mut pairs = Vec::new();
    for part in body.split(';') {
        let part = part.trim_matches(|c: char| c.is_whitespace() || c == '\0');
        if let Some((key, value)) = part.split_once(':') {
            let key = key.trim();
            if !key.is_empty() {
                pairs.push((key.to_string(), value.trim().to_string()));
            }
        }
    }
    Ok(DeviceId { pairs })
}

impl DeviceId {
    /// The value of a key, matched without case. Keys are as the device sent them.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.pairs
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.as_str())
    }

    /// The manufacturer, from `MFG` or the long form `MANUFACTURER`.
    pub fn manufacturer(&self) -> Option<&str> {
        self.get("MFG").or_else(|| self.get("MANUFACTURER"))
    }

    /// The model, from `MDL` or the long form `MODEL`.
    pub fn model(&self) -> Option<&str> {
        self.get("MDL").or_else(|| self.get("MODEL"))
    }

    /// The page description languages, from `CMD` or `COMMAND SET`, split on commas.
    pub fn command_sets(&self) -> Vec<&str> {
        self.get("CMD")
            .or_else(|| self.get("COMMAND SET"))
            .map(|c| c.split(',').map(str::trim).filter(|s| !s.is_empty()).collect::<Vec<&str>>())
            .unwrap_or_default()
    }

    /// The name to show in a printer list: the manufacturer and the model joined by a space. A model that already
    /// starts with the manufacturer name is shown alone, so `HP` with `HP LaserJet` gives `HP LaserJet`.
    pub fn display_name(&self) -> Option<String> {
        let mfg = self.manufacturer().map(str::trim).filter(|s| !s.is_empty());
        let mdl = self.model().map(str::trim).filter(|s| !s.is_empty());
        match (mfg, mdl) {
            (Some(m), Some(d)) if d.to_ascii_lowercase().starts_with(&m.to_ascii_lowercase()) => Some(d.to_string()),
            (Some(m), Some(d)) => Some(format!("{m} {d}")),
            (Some(m), None) => Some(m.to_string()),
            (None, Some(d)) => Some(d.to_string()),
            (None, None) => None,
        }
    }

    /// Whether the printer lists a page description language, such as `PCL` or `PS`, without case.
    pub fn supports(&self, language: &str) -> bool {
        self.command_sets().iter().any(|c| c.eq_ignore_ascii_case(language))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usb_descriptors::EndpointDescriptor;

    fn endpoint(address: u8, attributes: u8) -> EndpointDescriptor {
        EndpointDescriptor {
            address,
            attributes,
            max_packet_size: 512,
            interval: 0,
        }
    }

    fn interface(class: u8, subclass: u8, protocol: u8, endpoints: Vec<EndpointDescriptor>) -> InterfaceDescriptor {
        InterfaceDescriptor {
            number: 1,
            alternate: 0,
            class,
            subclass,
            protocol,
            endpoints,
        }
    }

    /// Wrap a device ID body with its two-byte length, which counts itself.
    fn device_id_bytes(body: &str) -> Vec<u8> {
        let total = body.len() + 2;
        let mut out = vec![(total >> 8) as u8, total as u8];
        out.extend_from_slice(body.as_bytes());
        out
    }

    #[test]
    fn printer_protocols_are_read_from_class_subclass_and_protocol() {
        let bidi = interface(7, 1, 2, vec![]);
        assert_eq!(protocol(&bidi), Some(Protocol::Bidirectional));
        assert!(is_printer_interface(&bidi));
        assert_eq!(protocol(&interface(7, 1, 1, vec![])), Some(Protocol::Unidirectional));
        assert_eq!(protocol(&interface(7, 1, 3, vec![])), Some(Protocol::Ieee1284Dot4));
        // Wrong subclass, unknown protocol and another class are not printers.
        assert_eq!(protocol(&interface(7, 2, 1, vec![])), None);
        assert_eq!(protocol(&interface(7, 1, 4, vec![])), None);
        assert!(!is_printer_interface(&interface(3, 1, 1, vec![])));
    }

    #[test]
    fn bulk_endpoints_pick_the_first_bulk_out_and_in() {
        // Bulk out 0x02, bulk in 0x81, an interrupt in 0x83 and a second bulk in 0x82 that must be ignored.
        let iface = interface(
            7,
            1,
            2,
            vec![endpoint(0x02, 2), endpoint(0x83, 3), endpoint(0x81, 2), endpoint(0x82, 2)],
        );
        assert_eq!(bulk_endpoints(&iface), (Some(0x02), Some(0x81)));
        // Only an interrupt endpoint: no print data path.
        assert_eq!(bulk_endpoints(&interface(7, 1, 1, vec![endpoint(0x83, 3)])), (None, None));
    }

    #[test]
    fn class_requests_use_the_interface_recipient() {
        // Device to host, class, interface: 0x80 | 0x20 | 0x01.
        let status = get_port_status(2);
        assert_eq!(status.request_type(), 0xA1);
        assert_eq!(status.request, request::GET_PORT_STATUS);
        assert_eq!(status.index, 2);
        assert_eq!(status.length, 1);

        let id = get_device_id(0, 1024);
        assert_eq!(id.request, request::GET_DEVICE_ID);
        assert_eq!(id.length, 1024);
        assert_eq!(id.value, 0);

        // Host to device, class, interface: 0x21, with no data stage.
        let reset = soft_reset(1);
        assert_eq!(reset.request_type(), 0x21);
        assert_eq!(reset.request, request::SOFT_RESET);
        assert_eq!(reset.length, 0);
    }

    #[test]
    fn port_status_bits_decide_readiness() {
        // 0x18: selected (bit 4) and no error (bit 3), paper present.
        assert!(PortStatus(0x18).ready());
        // 0x38: the same with the paper-empty bit (bit 5) set.
        assert!(PortStatus(0x38).paper_empty());
        assert!(!PortStatus(0x38).ready());
        // 0x08: no error, but not selected.
        assert!(!PortStatus(0x08).selected());
        assert!(!PortStatus(0x08).ready());
        // 0x00: the error bit is clear, so the printer has an error.
        assert!(!PortStatus(0x00).no_error());
    }

    #[test]
    fn device_id_gives_manufacturer_model_and_languages() {
        let bytes = device_id_bytes("MFG:HP;MDL:LaserJet 1020;CMD:PCL,PJL;");
        let id = parse_device_id(&bytes).unwrap();
        assert_eq!(id.manufacturer(), Some("HP"));
        assert_eq!(id.model(), Some("LaserJet 1020"));
        assert_eq!(id.get("mdl"), Some("LaserJet 1020"));
        assert_eq!(id.command_sets(), vec!["PCL", "PJL"]);
        assert!(id.supports("pjl"));
        assert!(!id.supports("PS"));
    }

    #[test]
    fn device_id_stops_at_its_declared_length() {
        // The declared length covers "MFG:X" only; the bytes after it are not part of the reply.
        let mut bytes = device_id_bytes("MFG:X");
        bytes.extend_from_slice(b"Y:Z");
        let id = parse_device_id(&bytes).unwrap();
        assert_eq!(id.manufacturer(), Some("X"));
        assert_eq!(id.get("Y"), None);
    }

    #[test]
    fn device_id_rejects_short_past_end_and_non_text() {
        assert_eq!(parse_device_id(&[0x00]), Err(DeviceIdError::TooShort));
        assert_eq!(parse_device_id(&[0x00, 0x01]), Err(DeviceIdError::TooShort));
        assert_eq!(parse_device_id(&[0x00, 0x20, b'A']), Err(DeviceIdError::LengthPastEnd));
        assert_eq!(parse_device_id(&[0x00, 0x04, 0xFF, 0xFE]), Err(DeviceIdError::NotText));
    }
}
