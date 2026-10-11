//! The pointer shape an app asks for (`NativeCursor`): a stack where `push` makes a shape current and `pop` returns to the
//! one below, plus a hide count where each `unhide` negates one earlier `hide`. The shell turns the current shape into
//! the pointer image; this module keeps the stack and the counts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Cursor {
    #[default]
    Arrow,
    IBeam,
    IBeamVertical,
    Crosshair,
    ClosedHand,
    OpenHand,
    PointingHand,
    ResizeLeft,
    ResizeRight,
    ResizeLeftRight,
    ResizeUp,
    ResizeDown,
    ResizeUpDown,
    ColumnResize,
    RowResize,
    OperationNotAllowed,
    ContextualMenu,
    DragCopy,
    DragLink,
    DisappearingItem,
    ZoomIn,
    ZoomOut,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CursorStack {
    stack: Vec<Cursor>,
    hides: u32,
}

impl CursorStack {
    pub fn new() -> Self {
        CursorStack::default()
    }

    /// The shape on top of the stack, or the arrow when the stack is empty (`NativeCursor.current`).
    pub fn current(&self) -> Cursor {
        self.stack.last().copied().unwrap_or_default()
    }

    /// Makes `cursor` current on top of the stack (`push`).
    pub fn push(&mut self, cursor: Cursor) {
        self.stack.push(cursor);
    }

    /// Removes the current shape and returns it (`pop`). An empty stack returns None and leaves the arrow current.
    pub fn pop(&mut self) -> Option<Cursor> {
        self.stack.pop()
    }

    /// Replaces the current shape without growing the stack (`set`). On an empty stack this pushes one shape.
    pub fn set(&mut self, cursor: Cursor) {
        match self.stack.last_mut() {
            Some(top) => *top = cursor,
            None => self.stack.push(cursor),
        }
    }

    /// The number of shapes pushed and not yet popped.
    pub fn depth(&self) -> usize {
        self.stack.len()
    }

    /// Hides the pointer (`hide`). Calls nest: the pointer shows again after as many `unhide` calls.
    pub fn hide(&mut self) {
        self.hides += 1;
    }

    /// Negates one earlier `hide` (`unhide`). Extra calls do nothing.
    pub fn unhide(&mut self) {
        self.hides = self.hides.saturating_sub(1);
    }

    pub fn is_visible(&self) -> bool {
        self.hides == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_stack_shows_the_arrow() {
        let mut stack = CursorStack::new();
        assert_eq!(stack.current(), Cursor::Arrow);
        assert_eq!(stack.depth(), 0);
        assert_eq!(stack.pop(), None);
        assert_eq!(stack.current(), Cursor::Arrow);
    }

    #[test]
    fn push_and_pop_restore_the_shape_below() {
        let mut stack = CursorStack::new();
        stack.push(Cursor::IBeam);
        stack.push(Cursor::ResizeLeftRight);
        assert_eq!(stack.current(), Cursor::ResizeLeftRight);
        assert_eq!(stack.depth(), 2);
        assert_eq!(stack.pop(), Some(Cursor::ResizeLeftRight));
        assert_eq!(stack.current(), Cursor::IBeam);
        assert_eq!(stack.pop(), Some(Cursor::IBeam));
        assert_eq!(stack.current(), Cursor::Arrow);
    }

    #[test]
    fn set_replaces_the_top_without_growing_the_stack() {
        let mut stack = CursorStack::new();
        stack.set(Cursor::Crosshair);
        assert_eq!((stack.depth(), stack.current()), (1, Cursor::Crosshair));
        stack.push(Cursor::OpenHand);
        stack.set(Cursor::ClosedHand);
        assert_eq!(stack.depth(), 2);
        assert_eq!(stack.current(), Cursor::ClosedHand);
        assert_eq!(stack.pop(), Some(Cursor::ClosedHand));
        assert_eq!(stack.current(), Cursor::Crosshair);
    }

    #[test]
    fn hide_calls_nest_and_extra_unhides_do_nothing() {
        let mut stack = CursorStack::new();
        assert!(stack.is_visible());
        stack.hide();
        stack.hide();
        stack.unhide();
        assert!(!stack.is_visible());
        stack.unhide();
        assert!(stack.is_visible());
        stack.unhide();
        assert!(stack.is_visible());
        stack.hide();
        assert!(!stack.is_visible());
    }

    #[test]
    fn hiding_does_not_change_the_shape() {
        let mut stack = CursorStack::new();
        stack.push(Cursor::PointingHand);
        stack.hide();
        assert_eq!(stack.current(), Cursor::PointingHand);
        assert_eq!(stack.depth(), 1);
    }
}
