//! Content items an external player publishes: a tree of containers and playable items. Each item has an identifier, a
//! title, a kind, flags for streaming and explicit content, and a playback progress. Containers hold children in the
//! order they were added; playable items are the ones that can be resolved for playback. Pure model: nothing is
//! published to the shell and no item plays.

use std::collections::BTreeMap;

/// Why a tree change or a playback request was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// An item with this identifier is already in the tree.
    Duplicate,
    /// The parent identifier is not in the tree.
    UnknownParent,
    /// Only containers hold children.
    NotAContainer,
    /// No item has this identifier.
    UnknownItem,
    /// Only playable items can be played.
    NotPlayable,
}

/// Whether an item holds other items or can be played.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Container,
    Playable,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ContentItem {
    identifier: String,
    title: String,
    kind: Kind,
    explicit: bool,
    streaming: bool,
    progress: f64,
}

impl ContentItem {
    pub fn new(identifier: &str, title: &str, kind: Kind) -> Self {
        Self { identifier: identifier.to_string(), title: title.to_string(), kind, explicit: false, streaming: false, progress: 0.0 }
    }

    pub fn explicit(mut self, explicit: bool) -> Self {
        self.explicit = explicit;
        self
    }

    pub fn streaming(mut self, streaming: bool) -> Self {
        self.streaming = streaming;
        self
    }

    /// Sets the playback progress as a fraction. Values outside 0 to 1 are clamped, and NaN counts as 0.
    pub fn progress(mut self, fraction: f64) -> Self {
        self.progress = clamp_fraction(fraction);
        self
    }

    pub fn identifier(&self) -> &str {
        &self.identifier
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn kind(&self) -> Kind {
        self.kind
    }

    pub fn is_explicit(&self) -> bool {
        self.explicit
    }

    pub fn is_streaming(&self) -> bool {
        self.streaming
    }

    pub fn playback_progress(&self) -> f64 {
        self.progress
    }
}

fn clamp_fraction(fraction: f64) -> f64 {
    if fraction.is_nan() {
        0.0
    } else {
        fraction.clamp(0.0, 1.0)
    }
}

/// The published tree: top-level items and the children of each container.
#[derive(Clone, Debug, Default)]
pub struct ContentTree {
    items: BTreeMap<String, ContentItem>,
    children: BTreeMap<String, Vec<String>>,
    roots: Vec<String>,
}

impl ContentTree {
    /// Adds `item` at the top level when `parent` is `None`, otherwise under the container `parent`.
    pub fn add(&mut self, parent: Option<&str>, item: ContentItem) -> Result<(), Error> {
        if self.items.contains_key(item.identifier()) {
            return Err(Error::Duplicate);
        }
        match parent {
            None => self.roots.push(item.identifier.clone()),
            Some(parent) => {
                let holder = self.items.get(parent).ok_or(Error::UnknownParent)?;
                if holder.kind != Kind::Container {
                    return Err(Error::NotAContainer);
                }
                self.children.entry(parent.to_string()).or_default().push(item.identifier.clone());
            }
        }
        self.items.insert(item.identifier.clone(), item);
        Ok(())
    }

    /// Sets the playback progress of an item that is in the tree.
    pub fn set_progress(&mut self, identifier: &str, fraction: f64) -> Result<(), Error> {
        let item = self.items.get_mut(identifier).ok_or(Error::UnknownItem)?;
        item.progress = clamp_fraction(fraction);
        Ok(())
    }

    pub fn item(&self, identifier: &str) -> Option<&ContentItem> {
        self.items.get(identifier)
    }

    /// The top-level items, in the order they were added.
    pub fn roots(&self) -> Vec<&ContentItem> {
        self.roots.iter().filter_map(|id| self.items.get(id)).collect()
    }

    /// The children of a container, in the order they were added. Empty for playable and unknown identifiers.
    pub fn children(&self, identifier: &str) -> Vec<&ContentItem> {
        self.children.get(identifier).map(|ids| ids.iter().filter_map(|id| self.items.get(id)).collect()).unwrap_or_default()
    }

    /// Resolves an item for playback. Containers and unknown identifiers are refused.
    pub fn resolve_playback(&self, identifier: &str) -> Result<&ContentItem, Error> {
        let item = self.items.get(identifier).ok_or(Error::UnknownItem)?;
        match item.kind {
            Kind::Playable => Ok(item),
            Kind::Container => Err(Error::NotPlayable),
        }
    }
}
