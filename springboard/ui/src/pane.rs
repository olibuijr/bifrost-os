//! Preference panes: a host (the Settings app) shows one pane at a time. A pane runs through a lifecycle (will and
//! did select, will and did unselect), can refuse to be left (should unselect), commits its text fields when it is
//! left (auto save), and adds items to the Help menu while it is shown. `PaneHost` drives the panes and queues
//! `PaneNotice`s for observers to take in order.

/// Notifications a host sends to observers while it runs panes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaneNotice {
    /// The host can show panes now.
    IsAvailable,
    /// The selected pane may be deselected (its refusal was not given).
    DoUnselect,
    /// The selected pane refused to be deselected, so it stays selected.
    CancelUnselect,
    /// The user selected a different pane.
    SwitchToPane,
    /// The Help menu items of the shown pane changed.
    UpdateHelpMenu,
}

/// A Help menu item a pane adds, with an optional help book anchor to open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HelpItem {
    pub title: String,
    pub anchor: Option<String>,
}

/// What a pane is: its name, title, keyboard focus entry and exit, auto save and Help menu items.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaneSpec {
    pub name: String,
    pub title: String,
    /// The view that takes keyboard focus when the pane is selected. Falls back to the first view if missing.
    pub initial_key_view: Option<u64>,
    /// The view that ends the pane's keyboard focus loop. Falls back to the last view if missing.
    pub last_key_view: Option<u64>,
    /// Whether text fields commit what the user typed when the pane is left. Uncommitted edits are dropped otherwise.
    pub auto_save_text_fields: bool,
    pub help: Vec<HelpItem>,
}

impl PaneSpec {
    /// A pane with no focus hints, no help items and auto save on.
    pub fn new(name: &str, title: &str) -> Self {
        Self {
            name: name.into(),
            title: title.into(),
            initial_key_view: None,
            last_key_view: None,
            auto_save_text_fields: true,
            help: Vec::new(),
        }
    }
}

/// The steps of a pane's lifecycle. Override only the steps the pane needs.
pub trait PaneHooks {
    /// Whether the pane may be left now. `false` keeps it selected (a veto).
    fn should_unselect(&mut self) -> bool {
        true
    }
    fn will_select(&mut self) {}
    fn did_select(&mut self) {}
    fn will_unselect(&mut self) {}
    fn did_unselect(&mut self) {}
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaneError {
    /// No pane has that name.
    Unknown,
    /// The selected pane refused to be left.
    Refused,
}

struct Entry {
    spec: PaneSpec,
    hooks: Box<dyn PaneHooks>,
    /// Edits typed in the pane and not yet committed, as (field, value).
    pending: Vec<(u64, String)>,
    /// Committed field values, as (field, value).
    saved: Vec<(u64, String)>,
}

/// Runs a set of panes. At most one pane is selected at a time.
pub struct PaneHost {
    panes: Vec<Entry>,
    selected: Option<usize>,
    notices: Vec<PaneNotice>,
}

impl Default for PaneHost {
    fn default() -> Self {
        Self::new()
    }
}

impl PaneHost {
    /// An empty host with no pane selected. It is available at once, so `IsAvailable` is queued.
    pub fn new() -> Self {
        Self { panes: Vec::new(), selected: None, notices: vec![PaneNotice::IsAvailable] }
    }

    /// Adds a pane. Returns false, and adds nothing, when a pane with the same name is already there.
    pub fn add(&mut self, spec: PaneSpec, hooks: Box<dyn PaneHooks>) -> bool {
        if self.index_of(&spec.name).is_some() {
            return false;
        }
        self.panes.push(Entry { spec, hooks, pending: Vec::new(), saved: Vec::new() });
        true
    }

    fn index_of(&self, name: &str) -> Option<usize> {
        self.panes.iter().position(|p| p.spec.name == name)
    }

    /// The description of a pane.
    pub fn spec(&self, name: &str) -> Option<&PaneSpec> {
        self.index_of(name).map(|i| &self.panes[i].spec)
    }

    /// The name of the selected pane.
    pub fn selected(&self) -> Option<&str> {
        self.selected.map(|i| self.panes[i].spec.name.as_str())
    }

    /// Selects a pane. Selecting the shown pane again does nothing. Otherwise the shown pane is asked first; if it
    /// refuses, nothing changes, `CancelUnselect` is queued and `PaneError::Refused` is returned.
    pub fn select(&mut self, name: &str) -> Result<(), PaneError> {
        let to = self.index_of(name).ok_or(PaneError::Unknown)?;
        if self.selected == Some(to) {
            return Ok(());
        }
        if let Some(from) = self.selected {
            if !self.panes[from].hooks.should_unselect() {
                self.notices.push(PaneNotice::CancelUnselect);
                return Err(PaneError::Refused);
            }
            self.notices.push(PaneNotice::DoUnselect);
            self.panes[from].hooks.will_unselect();
            self.commit(from);
            self.selected = None;
            self.panes[from].hooks.did_unselect();
        }
        self.panes[to].hooks.will_select();
        self.selected = Some(to);
        self.panes[to].hooks.did_select();
        self.notices.push(PaneNotice::SwitchToPane);
        Ok(())
    }

    /// Records what the user typed in a text field of the selected pane. Returns false when no pane is selected.
    pub fn edit_field(&mut self, field: u64, value: &str) -> bool {
        let Some(i) = self.selected else { return false };
        let pending = &mut self.panes[i].pending;
        match pending.iter_mut().find(|(f, _)| *f == field) {
            Some(entry) => entry.1 = value.to_string(),
            None => pending.push((field, value.to_string())),
        }
        true
    }

    /// The committed value of a text field of a pane.
    pub fn saved_field(&self, pane: &str, field: u64) -> Option<&str> {
        let i = self.index_of(pane)?;
        self.panes[i].saved.iter().find(|(f, _)| *f == field).map(|(_, v)| v.as_str())
    }

    /// Commits the pending edits of a pane when its auto save is on, and drops them otherwise.
    fn commit(&mut self, i: usize) {
        let entry = &mut self.panes[i];
        let pending = std::mem::take(&mut entry.pending);
        if !entry.spec.auto_save_text_fields {
            return;
        }
        for (field, value) in pending {
            match entry.saved.iter_mut().find(|(f, _)| *f == field) {
                Some(saved) => saved.1 = value,
                None => entry.saved.push((field, value)),
            }
        }
    }

    /// Replaces the Help menu items of a pane. `UpdateHelpMenu` is queued when that pane is the one shown.
    pub fn set_help(&mut self, name: &str, items: Vec<HelpItem>) -> Result<(), PaneError> {
        let i = self.index_of(name).ok_or(PaneError::Unknown)?;
        self.panes[i].spec.help = items;
        if self.selected == Some(i) {
            self.notices.push(PaneNotice::UpdateHelpMenu);
        }
        Ok(())
    }

    /// The Help menu items to show: those of the selected pane, none when no pane is selected.
    pub fn help_menu(&self) -> &[HelpItem] {
        match self.selected {
            Some(i) => &self.panes[i].spec.help,
            None => &[],
        }
    }

    /// Takes the queued notices, oldest first.
    pub fn drain_notices(&mut self) -> Vec<PaneNotice> {
        std::mem::take(&mut self.notices)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    /// Records each lifecycle step into a shared log and vetoes leaving while `veto` is set.
    struct Recorder {
        name: &'static str,
        log: Rc<RefCell<Vec<String>>>,
        veto: Rc<Cell<bool>>,
    }

    impl PaneHooks for Recorder {
        fn should_unselect(&mut self) -> bool {
            self.log.borrow_mut().push(format!("{}.should", self.name));
            !self.veto.get()
        }
        fn will_select(&mut self) {
            self.log.borrow_mut().push(format!("{}.will_select", self.name));
        }
        fn did_select(&mut self) {
            self.log.borrow_mut().push(format!("{}.did_select", self.name));
        }
        fn will_unselect(&mut self) {
            self.log.borrow_mut().push(format!("{}.will_unselect", self.name));
        }
        fn did_unselect(&mut self) {
            self.log.borrow_mut().push(format!("{}.did_unselect", self.name));
        }
    }

    fn host_with(names: &[&'static str]) -> (PaneHost, Rc<RefCell<Vec<String>>>, Rc<Cell<bool>>) {
        let log = Rc::new(RefCell::new(Vec::new()));
        let veto = Rc::new(Cell::new(false));
        let mut host = PaneHost::new();
        for &name in names {
            let hooks = Recorder { name, log: log.clone(), veto: veto.clone() };
            assert!(host.add(PaneSpec::new(name, name), Box::new(hooks)));
        }
        (host, log, veto)
    }

    #[test]
    fn switching_runs_the_lifecycle_in_order() {
        let (mut host, log, _) = host_with(&["a", "b"]);
        host.select("a").unwrap();
        log.borrow_mut().clear();
        host.select("b").unwrap();
        assert_eq!(host.selected(), Some("b"));
        assert_eq!(
            *log.borrow(),
            ["a.should", "a.will_unselect", "a.did_unselect", "b.will_select", "b.did_select"]
        );
    }

    #[test]
    fn a_veto_keeps_the_pane_and_queues_cancel() {
        let (mut host, _, veto) = host_with(&["a", "b"]);
        host.select("a").unwrap();
        host.drain_notices();
        veto.set(true);
        assert_eq!(host.select("b"), Err(PaneError::Refused));
        assert_eq!(host.selected(), Some("a"));
        assert_eq!(host.drain_notices(), [PaneNotice::CancelUnselect]);
        veto.set(false);
        assert_eq!(host.select("b"), Ok(()));
    }

    #[test]
    fn notices_are_queued_in_order_and_drained_once() {
        let (mut host, _, _) = host_with(&["a", "b"]);
        assert_eq!(host.drain_notices(), [PaneNotice::IsAvailable]);
        host.select("a").unwrap();
        host.select("b").unwrap();
        assert_eq!(
            host.drain_notices(),
            [PaneNotice::SwitchToPane, PaneNotice::DoUnselect, PaneNotice::SwitchToPane]
        );
        assert!(host.drain_notices().is_empty());
    }

    #[test]
    fn unknown_duplicate_and_repeated_selections() {
        let (mut host, log, _) = host_with(&["a"]);
        assert_eq!(host.select("zzz"), Err(PaneError::Unknown));
        assert!(!host.add(PaneSpec::new("a", "again"), Box::new(Recorder {
            name: "a",
            log: log.clone(),
            veto: Rc::new(Cell::new(false)),
        })));
        host.select("a").unwrap();
        host.drain_notices();
        host.select("a").unwrap();
        assert!(host.drain_notices().is_empty());
    }

    #[test]
    fn text_fields_commit_only_when_auto_save_is_on() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let veto = Rc::new(Cell::new(false));
        let mut host = PaneHost::new();
        let keep = PaneSpec::new("keep", "Keep");
        let mut dropping = PaneSpec::new("dropping", "Dropping");
        dropping.auto_save_text_fields = false;
        host.add(keep, Box::new(Recorder { name: "keep", log: log.clone(), veto: veto.clone() }));
        host.add(dropping, Box::new(Recorder { name: "dropping", log, veto }));
        host.select("keep").unwrap();
        assert!(host.edit_field(7, "typed"));
        host.select("dropping").unwrap();
        assert_eq!(host.saved_field("keep", 7), Some("typed"));
        host.edit_field(9, "lost");
        host.select("keep").unwrap();
        assert_eq!(host.saved_field("dropping", 9), None);
    }

    #[test]
    fn edits_need_a_selected_pane_and_keep_the_latest_value() {
        let (mut host, _, _) = host_with(&["a", "b"]);
        assert!(!host.edit_field(1, "nothing selected"));
        host.select("a").unwrap();
        host.edit_field(1, "first");
        host.edit_field(1, "second");
        host.select("b").unwrap();
        assert_eq!(host.saved_field("a", 1), Some("second"));
    }

    #[test]
    fn help_menu_shows_the_selected_pane_and_updates_when_it_changes() {
        let (mut host, _, _) = host_with(&["a", "b"]);
        let item = HelpItem { title: "Network Help".into(), anchor: Some("net".into()) };
        host.set_help("a", vec![item.clone()]).unwrap();
        assert!(host.help_menu().is_empty());
        assert!(host.drain_notices().iter().all(|n| *n != PaneNotice::UpdateHelpMenu));
        host.select("a").unwrap();
        host.drain_notices();
        assert_eq!(host.help_menu(), [item]);
        host.set_help("a", Vec::new()).unwrap();
        assert_eq!(host.drain_notices(), [PaneNotice::UpdateHelpMenu]);
        assert!(host.help_menu().is_empty());
        assert_eq!(host.set_help("zzz", Vec::new()), Err(PaneError::Unknown));
    }
}
