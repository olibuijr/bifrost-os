//! The music library: files found under the music folders, their tags,
//! and the Songs / Albums / Artists views built from them, sorted the way
//! bifrost sorts (case-insensitive, leading articles ignored).

use springboard::media::audiofile::{self, Tags};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// A cover bigger than this is not kept in memory.
const MAX_COVER: usize = 2 << 20;
pub const UNKNOWN_ARTIST: &str = "Unknown Artist";
pub const UNKNOWN_ALBUM: &str = "Unknown Album";

#[derive(Clone, Debug, PartialEq)]
pub struct Song {
    /// position in `Library::songs`
    pub id: usize,
    pub path: PathBuf,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub track: Option<u32>,
    pub year: Option<u32>,
    pub duration_ms: Option<f64>,
    pub has_cover: bool,
    /// when the file was last modified, Unix seconds ("date added")
    pub added: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Album {
    pub name: String,
    pub artist: String,
    pub year: Option<u32>,
    /// song ids in track order
    pub songs: Vec<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Artist {
    pub name: String,
    /// album indices in name order
    pub albums: Vec<usize>,
    pub song_count: usize,
}

/// Everything the Library tab shows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Library {
    /// in title order
    pub songs: Vec<Song>,
    /// in name order
    pub albums: Vec<Album>,
    /// in name order
    pub artists: Vec<Artist>,
    /// the cover picture of each album, still encoded, when a song has one
    pub covers: Vec<Option<Arc<[u8]>>>,
}

/// One scanned file before the library is built.
#[derive(Clone, Debug, PartialEq)]
pub struct Scanned {
    pub path: PathBuf,
    pub tags: Tags,
    pub duration_ms: Option<f64>,
    /// the file has a cover, even when `tags.cover` was dropped because its album has one
    pub has_cover: bool,
    /// file modification time, Unix seconds
    pub added: u64,
}

/// The key bifrost sorts names by: case-folded, leading "The"/"A"/"An" dropped,
/// digits and punctuation after letters.
pub fn sort_key(name: &str) -> String {
    let lower = name.trim().to_lowercase();
    let stripped = ["the ", "a ", "an "].iter().find_map(|p| lower.strip_prefix(p)).map(str::trim_start).unwrap_or(&lower).to_string();
    let first = stripped.chars().next().map_or(true, |c| c.is_alphabetic());
    format!("{}{stripped}", if first { '0' } else { '1' })
}

/// Every playable file under `roots`, recursively, in path order.
pub fn find_files(roots: &[PathBuf]) -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        let mut entries: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        entries.sort();
        for p in entries {
            if p.is_dir() {
                if depth < 8 && !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')) {
                    walk(&p, out, depth + 1);
                }
            } else if audiofile::format_of(&p).is_some() {
                out.push(p);
            }
        }
    }
    let mut out = Vec::new();
    for r in roots {
        walk(r, &mut out, 0);
    }
    out
}

/// Reads one file's tags and length; a file that will not open is skipped.
pub fn scan_file(path: &Path) -> Option<Scanned> {
    let mut tags = audiofile::read_tags(path).unwrap_or_default();
    if tags.cover.as_ref().is_some_and(|c| c.len() > MAX_COVER) {
        tags.cover = None;
    }
    let duration_ms = audiofile::open(path).ok().and_then(|d| d.duration_ms());
    let added = std::fs::metadata(path).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
    let has_cover = tags.cover.is_some();
    Some(Scanned { path: path.to_path_buf(), tags, duration_ms, has_cover, added })
}

/// Scans every file, keeping one cover per album: later songs of an album
/// that already has a cover drop theirs, so the peak stays at one picture per album.
pub fn scan_all(paths: &[PathBuf]) -> Vec<Scanned> {
    let mut covered: HashSet<(String, String)> = HashSet::new();
    let mut out = Vec::with_capacity(paths.len());
    for mut f in paths.iter().filter_map(|p| scan_file(p)) {
        drop_repeat_cover(&mut f, &mut covered);
        out.push(f);
    }
    out
}

/// Drops `f`'s cover when its album already has one in `covered`.
fn drop_repeat_cover(f: &mut Scanned, covered: &mut HashSet<(String, String)>) {
    if f.tags.cover.is_some() && !covered.insert(album_key(&f.tags)) {
        f.tags.cover = None;
    }
}

/// The album a file belongs to: (album, album artist), as the library groups them.
fn album_key(tags: &Tags) -> (String, String) {
    let artist = tags.artist.as_deref().unwrap_or(UNKNOWN_ARTIST);
    (sort_key(tags.album.as_deref().unwrap_or(UNKNOWN_ALBUM)), sort_key(tags.album_artist.as_deref().unwrap_or(artist)))
}

impl Library {
    /// Builds the views from scanned files.
    pub fn build(files: Vec<Scanned>) -> Library {
        let mut songs: Vec<(Song, Option<Arc<[u8]>>)> = files
            .into_iter()
            .map(|f| {
                let stem = f.path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                let artist = f.tags.artist.clone().unwrap_or_else(|| UNKNOWN_ARTIST.into());
                let song = Song {
                    id: 0,
                    path: f.path,
                    title: f.tags.title.clone().unwrap_or(stem),
                    album_artist: f.tags.album_artist.clone().unwrap_or_else(|| artist.clone()),
                    artist,
                    album: f.tags.album.clone().unwrap_or_else(|| UNKNOWN_ALBUM.into()),
                    track: f.tags.track,
                    year: f.tags.year,
                    duration_ms: f.duration_ms,
                    has_cover: f.has_cover,
                    added: f.added,
                };
                (song, f.tags.cover.map(Arc::from))
            })
            .collect();
        songs.sort_by_cached_key(|(s, _)| (sort_key(&s.title), s.path.clone()));
        let mut covers_by_song = Vec::with_capacity(songs.len());
        let songs: Vec<Song> = songs
            .into_iter()
            .enumerate()
            .map(|(i, (mut s, c))| {
                s.id = i;
                covers_by_song.push(c);
                s
            })
            .collect();
        // albums: keyed by (album, album artist)
        let mut album_of: HashMap<(String, String), usize> = HashMap::new();
        let mut albums: Vec<Album> = Vec::new();
        let mut covers: Vec<Option<Arc<[u8]>>> = Vec::new();
        for s in &songs {
            let key = (sort_key(&s.album), sort_key(&s.album_artist));
            let ai = *album_of.entry(key).or_insert_with(|| {
                albums.push(Album { name: s.album.clone(), artist: s.album_artist.clone(), year: s.year, songs: Vec::new() });
                covers.push(None);
                albums.len() - 1
            });
            albums[ai].songs.push(s.id);
            if covers[ai].is_none() {
                covers[ai] = covers_by_song[s.id].take();
            }
            if albums[ai].year.is_none() {
                albums[ai].year = s.year;
            }
        }
        for a in &mut albums {
            a.songs.sort_by_key(|&id| (songs[id].track.unwrap_or(u32::MAX), sort_key(&songs[id].title)));
        }
        let mut order: Vec<usize> = (0..albums.len()).collect();
        order.sort_by_cached_key(|&i| (sort_key(&albums[i].name), sort_key(&albums[i].artist)));
                let albums: Vec<Album> = order.iter().map(|&i| albums[i].clone()).collect();
        let covers: Vec<Option<Arc<[u8]>>> = order.iter().map(|&i| covers[i].take()).collect();
        // artists: keyed by album artist, albums in name order
        let mut artist_of: HashMap<String, usize> = HashMap::new();
        let mut artists: Vec<Artist> = Vec::new();
        for (ai, a) in albums.iter().enumerate() {
            let i = *artist_of.entry(sort_key(&a.artist)).or_insert_with(|| {
                artists.push(Artist { name: a.artist.clone(), albums: Vec::new(), song_count: 0 });
                artists.len() - 1
            });
            artists[i].albums.push(ai);
            artists[i].song_count += a.songs.len();
        }
        artists.sort_by_cached_key(|a| sort_key(&a.name));
        Library { songs, albums, artists, covers }
    }

    /// The album a song belongs to.
    pub fn album_of(&self, song: usize) -> Option<usize> {
        self.albums.iter().position(|a| a.songs.contains(&song))
    }

    pub fn is_empty(&self) -> bool {
        self.songs.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use springboard::shell::music::nowplaying::clock;

    fn scanned(title: &str, artist: Option<&str>, album: Option<&str>, track: Option<u32>, cover: bool) -> Scanned {
        Scanned {
            path: PathBuf::from(format!("/m/{}.mp3", title.replace(' ', "_"))),
            tags: Tags { title: Some(title.into()), artist: artist.map(Into::into), album: album.map(Into::into), album_artist: None, track, year: None, cover: cover.then(|| b"\xFF\xD8".to_vec()) },
            duration_ms: Some(200_000.0),
            has_cover: cover,
            added: 0,
        }
    }

    #[test]
    fn only_the_first_cover_of_an_album_is_kept_in_memory() {
        let mut covered = HashSet::new();
        let mut a = scanned("a", Some("X"), Some("Alb"), Some(1), true);
        let mut b = scanned("b", Some("X"), Some("Alb"), Some(2), true);
        let mut c = scanned("c", Some("X"), Some("Other"), Some(1), true);
        for f in [&mut a, &mut b, &mut c] {
            drop_repeat_cover(f, &mut covered);
        }
        assert!(a.tags.cover.is_some() && b.tags.cover.is_none() && c.tags.cover.is_some());
        assert!(b.has_cover, "the song still reports having a cover");
        let lib = Library::build(vec![a, b, c]);
        assert!(lib.songs.iter().all(|s| s.has_cover));
    }

    #[test]
    fn sort_keys_ignore_case_and_articles_and_put_digits_last() {
        let mut names = vec!["The Beatles", "abba", "Zed", "10cc", "A Tribe", "Björk"];
        names.sort_by_key(|n| sort_key(n));
        assert_eq!(names, ["abba", "The Beatles", "Björk", "A Tribe", "Zed", "10cc"]);
        assert_eq!(sort_key(""), "0");
    }

    #[test]
    fn clock_formats_minutes_and_hours() {
        assert_eq!(clock(0.0), "0:00");
        assert_eq!(clock(187_400.0), "3:07");
        assert_eq!(clock(3_600_000.0 + 61_000.0), "1:01:01");
        assert_eq!(clock(-5.0), "0:00");
    }

    #[test]
    fn builds_songs_albums_and_artists_sorted_with_fallbacks() {
        let files = vec![
            scanned("Zebra", Some("The Band"), Some("Second"), Some(1), false),
            scanned("apricot", Some("The Band"), Some("First"), Some(2), true),
            scanned("Mango", Some("The Band"), Some("First"), Some(1), false),
            scanned("The Lonely", Some("Abba"), Some("Gold"), None, false),
            Scanned { path: "/m/untagged.m4a".into(), tags: Tags::default(), duration_ms: None, has_cover: false, added: 0 },
        ];
        let lib = Library::build(files);
        let titles: Vec<&str> = lib.songs.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(titles, ["apricot", "The Lonely", "Mango", "untagged", "Zebra"]);
        assert!(lib.songs.iter().enumerate().all(|(i, s)| s.id == i));
        let untagged = &lib.songs[3];
        assert_eq!((untagged.artist.as_str(), untagged.album.as_str(), untagged.album_artist.as_str()), (UNKNOWN_ARTIST, UNKNOWN_ALBUM, UNKNOWN_ARTIST));
        let albums: Vec<(&str, &str)> = lib.albums.iter().map(|a| (a.name.as_str(), a.artist.as_str())).collect();
        assert_eq!(albums, [("First", "The Band"), ("Gold", "Abba"), ("Second", "The Band"), (UNKNOWN_ALBUM, UNKNOWN_ARTIST)]);
        let first = &lib.albums[0];
        assert_eq!(first.songs.iter().map(|&i| lib.songs[i].title.as_str()).collect::<Vec<_>>(), ["Mango", "apricot"], "track order");
        assert!(lib.covers[0].is_some() && lib.covers[1].is_none(), "the album takes a song's cover");
        let artists: Vec<(&str, usize)> = lib.artists.iter().map(|a| (a.name.as_str(), a.song_count)).collect();
        assert_eq!(artists, [("Abba", 1), ("The Band", 3), (UNKNOWN_ARTIST, 1)]);
        assert_eq!(lib.artists[1].albums.iter().map(|&a| lib.albums[a].name.as_str()).collect::<Vec<_>>(), ["First", "Second"]);
        assert_eq!(lib.album_of(4), Some(2));
        assert!(!lib.is_empty() && Library::default().is_empty());
    }

    #[test]
    fn finds_and_scans_files_under_the_roots() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/test-music-lib");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub/.hidden")).unwrap();
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../src/media/testdata");
        std::fs::copy(fixtures.join("tone_id3v23.mp3"), dir.join("b.mp3")).unwrap();
        std::fs::copy(fixtures.join("tone.m4a"), dir.join("sub/a.m4a")).unwrap();
        std::fs::copy(fixtures.join("tone.m4a"), dir.join("sub/.hidden/c.m4a")).unwrap();
        std::fs::write(dir.join("notes.txt"), b"x").unwrap();
        let files = find_files(&[dir.clone(), PathBuf::from("/nonexistent/music")]);
        assert_eq!(files, [dir.join("b.mp3"), dir.join("sub/a.m4a")]);
        let scanned: Vec<Scanned> = files.iter().filter_map(|p| scan_file(p)).collect();
        assert_eq!(scanned[0].tags.title.as_deref(), Some("Tone Song"));
        assert!((scanned[0].duration_ms.unwrap() - 1200.0).abs() < 50.0);
        assert!(scanned[1].tags.cover.is_some());
        let lib = Library::build(scanned);
        assert_eq!(lib.albums.len(), 2, "same album name, different album artists");
        assert_eq!(lib.artists.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), ["Fixture Artist", "Various"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
