//! Parity sdk-complete-43: headless tests of the library's artwork accessors. The tests build the library from scanned
//! tags held in memory, and read artwork bytes back by album. No file is decoded and no window opens.

use bifrost_music::music::library::{artwork_format, artwork_of, Library, Scanned};
use springboard::media::audiofile::Tags;
use std::path::{Path, PathBuf};

fn scanned(path: &str, album: &str, cover: Option<Vec<u8>>) -> Scanned {
    let has_cover = cover.is_some();
    let tags = Tags { title: Some(path.into()), album: Some(album.into()), artist: Some("Band".into()), cover, ..Default::default() };
    Scanned { path: PathBuf::from(path), tags, duration_ms: None, has_cover, added: 0 }
}

#[test]
fn each_album_returns_the_cover_its_songs_carried() {
    let png = b"\x89PNG\r\n\x1a\n-cover-".to_vec();
    let lib = Library::build(vec![
        scanned("/m/beta/1.m4a", "Beta", None),
        scanned("/m/alpha/1.m4a", "Alpha", Some(png.clone())),
        scanned("/m/alpha/2.m4a", "Alpha", Some(vec![1, 2, 3])),
    ]);
    assert_eq!(lib.albums[0].name, "Alpha", "albums are in name order");
    assert_eq!(lib.cover_of(0), Some(png.as_slice()), "the first cover of an album is kept");
    assert_eq!(lib.cover_of(1), None, "an album with no cover has none");
}

#[test]
fn a_missing_album_index_has_no_cover() {
    let lib = Library::build(vec![scanned("/m/a.m4a", "Alpha", Some(vec![9]))]);
    assert_eq!(lib.cover_of(0), Some(&[9u8][..]));
    assert_eq!(lib.cover_of(5), None);
}

#[test]
fn artwork_bytes_are_named_by_their_signature() {
    assert_eq!(artwork_format(b"\x89PNG\r\n\x1a\nrest"), Some("image/png"));
    assert_eq!(artwork_format(&[0xFF, 0xD8, 0xFF, 0xE0, 0x00]), Some("image/jpeg"));
    assert_eq!(artwork_format(b"GIF89a"), None, "other picture formats are not named");
    assert_eq!(artwork_format(&[]), None);
}

#[test]
fn artwork_of_a_file_that_does_not_read_is_none() {
    let missing = Path::new("/nonexistent-bifrost-music/none.m4a");
    assert_eq!(artwork_of(missing), None);
}
