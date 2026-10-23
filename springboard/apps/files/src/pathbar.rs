//! The path bar items: home or the computer, then each folder down to the open one.
use crate::*;

impl Files {
    /// The path bar: home or the computer, then each folder down to the open one (the stand-in lists are one item).
    pub(crate) fn path_items(&self) -> Vec<(String, &'static str, PathBuf)> {
        if let Some(tag) = tag_filter(&self.path) {
            return vec![(tag.map_or("Tags", |t| t.label()).into(), "tag", self.path.clone())];
        }
        if let Some(i) = smart::folder_index(&self.path) {
            return vec![(self.smart.get(i).map_or("Smart Folder", |f| f.name.as_str()).into(), "magnifyingglass", self.path.clone())];
        }
        if provider::is_path(&self.path) {
            return provider::shared().path_items(&self.path);
        }
        if applications::is_store_dir(&self.path) {
            return vec![("Applications".into(), "square.grid.2x2", self.path.clone())];
        }
        if self.in_recents() {
            return vec![("Recents".into(), "clock", self.path.clone())];
        }
        if self.in_bin() {
            return vec![("Trash".into(), "trash", self.bin())];
        }
        let (mut at, first, rest) = match self.path.strip_prefix(&self.home) {
            Ok(rest) => (self.home.clone(), (self.home.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or("Home".into()), "house"), rest.to_path_buf()),
            Err(_) => (PathBuf::from("/"), ("Computer".to_string(), "desktopcomputer"), self.path.strip_prefix("/").map(Path::to_path_buf).unwrap_or_default()),
        };
        let mut out = vec![(first.0, first.1, at.clone())];
        for c in rest.components() {
            at = at.join(c);
            out.push((c.as_os_str().to_string_lossy().into_owned(), "folder", at.clone()));
        }
        out
    }
}
