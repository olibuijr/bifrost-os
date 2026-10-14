//! Keyboard reports for virtual input devices (virtual device parity, keyboards only). A virtual keyboard sends
//! the eight-byte boot keyboard report: the modifier byte, a reserved byte and six key slots. This module builds,
//! checks and decodes those reports and holds the report descriptor that describes them. It does not create a device:
//! the shell's input layer owns the devices, and the uinput keyboard that the agent tooling uses is not an app API.
//!
//! Rules of this model:
//! - A report holds at most [`MAX_KEYS`] keys. Each key is a keyboard usage in [`KEY_USAGE_MIN`]..=[`KEY_USAGE_MAX`],
//!   and no key appears twice.
//! - Unused key slots are zero. Decoding drops zero slots.
//! - A device name is 1 to [`MAX_NAME_LEN`] bytes and is not blank.

/// Key slots in one boot keyboard report.
pub const MAX_KEYS: usize = 6;

/// Bytes in one boot keyboard report.
pub const REPORT_LEN: usize = 8;

/// The longest device name a virtual keyboard may carry.
pub const MAX_NAME_LEN: usize = 64;

/// The lowest keyboard usage that is a key (the letter A).
pub const KEY_USAGE_MIN: u8 = 0x04;

/// The highest keyboard usage that is a key (the keypad equals sign).
pub const KEY_USAGE_MAX: u8 = 0xA4;

/// The modifier bits of the first report byte.
pub mod modifier {
    pub const LEFT_CONTROL: u8 = 1 << 0;
    pub const LEFT_SHIFT: u8 = 1 << 1;
    pub const LEFT_ALT: u8 = 1 << 2;
    pub const LEFT_GUI: u8 = 1 << 3;
    pub const RIGHT_CONTROL: u8 = 1 << 4;
    pub const RIGHT_SHIFT: u8 = 1 << 5;
    pub const RIGHT_ALT: u8 = 1 << 6;
    pub const RIGHT_GUI: u8 = 1 << 7;
}

/// The report descriptor of a boot keyboard: eight modifier bits, one reserved byte, five LED outputs, three padding
/// bits and six key slots. A report built with [`KeyboardReport::encode`] matches this layout.
pub const BOOT_KEYBOARD_DESCRIPTOR: [u8; 63] = [
    0x05, 0x01, 0x09, 0x06, 0xa1, 0x01, 0x05, 0x07, 0x19, 0xe0, 0x29, 0xe7, 0x15, 0x00, 0x25, 0x01, 0x75, 0x01,
    0x95, 0x08, 0x81, 0x02, 0x95, 0x01, 0x75, 0x08, 0x81, 0x01, 0x95, 0x05, 0x75, 0x01, 0x05, 0x08, 0x19, 0x01,
    0x29, 0x05, 0x91, 0x02, 0x95, 0x01, 0x75, 0x03, 0x91, 0x01, 0x95, 0x06, 0x75, 0x08, 0x15, 0x00, 0x25, 0x65,
    0x05, 0x07, 0x19, 0x00, 0x29, 0x65, 0x81, 0x00, 0xc0,
];

/// Why a report or a device name is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReportError {
    /// More keys than the report has slots for.
    TooManyKeys(usize),
    /// A key usage outside the keyboard key range.
    InvalidKey(u8),
    /// The same key listed twice.
    DuplicateKey(u8),
    /// The device name is blank or longer than [`MAX_NAME_LEN`] bytes.
    InvalidName,
}

/// Checks a device name: it must not be blank and must fit in [`MAX_NAME_LEN`] bytes.
pub fn validate_name(name: &str) -> Result<(), ReportError> {
    if name.trim().is_empty() || name.len() > MAX_NAME_LEN {
        return Err(ReportError::InvalidName);
    }
    Ok(())
}

/// One boot keyboard report: the modifiers and the keys held down, in the order given.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyboardReport {
    modifiers: u8,
    keys: Vec<u8>,
}

impl KeyboardReport {
    /// A report with these modifiers and keys. Refuses more than six keys, a key outside the key range and a key
    /// listed twice.
    pub fn new(modifiers: u8, keys: &[u8]) -> Result<KeyboardReport, ReportError> {
        if keys.len() > MAX_KEYS {
            return Err(ReportError::TooManyKeys(keys.len()));
        }
        for (i, &key) in keys.iter().enumerate() {
            if !(KEY_USAGE_MIN..=KEY_USAGE_MAX).contains(&key) {
                return Err(ReportError::InvalidKey(key));
            }
            if keys[..i].contains(&key) {
                return Err(ReportError::DuplicateKey(key));
            }
        }
        Ok(KeyboardReport { modifiers, keys: keys.to_vec() })
    }

    /// The modifier bits.
    pub fn modifiers(&self) -> u8 {
        self.modifiers
    }

    /// The keys held down, in the order they were given.
    pub fn keys(&self) -> &[u8] {
        &self.keys
    }

    /// The eight bytes the report sends: the modifiers, a reserved zero, then the key slots with unused slots zero.
    pub fn encode(&self) -> [u8; REPORT_LEN] {
        let mut bytes = [0u8; REPORT_LEN];
        bytes[0] = self.modifiers;
        bytes[2..2 + self.keys.len()].copy_from_slice(&self.keys);
        bytes
    }

    /// Reads a report from eight bytes. The key slots that hold zero are dropped, and the result is checked like
    /// [`KeyboardReport::new`].
    pub fn decode(bytes: &[u8; REPORT_LEN]) -> Result<KeyboardReport, ReportError> {
        let keys: Vec<u8> = bytes[2..].iter().copied().filter(|&key| key != 0).collect();
        KeyboardReport::new(bytes[0], &keys)
    }
}

/// A virtual keyboard's identity: the name and the vendor and product ids the system shows for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VirtualKeyboard {
    name: String,
    vendor_id: u16,
    product_id: u16,
}

impl VirtualKeyboard {
    /// A keyboard with this name and these ids. Refuses a blank or overlong name.
    pub fn new(name: &str, vendor_id: u16, product_id: u16) -> Result<VirtualKeyboard, ReportError> {
        validate_name(name)?;
        Ok(VirtualKeyboard { name: name.to_string(), vendor_id, product_id })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn vendor_id(&self) -> u16 {
        self.vendor_id
    }

    pub fn product_id(&self) -> u16 {
        self.product_id
    }

    /// The bytes of one report from this keyboard.
    pub fn report(&self, modifiers: u8, keys: &[u8]) -> Result<[u8; REPORT_LEN], ReportError> {
        Ok(KeyboardReport::new(modifiers, keys)?.encode())
    }
}
