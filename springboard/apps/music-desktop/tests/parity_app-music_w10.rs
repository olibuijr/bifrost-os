//! Parity tests for the Music app's descending song order and its View menu item. The models are read through the
//! library's public API; no test opens a window or plays audio.

use bifrost_music::music::browse::{view_in_order, Scope, SortKey, RECENT_LIMIT};
use bifrost_music::music::library::Song;
use bifrost_music::music::menus::{build, Command, MenuState};
use bifrost_music::music::stats::Stats;
use std::path::PathBuf;

fn song(id: usize, title: &str, added: u64) -> Song {
    Song {
        id,
        path: PathBuf::from(format!("/music/{id}.mp3")),
        title: title.into(),
        artist: "Artist".into(),
        album: "Album".into(),
        album_artist: "Artist".into(),
        track: Some(id as u32 + 1),
        year: None,
        duration_ms: None,
        has_cover: false,
        added,
    }
}

fn library() -> Vec<Song> {
    vec![song(0, "Gold", 300), song(1, "The Wall", 100), song(2, "Chiquitita", 200)]
}

fn titles(songs: &[Song], ids: &[usize]) -> Vec<String> {
    ids.iter().map(|&i| songs[i].title.clone()).collect()
}

#[test]
fn descending_reverses_the_title_order() {
    let songs = library();
    let stats = Stats::default();
    let up = view_in_order(&songs, "", SortKey::Title, false, Scope::All, &stats);
    assert_eq!(titles(&songs, &up), ["Chiquitita", "Gold", "The Wall"]);
    let down = view_in_order(&songs, "", SortKey::Title, true, Scope::All, &stats);
    assert_eq!(titles(&songs, &down), ["The Wall", "Gold", "Chiquitita"]);
}

#[test]
fn descending_keeps_the_songs_a_search_leaves_in() {
    let songs = library();
    let stats = Stats::default();
    let found = view_in_order(&songs, "gold", SortKey::Title, true, Scope::All, &stats);
    assert_eq!(titles(&songs, &found), ["Gold"]);
}

#[test]
fn descending_the_newest_first_list_shows_the_oldest_first() {
    let songs = library();
    let stats = Stats::default();
    let oldest_first = view_in_order(&songs, "", SortKey::DateAdded, true, Scope::All, &stats);
    assert_eq!(titles(&songs, &oldest_first), ["The Wall", "Chiquitita", "Gold"]);
}

#[test]
fn a_recent_list_keeps_its_newest_songs_in_either_order() {
    let songs: Vec<Song> = (0..RECENT_LIMIT + 5).map(|i| song(i, &format!("Song {i:03}"), i as u64)).collect();
    let stats = Stats::default();
    let up = view_in_order(&songs, "", SortKey::Title, false, Scope::RecentlyAdded, &stats);
    let down = view_in_order(&songs, "", SortKey::Title, true, Scope::RecentlyAdded, &stats);
    assert_eq!(up.len(), RECENT_LIMIT, "the cap holds");
    let mut a = up.clone();
    let mut b = down.clone();
    a.sort_unstable();
    b.sort_unstable();
    assert_eq!(a, b, "the same songs in both orders");
    assert!(!down.contains(&0) && !down.contains(&4), "the oldest five are left out");
    assert!(down.contains(&(RECENT_LIMIT + 4)), "the newest song stays in");
}

#[test]
fn the_descending_menu_item_runs_its_command_and_shows_its_state() {
    assert_eq!(Command::from_id(Command::SortDescending.id()), Some(Command::SortDescending));
    let view = build(&MenuState { descending: true, ..MenuState::default() }).remove(3).items;
    let item = view.iter().find(|i| i.id == Command::SortDescending.id()).expect("the View menu carries Descending");
    assert_eq!(item.title, "Descending");
    assert!(item.checked, "the order shows as checked while it is on");
    let off = build(&MenuState::default()).remove(3).items;
    assert!(!off.iter().find(|i| i.id == Command::SortDescending.id()).unwrap().checked);
}
