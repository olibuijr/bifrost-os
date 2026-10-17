//! Typed metadata for a media file. A file stores its tags under keys from one key space: ID3 frame ids for MP3 and
//! the item atoms of an MP4 file. [`Identifier`] names what a tag means (title, track, cover) across both spaces,
//! [`Value`] is the typed payload, and [`Collection`] holds the tags of one file with each value checked against its
//! identifier. The readers in `id3` and `ilst` produce [`Tags`], which convert to and from a collection.

use super::audiofile::Tags;

/// The key space a tag key belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeySpace {
    /// ID3v2 frame ids, four characters (`TIT2`, `TPE1`, ...).
    Id3,
    /// The four-character item atoms of an MP4 file's metadata list (`\u{a9}nam`, `trkn`, ...).
    Mp4Item,
}

/// What a tag means, whatever key space stores it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Identifier {
    Title,
    Artist,
    Album,
    AlbumArtist,
    Track,
    Year,
    Cover,
}

impl Identifier {
    /// Every identifier, in the order a collection lists them.
    pub const ALL: [Identifier; 7] = [
        Identifier::Title,
        Identifier::Artist,
        Identifier::Album,
        Identifier::AlbumArtist,
        Identifier::Track,
        Identifier::Year,
        Identifier::Cover,
    ];

    /// The key this identifier is stored under in a key space.
    pub fn key(self, space: KeySpace) -> &'static str {
        match (space, self) {
            (KeySpace::Id3, Identifier::Title) => "TIT2",
            (KeySpace::Id3, Identifier::Artist) => "TPE1",
            (KeySpace::Id3, Identifier::Album) => "TALB",
            (KeySpace::Id3, Identifier::AlbumArtist) => "TPE2",
            (KeySpace::Id3, Identifier::Track) => "TRCK",
            (KeySpace::Id3, Identifier::Year) => "TDRC",
            (KeySpace::Id3, Identifier::Cover) => "APIC",
            (KeySpace::Mp4Item, Identifier::Title) => "\u{a9}nam",
            (KeySpace::Mp4Item, Identifier::Artist) => "\u{a9}ART",
            (KeySpace::Mp4Item, Identifier::Album) => "\u{a9}alb",
            (KeySpace::Mp4Item, Identifier::AlbumArtist) => "aART",
            (KeySpace::Mp4Item, Identifier::Track) => "trkn",
            (KeySpace::Mp4Item, Identifier::Year) => "\u{a9}day",
            (KeySpace::Mp4Item, Identifier::Cover) => "covr",
        }
    }

    /// The identifier a key of a key space stands for. `None` for a key the model does not name.
    pub fn from_key(space: KeySpace, key: &str) -> Option<Identifier> {
        Identifier::ALL.into_iter().find(|id| id.key(space) == key)
    }

    /// True when `value` has the type this identifier takes: text, a number or a picture.
    pub fn accepts(self, value: &Value) -> bool {
        matches!(
            (self, value),
            (Identifier::Title | Identifier::Artist | Identifier::Album | Identifier::AlbumArtist, Value::Text(_))
                | (Identifier::Track | Identifier::Year, Value::Number(_))
                | (Identifier::Cover, Value::Picture(_))
        )
    }
}

/// A typed tag value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Text(String),
    Number(u32),
    /// An encoded picture (JPEG or PNG bytes).
    Picture(Vec<u8>),
}

/// The tags of one file, each stored under its identifier with a value of the identifier's type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Collection {
    space: KeySpace,
    items: Vec<(Identifier, Value)>,
}

impl Collection {
    /// An empty collection that stores its tags in `space`.
    pub fn new(space: KeySpace) -> Self {
        Collection { space, items: Vec::new() }
    }

    /// The key space the collection's keys belong to.
    pub fn key_space(&self) -> KeySpace {
        self.space
    }

    /// Stores `value` under `id`, replacing an earlier value. `false` and no change when the value has the wrong type.
    pub fn set(&mut self, id: Identifier, value: Value) -> bool {
        if !id.accepts(&value) {
            return false;
        }
        match self.items.iter_mut().find(|(known, _)| *known == id) {
            Some(slot) => slot.1 = value,
            None => self.items.push((id, value)),
        }
        true
    }

    /// The value stored under `id`.
    pub fn get(&self, id: Identifier) -> Option<&Value> {
        self.items.iter().find(|(known, _)| *known == id).map(|(_, value)| value)
    }

    /// The text stored under `id`, when it is text.
    pub fn text(&self, id: Identifier) -> Option<&str> {
        match self.get(id)? {
            Value::Text(text) => Some(text.as_str()),
            _ => None,
        }
    }

    /// The number stored under `id`, when it is a number.
    pub fn number(&self, id: Identifier) -> Option<u32> {
        match self.get(id)? {
            Value::Number(n) => Some(*n),
            _ => None,
        }
    }

    /// The picture stored under `id`, when it is a picture.
    pub fn picture(&self, id: Identifier) -> Option<&[u8]> {
        match self.get(id)? {
            Value::Picture(bytes) => Some(bytes.as_slice()),
            _ => None,
        }
    }

    /// The number of tags stored.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// True when no tag is stored.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Builds the typed tags for `space` from the fields a reader produced.
    pub fn from_tags(space: KeySpace, tags: &Tags) -> Self {
        let mut out = Collection::new(space);
        let texts = [
            (Identifier::Title, &tags.title),
            (Identifier::Artist, &tags.artist),
            (Identifier::Album, &tags.album),
            (Identifier::AlbumArtist, &tags.album_artist),
        ];
        for (id, text) in texts {
            if let Some(text) = text {
                out.set(id, Value::Text(text.clone()));
            }
        }
        if let Some(track) = tags.track {
            out.set(Identifier::Track, Value::Number(track));
        }
        if let Some(year) = tags.year {
            out.set(Identifier::Year, Value::Number(year));
        }
        if let Some(cover) = &tags.cover {
            out.set(Identifier::Cover, Value::Picture(cover.clone()));
        }
        out
    }

    /// Builds a collection from raw `(key, value)` entries of a key space. An entry whose key the model does not name,
    /// or whose value has the wrong type, is skipped. Returns the collection and the number of skipped entries.
    pub fn from_keyed(space: KeySpace, entries: impl IntoIterator<Item = (String, Value)>) -> (Self, usize) {
        let mut out = Collection::new(space);
        let mut skipped = 0;
        for (key, value) in entries {
            let Some(id) = Identifier::from_key(space, &key) else {
                skipped += 1;
                continue;
            };
            if !out.set(id, value) {
                skipped += 1;
            }
        }
        (out, skipped)
    }

    /// The reader's fields for these tags. Each identifier with a value of its type fills its field.
    pub fn to_tags(&self) -> Tags {
        let mut tags = Tags::default();
        for (id, value) in &self.items {
            match (id, value) {
                (Identifier::Title, Value::Text(text)) => tags.title = Some(text.clone()),
                (Identifier::Artist, Value::Text(text)) => tags.artist = Some(text.clone()),
                (Identifier::Album, Value::Text(text)) => tags.album = Some(text.clone()),
                (Identifier::AlbumArtist, Value::Text(text)) => tags.album_artist = Some(text.clone()),
                (Identifier::Track, Value::Number(n)) => tags.track = Some(*n),
                (Identifier::Year, Value::Number(n)) => tags.year = Some(*n),
                (Identifier::Cover, Value::Picture(bytes)) => tags.cover = Some(bytes.clone()),
                _ => {}
            }
        }
        tags
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Tags {
        Tags {
            title: Some("Song".into()),
            artist: Some("Band".into()),
            album: Some("Record".into()),
            album_artist: Some("Various".into()),
            track: Some(3),
            year: Some(1999),
            cover: Some(vec![0xFF, 0xD8]),
            lyrics: None,
        }
    }

    #[test]
    fn each_identifier_has_one_key_in_each_space() {
        assert_eq!(Identifier::Title.key(KeySpace::Id3), "TIT2");
        assert_eq!(Identifier::Title.key(KeySpace::Mp4Item), "\u{a9}nam");
        assert_eq!(Identifier::Track.key(KeySpace::Mp4Item), "trkn");
        assert_eq!(Identifier::Cover.key(KeySpace::Id3), "APIC");
    }

    #[test]
    fn a_key_maps_back_to_its_identifier_in_its_own_space_only() {
        assert_eq!(Identifier::from_key(KeySpace::Id3, "TPE2"), Some(Identifier::AlbumArtist));
        assert_eq!(Identifier::from_key(KeySpace::Mp4Item, "TPE2"), None);
        assert_eq!(Identifier::from_key(KeySpace::Id3, "XXXX"), None);
        for space in [KeySpace::Id3, KeySpace::Mp4Item] {
            for id in Identifier::ALL {
                assert_eq!(Identifier::from_key(space, id.key(space)), Some(id));
            }
        }
    }

    #[test]
    fn a_value_must_have_the_type_of_its_identifier() {
        let mut tags = Collection::new(KeySpace::Id3);
        assert!(!tags.set(Identifier::Track, Value::Text("3".into())));
        assert!(!tags.set(Identifier::Cover, Value::Number(1)));
        assert!(tags.set(Identifier::Track, Value::Number(3)));
        assert!(!tags.is_empty());
        assert_eq!(tags.len(), 1);
    }

    #[test]
    fn setting_an_identifier_again_replaces_its_value() {
        let mut tags = Collection::new(KeySpace::Mp4Item);
        tags.set(Identifier::Title, Value::Text("First".into()));
        tags.set(Identifier::Title, Value::Text("Second".into()));
        assert_eq!(tags.len(), 1);
        assert_eq!(tags.text(Identifier::Title), Some("Second"));
    }

    #[test]
    fn typed_accessors_return_nothing_for_another_type() {
        let mut tags = Collection::new(KeySpace::Id3);
        tags.set(Identifier::Year, Value::Number(2001));
        tags.set(Identifier::Cover, Value::Picture(vec![1, 2]));
        assert_eq!(tags.number(Identifier::Year), Some(2001));
        assert_eq!(tags.text(Identifier::Year), None);
        assert_eq!(tags.picture(Identifier::Cover), Some(&[1, 2][..]));
        assert_eq!(tags.number(Identifier::Cover), None);
    }

    #[test]
    fn the_reader_fields_round_trip_through_a_collection() {
        let tags = sample();
        let collection = Collection::from_tags(KeySpace::Id3, &tags);
        assert_eq!(collection.len(), 7);
        assert_eq!(collection.key_space(), KeySpace::Id3);
        assert_eq!(collection.to_tags(), tags);
    }

    #[test]
    fn raw_entries_keep_the_known_keys_and_count_the_rest() {
        let entries = vec![
            ("\u{a9}nam".to_string(), Value::Text("Song".into())),
            ("trkn".to_string(), Value::Text("not a number".into())),
            ("trkn".to_string(), Value::Number(4)),
            ("wxyz".to_string(), Value::Number(9)),
            ("covr".to_string(), Value::Picture(vec![0x89])),
        ];
        let (collection, skipped) = Collection::from_keyed(KeySpace::Mp4Item, entries);
        assert_eq!(skipped, 2);
        assert_eq!(collection.text(Identifier::Title), Some("Song"));
        assert_eq!(collection.number(Identifier::Track), Some(4));
        assert_eq!(collection.picture(Identifier::Cover), Some(&[0x89][..]));
    }

    #[test]
    fn a_collection_reads_only_the_keys_of_its_own_space() {
        let entries = vec![("TIT2".to_string(), Value::Text("Song".into())), ("\u{a9}nam".to_string(), Value::Text("Other".into()))];
        let (collection, skipped) = Collection::from_keyed(KeySpace::Id3, entries);
        assert_eq!(collection.key_space(), KeySpace::Id3);
        assert_eq!(collection.text(Identifier::Title), Some("Song"));
        assert_eq!(skipped, 1);
    }
}
