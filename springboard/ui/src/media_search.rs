//! Media search requests: a person's audio search or playback request, classified into criteria, and matched
//! against the app's own catalogue. This mirrors the audio search type of the media intents framework (its
//! criteria enum and the intent value query that returns matching entities). Pure model: the caller supplies the
//! catalogue and draws the results. No speech recognition and no system delivery happen here.
use crate::fuzzy::fuzzy_match;

/// How a request was classified: free text, a link, or no specific criteria.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MediaCriteria {
    /// The request text, such as "energetic music" or "Example song by Example artist".
    SearchQuery(String),
    /// A vague request ("play some music") or an empty one. The app answers with recommendations.
    Unspecified,
    /// A link that names one catalogue item by its id.
    Url(String),
}

impl MediaCriteria {
    /// Classify raw request text: blank is `Unspecified`, an `http://` or `https://` link is `Url`, anything
    /// else is a `SearchQuery` with surrounding space removed.
    pub fn classify(text: &str) -> Self {
        let t = text.trim();
        if t.is_empty() {
            Self::Unspecified
        } else if t.starts_with("https://") || t.starts_with("http://") {
            Self::Url(t.to_string())
        } else {
            Self::SearchQuery(t.to_string())
        }
    }
}

/// The audio content kinds the catalogue can hold (song, album, artist, podcast episode).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaKind {
    Song,
    Album,
    Artist,
    Episode,
}

/// One catalogue entry an app can return for a request (an audio entity with its title and credits).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MediaItem {
    pub id: String,
    pub kind: MediaKind,
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
}

impl MediaItem {
    pub fn new(id: &str, kind: MediaKind, title: &str) -> Self {
        Self { id: id.to_string(), kind, title: title.to_string(), artist: None, album: None }
    }

    fn fields(&self) -> impl Iterator<Item = &str> {
        [Some(self.title.as_str()), self.artist.as_deref(), self.album.as_deref()].into_iter().flatten()
    }

    /// Sum of each query token's best field score, or `None` when some token matches no field.
    fn score(&self, tokens: &[String]) -> Option<i32> {
        let mut total = 0;
        for t in tokens {
            total += self.fields().filter_map(|f| fuzzy_match(t, f)).map(|m| m.score).max()?;
        }
        Some(total)
    }
}

/// A request: the classified criteria of one audio search or playback request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MediaSearch {
    pub criteria: MediaCriteria,
}

impl MediaSearch {
    pub fn new(criteria: MediaCriteria) -> Self {
        Self { criteria }
    }
}

/// Filler words that describe the request rather than the content ("play some ... by ...").
const FILLER: [&str; 9] = ["play", "some", "by", "the", "a", "an", "from", "of", "my"];

fn tokens(query: &str) -> Vec<String> {
    query
        .split_whitespace()
        .map(str::to_lowercase)
        .filter(|w| !FILLER.contains(&w.as_str()))
        .collect()
}

/// The catalogue items that answer `search`, best first, at most `limit` of them.
///
/// A search query matches an item when every content word matches some field (title, artist or album) with the
/// fuzzy matcher; items are ranked by their summed scores, ties keep catalogue order. A query with no content words
/// and an `Unspecified` request return the first items of the catalogue as recommendations. A `Url` returns the
/// item whose id equals the link.
pub fn values_for<'a>(search: &MediaSearch, catalogue: &'a [MediaItem], limit: usize) -> Vec<&'a MediaItem> {
    if limit == 0 {
        return Vec::new();
    }
    let words = match &search.criteria {
        MediaCriteria::Url(link) => return catalogue.iter().filter(|i| i.id == *link).take(limit).collect(),
        MediaCriteria::SearchQuery(q) => tokens(q),
        MediaCriteria::Unspecified => Vec::new(),
    };
    if words.is_empty() {
        return catalogue.iter().take(limit).collect();
    }
    let mut scored: Vec<(i32, usize, &MediaItem)> = catalogue
        .iter()
        .enumerate()
        .filter_map(|(i, item)| item.score(&words).map(|s| (s, i, item)))
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    scored.into_iter().take(limit).map(|(_, _, item)| item).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song(id: &str, title: &str, artist: &str) -> MediaItem {
        let mut m = MediaItem::new(id, MediaKind::Song, title);
        m.artist = Some(artist.to_string());
        m
    }

    fn ids(v: &[&MediaItem]) -> Vec<String> {
        v.iter().map(|i| i.id.clone()).collect()
    }

    #[test]
    fn classify_blank_link_and_text() {
        assert_eq!(MediaCriteria::classify(""), MediaCriteria::Unspecified);
        assert_eq!(MediaCriteria::classify("   "), MediaCriteria::Unspecified);
        assert_eq!(MediaCriteria::classify("  https://music.example/a/1 "), MediaCriteria::Url("https://music.example/a/1".into()));
        assert_eq!(MediaCriteria::classify(" energetic music "), MediaCriteria::SearchQuery("energetic music".into()));
    }

    #[test]
    fn multi_word_query_matches_title_and_artist() {
        let cat = vec![song("a1", "Example song", "Example artist"), song("b2", "Other tune", "Someone")];
        let search = MediaSearch::new(MediaCriteria::classify("Example song by Example artist"));
        assert_eq!(ids(&values_for(&search, &cat, 10)), vec!["a1"]);
    }

    #[test]
    fn item_missing_a_content_word_is_excluded() {
        let cat = vec![song("r", "Rain", "Ambient"), song("j", "Jazz Night", "Quartet")];
        let search = MediaSearch::new(MediaCriteria::SearchQuery("rain jazz".into()));
        assert!(values_for(&search, &cat, 10).is_empty());
    }

    #[test]
    fn closer_match_ranks_first() {
        let cat = vec![song("h", "Heavy rain", "Band"), song("r", "Rain", "Band")];
        let search = MediaSearch::new(MediaCriteria::SearchQuery("rain".into()));
        assert_eq!(ids(&values_for(&search, &cat, 10)), vec!["r", "h"]);
    }

    #[test]
    fn unspecified_returns_recommendations_in_catalogue_order() {
        let cat = vec![song("1", "One", "A"), song("2", "Two", "B"), song("3", "Three", "C")];
        let search = MediaSearch::new(MediaCriteria::Unspecified);
        assert_eq!(ids(&values_for(&search, &cat, 2)), vec!["1", "2"]);
        // A query made only of filler words is treated the same way.
        let filler = MediaSearch::new(MediaCriteria::classify("play some"));
        assert_eq!(ids(&values_for(&filler, &cat, 1)), vec!["1"]);
    }

    #[test]
    fn url_selects_the_item_with_that_id() {
        let cat = vec![song("1", "One", "A"), song("2", "Two", "B")];
        let search = MediaSearch::new(MediaCriteria::Url("2".into()));
        assert_eq!(ids(&values_for(&search, &cat, 10)), vec!["2"]);
    }

    #[test]
    fn zero_limit_returns_nothing() {
        let cat = vec![song("1", "One", "A")];
        let search = MediaSearch::new(MediaCriteria::Unspecified);
        assert!(values_for(&search, &cat, 0).is_empty());
    }
}
