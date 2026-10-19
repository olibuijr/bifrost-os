//! The widgets an app offers: the kinds it registers, the sizes (families) each kind supports, and the bundle that
//! exposes several kinds from one app. Pure model of the registration side of the widget protocol: a kind has a
//! unique identifier (the string a widget is reloaded by), the names shown when a user adds it, the sizes it can
//! take, and whether it takes user settings. Nothing here draws or stores anything; the shell reads the registry
//! when it builds the widget gallery.

/// The sizes a widget can take on the home screen or Today view, smallest first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Family {
    Small,
    Medium,
    Large,
}

impl Family {
    /// Every size, smallest first.
    pub const ALL: [Family; 3] = [Family::Small, Family::Medium, Family::Large];

    /// The name a size is stored under in configuration files.
    pub fn key(self) -> &'static str {
        match self {
            Family::Small => "small",
            Family::Medium => "medium",
            Family::Large => "large",
        }
    }

    /// The size stored under `key` (see [`Family::key`]); None for any other text.
    pub fn from_key(key: &str) -> Option<Family> {
        Family::ALL.iter().copied().find(|f| f.key() == key.trim())
    }

    /// The name shown on the gallery's size chip.
    pub fn label(self) -> &'static str {
        match self {
            Family::Small => "Small",
            Family::Medium => "Medium",
            Family::Large => "Large",
        }
    }
}

/// Longest identifier a kind may have, in bytes.
pub const MAX_ID_LEN: usize = 128;

/// Why a kind or a bundle was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KindError {
    /// The identifier is empty.
    EmptyId,
    /// The identifier is longer than `MAX_ID_LEN`.
    TooLong,
    /// The identifier has a character other than an ASCII letter, a digit, `.`, `-` or `_`.
    BadChar(char),
    /// The kind lists no sizes, so no widget could be shown.
    NoFamilies,
    /// A kind with this identifier is already in the bundle.
    DuplicateId(String),
}

fn check_id(id: &str) -> Result<(), KindError> {
    if id.is_empty() {
        return Err(KindError::EmptyId);
    }
    if id.len() > MAX_ID_LEN {
        return Err(KindError::TooLong);
    }
    match id.chars().find(|c| !(c.is_ascii_alphanumeric() || matches!(*c, '.' | '-' | '_'))) {
        Some(c) => Err(KindError::BadChar(c)),
        None => Ok(()),
    }
}

/// One widget kind an app registers.
#[derive(Clone, Debug, PartialEq)]
pub struct WidgetKind {
    id: String,
    /// The name shown when a user adds or edits the widget.
    pub display_name: String,
    /// The description shown with the name when a user adds or edits the widget.
    pub description: String,
    /// True when the kind takes user settings (see the widget_config module).
    pub configurable: bool,
    families: Vec<Family>,
}

impl WidgetKind {
    /// A kind with an identifier, a name and the sizes it supports. Sizes are kept sorted and without repeats.
    pub fn new(id: &str, display_name: &str, families: &[Family]) -> Result<Self, KindError> {
        check_id(id)?;
        if families.is_empty() {
            return Err(KindError::NoFamilies);
        }
        let mut sizes = families.to_vec();
        sizes.sort();
        sizes.dedup();
        Ok(WidgetKind {
            id: id.to_string(),
            display_name: display_name.to_string(),
            description: String::new(),
            configurable: false,
            families: sizes,
        })
    }

    /// Sets the description shown with the name in the gallery.
    pub fn with_description(mut self, description: &str) -> Self {
        self.description = description.to_string();
        self
    }

    /// Marks the kind as taking user settings (or not).
    pub fn with_configurable(mut self, configurable: bool) -> Self {
        self.configurable = configurable;
        self
    }

    /// The identifier the kind is reloaded by.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The sizes the kind supports, smallest first.
    pub fn families(&self) -> &[Family] {
        &self.families
    }

    /// True when the kind can be shown at this size.
    pub fn supports(&self, family: Family) -> bool {
        self.families.contains(&family)
    }
}

/// The kinds one app exposes together. Identifiers are unique within a bundle.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WidgetBundle {
    kinds: Vec<WidgetKind>,
}

impl WidgetBundle {
    pub fn new() -> Self {
        WidgetBundle { kinds: Vec::new() }
    }

    /// Adds a kind. Refused when a kind with the same identifier is already in the bundle.
    pub fn add(&mut self, kind: WidgetKind) -> Result<(), KindError> {
        if self.kinds.iter().any(|k| k.id == kind.id) {
            return Err(KindError::DuplicateId(kind.id.clone()));
        }
        self.kinds.push(kind);
        Ok(())
    }

    /// The kinds in registration order.
    pub fn kinds(&self) -> &[WidgetKind] {
        &self.kinds
    }

    /// The kind with this identifier, if the bundle has one.
    pub fn get(&self, id: &str) -> Option<&WidgetKind> {
        self.kinds.iter().find(|k| k.id == id)
    }

    pub fn len(&self) -> usize {
        self.kinds.len()
    }

    pub fn is_empty(&self) -> bool {
        self.kinds.is_empty()
    }

    /// The kinds that can be shown at this size, in registration order.
    pub fn offered_at(&self, family: Family) -> Vec<&WidgetKind> {
        self.kinds.iter().filter(|k| k.supports(family)).collect()
    }

    /// Every size some kind supports, smallest first, with no repeats.
    pub fn sizes(&self) -> Vec<Family> {
        Family::ALL.iter().copied().filter(|f| self.kinds.iter().any(|k| k.supports(*f))).collect()
    }
}

/// The text inside one pair of double quotes, or the trimmed text when it has none.
fn unquote(v: &str) -> String {
    let v = v.trim();
    v.strip_prefix('"').and_then(|r| r.strip_suffix('"')).unwrap_or(v).to_string()
}

/// The items of a one-line array, `["a", "b"]`, unquoted. Text that is not an array is one item.
fn list(v: &str) -> Vec<String> {
    let v = v.trim();
    match v.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
        Some(inner) => inner.split(',').map(unquote).filter(|s| !s.is_empty()).collect(),
        None => vec![unquote(v)],
    }
}

/// Reads the `[[widget]]` tables of an app manifest into kinds:
///
/// ```toml
/// [[widget]]
/// id = "next-event"
/// name = "Next Event"
/// description = "The next event in your calendar."
/// families = ["small", "medium"]   # left out: small and medium
/// configurable = true
/// ```
///
/// A table with an unsafe or empty id, or one whose families are all unknown, is dropped, so one bad table does not
/// hide the others; a second table with the same id is dropped too. A blank name shows the id.
pub fn parse_manifest(text: &str) -> Vec<WidgetKind> {
    #[derive(Default)]
    struct Draft {
        id: String,
        name: String,
        description: String,
        families: Option<Vec<String>>,
        configurable: bool,
    }
    let mut drafts: Vec<Draft> = Vec::new();
    let mut in_table = false;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && !line.contains('=') {
            in_table = line == "[[widget]]";
            if in_table {
                drafts.push(Draft::default());
            }
            continue;
        }
        if !in_table {
            continue;
        }
        let (Some((key, value)), Some(d)) = (line.split_once('='), drafts.last_mut()) else { continue };
        match key.trim() {
            "id" => d.id = unquote(value),
            "name" => d.name = unquote(value),
            "description" => d.description = unquote(value),
            "families" => d.families = Some(list(value)),
            "configurable" => d.configurable = value.trim() == "true",
            _ => {}
        }
    }
    let mut out: Vec<WidgetKind> = Vec::new();
    for d in drafts {
        let families: Vec<Family> = match &d.families {
            None => vec![Family::Small, Family::Medium],
            Some(keys) => keys.iter().filter_map(|k| Family::from_key(k)).collect(),
        };
        let name = if d.name.trim().is_empty() { d.id.clone() } else { d.name.trim().to_string() };
        let Ok(kind) = WidgetKind::new(&d.id, &name, &families) else { continue };
        if out.iter().any(|k| k.id() == kind.id()) {
            continue;
        }
        out.push(kind.with_description(&d.description).with_configurable(d.configurable));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_tables_become_kinds_with_their_sizes_and_texts() {
        let text = "id = \"is.x.app\"\n[[widget]]\nid = \"next\"\nname = \"Next Event\"\ndescription = \"The next one.\"\nfamilies = [\"large\", \"small\"]\nconfigurable = true\n\n[[widget]]\nid = \"plain\"\n";
        let kinds = parse_manifest(text);
        assert_eq!(kinds.len(), 2);
        assert_eq!(kinds[0].id(), "next");
        assert_eq!(kinds[0].display_name, "Next Event");
        assert_eq!(kinds[0].description, "The next one.");
        assert_eq!(kinds[0].families(), &[Family::Small, Family::Large], "sorted, smallest first");
        assert!(kinds[0].configurable);
        assert_eq!(kinds[1].display_name, "plain", "a blank name shows the id");
        assert_eq!(kinds[1].families(), &[Family::Small, Family::Medium], "left out: small and medium");
        assert!(!kinds[1].configurable);
    }

    #[test]
    fn a_bad_table_is_dropped_without_hiding_the_others() {
        let text = "[[widget]]\nid = \"bad id\"\n[[widget]]\nid = \"nofam\"\nfamilies = [\"huge\"]\n[[widget]]\nid = \"ok\"\nfamilies = [\"medium\", \"huge\"]\n[[widget]]\nid = \"ok\"\n[[control]]\nid = \"other\"\n";
        let kinds = parse_manifest(text);
        let ids: Vec<&str> = kinds.iter().map(|k| k.id()).collect();
        assert_eq!(ids, vec!["ok"], "unsafe id, unknown sizes, a repeated id and other tables are not kinds");
        assert_eq!(kinds[0].families(), &[Family::Medium], "the unknown size is skipped");
    }

    #[test]
    fn family_keys_round_trip_and_unknown_text_is_none() {
        for f in Family::ALL {
            assert_eq!(Family::from_key(f.key()), Some(f));
        }
        assert_eq!(Family::from_key(" medium "), Some(Family::Medium));
        assert_eq!(Family::from_key("extra-large"), None);
        assert_eq!(Family::Large.label(), "Large");
    }

    #[test]
    fn sizes_are_sorted_and_deduplicated() {
        let k = WidgetKind::new("app.stats", "Stats", &[Family::Large, Family::Small, Family::Large]).unwrap();
        assert_eq!(k.families(), &[Family::Small, Family::Large]);
        assert!(k.supports(Family::Large));
        assert!(!k.supports(Family::Medium));
    }

    #[test]
    fn identifier_rules() {
        assert_eq!(WidgetKind::new("", "x", &[Family::Small]).unwrap_err(), KindError::EmptyId);
        assert_eq!(WidgetKind::new("a b", "x", &[Family::Small]).unwrap_err(), KindError::BadChar(' '));
        assert_eq!(WidgetKind::new("a/b", "x", &[Family::Small]).unwrap_err(), KindError::BadChar('/'));
        let long = "a".repeat(MAX_ID_LEN + 1);
        assert_eq!(WidgetKind::new(&long, "x", &[Family::Small]).unwrap_err(), KindError::TooLong);
        let edge = "a".repeat(MAX_ID_LEN);
        assert!(WidgetKind::new(&edge, "x", &[Family::Small]).is_ok());
        assert!(WidgetKind::new("com.example-app_1.stats", "x", &[Family::Small]).is_ok());
    }

    #[test]
    fn a_kind_needs_at_least_one_size() {
        assert_eq!(WidgetKind::new("app.empty", "Empty", &[]).unwrap_err(), KindError::NoFamilies);
    }

    #[test]
    fn bundle_refuses_a_second_kind_with_the_same_id() {
        let mut b = WidgetBundle::new();
        b.add(WidgetKind::new("app.one", "One", &[Family::Small]).unwrap()).unwrap();
        let again = WidgetKind::new("app.one", "Other", &[Family::Medium]).unwrap();
        assert_eq!(b.add(again), Err(KindError::DuplicateId("app.one".to_string())));
        assert_eq!(b.len(), 1);
        assert_eq!(b.get("app.one").unwrap().display_name, "One");
        assert!(b.get("app.missing").is_none());
    }

    #[test]
    fn offered_at_keeps_registration_order() {
        let mut b = WidgetBundle::new();
        b.add(WidgetKind::new("a", "A", &[Family::Small]).unwrap()).unwrap();
        b.add(WidgetKind::new("b", "B", &[Family::Medium, Family::Large]).unwrap()).unwrap();
        b.add(WidgetKind::new("c", "C", &[Family::Small, Family::Medium]).unwrap()).unwrap();
        let medium: Vec<String> = b.offered_at(Family::Medium).iter().map(|k| k.id().to_string()).collect();
        assert_eq!(medium, vec!["b".to_string(), "c".to_string()]);
        let small: Vec<String> = b.offered_at(Family::Small).iter().map(|k| k.id().to_string()).collect();
        assert_eq!(small, vec!["a".to_string(), "c".to_string()]);
    }

    #[test]
    fn sizes_are_the_union_of_kind_sizes() {
        let mut b = WidgetBundle::new();
        assert!(b.sizes().is_empty());
        assert!(b.is_empty());
        b.add(WidgetKind::new("a", "A", &[Family::Large]).unwrap()).unwrap();
        b.add(WidgetKind::new("b", "B", &[Family::Small, Family::Large]).unwrap()).unwrap();
        assert_eq!(b.sizes(), vec![Family::Small, Family::Large]);
        assert_eq!(Family::Medium.key(), "medium");
    }
}
