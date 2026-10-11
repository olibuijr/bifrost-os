//! An undo manager (`NativeUndoManager`): commands with an inverse, grouped, with a bounded history and a redo stack.

struct Entry<T> {
    name: String,
    undo: Vec<Box<dyn Fn(&mut T)>>,
    redo: Vec<Box<dyn Fn(&mut T)>>,
}

/// Undo history over a state `T`. Register what you just did together with how to reverse it.
pub struct UndoManager<T> {
    done: Vec<Entry<T>>,
    undone: Vec<Entry<T>>,
    open: Option<Entry<T>>,
    limit: usize,
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
        UndoManager { done: Vec::new(), undone: Vec::new(), open: None, limit: limit.max(1), position: 0, epoch: 0 }
    }
    /// Records an action. `undo` reverses it and `redo` repeats it. Inside a group the pair joins the group;
    /// otherwise it is its own step. A new action clears the redo history.
    pub fn register(&mut self, name: &str, undo: impl Fn(&mut T) + 'static, redo: impl Fn(&mut T) + 'static) {
        if !self.undone.is_empty() {
            self.undone.clear();
            self.epoch += 1;
        }
        match self.open.as_mut() {
            Some(g) => {
                g.undo.push(Box::new(undo));
                g.redo.push(Box::new(redo));
            }
            None => {
                self.position += 1;
                self.push(Entry { name: name.into(), undo: vec![Box::new(undo)], redo: vec![Box::new(redo)] });
            }
        }
    }
    /// Starts a group: everything registered until `end_group` is one undo step (`beginUndoGrouping`).
    pub fn begin_group(&mut self, name: &str) {
        if self.open.is_none() {
            self.open = Some(Entry { name: name.into(), undo: Vec::new(), redo: Vec::new() });
        }
    }
    pub fn end_group(&mut self) {
        if let Some(g) = self.open.take() {
            if !g.undo.is_empty() {
                self.position += 1;
                self.push(g);
            }
        }
    }
    fn push(&mut self, e: Entry<T>) {
        self.done.push(e);
        if self.done.len() > self.limit {
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
    /// Reverses the latest step (a group's parts in reverse order). Returns its name.
    pub fn undo(&mut self, state: &mut T) -> Option<String> {
        self.end_group();
        let e = self.done.pop()?;
        self.position -= 1;
        e.undo.iter().rev().for_each(|f| f(state));
        let name = e.name.clone();
        self.undone.push(e);
        Some(name)
    }
    pub fn redo(&mut self, state: &mut T) -> Option<String> {
        let e = self.undone.pop()?;
        self.position += 1;
        e.redo.iter().for_each(|f| f(state));
        let name = e.name.clone();
        self.done.push(e);
        Some(name)
    }
    /// Forgets all history (`removeAllActions`), e.g. after saving a document that cannot be reopened.
    pub fn clear(&mut self) {
        self.done.clear();
        self.undone.clear();
        self.open = None;
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
}
