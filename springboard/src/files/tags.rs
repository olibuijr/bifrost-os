//! Colour tags, persisted in a sidecar index file at the storage root
//! (`.FilesTags`): one line per tagged path, "mask<TAB>path", with tabs,
//! newlines and backslashes in paths escaped.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

/// The sidecar file name under a storage root (hidden from listings).
pub const SIDECAR: &str = ".FilesTags";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tag {
    Red,
    Orange,
    Yellow,
    Green,
    Blue,
    Purple,
    Gray,
}

impl Tag {
    pub const ALL: [Tag; 7] = [Tag::Red, Tag::Orange, Tag::Yellow, Tag::Green, Tag::Blue, Tag::Purple, Tag::Gray];

    pub fn label(self) -> &'static str {
        match self {
            Tag::Red => "Red",
            Tag::Orange => "Orange",
            Tag::Yellow => "Yellow",
            Tag::Green => "Green",
            Tag::Blue => "Blue",
            Tag::Purple => "Purple",
            Tag::Gray => "Gray",
        }
    }

    /// bifrost tag colours as 0xRRGGBB.
    pub fn rgb(self) -> u32 {
        match self {
            Tag::Red => 0xFF3B30,
            Tag::Orange => 0xFF9500,
            Tag::Yellow => 0xFFCC00,
            Tag::Green => 0x34C759,
            Tag::Blue => 0x007AFF,
            Tag::Purple => 0xAF52DE,
            Tag::Gray => 0x8E8E93,
        }
    }

    fn bit(self) -> u8 {
        1 << (self as u8)
    }
}

pub fn sidecar_path(root: &Path) -> PathBuf {
    root.join(SIDECAR)
}

fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\t', "\\t").replace('\n', "\\n")
}

fn unescape(s: &str) -> String {
    let mut out = String::new();
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.next() {
                Some('t') => out.push('\t'),
                Some('n') => out.push('\n'),
                Some(o) => out.push(o),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TagIndex {
    map: BTreeMap<PathBuf, u8>,
}

impl TagIndex {
    pub fn parse(text: &str) -> TagIndex {
        let mut map = BTreeMap::new();
        for line in text.lines() {
            let Some((mask, path)) = line.split_once('\t') else { continue };
            if let Ok(m) = mask.parse::<u8>() {
                if m & 0x7F != 0 && !path.is_empty() {
                    map.insert(PathBuf::from(unescape(path)), m & 0x7F);
                }
            }
        }
        TagIndex { map }
    }

    pub fn serialize(&self) -> String {
        self.map.iter().map(|(p, m)| format!("{m}\t{}\n", escape(&p.to_string_lossy()))).collect()
    }

    /// Loads the sidecar; a missing or unreadable file is an empty index.
    pub fn load(file: &Path) -> TagIndex {
        std::fs::read_to_string(file).map(|t| TagIndex::parse(&t)).unwrap_or_default()
    }

    /// Writes through a temp file so a crash never leaves half an index.
    pub fn save(&self, file: &Path) -> io::Result<()> {
        let tmp = file.with_extension("tmp");
        std::fs::write(&tmp, self.serialize())?;
        std::fs::rename(tmp, file)
    }

    pub fn tags(&self, path: &Path) -> Vec<Tag> {
        let m = self.map.get(path).copied().unwrap_or(0);
        Tag::ALL.into_iter().filter(|t| m & t.bit() != 0).collect()
    }

    pub fn has(&self, path: &Path, tag: Tag) -> bool {
        self.map.get(path).is_some_and(|m| m & tag.bit() != 0)
    }

    /// Adds the tag when absent, removes it when present; returns whether it is now set.
    pub fn toggle(&mut self, path: &Path, tag: Tag) -> bool {
        let m = self.map.entry(path.to_path_buf()).or_insert(0);
        *m ^= tag.bit();
        let on = *m & tag.bit() != 0;
        if *m == 0 {
            self.map.remove(path);
        }
        on
    }

    pub fn clear(&mut self, path: &Path) {
        self.map.remove(path);
    }

    /// Paths carrying `tag`.
    pub fn with_tag(&self, tag: Tag) -> Vec<PathBuf> {
        self.map.iter().filter(|(_, m)| *m & tag.bit() != 0).map(|(p, _)| p.clone()).collect()
    }

    /// Follows a rename or move: `from` and everything under it now live under `to`.
    pub fn moved(&mut self, from: &Path, to: &Path) {
        let hits: Vec<PathBuf> = self.map.keys().filter(|p| p.starts_with(from)).cloned().collect();
        for old in hits {
            let m = self.map.remove(&old).unwrap();
            let new = to.join(old.strip_prefix(from).unwrap());
            self.map.insert(new, m);
        }
    }

    /// Copies tags onto a duplicate of `from` at `to`.
    pub fn copied(&mut self, from: &Path, to: &Path) {
        let hits: Vec<(PathBuf, u8)> = self.map.iter().filter(|(p, _)| p.starts_with(from)).map(|(p, m)| (p.clone(), *m)).collect();
        for (old, m) in hits {
            self.map.insert(to.join(old.strip_prefix(from).unwrap()), m);
        }
    }

    /// Forgets `path` and everything under it (after a delete).
    pub fn removed(&mut self, path: &Path) {
        self.map.retain(|p, _| !p.starts_with(path));
    }

    /// Drops entries whose file no longer exists; returns how many.
    pub fn prune(&mut self) -> usize {
        let before = self.map.len();
        self.map.retain(|p, _| p.symlink_metadata().is_ok());
        before - self.map.len()
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggle_sets_and_clears() {
        let mut t = TagIndex::default();
        let p = Path::new("/a/b.txt");
        assert!(t.toggle(p, Tag::Red));
        assert!(t.toggle(p, Tag::Blue));
        assert_eq!(t.tags(p), [Tag::Red, Tag::Blue]);
        assert!(!t.toggle(p, Tag::Red));
        assert!(t.has(p, Tag::Blue) && !t.has(p, Tag::Red));
        assert!(!t.toggle(p, Tag::Blue));
        assert!(t.is_empty());
    }

    #[test]
    fn serialises_with_awkward_paths() {
        let mut t = TagIndex::default();
        t.toggle(Path::new("/a/tab\there"), Tag::Green);
        t.toggle(Path::new("/a/new\nline\\slash"), Tag::Gray);
        t.toggle(Path::new("/plain"), Tag::Purple);
        t.toggle(Path::new("/plain"), Tag::Orange);
        let back = TagIndex::parse(&t.serialize());
        assert_eq!(back, t);
        assert_eq!(back.tags(Path::new("/plain")), [Tag::Orange, Tag::Purple]);
        assert_eq!(TagIndex::parse("garbage\n0\t/zero\nx\t/y\n3\t").len(), 0);
    }

    #[test]
    fn rename_move_copy_remove_follow_trees() {
        let mut t = TagIndex::default();
        t.toggle(Path::new("/r/dir"), Tag::Red);
        t.toggle(Path::new("/r/dir/f.txt"), Tag::Blue);
        t.toggle(Path::new("/r/dirx"), Tag::Green);
        t.moved(Path::new("/r/dir"), Path::new("/r/new"));
        assert!(t.has(Path::new("/r/new"), Tag::Red) && t.has(Path::new("/r/new/f.txt"), Tag::Blue));
        assert!(t.has(Path::new("/r/dirx"), Tag::Green));
        assert!(!t.has(Path::new("/r/dir"), Tag::Red));
        t.copied(Path::new("/r/new"), Path::new("/r/copy"));
        assert!(t.has(Path::new("/r/copy/f.txt"), Tag::Blue) && t.has(Path::new("/r/new/f.txt"), Tag::Blue));
        t.removed(Path::new("/r/new"));
        assert_eq!(t.with_tag(Tag::Blue), [PathBuf::from("/r/copy/f.txt")]);
    }

    #[test]
    fn persists_in_the_sidecar_and_prunes() {
        let d = super::super::fs::tests::temp("tags-persist");
        let file = sidecar_path(&d);
        assert!(TagIndex::load(&file).is_empty());
        std::fs::write(d.join("real.txt"), "x").unwrap();
        let mut t = TagIndex::default();
        t.toggle(&d.join("real.txt"), Tag::Yellow);
        t.toggle(&d.join("gone.txt"), Tag::Red);
        t.save(&file).unwrap();
        let mut back = TagIndex::load(&file);
        assert_eq!(back, t);
        assert_eq!(back.prune(), 1);
        assert_eq!(back.len(), 1);
    }
}
