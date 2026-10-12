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

#[cfg(test)]
mod tests {
    use super::*;

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
