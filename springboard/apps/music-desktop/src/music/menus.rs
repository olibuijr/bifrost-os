//! The app's menus (`App::main_menu`): File > Import, Song, Controls and View. This module is the model: the commands
//! with their ids and key equivalents, and the menus built from a snapshot of the app's state. `Music` runs the
//! commands (`Music::run_command`), `MusicApp` hands the shell the menus and the chosen id.

use super::browse::SortKey;
use bifrost_app::{AppMenu, AppMenuItem};

/// Where the content pane can be pointed from the View menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    Artists,
    Albums,
    Songs,
    Radio,
    Playlists,
    RecentlyAdded,
    RecentlyPlayed,
    Favourites,
    UpNext,
    History,
}

impl Place {
    pub const ALL: [Place; 10] = [Place::Artists, Place::Albums, Place::Songs, Place::Radio, Place::Playlists, Place::RecentlyAdded, Place::RecentlyPlayed, Place::Favourites, Place::UpNext, Place::History];

    pub fn title(self) -> &'static str {
        match self {
            Place::Artists => "Artists",
            Place::Albums => "Albums",
            Place::Songs => "Songs",
            Place::Radio => "Radio",
            Place::Playlists => "Playlists",
            Place::RecentlyAdded => "Recently Added",
            Place::RecentlyPlayed => "Recently Played",
            Place::Favourites => "Favourite Songs",
            Place::UpNext => "Up Next",
            Place::History => "History",
        }
    }

    fn key(self) -> &'static str {
        match self {
            Place::Artists => "cmd+1",
            Place::Albums => "cmd+2",
            Place::Songs => "cmd+3",
            Place::Radio => "cmd+4",
            Place::Playlists => "cmd+5",
            Place::UpNext => "cmd+alt+u",
            _ => "",
        }
    }
}

/// One menu command. The id the shell sends back is `Command::id`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    /// File > Import: copy songs into the library
    Import,
    PlayPause,
    Next,
    Previous,
    Shuffle,
    Repeat,
    VolumeUp,
    VolumeDown,
    /// Song: act on the song that is playing
    PlayNext,
    PlayLast,
    /// a new playlist that starts with the playing song
    NewPlaylist,
    /// add the playing song to the n-th playlist
    AddTo(usize),
    Favourite,
    /// 0 clears the rating
    Rate(u8),
    Show(Place),
    SortBy(SortKey),
    /// show or hide the lyrics on the Now Playing sheet
    Lyrics,
}

const IMPORT: u32 = 1;
const PLAY_PAUSE: u32 = 10;
const NEXT: u32 = 11;
const PREVIOUS: u32 = 12;
const SHUFFLE: u32 = 13;
const REPEAT: u32 = 14;
const VOLUME_UP: u32 = 15;
const VOLUME_DOWN: u32 = 16;
const PLAY_NEXT: u32 = 20;
const PLAY_LAST: u32 = 21;
const NEW_PLAYLIST: u32 = 22;
const FAVOURITE: u32 = 23;
const LYRICS: u32 = 24;
const RATE_BASE: u32 = 30;
const SHOW_BASE: u32 = 40;
const SORT_BASE: u32 = 60;
const ADD_TO_BASE: u32 = 1000;
/// The highest rating a Rate command carries.
const RATE_MAX: u8 = 5;

impl Command {
    /// The id the menu item carries.
    pub fn id(self) -> u32 {
        match self {
            Command::Import => IMPORT,
            Command::PlayPause => PLAY_PAUSE,
            Command::Next => NEXT,
            Command::Previous => PREVIOUS,
            Command::Shuffle => SHUFFLE,
            Command::Repeat => REPEAT,
            Command::VolumeUp => VOLUME_UP,
            Command::VolumeDown => VOLUME_DOWN,
            Command::PlayNext => PLAY_NEXT,
            Command::PlayLast => PLAY_LAST,
            Command::NewPlaylist => NEW_PLAYLIST,
            Command::Favourite => FAVOURITE,
            Command::Lyrics => LYRICS,
            Command::Rate(n) => RATE_BASE + u32::from(n.min(RATE_MAX)),
            Command::Show(p) => SHOW_BASE + Place::ALL.iter().position(|q| *q == p).unwrap_or(0) as u32,
            Command::SortBy(k) => SORT_BASE + SortKey::ALL.iter().position(|q| *q == k).unwrap_or(0) as u32,
            Command::AddTo(i) => ADD_TO_BASE + i as u32,
        }
    }

    /// The command behind a menu item id; None for an id no menu carries.
    pub fn from_id(id: u32) -> Option<Command> {
        let fixed = [
            Command::Import,
            Command::PlayPause,
            Command::Next,
            Command::Previous,
            Command::Shuffle,
            Command::Repeat,
            Command::VolumeUp,
            Command::VolumeDown,
            Command::PlayNext,
            Command::PlayLast,
            Command::NewPlaylist,
            Command::Favourite,
            Command::Lyrics,
        ];
        if let Some(c) = fixed.into_iter().find(|c| c.id() == id) {
            return Some(c);
        }
        match id {
            _ if (RATE_BASE..=RATE_BASE + u32::from(RATE_MAX)).contains(&id) => Some(Command::Rate((id - RATE_BASE) as u8)),
            _ if (SHOW_BASE..SHOW_BASE + Place::ALL.len() as u32).contains(&id) => Some(Command::Show(Place::ALL[(id - SHOW_BASE) as usize])),
            _ if (SORT_BASE..SORT_BASE + SortKey::ALL.len() as u32).contains(&id) => Some(Command::SortBy(SortKey::ALL[(id - SORT_BASE) as usize])),
            _ if id >= ADD_TO_BASE => Some(Command::AddTo((id - ADD_TO_BASE) as usize)),
            _ => None,
        }
    }
}

/// What the menus need to know about the app.
#[derive(Clone, Debug, PartialEq)]
pub struct MenuState {
    /// a song or a station is loaded
    pub has_track: bool,
    /// the transport is playing
    pub playing: bool,
    /// the loaded track is a station
    pub live: bool,
    /// a song of the library is loaded, so the Song menu has something to act on
    pub has_song: bool,
    pub shuffle: bool,
    pub repeat: bool,
    /// the loaded song's rating and favourite flag
    pub rating: u8,
    pub favourite: bool,
    pub playlists: Vec<String>,
    pub place: Place,
    pub sort: SortKey,
    pub lyrics_shown: bool,
    pub has_lyrics: bool,
}

impl Default for MenuState {
    fn default() -> Self {
        MenuState { has_track: false, playing: false, live: false, has_song: false, shuffle: false, repeat: false, rating: 0, favourite: false, playlists: Vec::new(), place: Place::Songs, sort: SortKey::Title, lyrics_shown: false, has_lyrics: false }
    }
}

fn item(cmd: Command, title: &str, key: &str) -> AppMenuItem {
    let base = AppMenuItem::new(cmd.id(), title);
    if key.is_empty() {
        base
    } else {
        base.key(key)
    }
}

/// The menus for the app's state: File, Song, Controls, View.
pub fn build(s: &MenuState) -> Vec<AppMenu> {
    let song = s.has_song;
    let mut add_to = vec![AppMenuItem::submenu("Add to Playlist"), item(Command::NewPlaylist, "New Playlist", "").depth(1).enabled(song)];
    if !s.playlists.is_empty() {
        add_to.push(AppMenuItem::separator().depth(1));
    }
    add_to.extend(s.playlists.iter().enumerate().map(|(i, name)| item(Command::AddTo(i), name, "").depth(1).enabled(song)));
    let mut rate = vec![AppMenuItem::submenu("Rating")];
    rate.extend((0..=RATE_MAX).map(|n| {
        let title = super::stats::rating_text(n);
        item(Command::Rate(n), &title, "").depth(1).enabled(song).checked(s.rating == n)
    }));
    let mut song_items = vec![item(Command::PlayNext, "Play Next", "").enabled(song), item(Command::PlayLast, "Play Last", "").enabled(song), AppMenuItem::separator()];
    song_items.extend(add_to);
    song_items.extend(rate);
    song_items.push(item(Command::Favourite, "Favourite", "cmd+l").enabled(song).checked(s.favourite));
    let controls = vec![
        item(Command::PlayPause, if s.playing { "Pause" } else { "Play" }, "").enabled(s.has_track),
        item(Command::Next, "Next", "cmd+right").enabled(s.has_track),
        item(Command::Previous, "Previous", "cmd+left").enabled(s.has_track),
        AppMenuItem::separator(),
        item(Command::Shuffle, "Shuffle", "").enabled(s.has_track && !s.live).checked(s.shuffle),
        item(Command::Repeat, "Repeat", "").enabled(s.has_track && !s.live).checked(s.repeat),
        AppMenuItem::separator(),
        item(Command::VolumeUp, "Volume Up", "cmd+up").enabled(true),
        item(Command::VolumeDown, "Volume Down", "cmd+down").enabled(true),
    ];
    let mut view: Vec<AppMenuItem> = Place::ALL.iter().map(|p| item(Command::Show(*p), p.title(), p.key()).checked(*p == s.place)).collect();
    view.insert(5, AppMenuItem::separator());
    view.insert(9, AppMenuItem::separator());
    view.push(AppMenuItem::separator());
    view.push(item(Command::Lyrics, "Show Lyrics", "").enabled(s.has_lyrics).checked(s.lyrics_shown));
    view.push(AppMenuItem::separator());
    view.push(AppMenuItem::submenu("Sort Songs By"));
    view.extend(SortKey::ALL.iter().map(|k| item(Command::SortBy(*k), k.label(), "").depth(1).checked(*k == s.sort)));
    vec![
        AppMenu { title: "File".into(), items: vec![item(Command::Import, "Import\u{2026}", "cmd+o")] },
        AppMenu { title: "Song".into(), items: song_items },
        AppMenu { title: "Controls".into(), items: controls },
        AppMenu { title: "View".into(), items: view },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_app::controls::keys::KeyCombo;

    fn full() -> MenuState {
        MenuState { has_track: true, playing: true, has_song: true, playlists: vec!["Road".into(), "Gym".into()], rating: 3, favourite: true, has_lyrics: true, ..MenuState::default() }
    }

    fn all_items(menus: &[AppMenu]) -> Vec<&AppMenuItem> {
        menus.iter().flat_map(|m| m.items.iter()).filter(|i| i.id != 0).collect()
    }

    #[test]
    fn every_command_id_reads_back_to_the_same_command() {
        let mut cmds = vec![Command::Import, Command::PlayPause, Command::Next, Command::Previous, Command::Shuffle, Command::Repeat, Command::VolumeUp, Command::VolumeDown];
        cmds.extend([Command::PlayNext, Command::PlayLast, Command::NewPlaylist, Command::Favourite, Command::Lyrics]);
        cmds.extend((0..=RATE_MAX).map(Command::Rate));
        cmds.extend(Place::ALL.map(Command::Show));
        cmds.extend(SortKey::ALL.map(Command::SortBy));
        cmds.extend([Command::AddTo(0), Command::AddTo(7), Command::AddTo(250)]);
        for c in &cmds {
            assert_eq!(Command::from_id(c.id()), Some(*c), "{c:?}");
        }
        let mut ids: Vec<u32> = cmds.iter().map(|c| c.id()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), cmds.len(), "no two commands share an id");
        assert_eq!(Command::from_id(0), None);
        assert_eq!(Command::from_id(999), None);
    }

    #[test]
    fn the_menus_have_unique_titles_and_item_ids() {
        let menus = build(&full());
        let titles: Vec<&str> = menus.iter().map(|m| m.title.as_str()).collect();
        assert_eq!(titles, ["File", "Song", "Controls", "View"]);
        let items = all_items(&menus);
        let mut ids: Vec<u32> = items.iter().map(|i| i.id).collect();
        let n = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), n, "an id is carried by one item only");
        assert!(items.iter().all(|i| !i.title.trim().is_empty()));
        assert!(items.iter().all(|i| Command::from_id(i.id).is_some()), "every item runs a command");
        assert!(items.iter().filter(|i| !i.key.is_empty()).all(|i| KeyCombo::parse(&i.key).is_some()), "every key equivalent parses");
        let keys: Vec<&str> = items.iter().map(|i| i.key.as_str()).filter(|k| !k.is_empty()).collect();
        let mut unique = keys.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), keys.len(), "no key is bound twice");
    }

    #[test]
    fn controls_follow_the_transport() {
        let controls = |s: &MenuState| build(s).remove(2).items;
        let idle = controls(&MenuState::default());
        assert!(idle.iter().filter(|i| i.id != 0 && i.id != VOLUME_UP && i.id != VOLUME_DOWN).all(|i| !i.enabled), "nothing to control without a track");
        let playing = controls(&full());
        assert_eq!(playing[0].title, "Pause");
        assert!(playing.iter().filter(|i| i.id != 0).all(|i| i.enabled));
        let station = controls(&MenuState { live: true, shuffle: false, ..full() });
        let shuffle = station.iter().find(|i| i.id == SHUFFLE).unwrap();
        assert!(!shuffle.enabled, "a station has no shuffle");
        let paused = controls(&MenuState { playing: false, shuffle: true, repeat: true, ..full() });
        assert_eq!(paused[0].title, "Play");
        assert!(paused.iter().find(|i| i.id == SHUFFLE).unwrap().checked && paused.iter().find(|i| i.id == REPEAT).unwrap().checked);
    }

    #[test]
    fn the_song_menu_acts_on_the_playing_song_and_shows_its_rating() {
        let song = |s: &MenuState| build(s).remove(1).items;
        let none = song(&MenuState::default());
        assert!(none.iter().filter(|i| i.id != 0).all(|i| !i.enabled), "no song, nothing to act on");
        let items = song(&full());
        let rating: Vec<(&str, bool)> = items.iter().filter(|i| (RATE_BASE..=RATE_BASE + 5).contains(&i.id)).map(|i| (i.title.as_str(), i.checked)).collect();
        assert_eq!(rating, [("None", false), ("1 Star", false), ("2 Stars", false), ("3 Stars", true), ("4 Stars", false), ("5 Stars", false)]);
        assert!(items.iter().find(|i| i.id == FAVOURITE).unwrap().checked);
        let header = items.iter().position(|i| i.submenu && i.title == "Add to Playlist").unwrap();
        let children: Vec<&str> = items[header + 1..].iter().take_while(|i| i.depth == 1).filter(|i| i.id != 0).map(|i| i.title.as_str()).collect();
        assert_eq!(children, ["New Playlist", "Road", "Gym"]);
    }

    #[test]
    fn the_view_menu_marks_the_place_and_the_sort() {
        let view = build(&MenuState { place: Place::Favourites, sort: SortKey::Plays, lyrics_shown: true, ..full() }).remove(3).items;
        let checked: Vec<&str> = view.iter().filter(|i| i.checked).map(|i| i.title.as_str()).collect();
        assert_eq!(checked, ["Favourite Songs", "Show Lyrics", "Plays"]);
        let sorts = view.iter().filter(|i| i.id != 0 && i.depth == 1).count();
        assert_eq!(sorts, SortKey::ALL.len());
        let no_lyrics = build(&MenuState::default()).remove(3).items;
        assert!(!no_lyrics.iter().find(|i| i.id == LYRICS).unwrap().enabled);
    }
}
