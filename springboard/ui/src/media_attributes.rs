//! Media object attributes: the typed dictionary that describes one media object (a song, a picture, a movie). Each
//! attribute has a fixed key, and each key accepts one kind of value, so a duration can never hold text and a rating
//! cannot leave its range. The keys cover the common descriptions of a song, a picture or a movie; an app that needs
//! another attribute adds a key here rather than storing an untyped string.
//!
//! [`MediaAttributes::set`] checks the value against its key and refuses a mismatch, and [`MediaObject`] pairs the
//! dictionary with the identifier the library knows the object by. Gaps: no custom attribute keys, no localized
//! attribute names, and no conversion from the tag formats the Music app reads (the tags stay in its own structs).

use std::collections::BTreeMap;
use std::fmt;

/// One attribute key of a media object. The order of the variants is the order the keys are listed in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AttributeKey {
    Title,
    Artist,
    Album,
    Genre,
    TrackNumber,
    Duration,
    Added,
    Width,
    Height,
    Rating,
    Favourite,
}

impl AttributeKey {
    /// Every key, in order.
    pub const ALL: [AttributeKey; 11] = [
        AttributeKey::Title,
        AttributeKey::Artist,
        AttributeKey::Album,
        AttributeKey::Genre,
        AttributeKey::TrackNumber,
        AttributeKey::Duration,
        AttributeKey::Added,
        AttributeKey::Width,
        AttributeKey::Height,
        AttributeKey::Rating,
        AttributeKey::Favourite,
    ];

    /// The key's name in the wire form, such as `trackNumber`.
    pub fn name(self) -> &'static str {
        match self {
            AttributeKey::Title => "title",
            AttributeKey::Artist => "artist",
            AttributeKey::Album => "album",
            AttributeKey::Genre => "genre",
            AttributeKey::TrackNumber => "trackNumber",
            AttributeKey::Duration => "duration",
            AttributeKey::Added => "added",
            AttributeKey::Width => "width",
            AttributeKey::Height => "height",
            AttributeKey::Rating => "rating",
            AttributeKey::Favourite => "favourite",
        }
    }

    /// The key with this wire name, or `None`.
    pub fn from_name(name: &str) -> Option<Self> {
        AttributeKey::ALL.iter().copied().find(|key| key.name() == name)
    }

    /// Whether `value` is of this key's kind and inside its range.
    fn accepts(self, value: &AttributeValue) -> bool {
        match (self, value) {
            (AttributeKey::Title | AttributeKey::Artist | AttributeKey::Album | AttributeKey::Genre, AttributeValue::Text(text)) => {
                !text.trim().is_empty()
            }
            (AttributeKey::TrackNumber, AttributeValue::Integer(n)) => *n >= 1,
            (AttributeKey::Width | AttributeKey::Height, AttributeValue::Integer(n)) => *n >= 1,
            (AttributeKey::Rating, AttributeValue::Integer(n)) => (0..=5).contains(n),
            (AttributeKey::Duration, AttributeValue::Seconds(seconds)) => seconds.is_finite() && *seconds >= 0.0,
            (AttributeKey::Added, AttributeValue::Date(_)) => true,
            (AttributeKey::Favourite, AttributeValue::Flag(_)) => true,
            _ => false,
        }
    }
}

/// A value of an attribute. The kind must match the key: text for names, whole numbers for counts and sizes, seconds for
/// durations, a date as seconds since the Unix epoch, and a flag.
#[derive(Clone, Debug, PartialEq)]
pub enum AttributeValue {
    Text(String),
    Integer(i64),
    Seconds(f64),
    Date(i64),
    Flag(bool),
}

/// Why an attribute could not be stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttributeError {
    /// The value is of the wrong kind for the key, or is outside the key's range.
    Rejected(AttributeKey),
}

impl fmt::Display for AttributeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AttributeError::Rejected(key) => write!(f, "the value is not valid for {}", key.name()),
        }
    }
}

/// The attributes of one media object: at most one value per key.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MediaAttributes {
    values: BTreeMap<AttributeKey, AttributeValue>,
}

impl MediaAttributes {
    /// An empty dictionary.
    pub fn new() -> Self {
        Self::default()
    }

    /// Stores `value` under `key`, replacing an earlier value. A value of the wrong kind or out of range is refused and
    /// the dictionary is unchanged.
    pub fn set(&mut self, key: AttributeKey, value: AttributeValue) -> Result<(), AttributeError> {
        if !key.accepts(&value) {
            return Err(AttributeError::Rejected(key));
        }
        self.values.insert(key, value);
        Ok(())
    }

    /// The value under `key`.
    pub fn get(&self, key: AttributeKey) -> Option<&AttributeValue> {
        self.values.get(&key)
    }

    /// The text under `key`, when the value is text.
    pub fn text(&self, key: AttributeKey) -> Option<&str> {
        match self.get(key) {
            Some(AttributeValue::Text(text)) => Some(text),
            _ => None,
        }
    }

    /// Removes and returns the value under `key`.
    pub fn remove(&mut self, key: AttributeKey) -> Option<AttributeValue> {
        self.values.remove(&key)
    }

    /// The keys that have a value, in key order.
    pub fn keys(&self) -> Vec<AttributeKey> {
        self.values.keys().copied().collect()
    }

    /// The number of attributes that have a value.
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Whether no attribute has a value.
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

/// One media object: the identifier the library knows it by and its attributes.
#[derive(Clone, Debug, PartialEq)]
pub struct MediaObject {
    id: String,
    attributes: MediaAttributes,
}

impl MediaObject {
    /// A media object with no attributes. The identifier is trimmed, and `None` is returned when nothing is left of it.
    pub fn new(id: &str) -> Option<Self> {
        let id = id.trim();
        (!id.is_empty()).then(|| MediaObject { id: id.to_string(), attributes: MediaAttributes::new() })
    }

    /// The identifier.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The attributes.
    pub fn attributes(&self) -> &MediaAttributes {
        &self.attributes
    }

    /// Stores an attribute on the object. See [`MediaAttributes::set`].
    pub fn set(&mut self, key: AttributeKey, value: AttributeValue) -> Result<(), AttributeError> {
        self.attributes.set(key, value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_key_has_a_wire_name_that_reads_back_to_the_key() {
        for key in AttributeKey::ALL {
            assert_eq!(AttributeKey::from_name(key.name()), Some(key));
        }
        assert_eq!(AttributeKey::TrackNumber.name(), "trackNumber");
        assert_eq!(AttributeKey::from_name("Title"), None, "names are case sensitive");
    }

    #[test]
    fn a_text_key_takes_text_and_refuses_a_blank_string() {
        let mut attrs = MediaAttributes::new();
        assert!(attrs.set(AttributeKey::Title, AttributeValue::Text("Blue".into())).is_ok());
        assert_eq!(attrs.text(AttributeKey::Title), Some("Blue"));
        assert_eq!(
            attrs.set(AttributeKey::Artist, AttributeValue::Text("   ".into())),
            Err(AttributeError::Rejected(AttributeKey::Artist))
        );
        assert_eq!(attrs.set(AttributeKey::Album, AttributeValue::Integer(3)), Err(AttributeError::Rejected(AttributeKey::Album)));
    }

    #[test]
    fn numeric_keys_keep_their_ranges() {
        let mut attrs = MediaAttributes::new();
        assert!(attrs.set(AttributeKey::TrackNumber, AttributeValue::Integer(1)).is_ok());
        assert!(attrs.set(AttributeKey::TrackNumber, AttributeValue::Integer(0)).is_err());
        assert!(attrs.set(AttributeKey::Width, AttributeValue::Integer(0)).is_err());
        assert!(attrs.set(AttributeKey::Height, AttributeValue::Integer(4032)).is_ok());
        assert!(attrs.set(AttributeKey::Rating, AttributeValue::Integer(5)).is_ok());
        assert!(attrs.set(AttributeKey::Rating, AttributeValue::Integer(6)).is_err());
        assert!(attrs.set(AttributeKey::Rating, AttributeValue::Integer(-1)).is_err());
        assert_eq!(attrs.get(AttributeKey::Rating), Some(&AttributeValue::Integer(5)));
    }

    #[test]
    fn a_duration_is_finite_and_not_negative() {
        let mut attrs = MediaAttributes::new();
        assert!(attrs.set(AttributeKey::Duration, AttributeValue::Seconds(0.0)).is_ok());
        assert!(attrs.set(AttributeKey::Duration, AttributeValue::Seconds(-0.5)).is_err());
        assert!(attrs.set(AttributeKey::Duration, AttributeValue::Seconds(f64::NAN)).is_err());
        assert!(attrs.set(AttributeKey::Duration, AttributeValue::Seconds(f64::INFINITY)).is_err());
        assert_eq!(attrs.get(AttributeKey::Duration), Some(&AttributeValue::Seconds(0.0)));
    }

    #[test]
    fn a_refused_value_leaves_the_earlier_value_in_place() {
        let mut attrs = MediaAttributes::new();
        attrs.set(AttributeKey::Rating, AttributeValue::Integer(4)).unwrap();
        assert!(attrs.set(AttributeKey::Rating, AttributeValue::Integer(9)).is_err());
        assert_eq!(attrs.get(AttributeKey::Rating), Some(&AttributeValue::Integer(4)));
    }

    #[test]
    fn flags_and_dates_take_their_own_kinds_only() {
        let mut attrs = MediaAttributes::new();
        assert!(attrs.set(AttributeKey::Favourite, AttributeValue::Flag(true)).is_ok());
        assert!(attrs.set(AttributeKey::Favourite, AttributeValue::Integer(1)).is_err());
        assert!(attrs.set(AttributeKey::Added, AttributeValue::Date(1_700_000_000)).is_ok());
        assert!(attrs.set(AttributeKey::Added, AttributeValue::Text("today".into())).is_err());
    }

    #[test]
    fn keys_are_listed_in_key_order_and_can_be_removed() {
        let mut attrs = MediaAttributes::new();
        attrs.set(AttributeKey::Album, AttributeValue::Text("Second".into())).unwrap();
        attrs.set(AttributeKey::Title, AttributeValue::Text("One".into())).unwrap();
        assert_eq!(attrs.keys(), vec![AttributeKey::Title, AttributeKey::Album]);
        assert_eq!(attrs.len(), 2);
        assert_eq!(attrs.remove(AttributeKey::Title), Some(AttributeValue::Text("One".into())));
        assert_eq!(attrs.remove(AttributeKey::Title), None);
        assert_eq!(attrs.keys(), vec![AttributeKey::Album]);
        assert!(!attrs.is_empty());
    }

    #[test]
    fn a_media_object_needs_an_identifier_and_stores_attributes() {
        assert!(MediaObject::new("  ").is_none());
        let mut object = MediaObject::new(" song-7 ").unwrap();
        assert_eq!(object.id(), "song-7");
        object.set(AttributeKey::TrackNumber, AttributeValue::Integer(7)).unwrap();
        assert_eq!(object.attributes().get(AttributeKey::TrackNumber), Some(&AttributeValue::Integer(7)));
        assert!(object.set(AttributeKey::Rating, AttributeValue::Integer(10)).is_err());
    }

    #[test]
    fn the_rejection_message_names_the_key() {
        let text = AttributeError::Rejected(AttributeKey::Duration).to_string();
        assert_eq!(text, "the value is not valid for duration");
    }
}
