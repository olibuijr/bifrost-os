//! The Songs list as the user sorts and searches it. Pure logic over the
//! library's songs; the result is a list of song ids to show.

use super::library::{sort_key, Song};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortKey {
    Title,
    Artist,
    Album,
    /// newest file first
    DateAdded,
}

impl SortKey {
    pub fn label(self) -> &'static str {
        match self {
            SortKey::Title => "Title",
            SortKey::Artist => "Artist",
            SortKey::Album => "Album",
            SortKey::DateAdded => "Recently Added",
        }
    }

    /// The next order when the sort button is tapped.
    pub fn cycle(self) -> SortKey {
        match self {
            SortKey::Title => SortKey::Artist,
            SortKey::Artist => SortKey::Album,
            SortKey::Album => SortKey::DateAdded,
            SortKey::DateAdded => SortKey::Title,
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
    let mut ids: Vec<usize> = songs.iter().filter(|s| matches(s, query)).map(|s| s.id).collect();
    let title = |s: &Song| (sort_key(&s.title), s.path.clone());
    match sort {
        SortKey::Title => ids.sort_by_cached_key(|&i| title(&songs[i])),
        SortKey::Artist => ids.sort_by_cached_key(|&i| (sort_key(&songs[i].artist), title(&songs[i]))),
        SortKey::Album => ids.sort_by_cached_key(|&i| (sort_key(&songs[i].album), songs[i].track.unwrap_or(u32::MAX), title(&songs[i]))),
        SortKey::DateAdded => ids.sort_by_cached_key(|&i| (std::cmp::Reverse(songs[i].added), title(&songs[i]))),
    }
    ids
}

/// The A-Z index slots, as bifrost shows them: A to Z, then "#" for everything else.
pub const INDEX: [char; 27] = ['A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'I', 'J', 'K', 'L', 'M', 'N', 'O', 'P', 'Q', 'R', 'S', 'T', 'U', 'V', 'W', 'X', 'Y', 'Z', '#'];

/// The index slot a name files under (leading articles ignored, like the sort).
pub fn slot_of(name: &str) -> usize {
    sort_key(name).chars().nth(1).map(|c| c.to_ascii_uppercase()).filter(char::is_ascii_alphabetic).map_or(26, |c| (c as u8 - b'A') as usize)
}

/// Whether the list order is alphabetic, so an A-Z index makes sense.
pub fn indexed(sort: SortKey) -> bool {
    sort != SortKey::DateAdded
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
    fn sort_cycles_through_all_four() {
        let mut k = SortKey::Title;
        let seen: Vec<_> = (0..4).map(|_| { let c = k; k = k.cycle(); c }).collect();
        assert_eq!(seen, [SortKey::Title, SortKey::Artist, SortKey::Album, SortKey::DateAdded]);
        assert_eq!(k, SortKey::Title);
        assert_eq!(SortKey::DateAdded.label(), "Recently Added");
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
