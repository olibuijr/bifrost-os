//! Object graph and persistent store: a model of entities with attributes and relationships,
//! managed objects that live in a context, and a container that keeps the saved objects and reads and
//! writes them as text.
//!
//! The model is checked when it is built: every relationship names an existing destination entity, and an
//! inverse must exist on the destination and point back. Edges are kept on both ends. Linking or unlinking
//! one side updates the inverse, and a to-one relationship replaces its old target.
//!
//! A [`Context`] works on its own copy of the store. [`Context::save`] writes the copy back only when the
//! container has not changed since the context read it (optimistic concurrency). A stale context gets
//! [`Error::Conflict`] and has to roll back and redo its work. [`Container::to_text`] and
//! [`Container::open`] write and read the whole store, so an app can keep it in one file.
//!
//! Not here: a SQLite-backed store, faulting (every object is loaded at once), undo in a context, and
//! per-property merge policies.

use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

/// The type an attribute holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttributeType {
    Text,
    Integer,
    Boolean,
}

/// An attribute value. `Null` means no value; it is allowed for any attribute type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Text(String),
}

impl Value {
    fn fits(&self, kind: AttributeType) -> bool {
        matches!(
            (self, kind),
            (Value::Null, _)
                | (Value::Bool(_), AttributeType::Boolean)
                | (Value::Int(_), AttributeType::Integer)
                | (Value::Text(_), AttributeType::Text)
        )
    }
}

/// Attribute description: name, type and whether a value is required.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attribute {
    pub name: String,
    pub kind: AttributeType,
    /// A required attribute must hold a value when a context is saved.
    pub optional: bool,
}

/// Relationship description: an edge to `destination`, with an optional inverse on that entity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Relationship {
    pub name: String,
    pub destination: String,
    pub inverse: Option<String>,
    /// False for a to-one relationship (at most one target), true for to-many.
    pub to_many: bool,
}

/// Entity description: a named record type with its properties.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entity {
    pub name: String,
    pub attributes: Vec<Attribute>,
    pub relationships: Vec<Relationship>,
}

/// The checked set of entities (the object model).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Model {
    entities: Vec<Entity>,
}

impl Model {
    /// Builds a model and checks it: unique entity names, unique property names per entity, existing
    /// destinations, and inverses that exist and point back.
    pub fn new(entities: Vec<Entity>) -> Result<Model, Error> {
        let model = Model { entities };
        model.check()?;
        Ok(model)
    }

    pub fn entity(&self, name: &str) -> Option<&Entity> {
        self.entities.iter().find(|e| e.name == name)
    }

    fn check(&self) -> Result<(), Error> {
        for (i, e) in self.entities.iter().enumerate() {
            if self.entities[..i].iter().any(|o| o.name == e.name) {
                return Err(Error::Model(format!("entity {} is declared twice", e.name)));
            }
            // Attributes and relationships share one namespace of property names.
            let mut names = BTreeSet::new();
            let properties = e
                .attributes
                .iter()
                .map(|a| a.name.as_str())
                .chain(e.relationships.iter().map(|r| r.name.as_str()));
            for name in properties {
                if !names.insert(name) {
                    return Err(Error::Model(format!("{} has two properties named {}", e.name, name)));
                }
            }
            for r in &e.relationships {
                let dest = self.entity(&r.destination).ok_or_else(|| {
                    Error::Model(format!("{}.{} points to unknown entity {}", e.name, r.name, r.destination))
                })?;
                if let Some(inv) = &r.inverse {
                    let back = dest
                        .relationships
                        .iter()
                        .find(|x| &x.name == inv)
                        .ok_or_else(|| Error::Model(format!("{}.{} is not declared", dest.name, inv)))?;
                    if back.destination != e.name || back.inverse.as_deref() != Some(r.name.as_str()) {
                        return Err(Error::Model(format!(
                            "inverse {}.{} does not point back to {}.{}",
                            dest.name, inv, e.name, r.name
                        )));
                    }
                }
            }
        }
        Ok(())
    }
}

/// The identifier of a managed object, unique within a store.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ObjectId(pub u64);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// The model failed its check; the text says which declaration.
    Model(String),
    UnknownEntity(String),
    UnknownObject(ObjectId),
    /// The entity has no attribute or relationship with this name.
    UnknownProperty { entity: String, name: String },
    /// The value does not fit the attribute's type.
    Type { attribute: String },
    /// A required attribute has no value at save time. The text is `Entity.attribute`.
    Required { attribute: String },
    /// The target of a relationship is not of the destination entity.
    WrongDestination { relationship: String, expected: String },
    /// The store changed after the context read it. `base` is the context's version, `store` the current one.
    Conflict { base: u64, store: u64 },
    /// A line of the text store could not be read (1-based).
    Format { line: usize },
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Object {
    entity: String,
    attributes: BTreeMap<String, Value>,
    relationships: BTreeMap<String, Vec<ObjectId>>,
}

/// The saved objects with their version. Each save makes a new version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Store {
    version: u64,
    next_id: u64,
    objects: BTreeMap<ObjectId, Object>,
}

impl Store {
    fn empty() -> Store {
        Store { version: 0, next_id: 1, objects: BTreeMap::new() }
    }

    pub fn version(&self) -> u64 {
        self.version
    }

    fn to_text(&self) -> String {
        let mut out = String::from("springboard-store-v1\n");
        out.push_str(&format!("version {}\nnext {}\n", self.version, self.next_id));
        for (id, o) in &self.objects {
            out.push_str(&format!("object {} {}\n", id.0, esc(&o.entity)));
            for (name, value) in &o.attributes {
                out.push_str(&format!("attribute {} {} {}\n", id.0, esc(name), encode_value(value)));
            }
            for (name, targets) in &o.relationships {
                if targets.is_empty() {
                    continue;
                }
                let ids: Vec<String> = targets.iter().map(|t| t.0.to_string()).collect();
                out.push_str(&format!("relationship {} {} {}\n", id.0, esc(name), ids.join(" ")));
            }
        }
        out
    }

    fn from_text(text: &str) -> Result<Store, Error> {
        let mut lines = text.lines();
        if lines.next() != Some("springboard-store-v1") {
            return Err(Error::Format { line: 1 });
        }
        let mut store = Store::empty();
        for (n, line) in lines.enumerate() {
            let line_no = n + 2;
            let bad = move || Error::Format { line: line_no };
            let parts: Vec<&str> = line.split(' ').collect();
            match parts.as_slice() {
                ["version", v] => store.version = v.parse().map_err(|_| bad())?,
                ["next", v] => store.next_id = v.parse().map_err(|_| bad())?,
                ["object", id, entity] => {
                    let id = parse_id(id).ok_or_else(bad)?;
                    let entity = unesc(entity).ok_or_else(bad)?;
                    let object = Object { entity, attributes: BTreeMap::new(), relationships: BTreeMap::new() };
                    store.objects.insert(id, object);
                }
                ["attribute", id, name, value] => {
                    let id = parse_id(id).ok_or_else(bad)?;
                    let name = unesc(name).ok_or_else(bad)?;
                    let value = decode_value(value).ok_or_else(bad)?;
                    let object = store.objects.get_mut(&id).ok_or_else(bad)?;
                    object.attributes.insert(name, value);
                }
                ["relationship", id, name, targets @ ..] => {
                    let id = parse_id(id).ok_or_else(bad)?;
                    let name = unesc(name).ok_or_else(bad)?;
                    let mut list = Vec::new();
                    for t in targets {
                        list.push(parse_id(t).ok_or_else(bad)?);
                    }
                    let object = store.objects.get_mut(&id).ok_or_else(bad)?;
                    object.relationships.insert(name, list);
                }
                _ => return Err(bad()),
            }
        }
        Ok(store)
    }
}

fn parse_id(s: &str) -> Option<ObjectId> {
    s.parse::<u64>().ok().map(ObjectId)
}

/// Percent-encodes everything except ASCII letters, digits and `_ - .`, so a token never holds a space.
fn esc(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b == b'.' {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

fn unesc(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = s.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn encode_value(v: &Value) -> String {
    match v {
        Value::Null => "n".to_string(),
        Value::Bool(true) => "b1".to_string(),
        Value::Bool(false) => "b0".to_string(),
        Value::Int(i) => format!("i{}", i),
        Value::Text(t) => format!("t{}", esc(t)),
    }
}

fn decode_value(s: &str) -> Option<Value> {
    let mut chars = s.chars();
    match chars.next()? {
        'n' if chars.as_str().is_empty() => Some(Value::Null),
        'b' => match chars.as_str() {
            "1" => Some(Value::Bool(true)),
            "0" => Some(Value::Bool(false)),
            _ => None,
        },
        'i' => chars.as_str().parse::<i64>().ok().map(Value::Int),
        't' => unesc(chars.as_str()).map(Value::Text),
        _ => None,
    }
}

/// Persistent container: a model and its store.
pub struct Container {
    model: Rc<Model>,
    store: Store,
}

impl Container {
    /// A container with an empty store.
    pub fn new(model: Model) -> Container {
        Container { model: Rc::new(model), store: Store::empty() }
    }

    /// Opens a store written by [`Container::to_text`]. Every object must name an entity of `model`.
    pub fn open(model: Model, text: &str) -> Result<Container, Error> {
        let store = Store::from_text(text)?;
        for o in store.objects.values() {
            if model.entity(&o.entity).is_none() {
                return Err(Error::UnknownEntity(o.entity.clone()));
            }
        }
        Ok(Container { model: Rc::new(model), store })
    }

    /// The whole store as text, for a file.
    pub fn to_text(&self) -> String {
        self.store.to_text()
    }

    pub fn version(&self) -> u64 {
        self.store.version
    }

    pub fn object_count(&self) -> usize {
        self.store.objects.len()
    }

    /// A new context that starts from the store as it is now.
    pub fn new_context(&self) -> Context {
        Context {
            model: self.model.clone(),
            base: self.store.clone(),
            work: self.store.clone(),
        }
    }
}

/// What a context would write on save, compared with the store it read.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Changes {
    pub inserted: Vec<ObjectId>,
    pub updated: Vec<ObjectId>,
    pub deleted: Vec<ObjectId>,
}

impl Changes {
    pub fn is_empty(&self) -> bool {
        self.inserted.is_empty() && self.updated.is_empty() && self.deleted.is_empty()
    }
}

/// Managed object context: a working copy of the store. Changes stay here until `save`.
pub struct Context {
    model: Rc<Model>,
    /// The store as this context last read it (at creation, or after its last save).
    base: Store,
    work: Store,
}

impl Context {
    /// Creates an object of `entity` with no values set.
    pub fn insert(&mut self, entity: &str) -> Result<ObjectId, Error> {
        if self.model.entity(entity).is_none() {
            return Err(Error::UnknownEntity(entity.to_string()));
        }
        let id = ObjectId(self.work.next_id);
        self.work.next_id += 1;
        let object = Object {
            entity: entity.to_string(),
            attributes: BTreeMap::new(),
            relationships: BTreeMap::new(),
        };
        self.work.objects.insert(id, object);
        Ok(id)
    }

    /// The entity name of an object in this context.
    pub fn entity_name(&self, id: ObjectId) -> Option<&str> {
        self.work.objects.get(&id).map(|o| o.entity.as_str())
    }

    /// The objects of `entity`, oldest first.
    pub fn fetch(&self, entity: &str) -> Vec<ObjectId> {
        self.work
            .objects
            .iter()
            .filter(|(_, o)| o.entity == entity)
            .map(|(id, _)| *id)
            .collect()
    }

    fn entity_of(&self, id: ObjectId) -> Result<Entity, Error> {
        let name = self.work.objects.get(&id).ok_or(Error::UnknownObject(id))?.entity.clone();
        self.model.entity(&name).cloned().ok_or(Error::UnknownEntity(name))
    }

    fn relationship_def(&self, id: ObjectId, name: &str) -> Result<Relationship, Error> {
        let entity = self.entity_of(id)?;
        entity
            .relationships
            .iter()
            .find(|r| r.name == name)
            .cloned()
            .ok_or_else(|| Error::UnknownProperty { entity: entity.name.clone(), name: name.to_string() })
    }

    /// The value of an attribute; `Null` when none was set.
    pub fn value(&self, id: ObjectId, attribute: &str) -> Result<Value, Error> {
        let entity = self.entity_of(id)?;
        if !entity.attributes.iter().any(|a| a.name == attribute) {
            return Err(Error::UnknownProperty { entity: entity.name.clone(), name: attribute.to_string() });
        }
        Ok(self
            .work
            .objects
            .get(&id)
            .and_then(|o| o.attributes.get(attribute))
            .cloned()
            .unwrap_or(Value::Null))
    }

    /// Sets an attribute. The value must fit the attribute's type. `Null` clears it.
    pub fn set(&mut self, id: ObjectId, attribute: &str, value: Value) -> Result<(), Error> {
        let entity = self.entity_of(id)?;
        let def = entity
            .attributes
            .iter()
            .find(|a| a.name == attribute)
            .ok_or_else(|| Error::UnknownProperty { entity: entity.name.clone(), name: attribute.to_string() })?;
        if !value.fits(def.kind) {
            return Err(Error::Type { attribute: attribute.to_string() });
        }
        let object = self.work.objects.get_mut(&id).ok_or(Error::UnknownObject(id))?;
        object.attributes.insert(attribute.to_string(), value);
        Ok(())
    }

    /// The targets of a relationship, in the order they were linked.
    pub fn targets(&self, id: ObjectId, relationship: &str) -> Result<Vec<ObjectId>, Error> {
        self.relationship_def(id, relationship)?;
        Ok(self
            .work
            .objects
            .get(&id)
            .and_then(|o| o.relationships.get(relationship))
            .cloned()
            .unwrap_or_default())
    }

    /// Links `source` to `target` through `relationship`. A to-one relationship drops its old target.
    /// If the inverse is to-one, the target drops its old source. The inverse edge is always kept.
    pub fn link(&mut self, source: ObjectId, relationship: &str, target: ObjectId) -> Result<(), Error> {
        let rel = self.relationship_def(source, relationship)?;
        let dest = self.entity_of(target)?;
        if dest.name != rel.destination {
            return Err(Error::WrongDestination {
                relationship: relationship.to_string(),
                expected: rel.destination.clone(),
            });
        }
        if !rel.to_many {
            for old in self.targets(source, relationship)? {
                self.remove_edge(source, relationship, old)?;
            }
        }
        if let Some(inv) = rel.inverse.clone() {
            let inverse_to_many = self
                .model
                .entity(&rel.destination)
                .and_then(|e| e.relationships.iter().find(|r| r.name == inv))
                .map(|r| r.to_many)
                .unwrap_or(true);
            if !inverse_to_many {
                for other in self.targets(target, &inv)? {
                    self.remove_edge(other, relationship, target)?;
                }
            }
        }
        self.add_edge(source, relationship, target, &rel)
    }

    /// Removes the link from `source` to `target` through `relationship`, and its inverse edge.
    pub fn unlink(&mut self, source: ObjectId, relationship: &str, target: ObjectId) -> Result<(), Error> {
        self.relationship_def(source, relationship)?;
        self.remove_edge(source, relationship, target)
    }

    fn add_edge(&mut self, source: ObjectId, name: &str, target: ObjectId, rel: &Relationship) -> Result<(), Error> {
        push_id(self.work.objects.get_mut(&source).ok_or(Error::UnknownObject(source))?, name, target);
        if let Some(inv) = &rel.inverse {
            push_id(self.work.objects.get_mut(&target).ok_or(Error::UnknownObject(target))?, inv, source);
        }
        Ok(())
    }

    fn remove_edge(&mut self, source: ObjectId, name: &str, target: ObjectId) -> Result<(), Error> {
        let rel = self.relationship_def(source, name)?;
        if let Some(o) = self.work.objects.get_mut(&source) {
            drop_id(o, name, target);
        }
        if let Some(inv) = &rel.inverse {
            if let Some(o) = self.work.objects.get_mut(&target) {
                drop_id(o, inv, source);
            }
        }
        Ok(())
    }

    /// Deletes an object and every link that points to or from it.
    pub fn delete(&mut self, id: ObjectId) -> Result<(), Error> {
        let entity = self.entity_of(id)?;
        for rel in &entity.relationships {
            for target in self.targets(id, &rel.name)? {
                self.remove_edge(id, &rel.name, target)?;
            }
        }
        let mut incoming = Vec::new();
        for (oid, o) in &self.work.objects {
            if *oid == id {
                continue;
            }
            for (name, list) in &o.relationships {
                if list.contains(&id) {
                    incoming.push((*oid, name.clone()));
                }
            }
        }
        for (other, name) in incoming {
            self.remove_edge(other, &name, id)?;
        }
        self.work.objects.remove(&id);
        Ok(())
    }

    /// The changes this context would write on save, compared with the store it read.
    pub fn changes(&self) -> Changes {
        let mut c = Changes::default();
        for (id, obj) in &self.work.objects {
            match self.base.objects.get(id) {
                None => c.inserted.push(*id),
                Some(b) if b != obj => c.updated.push(*id),
                Some(_) => {}
            }
        }
        for id in self.base.objects.keys() {
            if !self.work.objects.contains_key(id) {
                c.deleted.push(*id);
            }
        }
        c
    }

    pub fn has_changes(&self) -> bool {
        !self.changes().is_empty()
    }

    /// Drops every unsaved change and returns to the store as this context last read it.
    pub fn rollback(&mut self) {
        self.work = self.base.clone();
    }

    /// Writes this context's objects to `container` as a new version. Fails with `Conflict` when the
    /// container moved on since this context read it, and with `Required` when a required attribute
    /// has no value. A failed save changes nothing.
    pub fn save(&mut self, container: &mut Container) -> Result<(), Error> {
        if self.base.version != container.store.version {
            return Err(Error::Conflict { base: self.base.version, store: container.store.version });
        }
        for o in self.work.objects.values() {
            let entity = self.model.entity(&o.entity).ok_or_else(|| Error::UnknownEntity(o.entity.clone()))?;
            for a in &entity.attributes {
                let filled = matches!(o.attributes.get(&a.name), Some(v) if *v != Value::Null);
                if !a.optional && !filled {
                    return Err(Error::Required { attribute: format!("{}.{}", o.entity, a.name) });
                }
            }
        }
        container.store = Store {
            version: container.store.version + 1,
            next_id: self.work.next_id,
            objects: self.work.objects.clone(),
        };
        self.base = container.store.clone();
        self.work = self.base.clone();
        Ok(())
    }
}

fn push_id(o: &mut Object, name: &str, target: ObjectId) {
    let list = o.relationships.entry(name.to_string()).or_default();
    if !list.contains(&target) {
        list.push(target);
    }
}

fn drop_id(o: &mut Object, name: &str, target: ObjectId) {
    if let Some(list) = o.relationships.get_mut(name) {
        list.retain(|x| *x != target);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model() -> Model {
        Model::new(vec![
            Entity {
                name: "Person".into(),
                attributes: vec![
                    Attribute { name: "name".into(), kind: AttributeType::Text, optional: false },
                    Attribute { name: "age".into(), kind: AttributeType::Integer, optional: true },
                ],
                relationships: vec![Relationship {
                    name: "company".into(),
                    destination: "Company".into(),
                    inverse: Some("staff".into()),
                    to_many: false,
                }],
            },
            Entity {
                name: "Company".into(),
                attributes: vec![Attribute { name: "title".into(), kind: AttributeType::Text, optional: true }],
                relationships: vec![Relationship {
                    name: "staff".into(),
                    destination: "Person".into(),
                    inverse: Some("company".into()),
                    to_many: true,
                }],
            },
        ])
        .unwrap()
    }

    fn named(ctx: &mut Context, name: &str) -> ObjectId {
        let id = ctx.insert("Person").unwrap();
        ctx.set(id, "name", Value::Text(name.into())).unwrap();
        id
    }

    #[test]
    fn model_rejects_an_inverse_that_does_not_point_back() {
        let bad = Model::new(vec![
            Entity {
                name: "A".into(),
                attributes: vec![],
                relationships: vec![Relationship {
                    name: "b".into(),
                    destination: "B".into(),
                    inverse: Some("a".into()),
                    to_many: false,
                }],
            },
            Entity {
                name: "B".into(),
                attributes: vec![],
                relationships: vec![Relationship {
                    name: "a".into(),
                    destination: "C".into(),
                    inverse: None,
                    to_many: true,
                }],
            },
        ]);
        assert!(matches!(bad, Err(Error::Model(_))));
    }

    #[test]
    fn values_are_typed_and_unset_values_read_as_null() {
        let m = model();
        let mut c = Container::new(m).new_context();
        let p = c.insert("Person").unwrap();
        assert_eq!(c.value(p, "age").unwrap(), Value::Null);
        c.set(p, "age", Value::Int(41)).unwrap();
        assert_eq!(c.value(p, "age").unwrap(), Value::Int(41));
        assert_eq!(c.set(p, "age", Value::Text("41".into())), Err(Error::Type { attribute: "age".into() }));
        assert!(matches!(c.set(p, "shoe", Value::Null), Err(Error::UnknownProperty { .. })));
    }

    #[test]
    fn linking_one_side_updates_the_inverse() {
        let mut c = Container::new(model()).new_context();
        let (p1, p2) = (named(&mut c, "Ann"), named(&mut c, "Bo"));
        let co = c.insert("Company").unwrap();
        c.link(p1, "company", co).unwrap();
        c.link(p2, "company", co).unwrap();
        assert_eq!(c.targets(co, "staff").unwrap(), vec![p1, p2]);
        assert_eq!(c.targets(p2, "company").unwrap(), vec![co]);
    }

    #[test]
    fn a_to_one_link_replaces_the_old_target_and_its_inverse() {
        let mut c = Container::new(model()).new_context();
        let p = named(&mut c, "Ann");
        let first = c.insert("Company").unwrap();
        let second = c.insert("Company").unwrap();
        c.link(p, "company", first).unwrap();
        c.link(p, "company", second).unwrap();
        assert_eq!(c.targets(p, "company").unwrap(), vec![second]);
        assert!(c.targets(first, "staff").unwrap().is_empty());
        assert_eq!(c.targets(second, "staff").unwrap(), vec![p]);
    }

    #[test]
    fn a_link_to_the_wrong_entity_is_refused() {
        let mut c = Container::new(model()).new_context();
        let p = named(&mut c, "Ann");
        let other = named(&mut c, "Bo");
        assert_eq!(
            c.link(p, "company", other),
            Err(Error::WrongDestination { relationship: "company".into(), expected: "Company".into() })
        );
    }

    #[test]
    fn delete_removes_the_links_that_point_at_the_object() {
        let mut c = Container::new(model()).new_context();
        let p = named(&mut c, "Ann");
        let co = c.insert("Company").unwrap();
        c.link(p, "company", co).unwrap();
        c.delete(co).unwrap();
        assert!(c.targets(p, "company").unwrap().is_empty());
        assert_eq!(c.entity_name(co), None);
    }

    #[test]
    fn save_writes_changes_and_requires_the_required_attributes() {
        let mut container = Container::new(model());
        let mut c = container.new_context();
        let p = c.insert("Person").unwrap();
        assert_eq!(c.save(&mut container), Err(Error::Required { attribute: "Person.name".into() }));
        assert_eq!(container.version(), 0);
        c.set(p, "name", Value::Text("Ann".into())).unwrap();
        assert_eq!(c.changes().inserted, vec![p]);
        c.save(&mut container).unwrap();
        assert_eq!(container.version(), 1);
        assert_eq!(container.object_count(), 1);
        assert!(!c.has_changes());
    }

    #[test]
    fn a_stale_context_gets_a_conflict_and_can_roll_back() {
        let mut container = Container::new(model());
        let mut first = container.new_context();
        let mut second = container.new_context();
        named(&mut first, "Ann");
        first.save(&mut container).unwrap();
        named(&mut second, "Bo");
        assert_eq!(second.save(&mut container), Err(Error::Conflict { base: 0, store: 1 }));
        second.rollback();
        assert!(!second.has_changes());
        assert!(second.fetch("Person").is_empty());
        assert_eq!(first.fetch("Person").len(), 1);
    }

    #[test]
    fn store_text_round_trips_values_links_and_ids() {
        let mut container = Container::new(model());
        let mut c = container.new_context();
        let p = named(&mut c, "Anna Berg 100%");
        let co = c.insert("Company").unwrap();
        c.set(co, "title", Value::Text("Bakery".into())).unwrap();
        c.link(p, "company", co).unwrap();
        c.save(&mut container).unwrap();
        let text = container.to_text();
        let reopened = Container::open(model(), &text).unwrap();
        assert_eq!(reopened.version(), 1);
        assert_eq!(reopened.object_count(), 2);
        let again = reopened.new_context();
        assert_eq!(again.value(p, "name").unwrap(), Value::Text("Anna Berg 100%".into()));
        assert_eq!(again.targets(co, "staff").unwrap(), vec![p]);
        assert_eq!(again.value(co, "title").unwrap(), Value::Text("Bakery".into()));
        assert!(!again.has_changes());
    }

    #[test]
    fn a_damaged_store_text_is_refused_with_its_line() {
        let text = "springboard-store-v1\nversion 1\nnext 2\nobject x Person\n";
        assert_eq!(Container::open(model(), text).err(), Some(Error::Format { line: 4 }));
        assert_eq!(Container::open(model(), "other\n").err(), Some(Error::Format { line: 1 }));
    }
}
