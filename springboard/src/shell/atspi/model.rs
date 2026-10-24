//! The AT-SPI2 view of the app-control tree: pure data and mappings, no D-Bus. `bridge.rs` exports what this builds.
//! parity: the `NativeAccessibility` element hierarchy (`accessibilityChildren`, `accessibilityParent`, role, state).
use bifrost_proto::{AxMeta, AxText, Node, NodeExt, Role, UiRect as Rect};

/// `AtspiRole` values (atspi-constants.h).
pub mod role {
    pub const CHECK_BOX: u32 = 7;
    pub const FRAME: u32 = 23;
    pub const LABEL: u32 = 29;
    pub const LIST: u32 = 31;
    pub const LIST_ITEM: u32 = 32;
    pub const MENU_ITEM: u32 = 35;
    pub const PAGE_TAB_LIST: u32 = 38;
    pub const PROGRESS_BAR: u32 = 42;
    pub const PUSH_BUTTON: u32 = 43;
    pub const SLIDER: u32 = 51;
    pub const TOGGLE_BUTTON: u32 = 62;
    pub const APPLICATION: u32 = 75;
    pub const ENTRY: u32 = 79;
    pub const PASSWORD_TEXT: u32 = 40;
}

/// `AtspiStateType` bit positions.
pub mod state {
    pub const ACTIVE: u32 = 1;
    pub const CHECKED: u32 = 4;
    pub const EDITABLE: u32 = 7;
    pub const ENABLED: u32 = 8;
    pub const FOCUSABLE: u32 = 11;
    pub const FOCUSED: u32 = 12;
    pub const SENSITIVE: u32 = 24;
    pub const SHOWING: u32 = 25;
    pub const SINGLE_LINE: u32 = 26;
    pub const VISIBLE: u32 = 30;
    pub const CHECKABLE: u32 = 41;
}

pub fn atspi_role(r: Role) -> u32 {
    match r {
        Role::Button => role::PUSH_BUTTON,
        Role::TextField => role::ENTRY,
        Role::List => role::LIST,
        Role::Row => role::LIST_ITEM,
        Role::Toggle => role::TOGGLE_BUTTON,
        Role::Checkbox => role::CHECK_BOX,
        Role::Slider => role::SLIDER,
        Role::SegmentedControl => role::PAGE_TAB_LIST,
        Role::StaticText => role::LABEL,
        Role::MenuItem => role::MENU_ITEM,
        Role::Window => role::FRAME,
        Role::ProgressIndicator => role::PROGRESS_BAR,
    }
}

/// `Accessible.GetRoleName` (the non-localized AT-SPI role name).
pub fn role_name(r: Role) -> &'static str {
    match r {
        Role::Button => "push button",
        Role::TextField => "entry",
        Role::List => "list",
        Role::Row => "list item",
        Role::Toggle => "toggle button",
        Role::Checkbox => "check box",
        Role::Slider => "slider",
        Role::SegmentedControl => "page tab list",
        Role::StaticText => "label",
        Role::MenuItem => "menu item",
        Role::Window => "frame",
        Role::ProgressIndicator => "progress bar",
    }
}

/// One accessible element. `parent: None` means a direct child of the app's window.
#[derive(Clone, Debug, PartialEq)]
pub struct SNode {
    pub id: u64,
    pub parent: Option<u64>,
    pub role: Role,
    pub name: String,
    pub value: String,
    pub bounds: Rect,
    pub enabled: bool,
    pub focused: bool,
    pub checked: Option<bool>,
    /// `accessibilityTitle` (empty when the app sent none)
    pub title: String,
    /// `accessibilityHelp`
    pub help: String,
    /// caret, selection and visible range of a text element; `None` until the app reports them
    pub text: Option<AxText>,
    /// the element's named custom actions (`accessibilityCustomActions`, `ToShell::AxActions`), listed after the standard ones
    pub actions: Vec<String>,
}

impl SNode {
    /// The AT-SPI role: a secure text field is `PASSWORD_TEXT` so a screen reader says "password" and echoes no characters.
    pub fn atspi_role(&self) -> u32 {
        if self.role == Role::TextField && self.text.is_some_and(|t| t.secure) {
            role::PASSWORD_TEXT
        } else {
            atspi_role(self.role)
        }
    }
    /// `Accessible.Name`: `accessibilityLabel`, else `accessibilityTitle`.
    pub fn accessible_name(&self) -> &str {
        if self.name.is_empty() { &self.title } else { &self.name }
    }
}

/// One running app: its window frame and elements.
#[derive(Clone, Debug, PartialEq)]
pub struct AppSnap {
    pub slot: u8,
    pub app_id: String,
    pub name: String,
    pub nodes: Vec<SNode>,
}

/// Joins the flat `Controls` nodes with the optional `ControlsTree` entries. A parent that is missing, is the node itself
/// or leads into a cycle is dropped, so the node becomes a window child (old flat packets all come out this way).
/// `Toggle` and `Checkbox` nodes without a tree entry read their checked state from the value text ("on", "1", "checked").
pub fn build(nodes: &[Node], ext: &[NodeExt]) -> Vec<SNode> {
    build_ax(nodes, ext, &[], &[])
}

/// [`build`] plus the text attributes (`ToShell::AxText`) and the title/help split (`ToShell::AxMeta`). A secure text
/// field's value is replaced by one bullet per character here, so nothing downstream can ever read its content.
pub fn build_ax(nodes: &[Node], ext: &[NodeExt], texts: &[AxText], metas: &[AxMeta]) -> Vec<SNode> {
    let find = |id: u64| ext.iter().find(|e| e.id == id);
    let mut out: Vec<SNode> = nodes
        .iter()
        .map(|n| {
            let e = find(n.id);
            let guess = matches!(n.role, Role::Toggle | Role::Checkbox)
                .then(|| matches!(n.value.to_lowercase().as_str(), "on" | "1" | "true" | "checked" | "selected"));
            SNode {
                id: n.id,
                parent: e.and_then(|e| e.parent).filter(|p| *p != n.id && nodes.iter().any(|m| m.id == *p)),
                role: n.role,
                name: n.label.clone(),
                value: n.value.clone(),
                bounds: n.bounds,
                enabled: n.enabled,
                focused: e.is_some_and(|e| e.focused),
                checked: e.and_then(|e| e.checked).or(guess),
                title: metas.iter().find(|m| m.id == n.id).map(|m| m.title.clone()).unwrap_or_default(),
                help: metas.iter().find(|m| m.id == n.id).map(|m| m.help.clone()).unwrap_or_default(),
                text: (n.role == Role::TextField).then(|| texts.iter().find(|t| t.id == n.id).copied()).flatten(),
                actions: Vec::new(),
            }
        })
        .collect();
    for n in out.iter_mut() {
        if let Some(t) = n.text.filter(|t| t.secure) {
            n.value = bifrost_ui::ax::masked(t.len);
        }
    }
    let mut cyclic = Vec::new();
    for node in &out {
        let mut path = Vec::new();
        let mut at = Some(node.id);
        while let Some(id) = at {
            if let Some(start) = path.iter().position(|seen| *seen == id) {
                cyclic.extend_from_slice(&path[start..]);
                break;
            }
            path.push(id);
            at = out.iter().find(|parent| parent.id == id).and_then(|parent| parent.parent);
        }
    }
    for node in &mut out {
        if cyclic.contains(&node.id) {
            node.parent = None;
        }
    }
    out
}

/// One change of a text element's content as AT-SPI `text-changed` reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextEdit {
    /// `true` is `insert`, `false` is `delete`
    pub insert: bool,
    pub at: i32,
    pub len: i32,
    pub text: String,
}

/// The edits turning `old` into `new`, by common prefix and suffix (char offsets): a delete, then an insert for a replace.
/// Empty when nothing changed.
pub fn text_edits(old: &str, new: &str) -> Vec<TextEdit> {
    let (o, n): (Vec<char>, Vec<char>) = (old.chars().collect(), new.chars().collect());
    let pre = o.iter().zip(&n).take_while(|(a, b)| a == b).count();
    let suf = o[pre..].iter().rev().zip(n[pre..].iter().rev()).take_while(|(a, b)| a == b).count();
    let (gone, came) = (&o[pre..o.len() - suf], &n[pre..n.len() - suf]);
    let mut out = Vec::new();
    if !gone.is_empty() {
        out.push(TextEdit { insert: false, at: pre as i32, len: gone.len() as i32, text: gone.iter().collect() });
    }
    if !came.is_empty() {
        out.push(TextEdit { insert: true, at: pre as i32, len: came.len() as i32, text: came.iter().collect() });
    }
    out
}

/// Ids of the children of `parent` (`None` = the window), in packet order.
pub fn children(nodes: &[SNode], parent: Option<u64>) -> Vec<u64> {
    nodes.iter().filter(|n| n.parent == parent).map(|n| n.id).collect()
}

/// The `GetState` bit set as two `u32` words (bit n of the 64-bit set lives in word n/32).
pub fn states(n: &SNode) -> [u32; 2] {
    let mut bits = 0u64;
    let mut set = |b: u32| bits |= 1u64 << b;
    set(state::VISIBLE);
    set(state::SHOWING);
    if n.enabled {
        set(state::ENABLED);
        set(state::SENSITIVE);
    }
    if !matches!(n.role, Role::StaticText | Role::Window | Role::List) && n.enabled {
        set(state::FOCUSABLE);
    }
    if n.focused {
        set(state::FOCUSED);
    }
    if n.role == Role::TextField {
        set(state::EDITABLE);
        set(state::SINGLE_LINE);
    }
    if let Some(on) = n.checked {
        set(state::CHECKABLE);
        if on {
            set(state::CHECKED);
        }
    }
    [bits as u32, (bits >> 32) as u32]
}

/// States for an app window frame.
pub fn window_states() -> [u32; 2] {
    let bits = [state::ACTIVE, state::ENABLED, state::SENSITIVE, state::SHOWING, state::VISIBLE].iter().fold(0u64, |a, b| a | 1u64 << b);
    [bits as u32, (bits >> 32) as u32]
}

pub const ROOT_PATH: &str = "/org/a11y/atspi/accessible/root";
pub const NULL_PATH: &str = "/org/a11y/atspi/null";

pub fn app_path(slot: u8) -> String {
    format!("{ROOT_PATH}/app{slot}")
}

pub fn node_path(slot: u8, id: u64) -> String {
    format!("{ROOT_PATH}/app{slot}/n{id}")
}

/// The AT-SPI signal an element notification becomes (`bridge::Bridge::notify`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoteSignal {
    /// `Object:StateChanged` "focused" on the element
    Focused,
    /// `Object:PropertyChange` "accessible-value" on the element
    Value,
    /// `Object:SelectionChanged` on the element
    Selection,
    /// `Object:ChildrenChanged` "add" on the app: the layout moved
    Layout,
    /// `Object:ChildrenChanged` "remove" on the app: an element went away
    Removed,
}

/// The signal each notification maps to. Total: every notification has one.
pub fn note_signal(n: bifrost_ui::ax::AxNotification) -> NoteSignal {
    use bifrost_ui::ax::AxNotification as N;
    match n {
        N::FocusedUIElementChanged => NoteSignal::Focused,
        N::ValueChanged => NoteSignal::Value,
        N::SelectedChildrenChanged => NoteSignal::Selection,
        N::LayoutChanged => NoteSignal::Layout,
        N::ElementDestroyed => NoteSignal::Removed,
    }
}

/// What an object path refers to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    Root,
    App(u8),
    Node(u8, u64),
}

pub fn parse_path(p: &str) -> Option<Key> {
    let rest = p.strip_prefix(ROOT_PATH)?;
    if rest.is_empty() {
        return Some(Key::Root);
    }
    let rest = rest.strip_prefix("/app")?;
    let (slot, tail) = match rest.split_once('/') {
        Some((s, t)) => (s, Some(t)),
        None => (rest, None),
    };
    let slot = slot.parse().ok()?;
    match tail {
        None => Some(Key::App(slot)),
        Some(t) => Some(Key::Node(slot, t.strip_prefix('n')?.parse().ok()?)),
    }
}

pub fn key_path(k: Key) -> String {
    match k {
        Key::Root => ROOT_PATH.into(),
        Key::App(s) => app_path(s),
        Key::Node(s, id) => node_path(s, id),
    }
}

/// The slot of the pseudo-application that holds the shell's own chrome (menu bar, dock, search field). Apps use slots below it.
pub const CHROME_SLOT: u8 = 250;

/// The shell chrome as an application snapshot, so a screen reader reads the menu bar, the dock and the search field like any app.
/// `menus` is (menu title, [(row title, enabled)]); `dock` is (app name, open). Ids are positional and stable while the lists
/// keep their order: the menu bar is 1, a menu 100 + 100 * index, its rows follow it; the dock is 2, its apps 1000 + index;
/// the search field is 3. Menus nest their rows (`parent`), the bar nests the menus.
pub fn chrome_snap(menus: &[(String, Vec<(String, bool)>)], dock: &[(String, bool)]) -> AppSnap {
    let bounds = Rect { x: 0., y: 0., w: 0., h: 0. };
    let node = |id: u64, parent: Option<u64>, role: Role, name: &str, value: &str, enabled: bool| SNode {
        id, parent, role, name: name.into(), value: value.into(), bounds, enabled, focused: false, checked: None, title: String::new(), help: String::new(), text: None, actions: Vec::new(),
    };
    let mut nodes = vec![node(1, None, Role::List, "Menu Bar", "", true)];
    for (m, (title, rows)) in menus.iter().enumerate() {
        let mid = 100 + 100 * m as u64;
        nodes.push(node(mid, Some(1), Role::MenuItem, title, "", true));
        for (i, (row, enabled)) in rows.iter().enumerate().take(98) {
            nodes.push(node(mid + 1 + i as u64, Some(mid), Role::MenuItem, row, "", *enabled));
        }
    }
    nodes.push(node(2, None, Role::List, "Dock", "", true));
    for (i, (name, open)) in dock.iter().enumerate() {
        nodes.push(node(1000 + i as u64, Some(2), Role::Button, name, if *open { "open" } else { "" }, true));
    }
    nodes.push(node(3, None, Role::TextField, "Search", "", true));
    AppSnap { slot: CHROME_SLOT, app_id: "shell".into(), name: "BifrOSt Shell".into(), nodes }
}

/// What a press on a shell chrome element (`chrome_snap`, `add_widgets`) does: choose row `.1` of menu `.0`, open the
/// `.0`-th dock app, open the search field, or run button `.1` of widget row `.0`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChromePress {
    MenuRow(usize, usize),
    DockApp(usize),
    Search,
    Widget(usize, usize),
}

/// The first id of the dock's apps; the range ends where the widget rows start.
const DOCK_BASE: u64 = 1000;
/// The first widget row id; row `i` is `WIDGET_BASE + 10 * i`, its buttons the nine ids after it.
const WIDGET_BASE: u64 = 5000;

/// The press target of chrome element `id`, from the positional ids `chrome_snap` and `add_widgets` give out. The bar, a
/// menu title, the dock, the Widgets list and a widget row are containers and press nothing.
pub fn chrome_target(id: u64) -> Option<ChromePress> {
    match id {
        3 => Some(ChromePress::Search),
        100..=999 if id % 100 != 0 => Some(ChromePress::MenuRow((id / 100 - 1) as usize, (id % 100 - 1) as usize)),
        DOCK_BASE..=4999 => Some(ChromePress::DockApp((id - DOCK_BASE) as usize)),
        WIDGET_BASE.. => {
            let (row, slot) = (((id - WIDGET_BASE) / 10) as usize, (id - WIDGET_BASE) % 10);
            (slot != 0 && row < MAX_WIDGET_ROWS).then(|| ChromePress::Widget(row, slot as usize - 1))
        }
        _ => None,
    }
}

/// One widget row as the screen reader gets it: its title, its text as the value, the spoken label (title, value and
/// an actions hint) and its buttons as `(label, toggle state, enabled)`.
#[derive(Clone, Debug, PartialEq)]
pub struct WidgetSnap {
    pub title: String,
    pub value: String,
    pub label: String,
    pub buttons: Vec<(String, Option<bool>, bool)>,
}

/// The node id of the Widgets list in the shell chrome tree.
const WIDGETS_NODE: u64 = 4;
/// The most widget rows exported; each row's buttons take the ids just after its own.
const MAX_WIDGET_ROWS: usize = 64;

/// Adds the Notification Centre widgets to the shell chrome: a Widgets list under the application with one static
/// text per widget (name = title, value = its text, help = the spoken label) and its buttons and toggles below it.
pub fn add_widgets(snap: &mut AppSnap, widgets: &[WidgetSnap]) {
    if widgets.is_empty() {
        return;
    }
    let bounds = Rect { x: 0., y: 0., w: 0., h: 0. };
    let node = |id: u64, parent: Option<u64>, role: Role, name: &str, value: &str, enabled: bool, checked: Option<bool>, help: &str| SNode {
        id, parent, role, name: name.into(), value: value.into(), bounds, enabled, focused: false, checked, title: String::new(), help: help.into(), text: None, actions: Vec::new(),
    };
    snap.nodes.push(node(WIDGETS_NODE, None, Role::List, "Widgets", "", true, None, ""));
    for (i, w) in widgets.iter().enumerate().take(MAX_WIDGET_ROWS) {
        let wid = WIDGET_BASE + 10 * i as u64;
        snap.nodes.push(node(wid, Some(WIDGETS_NODE), Role::StaticText, &w.title, &w.value, true, None, &w.label));
        for (j, (label, on, enabled)) in w.buttons.iter().enumerate().take(9) {
            let role = if on.is_some() { Role::Toggle } else { Role::Button };
            snap.nodes.push(node(wid + 1 + j as u64, Some(wid), role, label, "", *enabled, *on, ""));
        }
    }
}

/// The AT-SPI `Action` names of an element: `press`, and for a numeric one `increment` and `decrement`.
pub fn action_names(numeric: bool) -> Vec<&'static str> {
    if numeric { vec!["press", "increment", "decrement"] } else { vec!["press"] }
}

/// Gives each node the custom actions its app named for it (`ToShell::AxActions`, already cleaned by the host). A set for
/// an id that is not in `nodes` is ignored, so stale actions never outlive their element.
pub fn with_actions(mut nodes: Vec<SNode>, sets: &[bifrost_ui::ax_action::AxActionSet]) -> Vec<SNode> {
    for n in nodes.iter_mut() {
        if let Some(s) = sets.iter().find(|s| s.id == n.id) {
            n.actions = s.names.clone();
        }
    }
    nodes
}

/// What `Action.DoAction(index)` asks for: index 0 is press (only on an element that can be pressed), then `increment` and
/// `decrement` on a numeric one, then the element's custom actions by name. `None` for an index past the list.
pub fn action_at(n: &SNode, actionable: bool, index: i32) -> Option<bifrost_proto::AxAction> {
    use bifrost_proto::AxAction;
    let numeric = n.role == Role::Slider;
    let i = usize::try_from(index).ok()?;
    let standard = bifrost_ui::ax_action::standard_count(numeric);
    if i >= standard {
        return n.actions.get(i - standard).map(|name| AxAction::Custom(name.clone()));
    }
    match action_names(numeric)[i] {
        "press" if actionable => Some(AxAction::Press),
        "increment" => Some(AxAction::Increment),
        "decrement" => Some(AxAction::Decrement),
        _ => None,
    }
}

/// A numeric value read from the node's value text: "0.4" is 0.4 of 1.0, "40%" is 40 of 100, and "3/10"
/// (`NativeAccessibilityValue` with an explicit `maxValue`) is 3 of 10.
pub fn numeric(value: &str) -> Option<(f64, f64)> {
    let v = value.trim();
    if let Some((a, b)) = v.split_once('/') {
        let (x, max): (f64, f64) = (a.trim().parse().ok()?, b.trim().parse().ok()?);
        return (max > 0.0).then_some((x, max));
    }
    match v.strip_suffix('%') {
        Some(p) => p.trim().parse().ok().map(|x| (x, 100.0)),
        None => v.parse().ok().map(|x: f64| (x, if x > 1.0 { x.max(100.0) } else { 1.0 })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_element_notification_names_one_signal() {
        use bifrost_ui::ax::AxNotification as N;
        assert_eq!(note_signal(N::FocusedUIElementChanged), NoteSignal::Focused);
        assert_eq!(note_signal(N::ValueChanged), NoteSignal::Value);
        assert_eq!(note_signal(N::SelectedChildrenChanged), NoteSignal::Selection);
        assert_eq!(note_signal(N::LayoutChanged), NoteSignal::Layout);
        assert_eq!(note_signal(N::ElementDestroyed), NoteSignal::Removed);
    }

    fn node(id: u64, role: Role, value: &str) -> Node {
        Node { id, role, label: format!("n{id}"), value: value.into(), bounds: Rect { x: 1., y: 2., w: 30., h: 40. }, enabled: true }
    }

    #[test]
    fn every_role_maps_to_the_atspi_value() {
        let pairs = [
            (Role::Button, 43, "push button"), (Role::TextField, 79, "entry"), (Role::List, 31, "list"), (Role::Row, 32, "list item"),
            (Role::Toggle, 62, "toggle button"), (Role::Checkbox, 7, "check box"), (Role::Slider, 51, "slider"),
            (Role::SegmentedControl, 38, "page tab list"), (Role::StaticText, 29, "label"), (Role::MenuItem, 35, "menu item"), (Role::Window, 23, "frame"), (Role::ProgressIndicator, 42, "progress bar"),
        ];
        for (r, n, name) in pairs {
            assert_eq!(atspi_role(r), n);
            assert_eq!(role_name(r), name);
        }
    }

    #[test]
    fn flat_packets_become_window_children() {
        let nodes = vec![node(1, Role::List, ""), node(2, Role::Row, "")];
        let t = build(&nodes, &[]);
        assert_eq!(children(&t, None), vec![1, 2]);
    }

    #[test]
    fn tree_entries_nest_rows_under_their_list() {
        let nodes = vec![node(1, Role::List, ""), node(2, Role::Row, ""), node(3, Role::Row, "")];
        let ext = [NodeExt { id: 2, parent: Some(1), focused: false, checked: None }, NodeExt { id: 3, parent: Some(1), focused: true, checked: None }];
        let t = build(&nodes, &ext);
        assert_eq!(children(&t, None), vec![1]);
        assert_eq!(children(&t, Some(1)), vec![2, 3]);
        assert!(t[2].focused);
    }

    #[test]
    fn bad_parents_are_dropped() {
        let nodes = vec![node(1, Role::Row, ""), node(2, Role::Row, ""), node(3, Role::Row, "")];
        let ext = [
            NodeExt { id: 1, parent: Some(2), focused: false, checked: None },
            NodeExt { id: 2, parent: Some(1), focused: false, checked: None },
            NodeExt { id: 3, parent: Some(99), focused: false, checked: None },
        ];
        let t = build(&nodes, &ext);
        assert_eq!(t[2].parent, None);
        // a cycle is cut: following parents from every node must end at the window
        for n in &t {
            let mut at = n.parent;
            let mut hops = 0;
            while let Some(p) = at {
                hops += 1;
                assert!(hops <= t.len(), "cycle left in the tree");
                at = t.iter().find(|m| m.id == p).and_then(|m| m.parent);
            }
        }
    }

    #[test]
    fn states_cover_enabled_focus_and_checked() {
        let t = build(&[node(1, Role::Checkbox, "on")], &[NodeExt { id: 1, parent: None, focused: true, checked: None }]);
        let s = states(&t[0]);
        let has = |b: u32| if b < 32 { s[0] >> b & 1 == 1 } else { s[1] >> (b - 32) & 1 == 1 };
        assert!(has(state::ENABLED) && has(state::FOCUSABLE) && has(state::FOCUSED) && has(state::CHECKABLE) && has(state::CHECKED));
        let mut off = t[0].clone();
        off.enabled = false;
        off.checked = Some(false);
        let s = states(&off);
        assert_eq!(s[0] >> state::ENABLED & 1, 0);
        assert_eq!(s[0] >> state::CHECKED & 1, 0);
    }

    #[test]
    fn paths_round_trip() {
        for k in [Key::Root, Key::App(3), Key::Node(3, u64::MAX)] {
            assert_eq!(parse_path(&key_path(k)), Some(k));
        }
        assert_eq!(parse_path("/org/a11y/atspi/accessible/rootx"), None);
        assert_eq!(parse_path("/org/a11y/atspi/accessible/root/app1/x2"), None);
    }

    #[test]
    fn shell_chrome_is_a_tree_under_its_own_application() {
        let menus = vec![("File".to_string(), vec![("New".to_string(), true), ("Close".to_string(), false)]), ("Edit".to_string(), vec![])];
        let dock = vec![("Notes".to_string(), true), ("Files".to_string(), false)];
        let s = chrome_snap(&menus, &dock);
        assert_eq!((s.slot, s.name.as_str()), (CHROME_SLOT, "BifrOSt Shell"));
        assert_eq!(children(&s.nodes, None), vec![1, 2, 3], "menu bar, dock and search sit under the application");
        assert_eq!(children(&s.nodes, Some(1)), vec![100, 200], "the bar holds its menus");
        assert_eq!(children(&s.nodes, Some(100)), vec![101, 102], "a menu holds its rows");
        let close = s.nodes.iter().find(|n| n.id == 102).unwrap();
        assert!(!close.enabled && close.name == "Close");
        assert_eq!(children(&s.nodes, Some(2)), vec![1000, 1001]);
        let notes = s.nodes.iter().find(|n| n.id == 1000).unwrap();
        assert_eq!((notes.role, notes.value.as_str()), (Role::Button, "open"));
        assert_eq!(s.nodes.iter().find(|n| n.id == 3).unwrap().role, Role::TextField);
    }

    #[test]
    fn every_pressable_chrome_element_names_its_target() {
        let menus = vec![("File".to_string(), vec![("New".to_string(), true), ("Close".to_string(), true)]), ("Edit".to_string(), vec![("Copy".to_string(), true)])];
        let dock = vec![("Notes".to_string(), false), ("Files".to_string(), true)];
        let s = chrome_snap(&menus, &dock);
        let find = |name: &str| s.nodes.iter().find(|n| n.name == name).unwrap().id;
        assert_eq!(chrome_target(find("Close")), Some(ChromePress::MenuRow(0, 1)));
        assert_eq!(chrome_target(find("Copy")), Some(ChromePress::MenuRow(1, 0)));
        assert_eq!(chrome_target(find("Files")), Some(ChromePress::DockApp(1)));
        assert_eq!(chrome_target(find("Search")), Some(ChromePress::Search));
        for container in ["Menu Bar", "File", "Dock"] {
            assert_eq!(chrome_target(find(container)), None, "{container} presses nothing");
        }
    }

    #[test]
    fn custom_actions_follow_the_standard_ones_and_run_by_index() {
        use bifrost_proto::{AxAction, Node, UiRect};
        use bifrost_ui::ax_action::AxActionSet;
        let r = UiRect { x: 0., y: 0., w: 10., h: 10. };
        let nodes = vec![
            Node { id: 1, role: Role::Row, label: "Mail".into(), value: String::new(), bounds: r, enabled: true },
            Node { id: 2, role: Role::Slider, label: "Volume".into(), value: "0.5".into(), bounds: r, enabled: true },
        ];
        let sets = [AxActionSet::new(1, &["Archive", "Reply"]), AxActionSet::new(2, &["Mute"]), AxActionSet::new(9, &["Gone"])];
        let s = with_actions(build(&nodes, &[]), &sets);
        assert_eq!(s[0].actions, vec!["Archive", "Reply"]);
        assert_eq!(action_at(&s[0], true, 0), Some(AxAction::Press));
        assert_eq!(action_at(&s[0], true, 1), Some(AxAction::Custom("Archive".into())));
        assert_eq!(action_at(&s[0], true, 2), Some(AxAction::Custom("Reply".into())));
        assert_eq!(action_at(&s[0], true, 3), None);
        assert_eq!(action_at(&s[0], false, 0), None, "a disabled element is not pressed");
        assert_eq!(action_at(&s[0], true, -1), None);
        assert_eq!(action_at(&s[1], true, 2), Some(AxAction::Decrement));
        assert_eq!(action_at(&s[1], true, 3), Some(AxAction::Custom("Mute".into())));
    }

    #[test]
    fn widgets_are_a_list_beside_the_chrome_with_their_buttons_below_them() {
        let mut s = chrome_snap(&[], &[]);
        add_widgets(&mut s, &[]);
        assert!(!s.nodes.iter().any(|n| n.id == WIDGETS_NODE), "no list without widgets");
        let w = WidgetSnap { title: "Next".into(), value: "Standup".into(), label: "Next, Standup, Actions: Done, Mute".into(), buttons: vec![("Done".into(), None, true), ("Mute".into(), Some(true), false)] };
        add_widgets(&mut s, &[w]);
        assert!(children(&s.nodes, None).contains(&WIDGETS_NODE));
        assert_eq!(children(&s.nodes, Some(WIDGETS_NODE)), vec![5000]);
        let row = s.nodes.iter().find(|n| n.id == 5000).unwrap();
        assert_eq!((row.role, row.name.as_str(), row.value.as_str()), (Role::StaticText, "Next", "Standup"));
        assert_eq!(row.help, "Next, Standup, Actions: Done, Mute", "the spoken label is the help text");
        assert_eq!(children(&s.nodes, Some(5000)), vec![5001, 5002]);
        let mute = s.nodes.iter().find(|n| n.id == 5002).unwrap();
        assert_eq!((mute.role, mute.checked, mute.enabled), (Role::Toggle, Some(true), false), "a waiting toggle is not enabled");
    }

    #[test]
    fn a_press_on_a_widget_button_reaches_the_widget_not_the_dock() {
        let dock: Vec<(String, bool)> = (0..3).map(|i| (format!("App {i}"), false)).collect();
        let mut s = chrome_snap(&[], &dock);
        let rows = [
            WidgetSnap { title: "Next".into(), value: String::new(), label: String::new(), buttons: vec![("Done".into(), None, true)] },
            WidgetSnap { title: "Timer".into(), value: String::new(), label: String::new(), buttons: vec![("Pause".into(), None, true), ("Mute".into(), Some(false), true)] },
        ];
        add_widgets(&mut s, &rows);
        let find = |name: &str| s.nodes.iter().find(|n| n.name == name).unwrap().id;
        assert_eq!(chrome_target(find("Done")), Some(ChromePress::Widget(0, 0)));
        assert_eq!(chrome_target(find("Mute")), Some(ChromePress::Widget(1, 1)));
        assert_eq!(chrome_target(find("App 2")), Some(ChromePress::DockApp(2)));
        for container in ["Widgets", "Next", "Timer"] {
            assert_eq!(chrome_target(find(container)), None, "{container} presses nothing");
        }
        assert_eq!(chrome_target(4999), Some(ChromePress::DockApp(3999)), "the dock range stops below the widgets");
        assert_eq!(chrome_target(WIDGET_BASE + 10 * MAX_WIDGET_ROWS as u64 + 1), None, "past the last widget row");
    }

    #[test]
    fn numeric_elements_list_increment_and_decrement() {
        assert_eq!(action_names(false), vec!["press"]);
        assert_eq!(action_names(true), vec!["press", "increment", "decrement"]);
    }

    #[test]
    fn numeric_reads_fractions_and_percent() {
        assert_eq!(numeric("0.4"), Some((0.4, 1.0)));
        assert_eq!(numeric("40%"), Some((40.0, 100.0)));
        assert_eq!(numeric("3/10"), Some((3.0, 10.0)));
        assert_eq!(numeric("3/0"), None);
        assert_eq!(numeric("abc"), None);
    }

    #[test]
    fn text_edits_report_inserts_deletes_and_replaces() {
        assert_eq!(text_edits("abc", "abc"), vec![]);
        assert_eq!(text_edits("ac", "abc"), vec![TextEdit { insert: true, at: 1, len: 1, text: "b".into() }]);
        assert_eq!(text_edits("abc", "ac"), vec![TextEdit { insert: false, at: 1, len: 1, text: "b".into() }]);
        let r = text_edits("h\u{e9}llo", "hallo");
        assert_eq!((r[0].insert, r[0].at, r[0].text.as_str()), (false, 1, "\u{e9}"));
        assert_eq!((r[1].insert, r[1].at, r[1].text.as_str()), (true, 1, "a"));
    }

    #[test]
    fn password_fields_never_expose_text() {
        let nodes = vec![node(1, Role::TextField, "hunter2")];
        let t = bifrost_proto::AxText { id: 1, caret: 7, sel_start: 7, sel_end: 7, vis_start: 0, vis_end: 7, secure: true, len: 7 };
        let s = build_ax(&nodes, &[], &[t], &[]);
        assert_eq!(s[0].value.chars().count(), 7);
        assert!(!s[0].value.contains("hunter"));
        assert_eq!(s[0].atspi_role(), role::PASSWORD_TEXT);
        // a plain field keeps ENTRY and its text
        let plain = build_ax(&nodes, &[], &[bifrost_proto::AxText { secure: false, ..t }], &[]);
        assert_eq!((plain[0].atspi_role(), plain[0].value.as_str()), (role::ENTRY, "hunter2"));
    }

    #[test]
    fn label_title_and_help_stay_separate() {
        let mut n = node(1, Role::Button, "");
        n.label = String::new();
        let metas = [bifrost_proto::AxMeta { id: 1, title: "Send".into(), help: "Sends the message".into() }];
        let s = build_ax(&[n.clone()], &[], &[], &metas);
        assert_eq!((s[0].accessible_name(), s[0].help.as_str()), ("Send", "Sends the message"));
        n.label = "Send message".into();
        let s = build_ax(&[n], &[], &[], &metas);
        assert_eq!((s[0].accessible_name(), s[0].title.as_str()), ("Send message", "Send"));
    }
}
