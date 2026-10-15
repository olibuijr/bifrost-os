//! Typed now-playing content: the media type, the duration and the metadata an item is described with.
//! Mirrors `MediaType`, `MediaDuration` and the fields that `MediaContentRepresentable` and `MusicContent`
//! share (identifier, title, artist, album, type, duration, explicit flag). Pure data and arithmetic: no artwork
//! bytes and no delivery to the shell; the audio service does not carry these fields yet.

/// `MediaType`: audio-only or video content.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MediaType {
    Audio,
    Video,
}

/// `MediaDuration`: a known length, a live stream, or content with no defined end.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MediaDuration {
    /// `continuous`: no defined duration and not a live broadcast.
    Continuous,
    /// `finite(_:)`: a known length in seconds.
    Finite(f64),
    /// `live`: a live or streaming item without a defined duration.
    Live,
}

/// The typed content classes an item can be described as: `MusicContent`, `PodcastContent`, `MovieContent`,
/// `TVShowContent`, `BookContent`, `RadioContent` and `GenericContent`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ContentKind {
    #[default]
    Generic,
    Music,
    Podcast,
    Movie,
    TvShow,
    Book,
    Radio,
}

impl ContentKind {
    /// The name the shell's now playing service reads for this kind.
    pub fn name(self) -> &'static str {
        match self {
            ContentKind::Generic => "generic",
            ContentKind::Music => "music",
            ContentKind::Podcast => "podcast",
            ContentKind::Movie => "movie",
            ContentKind::TvShow => "tv_show",
            ContentKind::Book => "book",
            ContentKind::Radio => "radio",
        }
    }
}

/// The cases `MediaContent::new` and `MediaDuration::finite_seconds` reject.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContentError {
    /// The identifier is empty after trimming.
    EmptyId,
    /// The title is empty after trimming.
    EmptyTitle,
    /// A finite length is not a positive, finite number of seconds.
    InvalidDuration,
}

impl MediaDuration {
    /// A finite length in seconds. Zero, negative, NaN and infinite lengths are rejected.
    pub fn finite_seconds(seconds: f64) -> Result<Self, ContentError> {
        if seconds.is_finite() && seconds > 0.0 {
            Ok(MediaDuration::Finite(seconds))
        } else {
            Err(ContentError::InvalidDuration)
        }
    }

    /// Only content with a known end can be scrubbed or shown as a fraction of its length.
    pub fn is_seekable(&self) -> bool {
        matches!(self, MediaDuration::Finite(_))
    }

    /// The text a card shows for the length: `3:45`, `1:02:05`, `Live`, or nothing for continuous content.
    pub fn label(&self) -> Option<String> {
        match *self {
            MediaDuration::Continuous => None,
            MediaDuration::Live => Some("Live".to_string()),
            MediaDuration::Finite(seconds) => {
                let total = seconds.round() as u64;
                let (h, m, s) = (total / 3600, (total % 3600) / 60, total % 60);
                if h > 0 {
                    Some(format!("{}:{:02}:{:02}", h, m, s))
                } else {
                    Some(format!("{}:{:02}", m, s))
                }
            }
        }
    }
}

/// The metadata of one playable item. `artist` and `album` are optional; empty text is stored as `None`.
#[derive(Clone, Debug, PartialEq)]
pub struct MediaContent {
    pub id: String,
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub media_type: MediaType,
    pub duration: MediaDuration,
    pub is_explicit: bool,
    pub kind: ContentKind,
}

impl MediaContent {
    /// An item with an identifier and a title. Both are trimmed and must not be empty.
    pub fn new(id: &str, title: &str, media_type: MediaType, duration: MediaDuration) -> Result<Self, ContentError> {
        let id = id.trim();
        let title = title.trim();
        if id.is_empty() {
            return Err(ContentError::EmptyId);
        }
        if title.is_empty() {
            return Err(ContentError::EmptyTitle);
        }
        Ok(MediaContent {
            id: id.to_string(),
            title: title.to_string(),
            artist: None,
            album: None,
            media_type,
            duration,
            is_explicit: false,
            kind: ContentKind::Generic,
        })
    }

    /// Describes the item as one of the typed content classes. Items start as `Generic`.
    pub fn with_kind(mut self, kind: ContentKind) -> Self {
        self.kind = kind;
        self
    }

    /// The artist and album joined for a card line, `Artist - Album`, skipping whatever is missing.
    pub fn subtitle(&self) -> String {
        let mut parts: Vec<&str> = Vec::new();
        if let Some(artist) = self.artist.as_deref() {
            parts.push(artist);
        }
        if let Some(album) = self.album.as_deref() {
            parts.push(album);
        }
        parts.join(" - ")
    }

    /// The elapsed share of a finite item, 0 to 1. `position` is in seconds and is clamped to the item.
    /// Continuous and live items have no end to measure against, so they give `None`.
    pub fn progress(&self, position: f64) -> Option<f32> {
        let length = match self.duration {
            MediaDuration::Finite(seconds) => seconds,
            _ => return None,
        };
        if !length.is_finite() || length <= 0.0 || !position.is_finite() {
            return None;
        }
        Some((position.clamp(0.0, length) / length) as f32)
    }
}

/// Trims an optional text field and drops it when nothing is left.
pub fn optional_text(text: &str) -> Option<String> {
    let text = text.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_requires_an_identifier_and_a_title() {
        assert_eq!(
            MediaContent::new("  ", "Song", MediaType::Audio, MediaDuration::Continuous),
            Err(ContentError::EmptyId)
        );
        assert_eq!(
            MediaContent::new("track-1", " \t ", MediaType::Audio, MediaDuration::Continuous),
            Err(ContentError::EmptyTitle)
        );
        let item = MediaContent::new(" track-1 ", " Song ", MediaType::Video, MediaDuration::Live).unwrap();
        assert_eq!(item.id, "track-1");
        assert_eq!(item.title, "Song");
        assert_eq!(item.media_type, MediaType::Video);
        assert!(!item.is_explicit);
    }

    #[test]
    fn finite_seconds_accepts_only_positive_finite_lengths() {
        assert_eq!(MediaDuration::finite_seconds(0.5), Ok(MediaDuration::Finite(0.5)));
        assert_eq!(MediaDuration::finite_seconds(0.0), Err(ContentError::InvalidDuration));
        assert_eq!(MediaDuration::finite_seconds(-3.0), Err(ContentError::InvalidDuration));
        assert_eq!(MediaDuration::finite_seconds(f64::NAN), Err(ContentError::InvalidDuration));
        assert_eq!(MediaDuration::finite_seconds(f64::INFINITY), Err(ContentError::InvalidDuration));
    }

    #[test]
    fn labels_show_minutes_hours_live_or_nothing() {
        assert_eq!(MediaDuration::Finite(225.0).label(), Some("3:45".to_string()));
        assert_eq!(MediaDuration::Finite(3725.0).label(), Some("1:02:05".to_string()));
        assert_eq!(MediaDuration::Finite(59.6).label(), Some("1:00".to_string()));
        assert_eq!(MediaDuration::Live.label(), Some("Live".to_string()));
        assert_eq!(MediaDuration::Continuous.label(), None);
    }

    #[test]
    fn only_finite_content_is_seekable() {
        assert!(MediaDuration::Finite(10.0).is_seekable());
        assert!(!MediaDuration::Live.is_seekable());
        assert!(!MediaDuration::Continuous.is_seekable());
    }

    #[test]
    fn progress_is_a_clamped_share_of_a_finite_item() {
        let mut item = MediaContent::new("t", "Song", MediaType::Audio, MediaDuration::Finite(200.0)).unwrap();
        assert_eq!(item.progress(50.0), Some(0.25));
        assert_eq!(item.progress(300.0), Some(1.0));
        assert_eq!(item.progress(-5.0), Some(0.0));
        assert_eq!(item.progress(f64::NAN), None);
        item.duration = MediaDuration::Live;
        assert_eq!(item.progress(10.0), None);
    }

    #[test]
    fn subtitle_joins_the_artist_and_album_that_exist() {
        let mut item = MediaContent::new("t", "Song", MediaType::Audio, MediaDuration::Continuous).unwrap();
        assert_eq!(item.subtitle(), "");
        item.artist = optional_text("  Artist ");
        assert_eq!(item.subtitle(), "Artist");
        item.album = Some("Album".to_string());
        assert_eq!(item.subtitle(), "Artist - Album");
        assert_eq!(optional_text("   "), None);
    }
}
