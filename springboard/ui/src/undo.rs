//! An undo manager (`NativeUndoManager`): commands with an inverse, grouped (nestable, optionally per event), with a
//! bounded history, named actions, per-target removal and a redo stack.

struct Entry<T> {
    name: String,
    undo: Vec<Box<dyn Fn(&mut T)>>,
    redo: Vec<Box<dyn Fn(&mut T)>>,
    /// the targets the registered pairs belong to (`removeAllActionsWithTarget:`)
    targets: Vec<String>,
    /// typing coalescing: the key of the run this entry belongs to and when it last grew (ms)
    key: Option<String>,
    at: f64,
}

impl<T> Entry<T> {
    fn new(name: &str) -> Self {
        Entry { name: name.into(), undo: Vec::new(), redo: Vec::new(), targets: Vec::new(), key: None, at: 0.0 }
    }
}

/// Undo history over a state `T`. Register what you just did together with how to reverse it.
pub struct UndoManager<T> {
    done: Vec<Entry<T>>,
    undone: Vec<Entry<T>>,
    open: Option<Entry<T>>,
    /// nesting depth of `begin_undo_grouping`
    depth: usize,
    /// the open group was started by `groups_by_event`, and closes at `end_event`
    implicit: bool,
    groups_by_event: bool,
    limit: usize,
    undoing: bool,
    redoing: bool,
    /// steps applied minus steps undone, and how often the redo history was thrown away (see `ChangeCount`)
    position: i64,
    epoch: u32,
}

/// Where the document is in its edit history (`NativeDocument.updateChangeCount`): equal counts mean equal content.
/// A new edit after an undo starts a new epoch, so a state saved on the abandoned branch is never mistaken for the
/// present one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct ChangeCount {
    pub position: i64,
    pub epoch: u32,
}

impl<T> UndoManager<T> {
    pub fn new(limit: usize) -> Self {
        UndoManager { done: Vec::new(), undone: Vec::new(), open: None, depth: 0, implicit: false, groups_by_event: false, limit: limit.max(1), undoing: false, redoing: false, position: 0, epoch: 0 }
    }
    /// `levelsOfUndo`: how many steps are kept; 0 means unlimited, as in the original.
    pub fn levels_of_undo(&self) -> usize {
        if self.limit == usize::MAX { 0 } else { self.limit }
    }
    /// `setLevelsOfUndo:`; older steps beyond the new limit are dropped at once.
    pub fn set_levels_of_undo(&mut self, levels: usize) {
        self.limit = if levels == 0 { usize::MAX } else { levels };
        self.trim();
    }
    /// `groupsByEvent`: with it on every registration outside an explicit group joins an implicit group that
    /// `end_event` closes, so one event's edits are one step. Off by default (the app calls `end_event` per event).
    pub fn groups_by_event(&self) -> bool {
        self.groups_by_event
    }
    pub fn set_groups_by_event(&mut self, on: bool) {
        self.groups_by_event = on;
    }
    /// The end of one event-loop turn: closes the implicit group `groups_by_event` opened.
    pub fn end_event(&mut self) {
        if self.implicit {
            self.implicit = false;
            self.depth = 0;
            self.close_open();
        }
    }
    /// Records an action. `undo` reverses it and `redo` repeats it. Inside a group the pair joins the group;
    /// otherwise it is its own step. A new action clears the redo history.
    pub fn register(&mut self, name: &str, undo: impl Fn(&mut T) + 'static, redo: impl Fn(&mut T) + 'static) {
        self.register_for("", name, undo, redo)
    }
    /// `register` for a `target` (any owner tag), so `remove_all_actions` can drop what refers to it later.
    pub fn register_for(&mut self, target: &str, name: &str, undo: impl Fn(&mut T) + 'static, redo: impl Fn(&mut T) + 'static) {
        self.note_new_action();
        if self.open.is_none() && self.groups_by_event {
            self.open = Some(Entry::new(name));
            self.implicit = true;
            self.depth = 1;
        }
        match self.open.as_mut() {
            Some(g) => {
                if g.name.is_empty() {
                    g.name = name.into();
                }
                g.undo.push(Box::new(undo));
                g.redo.push(Box::new(redo));
                g.targets.push(target.into());
            }
            None => {
                self.position += 1;
                let mut e = Entry::new(name);
                e.undo.push(Box::new(undo));
                e.redo.push(Box::new(redo));
                e.targets.push(target.into());
                self.push(e);
            }
        }
    }
    /// Typing: like `register`, but when the newest step has the same `key`, no redo is pending and it grew less than
    /// `gap_ms` ago, the pair joins that step instead of starting a new one (one undo per burst of typing).
    pub fn register_coalesced(&mut self, key: &str, now_ms: f64, gap_ms: f64, name: &str, undo: impl Fn(&mut T) + 'static, redo: impl Fn(&mut T) + 'static) {
        if self.open.is_none() && self.undone.is_empty() {
            if let Some(last) = self.done.last_mut() {
                if last.key.as_deref() == Some(key) && now_ms >= last.at && now_ms - last.at <= gap_ms {
                    last.undo.push(Box::new(undo));
                    last.redo.push(Box::new(redo));
                    last.targets.push(String::new());
                    last.at = now_ms;
                    return;
                }
            }
        }
        self.register(name, undo, redo);
        if self.open.is_none() {
            if let Some(last) = self.done.last_mut() {
                last.key = Some(key.into());
                last.at = now_ms;
            }
        }
    }
    fn note_new_action(&mut self) {
        if !self.undone.is_empty() && !self.undoing {
            self.undone.clear();
            self.epoch += 1;
        }
    }
    /// `beginUndoGrouping`: everything registered until the matching `end_undo_grouping` is one undo step. Groups nest;
    /// only the outermost one makes a step. The first registration names the step unless `set_action_name` does.
    pub fn begin_undo_grouping(&mut self) {
        if self.implicit {
            self.implicit = false; // an explicit group takes over the implicit one
        } else if self.open.is_none() {
            self.open = Some(Entry::new(""));
        }
        self.depth += 1;
    }
    pub fn end_undo_grouping(&mut self) {
        if self.depth > 0 {
            self.depth -= 1;
        }
        if self.depth == 0 {
            self.close_open();
        }
    }
    /// Starts a group with a name (`beginUndoGrouping` + `setActionName:`).
    pub fn begin_group(&mut self, name: &str) {
        self.begin_undo_grouping();
        if let Some(g) = self.open.as_mut() {
            if g.name.is_empty() {
                g.name = name.into();
            }
        }
    }
    pub fn end_group(&mut self) {
        self.end_undo_grouping();
    }
    /// How deep the open groups are nested.
    pub fn grouping_level(&self) -> usize {
        self.depth
    }
    fn close_open(&mut self) {
        if let Some(g) = self.open.take() {
            if !g.undo.is_empty() {
                self.position += 1;
                self.push(g);
            }
        }
    }
    /// `setActionName:`: names the open group, or else the newest step.
    pub fn set_action_name(&mut self, name: &str) {
        if let Some(g) = self.open.as_mut() {
            g.name = name.into();
        } else if let Some(e) = self.done.last_mut() {
            e.name = name.into();
        }
    }
    fn push(&mut self, e: Entry<T>) {
        self.done.push(e);
        self.trim();
    }
    fn trim(&mut self) {
        while self.done.len() > self.limit {
            self.done.remove(0);
        }
    }
    /// The edit history position, for `document::DocumentController::note_change`: it differs from the value taken at
    /// the last save exactly when the document is edited. `clear` does not reset it (the content is unchanged).
    pub fn change_count(&self) -> ChangeCount {
        ChangeCount { position: self.position, epoch: self.epoch }
    }
    pub fn can_undo(&self) -> bool {
        !self.done.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.undone.is_empty()
    }
    pub fn is_undoing(&self) -> bool {
        self.undoing
    }
    pub fn is_redoing(&self) -> bool {
        self.redoing
    }
    /// Completed steps in the undo history (an open group counts once it ends).
    pub fn steps(&self) -> usize {
        self.done.len()
    }
    /// "Undo Typing", or "Undo" when the action has no name; `None` when there is nothing to undo.
    pub fn undo_title(&self) -> Option<String> {
        self.done.last().map(|e| title("Undo", &e.name))
    }
    pub fn redo_title(&self) -> Option<String> {
        self.undone.last().map(|e| title("Redo", &e.name))
    }
    /// `undoMenuItemTitle`: "Undo Typing", or plain "Undo" when there is nothing to undo or the step has no name.
    pub fn undo_menu_item_title(&self) -> String {
        self.undo_title().unwrap_or_else(|| "Undo".into())
    }
    pub fn redo_menu_item_title(&self) -> String {
        self.redo_title().unwrap_or_else(|| "Redo".into())
    }
    /// Reverses the latest step (a group's parts in reverse order). Returns its name.
    pub fn undo(&mut self, state: &mut T) -> Option<String> {
        self.depth = 0;
        self.implicit = false;
        self.close_open();
        let e = self.done.pop()?;
        self.position -= 1;
        self.undoing = true;
        e.undo.iter().rev().for_each(|f| f(state));
        self.undoing = false;
        let name = e.name.clone();
        self.undone.push(e);
        Some(name)
    }
    pub fn redo(&mut self, state: &mut T) -> Option<String> {
        let e = self.undone.pop()?;
        self.position += 1;
        self.redoing = true;
        e.redo.iter().for_each(|f| f(state));
        self.redoing = false;
        let name = e.name.clone();
        self.done.push(e);
        Some(name)
    }
    /// Forgets all history, e.g. after saving a document that cannot be reopened.
    pub fn clear(&mut self) {
        self.done.clear();
        self.undone.clear();
        self.open = None;
        self.depth = 0;
        self.implicit = false;
    }
    /// `removeAllActionsWithTarget:`: drops every step that has an action registered for `target`
    /// (a group is dropped whole).
    pub fn remove_all_actions(&mut self, target: &str) {
        let keep = |e: &Entry<T>| !e.targets.iter().any(|t| t == target);
        self.done.retain(keep);
        self.undone.retain(keep);
        if self.open.as_ref().is_some_and(|g| g.targets.iter().any(|t| t == target)) {
            self.open = None;
            self.depth = 0;
            self.implicit = false;
        }
    }
}

fn title(verb: &str, name: &str) -> String {
    if name.is_empty() { verb.to_string() } else { format!("{verb} {name}") }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn add(m: &mut UndoManager<Vec<i32>>, s: &mut Vec<i32>, v: i32, name: &str) {
        s.push(v);
        m.register(name, |s| { s.pop(); }, move |s| s.push(v));
    }

    #[test]
    fn change_count_returns_to_the_saved_value_only_on_the_same_branch() {
        let (mut m, mut s) = (UndoManager::new(10), vec![]);
        add(&mut m, &mut s, 1, "");
        let saved = m.change_count();
        add(&mut m, &mut s, 2, "");
        assert_ne!(m.change_count(), saved);
        m.undo(&mut s);
        assert_eq!(m.change_count(), saved, "undo back to the saved state");
        m.begin_group("g");
        add(&mut m, &mut s, 3, "");
        add(&mut m, &mut s, 4, "");
        m.end_group();
        assert_ne!(m.change_count(), saved);
        m.undo(&mut s);
        assert_eq!(m.change_count().position, saved.position);
        m.redo(&mut s);
        m.undo(&mut s);
        m.undo(&mut s);
        add(&mut m, &mut s, 9, "");
        assert_ne!(m.change_count(), saved, "a new edit after undoing past the save is a different state");
    }

    #[test]
    fn undo_and_redo_walk_the_history_in_order() {
        let (mut m, mut s) = (UndoManager::new(10), vec![]);
        add(&mut m, &mut s, 1, "Add One");
        add(&mut m, &mut s, 2, "Add Two");
        assert_eq!(m.undo_title().as_deref(), Some("Undo Add Two"));
        assert_eq!(m.undo(&mut s).as_deref(), Some("Add Two"));
        assert_eq!(s, vec![1]);
        assert_eq!(m.redo_title().as_deref(), Some("Redo Add Two"));
        m.redo(&mut s);
        assert_eq!(s, vec![1, 2]);
        m.undo(&mut s);
        m.undo(&mut s);
        assert!(s.is_empty() && !m.can_undo() && m.can_redo());
        assert_eq!(m.undo(&mut s), None);
    }

    #[test]
    fn a_new_action_clears_the_redo_history() {
        let (mut m, mut s) = (UndoManager::new(10), vec![]);
        add(&mut m, &mut s, 1, "");
        m.undo(&mut s);
        assert!(m.can_redo());
        add(&mut m, &mut s, 9, "");
        assert!(!m.can_redo());
        assert_eq!(m.undo_title().as_deref(), Some("Undo"), "an unnamed action is just Undo");
    }

    #[test]
    fn a_group_is_one_step_undone_in_reverse() {
        let (mut m, mut s) = (UndoManager::new(10), vec![]);
        m.begin_group("Typing");
        add(&mut m, &mut s, 1, "");
        add(&mut m, &mut s, 2, "");
        add(&mut m, &mut s, 3, "");
        assert!(!m.can_undo(), "the group is still open");
        m.end_group();
        assert_eq!(m.undo(&mut s).as_deref(), Some("Typing"));
        assert!(s.is_empty(), "all three went in one undo");
        m.redo(&mut s);
        assert_eq!(s, vec![1, 2, 3], "and come back in order");
        m.begin_group("empty");
        m.end_group();
        assert_eq!(m.undo_title().as_deref(), Some("Undo Typing"), "an empty group adds nothing");
    }

    #[test]
    fn the_history_is_bounded_and_clear_forgets_it() {
        let (mut m, mut s) = (UndoManager::new(2), vec![]);
        for v in 1..=4 {
            add(&mut m, &mut s, v, "");
        }
        m.undo(&mut s);
        m.undo(&mut s);
        assert_eq!(m.undo(&mut s), None, "only the last two steps were kept");
        assert_eq!(s, vec![1, 2]);
        m.clear();
        assert!(!m.can_undo() && !m.can_redo());
    }

    #[test]
    fn menu_titles_follow_the_action_name() {
        let (mut m, mut s) = (UndoManager::new(10), vec![]);
        assert_eq!(m.undo_menu_item_title(), "Undo");
        assert_eq!(m.redo_menu_item_title(), "Redo");
        add(&mut m, &mut s, 1, "");
        m.set_action_name("Move");
        assert_eq!(m.undo_menu_item_title(), "Undo Move");
        m.undo(&mut s);
        assert_eq!(m.undo_menu_item_title(), "Undo");
        assert_eq!(m.redo_menu_item_title(), "Redo Move");
        assert!(!m.can_undo() && m.can_redo());
    }

    #[test]
    fn typing_coalesces_into_one_undo_group() {
        let (mut m, mut s) = (UndoManager::new(10), vec![]);
        m.set_groups_by_event(true);
        add(&mut m, &mut s, 1, "Typing");
        add(&mut m, &mut s, 2, "Typing");
        assert!(!m.can_undo(), "the event's group is still open");
        m.end_event();
        add(&mut m, &mut s, 3, "Typing");
        m.end_event();
        assert_eq!(m.steps(), 2);
        m.undo(&mut s);
        assert_eq!(s, vec![1, 2]);
        m.undo(&mut s);
        assert!(s.is_empty());
    }

    #[test]
    fn a_pause_in_typing_starts_a_new_undo_step() {
        let (mut m, mut s) = (UndoManager::new(10), vec![]);
        for (v, t) in [(1, 0.0), (2, 100.0), (3, 2000.0)] {
            s.push(v);
            m.register_coalesced("typing", t, 800.0, "Typing", |s: &mut Vec<i32>| { s.pop(); }, move |s: &mut Vec<i32>| s.push(v));
        }
        assert_eq!(m.steps(), 2);
        m.undo(&mut s);
        assert_eq!(s, vec![1, 2]);
        m.undo(&mut s);
        assert!(s.is_empty());
        m.redo(&mut s);
        assert_eq!(s, vec![1, 2], "a redo is not extended by new typing");
    }

    #[test]
    fn undoing_registers_the_redo() {
        let (mut m, mut s) = (UndoManager::new(10), vec![]);
        add(&mut m, &mut s, 1, "Add");
        assert!(!m.is_undoing());
        m.undo(&mut s);
        assert!(!m.is_undoing() && m.can_redo() && !m.can_undo());
        assert_eq!(m.redo_menu_item_title(), "Redo Add");
        let before = m.change_count();
        m.redo(&mut s);
        assert_eq!(s, vec![1]);
        assert_eq!(m.undo_menu_item_title(), "Undo Add", "redoing registers the undo again");
        assert_ne!(m.change_count(), before);
    }

    #[test]
    fn nested_groups_make_one_step_when_the_outermost_ends() {
        let (mut m, mut s) = (UndoManager::new(10), vec![]);
        m.begin_undo_grouping();
        add(&mut m, &mut s, 1, "");
        m.begin_undo_grouping();
        add(&mut m, &mut s, 2, "");
        m.end_undo_grouping();
        assert_eq!(m.grouping_level(), 1);
        assert!(!m.can_undo());
        m.set_action_name("Paste");
        m.end_undo_grouping();
        assert_eq!(m.steps(), 1);
        assert_eq!(m.undo(&mut s).as_deref(), Some("Paste"));
        assert!(s.is_empty());
    }

    #[test]
    fn levels_of_undo_limits_and_zero_is_unlimited() {
        let (mut m, mut s) = (UndoManager::new(10), vec![]);
        for v in 1..=5 {
            add(&mut m, &mut s, v, "");
        }
        m.set_levels_of_undo(2);
        assert_eq!((m.steps(), m.levels_of_undo()), (2, 2));
        m.set_levels_of_undo(0);
        for v in 6..=20 {
            add(&mut m, &mut s, v, "");
        }
        assert_eq!(m.steps(), 17);
        assert_eq!(m.levels_of_undo(), 0);
    }

    #[test]
    fn remove_all_actions_drops_only_that_target() {
        let (mut m, mut s) = (UndoManager::<Vec<i32>>::new(10), vec![]);
        m.register_for("a", "A", |_| {}, |_| {});
        m.register_for("b", "B", |_| {}, |_| {});
        m.register_for("a", "A2", |_| {}, |_| {});
        m.undo(&mut s);
        m.remove_all_actions("a");
        assert_eq!(m.steps(), 1);
        assert_eq!(m.undo_title().as_deref(), Some("Undo B"));
        assert!(!m.can_redo(), "the undone A2 went too");
    }
}
