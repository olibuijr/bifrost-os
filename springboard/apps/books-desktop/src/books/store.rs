//! What Books remembers: which books are in the library, where the reader
//! stopped in each, its bookmarks, and the reading settings. One small
//! tab-separated file, written whenever it changes.

use super::engine;
use std::path::{Path, PathBuf};

/// Folders scanned for books.
/// Where books are looked for: the user's Books and Downloads under `$HOME`.
pub fn dirs() -> Vec<std::path::PathBuf> {
    let home = std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default());
    vec![home.join("Books"), home.join("Downloads")]
}
const MAX_DEPTH: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Theme {
    White,
    Sepia,
    Dark,
}

impl Theme {
    pub const ALL: [Theme; 3] = [Theme::White, Theme::Sepia, Theme::Dark];

    pub fn name(self) -> &'static str {
        match self {
            Theme::White => "White",
            Theme::Sepia => "Sepia",
            Theme::Dark => "Dark",
        }
    }

    fn id(self) -> u8 {
        self as u8
    }

    fn from_id(i: u8) -> Theme {
        Theme::ALL.get(i as usize).copied().unwrap_or(Theme::White)
    }
}

/// Body text sizes in points, smallest to largest.
pub const SIZES: [f32; 7] = [14.0, 16.0, 18.0, 20.0, 23.0, 27.0, 32.0];
pub const DEFAULT_SIZE: usize = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    pub size: usize,
    pub theme: Theme,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { size: DEFAULT_SIZE, theme: Theme::White }
    }
}

/// A place in a book: a block and a char offset inside it. Independent of
/// font size and page geometry, so it survives re-pagination.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Pos {
    pub block: usize,
    pub offset: usize,
}

/// A run of text the reader marked: `len` chars from `offset` inside `block`. Like `Pos`, it does not depend on the
/// font size, so a highlight survives re-pagination.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Span {
    pub block: usize,
    pub offset: usize,
    pub len: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub path: PathBuf,
    pub pos: Pos,
    pub bookmarks: Vec<Pos>,
    /// highlighted text, sorted by place
    pub highlights: Vec<Span>,
    /// order of last opening (higher = more recent); 0 = never opened
    pub opened: u64,
    /// how far through the book, in thousandths
    pub progress: u16,
}

#[derive(Debug, Default)]
pub struct Store {
    file: PathBuf,
    pub entries: Vec<Entry>,
    pub settings: Settings,
    clock: u64,
}

fn clean(path: &Path) -> String {
    path.to_string_lossy().replace(['\t', '\n'], " ")
}

impl Store {
    pub fn load(file: PathBuf) -> Store {
        let mut s = Store { file, ..Default::default() };
        let Ok(text) = std::fs::read_to_string(&s.file) else { return s };
        for line in text.lines() {
            let f: Vec<&str> = line.split('\t').collect();
            match f.as_slice() {
                ["S", size, theme] => {
                    s.settings = Settings { size: size.parse::<usize>().unwrap_or(DEFAULT_SIZE).min(SIZES.len() - 1), theme: Theme::from_id(theme.parse().unwrap_or(0)) };
                }
                ["B", path, block, offset, opened, marks, ..] => {
                    let opened = opened.parse().unwrap_or(0);
                    s.clock = s.clock.max(opened);
                    // a highlight line may come before its book line
                    let e = s.entry_mut(Path::new(path));
                    e.pos = Pos { block: block.parse().unwrap_or(0), offset: offset.parse().unwrap_or(0) };
                    e.bookmarks = marks.split(',').filter_map(parse_pos).collect();
                    e.opened = opened;
                    e.progress = f.get(6).and_then(|p| p.parse::<u16>().ok()).unwrap_or(0).min(1000);
                }
                ["H", path, block, offset, len] => {
                    let span = Span { block: block.parse().unwrap_or(0), offset: offset.parse().unwrap_or(0), len: len.parse().unwrap_or(0) };
                    if span.len > 0 {
                        let e = s.entry_mut(Path::new(path));
                        if !e.highlights.contains(&span) {
                            e.highlights.push(span);
                            e.highlights.sort();
                        }
                    }
                }
                _ => {}
            }
        }
        s
    }

    pub fn save(&self) {
        let mut out = format!("S\t{}\t{}\n", self.settings.size, self.settings.theme.id());
        for e in &self.entries {
            let marks: Vec<String> = e.bookmarks.iter().map(|p| format!("{}.{}", p.block, p.offset)).collect();
            out.push_str(&format!("B\t{}\t{}\t{}\t{}\t{}\t{}\n", clean(&e.path), e.pos.block, e.pos.offset, e.opened, marks.join(","), e.progress));
            for h in &e.highlights {
                out.push_str(&format!("H\t{}\t{}\t{}\t{}\n", clean(&e.path), h.block, h.offset, h.len));
            }
        }
        if let Some(dir) = self.file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = self.file.with_extension("tmp");
        if std::fs::write(&tmp, out).is_ok() {
            let _ = std::fs::rename(&tmp, &self.file);
        }
    }

    pub fn entry(&self, path: &Path) -> Option<&Entry> {
        self.entries.iter().find(|e| e.path == path)
    }

    /// The entry for `path`, created when it is new.
    pub fn entry_mut(&mut self, path: &Path) -> &mut Entry {
        if let Some(i) = self.entries.iter().position(|e| e.path == path) {
            return &mut self.entries[i];
        }
        self.entries.push(Entry { path: path.to_path_buf(), pos: Pos::default(), bookmarks: vec![], highlights: vec![], opened: 0, progress: 0 });
        self.entries.last_mut().expect("just pushed")
    }

    /// Marks `path` as just opened.
    pub fn touch(&mut self, path: &Path) {
        self.clock += 1;
        let c = self.clock;
        self.entry_mut(path).opened = c;
    }

    pub fn set_pos(&mut self, path: &Path, pos: Pos, progress: u16) {
        let e = self.entry_mut(path);
        e.pos = pos;
        e.progress = progress.min(1000);
    }

    /// Adds or removes a bookmark; true when it now exists.
    pub fn toggle_bookmark(&mut self, path: &Path, pos: Pos) -> bool {
        let e = self.entry_mut(path);
        if let Some(i) = e.bookmarks.iter().position(|&b| b == pos) {
            e.bookmarks.remove(i);
            false
        } else {
            e.bookmarks.push(pos);
            e.bookmarks.sort_by_key(|p| (p.block, p.offset));
            true
        }
    }

    /// Adds or removes a highlight; true when it now exists. An empty span is ignored (false).
    pub fn toggle_highlight(&mut self, path: &Path, span: Span) -> bool {
        if span.len == 0 {
            return false;
        }
        let e = self.entry_mut(path);
        if let Some(i) = e.highlights.iter().position(|&h| h == span) {
            e.highlights.remove(i);
            false
        } else {
            e.highlights.push(span);
            e.highlights.sort();
            true
        }
    }

    pub fn forget(&mut self, path: &Path) {
        self.entries.retain(|e| e.path != path);
    }
}

fn parse_pos(s: &str) -> Option<Pos> {
    let (b, o) = s.split_once('.')?;
    Some(Pos { block: b.parse().ok()?, offset: o.parse().ok()? })
}

/// Every book under `dirs`, plus the explicitly added `extra` paths that still
/// exist, most recently opened first, then by name.
pub fn library(store: &Store, dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = vec![];
    for d in dirs {
        walk(d, 0, &mut found);
    }
    for e in &store.entries {
        if e.path.is_file() && !found.contains(&e.path) {
            found.push(e.path.clone());
        }
    }
    let opened = |p: &PathBuf| store.entry(p).map_or(0, |e| e.opened);
    found.sort_by(|a, b| opened(b).cmp(&opened(a)).then_with(|| a.file_name().cmp(&b.file_name())));
    found
}

fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let path = e.path();
        if e.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        if path.is_dir() {
            if depth < MAX_DEPTH {
                walk(&path, depth + 1, out);
            }
        } else if engine::is_book(&path) && !out.contains(&path) {
            out.push(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-books-store")).join(format!("{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn state_round_trips() {
        let dir = scratch("roundtrip");
        let file = dir.join("state.tsv");
        let mut s = Store::load(file.clone());
        s.settings = Settings { size: 4, theme: Theme::Dark };
        s.touch(Path::new("/a/one.epub"));
        s.set_pos(Path::new("/a/one.epub"), Pos { block: 12, offset: 34 }, 250);
        assert!(s.toggle_bookmark(Path::new("/a/one.epub"), Pos { block: 9, offset: 0 }));
        assert!(s.toggle_bookmark(Path::new("/a/one.epub"), Pos { block: 2, offset: 5 }));
        s.touch(Path::new("/a/two.pdf"));
        s.save();
        let t = Store::load(file);
        assert_eq!(t.settings, Settings { size: 4, theme: Theme::Dark });
        let one = t.entry(Path::new("/a/one.epub")).unwrap();
        assert_eq!(one.pos, Pos { block: 12, offset: 34 });
        assert_eq!(one.progress, 250);
        assert_eq!(one.bookmarks, vec![Pos { block: 2, offset: 5 }, Pos { block: 9, offset: 0 }]);
        assert!(t.entry(Path::new("/a/two.pdf")).unwrap().opened > one.opened);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn bookmark_toggle_removes() {
        let mut s = Store::default();
        let p = Path::new("x.pdf");
        assert!(s.toggle_bookmark(p, Pos { block: 1, offset: 1 }));
        assert!(!s.toggle_bookmark(p, Pos { block: 1, offset: 1 }));
        assert!(s.entry(p).unwrap().bookmarks.is_empty());
    }

    #[test]
    fn library_scans_sorts_and_includes_added_files() {
        let dir = scratch("lib");
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::create_dir_all(dir.join(".hidden")).unwrap();
        for f in ["b.epub", "a.pdf", "notes.txt", "sub/c.PDF", ".hidden/d.pdf"] {
            std::fs::write(dir.join(f), b"x").unwrap();
        }
        let elsewhere = dir.join("e.pdf");
        std::fs::write(&elsewhere, b"x").unwrap();
        let mut s = Store::default();
        s.touch(&dir.join("sub/c.PDF"));
        s.entry_mut(&elsewhere);
        s.entry_mut(Path::new("/gone/f.pdf"));
        let names = |v: Vec<PathBuf>| v.iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect::<Vec<_>>();
        assert_eq!(names(library(&s, &[dir.clone()])), ["c.PDF", "a.pdf", "b.epub", "e.pdf"]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn bad_settings_fall_back() {
        let dir = scratch("bad");
        let file = dir.join("state.tsv");
        std::fs::write(&file, "S\t99\t7\nB\tp\tx\ty\tz\t\ngarbage\n").unwrap();
        let s = Store::load(file);
        assert_eq!(s.settings, Settings { size: SIZES.len() - 1, theme: Theme::White });
        assert_eq!(s.entries[0].pos, Pos::default());
        let _ = std::fs::remove_dir_all(dir);
    }
}
