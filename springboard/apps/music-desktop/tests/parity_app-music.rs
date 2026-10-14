//! Parity tests for the Music app's Get Info edits and playlist folders: the models the app draws, read through the
//! library's public API. The pages and menus are covered by the app's own unit tests (`src/music.rs`, `src/music/menus.rs`).

use bifrost_music::music::folders::{rows, Folders, Row};
use bifrost_music::music::info::{Field, Info};
use bifrost_music::music::library::{Library, Scanned};
use bifrost_music::music::playlists::Playlist;
use springboard::media::audiofile::Tags;
use std::path::{Path, PathBuf};

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/test-parity-app-music").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn scanned(path: &str, title: &str, artist: &str, album: &str) -> Scanned {
    Scanned {
        path: PathBuf::from(path),
        tags: Tags { title: Some(title.into()), artist: Some(artist.into()), album: Some(album.into()), ..Tags::default() },
        duration_ms: Some(180_000.0),
        has_cover: false,
        added: 0,
    }
}

#[test]
fn an_edited_title_and_album_group_the_songs_the_way_the_edit_says() {
    let dir = scratch("group");
    let mut info = Info::open(dir.join("info.txt"));
    assert!(info.set(Path::new("/m/a.mp3"), Field::Album, "Greatest"));
    assert!(info.set(Path::new("/m/a.mp3"), Field::Title, "Alpha Edit"));
    let mut files = vec![scanned("/m/a.mp3", "Alpha", "Abba", "Old"), scanned("/m/b.mp3", "Bravo", "Blur", "Old")];
    for file in &mut files {
        info.apply(file);
    }
    let lib = Library::build(files);
    let a = lib.songs.iter().find(|s| s.path == Path::new("/m/a.mp3")).expect("song a is in the library");
    assert_eq!((a.title.as_str(), a.album.as_str()), ("Alpha Edit", "Greatest"));
    let b = lib.songs.iter().find(|s| s.path == Path::new("/m/b.mp3")).expect("song b is in the library");
    assert_eq!(b.album, "Old", "a song with no edit keeps its tag");
    assert!(lib.albums.iter().any(|album| album.name == "Greatest"));
    let back = Info::open(dir.join("info.txt"));
    assert_eq!(back.value(Path::new("/m/a.mp3"), Field::Title), Some("Alpha Edit"), "the edit survives a restart");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_playlists_tab_nests_folders_and_keeps_membership_through_a_rename() {
    let dir = scratch("tab");
    let file = dir.join("folders.txt");
    let mut folders = Folders::open(file.clone());
    folders.create("Road").expect("the folder is made");
    assert!(folders.put("Sunday", Some(0)));
    folders.rename_playlist("Sunday", "Sunday Mix");
    let folders = Folders::open(file);
    let lists = [Playlist { name: "Sunday Mix".into(), songs: Vec::new() }, Playlist { name: "Loose".into(), songs: Vec::new() }];
    let tab = rows(&folders, &lists, 2, &[]);
    assert_eq!(
        tab,
        [
            Row::NewPlaylist,
            Row::NewSmart,
            Row::NewFolder,
            Row::Folder(0),
            Row::Playlist { index: 0, in_folder: true },
            Row::Playlist { index: 1, in_folder: false },
            Row::Smart(0),
            Row::Smart(1),
        ]
    );
    let _ = std::fs::remove_dir_all(&dir);
}
