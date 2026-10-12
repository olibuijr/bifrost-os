//! Data detectors for event notes: the street address written in an event's notes, which the editor offers to use as
//! the event's location. The rules are the shared detector's (`bifrost_app::controls::detect`), the same ones the
//! Notes app uses.
use bifrost_app::controls::detect::{self, Kind};

/// The first street address in `notes`, as it is written there (for example "Laugavegur 12").
pub fn address(notes: &str) -> Option<String> {
    detect::find(notes, &[Kind::Address]).first().map(|m| m.text(notes).to_string())
}

/// The first phone number in `notes`, as it is written there (for example "+354 555 1234"), offered as a copy action.
pub fn phone(notes: &str) -> Option<String> {
    detect::find(notes, &[Kind::Phone]).first().map(|m| m.text(notes).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_street_address_is_offered_as_written() {
        assert_eq!(address("Meet at Laugavegur 12, 101 Reykjavík"), Some("Laugavegur 12".to_string()));
    }

    #[test]
    fn notes_without_an_address_offer_nothing() {
        assert_eq!(address("Bring the slides"), None);
        assert_eq!(address(""), None);
    }

    #[test]
    fn the_first_phone_number_is_offered_as_written() {
        // ten digits with spaces and a country code: inside the 7 to 15 digit range
        assert_eq!(phone("Call +354 555 1234 before lunch"), Some("+354 555 1234".to_string()));
        assert_eq!(phone("Bring the slides"), None);
        // a short run of digits is an address number, not a phone number
        assert_eq!(phone("Laugavegur 12"), None);
    }
}
