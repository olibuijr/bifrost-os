//! Onscreen context for app entities: an app annotates a view with the entity it shows, and the system asks which
//! annotated views are visible and selected (the reference SDK's view annotations used by intent tests). Pure model:
//! the shell or the app reports each view's visibility and selection; nothing here draws or reads a window.

use std::collections::BTreeMap;

/// The entity a view shows: the entity type's name and the entity's identifier.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct EntityRef {
    pub type_name: String,
    pub id: String,
}

/// One visible annotated view, as a query returns it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewAnnotation {
    pub entity: EntityRef,
    pub is_selected: bool,
}

/// Why an annotation is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnnotationError {
    /// The entity type name is empty.
    EmptyType,
    /// The entity identifier is empty.
    EmptyId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Entry {
    entity: EntityRef,
    selected: bool,
    visible: bool,
}

/// The annotations of the views in one window, keyed by a view number chosen by the caller.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OnscreenContext {
    views: BTreeMap<u64, Entry>,
}

impl OnscreenContext {
    pub fn new() -> OnscreenContext {
        OnscreenContext::default()
    }

    /// Annotates `view` with `entity`. A view that already has an annotation gets the new one. New annotations start
    /// visible.
    pub fn annotate(&mut self, view: u64, entity: EntityRef, selected: bool) -> Result<(), AnnotationError> {
        if entity.type_name.trim().is_empty() {
            return Err(AnnotationError::EmptyType);
        }
        if entity.id.trim().is_empty() {
            return Err(AnnotationError::EmptyId);
        }
        self.views.insert(view, Entry { entity, selected, visible: true });
        Ok(())
    }

    /// Removes the annotation of `view`. True when there was one.
    pub fn remove(&mut self, view: u64) -> bool {
        self.views.remove(&view).is_some()
    }

    /// Sets whether an annotated view is on screen. False when `view` has no annotation.
    pub fn set_visible(&mut self, view: u64, visible: bool) -> bool {
        match self.views.get_mut(&view) {
            Some(e) => {
                e.visible = visible;
                true
            }
            None => false,
        }
    }

    /// Sets whether an annotated view is selected. False when `view` has no annotation.
    pub fn set_selected(&mut self, view: u64, selected: bool) -> bool {
        match self.views.get_mut(&view) {
            Some(e) => {
                e.selected = selected;
                true
            }
            None => false,
        }
    }

    /// The annotations of the visible views, in view-number order. Hidden views are left out.
    pub fn view_annotations(&self) -> Vec<ViewAnnotation> {
        self.views
            .values()
            .filter(|e| e.visible)
            .map(|e| ViewAnnotation { entity: e.entity.clone(), is_selected: e.selected })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(id: &str) -> EntityRef {
        EntityRef { type_name: "Note".to_string(), id: id.to_string() }
    }

    #[test]
    fn visible_annotations_come_in_view_order_with_selection() {
        let mut ctx = OnscreenContext::new();
        ctx.annotate(20, note("b"), true).unwrap();
        ctx.annotate(10, note("a"), false).unwrap();
        let got = ctx.view_annotations();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0], ViewAnnotation { entity: note("a"), is_selected: false });
        assert_eq!(got[1], ViewAnnotation { entity: note("b"), is_selected: true });
    }

    #[test]
    fn hidden_views_are_left_out_until_shown_again() {
        let mut ctx = OnscreenContext::new();
        ctx.annotate(1, note("a"), false).unwrap();
        ctx.annotate(2, note("b"), false).unwrap();
        assert!(ctx.set_visible(2, false));
        assert_eq!(ctx.view_annotations().len(), 1);
        assert!(ctx.set_visible(2, true));
        assert_eq!(ctx.view_annotations().len(), 2);
    }

    #[test]
    fn selection_changes_are_reported() {
        let mut ctx = OnscreenContext::new();
        ctx.annotate(5, note("c"), false).unwrap();
        assert!(ctx.set_selected(5, true));
        assert!(ctx.view_annotations()[0].is_selected);
        assert!(!ctx.set_selected(9, true));
    }

    #[test]
    fn remove_drops_the_annotation() {
        let mut ctx = OnscreenContext::new();
        ctx.annotate(1, note("a"), false).unwrap();
        ctx.annotate(2, note("b"), false).unwrap();
        assert!(ctx.remove(1));
        assert!(!ctx.remove(1));
        assert_eq!(ctx.view_annotations()[0].entity.id, "b");
    }

    #[test]
    fn a_new_annotation_replaces_the_old_one_for_the_same_view() {
        let mut ctx = OnscreenContext::new();
        ctx.annotate(3, note("old"), true).unwrap();
        ctx.annotate(3, note("new"), false).unwrap();
        let got = ctx.view_annotations();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].entity.id, "new");
        assert!(!got[0].is_selected);
    }

    #[test]
    fn empty_type_or_id_is_refused() {
        let mut ctx = OnscreenContext::new();
        let no_type = EntityRef { type_name: " ".to_string(), id: "x".to_string() };
        assert_eq!(ctx.annotate(1, no_type, false), Err(AnnotationError::EmptyType));
        assert_eq!(ctx.annotate(1, note(""), false), Err(AnnotationError::EmptyId));
        assert!(ctx.view_annotations().is_empty());
    }
}
