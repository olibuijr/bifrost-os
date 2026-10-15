//! USB devices as the system reports them: device and interface class codes, and matching rules that select a device
//! by identifiers, class, subclass and protocol. The system's device monitor fills a [`Device`] from the descriptors of
//! each hotplugged device; this module never opens a device. Class codes follow the USB class definitions
//! (HID 1.11, Video Class 1.5, Audio Class 2.0).

/// Device or interface class code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceClass {
    Audio,
    Hid,
    Hub,
    Video,
    /// Any other class, by its code.
    Other(u8),
}

impl DeviceClass {
    /// The class for a descriptor's class code.
    pub fn from_code(code: u8) -> Self {
        match code {
            0x01 => Self::Audio,
            0x03 => Self::Hid,
            0x09 => Self::Hub,
            0x0e => Self::Video,
            other => Self::Other(other),
        }
    }

    /// The descriptor's class code.
    pub fn code(self) -> u8 {
        match self {
            Self::Audio => 0x01,
            Self::Hid => 0x03,
            Self::Hub => 0x09,
            Self::Video => 0x0e,
            Self::Other(code) => code,
        }
    }
}

/// HID interface protocol for a keyboard.
pub const HID_PROTOCOL_KEYBOARD: u8 = 1;
/// HID interface protocol for a mouse.
pub const HID_PROTOCOL_MOUSE: u8 = 2;
/// Video interface subclass for the control interface.
pub const VIDEO_SUBCLASS_CONTROL: u8 = 1;
/// Video interface subclass for the streaming interface.
pub const VIDEO_SUBCLASS_STREAMING: u8 = 2;
/// Audio interface subclass for the control interface.
pub const AUDIO_SUBCLASS_CONTROL: u8 = 1;
/// Audio interface subclass for audio streaming (playback and capture).
pub const AUDIO_SUBCLASS_STREAMING: u8 = 2;

/// One interface of a device, with the class triple the descriptor gives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Interface {
    pub class: DeviceClass,
    pub subclass: u8,
    pub protocol: u8,
}

/// A device and its interfaces as the system reports them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    pub vendor_id: u16,
    pub product_id: u16,
    /// Device-level class; 0 in the descriptor means the class is per interface.
    pub class: DeviceClass,
    pub interfaces: Vec<Interface>,
}

impl Device {
    /// Whether an interface is a HID keyboard.
    pub fn is_keyboard(&self) -> bool {
        self.interfaces.iter().any(|i| i.class == DeviceClass::Hid && i.protocol == HID_PROTOCOL_KEYBOARD)
    }

    /// Whether an interface is a HID mouse or other pointing device.
    pub fn is_pointer(&self) -> bool {
        self.interfaces.iter().any(|i| i.class == DeviceClass::Hid && i.protocol == HID_PROTOCOL_MOUSE)
    }

    /// Whether the device has a video streaming interface (a UVC camera).
    pub fn is_camera(&self) -> bool {
        self.interfaces.iter().any(|i| i.class == DeviceClass::Video && i.subclass == VIDEO_SUBCLASS_STREAMING)
    }

    /// Whether the device has an audio streaming interface.
    pub fn is_audio(&self) -> bool {
        self.interfaces.iter().any(|i| i.class == DeviceClass::Audio && i.subclass == AUDIO_SUBCLASS_STREAMING)
    }
}

/// Selects devices. Unset fields match anything; set fields must all match.
///
/// `class` matches the device class or any interface class. `interface_subclass` and `interface_protocol` must match
/// one interface together, so `class: Hid` with `interface_protocol: Some(HID_PROTOCOL_MOUSE)` never matches a
/// keyboard that only shares the class.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Matching {
    pub vendor_id: Option<u16>,
    pub product_id: Option<u16>,
    pub class: Option<DeviceClass>,
    pub interface_subclass: Option<u8>,
    pub interface_protocol: Option<u8>,
}

impl Matching {
    /// Whether `device` satisfies every set field.
    pub fn matches(&self, device: &Device) -> bool {
        let ids = self.vendor_id.map_or(true, |v| v == device.vendor_id) && self.product_id.map_or(true, |p| p == device.product_id);
        if !ids {
            return false;
        }
        let interface_terms = self.interface_subclass.is_some() || self.interface_protocol.is_some();
        if interface_terms {
            return device.interfaces.iter().any(|i| self.interface_matches(i));
        }
        match self.class {
            None => true,
            Some(class) => class == device.class || device.interfaces.iter().any(|i| i.class == class),
        }
    }

    fn interface_matches(&self, interface: &Interface) -> bool {
        self.class.map_or(true, |c| c == interface.class)
            && self.interface_subclass.map_or(true, |s| s == interface.subclass)
            && self.interface_protocol.map_or(true, |p| p == interface.protocol)
    }
}

/// Parses a sysfs identifier such as `1d6b` (hexadecimal, up to four digits).
pub fn parse_id(text: &str) -> Option<u16> {
    u16::from_str_radix(text.trim(), 16).ok()
}

/// Parses a sysfs class or subclass code such as `03` (hexadecimal, up to two digits).
pub fn parse_code(text: &str) -> Option<u8> {
    u8::from_str_radix(text.trim(), 16).ok()
}
