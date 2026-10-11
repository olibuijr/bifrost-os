//! Media playback requests: a spoken or typed request to play, queue or find audio, parsed into a verb and its
//! criteria, resolved against the app's catalogue, and turned into the action the app takes plus the short
//! dialog that answers the person. This mirrors the audio playback requests of the media intents framework (play,
//! add to queue, search) and its response dialog. Pure model: the caller supplies the catalogue, starts playback
//! and shows the dialog. No speech recognition and no speech synthesis happen here.
use crate::media_search::{values_for, MediaCriteria, MediaItem, MediaSearch};

/// What the person asked the app to do with the matching audio.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaVerb {
    /// Start playing the best match (the rest of the matches become the queue).
    Play,
    /// Append the matches to the play queue without interrupting playback.
    AddToQueue,
    /// Show the matches so the person can choose one.
    Search,
}

/// A parsed request: the verb and the criteria that name the content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlaybackRequest {
    pub verb: MediaVerb,
    pub search: MediaSearch,
}

/// What the app does for a request, after the catalogue is searched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlaybackAction<'a> {
    /// Play the first item; the others follow it in the queue.
    Play(Vec<&'a MediaItem>),
    /// Append these items to the queue.
    Enqueue(Vec<&'a MediaItem>),
    /// Show these items as results.
    Show(Vec<&'a MediaItem>),
    /// Nothing in the catalogue answers the request.
    NoMatch,
}

/// Case-insensitive prefix strip for ASCII prefixes; `None` when the text does not start with the prefix.
fn strip_prefix_ci<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let head = text.get(..prefix.len())?;
    if head.eq_ignore_ascii_case(prefix) {
        Some(&text[prefix.len()..])
    } else {
        None
    }
}

/// Case-insensitive suffix strip for ASCII suffixes; `None` when the text does not end with the suffix.
fn strip_suffix_ci<'a>(text: &'a str, suffix: &str) -> Option<&'a str> {
    let at = text.len().checked_sub(suffix.len())?;
    if text.is_char_boundary(at) && text[at..].eq_ignore_ascii_case(suffix) {
        Some(&text[..at])
    } else {
        None
    }
}

/// The name of one item for the dialog: its title, and the artist when there is one.
fn describe(item: &MediaItem) -> String {
    match &item.artist {
        Some(artist) => format!("{} by {}", item.title, artist),
        None => item.title.clone(),
    }
}

fn count_word(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

impl PlaybackRequest {
    fn new(verb: MediaVerb, text: &str) -> Self {
        Self { verb, search: MediaSearch::new(MediaCriteria::classify(text)) }
    }

    /// Parse request text. Recognised forms (case-insensitive verbs):
    /// `add X to queue` and `queue X` (AddToQueue); `search for X`, `search X`, `find X`, `look up X` (Search);
    /// `play X` and `play` (Play). Text with no verb is a Play request for the whole text.
    pub fn parse(text: &str) -> Self {
        let t = text.trim();
        if let Some(rest) = strip_prefix_ci(t, "add ") {
            if let Some(body) = strip_suffix_ci(rest, " to queue") {
                return Self::new(MediaVerb::AddToQueue, body);
            }
        }
        if let Some(rest) = strip_prefix_ci(t, "queue ") {
            return Self::new(MediaVerb::AddToQueue, rest);
        }
        for prefix in ["search for ", "search ", "find ", "look up "] {
            if let Some(rest) = strip_prefix_ci(t, prefix) {
                return Self::new(MediaVerb::Search, rest);
            }
        }
        if let Some(rest) = strip_prefix_ci(t, "play ") {
            return Self::new(MediaVerb::Play, rest);
        }
        if t.eq_ignore_ascii_case("play") {
            return Self::new(MediaVerb::Play, "");
        }
        Self::new(MediaVerb::Play, t)
    }

    /// Search the catalogue for this request and choose the action, with at most `limit` items.
    pub fn plan<'a>(&self, catalogue: &'a [MediaItem], limit: usize) -> PlaybackAction<'a> {
        let items = values_for(&self.search, catalogue, limit);
        if items.is_empty() {
            return PlaybackAction::NoMatch;
        }
        match self.verb {
            MediaVerb::Play => PlaybackAction::Play(items),
            MediaVerb::AddToQueue => PlaybackAction::Enqueue(items),
            MediaVerb::Search => PlaybackAction::Show(items),
        }
    }

    /// The dialog that answers the person for an action of this request.
    pub fn dialog(&self, action: &PlaybackAction<'_>) -> String {
        match action {
            PlaybackAction::Play(items) => match items.first() {
                Some(first) => format!("Playing {}.", describe(first)),
                None => self.no_match_dialog(),
            },
            PlaybackAction::Enqueue(items) => match items.as_slice() {
                [] => self.no_match_dialog(),
                [one] => format!("Added {} to the queue.", describe(one)),
                many => format!("Added {} to the queue.", count_word(many.len(), "item", "items")),
            },
            PlaybackAction::Show(items) => {
                format!("Found {}.", count_word(items.len(), "result", "results"))
            }
            PlaybackAction::NoMatch => self.no_match_dialog(),
        }
    }

    fn no_match_dialog(&self) -> String {
        match &self.search.criteria {
            MediaCriteria::SearchQuery(q) => format!("No results for {q}."),
            MediaCriteria::Url(_) => "That link does not match anything in the library.".to_string(),
            MediaCriteria::Unspecified => "Nothing matched that request.".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media_search::{MediaCriteria, MediaItem, MediaKind, MediaSearch};

    fn song(id: &str, title: &str, artist: &str) -> MediaItem {
        let mut m = MediaItem::new(id, MediaKind::Song, title);
        m.artist = Some(artist.to_string());
        m
    }

    /// Rain by Ambient, Jazz Night by Quartet, Heavy rain by Band.
    fn catalogue() -> Vec<MediaItem> {
        vec![song("r", "Rain", "Ambient"), song("j", "Jazz Night", "Quartet"), song("h", "Heavy rain", "Band")]
    }

    #[test]
    fn parse_recognises_each_verb() {
        let q = |s: &str| MediaCriteria::SearchQuery(s.into());
        assert_eq!(PlaybackRequest::parse("add Rain to queue"), PlaybackRequest { verb: MediaVerb::AddToQueue, search: MediaSearch::new(q("Rain")) });
        assert_eq!(PlaybackRequest::parse("queue rain").verb, MediaVerb::AddToQueue);
        assert_eq!(PlaybackRequest::parse("Search for Jazz").search.criteria, q("Jazz"));
        assert_eq!(PlaybackRequest::parse("Search for Jazz").verb, MediaVerb::Search);
        assert_eq!(PlaybackRequest::parse("find jazz").verb, MediaVerb::Search);
        assert_eq!(PlaybackRequest::parse("play").search.criteria, MediaCriteria::Unspecified);
        assert_eq!(PlaybackRequest::parse("Play Jazz Night").search.criteria, q("Jazz Night"));
        assert_eq!(PlaybackRequest::parse("energetic music").verb, MediaVerb::Play);
    }

    #[test]
    fn play_plans_the_matching_item() {
        let cat = catalogue();
        let req = PlaybackRequest::parse("play jazz");
        assert_eq!(req.plan(&cat, 10), PlaybackAction::Play(vec![&cat[1]]));
        // An unspecified play request starts from the first catalogue item.
        let any = PlaybackRequest::parse("play");
        assert_eq!(any.dialog(&any.plan(&cat, 10)), "Playing Rain by Ambient.");
    }

    #[test]
    fn enqueue_dialog_names_one_item_or_counts_several() {
        let cat = catalogue();
        let one = PlaybackRequest::parse("add Jazz to queue");
        assert_eq!(one.dialog(&one.plan(&cat, 10)), "Added Jazz Night by Quartet to the queue.");
        // "rain" matches both "Rain" and "Heavy rain".
        let two = PlaybackRequest::parse("queue rain");
        assert_eq!(two.dialog(&two.plan(&cat, 10)), "Added 2 items to the queue.");
    }

    #[test]
    fn search_dialog_counts_results_and_respects_the_limit() {
        let cat = catalogue();
        let req = PlaybackRequest::parse("find rain");
        assert_eq!(req.dialog(&req.plan(&cat, 10)), "Found 2 results.");
        assert_eq!(req.dialog(&req.plan(&cat, 1)), "Found 1 result.");
    }

    #[test]
    fn no_match_reports_the_query_or_a_generic_line() {
        let cat = catalogue();
        let zebra = PlaybackRequest::parse("play zebra");
        assert_eq!(zebra.plan(&cat, 10), PlaybackAction::NoMatch);
        assert_eq!(zebra.dialog(&PlaybackAction::NoMatch), "No results for zebra.");
        let empty = PlaybackRequest::parse("play");
        assert_eq!(empty.dialog(&empty.plan(&[], 10)), "Nothing matched that request.");
    }

    #[test]
    fn zero_limit_yields_no_match_and_strip_helpers_respect_boundaries() {
        let cat = catalogue();
        let req = PlaybackRequest::parse("play rain");
        assert_eq!(req.plan(&cat, 0), PlaybackAction::NoMatch);
        assert_eq!(strip_suffix_ci("Rain TO QUEUE", " to queue"), Some("Rain"));
        assert_eq!(strip_suffix_ci("ab", " to queue"), None);
        assert_eq!(strip_prefix_ci("é rain", "play "), None);
    }
}
