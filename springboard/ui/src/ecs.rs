//! Entities and components: an entity is an id-less bag of components, one per component kind, and a component system
//! runs the periodic `update` of every component of one kind that belongs to it.
//!
//! An `Entity` owns its components. Adding a component whose kind the entity already has replaces the old one and
//! returns it. A `ComponentSystem` manages one kind: it holds the ids of the entities that have that kind, in the order
//! they were added, and `update` visits them in that order against the caller's entity table. The caller owns the
//! entities (a `BTreeMap` from id to entity) so that removing an entity from the table is the caller's decision; the
//! system only forgets an id when `remove_entity` is called. Nothing here knows about time: the caller passes `dt`.

use std::collections::BTreeMap;

/// A piece of behaviour attached to an entity. `kind` names the kind of component and must be the same for every
/// value of one type. The default `update` does nothing, so a component that only holds data needs no code.
pub trait Component {
    fn kind(&self) -> &'static str;

    fn update(&mut self, _dt: f64) {}
}

/// A set of components, at most one of each kind.
#[derive(Default)]
pub struct Entity {
    components: Vec<Box<dyn Component>>,
}

impl Entity {
    pub fn new() -> Self {
        Entity { components: Vec::new() }
    }

    /// Adds a component. If the entity already has one of the same kind, it is replaced and returned.
    pub fn add(&mut self, component: Box<dyn Component>) -> Option<Box<dyn Component>> {
        let kind = component.kind();
        match self.components.iter().position(|c| c.kind() == kind) {
            Some(i) => {
                let old = std::mem::replace(&mut self.components[i], component);
                Some(old)
            }
            None => {
                self.components.push(component);
                None
            }
        }
    }

    /// Removes the component of this kind, if any, and returns it.
    pub fn remove(&mut self, kind: &str) -> Option<Box<dyn Component>> {
        let i = self.components.iter().position(|c| c.kind() == kind)?;
        Some(self.components.remove(i))
    }

    pub fn component(&self, kind: &str) -> Option<&dyn Component> {
        self.components.iter().find(|c| c.kind() == kind).map(|c| c.as_ref())
    }

    pub fn component_mut(&mut self, kind: &str) -> Option<&mut dyn Component> {
        self.components.iter_mut().find(|c| c.kind() == kind).map(|c| c.as_mut())
    }

    pub fn has(&self, kind: &str) -> bool {
        self.components.iter().any(|c| c.kind() == kind)
    }

    pub fn len(&self) -> usize {
        self.components.len()
    }

    pub fn is_empty(&self) -> bool {
        self.components.is_empty()
    }

    /// Calls `update` on every component, in the order they were added.
    pub fn update(&mut self, dt: f64) {
        for c in self.components.iter_mut() {
            c.update(dt);
        }
    }
}

/// Runs `update` for every component of one kind, for the entities that were added to it.
pub struct ComponentSystem {
    kind: &'static str,
    members: Vec<u64>,
}

impl ComponentSystem {
    pub fn new(kind: &'static str) -> Self {
        ComponentSystem { kind, members: Vec::new() }
    }

    pub fn kind(&self) -> &'static str {
        self.kind
    }

    /// Adds the entity with this id if it has a component of the system's kind. Returns false when it does not, or
    /// when the id is already a member.
    pub fn add_entity(&mut self, id: u64, entity: &Entity) -> bool {
        if !entity.has(self.kind) || self.members.contains(&id) {
            return false;
        }
        self.members.push(id);
        true
    }

    /// Forgets the entity with this id. Returns false when it was not a member.
    pub fn remove_entity(&mut self, id: u64) -> bool {
        match self.members.iter().position(|&m| m == id) {
            Some(i) => {
                self.members.remove(i);
                true
            }
            None => false,
        }
    }

    pub fn members(&self) -> &[u64] {
        &self.members
    }

    /// Updates the component of this kind in every member that is still in `entities`, in the order they were added.
    /// A member whose id is missing from the table, or whose component was removed, is skipped and stays a member.
    /// Returns how many components were updated.
    pub fn update(&self, entities: &mut BTreeMap<u64, Entity>, dt: f64) -> usize {
        let mut updated = 0;
        for id in &self.members {
            if let Some(entity) = entities.get_mut(id) {
                if let Some(component) = entity.component_mut(self.kind) {
                    component.update(dt);
                    updated += 1;
                }
            }
        }
        updated
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// Records its id in a shared log each time it is updated, so tests can see which components ran and in what order.
    struct Timer {
        id: u64,
        log: Rc<RefCell<Vec<u64>>>,
    }

    impl Component for Timer {
        fn kind(&self) -> &'static str {
            "timer"
        }

        fn update(&mut self, _dt: f64) {
            self.log.borrow_mut().push(self.id);
        }
    }

    struct Label;

    impl Component for Label {
        fn kind(&self) -> &'static str {
            "label"
        }
    }

    fn timer(id: u64, log: &Rc<RefCell<Vec<u64>>>) -> Box<dyn Component> {
        Box::new(Timer { id, log: Rc::clone(log) })
    }

    #[test]
    fn adding_a_second_component_of_the_same_kind_replaces_the_first() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut e = Entity::new();
        assert!(e.add(timer(1, &log)).is_none());
        let old = e.add(timer(2, &log));
        assert!(old.is_some(), "the replaced component is returned");
        assert_eq!(e.len(), 1, "one component per kind");
        e.update(0.5);
        assert_eq!(*log.borrow(), vec![2], "only the replacement is updated");
    }

    #[test]
    fn entity_update_visits_components_in_the_order_they_were_added() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut e = Entity::new();
        e.add(timer(7, &log));
        e.add(Box::new(Label));
        e.add(Box::new(Timer { id: 8, log: Rc::clone(&log) }));
        // The second timer has the same kind, so it replaces the first and sits in the first slot.
        e.update(1.0);
        assert_eq!(e.len(), 2);
        assert_eq!(*log.borrow(), vec![8]);
        assert!(e.has("label"));
    }

    #[test]
    fn system_updates_only_members_in_insertion_order() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut table: BTreeMap<u64, Entity> = BTreeMap::new();
        for id in [30u64, 10, 20] {
            let mut e = Entity::new();
            e.add(timer(id, &log));
            table.insert(id, e);
        }
        let mut lonely = Entity::new();
        lonely.add(Box::new(Label));
        table.insert(99, lonely);

        let mut system = ComponentSystem::new("timer");
        assert!(system.add_entity(30, &table[&30]));
        assert!(system.add_entity(10, &table[&10]));
        assert!(!system.add_entity(10, &table[&10]), "already a member");
        assert!(!system.add_entity(99, &table[&99]), "no timer component");
        assert_eq!(system.members(), [30u64, 10].as_slice());

        let n = system.update(&mut table, 0.25);
        assert_eq!(n, 2);
        assert_eq!(*log.borrow(), vec![30, 10], "insertion order, id 20 is not a member");
    }

    #[test]
    fn removed_members_and_missing_entities_are_skipped() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut table: BTreeMap<u64, Entity> = BTreeMap::new();
        for id in [1u64, 2, 3] {
            let mut e = Entity::new();
            e.add(timer(id, &log));
            table.insert(id, e);
        }
        let mut system = ComponentSystem::new("timer");
        for id in [1u64, 2, 3] {
            system.add_entity(id, &table[&id]);
        }
        assert!(system.remove_entity(2));
        assert!(!system.remove_entity(2));
        table.remove(&3);
        let n = system.update(&mut table, 1.0);
        assert_eq!(n, 1, "member 2 was forgotten and member 3 has no entity");
        assert_eq!(*log.borrow(), vec![1]);
    }

    #[test]
    fn component_removal_returns_the_component_and_leaves_the_rest() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut e = Entity::new();
        e.add(timer(5, &log));
        e.add(Box::new(Label));
        assert!(e.remove("missing").is_none());
        let removed = e.remove("timer").expect("timer was there");
        assert_eq!(removed.kind(), "timer");
        assert!(!e.has("timer"));
        assert!(e.has("label"));
        assert_eq!(e.len(), 1);
    }
}
