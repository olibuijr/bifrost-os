//! X11 keysyms, the key numbers RFB KeyEvent messages carry.
//!
//! Latin-1 characters are their own keysym; other characters use 0x01000000 + their code point. The shell sends
//! control keys as small codes (8 Backspace, 9 Tab, 13 Return, 27 Escape, 37 to 40 the arrows, 127 Delete), which
//! `from_code` maps to the matching keysyms.

/// The keysym for a typed character.
pub fn from_char(c: char) -> u32 {
    let cp = c as u32;
    if cp < 0x100 {
        cp
    } else {
        0x0100_0000 | cp
    }
}

/// The keysym for a shell control key code, or None when the code is not a control key.
pub fn from_code(code: u32) -> Option<u32> {
    Some(match code {
        8 => 0xff08,   // BackSpace
        9 => 0xff09,   // Tab
        13 => 0xff0d,  // Return
        27 => 0xff1b,  // Escape
        37 => 0xff51,  // Left
        38 => 0xff52,  // Up
        39 => 0xff53,  // Right
        40 => 0xff54,  // Down
        127 => 0xffff, // Delete
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latin_one_is_identity_and_other_characters_use_the_unicode_range() {
        assert_eq!(from_char('a'), 0x61);
        assert_eq!(from_char('A'), 0x41);
        assert_eq!(from_char(' '), 0x20);
        assert_eq!(from_char('é'), 0xe9);
        assert_eq!(from_char('€'), 0x0100_20ac);
    }

    #[test]
    fn control_codes_map_to_their_keysyms() {
        assert_eq!(from_code(13), Some(0xff0d));
        assert_eq!(from_code(8), Some(0xff08));
        assert_eq!(from_code(38), Some(0xff52));
        assert_eq!(from_code(127), Some(0xffff));
        assert_eq!(from_code(65), None, "letters arrive as text, not as codes");
    }
}
