//! Runtime font registry: the font families an app adds while it runs, with enumeration, removal, named collections
//! and a change counter that stands in for a font-changed notification. This is pure state. The caller reads the
//! font files (`fontprefs::family` gives the family name of a file) and passes the faces in; the registry never
//! touches the disk. Names match without regard to ASCII case, and listings come back sorted by name.

/// One face of a family: the file that holds it and its style.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Face {
    pub path: String,
    /// the style name from the font, such as "Regular" or "Bold Italic"
    pub style: String,
    /// fixed pitch
    pub mono: bool,
    /// carries a variation axis, so every weight renders from this one file
    pub variable: bool,
}

impl Face {
    /// Sort key for choosing one face of a family: upright before italic or oblique, variable before static, and
    /// the Regular style before the others. Lower is better.
    fn rank(&self) -> (bool, bool, bool) {
        let s = self.style.to_ascii_lowercase();
        (s.contains("italic") || s.contains("oblique"), !self.variable, s != "regular")
    }
}

/// A family registered by name, with its faces.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Family {
    pub name: String,
    pub faces: Vec<Face>,
}

/// A named set of registered families. Membership is by family name as the registry stores it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Collection {
    pub name: String,
    pub families: Vec<String>,
}

/// The families and collections registered at run time. `generation` goes up on every change, so a caller that
/// remembers the last value it saw knows when to refresh its font menus.
#[derive(Debug, Default)]
pub struct FontRegistry {
    /// sorted by name, ignoring ASCII case; names are unique under that rule
    families: Vec<Family>,
    /// sorted by name, ignoring ASCII case
    collections: Vec<Collection>,
    generation: u64,
}

fn same_name(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

fn sort_key(name: &str) -> String {
    name.to_ascii_lowercase()
}

impl FontRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds `family`, or replaces the family of the same name when its faces differ. Returns false, and changes
    /// nothing, when the name is empty, the family has no faces, or the same faces are already registered.
    pub fn register(&mut self, family: Family) -> bool {
        if family.name.trim().is_empty() || family.faces.is_empty() {
            return false;
        }
        if let Some(i) = self.index_of(&family.name) {
            if self.families[i].faces == family.faces && self.families[i].name == family.name {
                return false;
            }
            self.families[i] = family;
        } else {
            let at = self.families.partition_point(|f| sort_key(&f.name) < sort_key(&family.name));
            self.families.insert(at, family);
        }
        self.generation += 1;
        true
    }

    /// Removes the family of that name and drops it from every collection. Returns false when it was not registered.
    pub fn unregister(&mut self, name: &str) -> bool {
        let Some(i) = self.index_of(name) else { return false };
        let gone = self.families.remove(i).name;
        for c in &mut self.collections {
            c.families.retain(|f| !same_name(f, &gone));
        }
        self.generation += 1;
        true
    }

    /// The registered family of that name, ignoring ASCII case.
    pub fn family(&self, name: &str) -> Option<&Family> {
        self.index_of(name).map(|i| &self.families[i])
    }

    /// Every registered family name, sorted.
    pub fn family_names(&self) -> Vec<&str> {
        self.families.iter().map(|f| f.name.as_str()).collect()
    }

    /// The family names that are fixed pitch: a family is fixed pitch when every one of its faces is.
    pub fn monospaced_families(&self) -> Vec<&str> {
        self.families.iter().filter(|f| f.faces.iter().all(|x| x.mono)).map(|f| f.name.as_str()).collect()
    }

    /// The face to draw a family with: the best ranked face (upright, variable, Regular first). `None` when the
    /// family is not registered.
    pub fn best_face(&self, name: &str) -> Option<&Face> {
        self.family(name)?.faces.iter().min_by_key(|f| f.rank())
    }

    /// Adds the collection `name` with the listed families. Names that are not registered are left out, and
    /// duplicates count once. Returns false, and changes nothing, when the name is empty or none of the families
    /// is registered. An existing collection of the same name is replaced.
    pub fn add_collection(&mut self, name: &str, families: &[&str]) -> bool {
        if name.trim().is_empty() {
            return false;
        }
        let mut members: Vec<String> = Vec::new();
        for f in families {
            if let Some(i) = self.index_of(f) {
                let canonical = self.families[i].name.clone();
                if !members.iter().any(|m| same_name(m, &canonical)) {
                    members.push(canonical);
                }
            }
        }
        if members.is_empty() {
            return false;
        }
        let collection = Collection { name: name.to_string(), families: members };
        match self.collections.iter().position(|c| same_name(&c.name, name)) {
            Some(i) => self.collections[i] = collection,
            None => {
                let at = self.collections.partition_point(|c| sort_key(&c.name) < sort_key(name));
                self.collections.insert(at, collection);
            }
        }
        self.generation += 1;
        true
    }

    /// Removes the collection of that name. Returns false when there is none. The families stay registered.
    pub fn remove_collection(&mut self, name: &str) -> bool {
        let Some(i) = self.collections.iter().position(|c| same_name(&c.name, name)) else { return false };
        self.collections.remove(i);
        self.generation += 1;
        true
    }

    /// The collection of that name, ignoring ASCII case.
    pub fn collection(&self, name: &str) -> Option<&Collection> {
        self.collections.iter().find(|c| same_name(&c.name, name))
    }

    /// Every collection name, sorted.
    pub fn collection_names(&self) -> Vec<&str> {
        self.collections.iter().map(|c| c.name.as_str()).collect()
    }

    /// Changes on every registration, removal and collection edit. Starts at 0 for an empty registry.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    fn index_of(&self, name: &str) -> Option<usize> {
        let key = sort_key(name);
        let i = self.families.partition_point(|f| sort_key(&f.name) < key);
        (i < self.families.len() && sort_key(&self.families[i].name) == key).then_some(i)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn face(style: &str, mono: bool, variable: bool) -> Face {
        Face { path: format!("/fonts/{style}.ttf"), style: style.to_string(), mono, variable }
    }

    fn family(name: &str, faces: Vec<Face>) -> Family {
        Family { name: name.to_string(), faces }
    }

    #[test]
    fn registers_families_sorted_and_ignores_case_for_lookup() {
        let mut r = FontRegistry::new();
        assert!(r.register(family("Zed", vec![face("Regular", false, false)])));
        assert!(r.register(family("alpha", vec![face("Regular", false, false)])));
        assert!(r.register(family("Mono Code", vec![face("Regular", true, false)])));
        assert_eq!(r.family_names(), vec!["alpha", "Mono Code", "Zed"]);
        assert!(r.family("ALPHA").is_some());
        assert!(r.family("missing").is_none());
        assert_eq!(r.monospaced_families(), vec!["Mono Code"]);
        assert_eq!(r.generation(), 3);
    }

    #[test]
    fn rejects_empty_names_and_faceless_families_and_repeats() {
        let mut r = FontRegistry::new();
        assert!(!r.register(family("  ", vec![face("Regular", false, false)])));
        assert!(!r.register(family("Empty", vec![])));
        assert!(r.register(family("Same", vec![face("Regular", false, false)])));
        let g = r.generation();
        assert!(!r.register(family("Same", vec![face("Regular", false, false)])), "an exact repeat changes nothing");
        assert_eq!(r.generation(), g);
        assert_eq!(r.family_names(), vec!["Same"]);
        assert!(r.register(family("same", vec![face("Regular", false, false)])), "a different spelling of the name replaces");
        assert_eq!(r.family_names(), vec!["same"]);
        assert_eq!(r.generation(), g + 1);
    }

    #[test]
    fn a_new_face_set_replaces_the_family_and_bumps_the_generation() {
        let mut r = FontRegistry::new();
        r.register(family("Text", vec![face("Regular", false, false)]));
        let before = r.generation();
        assert!(r.register(family("Text", vec![face("Regular", false, false), face("Bold", false, false)])));
        assert_eq!(r.family("Text").unwrap().faces.len(), 2);
        assert_eq!(r.generation(), before + 1);
    }

    #[test]
    fn best_face_prefers_upright_variable_then_regular() {
        let mut r = FontRegistry::new();
        r.register(family("Sans", vec![face("Bold Italic", false, false), face("Bold", false, false), face("Regular", false, false)]));
        assert_eq!(r.best_face("sans").unwrap().style, "Regular");
        r.register(family("Sans", vec![face("Italic", false, true), face("Regular", false, false), face("Light", false, true)]));
        assert_eq!(r.best_face("Sans").unwrap().style, "Light", "a variable upright face wins over a static Regular");
        assert!(r.best_face("none").is_none());
    }

    #[test]
    fn unregister_removes_the_family_and_its_collection_members() {
        let mut r = FontRegistry::new();
        r.register(family("A", vec![face("Regular", false, false)]));
        r.register(family("B", vec![face("Regular", false, false)]));
        assert!(r.add_collection("Pair", &["a", "B"]));
        let before = r.generation();
        assert!(r.unregister("a"));
        assert_eq!(r.family_names(), vec!["B"]);
        assert_eq!(r.collection("pair").unwrap().families, vec!["B".to_string()]);
        assert!(!r.unregister("a"), "a second removal finds nothing");
        assert_eq!(r.generation(), before + 1);
    }

    #[test]
    fn collections_keep_registered_members_once_in_canonical_case() {
        let mut r = FontRegistry::new();
        r.register(family("Serif", vec![face("Regular", false, false)]));
        r.register(family("Sans", vec![face("Regular", false, false)]));
        assert!(!r.add_collection("", &["Serif"]));
        assert!(!r.add_collection("Nothing", &["Missing"]), "no registered member means no collection");
        assert!(r.add_collection("Text", &["serif", "Missing", "SERIF", "Sans"]));
        assert_eq!(r.collection("TEXT").unwrap().families, vec!["Serif".to_string(), "Sans".to_string()]);
        assert_eq!(r.collection_names(), vec!["Text"]);
        assert!(r.remove_collection("text"));
        assert!(!r.remove_collection("text"));
        assert!(r.family("Serif").is_some(), "removing a collection keeps its families");
    }

    #[test]
    fn generation_moves_on_every_change_and_not_on_reads() {
        let mut r = FontRegistry::new();
        assert_eq!(r.generation(), 0);
        r.register(family("One", vec![face("Regular", false, false)]));
        let g1 = r.generation();
        let _ = r.family_names();
        let _ = r.best_face("One");
        assert_eq!(r.generation(), g1, "reads do not change the generation");
        r.add_collection("C", &["One"]);
        assert_eq!(r.generation(), g1 + 1);
    }
}
