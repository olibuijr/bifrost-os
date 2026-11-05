//! Drag and drop in the retained view kit (the reference dragging source and destination, and registering for dragged types).
//!
//! A view is a drag source when [`View::dnd_source`] returns a [`DragSpec`]. The window watches presses: a press on (or
//! inside) a source that moves past [`DRAG_SLOP`] points is a drag, the control that held the press is cancelled, and
//! [`ViewApp::event`](super::ViewApp::event) starts the session with [`crate::dnd::begin`]. A view is a destination when
//! [`View::dnd_types`] lists types: the app registers their union with the shell, and the session's events
//! ([`ViewApp::drag`](super::ViewApp::drag), from `App::drag`) find the deepest accepting view under the pointer and call
//! its `dnd_over`, `dnd_exited` and `dnd_perform`. The window answers the shell with one operation that the source allows
//! (copy over link over move). The source view hears about the end of its session in `dnd_ended`.
//!
//! The items are the shell's ([`DragItem`]): text, links and granted files. Coordinates reach the views as window points.

use super::{at, at_mut, path_offset, path_to, pick, View, ViewEvent, Window};
use crate::proto::{dragop, DragItem, DragPhase, DragPreview};
use bifrost_ui::responder::Responder;

/// A press that moves this far (window points) from where it began is a drag (`bifrost_ui::gesture::SLOP`).
pub const DRAG_SLOP: f32 = bifrost_ui::gesture::SLOP;

/// What a drag source offers: the items, the operations it allows (`dragop` bits) and the drag image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DragSpec {
    pub items: Vec<DragItem>,
    pub allowed: u8,
    pub preview: DragPreview,
}

impl DragSpec {
    /// Offers `items`, allowing every operation and showing no drag image.
    pub fn new(items: Vec<DragItem>) -> Self {
        DragSpec { items, allowed: dragop::ALL, preview: DragPreview::default() }
    }

    /// Limits the operations the session allows (`dragop` bits).
    pub fn allowed(mut self, allowed: u8) -> Self {
        self.allowed = allowed;
        self
    }

    /// Sets the drag image.
    pub fn preview(mut self, preview: DragPreview) -> Self {
        self.preview = preview;
        self
    }
}

/// True when a destination that registered `types` takes every item: an item is taken when one of its types is, or
/// conforms to, a registered type (`public.image` takes `public.png`). No items are never accepted.
pub fn accepts_items(types: &[String], items: &[DragItem]) -> bool {
    !items.is_empty()
        && items.iter().all(|item| item.types.iter().any(|t| types.iter().any(|r| bifrost_ui::uttype::conforms(t, r))))
}

/// The single operation of a session: copy wins over link over move among the bits that both `wanted` (what the
/// destination would do) and `allowed` (what the source permits) hold; 0 when none is common.
pub fn pick_operation(wanted: u8, allowed: u8) -> u8 {
    let both = wanted & allowed;
    [dragop::COPY, dragop::LINK, dragop::MOVE].into_iter().find(|b| both & b != 0).unwrap_or(0)
}

/// What a window keeps for drag and drop.
#[derive(Default)]
pub(super) struct DragState {
    /// the source view a press began on, and where (window points)
    pub press: Option<(u64, f32, f32)>,
    /// a source whose press moved past the slop; taken by `take_drag_request`
    pub request: Option<u64>,
    /// the session this window's source began (its id) and the view that began it
    pub session: Option<(u32, u64)>,
    /// the destination the session is over
    pub over: Option<u64>,
}

impl<S: Responder> Window<S> {
    /// A press went down on the view at `path` at (`x`, `y`): remembers the nearest drag source on the path.
    pub(super) fn drag_press(&mut self, path: &[usize], x: f32, y: f32) {
        self.drag.press = None;
        for depth in (0..=path.len()).rev() {
            let v = at(self.root(), &path[..depth]);
            if v.dnd_source().is_some() {
                self.drag.press = Some((v.id(), x, y));
                return;
            }
        }
    }

    /// The press moved to (`x`, `y`). True when it went past the slop and became a drag: the control that took the press
    /// is cancelled (it must not act on the release) and the window stops tracking it.
    pub(super) fn drag_move(&mut self, x: f32, y: f32, state: &mut S) -> bool {
        let Some((id, x0, y0)) = self.drag.press else { return false };
        if (x - x0).hypot(y - y0) <= DRAG_SLOP {
            return false;
        }
        self.drag.press = None;
        self.drag.request = Some(id);
        let mut p = Vec::new();
        if self.tracking.is_some_and(|t| path_to(self.root(), t, &mut p)) {
            self.dispatch_at(&p, &ViewEvent::Cancel, state);
        }
        self.tracking = None;
        true
    }

    /// The drag a press began, once it moved past the slop: the source view and what it offers. Start the session with
    /// `dnd::begin` and report its id with [`Window::drag_began`]. `None` when no press became a drag, or the source
    /// has nothing to offer any more.
    pub fn take_drag_request(&mut self) -> Option<(u64, DragSpec)> {
        let id = self.drag.request.take()?;
        let mut p = Vec::new();
        if !path_to(self.root(), id, &mut p) {
            return None;
        }
        let spec = at(self.root(), &p).dnd_source()?;
        Some((id, spec))
    }

    /// The shell accepted the session `session` that `view` began.
    pub fn drag_began(&mut self, view: u64, session: u32) {
        self.drag.session = Some((session, view));
    }

    /// The session `session` ended with operation `op` (0: nobody took the drop). Tells the source view. True when it
    /// was this window's session.
    pub fn drag_ended(&mut self, session: u32, op: u8) -> bool {
        let Some((s, view)) = self.drag.session else { return false };
        if s != session {
            return false;
        }
        self.drag.session = None;
        let mut p = Vec::new();
        if path_to(self.root(), view, &mut p) {
            at_mut(self.root.as_mut(), &p).dnd_ended(op);
        }
        true
    }

    /// The types the views of the window take (`registerForDraggedTypes`): the union, sorted, without repeats.
    pub fn drop_types(&self) -> Vec<String> {
        fn walk(v: &dyn View, out: &mut Vec<String>) {
            if v.base().hidden {
                return;
            }
            out.extend(v.dnd_types());
            v.children().iter().for_each(|c| walk(c.as_ref(), out));
        }
        let mut types = Vec::new();
        walk(self.root(), &mut types);
        types.sort();
        types.dedup();
        types
    }

    /// The deepest view under (`x`, `y`) (window points), or the nearest ancestor of it, that takes `items`.
    fn drop_target_at(&self, x: f32, y: f32, items: &[DragItem]) -> Option<u64> {
        let mut p = Vec::new();
        if !pick(self.root(), x, y, &mut p) {
            return None;
        }
        for depth in (0..=p.len()).rev() {
            let v = at(self.root(), &p[..depth]);
            let types = v.dnd_types();
            if !types.is_empty() && accepts_items(&types, items) {
                return Some(v.id());
            }
        }
        None
    }

    /// The session left the destination it was over (`draggingExited`).
    fn drop_exit(&mut self) {
        let Some(id) = self.drag.over.take() else { return };
        let mut p = Vec::new();
        if path_to(self.root(), id, &mut p) {
            at_mut(self.root.as_mut(), &p).dnd_exited();
        }
    }

    /// One destination call of a session over this window: `phase` at (`x`, `y`) window points with `items` and the
    /// source's `allowed` operations. Returns the single operation to answer `Entered` and `Updated` with (0 refuses, and
    /// for `Exited` and `Perform`) and whether a `Perform` was accepted.
    pub fn drag_dest(&mut self, phase: DragPhase, x: f32, y: f32, allowed: u8, items: &[DragItem]) -> (u8, bool) {
        match phase {
            DragPhase::Exited => {
                self.drop_exit();
                (0, false)
            }
            DragPhase::Entered | DragPhase::Updated => {
                let target = self.drop_target_at(x, y, items);
                if target != self.drag.over {
                    self.drop_exit();
                }
                let Some(id) = target else { return (0, false) };
                let entering = self.drag.over != Some(id);
                self.drag.over = Some(id);
                let mut p = Vec::new();
                if !path_to(self.root(), id, &mut p) {
                    return (0, false);
                }
                let (ox, oy) = path_offset(self.root(), &p);
                let wanted = at_mut(self.root.as_mut(), &p).dnd_over(entering, x + ox, y + oy, items, allowed);
                (pick_operation(wanted, allowed), false)
            }
            DragPhase::Perform => {
                let id = self.drag.over.or_else(|| self.drop_target_at(x, y, items));
                let mut accepted = false;
                if let Some(id) = id {
                    let mut p = Vec::new();
                    if path_to(self.root(), id, &mut p) {
                        accepted = at_mut(self.root.as_mut(), &p).dnd_perform(items);
                    }
                    // the drop ends the highlight, as `draggingExited` does after a perform
                    self.drag.over = Some(id);
                    self.drop_exit();
                }
                (0, accepted)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::Event;
    use crate::proto::Payload;
    use crate::view::{Container, NoApp, ViewBase};
    use bifrost_ui::Rect;

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }

    fn text_item(s: &str) -> DragItem {
        DragItem::new(Payload::Text(s.into()))
    }

    /// A draggable tile that takes presses like a control: it tracks the press and records the events it sees.
    struct Tile {
        base: ViewBase,
        seen: Vec<String>,
        ended: Vec<u8>,
        offers: bool,
    }

    impl Tile {
        fn new(frame: Rect, offers: bool) -> Self {
            let mut t = Tile { base: ViewBase::default(), seen: Vec::new(), ended: Vec::new(), offers };
            t.base.frame = frame;
            t
        }
    }

    impl View for Tile {
        crate::view_base!();
        fn handle(&mut self, ev: &ViewEvent) -> bool {
            let name = match ev {
                ViewEvent::Pointer(Event::Down(..)) => "down",
                ViewEvent::Pointer(Event::Move(..)) => "move",
                ViewEvent::Pointer(Event::Up(..)) => "up",
                ViewEvent::Cancel => "cancel",
                _ => return false,
            };
            self.seen.push(name.into());
            true
        }
        fn dnd_source(&self) -> Option<DragSpec> {
            self.offers.then(|| DragSpec::new(vec![text_item("hello")]).allowed(dragop::COPY | dragop::MOVE))
        }
        fn dnd_ended(&mut self, op: u8) {
            self.ended.push(op);
        }
    }

    /// A destination that takes text, records what happened to it and wants the operations it was built with.
    struct Slot {
        base: ViewBase,
        types: Vec<String>,
        wants: u8,
        log: Vec<String>,
        take: bool,
    }

    impl Slot {
        fn new(frame: Rect, types: &[&str], wants: u8) -> Self {
            let mut s = Slot { base: ViewBase::default(), types: types.iter().map(|t| t.to_string()).collect(), wants, log: Vec::new(), take: true };
            s.base.frame = frame;
            s
        }
    }

    impl View for Slot {
        crate::view_base!();
        fn dnd_types(&self) -> Vec<String> {
            self.types.clone()
        }
        fn dnd_over(&mut self, entered: bool, x: f32, y: f32, _items: &[DragItem], _allowed: u8) -> u8 {
            self.log.push(format!("{} {x} {y}", if entered { "entered" } else { "updated" }));
            self.wants
        }
        fn dnd_exited(&mut self) {
            self.log.push("exited".into());
        }
        fn dnd_perform(&mut self, items: &[DragItem]) -> bool {
            self.log.push(format!("perform {}", items.len()));
            self.take
        }
    }

    /// root (0,0,300,100) > Tile source (0,0,50,50), Slot (100,0,100,50) taking text, Slot (220,0,60,50) taking images.
    fn window() -> (Window<NoApp>, [u64; 3]) {
        let tile = Tile::new(rect(0.0, 0.0, 50.0, 50.0), true);
        let text = Slot::new(rect(100.0, 0.0, 100.0, 50.0), &["public.plain-text"], dragop::COPY | dragop::MOVE);
        let image = Slot::new(rect(220.0, 0.0, 60.0, 50.0), &["public.image"], dragop::COPY);
        let ids = [tile.id(), text.id(), image.id()];
        let mut root = Container::new(vec![Box::new(tile), Box::new(text), Box::new(image)]);
        root.set_frame(rect(0.0, 0.0, 300.0, 100.0));
        (Window::new(root, 10), ids)
    }

    fn pointer(w: &mut Window<NoApp>, ev: Event) {
        w.event(&ViewEvent::Pointer(ev), &mut NoApp);
    }

    #[test]
    fn a_press_that_moves_past_the_slop_is_a_drag_and_cancels_the_control() {
        let (mut w, ids) = window();
        pointer(&mut w, Event::Down(10.0, 10.0));
        pointer(&mut w, Event::Move(12.0, 10.0));
        assert!(w.take_drag_request().is_none(), "inside the slop it is still a press");
        pointer(&mut w, Event::Move(30.0, 10.0));
        let (view, spec) = w.take_drag_request().expect("past the slop a drag begins");
        assert_eq!(view, ids[0]);
        assert_eq!(spec.items.len(), 1);
        assert_eq!(spec.allowed, dragop::COPY | dragop::MOVE);
        assert!(w.take_drag_request().is_none(), "the request is taken once");
        // the control saw the press and the first small move, then the cancel; the release after it reaches nobody
        pointer(&mut w, Event::Up(30.0, 10.0));
        let tile = w.view::<Tile>(ids[0]).unwrap();
        assert_eq!(tile.seen, vec!["down", "move", "cancel"]);
    }

    #[test]
    fn a_plain_click_on_a_source_is_not_a_drag() {
        let (mut w, ids) = window();
        pointer(&mut w, Event::Down(10.0, 10.0));
        pointer(&mut w, Event::Up(10.0, 10.0));
        assert!(w.take_drag_request().is_none());
        assert_eq!(w.view::<Tile>(ids[0]).unwrap().seen, vec!["down", "up"]);
        // a later press far from the first one does not inherit an old press
        pointer(&mut w, Event::Down(10.0, 10.0));
        pointer(&mut w, Event::Up(10.0, 10.0));
        pointer(&mut w, Event::Move(200.0, 10.0));
        assert!(w.take_drag_request().is_none());
    }

    #[test]
    fn a_view_with_nothing_to_offer_never_starts_a_drag() {
        let tile = Tile::new(rect(0.0, 0.0, 50.0, 50.0), false);
        let mut root = Container::new(vec![Box::new(tile)]);
        root.set_frame(rect(0.0, 0.0, 100.0, 100.0));
        let mut w: Window<NoApp> = Window::new(root, 10);
        pointer(&mut w, Event::Down(10.0, 10.0));
        pointer(&mut w, Event::Move(60.0, 10.0));
        assert!(w.take_drag_request().is_none());
    }

    #[test]
    fn a_session_end_reaches_the_view_that_began_it() {
        let (mut w, ids) = window();
        w.drag_began(ids[0], 7);
        assert!(!w.drag_ended(8, 1), "another session is not this window's");
        assert!(w.drag_ended(7, dragop::MOVE));
        assert_eq!(w.view::<Tile>(ids[0]).unwrap().ended, vec![dragop::MOVE]);
        assert!(!w.drag_ended(7, 0), "a session ends once");
    }

    #[test]
    fn the_registered_types_are_the_union_of_the_views_types() {
        let (w, _) = window();
        assert_eq!(w.drop_types(), vec!["public.image".to_string(), "public.plain-text".to_string()]);
    }

    #[test]
    fn types_match_by_conformance_and_every_item_must_be_taken() {
        let image = vec!["public.image".to_string()];
        let png = DragItem { types: vec!["public.png".into()], payload: Payload::Text("x".into()) };
        assert!(accepts_items(&image, &[png.clone()]), "public.image takes a PNG");
        assert!(!accepts_items(&["public.png".to_string()], &[DragItem { types: vec!["public.image".into()], payload: Payload::Text("x".into()) }]), "a PNG slot does not take any image");
        assert!(!accepts_items(&image, &[png, text_item("t")]), "one item that is not taken refuses the drop");
        assert!(!accepts_items(&image, &[]));
    }

    #[test]
    fn the_operation_is_one_bit_both_sides_agree_on() {
        assert_eq!(pick_operation(dragop::COPY | dragop::MOVE, dragop::ALL), dragop::COPY);
        assert_eq!(pick_operation(dragop::LINK | dragop::MOVE, dragop::ALL), dragop::LINK);
        assert_eq!(pick_operation(dragop::MOVE, dragop::COPY | dragop::MOVE), dragop::MOVE);
        assert_eq!(pick_operation(dragop::COPY, dragop::MOVE), 0);
        assert_eq!(pick_operation(0, dragop::ALL), 0);
    }

    #[test]
    fn a_session_enters_updates_exits_and_moves_between_destinations() {
        let (mut w, ids) = window();
        let items = vec![text_item("hello")];
        // over the text slot: entered at the point, in the slot's own content space
        let (op, _) = w.drag_dest(DragPhase::Entered, 110.0, 20.0, dragop::ALL, &items);
        assert_eq!(op, dragop::COPY);
        let (op, _) = w.drag_dest(DragPhase::Updated, 120.0, 25.0, dragop::MOVE, &items);
        assert_eq!(op, dragop::MOVE, "the source allows only a move");
        // over the image slot the text is not taken: the text slot is left and nothing answers
        let (op, _) = w.drag_dest(DragPhase::Updated, 230.0, 20.0, dragop::ALL, &items);
        assert_eq!(op, 0);
        assert_eq!(w.view::<Slot>(ids[1]).unwrap().log, vec!["entered 110 20", "updated 120 25", "exited"]);
        assert!(w.view::<Slot>(ids[2]).unwrap().log.is_empty());
        // back over the text slot enters it again; leaving the window exits it
        w.drag_dest(DragPhase::Updated, 150.0, 20.0, dragop::ALL, &items);
        w.drag_dest(DragPhase::Exited, 0.0, 0.0, dragop::ALL, &[]);
        let log = w.view::<Slot>(ids[1]).unwrap().log.clone();
        assert_eq!(log[3..].to_vec(), vec!["entered 150 20".to_string(), "exited".to_string()]);
    }

    #[test]
    fn a_drop_is_performed_by_the_view_under_it_and_ends_its_highlight() {
        let (mut w, ids) = window();
        let items = vec![text_item("hello")];
        w.drag_dest(DragPhase::Entered, 110.0, 20.0, dragop::ALL, &items);
        let (_, accepted) = w.drag_dest(DragPhase::Perform, 110.0, 20.0, dragop::ALL, &items);
        assert!(accepted);
        assert_eq!(w.view::<Slot>(ids[1]).unwrap().log, vec!["entered 110 20", "perform 1", "exited"]);
        // a drop over a view that refuses reports it
        w.view_mut::<Slot>(ids[1]).unwrap().take = false;
        w.drag_dest(DragPhase::Entered, 110.0, 20.0, dragop::ALL, &items);
        assert!(!w.drag_dest(DragPhase::Perform, 110.0, 20.0, dragop::ALL, &items).1);
        // a drop over empty space is not accepted
        assert!(!w.drag_dest(DragPhase::Perform, 5.0, 80.0, dragop::ALL, &items).1);
    }
}
