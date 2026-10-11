//! Cursor stack (`NativeCursor`): the shape the pointer shows is the top of a stack that code pushes and pops, plus a
//! hide count and a hide-until-moved flag. Pure model: the compositor turns `Kind::shape` into its cursor-shape request.

use std::collections::HashSet;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Kind {
    #[default]
    Arrow,
    IBeam,
    Crosshair,
    OpenHand,
    ClosedHand,
    PointingHand,
    ContextualMenu,
    OperationNotAllowed,
    DragCopy,
    DragLink,
    DisappearingItem,
    ZoomIn,
    ZoomOut,
    ResizeLeftRight,
    ResizeUpDown,
    ResizeLeft,
    ResizeRight,
    ResizeUp,
    ResizeDown,
}

impl Kind {
    pub const ALL: &'static [Kind] = &[
        Kind::Arrow,
        Kind::IBeam,
        Kind::Crosshair,
        Kind::OpenHand,
        Kind::ClosedHand,
        Kind::PointingHand,
        Kind::ContextualMenu,
        Kind::OperationNotAllowed,
        Kind::DragCopy,
        Kind::DragLink,
        Kind::DisappearingItem,
        Kind::ZoomIn,
        Kind::ZoomOut,
        Kind::ResizeLeftRight,
        Kind::ResizeUpDown,
        Kind::ResizeLeft,
        Kind::ResizeRight,
        Kind::ResizeUp,
        Kind::ResizeDown,
    ];

    /// The shape name in the cursor-shape protocol's spelling (`text`, `ew_resize`, ...).
    pub fn shape(self) -> &'static str {
        match self {
            Kind::Arrow => "default",
            Kind::IBeam => "text",
            Kind::Crosshair => "crosshair",
            Kind::OpenHand => "grab",
            Kind::ClosedHand => "grabbing",
            Kind::PointingHand => "pointer",
            Kind::ContextualMenu => "context_menu",
            Kind::OperationNotAllowed => "not_allowed",
            Kind::DragCopy => "copy",
            Kind::DragLink => "alias",
            Kind::DisappearingItem => "no_drop",
            Kind::ZoomIn => "zoom_in",
            Kind::ZoomOut => "zoom_out",
            Kind::ResizeLeftRight => "ew_resize",
            Kind::ResizeUpDown => "ns_resize",
            Kind::ResizeLeft => "w_resize",
            Kind::ResizeRight => "e_resize",
            Kind::ResizeUp => "n_resize",
            Kind::ResizeDown => "s_resize",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cursors {
    stack: Vec<Kind>,
    hidden: u32,
    hidden_until_moved: bool,
}

impl Cursors {
    pub fn new() -> Self {
        Self::default()
    }

    /// The shape on screen: the top of the stack, or the arrow when nothing was pushed.
    pub fn current(&self) -> Kind {
        self.stack.last().copied().unwrap_or_default()
    }

    pub fn depth(&self) -> usize {
        self.stack.len()
    }

    /// Puts a shape on top and makes it current.
    pub fn push(&mut self, kind: Kind) {
        self.stack.push(kind);
    }

    /// Removes the current shape; the one below becomes current. Popping an empty stack does nothing.
    pub fn pop(&mut self) -> Option<Kind> {
        self.stack.pop()
    }

    /// Replaces the current shape without growing the stack. With an empty stack this pushes it.
    pub fn set(&mut self, kind: Kind) {
        match self.stack.last_mut() {
            Some(top) => *top = kind,
            None => self.stack.push(kind),
        }
    }

    /// Hides are counted: each `hide` needs its own `unhide`, and an extra `unhide` does nothing.
    pub fn hide(&mut self) {
        self.hidden += 1;
    }

    pub fn unhide(&mut self) {
        self.hidden = self.hidden.saturating_sub(1);
    }

    /// Hidden until the next pointer move, whatever the hide count is.
    pub fn set_hidden_until_mouse_moves(&mut self, on: bool) {
        self.hidden_until_moved = on;
    }

    /// The pointer moved: a cursor hidden until the next move shows again.
    pub fn mouse_moved(&mut self) {
        self.hidden_until_moved = false;
    }

    pub fn is_visible(&self) -> bool {
        self.hidden == 0 && !self.hidden_until_moved
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_stack_shows_the_arrow_and_push_pop_is_last_in_first_out() {
        let mut c = Cursors::new();
        assert_eq!(c.current(), Kind::Arrow);
        assert_eq!(c.pop(), None, "popping an empty stack changes nothing");
        c.push(Kind::IBeam);
        c.push(Kind::Crosshair);
        assert_eq!((c.current(), c.depth()), (Kind::Crosshair, 2));
        assert_eq!(c.pop(), Some(Kind::Crosshair));
        assert_eq!(c.current(), Kind::IBeam, "the one below becomes current");
        c.pop();
        assert_eq!(c.current(), Kind::Arrow);
    }

    #[test]
    fn set_replaces_the_top_instead_of_growing_the_stack() {
        let mut c = Cursors::new();
        c.set(Kind::ZoomIn);
        assert_eq!((c.current(), c.depth()), (Kind::ZoomIn, 1));
        c.push(Kind::OpenHand);
        c.set(Kind::ClosedHand);
        assert_eq!((c.current(), c.depth()), (Kind::ClosedHand, 2));
        c.pop();
        assert_eq!(c.current(), Kind::ZoomIn, "the shape below was not touched");
    }

    #[test]
    fn hide_and_unhide_nest() {
        let mut c = Cursors::new();
        assert!(c.is_visible());
        c.hide();
        c.hide();
        c.unhide();
        assert!(!c.is_visible(), "one hide is still outstanding");
        c.unhide();
        assert!(c.is_visible());
        c.unhide();
        assert!(c.is_visible(), "an extra unhide does not go negative");
        c.hide();
        assert!(!c.is_visible(), "and the next hide still counts");
    }

    #[test]
    fn hidden_until_mouse_moves_returns_on_the_next_move() {
        let mut c = Cursors::new();
        c.set_hidden_until_mouse_moves(true);
        assert!(!c.is_visible());
        c.mouse_moved();
        assert!(c.is_visible());
        c.hide();
        c.set_hidden_until_mouse_moves(true);
        c.mouse_moved();
        assert!(!c.is_visible(), "a move does not undo a hide");
    }

    #[test]
    fn every_kind_has_its_own_shape_name() {
        let names: HashSet<&str> = Kind::ALL.iter().map(|k| k.shape()).collect();
        assert_eq!(names.len(), Kind::ALL.len());
        assert_eq!(Kind::IBeam.shape(), "text");
        assert_eq!(Kind::ResizeLeftRight.shape(), "ew_resize");
        assert_eq!(Kind::default(), Kind::Arrow);
        assert_eq!(Kind::default().shape(), "default");
    }
}
