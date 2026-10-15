//! The Songs list as the user sorts and searches it. Pure logic over the
//! library's songs; the result is a list of song ids to show.

use super::library::{sort_key, Song};
use super::stats::Stats;
use std::cmp::Reverse;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortKey {
    Title,
    Artist,
    Album,
    /// newest file first
    DateAdded,
    /// most stars first
    Rating,
    /// most played first
    Plays,
    /// started most recently first
    LastPlayed,
}

impl SortKey {
    /// Every order, as the sort button and the View menu step through them.
    pub const ALL: [SortKey; 7] = [SortKey::Title, SortKey::Artist, SortKey::Album, SortKey::DateAdded, SortKey::Rating, SortKey::Plays, SortKey::LastPlayed];

    pub fn label(self) -> &'static str {
        match self {
            SortKey::Title => "Title",
            SortKey::Artist => "Artist",
            SortKey::Album => "Album",
            SortKey::DateAdded => "Recently Added",
            SortKey::Rating => "Rating",
            SortKey::Plays => "Plays",
            SortKey::LastPlayed => "Recently Played",
        }
    }

    /// The next order when the sort button is tapped.
    pub fn cycle(self) -> SortKey {
        match self {
            SortKey::Title => SortKey::Artist,
            SortKey::Artist => SortKey::Album,
            SortKey::Album => SortKey::DateAdded,
            SortKey::DateAdded => SortKey::Rating,
            SortKey::Rating => SortKey::Plays,
            SortKey::Plays => SortKey::LastPlayed,
            SortKey::LastPlayed => SortKey::Title,
        }
    }
}

/// How many songs Recently Added and Recently Played list.
pub const RECENT_LIMIT: usize = 100;

/// Which songs the Songs list draws from: the whole library, or one of the lists the source list offers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Scope {
    #[default]
    All,
    /// the newest files
    RecentlyAdded,
    /// the songs that started lately
    RecentlyPlayed,
    /// the songs marked as favourites
    Favourites,
}

impl Scope {
    pub const ALL: [Scope; 4] = [Scope::All, Scope::RecentlyAdded, Scope::RecentlyPlayed, Scope::Favourites];

    pub fn label(self) -> &'static str {
        match self {
            Scope::All => "Songs",
            Scope::RecentlyAdded => "Recently Added",
            Scope::RecentlyPlayed => "Recently Played",
            Scope::Favourites => "Favourite Songs",
        }
    }

    /// The order the list opens in.
    pub fn sort(self) -> SortKey {
        match self {
            Scope::All | Scope::Favourites => SortKey::Title,
            Scope::RecentlyAdded => SortKey::DateAdded,
            Scope::RecentlyPlayed => SortKey::LastPlayed,
        }
    }

    /// Whether `song` belongs to the list.
    pub fn keeps(self, song: &Song, stats: &Stats) -> bool {
        match self {
            Scope::All | Scope::RecentlyAdded => true,
            Scope::RecentlyPlayed => stats.last_played(&song.path) > 0,
            Scope::Favourites => stats.is_favourite(&song.path),
        }
    }

    /// The order that picks which `RECENT_LIMIT` songs a recent list holds; None for a list that is not capped.
    fn cap(self) -> Option<SortKey> {
        match self {
            Scope::RecentlyAdded => Some(SortKey::DateAdded),
            Scope::RecentlyPlayed => Some(SortKey::LastPlayed),
            Scope::All | Scope::Favourites => None,
        }
    }
}

/// Whether every word of `query` appears in the song's title, artist or
/// album (case-insensitive). An empty query matches everything.
pub fn matches(song: &Song, query: &str) -> bool {
    let hay = format!("{}\n{}\n{}", song.title, song.artist, song.album).to_lowercase();
    query.split_whitespace().all(|w| hay.contains(&w.to_lowercase()))
}

/// Ids of the songs matching `query`, in `sort` order. Ties fall back to
/// the title, then the path, so the order is stable.
pub fn view(songs: &[Song], query: &str, sort: SortKey) -> Vec<usize> {
    view_with(songs, query, sort, Scope::All, &Stats::default())
}

/// Like `view`, for the songs of `scope`; `stats` supplies the ratings, plays and favourites. A recent list is capped
/// to its newest `RECENT_LIMIT` songs before `sort` orders them.
pub fn view_with(songs: &[Song], query: &str, sort: SortKey, scope: Scope, stats: &Stats) -> Vec<usize> {
    let mut ids: Vec<usize> = songs.iter().filter(|s| scope.keeps(s, stats) && matches(s, query)).map(|s| s.id).collect();
    if let Some(cap) = scope.cap() {
        order(songs, &mut ids, cap, stats);
        ids.truncate(RECENT_LIMIT);
    }
    order(songs, &mut ids, sort, stats);
    ids
}

/// `view_with`, reversed when `descending`: the same songs in the opposite order of `sort`. A recent list keeps its
/// newest `RECENT_LIMIT` songs whichever way it is sorted, because the cap is applied before the order.
pub fn view_in_order(songs: &[Song], query: &str, sort: SortKey, descending: bool, scope: Scope, stats: &Stats) -> Vec<usize> {
    let mut ids = view_with(songs, query, sort, scope, stats);
    if descending {
        ids.reverse();
    }
    ids
}

/// Sorts `ids` by `sort`; ties fall back to the title, then the path.
fn order(songs: &[Song], ids: &mut [usize], sort: SortKey, stats: &Stats) {
    let title = |s: &Song| (sort_key(&s.title), s.path.clone());
    match sort {
        SortKey::Title => ids.sort_by_cached_key(|&i| title(&songs[i])),
        SortKey::Artist => ids.sort_by_cached_key(|&i| (sort_key(&songs[i].artist), title(&songs[i]))),
        SortKey::Album => ids.sort_by_cached_key(|&i| (sort_key(&songs[i].album), songs[i].track.unwrap_or(u32::MAX), title(&songs[i]))),
        SortKey::DateAdded => ids.sort_by_cached_key(|&i| (Reverse(songs[i].added), title(&songs[i]))),
        SortKey::Rating => ids.sort_by_cached_key(|&i| (Reverse(stats.rating(&songs[i].path)), title(&songs[i]))),
        SortKey::Plays => ids.sort_by_cached_key(|&i| (Reverse(stats.plays(&songs[i].path)), title(&songs[i]))),
        SortKey::LastPlayed => ids.sort_by_cached_key(|&i| (Reverse(stats.last_played(&songs[i].path)), title(&songs[i]))),
    }
}

/// The A-Z index slots, as bifrost shows them: A to Z, then "#" for everything else.
pub const INDEX: [char; 27] = ['A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'I', 'J', 'K', 'L', 'M', 'N', 'O', 'P', 'Q', 'R', 'S', 'T', 'U', 'V', 'W', 'X', 'Y', 'Z', '#'];

/// The index slot a name files under (leading articles ignored, like the sort).
pub fn slot_of(name: &str) -> usize {
    sort_key(name).chars().nth(1).map(|c| c.to_ascii_uppercase()).filter(char::is_ascii_alphabetic).map_or(26, |c| (c as u8 - b'A') as usize)
}

/// Whether the list order is alphabetic, so an A-Z index makes sense.
pub fn indexed(sort: SortKey) -> bool {
    matches!(sort, SortKey::Title | SortKey::Artist | SortKey::Album)
}

/// For each index slot, the position in `ids` of the first song filed under
/// it (None when no song is), for the sort in force.
pub fn sections(songs: &[Song], ids: &[usize], sort: SortKey) -> [Option<usize>; 27] {
    let mut out = [None; 27];
    if !indexed(sort) {
        return out;
    }
    for (pos, &id) in ids.iter().enumerate() {
        let s = &songs[id];
        let slot = slot_of(match sort {
            SortKey::Artist => &s.artist,
            SortKey::Album => &s.album,
            _ => &s.title,
        });
        out[slot].get_or_insert(pos);
    }
    out
}

/// Where tapping `slot` scrolls to: its own section, else the next one down,
/// else the last one.
pub fn target_for(sections: &[Option<usize>; 27], slot: usize) -> Option<usize> {
    sections[slot.min(26)..].iter().flatten().next().or_else(|| sections.iter().flatten().last()).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song(id: usize, title: &str, artist: &str, album: &str, track: Option<u32>, added: u64) -> Song {
        Song { id, path: format!("/m/{id}.mp3").into(), title: title.into(), artist: artist.into(), album: album.into(), album_artist: artist.into(), track, year: None, duration_ms: None, has_cover: false, added }
    }

    fn songs() -> Vec<Song> {
        vec![
            song(0, "Gold", "Abba", "Gold", Some(2), 300),
            song(1, "The Wall", "Pink Floyd", "The Wall", Some(1), 100),
            song(2, "Chiquitita", "Abba", "Gold", Some(1), 200),
            song(3, "Another Brick", "Pink Floyd", "The Wall", Some(2), 100),
        ]
    }

    #[test]
    fn sorts_by_each_key() {
        let s = songs();
        assert_eq!(view(&s, "", SortKey::Title), [3, 2, 0, 1]);
        assert_eq!(view(&s, "", SortKey::Artist), [2, 0, 3, 1]);
        assert_eq!(view(&s, "", SortKey::Album), [2, 0, 1, 3], "Gold before The Wall; tracks in order");
        assert_eq!(view(&s, "", SortKey::DateAdded), [0, 2, 3, 1], "newest first, ties by title");
    }

    #[test]
    fn search_matches_every_word_across_title_artist_and_album() {
        let s = songs();
        assert_eq!(view(&s, "abba", SortKey::Title), [2, 0]);
        assert_eq!(view(&s, "WALL floyd", SortKey::Title), [3, 1]);
        assert_eq!(view(&s, "brick wall", SortKey::Title), [3], "words may come from different fields");
        assert_eq!(view(&s, "  ", SortKey::Title).len(), 4);
        assert!(view(&s, "zzz", SortKey::Title).is_empty());
    }

    #[test]
    fn sort_cycles_through_every_order() {
        let mut k = SortKey::Title;
        let seen: Vec<_> = (0..SortKey::ALL.len()).map(|_| { let c = k; k = k.cycle(); c }).collect();
        assert_eq!(seen, SortKey::ALL);
        assert_eq!(k, SortKey::Title);
        assert_eq!(SortKey::DateAdded.label(), "Recently Added");
        assert_eq!(SortKey::LastPlayed.label(), "Recently Played");
    }

    fn stats() -> Stats {
        let mut st = Stats::default();
        // Gold (0): 5 stars, 3 plays, favourite; The Wall (1): 2 stars, never played; Chiquitita (2): 5 stars, 9 plays
        st.set_rating(std::path::Path::new("/m/0.mp3"), 5);
        st.toggle_favourite(std::path::Path::new("/m/0.mp3"));
        for t in [10, 20, 30] {
            st.record_play(std::path::Path::new("/m/0.mp3"), t);
        }
        st.set_rating(std::path::Path::new("/m/1.mp3"), 2);
        st.set_rating(std::path::Path::new("/m/2.mp3"), 5);
        for t in 1..=9 {
            st.record_play(std::path::Path::new("/m/2.mp3"), 100 + t);
        }
        st
    }

    #[test]
    fn sorts_by_rating_plays_and_last_played() {
        let (s, st) = (songs(), stats());
        assert_eq!(view_with(&s, "", SortKey::Rating, Scope::All, &st), [2, 0, 1, 3], "5 stars by title, then 2 stars, then none");
        assert_eq!(view_with(&s, "", SortKey::Plays, Scope::All, &st), [2, 0, 3, 1], "9 plays, 3 plays, then the rest by title");
        assert_eq!(view_with(&s, "", SortKey::LastPlayed, Scope::All, &st), [2, 0, 3, 1], "song 2 played last (109), then 0 (30)");
    }

    #[test]
    fn scopes_pick_their_songs() {
        let (s, st) = (songs(), stats());
        assert_eq!(view_with(&s, "", SortKey::Title, Scope::Favourites, &st), [0]);
        assert_eq!(view_with(&s, "", Scope::RecentlyPlayed.sort(), Scope::RecentlyPlayed, &st), [2, 0], "only songs that played, latest first");
        assert_eq!(view_with(&s, "", Scope::RecentlyAdded.sort(), Scope::RecentlyAdded, &st), [0, 2, 3, 1]);
        assert_eq!(view_with(&s, "abba", Scope::Favourites.sort(), Scope::Favourites, &st), [0], "the search applies inside a scope");
        assert!(view_with(&s, "wall", SortKey::Title, Scope::Favourites, &st).is_empty());
        assert_eq!(Scope::All.sort(), SortKey::Title);
        assert_eq!(Scope::ALL.map(Scope::label), ["Songs", "Recently Added", "Recently Played", "Favourite Songs"]);
    }

    #[test]
    fn a_recent_list_keeps_only_its_newest_songs() {
        let many: Vec<Song> = (0..RECENT_LIMIT + 30).map(|i| song(i, &format!("Song {i:03}"), "A", "B", None, i as u64)).collect();
        let ids = view_with(&many, "", SortKey::Title, Scope::RecentlyAdded, &Stats::default());
        assert_eq!(ids.len(), RECENT_LIMIT);
        assert!(ids.iter().all(|&i| many[i].added >= 30), "the 30 oldest are left out");
        assert_eq!(view_with(&many, "", SortKey::DateAdded, Scope::RecentlyAdded, &Stats::default())[0], RECENT_LIMIT + 29, "newest first");
    }

    #[test]
    fn only_the_alphabetic_orders_have_an_index() {
        assert!([SortKey::Title, SortKey::Artist, SortKey::Album].into_iter().all(indexed));
        assert!([SortKey::DateAdded, SortKey::Rating, SortKey::Plays, SortKey::LastPlayed].into_iter().all(|k| !indexed(k)));
    }

    #[test]
    fn index_sections_and_targets() {
        let s = songs();
        let ids = view(&s, "", SortKey::Title);
        let sec = sections(&s, &ids, SortKey::Title);
        assert_eq!((sec[0], sec[2], sec[6], sec[22]), (Some(0), Some(1), Some(2), Some(3)), "The Wall files under W");
        assert_eq!(target_for(&sec, 1), Some(1), "B has none, goes to C");
        assert_eq!(target_for(&sec, 26), Some(3), "past the end goes to the last section");
        assert_eq!(sections(&s, &ids, SortKey::DateAdded), [None; 27]);
        assert_eq!(slot_of("10cc"), 26);
    }
}
