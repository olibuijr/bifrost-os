//! Cursor stack (`NativeCursor`): the shape the pointer shows is the top of a stack that code pushes and pops, plus a
//! hide count and a hide-until-moved flag. Pure model: the compositor turns `Kind::shape` into its cursor-shape request.


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
    ResizeNorthEastSouthWest,
    ResizeNorthWestSouthEast,
    ColumnResize,
    RowResize,
    Move,
    Rotate,
    ScreenshotWindow,
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
        Kind::ResizeNorthEastSouthWest,
        Kind::ResizeNorthWestSouthEast,
        Kind::ColumnResize,
        Kind::RowResize,
        Kind::Move,
        Kind::Rotate,
        Kind::ScreenshotWindow,
    ];

    /// The name apps send over the `cursor` service (`i_beam`, `resize_left_right`, ...). [`Kind::from_name`] reads it back.
    pub fn name(self) -> &'static str {
        match self {
            Kind::Arrow => "arrow",
            Kind::IBeam => "i_beam",
            Kind::Crosshair => "crosshair",
            Kind::OpenHand => "open_hand",
            Kind::ClosedHand => "closed_hand",
            Kind::PointingHand => "pointing_hand",
            Kind::ContextualMenu => "contextual_menu",
            Kind::OperationNotAllowed => "operation_not_allowed",
            Kind::DragCopy => "drag_copy",
            Kind::DragLink => "drag_link",
            Kind::DisappearingItem => "disappearing_item",
            Kind::ZoomIn => "zoom_in",
            Kind::ZoomOut => "zoom_out",
            Kind::ResizeLeftRight => "resize_left_right",
            Kind::ResizeUpDown => "resize_up_down",
            Kind::ResizeLeft => "resize_left",
            Kind::ResizeRight => "resize_right",
            Kind::ResizeUp => "resize_up",
            Kind::ResizeDown => "resize_down",
            Kind::ResizeNorthEastSouthWest => "resize_north_east_south_west",
            Kind::ResizeNorthWestSouthEast => "resize_north_west_south_east",
            Kind::ColumnResize => "column_resize",
            Kind::RowResize => "row_resize",
            Kind::Move => "move",
            Kind::Rotate => "rotate",
            Kind::ScreenshotWindow => "screenshot_window",
        }
    }

    /// The kind called `name` (as [`Kind::name`] spells it), or `None` for an unknown name.
    pub fn from_name(name: &str) -> Option<Kind> {
        Kind::ALL.iter().copied().find(|k| k.name() == name)
    }

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
            Kind::ResizeNorthEastSouthWest => "nesw_resize",
            Kind::ResizeNorthWestSouthEast => "nwse_resize",
            Kind::ColumnResize => "col_resize",
            Kind::RowResize => "row_resize",
            Kind::Move => "move",
            Kind::Rotate => "all_scroll",
            Kind::ScreenshotWindow => "cell",
        }
    }
}

/// Which edge or corner of a frame the pointer is on (`NSCursor.FrameResizePosition`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FrameResizePosition {
    Top,
    Left,
    Bottom,
    Right,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

macro_rules! direction_set {
    ($(#[$doc:meta])* $name:ident { $($flag:ident = $bit:expr),+ }) => {
        $(#[$doc])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub struct $name(pub u8);
        impl $name {
            $(pub const $flag: $name = $name($bit);)+
            pub const ALL: $name = $name(0 $(| $bit)+);
            pub fn contains(self, other: $name) -> bool {
                other.0 != 0 && self.0 & other.0 == other.0
            }
        }
    };
}

direction_set!(
    /// Directions an edge can still move (`NSCursor.FrameResizeDirections`).
    FrameResizeDirections { INWARD = 1, OUTWARD = 2 }
);
direction_set!(
    /// `NSCursor.ColumnResizeDirections`.
    ColumnResizeDirections { LEFT = 1, RIGHT = 2 }
);
direction_set!(
    /// `NSCursor.RowResizeDirections`.
    RowResizeDirections { UP = 1, DOWN = 2 }
);

impl Kind {
    /// `NSCursor.frameResize(position:directions:)`. A one-way edge shows the one-sided arrow; corners only have
    /// diagonal shapes. No direction at all shows the arrow.
    pub fn frame_resize(position: FrameResizePosition, directions: FrameResizeDirections) -> Kind {
        use FrameResizePosition::*;
        let inward = directions.contains(FrameResizeDirections::INWARD);
        let outward = directions.contains(FrameResizeDirections::OUTWARD);
        if !inward && !outward {
            return Kind::Arrow;
        }
        let both = inward && outward;
        match position {
            TopLeft | BottomRight => Kind::ResizeNorthWestSouthEast,
            TopRight | BottomLeft => Kind::ResizeNorthEastSouthWest,
            Top if both => Kind::ResizeUpDown,
            Bottom if both => Kind::ResizeUpDown,
            Left if both => Kind::ResizeLeftRight,
            Right if both => Kind::ResizeLeftRight,
            Top => if outward { Kind::ResizeUp } else { Kind::ResizeDown },
            Bottom => if outward { Kind::ResizeDown } else { Kind::ResizeUp },
            Left => if outward { Kind::ResizeLeft } else { Kind::ResizeRight },
            Right => if outward { Kind::ResizeRight } else { Kind::ResizeLeft },
        }
    }

    /// `NSCursor.columnResize(directions:)`.
    pub fn column_resize(directions: ColumnResizeDirections) -> Kind {
        let (l, r) = (directions.contains(ColumnResizeDirections::LEFT), directions.contains(ColumnResizeDirections::RIGHT));
        match (l, r) {
            (true, true) => Kind::ColumnResize,
            (true, false) => Kind::ResizeLeft,
            (false, true) => Kind::ResizeRight,
            _ => Kind::Arrow,
        }
    }

    /// `NSCursor.rowResize(directions:)`.
    pub fn row_resize(directions: RowResizeDirections) -> Kind {
        let (u, d) = (directions.contains(RowResizeDirections::UP), directions.contains(RowResizeDirections::DOWN));
        match (u, d) {
            (true, true) => Kind::RowResize,
            (true, false) => Kind::ResizeUp,
            (false, true) => Kind::ResizeDown,
            _ => Kind::Arrow,
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
    use std::collections::HashSet;

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
    fn every_kind_round_trips_through_its_service_name() {
        let names: HashSet<&str> = Kind::ALL.iter().map(|k| k.name()).collect();
        assert_eq!(names.len(), Kind::ALL.len(), "names are unique");
        for &k in Kind::ALL {
            assert_eq!(Kind::from_name(k.name()), Some(k));
        }
        assert_eq!(Kind::from_name("i_beam"), Some(Kind::IBeam));
        assert_eq!(Kind::from_name("text"), None, "the protocol spelling is not a service name");
        assert_eq!(Kind::from_name(""), None);
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
    #[test]
    fn the_new_shapes_round_trip_and_have_protocol_names() {
        assert_eq!(Kind::from_name("move"), Some(Kind::Move));
        assert_eq!(Kind::from_name("screenshot_window"), Some(Kind::ScreenshotWindow));
        assert_eq!(Kind::ResizeNorthEastSouthWest.shape(), "nesw_resize");
        assert_eq!(Kind::ColumnResize.shape(), "col_resize");
        assert_eq!(Kind::RowResize.shape(), "row_resize");
        assert!(Kind::ALL.contains(&Kind::Rotate));
    }

    #[test]
    fn frame_resize_covers_all_positions_and_directions() {
        use FrameResizeDirections as D;
        use FrameResizePosition::*;
        let all = D::ALL;
        assert_eq!(Kind::frame_resize(Top, all), Kind::ResizeUpDown);
        assert_eq!(Kind::frame_resize(Bottom, all), Kind::ResizeUpDown);
        assert_eq!(Kind::frame_resize(Left, all), Kind::ResizeLeftRight);
        assert_eq!(Kind::frame_resize(Right, all), Kind::ResizeLeftRight);
        assert_eq!(Kind::frame_resize(Top, D::OUTWARD), Kind::ResizeUp);
        assert_eq!(Kind::frame_resize(Top, D::INWARD), Kind::ResizeDown);
        assert_eq!(Kind::frame_resize(Bottom, D::OUTWARD), Kind::ResizeDown);
        assert_eq!(Kind::frame_resize(Bottom, D::INWARD), Kind::ResizeUp);
        assert_eq!(Kind::frame_resize(Left, D::OUTWARD), Kind::ResizeLeft);
        assert_eq!(Kind::frame_resize(Left, D::INWARD), Kind::ResizeRight);
        assert_eq!(Kind::frame_resize(Right, D::OUTWARD), Kind::ResizeRight);
        assert_eq!(Kind::frame_resize(Right, D::INWARD), Kind::ResizeLeft);
        for d in [D::INWARD, D::OUTWARD, all] {
            assert_eq!(Kind::frame_resize(TopLeft, d), Kind::ResizeNorthWestSouthEast);
            assert_eq!(Kind::frame_resize(BottomRight, d), Kind::ResizeNorthWestSouthEast);
            assert_eq!(Kind::frame_resize(TopRight, d), Kind::ResizeNorthEastSouthWest);
            assert_eq!(Kind::frame_resize(BottomLeft, d), Kind::ResizeNorthEastSouthWest);
        }
        assert_eq!(Kind::frame_resize(Top, D(0)), Kind::Arrow);
    }

    #[test]
    fn column_and_row_resize_follow_their_directions() {
        assert_eq!(Kind::column_resize(ColumnResizeDirections::ALL), Kind::ColumnResize);
        assert_eq!(Kind::column_resize(ColumnResizeDirections::LEFT), Kind::ResizeLeft);
        assert_eq!(Kind::column_resize(ColumnResizeDirections::RIGHT), Kind::ResizeRight);
        assert_eq!(Kind::row_resize(RowResizeDirections::ALL), Kind::RowResize);
        assert_eq!(Kind::row_resize(RowResizeDirections::UP), Kind::ResizeUp);
        assert_eq!(Kind::row_resize(RowResizeDirections::DOWN), Kind::ResizeDown);
        assert_eq!(Kind::row_resize(RowResizeDirections(0)), Kind::Arrow);
    }
}
