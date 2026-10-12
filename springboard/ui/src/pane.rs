//! Preference panes: a host (the Settings app) shows one pane at a time. A pane runs through a lifecycle (will and
//! did select, will and did unselect), can refuse to be left (should unselect), commits its text fields when it is
//! left (auto save), and adds items to the Help menu while it is shown. The host loads each pane's main view once,
//! on its first selection. `PaneHost` drives the panes and queues `PaneNotice`s for observers to take in order.
//! `parse_manifest` reads a pane's description from a text manifest (name, title, focus hints, auto save and Help
//! menu items), the counterpart of a pane bundle's property list.

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

/// Why a pane manifest could not be read. Line numbers start at 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ManifestError {
    /// The manifest has no `name` (or it is empty).
    MissingName,
    /// The manifest has no `title` (or it is empty).
    MissingTitle,
    /// A line is not `key = value`.
    Line(usize),
    /// A line uses a key the manifest does not define.
    UnknownKey(usize),
    /// A value does not fit its key: a bool that is not true or false, a number that is not a view handle, or an
    /// empty Help menu title.
    BadValue(usize),
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
    /// Builds the pane's main view and returns its handle. Called once, before the first selection. `None` means the
    /// view could not be built, so the host asks again on the next selection.
    fn load_main_view(&mut self) -> Option<u64> {
        None
    }
    /// Called once the main view from `load_main_view` is assigned to the pane.
    fn main_view_did_load(&mut self) {}
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
    /// The main view handle, once loaded.
    view: Option<u64>,
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
        self.panes.push(Entry { spec, hooks, pending: Vec::new(), saved: Vec::new(), view: None });
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
        if self.panes[to].view.is_none() {
            if let Some(view) = self.panes[to].hooks.load_main_view() {
                self.panes[to].view = Some(view);
                self.panes[to].hooks.main_view_did_load();
            }
        }
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

    /// The main view handle of a pane, once it has loaded.
    pub fn main_view(&self, name: &str) -> Option<u64> {
        self.index_of(name).and_then(|i| self.panes[i].view)
    }

    /// The keyboard focus chain of a pane over the views it owns, in the order given by `views`. The chain starts at
    /// the initial key view and walks forward, wrapping at the end, to the last key view, both inclusive. A missing
    /// or unknown initial view starts at the first view, and a missing or unknown last view ends at the last one.
    /// `None` when no pane has that name.
    pub fn key_loop(&self, name: &str, views: &[u64]) -> Option<Vec<u64>> {
        let spec = self.spec(name)?;
        let n = views.len();
        if n == 0 {
            return Some(Vec::new());
        }
        let start = spec.initial_key_view.and_then(|v| views.iter().position(|&x| x == v)).unwrap_or(0);
        let end = spec.last_key_view.and_then(|v| views.iter().position(|&x| x == v)).unwrap_or(n - 1);
        let len = (end + n - start) % n + 1;
        Some((0..len).map(|k| views[(start + k) % n]).collect())
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

/// Reads a pane description from a text manifest. Each non-empty line not starting with `#` is `key = value`. Keys:
/// `name`, `title`, `auto_save_text_fields` (true or false), `initial_key_view` and `last_key_view` (view handles),
/// and `help`, repeatable, as `Title` or `Title | anchor`. Missing `auto_save_text_fields` means on.
pub fn parse_manifest(text: &str) -> Result<PaneSpec, ManifestError> {
    let mut name: Option<String> = None;
    let mut title: Option<String> = None;
    let mut auto_save = true;
    let mut initial: Option<u64> = None;
    let mut last: Option<u64> = None;
    let mut help: Vec<HelpItem> = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let n = i + 1;
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = line.split_once('=').ok_or(ManifestError::Line(n))?;
        let (key, value) = (key.trim(), value.trim());
        match key {
            "name" => name = Some(value.to_string()),
            "title" => title = Some(value.to_string()),
            "auto_save_text_fields" => {
                auto_save = match value {
                    "true" => true,
                    "false" => false,
                    _ => return Err(ManifestError::BadValue(n)),
                }
            }
            "initial_key_view" => initial = Some(value.parse().map_err(|_| ManifestError::BadValue(n))?),
            "last_key_view" => last = Some(value.parse().map_err(|_| ManifestError::BadValue(n))?),
            "help" => {
                let (item_title, anchor) = match value.split_once('|') {
                    Some((t, a)) => (t.trim(), Some(a.trim().to_string()).filter(|a| !a.is_empty())),
                    None => (value, None),
                };
                if item_title.is_empty() {
                    return Err(ManifestError::BadValue(n));
                }
                help.push(HelpItem { title: item_title.to_string(), anchor });
            }
            _ => return Err(ManifestError::UnknownKey(n)),
        }
    }
    let name = name.filter(|s| !s.is_empty()).ok_or(ManifestError::MissingName)?;
    let title = title.filter(|s| !s.is_empty()).ok_or(ManifestError::MissingTitle)?;
    Ok(PaneSpec {
        name,
        title,
        initial_key_view: initial,
        last_key_view: last,
        auto_save_text_fields: auto_save,
        help,
    })
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

    /// Counts loads of the main view and records the order of the load steps.
    struct ViewPane {
        loads: Rc<Cell<u32>>,
        log: Rc<RefCell<Vec<&'static str>>>,
    }

    impl PaneHooks for ViewPane {
        fn will_select(&mut self) {
            self.log.borrow_mut().push("will_select");
        }
        fn did_select(&mut self) {
            self.log.borrow_mut().push("did_select");
        }
        fn load_main_view(&mut self) -> Option<u64> {
            self.loads.set(self.loads.get() + 1);
            self.log.borrow_mut().push("load");
            Some(42)
        }
        fn main_view_did_load(&mut self) {
            self.log.borrow_mut().push("did_load");
        }
    }

    #[test]
    fn main_view_loads_once_between_will_select_and_did_select() {
        let loads = Rc::new(Cell::new(0));
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut host = PaneHost::new();
        host.add(PaneSpec::new("v", "View"), Box::new(ViewPane { loads: loads.clone(), log: log.clone() }));
        assert_eq!(host.main_view("v"), None);
        host.select("v").unwrap();
        assert_eq!(host.main_view("v"), Some(42));
        assert_eq!(*log.borrow(), ["will_select", "load", "did_load", "did_select"]);
        host.select("v").unwrap();
        assert_eq!(loads.get(), 1);
        assert_eq!(host.main_view("missing"), None);
    }

    #[test]
    fn key_loop_walks_from_initial_to_last_and_wraps() {
        let mut host = PaneHost::new();
        let mut spec = PaneSpec::new("k", "Keys");
        spec.initial_key_view = Some(30);
        spec.last_key_view = Some(20);
        let hooks = Recorder { name: "k", log: Rc::new(RefCell::new(Vec::new())), veto: Rc::new(Cell::new(false)) };
        host.add(spec, Box::new(hooks));
        assert_eq!(host.key_loop("k", &[10, 20, 30, 40]), Some(vec![30, 40, 10, 20]));
        assert_eq!(host.key_loop("nope", &[10]), None);
        assert_eq!(host.key_loop("k", &[]), Some(vec![]));
    }

    #[test]
    fn key_loop_falls_back_to_first_and_last_views() {
        let mut host = PaneHost::new();
        let mut spec = PaneSpec::new("k", "Keys");
        spec.initial_key_view = Some(99);
        let hooks = Recorder { name: "k", log: Rc::new(RefCell::new(Vec::new())), veto: Rc::new(Cell::new(false)) };
        host.add(spec, Box::new(hooks));
        // An unknown initial view starts at the first view; with no last view the chain ends at the last one.
        assert_eq!(host.key_loop("k", &[1, 2, 3]), Some(vec![1, 2, 3]));
        assert_eq!(host.key_loop("k", &[7]), Some(vec![7]));
    }

    #[test]
    fn manifest_reads_every_key() {
        let text = "# pane
name = network
title = Network
auto_save_text_fields = false
                    initial_key_view = 5
last_key_view = 9
help = Network Help | net
help = Troubleshooting
";
        let spec = parse_manifest(text).unwrap();
        assert_eq!(spec.name, "network");
        assert_eq!(spec.title, "Network");
        assert!(!spec.auto_save_text_fields);
        assert_eq!((spec.initial_key_view, spec.last_key_view), (Some(5), Some(9)));
        assert_eq!(
            spec.help,
            [
                HelpItem { title: "Network Help".into(), anchor: Some("net".into()) },
                HelpItem { title: "Troubleshooting".into(), anchor: None },
            ]
        );
        let minimal = parse_manifest("name = a
title = A
").unwrap();
        assert!(minimal.auto_save_text_fields);
        assert!(minimal.help.is_empty());
    }

    #[test]
    fn manifest_reports_the_line_of_each_error() {
        assert_eq!(parse_manifest("title = A
"), Err(ManifestError::MissingName));
        assert_eq!(parse_manifest("name = a
"), Err(ManifestError::MissingTitle));
        assert_eq!(parse_manifest("name =
title = A
"), Err(ManifestError::MissingName));
        assert_eq!(parse_manifest("name = a
title = A
bogus
"), Err(ManifestError::Line(3)));
        assert_eq!(parse_manifest("name = a
title = A
colour = red
"), Err(ManifestError::UnknownKey(3)));
        assert_eq!(
            parse_manifest("name = a
title = A
auto_save_text_fields = yes
"),
            Err(ManifestError::BadValue(3))
        );
        assert_eq!(parse_manifest("name = a
title = A
last_key_view = x
"), Err(ManifestError::BadValue(3)));
        assert_eq!(parse_manifest("name = a
title = A
help = | net
"), Err(ManifestError::BadValue(3)));
    }
}
