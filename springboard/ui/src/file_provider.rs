//! Provided items of a file provider domain (`provided item` parity): the identifier, the capability set, the
//! filename rules and the type and creator codes a provider reports for each item it manages. Pure model: nothing is
//! read from disk and no extension host runs. The domain registry and the provider errors live in
//! `bifrost_app::file_provider`.
//!
//! Mirrors `item identifier` (`rootContainer`, `workingSet`, `trashContainer`),
//! `item capabilities` (the `allows*` values and `allowsAll`), `type and creator`, the
//! identifier, parent and filename properties of `item protocol`, and its content properties `documentSize`,
//! `contentModificationDate`, `isDownloaded` and `isDownloading`. An item can be a placeholder: listed with its size
//! and dates, its bytes not on this machine, and downloaded on request (`allowsReading`) or dropped again
//! (`allowsEvicting`).
//!
//! Gaps: no content version or sync anchor, no upload state, no download error or progress, no tags, and the
//! capability bits are this crate's own layout, not the system's raw values.

use std::fmt;

/// `item identifier`: a persistent, non-empty name the provider gives an item.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ItemIdentifier(String);

impl ItemIdentifier {
    /// `rootContainer`: the root of the provider's hierarchy.
    pub fn root_container() -> Self {
        Self("root".into())
    }

    /// `workingSet`: the items the system should keep up to date.
    pub fn working_set() -> Self {
        Self("workingSet".into())
    }

    /// `trashContainer`: the parent of all trashed items.
    pub fn trash_container() -> Self {
        Self("trash".into())
    }

    /// Wraps an identifier the provider chose. `None` when it is empty.
    pub fn new(raw: &str) -> Option<Self> {
        (!raw.is_empty()).then(|| Self(raw.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ItemIdentifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// `item capabilities`: the actions the user may take on an item. Each flag is one bit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ItemCapabilities(pub u32);

impl ItemCapabilities {
    /// `allowsAddingSubItems`
    pub const ADDING_SUB_ITEMS: Self = Self(1 << 0);
    /// `allowsContentEnumerating`
    pub const CONTENT_ENUMERATING: Self = Self(1 << 1);
    /// `allowsDeleting`
    pub const DELETING: Self = Self(1 << 2);
    /// `allowsEvicting`: the system may drop the local copy.
    pub const EVICTING: Self = Self(1 << 3);
    /// `allowsReading`
    pub const READING: Self = Self(1 << 4);
    /// `allowsRenaming`
    pub const RENAMING: Self = Self(1 << 5);
    /// `allowsReparenting`: the item may be moved.
    pub const REPARENTING: Self = Self(1 << 6);
    /// `allowsTrashing`
    pub const TRASHING: Self = Self(1 << 7);
    /// `allowsWriting`
    pub const WRITING: Self = Self(1 << 8);
    /// `allowsExcludingFromSync`
    pub const EXCLUDING_FROM_SYNC: Self = Self(1 << 9);
    /// `allowsAll`: every flag above.
    pub const ALL: Self = Self(0x3ff);

    /// True when every bit of `other` is set here.
    pub fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Sets or clears `flag` and leaves the other flags as they are.
    pub fn with(self, flag: Self, enabled: bool) -> Self {
        if enabled {
            self.union(flag)
        } else {
            Self(self.0 & !flag.0)
        }
    }
}

/// `type and creator`: the four-character type and creator codes of an item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TypeAndCreator {
    pub file_type: [u8; 4],
    pub creator: [u8; 4],
}

impl TypeAndCreator {
    /// Both codes must be exactly four printable ASCII characters.
    pub fn new(file_type: &str, creator: &str) -> Option<Self> {
        Some(Self { file_type: four_cc(file_type)?, creator: four_cc(creator)? })
    }

    /// The type code packed big-endian, as the system stores an OSType.
    pub fn file_type_code(&self) -> u32 {
        u32::from_be_bytes(self.file_type)
    }
}

fn four_cc(code: &str) -> Option<[u8; 4]> {
    let bytes: [u8; 4] = code.as_bytes().try_into().ok()?;
    bytes.iter().all(|b| b.is_ascii_graphic()).then_some(bytes)
}

/// Why an item operation was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemError {
    /// The filename is empty, or is `.` or `..`.
    InvalidFilename,
    /// The filename holds a path separator or a NUL byte.
    ForbiddenCharacter,
    /// The item does not grant the capability the operation needs.
    NotAllowed(ItemCapabilities),
    /// The item is not in the state the operation needs, such as finishing a download that never started.
    InvalidState,
}

/// Whether the bytes of an item are on this machine: `isDownloaded` and `isDownloading`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocalContent {
    /// A placeholder: the item is listed with its size and dates, but its bytes are not on this machine.
    Placeholder,
    /// A transfer is running. The bytes are not on this machine yet.
    Downloading,
    /// The bytes are on this machine.
    Downloaded,
}

fn check_filename(name: &str) -> Result<(), ItemError> {
    if name.is_empty() || name == "." || name == ".." {
        return Err(ItemError::InvalidFilename);
    }
    if name.contains('/') || name.contains('\0') {
        return Err(ItemError::ForbiddenCharacter);
    }
    Ok(())
}

/// One item a provider manages: `item protocol`'s required properties and its capability set.
#[derive(Clone, Debug, PartialEq)]
pub struct ProvidedItem {
    pub identifier: ItemIdentifier,
    pub parent: ItemIdentifier,
    filename: String,
    /// A uniform type identifier, such as `public.rust-source`, when the provider knows it.
    pub content_type: Option<String>,
    pub type_and_creator: Option<TypeAndCreator>,
    /// `documentSize`: the size of the content in bytes, when the provider knows it.
    pub document_size: Option<u64>,
    /// `contentModificationDate`: Unix seconds of the last change to the content, when known.
    pub content_modification_date: Option<u64>,
    capabilities: ItemCapabilities,
    local: LocalContent,
}

impl ProvidedItem {
    /// A new item with no capabilities granted: the provider opts in to each action with `with_capabilities`.
    /// The bytes are on this machine until the item is marked as a placeholder.
    pub fn new(identifier: ItemIdentifier, parent: ItemIdentifier, filename: &str) -> Result<Self, ItemError> {
        check_filename(filename)?;
        Ok(Self {
            identifier,
            parent,
            filename: filename.into(),
            content_type: None,
            type_and_creator: None,
            document_size: None,
            content_modification_date: None,
            capabilities: ItemCapabilities::default(),
            local: LocalContent::Downloaded,
        })
    }

    /// Marks the item as a placeholder: it is listed, but its bytes are not on this machine yet.
    pub fn as_placeholder(mut self) -> Self {
        self.local = LocalContent::Placeholder;
        self
    }

    pub fn with_document_size(mut self, bytes: u64) -> Self {
        self.document_size = Some(bytes);
        self
    }

    pub fn local_content(&self) -> LocalContent {
        self.local
    }

    /// Starts the download of a placeholder. Needs `READING`. Asking again while the download runs, or for bytes
    /// already on this machine, changes nothing.
    pub fn request_download(&mut self) -> Result<(), ItemError> {
        if !self.can(ItemCapabilities::READING) {
            return Err(ItemError::NotAllowed(ItemCapabilities::READING));
        }
        if self.local == LocalContent::Placeholder {
            self.local = LocalContent::Downloading;
        }
        Ok(())
    }

    /// Records that the running download finished. Only a download that is running can finish.
    pub fn finish_download(&mut self) -> Result<(), ItemError> {
        if self.local != LocalContent::Downloading {
            return Err(ItemError::InvalidState);
        }
        self.local = LocalContent::Downloaded;
        Ok(())
    }

    /// Drops the bytes from this machine and keeps the item listed as a placeholder. Needs `EVICTING`. A placeholder
    /// stays as it is. A running download cannot be evicted.
    pub fn evict(&mut self) -> Result<(), ItemError> {
        if !self.can(ItemCapabilities::EVICTING) {
            return Err(ItemError::NotAllowed(ItemCapabilities::EVICTING));
        }
        match self.local {
            LocalContent::Downloaded => {
                self.local = LocalContent::Placeholder;
                Ok(())
            }
            LocalContent::Placeholder => Ok(()),
            LocalContent::Downloading => Err(ItemError::InvalidState),
        }
    }

    pub fn with_capabilities(mut self, capabilities: ItemCapabilities) -> Self {
        self.capabilities = capabilities;
        self
    }

    pub fn filename(&self) -> &str {
        &self.filename
    }

    pub fn capabilities(&self) -> ItemCapabilities {
        self.capabilities
    }

    /// True when the item grants every flag in `action`.
    pub fn can(&self, action: ItemCapabilities) -> bool {
        self.capabilities.contains(action)
    }

    /// Renames the item. Needs `RENAMING`; the name is checked after the capability.
    pub fn rename(&mut self, filename: &str) -> Result<(), ItemError> {
        if !self.can(ItemCapabilities::RENAMING) {
            return Err(ItemError::NotAllowed(ItemCapabilities::RENAMING));
        }
        check_filename(filename)?;
        self.filename = filename.into();
        Ok(())
    }

    /// Moves the item under another parent. Needs `REPARENTING`.
    pub fn reparent(&mut self, parent: ItemIdentifier) -> Result<(), ItemError> {
        if !self.can(ItemCapabilities::REPARENTING) {
            return Err(ItemError::NotAllowed(ItemCapabilities::REPARENTING));
        }
        self.parent = parent;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_reject_empty_and_keep_the_well_known_containers_apart() {
        assert!(ItemIdentifier::new("").is_none());
        assert_eq!(ItemIdentifier::new("doc-7").unwrap().as_str(), "doc-7");
        let roots = [ItemIdentifier::root_container(), ItemIdentifier::working_set(), ItemIdentifier::trash_container()];
        for (i, a) in roots.iter().enumerate() {
            for b in &roots[i + 1..] {
                assert_ne!(a, b);
            }
        }
    }

    #[test]
    fn capabilities_toggle_one_flag_without_touching_the_others() {
        let caps = ItemCapabilities::default()
            .with(ItemCapabilities::READING, true)
            .with(ItemCapabilities::WRITING, true);
        assert!(caps.contains(ItemCapabilities::READING));
        let caps = caps.with(ItemCapabilities::WRITING, false);
        assert!(caps.contains(ItemCapabilities::READING));
        assert!(!caps.contains(ItemCapabilities::WRITING));
        assert!(ItemCapabilities::ALL.contains(caps));
        assert!(!caps.contains(ItemCapabilities::ALL));
    }

    #[test]
    fn all_is_the_union_of_every_flag_and_each_flag_is_one_bit() {
        let each = [
            ItemCapabilities::ADDING_SUB_ITEMS,
            ItemCapabilities::CONTENT_ENUMERATING,
            ItemCapabilities::DELETING,
            ItemCapabilities::EVICTING,
            ItemCapabilities::READING,
            ItemCapabilities::RENAMING,
            ItemCapabilities::REPARENTING,
            ItemCapabilities::TRASHING,
            ItemCapabilities::WRITING,
            ItemCapabilities::EXCLUDING_FROM_SYNC,
        ];
        let union = each.iter().fold(ItemCapabilities::default(), |acc, f| acc.union(*f));
        assert_eq!(union, ItemCapabilities::ALL);
        for flag in each {
            assert_eq!(flag.0.count_ones(), 1);
        }
    }

    #[test]
    fn type_and_creator_takes_exactly_four_printable_codes() {
        assert!(TypeAndCreator::new("TXT", "ttxt").is_none());
        assert!(TypeAndCreator::new("TE T", "ttxt").is_none());
        let t = TypeAndCreator::new("TEXT", "ttxt").unwrap();
        assert_eq!(t.file_type_code(), 0x5445_5854);
        assert_eq!(t.creator, *b"ttxt");
    }

    #[test]
    fn new_item_rejects_names_that_cannot_be_a_filename() {
        let id = ItemIdentifier::new("a").unwrap();
        for bad in ["", ".", "..", "x/y", "a\0b"] {
            assert!(ProvidedItem::new(id.clone(), ItemIdentifier::root_container(), bad).is_err(), "{bad:?}");
        }
        let item = ProvidedItem::new(id, ItemIdentifier::root_container(), "notes.txt").unwrap();
        assert_eq!(item.filename(), "notes.txt");
        assert_eq!(item.capabilities(), ItemCapabilities::default());
    }

    #[test]
    fn rename_and_reparent_need_their_capability() {
        let id = ItemIdentifier::new("doc").unwrap();
        let mut item = ProvidedItem::new(id, ItemIdentifier::root_container(), "a.txt").unwrap();
        assert_eq!(item.rename("b.txt"), Err(ItemError::NotAllowed(ItemCapabilities::RENAMING)));
        item = item.with_capabilities(ItemCapabilities::RENAMING);
        item.rename("b.txt").unwrap();
        assert_eq!(item.filename(), "b.txt");
        assert_eq!(item.rename("c/d"), Err(ItemError::ForbiddenCharacter));
        assert_eq!(item.reparent(ItemIdentifier::working_set()), Err(ItemError::NotAllowed(ItemCapabilities::REPARENTING)));
        assert_eq!(item.parent, ItemIdentifier::root_container());
    }

    fn file(name: &str) -> ProvidedItem {
        ProvidedItem::new(ItemIdentifier::new("f").unwrap(), ItemIdentifier::root_container(), name).unwrap()
    }

    #[test]
    fn new_items_hold_their_bytes_until_marked_as_placeholders() {
        let item = file("a.txt");
        assert_eq!(item.local_content(), LocalContent::Downloaded);
        assert_eq!(item.document_size, None);
        let placeholder = file("a.txt").as_placeholder().with_document_size(4096);
        assert_eq!(placeholder.local_content(), LocalContent::Placeholder);
        assert_eq!(placeholder.document_size, Some(4096));
    }

    #[test]
    fn a_placeholder_downloads_only_with_the_reading_capability() {
        let mut item = file("a.txt").as_placeholder();
        assert_eq!(item.request_download(), Err(ItemError::NotAllowed(ItemCapabilities::READING)));
        assert_eq!(item.local_content(), LocalContent::Placeholder);
        item = item.with_capabilities(ItemCapabilities::READING);
        item.request_download().unwrap();
        assert_eq!(item.local_content(), LocalContent::Downloading);
        item.request_download().unwrap();
        assert_eq!(item.local_content(), LocalContent::Downloading, "a second request changes nothing");
        item.finish_download().unwrap();
        assert_eq!(item.local_content(), LocalContent::Downloaded);
    }

    #[test]
    fn only_a_running_download_can_finish() {
        let mut item = file("a.txt").as_placeholder();
        assert_eq!(item.finish_download(), Err(ItemError::InvalidState));
        assert_eq!(item.local_content(), LocalContent::Placeholder);
        let mut on_disk = file("b.txt");
        assert_eq!(on_disk.finish_download(), Err(ItemError::InvalidState));
    }

    #[test]
    fn eviction_keeps_the_item_listed_as_a_placeholder() {
        let mut item = file("a.txt").with_capabilities(ItemCapabilities::EVICTING).with_document_size(12);
        item.content_modification_date = Some(1_700_000_000);
        item.evict().unwrap();
        assert_eq!(item.local_content(), LocalContent::Placeholder);
        assert_eq!(item.filename(), "a.txt");
        assert_eq!(item.document_size, Some(12));
        assert_eq!(item.content_modification_date, Some(1_700_000_000));
        item.evict().unwrap();
        assert_eq!(item.local_content(), LocalContent::Placeholder, "evicting a placeholder changes nothing");
    }

    #[test]
    fn eviction_needs_the_capability_and_no_running_download() {
        let mut plain = file("a.txt");
        assert_eq!(plain.evict(), Err(ItemError::NotAllowed(ItemCapabilities::EVICTING)));
        assert_eq!(plain.local_content(), LocalContent::Downloaded);
        let mut busy = file("b.txt")
            .as_placeholder()
            .with_capabilities(ItemCapabilities::EVICTING.union(ItemCapabilities::READING));
        busy.request_download().unwrap();
        assert_eq!(busy.evict(), Err(ItemError::InvalidState));
        assert_eq!(busy.local_content(), LocalContent::Downloading);
    }
}
