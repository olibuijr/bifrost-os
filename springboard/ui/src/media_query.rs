//! Media library queries: a query picks items from a library by property predicates, orders them by one grouping
//! key, and returns sections (a title and a range of positions per group). This mirrors the query, property
//! predicate, grouping and section types of the media player framework. Pure model: the caller supplies the
//! library records (an app's own index or a scan) and draws the results.
//!
//! Semantics: the predicates of one query combine with AND. Text comparisons ignore case. `Comparison::Contains`
//! on text matches when the item's value contains the predicate text; an empty predicate text matches every item.
//! Numeric properties support `EqualTo` only. Playlist membership matches when any playlist name equals the text.
//!
//! Gaps: no live updates while a query runs, no item collections (a group is a section over one sorted list),
//! no playlist grouping, no media kind property (songs and podcasts are not told apart), and no system library
//! access: the app passes in its records.

use std::ops::Range;

/// One library record with the properties a predicate can test. Unknown text is the empty string.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LibraryItem {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub composer: String,
    pub genre: String,
    pub podcast_title: String,
    pub playlists: Vec<String>,
    pub play_count: u32,
    /// 0 to 5, where 0 means unrated.
    pub rating: u8,
    pub duration_secs: f64,
    /// Position on the album, used to order songs within an album.
    pub track_number: u32,
}

impl LibraryItem {
    /// A record with only a title; every other property is empty or zero.
    pub fn new(title: &str) -> Self {
        Self { title: title.to_string(), ..Self::default() }
    }

    /// The rating clamped to 0 to 5.
    pub fn with_rating(mut self, rating: u8) -> Self {
        self.rating = rating.min(5);
        self
    }
}

/// A property a predicate can test.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Property {
    Title,
    Artist,
    Album,
    AlbumArtist,
    Composer,
    Genre,
    PodcastTitle,
    Playlist,
    PlayCount,
    Rating,
    PlaybackDuration,
}

impl Property {
    /// True for properties compared as text, false for the numeric ones.
    pub fn is_text(self) -> bool {
        !matches!(self, Property::PlayCount | Property::Rating | Property::PlaybackDuration)
    }
}

/// The value a predicate matches against: text for text properties, a number for numeric ones.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Text(String),
    Number(f64),
}

/// How a predicate compares an item's value with its own value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Comparison {
    EqualTo,
    Contains,
}

/// One filter: a property, the value to match and the comparison.
#[derive(Clone, Debug, PartialEq)]
pub struct Predicate {
    pub property: Property,
    pub value: Value,
    pub comparison: Comparison,
}

impl Predicate {
    /// A predicate with the default comparison, `EqualTo`.
    pub fn equal_to(property: Property, value: Value) -> Self {
        Self { property, value, comparison: Comparison::EqualTo }
    }

    /// A predicate with an explicit comparison.
    pub fn new(property: Property, value: Value, comparison: Comparison) -> Self {
        Self { property, value, comparison }
    }

    /// Whether one record passes this predicate. A value of the wrong kind for the property never matches.
    pub fn matches(&self, item: &LibraryItem) -> bool {
        match (&self.value, self.property.is_text()) {
            (Value::Text(want), true) => {
                let want = want.to_lowercase();
                let values: Vec<String> = match self.property {
                    Property::Playlist => item.playlists.iter().map(|p| p.to_lowercase()).collect(),
                    other => vec![text_of(item, other).to_lowercase()],
                };
                match self.comparison {
                    Comparison::EqualTo => values.iter().any(|v| *v == want),
                    Comparison::Contains => values.iter().any(|v| v.contains(&want)),
                }
            }
            (Value::Number(want), false) if self.comparison == Comparison::EqualTo => {
                let have = match self.property {
                    Property::PlayCount => item.play_count as f64,
                    Property::Rating => item.rating as f64,
                    _ => item.duration_secs,
                };
                (have - want).abs() < 1e-9
            }
            _ => false,
        }
    }
}

fn text_of(item: &LibraryItem, property: Property) -> &str {
    match property {
        Property::Title => &item.title,
        Property::Artist => &item.artist,
        Property::Album => &item.album,
        Property::AlbumArtist => &item.album_artist,
        Property::Composer => &item.composer,
        Property::Genre => &item.genre,
        Property::PodcastTitle => &item.podcast_title,
        Property::Playlist | Property::PlayCount | Property::Rating | Property::PlaybackDuration => "",
    }
}

/// The key a query orders and groups by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grouping {
    Title,
    Album,
    Artist,
    AlbumArtist,
    Composer,
    Genre,
    PodcastTitle,
}

impl Grouping {
    /// The text this record sorts under for the grouping.
    pub fn key<'a>(self, item: &'a LibraryItem) -> &'a str {
        match self {
            Grouping::Title => &item.title,
            Grouping::Album => &item.album,
            Grouping::Artist => &item.artist,
            Grouping::AlbumArtist => &item.album_artist,
            Grouping::Composer => &item.composer,
            Grouping::Genre => &item.genre,
            Grouping::PodcastTitle => &item.podcast_title,
        }
    }
}

/// A run of consecutive positions in the sorted result that share one grouping key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Section {
    /// The shared key; empty when the records have no value for the grouping.
    pub title: String,
    /// Positions in the list returned by [`Query::items`].
    pub range: Range<usize>,
}

/// A query: a set of predicates (AND) and at most one grouping. Without a grouping the result is ordered by title.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Query {
    filters: Vec<Predicate>,
    grouping: Option<Grouping>,
}

impl Query {
    /// A query with no filter and no grouping; it matches the whole library.
    pub fn new() -> Self {
        Self::default()
    }

    /// Presets that set the grouping, as the media player framework does for its convenience constructors.
    pub fn songs() -> Self {
        Self { filters: Vec::new(), grouping: Some(Grouping::Title) }
    }

    pub fn albums() -> Self {
        Self { filters: Vec::new(), grouping: Some(Grouping::Album) }
    }

    pub fn artists() -> Self {
        Self { filters: Vec::new(), grouping: Some(Grouping::Artist) }
    }

    pub fn composers() -> Self {
        Self { filters: Vec::new(), grouping: Some(Grouping::Composer) }
    }

    pub fn genres() -> Self {
        Self { filters: Vec::new(), grouping: Some(Grouping::Genre) }
    }

    pub fn grouping(&self) -> Option<Grouping> {
        self.grouping
    }

    pub fn set_grouping(&mut self, grouping: Option<Grouping>) {
        self.grouping = grouping;
    }

    pub fn filters(&self) -> &[Predicate] {
        &self.filters
    }

    /// Adds a predicate. The filters are a set: an equal predicate already present is not added twice, and the
    /// return value says whether the predicate was added.
    pub fn add_filter(&mut self, predicate: Predicate) -> bool {
        if self.filters.contains(&predicate) {
            return false;
        }
        self.filters.push(predicate);
        true
    }

    /// Removes an equal predicate; returns whether one was removed.
    pub fn remove_filter(&mut self, predicate: &Predicate) -> bool {
        let before = self.filters.len();
        self.filters.retain(|p| p != predicate);
        self.filters.len() != before
    }

    /// Whether one record passes every predicate.
    pub fn matches(&self, item: &LibraryItem) -> bool {
        self.filters.iter().all(|p| p.matches(item))
    }

    /// Indices into `library` of the matching records, ordered by the grouping key (case-insensitive), then by
    /// track number, then by title. Without a grouping the key is the title.
    pub fn items(&self, library: &[LibraryItem]) -> Vec<usize> {
        let grouping = self.grouping.unwrap_or(Grouping::Title);
        let mut hits: Vec<usize> = (0..library.len()).filter(|&i| self.matches(&library[i])).collect();
        hits.sort_by_cached_key(|&i| {
            let item = &library[i];
            (grouping.key(item).to_lowercase(), item.track_number, item.title.to_lowercase(), i)
        });
        hits
    }

    /// Sections over [`Query::items`]: each run of records with the same key (ignoring case) is one section.
    /// Without a grouping the whole result is one section titled by the empty string.
    pub fn sections(&self, library: &[LibraryItem]) -> Vec<Section> {
        let grouping = self.grouping.unwrap_or(Grouping::Title);
        let hits = self.items(library);
        let mut out: Vec<Section> = Vec::new();
        for (pos, &i) in hits.iter().enumerate() {
            let key = grouping.key(&library[i]);
            match out.last_mut() {
                Some(last) if last.title.to_lowercase() == key.to_lowercase() => last.range.end = pos + 1,
                _ => out.push(Section { title: key.to_string(), range: pos..pos + 1 }),
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song(title: &str, artist: &str, album: &str, track: u32) -> LibraryItem {
        LibraryItem {
            title: title.into(),
            artist: artist.into(),
            album: album.into(),
            track_number: track,
            ..LibraryItem::default()
        }
    }

    fn library() -> Vec<LibraryItem> {
        vec![
            song("Second", "Example Band", "Alpha", 2),
            song("First", "Example Band", "alpha", 1),
            song("Loose", "Other Act", "", 0),
            song("Third", "Example Band", "Beta", 1).with_rating(5),
        ]
    }

    #[test]
    fn text_equal_ignores_case_and_contains_matches_substrings() {
        let lib = library();
        let equal = Predicate::equal_to(Property::Artist, Value::Text("example band".into()));
        let hits: Vec<bool> = lib.iter().map(|i| equal.matches(i)).collect();
        assert_eq!(hits, vec![true, true, false, true]);

        let contains = Predicate::new(Property::Title, Value::Text("ir".into()), Comparison::Contains);
        let titles: Vec<&str> = lib.iter().filter(|i| contains.matches(i)).map(|i| i.title.as_str()).collect();
        assert_eq!(titles, vec!["First", "Third"]);

        let any = Predicate::new(Property::Title, Value::Text(String::new()), Comparison::Contains);
        assert!(lib.iter().all(|i| any.matches(i)));
    }

    #[test]
    fn numeric_properties_match_equal_only_and_reject_text_values() {
        let lib = library();
        let five = Predicate::equal_to(Property::Rating, Value::Number(5.0));
        let rated: Vec<&str> = lib.iter().filter(|i| five.matches(i)).map(|i| i.title.as_str()).collect();
        assert_eq!(rated, vec!["Third"]);

        let wrong_kind = Predicate::equal_to(Property::Rating, Value::Text("5".into()));
        assert!(!lib.iter().any(|i| wrong_kind.matches(i)));

        let range_like = Predicate::new(Property::PlayCount, Value::Number(0.0), Comparison::Contains);
        assert!(!lib.iter().any(|i| range_like.matches(i)));
    }

    #[test]
    fn playlist_filter_matches_any_membership() {
        let mut item = LibraryItem::new("Road");
        item.playlists = vec!["Driving".into(), "Gym".into()];
        let gym = Predicate::equal_to(Property::Playlist, Value::Text("gym".into()));
        assert!(gym.matches(&item));
        let office = Predicate::equal_to(Property::Playlist, Value::Text("Office".into()));
        assert!(!office.matches(&item));
    }

    #[test]
    fn filters_form_a_set_combined_with_and() {
        let mut q = Query::new();
        let artist = Predicate::equal_to(Property::Artist, Value::Text("Example Band".into()));
        let album = Predicate::equal_to(Property::Album, Value::Text("Alpha".into()));
        assert!(q.add_filter(artist.clone()));
        assert!(!q.add_filter(artist.clone()), "an equal predicate is not added twice");
        assert!(q.add_filter(album.clone()));
        assert_eq!(q.filters().len(), 2);

        let lib = library();
        let hits: Vec<&str> = q.items(&lib).iter().map(|&i| lib[i].title.as_str()).collect();
        assert_eq!(hits, vec!["First", "Second"]);

        assert!(q.remove_filter(&album));
        assert!(!q.remove_filter(&album));
        assert_eq!(q.items(&lib).len(), 3);
    }

    #[test]
    fn items_order_by_grouping_then_track_number() {
        let lib = library();
        let q = Query::albums();
        let titles: Vec<&str> = q.items(&lib).iter().map(|&i| lib[i].title.as_str()).collect();
        // "" (no album) sorts first, then "alpha" (case-insensitive) with track order 1 before 2, then "beta".
        assert_eq!(titles, vec!["Loose", "First", "Second", "Third"]);
    }

    #[test]
    fn sections_are_runs_of_one_key_over_the_sorted_result() {
        let lib = library();
        let sections = Query::albums().sections(&lib);
        assert_eq!(
            sections,
            vec![
                Section { title: String::new(), range: 0..1 },
                Section { title: "alpha".into(), range: 1..3 },
                Section { title: "Beta".into(), range: 3..4 },
            ]
        );

        let by_artist = Query::artists().sections(&lib);
        assert_eq!(by_artist.len(), 2);
        assert_eq!(by_artist[1].title, "Other Act");
        assert_eq!(by_artist[1].range, 3..4);
    }

    #[test]
    fn query_without_grouping_orders_by_title() {
        let lib = library();
        let titles: Vec<&str> = Query::new().items(&lib).iter().map(|&i| lib[i].title.as_str()).collect();
        assert_eq!(titles, vec!["First", "Loose", "Second", "Third"]);
        let sections = Query::new().sections(&lib);
        assert_eq!(sections, vec![Section { title: "First".into(), range: 0..1 }, Section { title: "Loose".into(), range: 1..2 }, Section { title: "Second".into(), range: 2..3 }, Section { title: "Third".into(), range: 3..4 }]);
    }

    #[test]
    fn rating_is_clamped_and_empty_library_gives_no_sections() {
        assert_eq!(LibraryItem::new("x").with_rating(9).rating, 5);
        assert!(Query::albums().sections(&[]).is_empty());
        assert!(Query::albums().items(&[]).is_empty());
    }
}
