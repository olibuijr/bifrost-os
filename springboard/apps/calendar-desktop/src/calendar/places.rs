//! Show in Maps: the editor's location as a `maps:` link. The Maps app opens it as a search, and does its own place
//! lookup with the geocoder it already uses (a Nominatim-compatible service), so the calendar adds no geocoding of its own.

/// The `maps:` search link for `place`, with the text percent-encoded (`q` is the search, as Maps reads it). None for a
/// blank place, which has nothing to show.
pub fn search_link(place: &str) -> Option<String> {
    let place = place.trim();
    if place.is_empty() {
        return None;
    }
    Some(format!("maps://?q={}", encode(place)))
}

/// Percent-encodes everything but ASCII letters, digits and `-._~` (RFC 3986 unreserved characters).
fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_blank_location_has_no_link() {
        assert_eq!(search_link("   "), None);
    }

    #[test]
    fn the_location_is_the_search_text_of_a_maps_link() {
        assert_eq!(search_link(" Hallgrímskirkja, Reykjavík ").as_deref(), Some("maps://?q=Hallgr%C3%ADmskirkja%2C%20Reykjav%C3%ADk"));
        assert_eq!(search_link("Room 4a").as_deref(), Some("maps://?q=Room%204a"));
    }
}
