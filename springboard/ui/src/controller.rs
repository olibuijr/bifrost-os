//! Bindings controllers. `ObjectController` holds one model object (a form or inspector edits it). `ArrayController`
//! holds a collection, arranges it with sort descriptors and a filter, and tracks the selection by content index, so a
//! list view reads the arranged objects and the selection and writes edits back through the controller.
//! Pure model: no views and no key-value paths. Objects edited in place must call `rearrange`, because nothing
//! observes their keys (gap: automatic rearrangement on key changes).
use std::cmp::Ordering;

type Comparator<T> = Box<dyn Fn(&T, &T) -> Ordering>;
type Predicate<T> = Box<dyn Fn(&T) -> bool>;

/// Holds one object (single-object controller): the content that is added, edited and removed as a whole.
#[derive(Clone, Debug, PartialEq)]
pub struct ObjectController<T> {
    content: Option<T>,
    /// `isEditable`: whether adding and removing the content is allowed.
    pub editable: bool,
}

impl<T> ObjectController<T> {
    pub fn new(content: Option<T>) -> Self {
        ObjectController { content, editable: true }
    }
    pub fn content(&self) -> Option<&T> {
        self.content.as_ref()
    }
    /// Replaces the content and returns the previous one.
    pub fn set_content(&mut self, content: Option<T>) -> Option<T> {
        std::mem::replace(&mut self.content, content)
    }
    /// `canAdd`: a new object may be created when the controller is editable and holds none.
    pub fn can_add(&self) -> bool {
        self.editable && self.content.is_none()
    }
    /// `add`: creates the object with `make` when `can_add`; returns whether it was added.
    pub fn add(&mut self, make: impl FnOnce() -> T) -> bool {
        if !self.can_add() {
            return false;
        }
        self.content = Some(make());
        true
    }
    /// `canRemove`: content may be removed when the controller is editable and holds one.
    pub fn can_remove(&self) -> bool {
        self.editable && self.content.is_some()
    }
    /// `remove`: takes the content out when allowed.
    pub fn remove(&mut self) -> Option<T> {
        if self.editable {
            self.content.take()
        } else {
            None
        }
    }
}

/// Holds a collection (array controller). `content` is the model array; `arranged` is the filtered and sorted view
/// of it; `selection` holds content indexes, so it survives sorting and filtering.
pub struct ArrayController<T> {
    content: Vec<T>,
    arranged: Vec<usize>,
    selection: Vec<usize>,
    sort: Vec<Comparator<T>>,
    filter: Option<Predicate<T>>,
    /// `avoidsEmptySelection`: keeps one arranged object selected while any is shown.
    pub avoids_empty_selection: bool,
    /// `preservesSelection`: keeps the selected indexes when the content is replaced.
    pub preserves_selection: bool,
    /// `selectsInsertedObjects`: an inserted object becomes the selection.
    pub selects_inserted: bool,
    /// `clearsFilterPredicateOnInsertion`: a filter would hide the new object, so it is removed.
    pub clears_filter_on_insert: bool,
}

/// Sort descriptors apply in order: the first one that does not tie decides.
fn compare<T>(sort: &[Comparator<T>], a: &T, b: &T) -> Ordering {
    for d in sort {
        let o = d(a, b);
        if o != Ordering::Equal {
            return o;
        }
    }
    Ordering::Equal
}

impl<T> ArrayController<T> {
    /// Defaults follow the reference controller: the selection is kept and never empty, inserted objects are not selected, and an insert
    /// clears the filter.
    pub fn new(content: Vec<T>) -> Self {
        let mut c = ArrayController {
            content,
            arranged: Vec::new(),
            selection: Vec::new(),
            sort: Vec::new(),
            filter: None,
            avoids_empty_selection: true,
            preserves_selection: true,
            selects_inserted: false,
            clears_filter_on_insert: true,
        };
        c.rearrange();
        c.enforce_avoid_empty();
        c
    }

    pub fn content(&self) -> &[T] {
        &self.content
    }
    /// Content indexes in arranged order.
    pub fn arranged_indexes(&self) -> &[usize] {
        &self.arranged
    }
    pub fn arranged_objects(&self) -> Vec<&T> {
        self.arranged.iter().map(|&i| &self.content[i]).collect()
    }
    /// Content indexes of the selection, ascending.
    pub fn selection_indexes(&self) -> &[usize] {
        &self.selection
    }
    /// `selectionIndex`: the first selected content index.
    pub fn selection_index(&self) -> Option<usize> {
        self.selection.first().copied()
    }
    pub fn selected_objects(&self) -> Vec<&T> {
        self.selection.iter().map(|&i| &self.content[i]).collect()
    }

    /// Adds a sort descriptor after the existing ones and rearranges.
    pub fn add_sort(&mut self, cmp: impl Fn(&T, &T) -> Ordering + 'static) {
        self.sort.push(Box::new(cmp));
        self.rearrange();
        self.enforce_avoid_empty();
    }
    pub fn clear_sort(&mut self) {
        self.sort.clear();
        self.rearrange();
        self.enforce_avoid_empty();
    }
    /// `filterPredicate`: only objects for which `keep` is true are arranged.
    pub fn set_filter(&mut self, keep: impl Fn(&T) -> bool + 'static) {
        self.filter = Some(Box::new(keep));
        self.rearrange();
        self.enforce_avoid_empty();
    }
    pub fn clear_filter(&mut self) {
        self.filter = None;
        self.rearrange();
        self.enforce_avoid_empty();
    }
    /// `rearrangeObjects`: recomputes the arranged order after an object changed in place.
    pub fn rearrange(&mut self) {
        let content = &self.content;
        let filter = &self.filter;
        let sort = &self.sort;
        let mut order: Vec<usize> =
            (0..content.len()).filter(|&i| filter.as_ref().map_or(true, |f| f(&content[i]))).collect();
        order.sort_by(|&a, &b| compare(sort, &content[a], &content[b]));
        self.arranged = order;
    }

    /// Replaces the whole content. The selection is kept only when `preserves_selection` is set.
    pub fn set_content(&mut self, content: Vec<T>) {
        self.content = content;
        if !self.preserves_selection {
            self.selection.clear();
        }
        let n = self.content.len();
        self.selection.retain(|&s| s < n);
        self.rearrange();
        self.enforce_avoid_empty();
    }

    /// `setSelectionIndexes`: out-of-range and repeated indexes are dropped. Returns whether the selection changed.
    pub fn set_selection_indexes(&mut self, mut indexes: Vec<usize>) -> bool {
        let before = self.selection.clone();
        indexes.retain(|&i| i < self.content.len());
        indexes.sort_unstable();
        indexes.dedup();
        self.selection = indexes;
        self.enforce_avoid_empty();
        before != self.selection
    }
    /// `addSelectionIndexes`
    pub fn add_selection_indexes(&mut self, indexes: &[usize]) -> bool {
        let mut all = self.selection.clone();
        all.extend_from_slice(indexes);
        self.set_selection_indexes(all)
    }
    /// `removeSelectionIndexes`
    pub fn remove_selection_indexes(&mut self, indexes: &[usize]) -> bool {
        let kept: Vec<usize> = self.selection.iter().copied().filter(|i| !indexes.contains(i)).collect();
        self.set_selection_indexes(kept)
    }

    /// The content index that `select_next` (forward) or `select_previous` would select.
    fn step(&self, forward: bool) -> Option<usize> {
        let cur = self.selection.first().and_then(|&s| self.arranged.iter().position(|&i| i == s));
        let next = match (cur, forward) {
            (Some(p), true) => p + 1,
            (Some(p), false) => p.checked_sub(1)?,
            (None, true) => 0,
            (None, false) => return None,
        };
        (next < self.arranged.len()).then(|| self.arranged[next])
    }
    /// `canSelectNext`
    pub fn can_select_next(&self) -> bool {
        self.step(true).is_some()
    }
    /// `selectNext`: selects the object after the current selection in arranged order.
    pub fn select_next(&mut self) -> bool {
        match self.step(true) {
            Some(i) => self.set_selection_indexes(vec![i]),
            None => false,
        }
    }
    /// `canSelectPrevious`
    pub fn can_select_previous(&self) -> bool {
        self.step(false).is_some()
    }
    /// `selectPrevious`: selects the object before the current selection in arranged order.
    pub fn select_previous(&mut self) -> bool {
        match self.step(false) {
            Some(i) => self.set_selection_indexes(vec![i]),
            None => false,
        }
    }

    /// `add`: appends an object. Returns its content index.
    pub fn add(&mut self, item: T) -> usize {
        let at = self.content.len();
        self.insert_content(at, item)
    }
    /// `insertAtArrangedObjectIndex`: inserts before the object at arranged position `arranged_index` (or at the end).
    /// Returns the content index of the new object.
    pub fn insert_at_arranged(&mut self, item: T, arranged_index: usize) -> usize {
        let at = self.arranged.get(arranged_index).copied().unwrap_or(self.content.len());
        self.insert_content(at, item)
    }
    fn insert_content(&mut self, at: usize, item: T) -> usize {
        if self.clears_filter_on_insert {
            self.filter = None;
        }
        self.content.insert(at, item);
        for s in self.selection.iter_mut() {
            if *s >= at {
                *s += 1;
            }
        }
        self.rearrange();
        if self.selects_inserted {
            self.selection = vec![at];
        }
        self.enforce_avoid_empty();
        at
    }

    /// `removeAtArrangedObjectIndex`: removes the object at an arranged position and returns it.
    pub fn remove_at_arranged(&mut self, arranged_index: usize) -> Option<T> {
        let at = *self.arranged.get(arranged_index)?;
        Some(self.remove_content(at))
    }
    /// `remove`: removes every selected object. Returns them in content order.
    pub fn remove_selected(&mut self) -> Vec<T> {
        let mut desc = self.selection.clone();
        desc.sort_unstable_by(|a, b| b.cmp(a));
        let mut out: Vec<T> = desc.into_iter().map(|i| self.remove_content(i)).collect();
        out.reverse();
        out
    }
    fn remove_content(&mut self, at: usize) -> T {
        let item = self.content.remove(at);
        self.selection.retain(|&s| s != at);
        for s in self.selection.iter_mut() {
            if *s > at {
                *s -= 1;
            }
        }
        self.rearrange();
        self.enforce_avoid_empty();
        item
    }

    /// Selects the first arranged object when the selection is empty and `avoids_empty_selection` is set.
    fn enforce_avoid_empty(&mut self) {
        if self.avoids_empty_selection && self.selection.is_empty() {
            if let Some(&first) = self.arranged.first() {
                self.selection = vec![first];
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type Row = (&'static str, u32);

    fn names<T: Copy>(v: Vec<&T>) -> Vec<T> {
        v.into_iter().copied().collect()
    }

    #[test]
    fn object_controller_adds_and_removes_content_when_editable() {
        let mut c: ObjectController<u32> = ObjectController::new(None);
        assert!(c.can_add() && !c.can_remove());
        assert!(c.add(|| 5));
        assert!(!c.add(|| 9), "a second object is refused while one is held");
        assert_eq!(c.content(), Some(&5));
        assert_eq!(c.remove(), Some(5));
        assert_eq!(c.content(), None);
        c.editable = false;
        assert!(!c.add(|| 7));
        c.set_content(Some(3));
        assert_eq!(c.remove(), None, "a read-only controller keeps its content");
        assert_eq!(c.content(), Some(&3));
    }

    #[test]
    fn arranges_by_filter_then_sort_descriptors_in_order() {
        let mut c = ArrayController::new(vec![("bo", 30), ("al", 17), ("cy", 30), ("di", 12)]);
        c.add_sort(|a: &Row, b: &Row| a.1.cmp(&b.1));
        c.add_sort(|a: &Row, b: &Row| a.0.cmp(b.0));
        c.set_filter(|r: &Row| r.1 >= 17);
        assert_eq!(names(c.arranged_objects()), vec![("al", 17), ("bo", 30), ("cy", 30)]);
        c.clear_filter();
        assert_eq!(c.arranged_objects().len(), 4);
        assert_eq!(c.arranged_objects()[0], &("di", 12));
    }

    #[test]
    fn insert_shifts_selection_so_it_keeps_pointing_at_the_same_object() {
        let mut c = ArrayController::new(vec!["a", "b", "c"]);
        assert!(c.set_selection_indexes(vec![2]));
        assert_eq!(c.selected_objects(), vec![&"c"]);
        c.insert_at_arranged("z", 0);
        assert_eq!(c.selection_indexes(), &[3]);
        assert_eq!(c.selected_objects(), vec![&"c"]);
        c.add("d");
        assert_eq!(c.selected_objects(), vec![&"c"], "appending does not move the selection");
    }

    #[test]
    fn selects_inserted_object_only_when_asked() {
        let mut c = ArrayController::new(vec![1, 2]);
        c.set_selection_indexes(vec![0]);
        c.add(9);
        assert_eq!(c.selected_objects(), vec![&1]);
        c.selects_inserted = true;
        let at = c.add(10);
        assert_eq!(c.selection_indexes(), &[at]);
        assert_eq!(c.selected_objects(), vec![&10]);
    }

    #[test]
    fn removing_selected_objects_returns_them_and_keeps_a_selection() {
        let mut c = ArrayController::new(vec![1, 2, 3, 4]);
        c.set_selection_indexes(vec![1, 2]);
        assert_eq!(c.remove_selected(), vec![2, 3]);
        assert_eq!(c.content(), &[1, 4]);
        assert_eq!(c.selection_indexes(), &[0], "avoids an empty selection by selecting the first object");
        let mut keep = ArrayController::new(vec![1, 2]);
        keep.avoids_empty_selection = false;
        keep.set_selection_indexes(vec![0]);
        keep.remove_selected();
        assert!(keep.selection_indexes().is_empty());
    }

    #[test]
    fn select_next_and_previous_walk_the_arranged_order() {
        let mut c = ArrayController::new(vec![5, 1, 3]);
        // avoiding an empty selection selects the first content object (5); sorting keeps it, now last
        c.add_sort(|a: &i32, b: &i32| a.cmp(b));
        assert_eq!(c.selected_objects(), vec![&5]);
        assert!(!c.can_select_next(), "the last arranged object has no successor");
        assert!(c.select_previous());
        assert!(c.select_previous());
        assert_eq!(c.selected_objects(), vec![&1]);
        assert!(!c.can_select_previous(), "the first arranged object has no predecessor");
        assert!(c.select_next());
        assert_eq!(c.selected_objects(), vec![&3]);
    }

    #[test]
    fn insert_clears_a_filter_that_would_hide_the_new_object() {
        let mut c = ArrayController::new(vec!["keep", "drop"]);
        c.set_filter(|s: &&str| *s != "new-hidden");
        c.clears_filter_on_insert = false;
        c.add("new-hidden");
        assert_eq!(c.arranged_objects().len(), 2, "the filter stays when the flag is off");
        c.clears_filter_on_insert = true;
        c.add("another");
        assert_eq!(c.arranged_objects().len(), 4, "the filter is cleared when the flag is on");
    }
}
