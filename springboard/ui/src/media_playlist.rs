//! Playlists as a pure model: a named, ordered list of media item identifiers with its description and author. It
//! mirrors the playlist of the media player framework (name, description text, author display name) and its creation
//! metadata. As in that framework, an app adds items to the end and has no other way to edit a playlist. Duplicate
//! items are kept in the order they were added. Gaps: no persistent or cloud identifiers, no playlist attributes, no
//! seed items for generated playlists, and no storage; the Music app keeps its own `.m3u` files.

/// Creation metadata for a playlist: the name is required, the description and the author are optional.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct PlaylistMetadata {
    pub name: String,
    pub description: Option<String>,
    pub author: Option<String>,
}

/// The cases `Playlist::new` and `Playlist::add` refuse.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlaylistError {
    /// The name is empty after trimming.
    EmptyName,
    /// An item identifier is empty after trimming.
    EmptyItem,
}

/// An ordered list of item identifiers with the playlist's name, description and author.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Playlist {
    name: String,
    description: Option<String>,
    author: Option<String>,
    items: Vec<String>,
}

/// Trims an optional text and drops it when nothing is left.
fn non_empty(text: Option<String>) -> Option<String> {
    text.map(|t| t.trim().to_string()).filter(|t| !t.is_empty())
}

impl Playlist {
    /// An empty playlist from its metadata. The name is trimmed and must not be empty. A blank description or author
    /// is stored as None.
    pub fn new(metadata: PlaylistMetadata) -> Result<Self, PlaylistError> {
        let name = metadata.name.trim();
        if name.is_empty() {
            return Err(PlaylistError::EmptyName);
        }
        Ok(Self {
            name: name.to_string(),
            description: non_empty(metadata.description),
            author: non_empty(metadata.author),
            items: Vec::new(),
        })
    }

    /// The playlist name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The description text, when one was given.
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    /// The author display name, when one was given.
    pub fn author(&self) -> Option<&str> {
        self.author.as_deref()
    }

    /// The item identifiers in playlist order.
    pub fn items(&self) -> &[String] {
        &self.items
    }

    /// The number of items, duplicates included.
    pub fn count(&self) -> usize {
        self.items.len()
    }

    /// Whether an item with this identifier is in the playlist.
    pub fn contains(&self, id: &str) -> bool {
        let id = id.trim();
        self.items.iter().any(|item| item == id)
    }

    /// Appends one item. The identifier is trimmed and must not be empty.
    pub fn add(&mut self, id: &str) -> Result<(), PlaylistError> {
        let id = id.trim();
        if id.is_empty() {
            return Err(PlaylistError::EmptyItem);
        }
        self.items.push(id.to_string());
        Ok(())
    }

    /// Appends several items in one step. Every identifier is checked first, so a bad one leaves the playlist
    /// unchanged. Returns the number of items appended.
    pub fn add_all(&mut self, ids: &[&str]) -> Result<usize, PlaylistError> {
        let trimmed: Vec<&str> = ids.iter().map(|id| id.trim()).collect();
        if trimmed.iter().any(|id| id.is_empty()) {
            return Err(PlaylistError::EmptyItem);
        }
        self.items.extend(trimmed.iter().map(|id| id.to_string()));
        Ok(trimmed.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(name: &str) -> PlaylistMetadata {
        PlaylistMetadata { name: name.to_string(), ..PlaylistMetadata::default() }
    }

    #[test]
    fn name_is_trimmed_and_a_blank_name_is_refused() {
        let playlist = Playlist::new(meta("  Morning  ")).expect("named");
        assert_eq!(playlist.name(), "Morning");
        assert_eq!(Playlist::new(meta("   ")), Err(PlaylistError::EmptyName));
        assert_eq!(Playlist::new(meta("")), Err(PlaylistError::EmptyName));
    }

    #[test]
    fn blank_description_and_author_are_stored_as_none() {
        let playlist = Playlist::new(PlaylistMetadata {
            name: "Road".into(),
            description: Some("   ".into()),
            author: Some(" Ola ".into()),
        })
        .expect("named");
        assert_eq!(playlist.description(), None);
        assert_eq!(playlist.author(), Some("Ola"));
    }

    #[test]
    fn add_appends_in_order_and_keeps_duplicates() {
        let mut playlist = Playlist::new(meta("Mix")).expect("named");
        playlist.add("a").expect("item");
        playlist.add("b").expect("item");
        playlist.add("a").expect("item");
        assert_eq!(playlist.count(), 3);
        assert_eq!(playlist.items(), &["a", "b", "a"]);
        assert!(playlist.contains("b"));
        assert!(!playlist.contains("c"));
    }

    #[test]
    fn empty_item_is_refused_and_not_added() {
        let mut playlist = Playlist::new(meta("Mix")).expect("named");
        assert_eq!(playlist.add("  "), Err(PlaylistError::EmptyItem));
        assert_eq!(playlist.count(), 0);
    }

    #[test]
    fn add_all_is_atomic_and_reports_how_many_were_added() {
        let mut playlist = Playlist::new(meta("Mix")).expect("named");
        // The blank identifier in the middle refuses the whole batch.
        assert_eq!(playlist.add_all(&["x", " ", "y"]), Err(PlaylistError::EmptyItem));
        assert_eq!(playlist.count(), 0);
        assert_eq!(playlist.add_all(&[" x ", "y"]), Ok(2));
        assert_eq!(playlist.items(), &["x", "y"]);
    }
}
