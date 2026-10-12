//! Place details: the name, address, phone number, website, time zone and coordinate of one place, and the rows a
//! detail sheet lists for it.
//!
//! The fields follow the attributes of the official map item: a name, a phone number, a URL, a time zone and the
//! location and address the item sits at. The address is the structured [`Address`] from `postal_address`. The model
//! holds no map, makes no request and opens nothing. [`MapItem::detail_rows`] returns the rows in a fixed order and
//! leaves out any attribute that is empty. The coordinate row is always present. The labels are this module's own
//! English text.

use crate::geo::Coordinate;
use crate::postal_address::Address;

/// One place with its details.
#[derive(Clone, Debug, PartialEq)]
pub struct MapItem {
    pub name: String,
    pub coordinate: Coordinate,
    pub address: Option<Address>,
    /// Trimmed. Empty when unknown.
    pub phone_number: String,
    /// Trimmed. Empty when unknown.
    pub url: String,
    /// Time zone name, such as "Atlantic/Reykjavik". Empty when unknown.
    pub time_zone: String,
}

/// Which attribute a detail row shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DetailKind {
    Address,
    PhoneNumber,
    Website,
    TimeZone,
    Coordinates,
}

/// One labelled line of a detail sheet.
#[derive(Clone, Debug, PartialEq)]
pub struct DetailRow {
    pub kind: DetailKind,
    pub label: &'static str,
    pub value: String,
}

impl MapItem {
    /// `None` when the name is blank after trimming. The name is stored trimmed.
    pub fn new(name: &str, coordinate: Coordinate) -> Option<Self> {
        let name = name.trim();
        (!name.is_empty()).then(|| MapItem {
            name: name.to_string(),
            coordinate,
            address: None,
            phone_number: String::new(),
            url: String::new(),
            time_zone: String::new(),
        })
    }

    pub fn with_address(mut self, address: Address) -> Self {
        self.address = Some(address);
        self
    }

    pub fn with_phone_number(mut self, phone: &str) -> Self {
        self.phone_number = phone.trim().to_string();
        self
    }

    pub fn with_url(mut self, url: &str) -> Self {
        self.url = url.trim().to_string();
        self
    }

    pub fn with_time_zone(mut self, zone: &str) -> Self {
        self.time_zone = zone.trim().to_string();
        self
    }

    /// The rows a detail sheet shows, in this order: address, phone number, website, time zone, coordinates. The
    /// address is written one line per part, as its country prints it. Empty attributes produce no row.
    pub fn detail_rows(&self) -> Vec<DetailRow> {
        let mut rows = Vec::new();
        if let Some(address) = &self.address {
            let text = address.format();
            if !text.is_empty() {
                rows.push(DetailRow { kind: DetailKind::Address, label: "address", value: text });
            }
        }
        let text_rows = [
            (DetailKind::PhoneNumber, "phone", &self.phone_number),
            (DetailKind::Website, "website", &self.url),
            (DetailKind::TimeZone, "time zone", &self.time_zone),
        ];
        for (kind, label, value) in text_rows {
            if !value.is_empty() {
                rows.push(DetailRow { kind, label, value: value.clone() });
            }
        }
        rows.push(DetailRow {
            kind: DetailKind::Coordinates,
            label: "coordinates",
            value: format!("{:.5}, {:.5}", self.coordinate.latitude, self.coordinate.longitude),
        });
        rows
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point() -> Coordinate {
        Coordinate::new(64.1466, -21.9426).unwrap()
    }

    fn reykjavik_address() -> Address {
        Address {
            street: "Austurbakki 2".into(),
            city: "Reykjavik".into(),
            postal_code: "101".into(),
            country: "Iceland".into(),
            iso_country_code: "IS".into(),
            ..Default::default()
        }
    }

    #[test]
    fn name_is_required_and_trimmed() {
        assert!(MapItem::new("   ", point()).is_none());
        assert_eq!(MapItem::new("  Harpa ", point()).unwrap().name, "Harpa");
    }

    #[test]
    fn name_only_item_shows_just_the_coordinates() {
        let rows = MapItem::new("Harpa", point()).unwrap().detail_rows();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].kind, DetailKind::Coordinates);
        assert_eq!(rows[0].label, "coordinates");
        assert_eq!(rows[0].value, "64.14660, -21.94260");
    }

    #[test]
    fn full_item_lists_rows_in_fixed_order() {
        let item = MapItem::new("Harpa", point())
            .unwrap()
            .with_time_zone(" Atlantic/Reykjavik ")
            .with_url("https://harpa.example/")
            .with_phone_number(" +354 555 0100 ")
            .with_address(reykjavik_address());
        let rows = item.detail_rows();
        let kinds: Vec<DetailKind> = rows.iter().map(|r| r.kind).collect();
        assert_eq!(
            kinds,
            vec![
                DetailKind::Address,
                DetailKind::PhoneNumber,
                DetailKind::Website,
                DetailKind::TimeZone,
                DetailKind::Coordinates,
            ]
        );
        assert_eq!(rows[0].value, "Austurbakki 2\n101 Reykjavik\nIceland");
        assert_eq!(rows[1].value, "+354 555 0100");
        assert_eq!(rows[2].value, "https://harpa.example/");
        assert_eq!(rows[3].value, "Atlantic/Reykjavik");
    }

    #[test]
    fn blank_attributes_produce_no_row() {
        let item = MapItem::new("Harpa", point()).unwrap().with_phone_number("   ").with_url("");
        let rows = item.detail_rows();
        assert!(rows.iter().all(|r| r.kind != DetailKind::PhoneNumber));
        assert!(rows.iter().all(|r| r.kind != DetailKind::Website));
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn coordinates_use_five_decimals_with_sign() {
        let c = Coordinate::new(0.0, -0.5).unwrap();
        let rows = MapItem::new("Origin", c).unwrap().detail_rows();
        assert_eq!(rows[0].value, "0.00000, -0.50000");
    }

    #[test]
    fn address_with_no_parts_produces_no_row() {
        let item = MapItem::new("Harpa", point()).unwrap().with_address(Address::default());
        assert!(item.detail_rows().iter().all(|r| r.kind != DetailKind::Address));
    }
}
