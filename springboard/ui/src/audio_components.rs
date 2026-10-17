//! Audio component registry: the plug-in units (sources, effects and sinks) that a host discovers and loads by id.
//! The registry holds descriptions only; `audio_engine` builds the nodes. A pure model with no I/O.

/// What a component does in a graph.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComponentKind {
    /// Produces audio, such as a player or a synth.
    Source,
    /// Changes audio, such as a gain or a filter.
    Effect,
    /// Receives audio, such as an output device.
    Sink,
}

/// One registered component.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Component {
    /// A stable reverse-DNS style id, unique in the registry.
    pub id: String,
    pub name: String,
    pub kind: ComponentKind,
    pub manufacturer: String,
    pub version: u32,
}

/// The components a host can discover, in registration order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ComponentRegistry {
    components: Vec<Component>,
}

impl ComponentRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a component. An entry with the same id is replaced, which is how a newer version of a unit is
    /// installed. False when the id is empty.
    pub fn register(&mut self, component: Component) -> bool {
        if component.id.is_empty() {
            return false;
        }
        match self.components.iter().position(|c| c.id == component.id) {
            Some(i) => self.components[i] = component,
            None => self.components.push(component),
        }
        true
    }

    /// Removes a component. False when no component has the id.
    pub fn unregister(&mut self, id: &str) -> bool {
        let before = self.components.len();
        self.components.retain(|c| c.id != id);
        self.components.len() != before
    }

    /// The component with this id.
    pub fn find(&self, id: &str) -> Option<&Component> {
        self.components.iter().find(|c| c.id == id)
    }

    /// Every component of one kind, in registration order.
    pub fn discover(&self, kind: ComponentKind) -> Vec<&Component> {
        self.components.iter().filter(|c| c.kind == kind).collect()
    }

    pub fn len(&self) -> usize {
        self.components.len()
    }

    pub fn is_empty(&self) -> bool {
        self.components.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(id: &str, kind: ComponentKind, version: u32) -> Component {
        Component { id: id.into(), name: format!("Unit {id}"), kind, manufacturer: "BifrOSt".into(), version }
    }

    #[test]
    fn registering_makes_a_component_discoverable_by_id_and_kind() {
        let mut r = ComponentRegistry::new();
        assert!(r.is_empty());
        assert!(r.register(unit("os.gain", ComponentKind::Effect, 1)));
        assert!(r.register(unit("os.player", ComponentKind::Source, 1)));
        assert!(r.register(unit("os.out", ComponentKind::Sink, 1)));
        assert_eq!(r.len(), 3);
        assert_eq!(r.find("os.gain").map(|c| c.kind), Some(ComponentKind::Effect));
        let effects: Vec<&str> = r.discover(ComponentKind::Effect).iter().map(|c| c.id.as_str()).collect();
        assert_eq!(effects, vec!["os.gain"]);
        assert!(r.discover(ComponentKind::Source).iter().all(|c| c.kind == ComponentKind::Source));
    }

    #[test]
    fn an_empty_id_is_refused() {
        let mut r = ComponentRegistry::new();
        assert!(!r.register(unit("", ComponentKind::Sink, 1)));
        assert!(r.is_empty());
    }

    #[test]
    fn a_newer_version_replaces_the_entry_in_place() {
        let mut r = ComponentRegistry::new();
        r.register(unit("os.a", ComponentKind::Source, 1));
        r.register(unit("os.b", ComponentKind::Source, 1));
        r.register(unit("os.a", ComponentKind::Source, 2));
        assert_eq!(r.len(), 2);
        assert_eq!(r.find("os.a").map(|c| c.version), Some(2));
        let ids: Vec<&str> = r.discover(ComponentKind::Source).iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, vec!["os.a", "os.b"], "registration order is kept");
    }

    #[test]
    fn unregister_removes_only_the_named_component() {
        let mut r = ComponentRegistry::new();
        r.register(unit("os.a", ComponentKind::Effect, 1));
        r.register(unit("os.b", ComponentKind::Effect, 1));
        assert!(r.unregister("os.a"));
        assert!(!r.unregister("os.a"));
        assert!(r.find("os.a").is_none());
        assert!(r.find("os.b").is_some());
    }
}
