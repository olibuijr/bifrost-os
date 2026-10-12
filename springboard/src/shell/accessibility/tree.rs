//! The element tree the `ax` control command serves: parents, keyboard focus and checked state.
//!
//! The map (`bifrost_ui::ax::Map`) already nests addresses (`scope/parent/role/name`), but an address does not say which
//! element is the parent when the parent's address is not an element, nor which element holds focus or what a checkbox
//! shows. This module adds those. An app declares its hierarchy and state through `ControlsTree` (`NodeExt`); a Settings
//! toggle carries its state in its value (`on` or `off`). With no tree every app node is a root, so the addresses are the
//! ones `Map::add_nodes` gives and nothing changes for flat apps.
//!
//! The listing is the old five columns (address, role, name, value, enabled) followed by parent, checked and focused.
//! Agents that read the first five keep working.

use bifrost_proto::{Node, NodeExt};
use bifrost_ui::ax::{role_name, Element, Map};
use std::collections::HashMap;

/// The state the map does not carry: which addresses hold focus and the checked state an app reported.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AxState {
    pub focused: Vec<String>,
    /// (address, checked) for app elements whose `NodeExt::checked` is set
    pub checked: Vec<(String, bool)>,
}

/// `checked`, `unchecked`, or "" when the element is not checkable. An app's report wins over the value.
pub fn checked_word(e: &Element, st: &AxState) -> &'static str {
    let reported = st.checked.iter().find(|(a, _)| a == &e.address).map(|(_, c)| *c);
    let state = reported.or_else(|| match (e.role.as_str(), e.value.as_str()) {
        ("toggle" | "checkbox", "on") => Some(true),
        ("toggle" | "checkbox", "off") => Some(false),
        _ => None,
    });
    match state {
        Some(true) => "checked",
        Some(false) => "unchecked",
        None => "",
    }
}

/// The address above `address` when that address is itself an element; "" when the element sits directly in its window.
pub fn parent_of(map: &Map, address: &str) -> String {
    match address.rsplit_once('/') {
        Some((p, _)) if map.get(p).is_some() => p.to_string(),
        _ => String::new(),
    }
}

/// One line per element under `prefix`: `address, role, name, value, enabled, parent, checked, focused`, tab-separated.
pub fn listing(map: &Map, st: &AxState, prefix: &str) -> String {
    let mut out = String::new();
    for e in map.under(prefix) {
        let focused = if st.focused.iter().any(|a| a == &e.address) { "focused" } else { "" };
        out.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            e.address,
            e.role,
            e.name,
            e.value,
            if e.enabled { "enabled" } else { "disabled" },
            parent_of(map, &e.address),
            checked_word(e, st),
            focused,
        ));
    }
    out
}

/// The nodes of one app, joined to its hierarchy. Each node is visited once, from its root down, so a cycle or a parent that
/// is missing or is the node itself cannot loop: such a node is a root.
struct Tree<'a> {
    scope: &'a str,
    nodes: &'a [Node],
    kids: Vec<Vec<usize>>,
    focus: Vec<bool>,
    check: Vec<Option<bool>>,
    seen: Vec<bool>,
}

impl Tree<'_> {
    /// Adds node `i` under `parent` (an element address), or under its scope and role when it is a root.
    fn visit(&mut self, map: &mut Map, st: &mut AxState, i: usize, parent: Option<&str>) {
        if self.seen[i] {
            return;
        }
        self.seen[i] = true;
        let n = &self.nodes[i];
        let root = format!("{}/{}", self.scope, role_name(n.role));
        let addr = map.add(parent.unwrap_or(root.as_str()), role_name(n.role), &n.label, &n.value, n.enabled, Some(n.bounds), &n.id.to_string());
        if self.focus[i] {
            st.focused.push(addr.clone());
        }
        if let Some(c) = self.check[i] {
            st.checked.push((addr.clone(), c));
        }
        let kids = self.kids[i].clone();
        for k in kids {
            self.visit(map, st, k, Some(&addr));
        }
    }
}

/// Adds one app's nodes under `scope` (`app/<id>`), nested as its `ControlsTree` says, and records focus and checked state
/// into `st`. Addresses match `Map::add_nodes` when no node has a parent.
pub fn add_app_tree(map: &mut Map, scope: &str, nodes: &[Node], ext: &[NodeExt], st: &mut AxState) {
    let index: HashMap<u64, usize> = nodes.iter().enumerate().map(|(i, n)| (n.id, i)).collect();
    let mut parent = vec![None; nodes.len()];
    let mut focus = vec![false; nodes.len()];
    let mut check = vec![None; nodes.len()];
    for e in ext {
        let Some(&i) = index.get(&e.id) else { continue };
        if let Some(&p) = e.parent.and_then(|p| index.get(&p)).filter(|&&p| p != i) {
            parent[i] = Some(p);
        }
        focus[i] = e.focused;
        check[i] = e.checked;
    }
    let mut kids = vec![Vec::new(); nodes.len()];
    for (i, p) in parent.iter().enumerate() {
        if let Some(p) = p {
            kids[*p].push(i);
        }
    }
    let mut tree = Tree { scope, nodes, kids, focus, check, seen: vec![false; nodes.len()] };
    for i in 0..nodes.len() {
        if parent[i].is_none() {
            tree.visit(map, st, i, None);
        }
    }
    // a cycle has no root: its nodes are added as roots so none is lost
    for i in 0..nodes.len() {
        if !tree.seen[i] {
            tree.visit(map, st, i, None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_proto::Role;
    use bifrost_ui::Rect;

    fn node(id: u64, role: Role, label: &str, value: &str) -> Node {
        Node { id, role, label: label.into(), value: value.into(), bounds: Rect { x: 0., y: 0., w: 10., h: 10. }, enabled: true }
    }

    fn ext(id: u64, parent: Option<u64>, focused: bool, checked: Option<bool>) -> NodeExt {
        NodeExt { id, parent, focused, checked }
    }

    #[test]
    fn a_flat_app_gets_the_addresses_add_nodes_gives() {
        let mut m = Map::new();
        let mut st = AxState::default();
        add_app_tree(&mut m, "app/x", &[node(1, Role::Button, "Save", ""), node(2, Role::Row, "A", "")], &[], &mut st);
        assert!(m.get("app/x/button/save").is_some_and(|e| e.handle == "1"));
        assert!(m.get("app/x/row/a").is_some_and(|e| e.handle == "2"));
        assert_eq!(st, AxState::default(), "no focus or checked state without a tree");
        assert_eq!(parent_of(&m, "app/x/row/a"), "", "a root sits directly in its window");
    }

    #[test]
    fn a_tree_nests_addresses_and_records_focus_and_checked_state() {
        let nodes = [
            node(10, Role::List, "Files", ""),
            node(11, Role::Row, "Report", ""),
            node(12, Role::Row, "Notes", ""),
            node(13, Role::Checkbox, "Pinned", ""),
        ];
        let tree = [ext(11, Some(10), true, None), ext(12, Some(10), false, None), ext(13, None, false, Some(true))];
        let mut m = Map::new();
        let mut st = AxState::default();
        add_app_tree(&mut m, "app/x", &nodes, &tree, &mut st);
        assert!(m.get("app/x/list/files/report").is_some(), "{}", m.listing("app/x"));
        assert!(m.get("app/x/list/files/notes").is_some());
        assert_eq!(st.focused, ["app/x/list/files/report"]);
        assert_eq!(st.checked, [("app/x/checkbox/pinned".to_string(), true)]);
        let text = listing(&m, &st, "app/x");
        assert!(text.contains("app/x/list/files/report\trow\tReport\t\tenabled\tapp/x/list/files\t\tfocused\n"), "{text}");
        assert!(text.contains("app/x/checkbox/pinned\tcheckbox\tPinned\t\tenabled\t\tchecked\t\n"), "{text}");
        assert!(text.contains("app/x/list/files\tlist\tFiles\t\tenabled\t\t\t\n"), "{text}");
    }

    #[test]
    fn a_cycle_or_a_self_parent_cannot_hang_or_lose_nodes() {
        let nodes = [node(1, Role::Button, "A", ""), node(2, Role::Button, "B", ""), node(3, Role::Button, "C", "")];
        let tree = [ext(1, Some(2), false, None), ext(2, Some(1), false, None), ext(3, Some(3), false, None)];
        let mut m = Map::new();
        let mut st = AxState::default();
        add_app_tree(&mut m, "app/x", &nodes, &tree, &mut st);
        assert_eq!(m.len(), 3, "each node is added exactly once");
        assert!(m.get("app/x/button/c").is_some_and(|e| parent_of(&m, &e.address).is_empty()), "a self parent makes a root");
    }

    #[test]
    fn toggles_report_checked_from_their_value() {
        let mut m = Map::new();
        m.add("os/settings", "row", "General", "", true, None, "");
        m.add("os/settings/general", "toggle", "Reduce Motion", "on", true, None, "");
        m.add("os/settings/general", "toggle", "Bold Text", "off", true, None, "");
        m.add("os/settings/general", "button", "Reset", "on", true, None, "");
        let st = AxState::default();
        let text = listing(&m, &st, "os/settings");
        assert!(text.contains("os/settings/general/reduce-motion\ttoggle\tReduce Motion\ton\tenabled\tos/settings/general\tchecked\t\n"), "{text}");
        assert!(text.contains("os/settings/general/bold-text\ttoggle\tBold Text\toff\tenabled\tos/settings/general\tunchecked\t\n"), "{text}");
        assert!(text.contains("os/settings/general/reset\tbutton\tReset\ton\tenabled\tos/settings/general\t\t\n"), "a button is not checkable: {text}");
    }

    #[test]
    fn an_app_report_overrides_the_value() {
        let mut m = Map::new();
        let a = m.add("app/x", "toggle", "Live", "on", true, None, "");
        let st = AxState { focused: vec![], checked: vec![(a.clone(), false)] };
        assert_eq!(checked_word(m.get(&a).unwrap(), &st), "unchecked");
    }
}
