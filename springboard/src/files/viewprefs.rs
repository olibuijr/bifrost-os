//! Per-folder view options: list, icons or columns, the sort key and its
//! direction, remembered between visits in a small text file.

use super::fs::{self, Entry, Sort};
use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewMode {
    List,
    Icons,
    Columns,
}

impl ViewMode {
    pub const ALL: [ViewMode; 3] = [ViewMode::List, ViewMode::Icons, ViewMode::Columns];

    pub fn label(self) -> &'static str {
        match self {
            ViewMode::List => "as List",
            ViewMode::Icons => "as Icons",
            ViewMode::Columns => "as Columns",
        }
    }

    fn key(self) -> &'static str {
        match self {
            ViewMode::List => "list",
            ViewMode::Icons => "icons",
            ViewMode::Columns => "columns",
        }
    }

    fn from_key(k: &str) -> Option<ViewMode> {
        ViewMode::ALL.into_iter().find(|m| m.key() == k)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FolderView {
    pub mode: ViewMode,
    pub sort: Sort,
    pub descending: bool,
}

impl Default for FolderView {
    fn default() -> Self {
        FolderView { mode: ViewMode::List, sort: Sort::Name, descending: false }
    }
}

/// Sorts with the folder's key, reversed when descending.
pub fn sort_entries(entries: &mut [Entry], view: FolderView) {
    fs::sort(entries, view.sort);
    if view.descending {
        entries.reverse();
    }
}

/// Columns the icon and column layouts fit in `width_pt` points.
pub fn grid_columns(mode: ViewMode, width_pt: f32) -> usize {
    let cell = match mode {
        ViewMode::List => return 1,
        ViewMode::Icons => 120.0,
        ViewMode::Columns => 300.0,
    };
    ((width_pt / cell).floor() as usize).max(1)
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ViewPrefs {
    map: BTreeMap<PathBuf, FolderView>,
}

fn sort_key(s: Sort) -> &'static str {
    match s {
        Sort::Name => "name",
        Sort::Date => "date",
        Sort::Size => "size",
        Sort::Kind => "kind",
    }
}

impl ViewPrefs {
    /// The view for `dir`, defaulting to a name-sorted list.
    pub fn get(&self, dir: &Path) -> FolderView {
        self.map.get(dir).copied().unwrap_or_default()
    }

    /// Remembers `view` for `dir` (the default is not stored).
    pub fn set(&mut self, dir: &Path, view: FolderView) {
        if view == FolderView::default() {
            self.map.remove(dir);
        } else {
            self.map.insert(dir.to_path_buf(), view);
        }
    }

    pub fn serialize(&self) -> String {
        self.map.iter().map(|(p, v)| format!("{}\t{}\t{}\t{}\n", v.mode.key(), sort_key(v.sort), v.descending as u8, p.to_string_lossy().replace('\n', " "))).collect()
    }

    pub fn parse(text: &str) -> ViewPrefs {
        let mut map = BTreeMap::new();
        for line in text.lines() {
            let f: Vec<&str> = line.splitn(4, '\t').collect();
            let [mode, sort, desc, path] = f[..] else { continue };
            let (Some(mode), Some(sort)) = (ViewMode::from_key(mode), Sort::ALL.into_iter().find(|s| sort_key(*s) == sort)) else { continue };
            if !path.is_empty() {
                map.insert(PathBuf::from(path), FolderView { mode, sort, descending: desc == "1" });
            }
        }
        ViewPrefs { map }
    }

    pub fn load(file: &Path) -> ViewPrefs {
        std::fs::read_to_string(file).map(|t| ViewPrefs::parse(&t)).unwrap_or_default()
    }

    pub fn save(&self, file: &Path) -> io::Result<()> {
        let tmp = file.with_extension("tmp");
        std::fs::write(&tmp, self.serialize())?;
        std::fs::rename(tmp, file)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(name: &str, size: u64) -> Entry {
        Entry { kind: fs::kind_of(name, false), name: name.into(), path: PathBuf::from(name), is_dir: false, size, mtime: 0, items: None }
    }

    #[test]
    fn defaults_and_remembering() {
        let mut p = ViewPrefs::default();
        let d = Path::new("/media/Photos");
        assert_eq!(p.get(d), FolderView::default());
        let v = FolderView { mode: ViewMode::Icons, sort: Sort::Date, descending: true };
        p.set(d, v);
        assert_eq!(p.get(d), v);
        assert_eq!(p.get(Path::new("/media/Other")), FolderView::default());
        p.set(d, FolderView::default());
        assert_eq!(p, ViewPrefs::default());
    }

    #[test]
    fn round_trips_through_text_and_file() {
        let mut p = ViewPrefs::default();
        p.set(Path::new("/a b/c"), FolderView { mode: ViewMode::Columns, sort: Sort::Size, descending: false });
        p.set(Path::new("/z"), FolderView { mode: ViewMode::List, sort: Sort::Date, descending: true });
        assert_eq!(ViewPrefs::parse(&p.serialize()), p);
        assert_eq!(ViewPrefs::parse("bad\nlist\tname\t0\t").map.len(), 0);
        let f = fs::tests::temp("viewprefs").join("v");
        p.save(&f).unwrap();
        assert_eq!(ViewPrefs::load(&f), p);
        assert_eq!(ViewPrefs::load(Path::new("/nonexistent/v")), ViewPrefs::default());
    }

    #[test]
    fn sorting_direction() {
        let mut v = vec![e("b", 1), e("a", 5), e("c", 3)];
        sort_entries(&mut v, FolderView::default());
        assert_eq!(v.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["a", "b", "c"]);
        sort_entries(&mut v, FolderView { descending: true, ..Default::default() });
        assert_eq!(v.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["c", "b", "a"]);
        sort_entries(&mut v, FolderView { sort: Sort::Size, ..Default::default() });
        assert_eq!(v.iter().map(|e| e.size).collect::<Vec<_>>(), [5, 3, 1]);
    }

    #[test]
    fn grid_follows_width() {
        assert_eq!(grid_columns(ViewMode::List, 1000.0), 1);
        assert_eq!(grid_columns(ViewMode::Icons, 1000.0), 8);
        assert_eq!(grid_columns(ViewMode::Icons, 50.0), 1);
        assert_eq!(grid_columns(ViewMode::Columns, 1000.0), 3);
    }
}
