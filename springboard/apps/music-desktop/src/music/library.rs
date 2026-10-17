//! The music library: files found under the music folders, their tags,
//! and the Songs / Albums / Artists views built from them, sorted the way
//! bifrost sorts (case-insensitive, leading articles ignored).

use bifrost_app::controls::media_changes::Stamp;
use bifrost_app::controls::media_sources::{LoadOptions, MediaSource, SourceType};
use bifrost_app::defaults::UserDefaults;
use bifrost_core::smf::Smf;
use springboard::media::audiofile::{self, Tags};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::UNIX_EPOCH;

/// A cover bigger than this is not kept in memory.
const MAX_COVER: usize = 2 << 20;
/// A MIDI file bigger than this is skipped (its events are read whole).
const MAX_MIDI: u64 = 16 << 20;
pub const UNKNOWN_ARTIST: &str = "Unknown Artist";
pub const UNKNOWN_ALBUM: &str = "Unknown Album";
/// The artist a MIDI file is listed under: it has no tags, and it plays no sound yet.
pub const MIDI_ARTIST: &str = "MIDI file";

/// The defaults keys the library load options are read from (`library_load_options`). A folder source is a folder
/// directly inside a music root; the include and exclude lists hold one folder path per line.
pub const PREF_LOAD_FOLDERS: &str = "music.library.loadFolders";
pub const PREF_INCLUDE: &str = "music.library.include";
pub const PREF_EXCLUDE: &str = "music.library.exclude";

/// Whether a file is a Standard MIDI File by its extension.
pub fn is_midi(path: &Path) -> bool {
    matches!(path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref(), Some("mid") | Some("midi"))
}

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

impl Song {
    /// A MIDI file: listed and timed, but the audio service has no decoder for it, so it is never queued.
    pub fn is_midi(&self) -> bool {
        is_midi(&self.path)
    }
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
    find_files_except(roots, &[])
}

/// Like `find_files`, but the folders in `skip` (and what is inside them) are left out.
pub fn find_files_except(roots: &[PathBuf], skip: &[PathBuf]) -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>, depth: usize, skip: &[PathBuf]) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        let mut entries: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        entries.sort();
        for p in entries {
            if p.is_dir() {
                if depth < 8 && !skip.contains(&p) && !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')) {
                    walk(&p, out, depth + 1, skip);
                }
            } else if audiofile::format_of(&p).is_some() || is_midi(&p) {
                out.push(p);
            }
        }
    }
    let mut out = Vec::new();
    for r in roots {
        walk(r, &mut out, 0, skip);
    }
    out
}

/// The folders directly inside `root`, visible ones only, in path order.
fn child_folders(root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(root) else { return Vec::new() };
    let mut dirs: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.is_dir() && !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.'))).collect();
    dirs.sort();
    dirs
}

/// The folders to walk and the folders to leave out, for each music root under the load options. The root is a file
/// source of its own, and each folder directly inside it is a folder source. A kept root is walked with the dropped
/// folders skipped. A dropped root is not walked, but each kept folder inside it is.
pub fn plan(roots: &[PathBuf], opts: &LoadOptions) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let mut walk = Vec::new();
    let mut skip = Vec::new();
    for root in roots {
        let folders = child_folders(root);
        let mut sources: Vec<MediaSource> = MediaSource::new(&root.to_string_lossy(), SourceType::Audio, false).into_iter().collect();
        for f in &folders {
            sources.extend(MediaSource::new(&f.to_string_lossy(), SourceType::Audio, true));
        }
        let kept: Vec<PathBuf> = opts.filter(&sources).into_iter().map(|s| PathBuf::from(s.identifier)).collect();
        if kept.iter().any(|k| k == root) {
            walk.push(root.clone());
            skip.extend(folders.iter().filter(|f| !kept.contains(*f)).cloned());
        } else {
            walk.extend(folders.iter().filter(|f| kept.contains(*f)).cloned());
        }
    }
    (walk, skip)
}

/// The library load options from the app's defaults. An absent folders key loads folders; an absent include key
/// applies no include filter; the exclude list is empty when the key is absent.
pub fn library_load_options(d: &UserDefaults) -> LoadOptions {
    let folders = d.contains(PREF_LOAD_FOLDERS).then(|| d.bool(PREF_LOAD_FOLDERS));
    let include = d.contains(PREF_INCLUDE).then(|| path_lines(&d.string(PREF_INCLUDE).unwrap_or_default()));
    let exclude = path_lines(&d.string(PREF_EXCLUDE).unwrap_or_default());
    LoadOptions::from_keys(folders, include, Some(exclude))
}

/// One path per line; blank lines are dropped and each path is trimmed.
fn path_lines(text: &str) -> Vec<String> {
    text.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect()
}

/// The change stamps of `files`: the identifier is the path, and the revision moves when the file's modification time
/// or size changes.
pub fn stamps_of(files: &[PathBuf]) -> Vec<Stamp> {
    files.iter().map(|p| Stamp { id: p.to_string_lossy().into_owned(), revision: revision_of(p) }).collect()
}

fn revision_of(path: &Path) -> u64 {
    match std::fs::metadata(path) {
        Ok(m) => {
            let modified_ns = m.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos() as u64);
            modified_ns.wrapping_add(m.len())
        }
        Err(_) => 0,
    }
}

/// Modification time in Unix seconds ("date added"), 0 when unknown.
fn added_of(path: &Path) -> u64 {
    std::fs::metadata(path).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs())
}

/// The folder inside a music root that File > Import copies songs into.
pub const IMPORT_DIR: &str = "Imported";

/// Whether the library lists a file of this name: a format the audio service plays, or a MIDI file.
pub fn is_library_file(name: &str) -> bool {
    let path = Path::new(name);
    audiofile::format_of(path).is_some() || is_midi(path)
}

/// The path in `dir` for a file called `name` that does not exist yet: `name` itself, else "stem 2.ext", "stem 3.ext", ...
pub fn free_name(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let path = Path::new(name);
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = path.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    (2..).map(|n| dir.join(format!("{stem} {n}{ext}"))).find(|p| !p.exists()).unwrap_or(first)
}

/// Copies one song into the library: reads `source` into `<root>/Imported/<name>`, under a free name. Returns the new
/// path, or None when `name` is not a file the library lists (nothing is written then). The file is written under a
/// temporary name and moved into place, so a scan never sees half of it.
pub fn import_into(root: &Path, name: &str, source: &mut impl std::io::Read) -> std::io::Result<Option<PathBuf>> {
    // only the last component counts: a name never reaches outside the import folder
    let Some(name) = Path::new(name).file_name().map(|n| n.to_string_lossy().into_owned()) else { return Ok(None) };
    if name.starts_with('.') || !is_library_file(&name) {
        return Ok(None);
    }
    let dir = root.join(IMPORT_DIR);
    std::fs::create_dir_all(&dir)?;
    let dest = free_name(&dir, &name);
    let tmp = dir.join(format!(".{name}.part"));
    let copied = std::fs::File::create(&tmp).and_then(|mut out| std::io::copy(source, &mut out));
    if let Err(e) = copied {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    std::fs::rename(&tmp, &dest)?;
    Ok(Some(dest))
}

/// A MIDI file: its length is the sequence's length from the Standard MIDI File reader. It has no tags and no
/// cover. A file that is too big or does not parse is skipped.
fn scan_midi(path: &Path) -> Option<Scanned> {
    if std::fs::metadata(path).ok()?.len() > MAX_MIDI {
        return None;
    }
    let smf = Smf::parse(&std::fs::read(path).ok()?).ok()?;
    let tags = Tags { artist: Some(MIDI_ARTIST.into()), ..Tags::default() };
    Some(Scanned { path: path.to_path_buf(), tags, duration_ms: Some(smf.duration() * 1000.0), has_cover: false, added: added_of(path) })
}

/// Reads one file's tags and length; a file that will not open is skipped.
pub fn scan_file(path: &Path) -> Option<Scanned> {
    if is_midi(path) {
        return scan_midi(path);
    }
    let mut tags = audiofile::read_tags(path).unwrap_or_default();
    if tags.cover.as_ref().is_some_and(|c| c.len() > MAX_COVER) {
        tags.cover = None;
    }
    let duration_ms = audiofile::open(path).ok().and_then(|d| d.duration_ms());
    let has_cover = tags.cover.is_some();
    Some(Scanned { path: path.to_path_buf(), tags, duration_ms, has_cover, added: added_of(path) })
}

/// The encoded picture embedded in one file's tags, when it has one within the size cap. A file that does not read has none.
pub fn artwork_of(path: &Path) -> Option<Vec<u8>> {
    audiofile::read_tags(path).ok()?.cover.filter(|c| c.len() <= MAX_COVER)
}

/// The MIME type of an encoded picture, named by its signature: PNG or JPEG. Other formats give None.
pub fn artwork_format(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else {
        None
    }
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

    /// The encoded cover picture of album `album` (an index into `albums`), when one was read.
    pub fn cover_of(&self, album: usize) -> Option<&[u8]> {
        self.covers.get(album)?.as_deref()
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
            tags: Tags { title: Some(title.into()), artist: artist.map(Into::into), album: album.map(Into::into), album_artist: None, track, year: None, cover: cover.then(|| b"\xFF\xD8".to_vec()), lyrics: None, replaygain_db: None },
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

    /// A format 0 file: 480 ticks per quarter, one note on at tick 0, its note off and the end of the track at tick
    /// 960. At the default tempo (120 beats per minute) 960 ticks are two quarters, so the length is 1.0 s.
    fn midi_bytes() -> Vec<u8> {
        let mut b = b"MThd".to_vec();
        b.extend([0, 0, 0, 6, 0, 0, 0, 1, 0x01, 0xE0]);
        b.extend(b"MTrk");
        b.extend([0, 0, 0, 13]);
        b.extend([0x00, 0x90, 0x3C, 0x40, 0x87, 0x40, 0x80, 0x3C, 0x00, 0x00, 0xFF, 0x2F, 0x00]);
        b
    }

    #[test]
    fn importing_copies_a_song_into_the_import_folder_under_a_free_name() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/test-music-import");
        let _ = std::fs::remove_dir_all(&root);
        let bytes = midi_bytes();
        let first = import_into(&root, "riff.mid", &mut bytes.as_slice()).unwrap().unwrap();
        assert_eq!(first, root.join("Imported/riff.mid"));
        assert_eq!(std::fs::read(&first).unwrap(), bytes);
        let second = import_into(&root, "riff.mid", &mut bytes.as_slice()).unwrap().unwrap();
        assert_eq!(second, root.join("Imported/riff 2.mid"), "a taken name gets a number");
        let third = import_into(&root, "riff.mid", &mut bytes.as_slice()).unwrap().unwrap();
        assert_eq!(third, root.join("Imported/riff 3.mid"));
        assert_eq!(import_into(&root, "notes.txt", &mut bytes.as_slice()).unwrap(), None, "not a song");
        assert_eq!(import_into(&root, ".hidden.mid", &mut bytes.as_slice()).unwrap(), None);
        assert_eq!(import_into(&root, "", &mut bytes.as_slice()).unwrap(), None);
        let escaped = import_into(&root, "../../out.mid", &mut bytes.as_slice()).unwrap().unwrap();
        assert_eq!(escaped, root.join("Imported/out.mid"), "only the file name is used");
        assert!(!root.join("Imported").read_dir().unwrap().flatten().any(|e| e.file_name().to_string_lossy().ends_with(".part")), "no temporary file is left");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_library_lists_the_formats_the_audio_service_plays() {
        assert!(is_library_file("a.mp3") && is_library_file("B.M4A") && is_library_file("c.mid"));
        assert!(!is_library_file("d.txt") && !is_library_file("e") && !is_library_file("f.jpg"));
    }

    #[test]
    fn midi_files_are_recognised_by_extension() {
        assert!(is_midi(Path::new("/m/Song.MID")));
        assert!(is_midi(Path::new("b.midi")));
        assert!(!is_midi(Path::new("c.mp3")));
        assert!(!is_midi(Path::new("noext")));
    }

    #[test]
    fn a_midi_file_is_listed_with_its_sequence_length_and_one_that_does_not_parse_is_skipped() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/test-music-midi");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("a")).unwrap();
        std::fs::write(dir.join("riff.mid"), midi_bytes()).unwrap();
        std::fs::write(dir.join("a/Bad.midi"), b"not midi").unwrap();
        std::fs::write(dir.join("a/two.mid"), midi_bytes()).unwrap();
        let files = find_files(&[dir.clone()]);
        assert_eq!(files, [dir.join("a/Bad.midi"), dir.join("a/two.mid"), dir.join("riff.mid")]);
        let scanned: Vec<Scanned> = files.iter().filter_map(|p| scan_file(p)).collect();
        assert_eq!(scanned.len(), 2, "the file that does not parse is skipped");
        assert_eq!(scanned[0].tags.artist.as_deref(), Some(MIDI_ARTIST));
        assert!((scanned[0].duration_ms.unwrap() - 1000.0).abs() < 1e-6);
        let lib = Library::build(scanned);
        assert_eq!(lib.songs[0].title, "riff", "the title comes from the file name");
        assert!(lib.songs[0].is_midi());
        assert_eq!(lib.songs[0].artist, MIDI_ARTIST);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_load_options_choose_the_folders_a_library_walks() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/test-music-plan");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("a")).unwrap();
        std::fs::create_dir_all(dir.join("b")).unwrap();
        std::fs::write(dir.join("top.mid"), midi_bytes()).unwrap();
        std::fs::write(dir.join("a/one.mid"), midi_bytes()).unwrap();
        std::fs::write(dir.join("b/two.mid"), midi_bytes()).unwrap();
        let roots = [dir.clone()];
        let (a, b) = (dir.join("a"), dir.join("b"));
        let load = |opts: &LoadOptions| {
            let (walk, skip) = plan(&roots, opts);
            find_files_except(&walk, &skip)
        };
        // everything: the root is walked, folders in path order
        assert_eq!(load(&LoadOptions::all()), [a.join("one.mid"), b.join("two.mid"), dir.join("top.mid")]);
        // folder a excluded: the root is walked with a skipped
        let no_a = LoadOptions::from_keys(None, None, Some(vec![a.to_string_lossy().into_owned()]));
        assert_eq!(load(&no_a), [b.join("two.mid"), dir.join("top.mid")]);
        // folders off: only the root's own files
        assert_eq!(load(&LoadOptions::from_keys(Some(false), None, None)), [dir.join("top.mid")]);
        // only b included: the root is dropped, and b is walked on its own
        let only_b = LoadOptions::from_keys(None, Some(vec![b.to_string_lossy().into_owned()]), None);
        assert_eq!(load(&only_b), [b.join("two.mid")]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn stamps_name_each_file_and_move_when_it_changes() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/test-music-stamps");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("x.mid"), midi_bytes()).unwrap();
        let files = vec![dir.join("x.mid")];
        let first = stamps_of(&files);
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].id, dir.join("x.mid").to_string_lossy().into_owned());
        assert_eq!(stamps_of(&files), first, "an unchanged file keeps its revision");
        std::fs::write(dir.join("x.mid"), b"a longer body than the first one").unwrap();
        assert_ne!(stamps_of(&files)[0].revision, first[0].revision);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_library_load_keys_are_read_from_the_defaults() {
        let mut d = UserDefaults::memory();
        assert_eq!(library_load_options(&d), LoadOptions::all());
        d.set_string(PREF_EXCLUDE, "/m/a\n  /m/b \n\n");
        d.set_bool(PREF_LOAD_FOLDERS, false);
        d.set_string(PREF_INCLUDE, "/m/a");
        assert_eq!(
            library_load_options(&d),
            LoadOptions { load_folders: false, include: Some(vec!["/m/a".into()]), exclude: vec!["/m/a".into(), "/m/b".into()] }
        );
    }
}
