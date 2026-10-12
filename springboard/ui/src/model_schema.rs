//! Attribute options for the fields of a model entity: unique, indexed and transient.
//! The schema records which fields carry each option. `model_store` enforces uniqueness and refuses fields the schema
//! does not declare for an entity that has a schema. Transient fields stay in memory and are left out of what a
//! backend persists. Nothing here stores data.

use std::collections::BTreeMap;

/// The options of one field. A field with no options is a plain stored field.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Attribute {
    /// No two records of the entity may hold the same value for this field.
    pub unique: bool,
    /// The field is meant for lookups by value.
    pub indexed: bool,
    /// The field lives in memory only and is not persisted.
    pub transient: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SchemaError {
    /// The entity has a schema and this field is not declared on it.
    UnknownField { entity: String, field: String },
}

/// The declared fields of each entity. An entity that was never declared accepts any field.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Schema {
    entities: BTreeMap<String, BTreeMap<String, Attribute>>,
}

impl Schema {
    pub fn new() -> Self {
        Schema::default()
    }

    /// Declares a field of an entity with its options. Declaring the same field again replaces its options.
    pub fn declare(&mut self, entity: &str, field: &str, attribute: Attribute) {
        self.entities.entry(entity.to_string()).or_default().insert(field.to_string(), attribute);
    }

    /// The options of a declared field.
    pub fn attribute(&self, entity: &str, field: &str) -> Option<Attribute> {
        self.entities.get(entity)?.get(field).copied()
    }

    /// The names of the unique fields of an entity, in name order.
    pub fn unique_fields(&self, entity: &str) -> Vec<&str> {
        self.fields_where(entity, |attribute| attribute.unique)
    }

    /// The names of the indexed fields of an entity, in name order.
    pub fn indexed_fields(&self, entity: &str) -> Vec<&str> {
        self.fields_where(entity, |attribute| attribute.indexed)
    }

    fn fields_where(&self, entity: &str, keep: impl Fn(&Attribute) -> bool) -> Vec<&str> {
        match self.entities.get(entity) {
            Some(fields) => fields.iter().filter(|(_, attribute)| keep(*attribute)).map(|(name, _)| name.as_str()).collect(),
            None => Vec::new(),
        }
    }

    /// Checks that every field of a write is declared, when the entity has a schema.
    pub fn validate(&self, entity: &str, fields: &BTreeMap<String, String>) -> Result<(), SchemaError> {
        let Some(declared) = self.entities.get(entity) else {
            return Ok(());
        };
        match fields.keys().find(|name| !declared.contains_key(name.as_str())) {
            Some(field) => Err(SchemaError::UnknownField { entity: entity.to_string(), field: field.clone() }),
            None => Ok(()),
        }
    }

    /// The fields a backend persists: every field except the transient ones.
    pub fn persistent_fields(&self, entity: &str, fields: &BTreeMap<String, String>) -> BTreeMap<String, String> {
        let transient = |name: &str| self.attribute(entity, name).is_some_and(|attribute| attribute.transient);
        fields.iter().filter(|(name, _)| !transient(name.as_str())).map(|(name, value)| (name.clone(), value.clone())).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(name, value)| (name.to_string(), value.to_string())).collect()
    }

    #[test]
    fn an_undeclared_entity_accepts_any_field() {
        let schema = Schema::new();
        assert_eq!(schema.validate("Note", &fields(&[("anything", "1")])), Ok(()));
        assert_eq!(schema.persistent_fields("Note", &fields(&[("a", "1")])), fields(&[("a", "1")]));
        assert!(schema.unique_fields("Note").is_empty());
    }

    #[test]
    fn a_declared_entity_refuses_an_undeclared_field() {
        let mut schema = Schema::new();
        schema.declare("Contact", "email", Attribute { unique: true, ..Attribute::default() });
        assert_eq!(schema.validate("Contact", &fields(&[("email", "a@example.test")])), Ok(()));
        assert_eq!(
            schema.validate("Contact", &fields(&[("email", "a@example.test"), ("nick", "ada")])),
            Err(SchemaError::UnknownField { entity: "Contact".into(), field: "nick".into() })
        );
    }

    #[test]
    fn transient_fields_are_left_out_of_the_persisted_fields() {
        let mut schema = Schema::new();
        schema.declare("Note", "title", Attribute::default());
        schema.declare("Note", "scratch", Attribute { transient: true, ..Attribute::default() });
        let all = fields(&[("title", "one"), ("scratch", "draft")]);
        assert_eq!(schema.persistent_fields("Note", &all), fields(&[("title", "one")]));
        assert_eq!(all.len(), 2, "the input is not changed");
    }

    #[test]
    fn unique_and_indexed_fields_are_listed_in_name_order() {
        let mut schema = Schema::new();
        schema.declare("Track", "title", Attribute { indexed: true, ..Attribute::default() });
        schema.declare("Track", "isrc", Attribute { unique: true, indexed: true, transient: false });
        schema.declare("Track", "artist", Attribute { indexed: true, ..Attribute::default() });
        schema.declare("Track", "rating", Attribute::default());
        assert_eq!(schema.unique_fields("Track"), vec!["isrc"]);
        assert_eq!(schema.indexed_fields("Track"), vec!["artist", "isrc", "title"]);
    }

    #[test]
    fn declaring_a_field_again_replaces_its_options() {
        let mut schema = Schema::new();
        schema.declare("Note", "title", Attribute { unique: true, ..Attribute::default() });
        schema.declare("Note", "title", Attribute { indexed: true, ..Attribute::default() });
        assert_eq!(schema.attribute("Note", "title"), Some(Attribute { unique: false, indexed: true, transient: false }));
        assert_eq!(schema.attribute("Note", "body"), None);
        assert!(schema.unique_fields("Note").is_empty());
    }
}
