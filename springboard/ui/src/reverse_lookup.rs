//! Nearest-place lookup: turns a coordinate into the closest named place from a list the caller holds.
//!
//! The index is built from places the caller already has, for example the results of a place search. It makes no
//! network request and keeps no cache. Distances are great-circle metres from `Coordinate::distance_m`. Results that
//! are equally far keep the order in which the places were added.

use crate::geo::Coordinate;

/// A named place with a region label, such as a town and its county, at a coordinate.
#[derive(Clone, Debug, PartialEq)]
pub struct Place {
    pub name: String,
    pub region: String,
    pub coordinate: Coordinate,
}

/// A place found near a coordinate, with its distance from that coordinate in metres.
#[derive(Clone, Debug, PartialEq)]
pub struct NearbyPlace {
    pub place: Place,
    pub distance_m: f64,
}

/// A set of places that can be searched by distance.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlaceIndex {
    places: Vec<Place>,
}

impl PlaceIndex {
    pub fn new() -> Self {
        Self::default()
    }

    /// Builds an index from places, kept in the order given.
    pub fn from_places(places: Vec<Place>) -> Self {
        PlaceIndex { places }
    }

    /// Adds a place at the end of the list.
    pub fn add(&mut self, place: Place) {
        self.places.push(place);
    }

    pub fn len(&self) -> usize {
        self.places.len()
    }

    pub fn is_empty(&self) -> bool {
        self.places.is_empty()
    }

    /// The closest place within `max_m` metres of `c` (the limit itself counts). `None` when no place is that close,
    /// or when `max_m` is NaN or negative. An infinite limit accepts every place.
    pub fn nearest(&self, c: Coordinate, max_m: f64) -> Option<NearbyPlace> {
        if max_m.is_nan() || max_m < 0.0 {
            return None;
        }
        let mut best: Option<NearbyPlace> = None;
        for place in &self.places {
            let d = place.coordinate.distance_m(c);
            if d > max_m {
                continue;
            }
            if best.as_ref().map_or(true, |b| d < b.distance_m) {
                best = Some(NearbyPlace { place: place.clone(), distance_m: d });
            }
        }
        best
    }

    /// Every place within `max_m` metres of `c`, nearest first. Equal distances keep the insertion order.
    pub fn within(&self, c: Coordinate, max_m: f64) -> Vec<NearbyPlace> {
        if max_m.is_nan() || max_m < 0.0 {
            return Vec::new();
        }
        let mut found: Vec<NearbyPlace> = self
            .places
            .iter()
            .filter_map(|p| {
                let d = p.coordinate.distance_m(c);
                (d <= max_m).then(|| NearbyPlace { place: p.clone(), distance_m: d })
            })
            .collect();
        found.sort_by(|a, b| a.distance_m.total_cmp(&b.distance_m));
        found
    }
}

/// A one-line label for a place: "Name, Region", or only the name when the region is empty. Surrounding spaces are
/// removed from both parts.
pub fn address_line(place: &Place) -> String {
    let name = place.name.trim();
    let region = place.region.trim();
    if region.is_empty() {
        name.to_string()
    } else {
        format!("{name}, {region}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(latitude: f64, longitude: f64) -> Coordinate {
        Coordinate::new(latitude, longitude).expect("valid test coordinate")
    }

    fn place(name: &str, region: &str, latitude: f64, longitude: f64) -> Place {
        Place { name: name.to_string(), region: region.to_string(), coordinate: c(latitude, longitude) }
    }

    fn two_towns() -> PlaceIndex {
        // A is at 64.0 N and B is 0.1 degrees of latitude further north, about 11.1 km apart.
        PlaceIndex::from_places(vec![place("A", "South", 64.0, -22.0), place("B", "North", 64.1, -22.0)])
    }

    #[test]
    fn nearest_picks_the_closest_place_within_the_limit() {
        let index = two_towns();
        // The query is 0.01 degrees of latitude from B (about 1112 m) and 0.09 degrees from A (about 10 km).
        let query = c(64.09, -22.0);
        let found = index.nearest(query, 5000.0).expect("B is within 5 km");
        assert_eq!(found.place.name, "B");
        assert!((1100.0..1125.0).contains(&found.distance_m), "distance was {}", found.distance_m);
    }

    #[test]
    fn nearest_returns_none_when_every_place_is_beyond_the_limit() {
        let index = two_towns();
        assert!(index.nearest(c(64.09, -22.0), 500.0).is_none());
        assert!(index.nearest(c(64.09, -22.0), f64::NAN).is_none());
        assert!(index.nearest(c(64.09, -22.0), -1.0).is_none());
        let everything = index.nearest(c(64.09, -22.0), f64::INFINITY).expect("no limit");
        assert_eq!(everything.place.name, "B");
    }

    #[test]
    fn within_lists_places_nearest_first() {
        let index = two_towns();
        let found = index.within(c(64.09, -22.0), 20_000.0);
        let names: Vec<&str> = found.iter().map(|n| n.place.name.as_str()).collect();
        assert_eq!(names, vec!["B", "A"]);
        assert!(found[0].distance_m <= found[1].distance_m);
        assert!(index.within(c(64.09, -22.0), 500.0).is_empty());
    }

    #[test]
    fn equal_distances_keep_the_insertion_order() {
        let index = PlaceIndex::from_places(vec![place("First", "", 10.0, 10.0), place("Second", "", 10.0, 10.0)]);
        let found = index.within(c(10.0, 10.0), 1.0);
        let names: Vec<&str> = found.iter().map(|n| n.place.name.as_str()).collect();
        assert_eq!(names, vec!["First", "Second"]);
        let nearest = index.nearest(c(10.0, 10.0), 1.0).expect("both are at zero distance");
        assert_eq!(nearest.place.name, "First");
        assert_eq!(nearest.distance_m, 0.0);
    }

    #[test]
    fn empty_index_finds_nothing_and_add_grows_it() {
        let mut index = PlaceIndex::new();
        assert!(index.is_empty());
        assert!(index.nearest(c(0.0, 0.0), 1e9).is_none());
        index.add(place("Origin", "Gulf of Guinea", 0.0, 0.0));
        assert_eq!(index.len(), 1);
        assert_eq!(index.nearest(c(0.0, 0.0), 1.0).expect("exact hit").place.name, "Origin");
    }

    #[test]
    fn address_line_joins_name_and_region_and_trims_spaces() {
        assert_eq!(address_line(&place(" Akureyri ", " North ", 65.7, -18.1)), "Akureyri, North");
        assert_eq!(address_line(&place("Akureyri", "  ", 65.7, -18.1)), "Akureyri");
    }
}
