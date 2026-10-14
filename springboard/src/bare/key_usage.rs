//! Keyboard usage table: maps the evdev key codes that `input` reports to USB HID keyboard usages (usage page 0x07,
//! "Keyboard/Keypad") and gives the boot-report modifier bit of each modifier key. Pure table: no device is opened, so
//! the mapping can be tested on any host. Keys without a usage here (media keys, keypad keys, vendor keys) map to None.

/// The HID usage page of keyboard and keypad usages.
pub const KEYBOARD_PAGE: u16 = 0x07;

/// The first usage of the eight modifier keys (left control through right meta) in usage page 0x07.
const FIRST_MODIFIER_USAGE: u8 = 0xE0;

/// Pairs of (evdev code, HID usage), one row per key. Codes and usages are each listed once.
const KEYS: [(u16, u8); 83] = [
    (30, 0x04), (48, 0x05), (46, 0x06), (32, 0x07), (18, 0x08), (33, 0x09), (34, 0x0A), (35, 0x0B),
    (23, 0x0C), (36, 0x0D), (37, 0x0E), (38, 0x0F), (50, 0x10), (49, 0x11), (24, 0x12), (25, 0x13),
    (16, 0x14), (19, 0x15), (31, 0x16), (20, 0x17), (22, 0x18), (47, 0x19), (17, 0x1A), (45, 0x1B),
    (21, 0x1C), (44, 0x1D),
    (2, 0x1E), (3, 0x1F), (4, 0x20), (5, 0x21), (6, 0x22), (7, 0x23), (8, 0x24), (9, 0x25), (10, 0x26),
    (11, 0x27),
    (28, 0x28), (1, 0x29), (14, 0x2A), (15, 0x2B), (57, 0x2C), (12, 0x2D), (13, 0x2E), (26, 0x2F),
    (27, 0x30), (43, 0x31), (39, 0x33), (40, 0x34), (41, 0x35), (51, 0x36), (52, 0x37), (53, 0x38),
    (58, 0x39),
    (59, 0x3A), (60, 0x3B), (61, 0x3C), (62, 0x3D), (63, 0x3E), (64, 0x3F), (65, 0x40), (66, 0x41),
    (67, 0x42), (68, 0x43), (87, 0x44), (88, 0x45),
    (110, 0x49), (102, 0x4A), (104, 0x4B), (111, 0x4C), (107, 0x4D), (109, 0x4E), (106, 0x4F),
    (105, 0x50), (108, 0x51), (103, 0x52),
    (29, 0xE0), (42, 0xE1), (56, 0xE2), (125, 0xE3), (97, 0xE4), (54, 0xE5), (100, 0xE6), (126, 0xE7),
];

/// The HID usage of an evdev key code, or None when the key has no keyboard usage here.
pub fn usage_of(code: u16) -> Option<u8> {
    KEYS.iter().find(|(c, _)| *c == code).map(|&(_, usage)| usage)
}

/// The evdev key code of a HID keyboard usage, the inverse of [`usage_of`].
pub fn code_of(usage: u8) -> Option<u16> {
    KEYS.iter().find(|(_, u)| *u == usage).map(|&(code, _)| code)
}

/// The bit a modifier key sets in the first byte of a boot keyboard report, or None for other keys. Left control is
/// bit 0 and right meta is bit 7, in the order of their usages.
pub fn boot_modifier_bit(code: u16) -> Option<u8> {
    let usage = usage_of(code)?;
    (FIRST_MODIFIER_USAGE..=FIRST_MODIFIER_USAGE + 7)
        .contains(&usage)
        .then(|| 1u8 << (usage - FIRST_MODIFIER_USAGE))
}

/// The key-slot value a boot report carries when more than six keys are down (HID "ErrorRollOver").
pub const ROLL_OVER: u8 = 0x01;

/// The eight-byte boot keyboard report for the keys down, in press order: the modifier bits, a reserved byte, then up to
/// six key usages. Keys without a usage here are left out, and a repeated key is listed once. With seven or more keys
/// down every key slot holds [`ROLL_OVER`].
pub fn boot_report(pressed: &[u16]) -> [u8; 8] {
    let mut report = [0u8; 8];
    let mut keys: Vec<u8> = Vec::new();
    for &code in pressed {
        if let Some(bit) = boot_modifier_bit(code) {
            report[0] |= bit;
        } else if let Some(usage) = usage_of(code) {
            if !keys.contains(&usage) {
                keys.push(usage);
            }
        }
    }
    if keys.len() > 6 {
        report[2..].fill(ROLL_OVER);
    } else {
        report[2..2 + keys.len()].copy_from_slice(&keys);
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_map_to_their_alphabetical_usages_not_their_keyboard_positions() {
        assert_eq!(usage_of(30), Some(0x04), "evdev A is usage 0x04");
        assert_eq!(usage_of(16), Some(0x14), "evdev Q, the top-left letter, is usage 0x14 (Q)");
        assert_eq!(usage_of(44), Some(0x1D), "evdev Z is usage 0x1D");
    }

    #[test]
    fn digit_row_starts_at_one_and_ends_at_zero() {
        assert_eq!(usage_of(2), Some(0x1E), "the 1 key");
        assert_eq!(usage_of(10), Some(0x26), "the 9 key");
        assert_eq!(usage_of(11), Some(0x27), "the 0 key");
    }

    #[test]
    fn editing_and_navigation_keys_have_their_usages() {
        assert_eq!(usage_of(28), Some(0x28), "Enter");
        assert_eq!(usage_of(1), Some(0x29), "Escape");
        assert_eq!(usage_of(14), Some(0x2A), "Backspace");
        assert_eq!(usage_of(57), Some(0x2C), "Space");
        assert_eq!(usage_of(59), Some(0x3A), "F1");
        assert_eq!(usage_of(88), Some(0x45), "F12");
        assert_eq!(usage_of(102), Some(0x4A), "Home");
        assert_eq!(usage_of(103), Some(0x52), "Up arrow");
    }

    #[test]
    fn a_key_without_a_keyboard_usage_maps_to_none() {
        assert_eq!(usage_of(114), None, "volume down is a consumer key, not a keyboard usage");
        assert_eq!(usage_of(0), None, "code zero is no key");
    }

    #[test]
    fn usage_and_code_are_inverse_for_every_listed_key() {
        for &(code, usage) in KEYS.iter() {
            assert_eq!(usage_of(code), Some(usage));
            assert_eq!(code_of(usage), Some(code));
        }
    }

    #[test]
    fn no_code_or_usage_is_listed_twice() {
        for (i, a) in KEYS.iter().enumerate() {
            for b in &KEYS[i + 1..] {
                assert_ne!(a.0, b.0, "evdev code {} listed twice", a.0);
                assert_ne!(a.1, b.1, "usage {:#04x} listed twice", a.1);
            }
        }
    }

    #[test]
    fn modifier_keys_set_the_boot_report_bits_in_usage_order() {
        assert_eq!(boot_modifier_bit(29), Some(0x01), "left control is bit 0");
        assert_eq!(boot_modifier_bit(42), Some(0x02), "left shift is bit 1");
        assert_eq!(boot_modifier_bit(56), Some(0x04), "left alt is bit 2");
        assert_eq!(boot_modifier_bit(125), Some(0x08), "left meta is bit 3");
        assert_eq!(boot_modifier_bit(97), Some(0x10), "right control is bit 4");
        assert_eq!(boot_modifier_bit(54), Some(0x20), "right shift is bit 5");
        assert_eq!(boot_modifier_bit(100), Some(0x40), "right alt is bit 6");
        assert_eq!(boot_modifier_bit(126), Some(0x80), "right meta is bit 7");
    }

    #[test]
    fn a_boot_report_carries_the_modifiers_and_the_keys_in_press_order() {
        // Left control and left shift with the A key, then the B key.
        assert_eq!(boot_report(&[29, 42, 30, 48]), [0x03, 0, 0x04, 0x05, 0, 0, 0, 0]);
    }

    #[test]
    fn a_boot_report_lists_a_repeated_key_once_and_skips_keys_without_usage() {
        assert_eq!(boot_report(&[30, 30, 114]), [0, 0, 0x04, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn a_seventh_key_turns_every_key_slot_into_roll_over() {
        let pressed: Vec<u16> = (30..37).collect();
        assert_eq!(boot_report(&pressed), [0, 0, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01]);
    }

    #[test]
    fn a_letter_has_no_modifier_bit() {
        assert_eq!(boot_modifier_bit(30), None);
    }
}
