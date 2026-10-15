//! Media groups: a tree of named groups, each with a group type, the media items it holds and its child groups. A group
//! of the media library is what the library shows as an album, an artist, a folder or a playlist folder, and the type
//! says which of those it is.
//!
//! A group type is an identifier chosen by the app: lower-case ASCII letters at the start, then letters, digits, dots and
//! hyphens, at most 64 bytes. Every group id is unique in the whole tree, so a lookup by id has one answer. A group keeps
//! each media item once, in the order it was added, and the items of a group and of its descendants are listed together
//! by [`MediaGroup::all_items`]. The model holds identifiers only; the media objects themselves stay with the library.
//! Gaps: no group ordering rules of the reference library, no smart groups that compute their items, and no persistence.

use std::fmt;

/// The longest group type identifier, in bytes.
pub const MAX_GROUP_TYPE_BYTES: usize = 64;

/// A group type identifier, such as `album` or `folder`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct GroupType(String);

impl GroupType {
    /// A group type from `identifier`, or `None` when it is empty, too long, or not lower-case ASCII with the allowed
    /// characters.
    pub fn new(identifier: &str) -> Option<Self> {
        let bytes = identifier.as_bytes();
        let first_ok = bytes.first().is_some_and(|b| b.is_ascii_lowercase());
        let rest_ok = bytes.iter().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'.' || *b == b'-');
        (first_ok && rest_ok && bytes.len() <= MAX_GROUP_TYPE_BYTES).then(|| GroupType(identifier.to_string()))
    }

    /// The identifier text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Why a group could not be added to a tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GroupError {
    /// A group with this id is already in the tree (the parent included).
    DuplicateId(String),
}

impl fmt::Display for GroupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GroupError::DuplicateId(id) => write!(f, "a group named {id} is already in the tree"),
        }
    }
}

/// One group of the media library: its id, type, display name, media item identifiers and child groups.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MediaGroup {
    id: String,
    kind: GroupType,
    name: String,
    items: Vec<String>,
    children: Vec<MediaGroup>,
}

impl MediaGroup {
    /// A group with no items and no children. The id and the name are trimmed, and `None` is returned when either is
    /// empty afterwards.
    pub fn new(id: &str, kind: GroupType, name: &str) -> Option<Self> {
        let id = id.trim();
        let name = name.trim();
        (!id.is_empty() && !name.is_empty()).then(|| MediaGroup {
            id: id.to_string(),
            kind,
            name: name.to_string(),
            items: Vec::new(),
            children: Vec::new(),
        })
    }

    /// The group id.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The group type.
    pub fn kind(&self) -> &GroupType {
        &self.kind
    }

    /// The display name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The media item identifiers this group holds directly, in the order they were added.
    pub fn items(&self) -> &[String] {
        &self.items
    }

    /// The child groups, in the order they were added.
    pub fn children(&self) -> &[MediaGroup] {
        &self.children
    }

    /// Adds a media item identifier. Returns `false` when the identifier is empty after trimming or the group already
    /// holds it.
    pub fn add_item(&mut self, item: &str) -> bool {
        let item = item.trim();
        if item.is_empty() || self.items.iter().any(|held| held == item) {
            return false;
        }
        self.items.push(item.to_string());
        true
    }

    /// Adds a child group at the end. Fails when the child's id, or the id of any group below it, is already in this
    /// tree.
    pub fn add_child(&mut self, child: MediaGroup) -> Result<(), GroupError> {
        if self.find(child.id()).is_some() || child.ids().iter().any(|id| self.find(id).is_some()) {
            return Err(GroupError::DuplicateId(child.id));
        }
        self.children.push(child);
        Ok(())
    }

    /// The group with `id`: this group or one of its descendants, searched depth first.
    pub fn find(&self, id: &str) -> Option<&MediaGroup> {
        if self.id == id {
            return Some(self);
        }
        self.children.iter().find_map(|child| child.find(id))
    }

    /// Removes the group with `id` and everything below it, and returns it. This group itself cannot be removed.
    pub fn remove(&mut self, id: &str) -> Option<MediaGroup> {
        if let Some(index) = self.children.iter().position(|child| child.id == id) {
            return Some(self.children.remove(index));
        }
        self.children.iter_mut().find_map(|child| child.remove(id))
    }

    /// The media item identifiers of this group and of all its descendants, each once, depth first.
    pub fn all_items(&self) -> Vec<&str> {
        let mut out = Vec::new();
        self.collect_items(&mut out);
        out
    }

    /// The display names from this group down to the group with `id`, both included, or `None` when no such group is
    /// in the tree.
    pub fn path_to(&self, id: &str) -> Option<Vec<&str>> {
        if self.id == id {
            return Some(vec![self.name.as_str()]);
        }
        self.children.iter().find_map(|child| {
            child.path_to(id).map(|mut path| {
                path.insert(0, self.name.as_str());
                path
            })
        })
    }

    /// The number of groups in this tree, this group included.
    pub fn count(&self) -> usize {
        1 + self.children.iter().map(MediaGroup::count).sum::<usize>()
    }

    fn collect_items<'a>(&'a self, out: &mut Vec<&'a str>) {
        for item in &self.items {
            if !out.contains(&item.as_str()) {
                out.push(item.as_str());
            }
        }
        for child in &self.children {
            child.collect_items(out);
        }
    }

    fn ids(&self) -> Vec<&str> {
        let mut ids = vec![self.id.as_str()];
        for child in &self.children {
            ids.extend(child.ids());
        }
        ids
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn group(id: &str, kind: &str, name: &str) -> MediaGroup {
        MediaGroup::new(id, GroupType::new(kind).unwrap(), name).unwrap()
    }

    #[test]
    fn a_group_type_is_lower_case_ascii_with_dots_and_hyphens() {
        assert_eq!(GroupType::new("album").unwrap().as_str(), "album");
        assert!(GroupType::new("smart-folder.2").is_some());
        assert!(GroupType::new("").is_none());
        assert!(GroupType::new("Album").is_none());
        assert!(GroupType::new("2fast").is_none());
        assert!(GroupType::new("a b").is_none());
        assert!(GroupType::new(&"a".repeat(MAX_GROUP_TYPE_BYTES)).is_some());
        assert!(GroupType::new(&"a".repeat(MAX_GROUP_TYPE_BYTES + 1)).is_none());
    }

    #[test]
    fn a_group_needs_a_non_empty_id_and_name() {
        assert!(MediaGroup::new("  ", GroupType::new("album").unwrap(), "Name").is_none());
        assert!(MediaGroup::new("id", GroupType::new("album").unwrap(), " ").is_none());
        let g = MediaGroup::new(" id-1 ", GroupType::new("album").unwrap(), " Blue ").unwrap();
        assert_eq!(g.id(), "id-1");
        assert_eq!(g.name(), "Blue");
        assert_eq!(g.kind().as_str(), "album");
        assert!(g.items().is_empty() && g.children().is_empty());
    }

    #[test]
    fn items_are_kept_once_in_the_order_they_were_added() {
        let mut g = group("a", "album", "A");
        assert!(g.add_item("song-2"));
        assert!(g.add_item(" song-1 "));
        assert!(!g.add_item("song-2"), "a second copy is refused");
        assert!(!g.add_item("   "));
        assert_eq!(g.items(), ["song-2".to_string(), "song-1".to_string()]);
    }

    #[test]
    fn children_are_found_by_id_at_any_depth_and_ids_stay_unique() {
        let mut root = group("root", "folder", "Library");
        let mut artist = group("artist", "artist", "Band");
        artist.add_child(group("album", "album", "First")).unwrap();
        root.add_child(artist).unwrap();
        assert_eq!(root.find("album").unwrap().name(), "First");
        assert_eq!(root.find("root").unwrap().name(), "Library");
        assert_eq!(root.count(), 3);
        assert_eq!(
            root.add_child(group("album", "album", "Copy")),
            Err(GroupError::DuplicateId("album".to_string()))
        );
        assert_eq!(root.add_child(group("root", "folder", "Again")), Err(GroupError::DuplicateId("root".to_string())));
        assert_eq!(root.count(), 3, "a refused child leaves the tree as it was");
    }

    #[test]
    fn a_child_whose_descendant_repeats_an_id_is_refused_as_a_whole() {
        let mut root = group("root", "folder", "Library");
        root.add_child(group("taken", "album", "Taken")).unwrap();
        let mut parent = group("parent", "artist", "Band");
        parent.add_child(group("taken", "album", "Other")).unwrap();
        assert_eq!(root.add_child(parent), Err(GroupError::DuplicateId("parent".to_string())));
        assert!(root.find("parent").is_none());
    }

    #[test]
    fn removing_a_group_takes_its_whole_subtree_and_never_the_root() {
        let mut root = group("root", "folder", "Library");
        let mut artist = group("artist", "artist", "Band");
        artist.add_child(group("album", "album", "First")).unwrap();
        root.add_child(artist).unwrap();
        let removed = root.remove("artist").unwrap();
        assert_eq!(removed.count(), 2);
        assert!(root.find("album").is_none());
        assert_eq!(root.count(), 1);
        assert!(root.remove("root").is_none(), "the tree cannot remove its own root");
        assert!(root.remove("missing").is_none());
    }

    #[test]
    fn all_items_lists_descendant_items_once_depth_first() {
        let mut root = group("root", "folder", "Library");
        root.add_item("s1");
        let mut artist = group("artist", "artist", "Band");
        artist.add_item("s2");
        artist.add_item("s1");
        root.add_child(artist).unwrap();
        let mut other = group("other", "album", "Other");
        other.add_item("s3");
        root.add_child(other).unwrap();
        assert_eq!(root.all_items(), vec!["s1", "s2", "s3"]);
    }

    #[test]
    fn the_path_runs_from_the_root_to_the_group_by_name() {
        let mut root = group("root", "folder", "Library");
        let mut artist = group("artist", "artist", "Band");
        artist.add_child(group("album", "album", "First")).unwrap();
        root.add_child(artist).unwrap();
        assert_eq!(root.path_to("album"), Some(vec!["Library", "Band", "First"]));
        assert_eq!(root.path_to("root"), Some(vec!["Library"]));
        assert_eq!(root.path_to("nowhere"), None);
    }

    #[test]
    fn group_errors_name_the_clashing_id() {
        let text = GroupError::DuplicateId("x".into()).to_string();
        assert_eq!(text, "a group named x is already in the tree");
    }
}
