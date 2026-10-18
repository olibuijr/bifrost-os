//! Searchable entities: an app value that it describes once and the system indexes as a searchable item. The entity
//! gives its identifier, the domain it is grouped under, its attribute set and an optional expiry. `searchable_item`
//! builds the item that the index stores, and `IndexError::check` validates it. Indexing and removal stay with the
//! index type in `quicksearch`, so this module needs no index connection and no device.

use crate::quicksearch::{CSSearchableItem, CSSearchableItemAttributeSet};

/// An app value that the search index can hold as a searchable item.
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

    /// Whether the entity is kept out of search results (`hideInSpotlight`). It stays indexed.
    fn hide_in_spotlight(&self) -> bool {
        false
    }

    /// The searchable item the index stores for this entity.
    fn searchable_item(&self) -> CSSearchableItem {
        let item = CSSearchableItem::new(&self.entity_identifier(), &self.domain_identifier(), self.attribute_set());
        let item = match self.expires_at() {
            Some(secs) => item.expiration_date(secs),
            None => item,
        };
        if self.hide_in_spotlight() {
            item.hide_in_spotlight()
        } else {
            item
        }
    }
}

/// The searchable items for a batch of entities, in the order given.
pub fn searchable_items<E: SearchableEntity>(entities: &[E]) -> Vec<CSSearchableItem> {
    entities.iter().map(SearchableEntity::searchable_item).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quicksearch::CSSearchableItemAttributeSet;

    struct Note {
        id: &'static str,
        hidden: bool,
        expires: Option<i64>,
    }

    impl SearchableEntity for Note {
        fn entity_identifier(&self) -> String {
            self.id.to_string()
        }

        fn domain_identifier(&self) -> String {
            "notes".to_string()
        }

        fn attribute_set(&self) -> CSSearchableItemAttributeSet {
            CSSearchableItemAttributeSet { title: self.id.to_string(), ..Default::default() }
        }

        fn expires_at(&self) -> Option<i64> {
            self.expires
        }

        fn hide_in_spotlight(&self) -> bool {
            self.hidden
        }
    }

    #[test]
    fn an_entity_is_visible_in_search_unless_it_asks_to_be_hidden() {
        let shown = Note { id: "a", hidden: false, expires: None }.searchable_item();
        assert!(!shown.hide_in_spotlight);
        let hidden = Note { id: "b", hidden: true, expires: None }.searchable_item();
        assert!(hidden.hide_in_spotlight);
        assert_eq!(hidden.unique_identifier, "b", "a hidden entity is still indexed under its identifier");
    }

    #[test]
    fn hiding_and_expiry_combine_and_batches_keep_order() {
        let item = Note { id: "c", hidden: true, expires: Some(1_700_000_000) }.searchable_item();
        assert!(item.hide_in_spotlight);
        assert_eq!(item.expiration_date, Some(1_700_000_000));
        let batch = searchable_items(&[Note { id: "d", hidden: false, expires: None }, Note { id: "e", hidden: true, expires: None }]);
        assert_eq!(batch.iter().map(|i| i.hide_in_spotlight).collect::<Vec<_>>(), vec![false, true]);
    }
}
