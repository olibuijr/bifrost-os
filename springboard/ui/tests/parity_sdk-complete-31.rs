//! Parity sdk-complete-31: headless tests for the media library models (the file kinds, the group tree and the typed
//! attributes of a media object). Pure models: no library scan, no disk and no notification runs here.

use bifrost_ui::media_attributes::{AttributeError, AttributeKey, AttributeValue, MediaAttributes, MediaObject};
use bifrost_ui::media_group::{GroupError, GroupType, MediaGroup};
use bifrost_ui::media_kind::kind_of_name;
use bifrost_ui::media_sources::SourceType;

// ---- media types: which kind of media a file is (bifrost_ui::media_kind) ----

#[test]
fn a_library_listing_sorts_its_files_into_pictures_audio_and_movies() {
    let listing = ["IMG_0001.HEIC", "song.flac", "notes.txt", "clip.mov", ".DS_cache.png", "cover.jpeg"];
    let kinds: Vec<Option<SourceType>> = listing.iter().map(|name| kind_of_name(name)).collect();
    assert_eq!(
        kinds,
        vec![
            Some(SourceType::Image),
            Some(SourceType::Audio),
            None,
            Some(SourceType::Movie),
            None,
            Some(SourceType::Image),
        ]
    );
}

#[test]
fn a_midi_file_is_audio_and_a_bare_extension_is_not_media() {
    assert_eq!(kind_of_name("theme.midi"), Some(SourceType::Audio));
    assert_eq!(kind_of_name("mp3"), None, "no dot, no extension");
    assert_eq!(kind_of_name(".mp3"), None, "a dot file with no stem is hidden");
}

// ---- media groups: the group tree and its group types (bifrost_ui::media_group) ----

fn album(id: &str, title: &str, songs: &[&str]) -> MediaGroup {
    let mut group = MediaGroup::new(id, GroupType::new("album").unwrap(), title).unwrap();
    for song in songs {
        assert!(group.add_item(song));
    }
    group
}

#[test]
fn a_library_tree_holds_artists_with_their_albums_and_finds_each_by_id() {
    let mut library = MediaGroup::new("library", GroupType::new("folder").unwrap(), "Library").unwrap();
    let mut band = MediaGroup::new("band", GroupType::new("artist").unwrap(), "Band").unwrap();
    band.add_child(album("first", "First", &["s1", "s2"])).unwrap();
    band.add_child(album("second", "Second", &["s3"])).unwrap();
    library.add_child(band).unwrap();

    assert_eq!(library.count(), 4);
    assert_eq!(library.find("second").unwrap().kind().as_str(), "album");
    assert_eq!(library.find("band").unwrap().children().len(), 2);
    assert_eq!(library.path_to("first"), Some(vec!["Library", "Band", "First"]));
    assert_eq!(library.all_items(), vec!["s1", "s2", "s3"]);
}

#[test]
fn a_group_id_is_unique_in_the_whole_tree() {
    let mut library = MediaGroup::new("library", GroupType::new("folder").unwrap(), "Library").unwrap();
    library.add_child(album("a", "A", &[])).unwrap();
    let clash = album("a", "Another A", &[]);
    assert_eq!(library.add_child(clash), Err(GroupError::DuplicateId("a".to_string())));
    assert_eq!(library.count(), 2, "the refused group is not added");
}

#[test]
fn removing_a_group_removes_its_media_from_the_listing() {
    let mut library = MediaGroup::new("library", GroupType::new("folder").unwrap(), "Library").unwrap();
    library.add_child(album("a", "A", &["s1"])).unwrap();
    library.add_child(album("b", "B", &["s2"])).unwrap();
    assert!(library.remove("a").is_some());
    assert_eq!(library.all_items(), vec!["s2"]);
    assert!(library.remove("a").is_none());
}

#[test]
fn a_group_type_rejects_names_outside_the_allowed_form() {
    assert!(GroupType::new("smart.folder-2").is_some());
    assert!(GroupType::new("Album").is_none(), "upper case is refused");
    assert!(GroupType::new("9lives").is_none(), "a digit cannot start the identifier");
    assert!(GroupType::new("").is_none());
}

// ---- media objects: typed attributes (bifrost_ui::media_attributes) ----

#[test]
fn a_song_object_keeps_its_typed_attributes_and_refuses_a_mismatch() {
    let mut song = MediaObject::new("song-42").unwrap();
    song.set(AttributeKey::Title, AttributeValue::Text("Blue".into())).unwrap();
    song.set(AttributeKey::Duration, AttributeValue::Seconds(215.5)).unwrap();
    song.set(AttributeKey::TrackNumber, AttributeValue::Integer(3)).unwrap();

    let attrs = song.attributes();
    assert_eq!(attrs.text(AttributeKey::Title), Some("Blue"));
    assert_eq!(attrs.get(AttributeKey::Duration), Some(&AttributeValue::Seconds(215.5)));
    assert_eq!(
        song.set(AttributeKey::Duration, AttributeValue::Text("long".into())),
        Err(AttributeError::Rejected(AttributeKey::Duration))
    );
    assert_eq!(song.attributes().get(AttributeKey::Duration), Some(&AttributeValue::Seconds(215.5)));
}

#[test]
fn a_picture_object_records_its_size_and_date() {
    let mut picture = MediaObject::new("IMG_0001").unwrap();
    picture.set(AttributeKey::Width, AttributeValue::Integer(4032)).unwrap();
    picture.set(AttributeKey::Height, AttributeValue::Integer(3024)).unwrap();
    picture.set(AttributeKey::Added, AttributeValue::Date(1_700_000_000)).unwrap();
    assert_eq!(picture.attributes().keys(), vec![AttributeKey::Width, AttributeKey::Height, AttributeKey::Added]);
}

#[test]
fn an_attribute_dictionary_lists_keys_in_key_order_and_reads_names_back() {
    let mut attrs = MediaAttributes::new();
    attrs.set(AttributeKey::Favourite, AttributeValue::Flag(true)).unwrap();
    attrs.set(AttributeKey::Artist, AttributeValue::Text("Band".into())).unwrap();
    assert_eq!(attrs.keys(), vec![AttributeKey::Artist, AttributeKey::Favourite]);
    assert_eq!(AttributeKey::from_name("favourite"), Some(AttributeKey::Favourite));
    assert_eq!(AttributeKey::from_name("unknown"), None);
    assert!(attrs.remove(AttributeKey::Artist).is_some());
    assert_eq!(attrs.len(), 1);
}
