//! Data detectors for event notes: the street address written in an event's notes, which the editor offers to use as
//! the event's location. The rules are the shared detector's (`bifrost_app::controls::detect`), the same ones the
//! Notes app uses.
use bifrost_app::controls::detect::{self, Kind};

/// The first street address in `notes`, as it is written there (for example "Laugavegur 12").
pub fn address(notes: &str) -> Option<String> {
    detect::find(notes, &[Kind::Address]).first().map(|m| m.text(notes).to_string())
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
}
