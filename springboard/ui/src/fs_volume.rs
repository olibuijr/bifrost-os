//! Volumes and items (`volume and item` parity): an in-memory model of one volume's directory tree. A volume has an
//! identifier, a name and a root directory. Items are files, directories, symbolic links and special nodes; each
//! item has one parent and a name that is unique within its parent. Pure model: nothing is read from or written to
//! disk, and no mount happens.
//!
//! Mirrors the volume identifier and name, the item identifier, parent, name, type and size properties, the item
//! type enumeration, and the filename rules (a non-empty name of at most 255 bytes with no separator and no NUL,
//! and not `.` or `..`).
//!
//! Gaps: no attributes beyond size and type (no times, owners or permissions), no extended attributes, no
//! open, read or write handlers, no rename, no mount state and no block resources. Names are compared as UTF-8 text,
//! without Unicode normalization.

use std::collections::BTreeMap;
use std::fmt;

/// The longest file name, in bytes, that the model accepts.
pub const MAX_NAME_BYTES: usize = 255;

/// The identifier of the root directory of every volume.
pub const ROOT_ITEM: ItemId = 1;

/// The identifier of an item. Ids are unique within one volume and increase with creation order.
pub type ItemId = u64;

/// The identifier of a volume: a non-empty name the module chose.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct VolumeIdentifier(String);

impl VolumeIdentifier {
    /// Wraps an identifier. `None` when it is empty.
    pub fn new(raw: &str) -> Option<Self> {
        (!raw.is_empty()).then(|| Self(raw.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The name of one file or directory. Valid names are non-empty, at most `MAX_NAME_BYTES` bytes, contain no `/`
/// and no NUL byte, and are neither `.` nor `..`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FileName(String);

impl FileName {
    pub fn new(raw: &str) -> Result<Self, VolumeError> {
        if raw.is_empty() {
            return Err(VolumeError::EmptyName);
        }
        if raw.len() > MAX_NAME_BYTES {
            return Err(VolumeError::NameTooLong);
        }
        if raw == "." || raw == ".." || raw.contains('/') || raw.contains('\0') {
            return Err(VolumeError::InvalidName);
        }
        Ok(Self(raw.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }
}

/// The kind of an item in the file hierarchy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemType {
    File,
    Directory,
    Symlink,
    Socket,
    Fifo,
    CharacterDevice,
    BlockDevice,
}

impl ItemType {
    pub fn is_directory(self) -> bool {
        self == ItemType::Directory
    }
}

/// One item of a volume. The root directory has no parent and no name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub id: ItemId,
    pub parent: Option<ItemId>,
    pub name: Option<FileName>,
    pub kind: ItemType,
    /// Size in bytes. Always 0 for a directory.
    pub size: u64,
}

/// Why a volume operation was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VolumeError {
    EmptyName,
    NameTooLong,
    InvalidName,
    /// The parent item does not exist, or the id names no item.
    NoSuchItem,
    /// The parent is not a directory.
    NotDirectory,
    /// The parent already has an item with this name.
    Exists,
    /// The directory still has children.
    NotEmpty,
    /// The root directory cannot be removed.
    RootItem,
}

impl fmt::Display for VolumeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            VolumeError::EmptyName => "the name is empty",
            VolumeError::NameTooLong => "the name is longer than 255 bytes",
            VolumeError::InvalidName => "the name contains a separator or NUL, or is . or ..",
            VolumeError::NoSuchItem => "no such item",
            VolumeError::NotDirectory => "the parent is not a directory",
            VolumeError::Exists => "an item with this name already exists",
            VolumeError::NotEmpty => "the directory is not empty",
            VolumeError::RootItem => "the root directory cannot be removed",
        };
        f.write_str(text)
    }
}

impl std::error::Error for VolumeError {}

/// One volume: its identity and its item tree.
#[derive(Clone, Debug)]
pub struct Volume {
    id: VolumeIdentifier,
    name: String,
    items: BTreeMap<ItemId, Item>,
    next_id: ItemId,
}

impl Volume {
    /// Creates a volume holding only its root directory. Fails when the identifier or the name is empty.
    pub fn new(id: &str, name: &str) -> Result<Self, VolumeError> {
        let id = VolumeIdentifier::new(id).ok_or(VolumeError::EmptyName)?;
        if name.is_empty() {
            return Err(VolumeError::EmptyName);
        }
        let mut items = BTreeMap::new();
        items.insert(
            ROOT_ITEM,
            Item {
                id: ROOT_ITEM,
                parent: None,
                name: None,
                kind: ItemType::Directory,
                size: 0,
            },
        );
        Ok(Self {
            id,
            name: name.to_string(),
            items,
            next_id: ROOT_ITEM + 1,
        })
    }

    pub fn identifier(&self) -> &VolumeIdentifier {
        &self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn item(&self, id: ItemId) -> Option<&Item> {
        self.items.get(&id)
    }

    /// Number of items, the root included.
    pub fn item_count(&self) -> usize {
        self.items.len()
    }

    /// Adds an item under `parent` and returns its id. A directory's size is forced to 0.
    pub fn create(
        &mut self,
        parent: ItemId,
        name: &str,
        kind: ItemType,
        size: u64,
    ) -> Result<ItemId, VolumeError> {
        let name = FileName::new(name)?;
        let parent_item = self.items.get(&parent).ok_or(VolumeError::NoSuchItem)?;
        if !parent_item.kind.is_directory() {
            return Err(VolumeError::NotDirectory);
        }
        if self.lookup(parent, name.as_str()).is_some() {
            return Err(VolumeError::Exists);
        }
        let size = if kind.is_directory() { 0 } else { size };
        let id = self.next_id;
        self.next_id += 1;
        self.items.insert(
            id,
            Item {
                id,
                parent: Some(parent),
                name: Some(name),
                kind,
                size,
            },
        );
        Ok(id)
    }

    /// The id of the child of `parent` called `name`, if there is one.
    pub fn lookup(&self, parent: ItemId, name: &str) -> Option<ItemId> {
        self.items
            .values()
            .find(|i| i.parent == Some(parent) && i.name.as_ref().map(|n| n.as_str()) == Some(name))
            .map(|i| i.id)
    }

    /// The children of `parent`, in id order.
    pub fn children(&self, parent: ItemId) -> Vec<&Item> {
        self.items.values().filter(|i| i.parent == Some(parent)).collect()
    }

    /// The absolute path of an item, such as `/docs/readme.txt`. `None` when the id names no item.
    pub fn path_of(&self, id: ItemId) -> Option<String> {
        let mut parts: Vec<&str> = Vec::new();
        let mut current = id;
        loop {
            let item = self.items.get(&current)?;
            match (item.parent, &item.name) {
                (None, _) => break,
                (Some(parent), Some(name)) => {
                    parts.push(name.as_str());
                    current = parent;
                }
                (Some(_), None) => return None,
            }
        }
        parts.reverse();
        Some(format!("/{}", parts.join("/")))
    }

    /// Removes an item. A directory must be empty, and the root cannot be removed.
    pub fn remove(&mut self, id: ItemId) -> Result<(), VolumeError> {
        if id == ROOT_ITEM {
            return Err(VolumeError::RootItem);
        }
        if !self.items.contains_key(&id) {
            return Err(VolumeError::NoSuchItem);
        }
        if !self.children(id).is_empty() {
            return Err(VolumeError::NotEmpty);
        }
        self.items.remove(&id);
        Ok(())
    }

    /// Sum of the sizes of all items.
    pub fn used_bytes(&self) -> u64 {
        self.items.values().map(|i| i.size).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_name_rules() {
        assert_eq!(FileName::new(""), Err(VolumeError::EmptyName));
        assert_eq!(FileName::new("a/b"), Err(VolumeError::InvalidName));
        assert_eq!(FileName::new(".."), Err(VolumeError::InvalidName));
        assert_eq!(FileName::new("a\0b"), Err(VolumeError::InvalidName));
        assert_eq!(FileName::new(&"a".repeat(256)), Err(VolumeError::NameTooLong));
        assert_eq!(FileName::new(&"a".repeat(255)).unwrap().as_bytes().len(), 255);
        // Dots that are part of a longer name are fine.
        assert!(FileName::new("..hidden").is_ok());
    }

    #[test]
    fn new_volume_has_only_the_root() {
        let v = Volume::new("vol-1", "Data").unwrap();
        assert_eq!(v.item_count(), 1);
        assert_eq!(v.path_of(ROOT_ITEM).as_deref(), Some("/"));
        assert_eq!(v.name(), "Data");
        assert_eq!(Volume::new("", "Data").unwrap_err(), VolumeError::EmptyName);
        assert_eq!(Volume::new("vol-1", "").unwrap_err(), VolumeError::EmptyName);
    }

    #[test]
    fn nested_items_have_paths_and_sizes() {
        let mut v = Volume::new("vol-1", "Data").unwrap();
        let docs = v.create(ROOT_ITEM, "docs", ItemType::Directory, 99).unwrap();
        let readme = v.create(docs, "readme.txt", ItemType::File, 120).unwrap();
        // ids follow creation order: root is 1, so docs is 2 and readme is 3.
        assert_eq!(docs, 2);
        assert_eq!(readme, 3);
        assert_eq!(v.path_of(readme).as_deref(), Some("/docs/readme.txt"));
        // A directory's size is forced to 0.
        assert_eq!(v.item(docs).unwrap().size, 0);
        assert_eq!(v.used_bytes(), 120);
        assert_eq!(v.lookup(docs, "readme.txt"), Some(readme));
        assert_eq!(v.children(ROOT_ITEM).len(), 1);
    }

    #[test]
    fn names_are_unique_per_directory() {
        let mut v = Volume::new("vol-1", "Data").unwrap();
        let a = v.create(ROOT_ITEM, "a", ItemType::Directory, 0).unwrap();
        let b = v.create(ROOT_ITEM, "b", ItemType::Directory, 0).unwrap();
        v.create(a, "same", ItemType::File, 1).unwrap();
        // The same name under another directory is fine; under the same one it is not.
        assert!(v.create(b, "same", ItemType::File, 2).is_ok());
        assert_eq!(v.create(a, "same", ItemType::File, 3), Err(VolumeError::Exists));
    }

    #[test]
    fn parents_must_be_directories() {
        let mut v = Volume::new("vol-1", "Data").unwrap();
        let file = v.create(ROOT_ITEM, "f", ItemType::File, 5).unwrap();
        assert_eq!(
            v.create(file, "child", ItemType::File, 1),
            Err(VolumeError::NotDirectory)
        );
        assert_eq!(
            v.create(42, "child", ItemType::File, 1),
            Err(VolumeError::NoSuchItem)
        );
    }

    #[test]
    fn remove_refuses_non_empty_directories_and_the_root() {
        let mut v = Volume::new("vol-1", "Data").unwrap();
        let dir = v.create(ROOT_ITEM, "dir", ItemType::Directory, 0).unwrap();
        let child = v.create(dir, "x", ItemType::Symlink, 0).unwrap();
        assert_eq!(v.remove(dir), Err(VolumeError::NotEmpty));
        assert_eq!(v.remove(ROOT_ITEM), Err(VolumeError::RootItem));
        v.remove(child).unwrap();
        v.remove(dir).unwrap();
        assert_eq!(v.item_count(), 1);
        assert_eq!(v.remove(dir), Err(VolumeError::NoSuchItem));
    }

    #[test]
    fn errors_have_text() {
        assert_eq!(VolumeError::Exists.to_string(), "an item with this name already exists");
    }
}
