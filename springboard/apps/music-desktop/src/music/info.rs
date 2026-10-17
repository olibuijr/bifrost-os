//! Get Info: the title, artist and album a song is shown with, as the user edits them in the app. The edits are kept in
//! a text file beside the library, one line per song and keyed by its path, and laid over the file's tags at every
//! scan. An edit is also written into the song's file through the tag writer where the format takes tags (MP3, ADTS,
//! M4A); see `write_back`. Pure file logic, no UI.

use super::library::Scanned;
use springboard::media::audiofile;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The tags Get Info edits, in the order the sheet lists them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Title,
    Artist,
    Album,
}

impl Field {
    pub const ALL: [Field; 3] = [Field::Title, Field::Artist, Field::Album];

    pub fn label(self) -> &'static str {
        match self {
            Field::Title => "Title",
            Field::Artist => "Artist",
            Field::Album => "Album",
        }
    }
}

/// The edits made to one song. A field that is None shows the file's own tag.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Edit {
    title: Option<String>,
    artist: Option<String>,
    album: Option<String>,
}

impl Edit {
    fn slot(&self, field: Field) -> &Option<String> {
        match field {
            Field::Title => &self.title,
            Field::Artist => &self.artist,
            Field::Album => &self.album,
        }
    }

    fn slot_mut(&mut self, field: Field) -> &mut Option<String> {
        match field {
            Field::Title => &mut self.title,
            Field::Artist => &mut self.artist,
            Field::Album => &mut self.album,
        }
    }

    /// The edited value of `field`, if the song has one.
    pub fn get(&self, field: Field) -> Option<&str> {
        self.slot(field).as_deref()
    }

    fn is_empty(&self) -> bool {
        Field::ALL.iter().all(|f| self.slot(*f).is_none())
    }
}

/// The edits of every song, by path, and the file they are kept in.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Info {
    file: Option<PathBuf>,
    edits: BTreeMap<PathBuf, Edit>,
}

impl Info {
    /// Reads the edits kept in `file`, which may not exist yet. A line that does not parse is skipped.
    pub fn open(file: PathBuf) -> Self {
        let edits = std::fs::read_to_string(&file).map(|text| parse(&text)).unwrap_or_default();
        Info { file: Some(file), edits }
    }

    /// The value `field` of the song at `path` is edited to, if it is.
    pub fn value(&self, path: &Path, field: Field) -> Option<&str> {
        self.edits.get(path).and_then(|e| e.get(field))
    }

    /// Sets `field` of the song at `path` to `text`, trimmed. An empty text takes the edit away. True when the edits
    /// changed (and were saved).
    pub fn set(&mut self, path: &Path, field: Field, text: &str) -> bool {
        let text = text.trim();
        let value = (!text.is_empty()).then(|| text.to_string());
        let current = self.value(path, field).map(str::to_string);
        if current == value {
            return false;
        }
        let entry = self.edits.entry(path.to_path_buf()).or_default();
        *entry.slot_mut(field) = value;
        if entry.is_empty() {
            self.edits.remove(path);
        }
        self.save();
        write_back(path, field, self.value(path, field));
        true
    }

    /// Drops every edit of the song at `path`. True when it had any.
    pub fn reset(&mut self, path: &Path) -> bool {
        let had = self.edits.remove(path).is_some();
        if had {
            self.save();
        }
        had
    }

    /// Lays the edits of a scanned file over its tags, before the library is built from it.
    pub fn apply(&self, file: &mut Scanned) {
        let Some(edit) = self.edits.get(&file.path) else { return };
        if let Some(title) = &edit.title {
            file.tags.title = Some(title.clone());
        }
        if let Some(artist) = &edit.artist {
            file.tags.artist = Some(artist.clone());
        }
        if let Some(album) = &edit.album {
            file.tags.album = Some(album.clone());
        }
    }

    fn save(&self) {
        let Some(file) = &self.file else { return };
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = file.with_extension("tmp");
        if std::fs::write(&tmp, self.text()).is_ok() {
            let _ = std::fs::rename(&tmp, file);
        }
    }

    /// The file's text: one line per song, its path and the three edited fields, tab-separated; an empty field is no
    /// edit.
    fn text(&self) -> String {
        let mut text = String::new();
        for (path, edit) in &self.edits {
            let field = |f: Field| edit.get(f).map(escape).unwrap_or_default();
            text.push_str(&format!("{}\t{}\t{}\t{}\n", escape(&path.to_string_lossy()), field(Field::Title), field(Field::Artist), field(Field::Album)));
        }
        text
    }
}

/// Writes one edited field into the song's own file through the tag writer (MP3, ADTS and M4A). The other tags are
/// read first and written back unchanged. A file whose tags cannot be written (FLAC, WAV, ...) keeps the sidecar edit
/// only, and so does a file with chapters, because the ID3 writer does not emit `CHAP` frames yet. An empty value
/// clears the tag in the file.
fn write_back(path: &Path, field: Field, value: Option<&str>) {
    let Ok(mut tags) = audiofile::read_tags(path) else { return };
    if !tags.chapters.is_empty() {
        return;
    }
    let value = value.map(str::to_string);
    match field {
        Field::Title => tags.title = value,
        Field::Artist => tags.artist = value,
        Field::Album => tags.album = value,
    }
    let _ = audiofile::write_tags(path, &tags);
}

/// The edits in a file's text, as `Info::text` writes them.
fn parse(text: &str) -> BTreeMap<PathBuf, Edit> {
    let mut edits = BTreeMap::new();
    for line in text.lines() {
        let cols: Vec<String> = line.split('\t').map(unescape).collect();
        if cols.len() != 4 || cols[0].is_empty() {
            continue;
        }
        let some = |s: &String| (!s.is_empty()).then(|| s.clone());
        let edit = Edit { title: some(&cols[1]), artist: some(&cols[2]), album: some(&cols[3]) };
        if !edit.is_empty() {
            edits.insert(PathBuf::from(&cols[0]), edit);
        }
    }
    edits
}

/// Escapes a backslash, a tab and a line break, so a value never spans columns or lines.
fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out
}

/// Reads back what `escape` wrote.
fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_edit_is_kept_by_path_and_an_empty_value_takes_it_away() {
        let mut info = Info::default();
        let song = Path::new("/m/a.mp3");
        assert!(info.set(song, Field::Title, "  New title "));
        assert_eq!(info.value(song, Field::Title), Some("New title"));
        assert_eq!(info.value(song, Field::Artist), None);
        assert!(!info.set(song, Field::Title, "New title"), "the same value changes nothing");
        assert!(info.set(song, Field::Title, "   "), "blank takes the edit away");
        assert_eq!(info.value(song, Field::Title), None);
        assert!(info.edits.is_empty(), "a song with no edit is not kept");
        assert!(!info.reset(song));
    }

    #[test]
    fn edits_survive_a_reopen_with_tabs_and_line_breaks_in_them() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/test-info/roundtrip");
        let _ = std::fs::remove_dir_all(&dir);
        let file = dir.join("info.txt");
        let mut info = Info::open(file.clone());
        let song = Path::new("/m/tab\tname.mp3");
        info.set(song, Field::Artist, "Odd\\Artist\twith tab");
        info.set(song, Field::Album, "Line\nBreak");
        let back = Info::open(file);
        assert_eq!(back.value(song, Field::Artist), Some("Odd\\Artist\twith tab"));
        assert_eq!(back.value(song, Field::Album), Some("Line\nBreak"));
        assert_eq!(back.value(song, Field::Title), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_line_that_does_not_parse_is_skipped() {
        let edits = parse("garbage\n\t\t\t\n/m/ok.mp3\tT\t\t\n/m/short.mp3\tT\n");
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[Path::new("/m/ok.mp3")].get(Field::Title), Some("T"));
    }
}
