//! Point of interest categories and the filter that includes or excludes them. A category is its official raw name
//! (for example `cafe`, `evCharger`). The filter keeps either the set of categories to show or the set to hide, or
//! it is one of the two "all" filters. It decides which places a map, a local search or a search completion may
//! return; nothing here fetches places. The types mirror the official point of interest filter in outline.

use std::collections::BTreeSet;

/// The official point of interest category names, in the order the official reference groups them. Raw names are
/// compared exactly, so `Cafe` is not a category.
pub const KNOWN_CATEGORIES: &[&str] = &[
    "museum", "musicVenue", "theater", "library", "planetarium", "school", "university", "movieTheater",
    "nightlife", "fireStation", "hospital", "pharmacy", "police", "castle", "fortress", "landmark",
    "nationalMonument", "bakery", "brewery", "cafe", "distillery", "foodMarket", "restaurant", "winery",
    "animalService", "atm", "automotiveRepair", "bank", "beauty", "evCharger", "fitnessCenter", "laundry",
    "mailbox", "postOffice", "restroom", "spa", "store", "amusementPark", "aquarium", "beach", "campground",
    "fairground", "marina", "nationalPark", "park", "rvPark", "zoo", "baseball", "basketball", "bowling", "goKart",
    "golf", "hiking", "miniGolf", "rockClimbing", "skatePark", "skating", "skiing", "soccer", "stadium", "tennis",
    "volleyball", "airport", "carRental", "conventionCenter", "gasStation", "hotel", "parking", "publicTransport",
    "fishing", "kayaking", "surfing", "swimming", "airportTerminal", "automotiveDealership",
    "commercialVehicleDealership", "informationBooth", "motorbikeDealership", "picnicArea", "rangerStation",
    "restArea", "scenicView", "ticketOffice", "visitorCenter",
];

/// True when `raw` is one of the names in [`KNOWN_CATEGORIES`].
pub fn is_known_category(raw: &str) -> bool {
    KNOWN_CATEGORIES.contains(&raw)
}

/// Which categories a search or map may return.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PointOfInterestFilter {
    /// Every category, including names this module does not list.
    IncludingAll,
    /// No category at all.
    ExcludingAll,
    /// Only the categories in the set.
    Including(BTreeSet<String>),
    /// Every category except the ones in the set.
    Excluding(BTreeSet<String>),
}

impl Default for PointOfInterestFilter {
    fn default() -> Self {
        PointOfInterestFilter::IncludingAll
    }
}

impl PointOfInterestFilter {
    /// A filter that includes only `categories`. An empty list includes nothing.
    pub fn including<I, S>(categories: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        PointOfInterestFilter::Including(categories.into_iter().map(Into::into).collect())
    }

    /// A filter that excludes `categories` and includes the rest. An empty list excludes nothing.
    pub fn excluding<I, S>(categories: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        PointOfInterestFilter::Excluding(categories.into_iter().map(Into::into).collect())
    }

    /// True when places in `category` may be returned.
    pub fn includes(&self, category: &str) -> bool {
        match self {
            PointOfInterestFilter::IncludingAll => true,
            PointOfInterestFilter::ExcludingAll => false,
            PointOfInterestFilter::Including(set) => set.contains(category),
            PointOfInterestFilter::Excluding(set) => !set.contains(category),
        }
    }

    /// True when places in `category` are hidden. Always the opposite of [`PointOfInterestFilter::includes`].
    pub fn excludes(&self, category: &str) -> bool {
        !self.includes(category)
    }

    /// The categories from `categories` that this filter keeps, in their original order.
    pub fn apply<'a>(&self, categories: &[&'a str]) -> Vec<&'a str> {
        categories.iter().copied().filter(|c| self.includes(c)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_filter_includes_everything() {
        let f = PointOfInterestFilter::default();
        assert_eq!(f, PointOfInterestFilter::IncludingAll);
        assert!(f.includes("cafe"));
        assert!(f.includes("someFutureCategory"));
        assert!(!f.excludes("cafe"));
    }

    #[test]
    fn including_filter_keeps_only_the_listed_categories() {
        let f = PointOfInterestFilter::including(["cafe", "bank"]);
        assert!(f.includes("cafe"));
        assert!(f.includes("bank"));
        assert!(!f.includes("park"));
        assert!(f.excludes("park"));
    }

    #[test]
    fn excluding_filter_hides_only_the_listed_categories() {
        let f = PointOfInterestFilter::excluding(vec![String::from("atm")]);
        assert!(!f.includes("atm"));
        assert!(f.includes("bank"));
        assert!(f.includes("cafe"));
    }

    #[test]
    fn all_and_empty_sets_behave_as_their_names_say() {
        let none = PointOfInterestFilter::ExcludingAll;
        assert!(!none.includes("cafe"));
        // An empty including list is the same as excluding everything.
        let empty_including = PointOfInterestFilter::including(Vec::<String>::new());
        assert!(!empty_including.includes("cafe"));
        // An empty excluding list hides nothing.
        let empty_excluding = PointOfInterestFilter::excluding(Vec::<String>::new());
        assert!(empty_excluding.includes("cafe"));
    }

    #[test]
    fn apply_keeps_order_and_drops_hidden_categories() {
        let f = PointOfInterestFilter::including(["cafe", "park"]);
        let kept = f.apply(&["airport", "cafe", "park", "bank"]);
        assert_eq!(kept, vec!["cafe", "park"]);
        let all = PointOfInterestFilter::default().apply(&["bank", "atm"]);
        assert_eq!(all, vec!["bank", "atm"]);
    }

    #[test]
    fn known_categories_match_the_official_raw_names_exactly() {
        assert!(is_known_category("evCharger"));
        assert!(is_known_category("gasStation"));
        assert!(!is_known_category("Cafe"));
        assert!(!is_known_category("ev_charger"));
        assert!(!is_known_category(""));
        assert_eq!(KNOWN_CATEGORIES.len(), 84);
    }
}
