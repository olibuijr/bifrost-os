//! A place shown in a detail sheet: the map item of the official mapping framework in outline. It holds a placemark
//! (a name, a region and a coordinate, as [`Place`]), an optional phone number, an optional web address, an optional
//! point-of-interest category and a flag for the user's own location. The setters check their input and return
//! `None` when it is not valid, so a stored item is always well formed. Nothing here opens a map or calls a service.

use crate::geo::Coordinate;
use crate::poi_filter::is_known_category;
use crate::reverse_lookup::Place;

/// A place with the details a detail sheet shows.
#[derive(Clone, Debug, PartialEq)]
pub struct MapItem {
    pub placemark: Place,
    pub phone_number: Option<String>,
    pub url: Option<String>,
    pub category: Option<String>,
    pub is_current_location: bool,
}

impl MapItem {
    /// An item for a placemark, with no phone, address or category.
    pub fn new(placemark: Place) -> Self {
        MapItem { placemark, phone_number: None, url: None, category: None, is_current_location: false }
    }

    /// An item for the user's own position: an empty name and region at `coordinate`.
    pub fn current_location(coordinate: Coordinate) -> Self {
        let placemark = Place { name: String::new(), region: String::new(), coordinate };
        MapItem { is_current_location: true, ..MapItem::new(placemark) }
    }

    /// Sets the phone number. The text is trimmed. It may hold digits, spaces, `-`, `(` and `)`, and a leading `+`,
    /// and it needs at least seven digits. Any other text gives `None`.
    pub fn with_phone(mut self, phone: &str) -> Option<Self> {
        let phone = phone.trim();
        let allowed = phone
            .char_indices()
            .all(|(i, c)| c.is_ascii_digit() || matches!(c, ' ' | '-' | '(' | ')') || (c == '+' && i == 0));
        let digits = phone.chars().filter(|c| c.is_ascii_digit()).count();
        if !allowed || digits < 7 {
            return None;
        }
        self.phone_number = Some(phone.to_string());
        Some(self)
    }

    /// Sets the web address. It must start with `http://` or `https://`, have a host after the scheme, and contain
    /// no spaces.
    pub fn with_url(mut self, url: &str) -> Option<Self> {
        let rest = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://"))?;
        let host = rest.split('/').next().unwrap_or("");
        if host.is_empty() || url.contains(char::is_whitespace) {
            return None;
        }
        self.url = Some(url.to_string());
        Some(self)
    }

    /// Sets the point-of-interest category. It must be one of the official raw names in `poi_filter`, compared
    /// exactly.
    pub fn with_category(mut self, raw: &str) -> Option<Self> {
        if !is_known_category(raw) {
            return None;
        }
        self.category = Some(raw.to_string());
        Some(self)
    }

    /// Metres from `from` to this item's coordinate, along a great circle.
    pub fn distance_m(&self, from: Coordinate) -> f64 {
        from.distance_m(self.placemark.coordinate)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lab() -> Place {
        Place {
            name: "Lab".into(),
            region: "Reykjavik".into(),
            coordinate: Coordinate::new(64.0, -22.0).unwrap(),
        }
    }

    #[test]
    fn new_item_has_no_details() {
        let item = MapItem::new(lab());
        assert_eq!(item.placemark.name, "Lab");
        assert_eq!(item.phone_number, None);
        assert_eq!(item.url, None);
        assert_eq!(item.category, None);
        assert!(!item.is_current_location);
    }

    #[test]
    fn accepts_a_phone_with_country_code_and_trims_it() {
        let item = MapItem::new(lab()).with_phone("  +354 555 1234 ").unwrap();
        assert_eq!(item.phone_number.as_deref(), Some("+354 555 1234"));
    }

    #[test]
    fn rejects_a_phone_with_letters_a_late_plus_or_too_few_digits() {
        assert!(MapItem::new(lab()).with_phone("call me").is_none());
        assert!(MapItem::new(lab()).with_phone("354+555 1234").is_none());
        assert!(MapItem::new(lab()).with_phone("12345").is_none());
    }

    #[test]
    fn accepts_http_and_https_addresses_with_a_host() {
        assert!(MapItem::new(lab()).with_url("https://example.com/menu").is_some());
        assert!(MapItem::new(lab()).with_url("http://example.com").is_some());
        assert!(MapItem::new(lab()).with_url("ftp://example.com").is_none());
        assert!(MapItem::new(lab()).with_url("https://").is_none());
        assert!(MapItem::new(lab()).with_url("https://exa mple.com").is_none());
    }

    #[test]
    fn category_must_be_an_official_raw_name() {
        let item = MapItem::new(lab()).with_category("cafe").unwrap();
        assert_eq!(item.category.as_deref(), Some("cafe"));
        assert!(MapItem::new(lab()).with_category("Cafe").is_none());
    }

    #[test]
    fn current_location_is_flagged_and_zero_metres_from_itself() {
        let here = Coordinate::new(64.0, -22.0).unwrap();
        let item = MapItem::current_location(here);
        assert!(item.is_current_location);
        assert_eq!(item.placemark.name, "");
        assert!(item.distance_m(here).abs() < 1e-6);
    }
}
