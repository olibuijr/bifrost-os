//! The component registry: finds units by a three-code description (kind, sub-kind, manufacturer) and creates
//! them. A zero code in a query matches any value.

use crate::unit::{AudioUnit, UnitError};

/// A four-ASCII-character code as a big-endian `u32` (0 = wildcard in a query).
pub fn four_cc(text: &str) -> Result<u32, UnitError> {
    let b = text.as_bytes();
    if b.len() != 4 || !b.iter().all(|c| c.is_ascii() && !c.is_ascii_control()) {
        return Err(UnitError::BadCode);
    }
    Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ComponentDescription {
    pub kind: u32,
    pub sub_kind: u32,
    pub manufacturer: u32,
}

impl ComponentDescription {
    pub fn new(kind: &str, sub_kind: &str, manufacturer: &str) -> Result<ComponentDescription, UnitError> {
        Ok(ComponentDescription { kind: four_cc(kind)?, sub_kind: four_cc(sub_kind)?, manufacturer: four_cc(manufacturer)? })
    }

    /// True when `self` (a query) matches `candidate`.
    pub fn matches(&self, candidate: &ComponentDescription) -> bool {
        let m = |q: u32, c: u32| q == 0 || q == c;
        m(self.kind, candidate.kind) && m(self.sub_kind, candidate.sub_kind) && m(self.manufacturer, candidate.manufacturer)
    }
}

pub type UnitFactory = fn() -> Box<dyn AudioUnit>;

/// A factory that may carry state (a loaded plug-in library) and may fail (the plug-in could not make an instance).
pub type BoxedFactory = Box<dyn Fn() -> Result<Box<dyn AudioUnit>, UnitError> + Send + Sync>;

struct Entry {
    description: ComponentDescription,
    name: String,
    factory: BoxedFactory,
}

#[derive(Default)]
pub struct ComponentRegistry {
    entries: Vec<Entry>,
}

impl ComponentRegistry {
    pub fn new() -> ComponentRegistry {
        ComponentRegistry::default()
    }

    /// The registry with the built-in units (`aufx/gain/bfst`, `aufx/bqad/bfst`, `aumu/sine/bfst`, `aumu/gmsy/bfst`).
    pub fn with_builtins() -> ComponentRegistry {
        let mut r = ComponentRegistry::new();
        let d = |k: &str, s: &str| ComponentDescription::new(k, s, "bfst").expect("static codes");
        r.register(d("aufx", "gain"), "Gain", || Box::new(crate::builtin::Gain::new()));
        r.register(d("aufx", "bqad"), "Biquad Filter", || Box::new(crate::builtin::Biquad::new()));
        r.register(d("aumu", "sine"), "Sine Synth", || Box::new(crate::builtin::SineSynth::new()));
        r.register(d("aumu", "gmsy"), "General MIDI Synth", || Box::new(crate::synth::GmSynth::new()));
        r
    }

    pub fn register(&mut self, description: ComponentDescription, name: &str, factory: UnitFactory) {
        self.entries.push(Entry { description, name: name.to_string(), factory: Box::new(move || Ok(factory())) });
    }

    /// Registers a component whose factory carries state or can fail, such as a plug-in loaded from a library.
    pub fn register_boxed(&mut self, description: ComponentDescription, name: &str, factory: BoxedFactory) {
        self.entries.push(Entry { description, name: name.to_string(), factory });
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The next entry after `after` (or the first) matching `query`.
    pub fn find_next(&self, query: &ComponentDescription, after: Option<usize>) -> Option<usize> {
        let start = after.map_or(0, |a| a + 1);
        (start..self.entries.len()).find(|&i| query.matches(&self.entries[i].description))
    }

    pub fn name(&self, index: usize) -> Option<&str> {
        self.entries.get(index).map(|e| e.name.as_str())
    }

    pub fn description(&self, index: usize) -> Option<ComponentDescription> {
        self.entries.get(index).map(|e| e.description)
    }

    pub fn instantiate(&self, index: usize) -> Result<Box<dyn AudioUnit>, UnitError> {
        self.entries.get(index).ok_or(UnitError::NoComponent).and_then(|e| (e.factory)())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn four_cc_is_big_endian_ascii() {
        assert_eq!(four_cc("aufx"), Ok(0x6175_6678));
        assert_eq!(four_cc("abc"), Err(UnitError::BadCode));
        assert_eq!(four_cc("abcde"), Err(UnitError::BadCode));
        assert_eq!(four_cc("ab\u{e9}"), Err(UnitError::BadCode));
    }

    #[test]
    fn wildcard_queries_match() {
        let c = ComponentDescription::new("aufx", "gain", "bfst").unwrap();
        let any = ComponentDescription { kind: 0, sub_kind: 0, manufacturer: 0 };
        assert!(any.matches(&c));
        assert!(ComponentDescription { kind: c.kind, sub_kind: 0, manufacturer: 0 }.matches(&c));
        assert!(!ComponentDescription::new("aumu", "gain", "bfst").unwrap().matches(&c));
    }

    #[test]
    fn find_next_walks_matches_in_order() {
        let r = ComponentRegistry::with_builtins();
        let fx = ComponentDescription { kind: four_cc("aufx").unwrap(), sub_kind: 0, manufacturer: 0 };
        let a = r.find_next(&fx, None).unwrap();
        let b = r.find_next(&fx, Some(a)).unwrap();
        assert_eq!((a, b), (0, 1));
        assert_eq!(r.find_next(&fx, Some(b)), None);
        assert_eq!(r.name(0), Some("Gain"));
    }

    #[test]
    fn instantiate_creates_units_and_refuses_bad_indices() {
        let r = ComponentRegistry::with_builtins();
        assert_eq!(r.instantiate(0).unwrap().info().name, "Gain");
        assert!(matches!(r.instantiate(9), Err(UnitError::NoComponent)));
    }

    #[test]
    fn boxed_factories_carry_state_and_can_fail() {
        let mut r = ComponentRegistry::new();
        let d = ComponentDescription::new("aufx", "bxd1", "test").unwrap();
        let made = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let m = made.clone();
        r.register_boxed(
            d,
            "Counted",
            Box::new(move || {
                m.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                Ok(Box::new(crate::builtin::Gain::new()) as Box<dyn AudioUnit>)
            }),
        );
        r.register_boxed(d, "Broken", Box::new(|| Err(UnitError::Remote)));
        assert!(r.instantiate(0).is_ok());
        assert!(r.instantiate(0).is_ok());
        assert_eq!(made.load(std::sync::atomic::Ordering::Relaxed), 2);
        assert!(matches!(r.instantiate(1), Err(UnitError::Remote)));
        assert_eq!(r.name(1), Some("Broken"));
    }
}
