//! Custom note colours: the `#RRGGBB` text a note colour is typed and stored as, and the entry that Colour,
//! Custom Colour... edits.

/// A colour as red, green and blue bytes.
pub type Rgb = (u8, u8, u8);

/// Hex digits a colour entry holds.
pub const DIGITS: usize = 6;

/// Reads `#RRGGBB`, `RRGGBB`, `#RGB` or `RGB` (the short form doubles each digit). `None` for anything else.
pub fn parse(text: &str) -> Option<Rgb> {
    let text = text.trim();
    let digits = text.strip_prefix('#').unwrap_or(text);
    if !digits.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let full: String = match digits.len() {
        3 => digits.chars().flat_map(|c| [c, c]).collect(),
        DIGITS => digits.to_string(),
        _ => return None,
    };
    let v = u32::from_str_radix(&full, 16).ok()?;
    Some(((v >> 16) as u8, (v >> 8) as u8, v as u8))
}

/// The `#RRGGBB` text of a colour, upper case.
pub fn hex(c: Rgb) -> String {
    format!("#{:02X}{:02X}{:02X}", c.0, c.1, c.2)
}

/// Adds a typed character to a colour entry: one leading `#`, then up to six hex digits. False when it is refused.
pub fn push(entry: &mut String, c: char) -> bool {
    let digits = entry.chars().filter(char::is_ascii_hexdigit).count();
    let ok = if c == '#' { entry.is_empty() } else { c.is_ascii_hexdigit() && digits < DIGITS };
    if ok {
        entry.push(c.to_ascii_uppercase());
    }
    ok
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_and_short_forms_parse_with_or_without_the_hash() {
        assert_eq!(parse("#FFCC00"), Some((255, 204, 0)));
        assert_eq!(parse("ffcc00"), Some((255, 204, 0)));
        assert_eq!(parse(" #0a8 "), Some((0, 170, 136)));
        assert_eq!(parse("f00"), Some((255, 0, 0)));
    }

    #[test]
    fn junk_and_wrong_lengths_do_not_parse() {
        for bad in ["", "#", "#12", "#12345", "#1234567", "#GGGGGG", "##FFFFFF", "yellow"] {
            assert_eq!(parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn hex_prints_what_parse_reads() {
        assert_eq!(hex((10, 132, 255)), "#0A84FF");
        assert_eq!(parse(&hex((1, 2, 3))), Some((1, 2, 3)));
    }

    #[test]
    fn the_entry_takes_one_hash_first_and_six_hex_digits() {
        let mut e = String::new();
        assert!(push(&mut e, '#'));
        assert!(!push(&mut e, '#'), "a second hash");
        assert!(!push(&mut e, 'g'));
        for c in "ab12cd".chars() {
            assert!(push(&mut e, c));
        }
        assert!(!push(&mut e, 'e'), "a seventh digit");
        assert_eq!(e, "#AB12CD");
        let mut plain = String::from("1");
        assert!(!push(&mut plain, '#'), "the hash only goes first");
    }
}
