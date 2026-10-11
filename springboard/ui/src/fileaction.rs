//! Actions a provider adds to the context menu of the system document browser: typed action identifiers, a
//! registry of the actions each app offers, and the rule that decides which of them apply to a selection.
//! Pure model: the browser asks `actions_for` with the uniform type identifiers of the selected items, and runs
//! the chosen action through an `actionrequest::ActionRequest`, which the hosted view completes or cancels.
use crate::uttype;

/// A typed identifier for a file action: the registering app's id and the action's name, both non-empty and made
/// of ASCII letters, digits, `-`, `_` or `.` (so the joined form `owner/name` is unambiguous).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ActionId {
    owner: String,
    name: String,
}

impl ActionId {
    /// `None` when either part is empty or holds a character outside the allowed set.
    pub fn new(owner: &str, name: &str) -> Option<Self> {
        if valid_part(owner) && valid_part(name) {
            Some(Self { owner: owner.to_string(), name: name.to_string() })
        } else {
            None
        }
    }

    pub fn owner(&self) -> &str {
        &self.owner
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

fn valid_part(part: &str) -> bool {
    !part.is_empty() && part.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// One action an app offers for the items it accepts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileAction {
    pub id: ActionId,
    pub title: String,
    /// Uniform type identifiers the action takes. An item qualifies when it conforms to one of them.
    pub accepts: Vec<String>,
    /// Largest selection the action takes; `None` takes any number of items.
    pub max_items: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegisterError {
    /// An action with this identifier is already registered.
    Duplicate,
    /// `accepts` is empty, or it names a type the uniform type model does not know.
    UnknownType(String),
}

/// The actions every app has registered, in registration order.
#[derive(Clone, Debug, Default)]
pub struct ActionRegistry {
    actions: Vec<FileAction>,
}

impl ActionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.actions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }

    pub fn register(&mut self, action: FileAction) -> Result<(), RegisterError> {
        if self.get(&action.id).is_some() {
            return Err(RegisterError::Duplicate);
        }
        if action.accepts.is_empty() {
            return Err(RegisterError::UnknownType(String::new()));
        }
        if let Some(bad) = action.accepts.iter().find(|t| uttype::description(t).is_empty()) {
            return Err(RegisterError::UnknownType(bad.clone()));
        }
        self.actions.push(action);
        Ok(())
    }

    pub fn get(&self, id: &ActionId) -> Option<&FileAction> {
        self.actions.iter().find(|a| &a.id == id)
    }

    /// Removes one action; `true` when it was registered.
    pub fn unregister(&mut self, id: &ActionId) -> bool {
        let before = self.actions.len();
        self.actions.retain(|a| &a.id != id);
        self.actions.len() != before
    }

    /// Removes every action an app owns, for example when the app quits. Returns how many were removed.
    pub fn unregister_owner(&mut self, owner: &str) -> usize {
        let before = self.actions.len();
        self.actions.retain(|a| a.id.owner() != owner);
        before - self.actions.len()
    }

    /// The actions that apply to a selection of items, given by their uniform type identifiers. Every item must
    /// conform to one of an action's accepted types, and the selection must fit `max_items`. An empty selection
    /// matches nothing.
    pub fn actions_for(&self, items: &[&str]) -> Vec<&FileAction> {
        if items.is_empty() {
            return Vec::new();
        }
        self.actions
            .iter()
            .filter(|a| a.max_items.map_or(true, |max| items.len() <= max))
            .filter(|a| items.iter().all(|item| a.accepts.iter().any(|t| uttype::conforms(item, t))))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn action(owner: &str, name: &str, accepts: &[&str], max_items: Option<usize>) -> FileAction {
        FileAction {
            id: ActionId::new(owner, name).unwrap(),
            title: format!("{name} for {owner}"),
            accepts: accepts.iter().map(|t| t.to_string()).collect(),
            max_items,
        }
    }

    #[test]
    fn action_ids_need_two_clean_parts() {
        assert!(ActionId::new("org.example.notes", "export-pdf").is_some());
        assert_eq!(ActionId::new("", "x"), None);
        assert_eq!(ActionId::new("app", "has space"), None);
        assert_eq!(ActionId::new("app", "a/b"), None);
        let id = ActionId::new("org.example", "share").unwrap();
        assert_eq!((id.owner(), id.name()), ("org.example", "share"));
    }

    #[test]
    fn registration_rejects_duplicates_and_unknown_types() {
        let mut r = ActionRegistry::new();
        assert_eq!(r.register(action("app", "tag", &["public.image"], None)), Ok(()));
        assert_eq!(r.register(action("app", "tag", &["public.data"], None)), Err(RegisterError::Duplicate));
        assert_eq!(
            r.register(action("app", "bad", &["no.such.type"], None)),
            Err(RegisterError::UnknownType("no.such.type".to_string()))
        );
        assert_eq!(r.register(action("app", "empty", &[], None)), Err(RegisterError::UnknownType(String::new())));
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn an_action_applies_to_items_that_conform_to_its_types() {
        let mut r = ActionRegistry::new();
        r.register(action("photos", "resize", &["public.image"], None)).unwrap();
        r.register(action("reader", "open-text", &["public.text"], None)).unwrap();
        let names = |items: &[&str]| -> Vec<String> { r.actions_for(items).iter().map(|a| a.id.name().to_string()).collect() };
        assert_eq!(names(&["public.png"]), vec!["resize"]);
        assert_eq!(names(&["public.rust-source"]), vec!["open-text"]);
        assert_eq!(names(&["com.adobe.pdf"]), Vec::<String>::new());
    }

    #[test]
    fn every_selected_item_must_qualify() {
        let mut r = ActionRegistry::new();
        r.register(action("app", "image-only", &["public.image"], None)).unwrap();
        r.register(action("app", "any-data", &["public.data"], None)).unwrap();
        let mixed = r.actions_for(&["public.png", "com.adobe.pdf"]);
        assert_eq!(mixed.len(), 1);
        assert_eq!(mixed[0].id.name(), "any-data");
        assert!(r.actions_for(&[]).is_empty(), "an empty selection matches nothing");
    }

    #[test]
    fn max_items_limits_the_selection_size() {
        let mut r = ActionRegistry::new();
        r.register(action("app", "single", &["public.data"], Some(1))).unwrap();
        assert_eq!(r.actions_for(&["public.png"]).len(), 1);
        assert!(r.actions_for(&["public.png", "public.png"]).is_empty());
    }

    #[test]
    fn unregistering_an_owner_keeps_the_other_apps_actions_in_order() {
        let mut r = ActionRegistry::new();
        r.register(action("a", "one", &["public.data"], None)).unwrap();
        r.register(action("b", "two", &["public.data"], None)).unwrap();
        r.register(action("a", "three", &["public.data"], None)).unwrap();
        assert_eq!(r.unregister_owner("a"), 2);
        assert_eq!(r.unregister_owner("a"), 0);
        let left: Vec<String> = r.actions_for(&["public.png"]).iter().map(|x| x.id.owner().to_string()).collect();
        assert_eq!(left, vec!["b".to_string()]);
        let id = ActionId::new("b", "two").unwrap();
        assert!(r.unregister(&id));
        assert!(!r.unregister(&id));
        assert!(r.is_empty());
    }
}
