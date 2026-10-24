//! Named custom actions of an app's elements (`accessibilityCustomActions`): the extra things an element can do beyond
//! press ("Archive", "Mark as Read", "Reply"), listed to assistive technology by name and run by name.
//!
//! An element's actions travel as one [`AxActionSet`]: the element's `Node::id` and its action names in the order the user
//! should hear them. Apps either return the sets from `App::ax_custom_actions` or, while drawing, call [`record`] for an
//! element the kit just recorded; the runtime sends the union whenever it changes. The shell keeps what [`clean`] leaves,
//! lists the names after the standard ones (`press`, `increment`, `decrement`) on the AT-SPI `Action` interface, and runs
//! one by sending its name back as `AxAction::Custom`.
//!
//! The reference lists an element's custom actions as `AXCustomActions` entries with a name and an action selector; a
//! screen reader offers them in an Actions rotor and performs the one chosen. There is no selector on the wire here: the
//! app gets the name back and decides.

/// Most elements one app may give custom actions.
pub const MAX_ACTION_ELEMENTS: usize = 64;
/// Most custom actions one element may list.
pub const MAX_ACTIONS_PER_ELEMENT: usize = 8;
/// Longest action name, in bytes.
pub const MAX_ACTION_NAME: usize = 64;

/// The custom actions of one element, in the order the user should hear them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AxActionSet {
    /// the `Node::id` the actions belong to
    pub id: u64,
    pub names: Vec<String>,
}

impl AxActionSet {
    pub fn new(id: u64, names: &[&str]) -> Self {
        AxActionSet { id, names: names.iter().map(|n| n.to_string()).collect() }
    }

    /// Whether this element has an action called `name` (case-insensitive, outer spaces ignored).
    pub fn has(&self, name: &str) -> bool {
        let name = name.trim();
        self.names.iter().any(|n| n.eq_ignore_ascii_case(name))
    }

    /// The name the element itself uses for `name` (its own spelling), or `None` when it has no such action.
    pub fn canonical(&self, name: &str) -> Option<&str> {
        let name = name.trim();
        self.names.iter().find(|n| n.eq_ignore_ascii_case(name)).map(String::as_str)
    }
}

/// Collapses inner runs of white space, trims the ends and cuts the name to `MAX_ACTION_NAME` bytes on a character boundary.
fn clean_name(s: &str) -> String {
    let joined = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut n = joined.len().min(MAX_ACTION_NAME);
    while !joined.is_char_boundary(n) {
        n -= 1;
    }
    joined[..n].trim_end().to_string()
}

/// What the shell keeps: blank names dropped, repeated names within an element dropped (compared without case, the first
/// wins), elements with no usable name dropped, a repeated element merged into its first set, and every cap applied.
pub fn clean(sets: Vec<AxActionSet>) -> Vec<AxActionSet> {
    let mut out: Vec<AxActionSet> = Vec::new();
    for s in sets {
        let at = match out.iter().position(|o| o.id == s.id) {
            Some(i) => i,
            None if out.len() < MAX_ACTION_ELEMENTS => {
                out.push(AxActionSet { id: s.id, names: Vec::new() });
                out.len() - 1
            }
            None => continue,
        };
        for name in s.names {
            let name = clean_name(&name);
            let set = &mut out[at];
            if name.is_empty() || set.names.len() >= MAX_ACTIONS_PER_ELEMENT || set.has(&name) {
                continue;
            }
            set.names.push(name);
        }
    }
    out.retain(|s| !s.names.is_empty());
    out
}

/// The set an element answers through `AccessibilityElement::accessibility_custom_actions`, for an app that keeps
/// stand-in elements (`ax::AxElement`) for things it draws.
pub fn of_element(id: u64, element: &dyn crate::ax::AccessibilityElement) -> AxActionSet {
    AxActionSet { id, names: element.accessibility_custom_actions().into_iter().map(|a| a.name).collect() }
}

// --- recording: an immediate-mode app names an element's actions right after the kit recorded the element -------------------
use std::cell::RefCell;
thread_local! {
    static FRAME: RefCell<Vec<AxActionSet>> = const { RefCell::new(Vec::new()) };
}

/// Gives the element `id` (as `ax::record` returned it this frame) the custom actions `names`. A zero id (a full frame) is
/// ignored. Calling it twice for one element adds to its list.
pub fn record(id: u64, names: &[&str]) {
    if id == 0 || names.is_empty() {
        return;
    }
    FRAME.with(|f| {
        let mut f = f.borrow_mut();
        match f.iter_mut().find(|s| s.id == id) {
            Some(s) => s.names.extend(names.iter().map(|n| n.to_string())),
            None if f.len() < MAX_ACTION_ELEMENTS => f.push(AxActionSet::new(id, names)),
            None => {}
        }
    });
}

/// The actions recorded since the last call, in recording order. `ax::begin_frame` forgets them too.
pub fn take() -> Vec<AxActionSet> {
    FRAME.with(|f| std::mem::take(&mut *f.borrow_mut()))
}

/// Forgets what was recorded (called by `ax::begin_frame`).
pub(crate) fn clear() {
    FRAME.with(|f| f.borrow_mut().clear());
}

/// The app's declared sets followed by the recorded ones, cleaned: what the runtime sends.
pub fn merge(declared: Vec<AxActionSet>, recorded: &[AxActionSet]) -> Vec<AxActionSet> {
    let mut all = declared;
    all.extend_from_slice(recorded);
    clean(all)
}

/// The AT-SPI `Action` names of an element: the standard ones (`press`, plus `increment` and `decrement` for a numeric
/// element), then its custom actions. Index `i` of the list is what `DoAction(i)` runs.
pub fn names_with_standard(numeric: bool, custom: &[String]) -> Vec<String> {
    let mut v: Vec<String> = if numeric { vec!["press".into(), "increment".into(), "decrement".into()] } else { vec!["press".into()] };
    v.extend(custom.iter().cloned());
    v
}

/// How many entries of [`names_with_standard`] are standard; an index at or past this is a custom action.
pub fn standard_count(numeric: bool) -> usize {
    if numeric { 3 } else { 1 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_drops_blanks_and_repeats_and_merges_an_element_listed_twice() {
        let sets = vec![
            AxActionSet::new(1, &["Archive", "  mark   as read ", "archive", ""]),
            AxActionSet::new(2, &["   "]),
            AxActionSet::new(1, &["Reply", "Mark as Read"]),
        ];
        let out = clean(sets);
        assert_eq!(out, vec![AxActionSet::new(1, &["Archive", "mark as read", "Reply"])]);
    }

    #[test]
    fn clean_applies_every_cap() {
        let long = "\u{e9}".repeat(MAX_ACTION_NAME);
        let many: Vec<String> = (0..MAX_ACTIONS_PER_ELEMENT + 4).map(|i| format!("a{i}")).collect();
        let mut sets = vec![AxActionSet { id: 1, names: many }, AxActionSet { id: 2, names: vec![long] }];
        sets.extend((0..MAX_ACTION_ELEMENTS + 5).map(|i| AxActionSet::new(100 + i as u64, &["Go"])));
        let out = clean(sets);
        assert_eq!(out.len(), MAX_ACTION_ELEMENTS);
        assert_eq!(out[0].names.len(), MAX_ACTIONS_PER_ELEMENT);
        assert!(out[1].names[0].len() <= MAX_ACTION_NAME);
        assert!(out[1].names[0].chars().all(|c| c == '\u{e9}'));
    }

    #[test]
    fn names_are_found_without_case_and_keep_their_own_spelling() {
        let s = AxActionSet::new(3, &["Mark as Read"]);
        assert!(s.has(" mark as read "));
        assert_eq!(s.canonical("MARK AS READ"), Some("Mark as Read"));
        assert_eq!(s.canonical("Delete"), None);
    }

    #[test]
    fn recorded_actions_merge_after_the_declared_ones() {
        let _ = take();
        record(7, &["Flag"]);
        record(0, &["Ignored"]);
        record(7, &["Delete"]);
        let recorded = take();
        assert!(take().is_empty(), "taking empties the frame");
        let out = merge(vec![AxActionSet::new(7, &["Reply"]), AxActionSet::new(9, &["Open"])], &recorded);
        assert_eq!(out, vec![AxActionSet::new(7, &["Reply", "Flag", "Delete"]), AxActionSet::new(9, &["Open"])]);
    }

    #[test]
    fn custom_names_follow_the_standard_ones() {
        let custom = vec!["Archive".to_string()];
        assert_eq!(names_with_standard(false, &custom), vec!["press", "Archive"]);
        assert_eq!(names_with_standard(true, &custom), vec!["press", "increment", "decrement", "Archive"]);
        assert_eq!(standard_count(false), 1);
        assert_eq!(standard_count(true), 3);
    }

    #[test]
    fn a_stand_in_element_answers_its_custom_actions() {
        use crate::ax::{AxCustomAction, AxElement, AxRole};
        let mut e = AxElement::new(5, AxRole::Row, "Mail", crate::Rect { x: 0.0, y: 0.0, w: 10.0, h: 10.0 });
        e.custom_actions.push(AxCustomAction::new("Archive"));
        assert_eq!(of_element(5, &e), AxActionSet::new(5, &["Archive"]));
    }
}
