//! Search indexing of intent entities (`IndexedEntity`, hidden-from-search state).
//!
//! An app lists its entities (notes, folders, people) with [`IndexedEntity`] and calls [`index`]: each visible entity
//! becomes a local search item in the domain `entity.<type>`, so Search finds it without launching the app. An entity
//! marked with [`IndexedEntity::hide_in_spotlight`] is never indexed, and is removed when an earlier index call had
//! added it. The item's identifier is `<type>/<id>`; opening the result follows the same path as any Core Search item
//! (see [`crate::quicksearch`]).

use crate::intents::Entity;
use crate::quicksearch::{CSSearchableIndex, CSSearchableItem, CSSearchableItemAttributeSet, IndexError};
use crate::Ctx;
use bifrost_core::json::Json;

/// An entity offered to Search, with the keywords it is found by.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexedEntity {
    pub entity: Entity,
    /// Extra words the entity is found by, besides its title and subtitle.
    pub keywords: Vec<String>,
    /// The entity is not included in local search results.
    pub hidden: bool,
}

impl IndexedEntity {
    pub fn new(entity: Entity) -> IndexedEntity {
        IndexedEntity { entity, keywords: Vec::new(), hidden: false }
    }
    pub fn keyword(mut self, word: &str) -> IndexedEntity {
        self.keywords.push(word.into());
        self
    }
    pub fn hide_in_spotlight(mut self) -> IndexedEntity {
        self.hidden = true;
        self
    }
    /// The Core Search domain of this entity's type, used to remove a whole type at once.
    pub fn domain(&self) -> String {
        domain_of(&self.entity.ty)
    }
    /// The Core Search identifier: `<type>/<id>`.
    pub fn unique_identifier(&self) -> String {
        format!("{}/{}", self.entity.ty, self.entity.id)
    }
    /// The wire form for an `entity.query` reply: the entity's fields plus `hidden`. The shell's entity index
    /// (`shell::entity_index`) reads `hidden` and leaves such an entity out of Search.
    pub fn to_json(&self) -> Json {
        let mut fields = match self.entity.to_json() {
            Json::Obj(fields) => fields,
            other => return other,
        };
        fields.push(("hidden".into(), Json::Bool(self.hidden)));
        Json::Obj(fields)
    }
    /// The Core Search item for this entity, or `None` when it is hidden.
    pub fn searchable_item(&self) -> Option<CSSearchableItem> {
        if self.hidden {
            return None;
        }
        let words: Vec<&str> = self.keywords.iter().map(String::as_str).collect();
        let mut attributes = CSSearchableItemAttributeSet::new(&self.entity.title).keywords(&words);
        if let Some(subtitle) = &self.entity.subtitle {
            attributes = attributes.content_description(subtitle);
        }
        Some(CSSearchableItem::new(&self.unique_identifier(), &self.domain(), attributes))
    }
}

/// The Core Search domain for entities of type `ty`.
pub fn domain_of(ty: &str) -> String {
    format!("entity.{ty}")
}

/// Indexes the visible entities in `entities` and removes the hidden ones. Returns the items the index refused, with
/// their position in the visible list.
pub fn index(ctx: &Ctx, entities: &[IndexedEntity]) -> Vec<(usize, IndexError)> {
    let search = CSSearchableIndex::default_index();
    let hidden: Vec<String> = entities.iter().filter(|e| e.hidden).map(IndexedEntity::unique_identifier).collect();
    if !hidden.is_empty() {
        let ids: Vec<&str> = hidden.iter().map(String::as_str).collect();
        search.delete_searchable_items(ctx, &ids);
    }
    let items: Vec<CSSearchableItem> = entities.iter().filter_map(IndexedEntity::searchable_item).collect();
    search.index_checked(ctx, &items)
}

/// Removes every indexed entity of type `ty`.
pub fn remove_type(ctx: &Ctx, ty: &str) {
    let domain = domain_of(ty);
    CSSearchableIndex::default_index().delete_searchable_items_with_domain_identifiers(ctx, &[domain.as_str()]);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(id: &str) -> Entity {
        Entity::new("Note", id, "Shopping list").subtitle("Milk, bread")
    }

    #[test]
    fn visible_entity_becomes_an_item_in_its_type_domain() {
        let item = IndexedEntity::new(note("n1")).keyword("groceries").searchable_item().expect("visible");
        assert_eq!(item.unique_identifier, "Note/n1");
        assert_eq!(item.domain_identifier, "entity.Note");
        assert_eq!(item.attribute_set.title, "Shopping list");
        assert_eq!(item.attribute_set.content_description, "Milk, bread");
        assert_eq!(item.attribute_set.keywords, vec!["groceries".to_string()]);
    }

    #[test]
    fn hidden_entity_has_no_item() {
        let hidden = IndexedEntity::new(note("n2")).hide_in_spotlight();
        assert!(hidden.hidden);
        assert!(hidden.searchable_item().is_none());
    }

    #[test]
    fn the_wire_form_carries_the_hidden_flag() {
        let shown = IndexedEntity::new(note("n1")).to_json();
        assert_eq!(shown.get("id"), Some(&Json::Str("n1".into())));
        assert_eq!(shown.get("hidden"), Some(&Json::Bool(false)));
        let hidden = IndexedEntity::new(note("n2")).hide_in_spotlight().to_json();
        assert_eq!(hidden.get("hidden"), Some(&Json::Bool(true)));
        assert_eq!(hidden.get("title"), Some(&Json::Str("Shopping list".into())));
    }

    #[test]
    fn entity_without_subtitle_leaves_the_description_empty() {
        let plain = IndexedEntity::new(Entity::new("Folder", "f1", "Work"));
        let item = plain.searchable_item().expect("visible");
        assert!(item.attribute_set.content_description.is_empty());
        assert_eq!(item.domain_identifier, domain_of("Folder"));
    }
}
