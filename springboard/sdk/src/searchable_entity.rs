//! Searchable entities: an app value that it describes once and the system indexes as a searchable item. The entity
//! gives its identifier, the domain it is grouped under, its attribute set and an optional expiry. `searchable_item`
//! builds the item that the index stores, and `IndexError::check` validates it. Indexing and removal stay with
//! `CSSearchableIndex`, so this module needs no index connection and no device.

use crate::quicksearch::{CSSearchableItem, CSSearchableItemAttributeSet};

/// An app value that the Spotlight index can hold as a searchable item.
pub trait SearchableEntity {
    /// The stable identifier of the entity. Indexing an entity whose identifier is already indexed replaces it.
    fn entity_identifier(&self) -> String;

    /// The domain the entity is grouped under, so that a whole group can be removed at once.
    fn domain_identifier(&self) -> String;

    /// The attributes that make the entity searchable and that a result shows.
    fn attribute_set(&self) -> CSSearchableItemAttributeSet;

    /// Seconds since the Unix epoch after which the index drops the entity. None keeps it until it is removed.
    fn expires_at(&self) -> Option<i64> {
        None
    }

    /// The searchable item the index stores for this entity.
    fn searchable_item(&self) -> CSSearchableItem {
        let item = CSSearchableItem::new(&self.entity_identifier(), &self.domain_identifier(), self.attribute_set());
        match self.expires_at() {
            Some(secs) => item.expiration_date(secs),
            None => item,
        }
    }
}

/// The searchable items for a batch of entities, in the order given.
pub fn searchable_items<E: SearchableEntity>(entities: &[E]) -> Vec<CSSearchableItem> {
    entities.iter().map(SearchableEntity::searchable_item).collect()
}
