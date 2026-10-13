//! Bluetooth device addresses and classes of device (std only).
//!
//! A `BdAddr` is the six-byte address of a device. It is written most significant byte first as `AA:BB:CC:DD:EE:FF`
//! (a hyphen separator is accepted too) and stored in that same order. The radio sends the bytes in the reverse order;
//! the caller reverses them at that boundary.
//!
//! A `ClassOfDevice` is the 24-bit value a device advertises when it is found: two format bits (zero for the class
//! defined today), a six-bit minor class, a five-bit major class, then eleven service class bits. This module parses,
//! prints and classifies both values. It opens no radio and no socket.

use std::fmt;
use std::str::FromStr;

/// The six-byte address of a Bluetooth device, stored in the order it is printed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct BdAddr(pub [u8; 6]);

/// Why text is not a device address.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AddrError {
    /// The text is not six groups of two characters, separated by `:` or `-`.
    BadFormat,
    /// A group holds a character that is not a hexadecimal digit.
    BadHex,
}

impl BdAddr {
    /// True for the all-zero address, which no device uses.
    pub fn is_zero(&self) -> bool {
        self.0 == [0; 6]
    }
}

impl FromStr for BdAddr {
    type Err = AddrError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let groups: Vec<&str> = s.split([':', '-']).collect();
        if groups.len() != 6 || groups.iter().any(|g| g.len() != 2) {
            return Err(AddrError::BadFormat);
        }
        let mut out = [0u8; 6];
        for (slot, &group) in out.iter_mut().zip(groups.iter()) {
            if !group.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(AddrError::BadHex);
            }
            *slot = u8::from_str_radix(group, 16).map_err(|_| AddrError::BadHex)?;
        }
        Ok(BdAddr(out))
    }
}

impl fmt::Display for BdAddr {
    /// Upper-case hex, two digits per byte, colon separated.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let b = self.0;
        write!(f, "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}", b[0], b[1], b[2], b[3], b[4], b[5])
    }
}

/// Service class bit: positioning (bit 16 of the class of device).
pub const SERVICE_POSITIONING: u32 = 1 << 16;
/// Service class bit: networking (bit 17).
pub const SERVICE_NETWORKING: u32 = 1 << 17;
/// Service class bit: rendering, such as a speaker or a display (bit 18).
pub const SERVICE_RENDERING: u32 = 1 << 18;
/// Service class bit: capturing, such as a microphone or a camera (bit 19).
pub const SERVICE_CAPTURING: u32 = 1 << 19;
/// Service class bit: object transfer (bit 20).
pub const SERVICE_OBJECT_TRANSFER: u32 = 1 << 20;
/// Service class bit: audio (bit 21).
pub const SERVICE_AUDIO: u32 = 1 << 21;
/// Service class bit: telephony (bit 22).
pub const SERVICE_TELEPHONY: u32 = 1 << 22;
/// Service class bit: information (bit 23).
pub const SERVICE_INFORMATION: u32 = 1 << 23;

/// The major device class: the broad kind of device.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MajorClass {
    Miscellaneous,
    Computer,
    Phone,
    NetworkAccess,
    AudioVideo,
    Peripheral,
    Imaging,
    Wearable,
    Toy,
    Health,
    Uncategorized,
    /// A major class code this module does not name.
    Reserved(u8),
}

/// What a device is, as far as this module can tell from its class of device.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DeviceKind {
    Computer,
    Phone,
    Headset,
    HandsFree,
    Microphone,
    Loudspeaker,
    Headphones,
    Keyboard,
    Pointer,
    KeyboardAndPointer,
    Other,
}

/// The 24-bit class of device of a remote device.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ClassOfDevice(u32);

impl ClassOfDevice {
    /// Builds a class from its raw value. Values above 24 bits are refused.
    pub fn new(raw: u32) -> Option<Self> {
        if raw <= 0xFF_FFFF {
            Some(ClassOfDevice(raw))
        } else {
            None
        }
    }

    /// The raw 24-bit value.
    pub fn raw(self) -> u32 {
        self.0
    }

    /// The six-bit minor device class (bits 2 to 7).
    pub fn minor(self) -> u8 {
        ((self.0 >> 2) & 0x3F) as u8
    }

    /// The major device class (bits 8 to 12).
    pub fn major(self) -> MajorClass {
        match ((self.0 >> 8) & 0x1F) as u8 {
            0 => MajorClass::Miscellaneous,
            1 => MajorClass::Computer,
            2 => MajorClass::Phone,
            3 => MajorClass::NetworkAccess,
            4 => MajorClass::AudioVideo,
            5 => MajorClass::Peripheral,
            6 => MajorClass::Imaging,
            7 => MajorClass::Wearable,
            8 => MajorClass::Toy,
            9 => MajorClass::Health,
            31 => MajorClass::Uncategorized,
            other => MajorClass::Reserved(other),
        }
    }

    /// True when the service class bit `service` (one of the `SERVICE_*` constants) is set.
    pub fn has_service(self, service: u32) -> bool {
        self.0 & service != 0
    }

    /// The kind of device. Audio-video minor classes name headsets and speakers; peripheral classes say whether the
    /// device is a keyboard (bit 6 of the class) or a pointing device (bit 7).
    pub fn kind(self) -> DeviceKind {
        match self.major() {
            MajorClass::Computer => DeviceKind::Computer,
            MajorClass::Phone => DeviceKind::Phone,
            MajorClass::AudioVideo => match self.minor() {
                1 => DeviceKind::Headset,
                2 => DeviceKind::HandsFree,
                4 => DeviceKind::Microphone,
                5 => DeviceKind::Loudspeaker,
                6 => DeviceKind::Headphones,
                _ => DeviceKind::Other,
            },
            MajorClass::Peripheral => match (self.0 & 0x40 != 0, self.0 & 0x80 != 0) {
                (true, true) => DeviceKind::KeyboardAndPointer,
                (true, false) => DeviceKind::Keyboard,
                (false, true) => DeviceKind::Pointer,
                (false, false) => DeviceKind::Other,
            },
            _ => DeviceKind::Other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colon_and_hyphen_forms_parse_to_the_same_address() {
        let a: BdAddr = "aa:bb:0c:1D:2e:3F".parse().unwrap();
        let b: BdAddr = "AA-BB-0C-1D-2E-3F".parse().unwrap();
        assert_eq!(a, b);
        assert_eq!(a.0, [0xAA, 0xBB, 0x0C, 0x1D, 0x2E, 0x3F]);
    }

    #[test]
    fn display_is_upper_case_colon_separated_with_two_digits_per_byte() {
        let a = BdAddr([0x00, 0x1A, 0x7d, 0xDA, 0x71, 0x13]);
        assert_eq!(a.to_string(), "00:1A:7D:DA:71:13");
    }

    #[test]
    fn malformed_addresses_are_refused() {
        assert_eq!("AA:BB:CC:DD:EE".parse::<BdAddr>(), Err(AddrError::BadFormat));
        assert_eq!("AA:BB:CC:DD:EE:F".parse::<BdAddr>(), Err(AddrError::BadFormat));
        assert_eq!("AA:BB:CC:DD:EE:FG".parse::<BdAddr>(), Err(AddrError::BadHex));
        assert_eq!("+A:BB:CC:DD:EE:FF".parse::<BdAddr>(), Err(AddrError::BadHex));
    }

    #[test]
    fn only_the_all_zero_address_is_zero() {
        assert!(BdAddr([0; 6]).is_zero());
        assert!(!BdAddr([0, 0, 0, 0, 0, 1]).is_zero());
    }

    #[test]
    fn a_headset_class_is_audio_video_minor_one_with_audio_and_rendering() {
        let c = ClassOfDevice::new(0x24_0404).unwrap();
        assert_eq!(c.major(), MajorClass::AudioVideo);
        assert_eq!(c.minor(), 1);
        assert_eq!(c.kind(), DeviceKind::Headset);
        assert!(c.has_service(SERVICE_AUDIO));
        assert!(c.has_service(SERVICE_RENDERING));
        assert!(!c.has_service(SERVICE_TELEPHONY));
    }

    #[test]
    fn peripheral_keyboard_and_pointer_flags_come_from_bits_six_and_seven() {
        let keyboard = ClassOfDevice::new(0x00_2540).unwrap();
        assert_eq!(keyboard.major(), MajorClass::Peripheral);
        assert_eq!(keyboard.minor(), 16);
        assert_eq!(keyboard.kind(), DeviceKind::Keyboard);
        let combo = ClassOfDevice::new(0x00_25C0).unwrap();
        assert_eq!(combo.kind(), DeviceKind::KeyboardAndPointer);
        let mouse = ClassOfDevice::new(0x00_2580).unwrap();
        assert_eq!(mouse.kind(), DeviceKind::Pointer);
    }

    #[test]
    fn a_class_above_twenty_four_bits_is_refused() {
        assert!(ClassOfDevice::new(0x0100_0000).is_none());
        assert_eq!(ClassOfDevice::new(0xFF_FFFF).unwrap().raw(), 0xFF_FFFF);
    }
}
