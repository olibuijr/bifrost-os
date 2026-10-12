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

/// Where the file manager shows an action: the menu for selected items, the menu for the folder background (the
/// container, which is passed as its type identifier, for example `public.folder`), or the toolbar.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Placement {
    ItemsMenu,
    ContainerMenu,
    Toolbar,
}

/// The actions every app has registered, in registration order, and the placements each one is shown in.
#[derive(Clone, Debug, Default)]
pub struct ActionRegistry {
    actions: Vec<FileAction>,
    placements: Vec<(ActionId, Placement)>,
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
        self.placements.retain(|(a, _)| a != id);
        self.actions.len() != before
    }

    /// Removes every action an app owns, for example when the app quits. Returns how many were removed.
    pub fn unregister_owner(&mut self, owner: &str) -> usize {
        let before = self.actions.len();
        self.actions.retain(|a| a.id.owner() != owner);
        self.placements.retain(|(a, _)| a.owner() != owner);
        before - self.actions.len()
    }

    /// Shows a registered action in a placement. Placing it twice in the same place changes nothing. `false` when the
    /// action is not registered.
    pub fn place(&mut self, id: &ActionId, placement: Placement) -> bool {
        if self.get(id).is_none() {
            return false;
        }
        if !self.placements.iter().any(|(a, p)| a == id && *p == placement) {
            self.placements.push((id.clone(), placement));
        }
        true
    }

    /// Hides an action from a placement. `true` when it was shown there.
    pub fn unplace(&mut self, id: &ActionId, placement: Placement) -> bool {
        let before = self.placements.len();
        self.placements.retain(|(a, p)| !(a == id && *p == placement));
        self.placements.len() != before
    }

    /// The actions shown in a placement that apply to a selection, in registration order. The rule is the one of
    /// [`ActionRegistry::actions_for`]. For [`Placement::ContainerMenu`], pass the folder's type identifier as the item.
    pub fn placed_for(&self, placement: Placement, items: &[&str]) -> Vec<&FileAction> {
        self.actions_for(items)
            .into_iter()
            .filter(|a| self.placements.iter().any(|(id, p)| id == &a.id && *p == placement))
            .collect()
    }

    /// The actions shown in the toolbar, in registration order. The toolbar does not depend on the selection.
    pub fn toolbar_actions(&self) -> Vec<&FileAction> {
        self.actions
            .iter()
            .filter(|a| self.placements.iter().any(|(id, p)| id == &a.id && *p == Placement::Toolbar))
            .collect()
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

    #[test]
    fn an_action_is_shown_only_in_the_placements_it_was_given() {
        let mut r = ActionRegistry::new();
        r.register(action("app", "resize", &["public.image"], None)).unwrap();
        let id = ActionId::new("app", "resize").unwrap();
        assert!(r.place(&id, Placement::ItemsMenu));
        assert!(r.place(&id, Placement::ItemsMenu), "placing twice is allowed and changes nothing");
        assert_eq!(r.placements.len(), 1);
        assert_eq!(r.placed_for(Placement::ItemsMenu, &["public.png"]).len(), 1);
        assert!(r.placed_for(Placement::ContainerMenu, &["public.png"]).is_empty());
        assert!(r.unplace(&id, Placement::ItemsMenu));
        assert!(!r.unplace(&id, Placement::ItemsMenu));
        assert!(r.placed_for(Placement::ItemsMenu, &["public.png"]).is_empty());
    }

    #[test]
    fn the_folder_background_menu_takes_the_folder_type() {
        let mut r = ActionRegistry::new();
        r.register(action("app", "new-note", &["public.folder"], None)).unwrap();
        let id = ActionId::new("app", "new-note").unwrap();
        r.place(&id, Placement::ContainerMenu);
        assert_eq!(r.placed_for(Placement::ContainerMenu, &["public.folder"]).len(), 1);
        assert!(r.placed_for(Placement::ContainerMenu, &["public.png"]).is_empty());
        assert!(r.placed_for(Placement::ItemsMenu, &["public.folder"]).is_empty());
    }

    #[test]
    fn placing_an_unregistered_action_is_refused() {
        let mut r = ActionRegistry::new();
        let id = ActionId::new("app", "ghost").unwrap();
        assert!(!r.place(&id, Placement::Toolbar));
        assert!(r.toolbar_actions().is_empty());
    }

    #[test]
    fn toolbar_actions_ignore_the_selection() {
        let mut r = ActionRegistry::new();
        r.register(action("app", "sync", &["public.image"], None)).unwrap();
        r.register(action("app", "hidden", &["public.image"], None)).unwrap();
        r.place(&ActionId::new("app", "sync").unwrap(), Placement::Toolbar);
        let shown: Vec<&str> = r.toolbar_actions().iter().map(|a| a.id.name()).collect();
        assert_eq!(shown, vec!["sync"]);
        assert_eq!(r.placed_for(Placement::Toolbar, &["com.adobe.pdf"]).len(), 0);
        assert_eq!(r.placed_for(Placement::Toolbar, &["public.png"]).len(), 1);
    }

    #[test]
    fn unregistering_removes_the_placements_too() {
        let mut r = ActionRegistry::new();
        r.register(action("a", "one", &["public.data"], None)).unwrap();
        r.register(action("b", "two", &["public.data"], None)).unwrap();
        r.place(&ActionId::new("a", "one").unwrap(), Placement::Toolbar);
        r.place(&ActionId::new("b", "two").unwrap(), Placement::Toolbar);
        assert_eq!(r.unregister_owner("a"), 1);
        assert_eq!(r.placements.len(), 1);
        assert_eq!(r.toolbar_actions().len(), 1);
        assert!(r.unregister(&ActionId::new("b", "two").unwrap()));
        assert!(r.placements.is_empty());
    }
}
