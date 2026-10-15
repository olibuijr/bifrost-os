//! Drag session state machine (`DraggingSession`, `DraggingSource`, `DraggingDestination`): operation
//! negotiation between the source's allowed mask and the destination's answer, and the callbacks in the reference order:
//! entered, updated..., exited, or performDragOperation then concludeDragOperation.

/// `DragOperation` bits.
pub mod op {
    pub const NONE: u8 = 0;
    pub const COPY: u8 = 1;
    pub const LINK: u8 = 2;
    pub const MOVE: u8 = 16;
    pub const EVERY: u8 = COPY | LINK | MOVE;
}

/// One pasteboard item in a drag (`DraggingItem` with an `PasteboardItem`): the types it carries, and an
/// optional file promise (`FilePromiseProvider`) whose file is written on drop.
#[derive(Clone, Debug, PartialEq)]
pub struct DraggingItem {
    pub types: Vec<String>,
    pub promised_file: Option<PromisedFile>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PromisedFile {
    pub name: String,
    pub contents: Vec<u8>,
}

/// Calls the destination receives, in order.
#[derive(Clone, Debug, PartialEq)]
pub enum Callback {
    Entered(u64),
    Updated(u64),
    Exited(u64),
    Perform(u64),
    Conclude(u64),
}

/// A destination under the pointer: `registerForDraggedTypes` plus its answer.
pub trait Destination {
    fn id(&self) -> u64;
    fn registered_types(&self) -> &[String];
    /// `draggingEntered` / `draggingUpdated`: the operation wanted for the given source mask.
    fn operation(&mut self, source_mask: u8) -> u8;
    /// `performDragOperation`: true when the drop was taken. Promised files are written to `written`.
    fn perform(&mut self, items: &[DraggingItem], written: &mut Vec<PromisedFile>) -> bool;
    /// `draggingExited`: the pointer left this destination, or the drop ended on it. Default: nothing to reset.
    fn exited(&mut self) {}
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Dragging,
    Dropped,
    Cancelled,
}

pub struct Session {
    source_mask: u8,
    items: Vec<DraggingItem>,
    over: Option<u64>,
    operation: u8,
    state: State,
    pub log: Vec<Callback>,
    pub written_files: Vec<PromisedFile>,
}

impl Session {
    /// `draggingSession(_:sourceOperationMaskFor:)` gives `source_mask`. Items must not be empty.
    pub fn begin(source_mask: u8, items: Vec<DraggingItem>) -> Option<Self> {
        if items.is_empty() {
            return None;
        }
        Some(Session { source_mask, items, over: None, operation: op::NONE, state: State::Dragging, log: vec![], written_files: vec![] })
    }

    pub fn state(&self) -> State {
        self.state
    }
    /// The operation currently negotiated (0 when refused or over nothing).
    pub fn operation(&self) -> u8 {
        self.operation
    }

    fn accepts(&self, d: &dyn Destination) -> bool {
        self.items.iter().all(|i| i.types.iter().any(|t| d.registered_types().iter().any(|r| r == t)))
    }

    /// The pointer moved; `previous` is the destination it left and `dest` is the destination now under it.
    /// Pass the previous destination so its `exited` callback can clear bindings such as `isTargeted`.
    pub fn moved(&mut self, previous: Option<&mut dyn Destination>, dest: Option<&mut dyn Destination>) {
        if self.state != State::Dragging {
            return;
        }
        let next_id = dest.as_ref().map(|d| d.id());
        if self.over.is_some() && self.over != next_id {
            if let Some(previous) = previous {
                if self.over == Some(previous.id()) {
                    previous.exited();
                }
            }
            self.leave();
        }
        match dest {
            Some(d) if self.accepts(d) => {
                let id = d.id();
                if self.over != Some(id) {
                    self.leave();
                    self.log.push(Callback::Entered(id));
                    self.over = Some(id);
                } else {
                    self.log.push(Callback::Updated(id));
                }
                let both = d.operation(self.source_mask) & self.source_mask & op::EVERY;
                // a single operation: copy wins over link over move when several bits remain
                self.operation = [op::COPY, op::LINK, op::MOVE].into_iter().find(|b| both & b != 0).unwrap_or(op::NONE);
            }
            other => {
                if let Some(d) = other {
                    if self.over == Some(d.id()) {
                        d.exited();
                    }
                }
                self.leave();
            }
        }
    }

    fn leave(&mut self) {
        if let Some(id) = self.over.take() {
            self.log.push(Callback::Exited(id));
        }
        self.operation = op::NONE;
    }

    /// The button was released. Returns the operation performed (0 when nothing took it).
    pub fn drop_on(&mut self, previous: Option<&mut dyn Destination>, dest: Option<&mut dyn Destination>) -> u8 {
        if self.state != State::Dragging {
            return op::NONE;
        }
        let taken = match dest {
            Some(d) if self.over == Some(d.id()) && self.operation != op::NONE => {
                let id = d.id();
                self.log.push(Callback::Perform(id));
                let ok = d.perform(&self.items, &mut self.written_files);
                d.exited();
                if ok {
                    self.log.push(Callback::Conclude(id));
                }
                ok
            }
            other => {
                let over = self.over;
                if let Some(previous) = previous {
                    if over == Some(previous.id()) {
                        previous.exited();
                    }
                }
                if let Some(d) = other {
                    if over == Some(d.id()) {
                        d.exited();
                    }
                }
                self.leave();
                false
            }
        };
        let performed = self.operation;
        self.over = None;
        self.state = if taken { State::Dropped } else { State::Cancelled };
        if taken { performed } else { op::NONE }
    }

    /// Escape: the source gives the session up.
    pub fn cancel(&mut self, over: Option<&mut dyn Destination>) {
        if self.state == State::Dragging {
            if let Some(over) = over {
                if self.over == Some(over.id()) {
                    over.exited();
                }
            }
            self.leave();
            self.state = State::Cancelled;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Well {
        id: u64,
        types: Vec<String>,
        want: u8,
        take: bool,
        got: Vec<DraggingItem>,
    }
    impl Destination for Well {
        fn id(&self) -> u64 {
            self.id
        }
        fn registered_types(&self) -> &[String] {
            &self.types
        }
        fn operation(&mut self, _m: u8) -> u8 {
            self.want
        }
        fn perform(&mut self, items: &[DraggingItem], written: &mut Vec<PromisedFile>) -> bool {
            self.got = items.to_vec();
            written.extend(items.iter().filter_map(|i| i.promised_file.clone()));
            self.take
        }
    }
    fn well(id: u64, want: u8, take: bool) -> Well {
        Well { id, types: vec!["public.utf8-plain-text".into()], want, take, got: vec![] }
    }
    fn text_item() -> DraggingItem {
        DraggingItem { types: vec!["public.utf8-plain-text".into()], promised_file: None }
    }

    #[test]
    fn empty_drag_does_not_begin() {
        assert!(Session::begin(op::COPY, vec![]).is_none());
    }

    #[test]
    fn enter_update_perform_conclude_in_order() {
        let mut s = Session::begin(op::COPY | op::MOVE, vec![text_item()]).unwrap();
        let mut w = well(1, op::EVERY, true);
        s.moved(None, Some(&mut w));
        s.moved(None, Some(&mut w));
        assert_eq!(s.operation(), op::COPY, "mask intersected with the answer, one bit");
        assert_eq!(s.drop_on(None, Some(&mut w)), op::COPY);
        assert_eq!(s.log, vec![Callback::Entered(1), Callback::Updated(1), Callback::Perform(1), Callback::Conclude(1)]);
        assert_eq!(s.state(), State::Dropped);
        assert_eq!(w.got.len(), 1);
    }

    #[test]
    fn refusing_destination_gets_no_perform_and_leaving_exits() {
        let mut s = Session::begin(op::COPY, vec![text_item()]).unwrap();
        let mut refuse = well(1, op::NONE, true);
        s.moved(None, Some(&mut refuse));
        assert_eq!(s.drop_on(None, Some(&mut refuse)), op::NONE);
        assert!(!s.log.contains(&Callback::Perform(1)));
        assert_eq!(s.state(), State::Cancelled);

        let mut s = Session::begin(op::COPY, vec![text_item()]).unwrap();
        let (mut a, mut b) = (well(1, op::COPY, true), well(2, op::COPY, true));
        s.moved(None, Some(&mut a));
        s.moved(Some(&mut a), Some(&mut b));
        s.moved(Some(&mut b), None);
        assert_eq!(s.log, vec![Callback::Entered(1), Callback::Exited(1), Callback::Entered(2), Callback::Exited(2)]);
        assert_eq!(s.operation(), op::NONE);
    }

    #[test]
    fn destination_not_registered_for_the_types_is_ignored() {
        let item = DraggingItem { types: vec!["public.png".into()], promised_file: None };
        let mut s = Session::begin(op::COPY, vec![item]).unwrap();
        let mut w = well(1, op::COPY, true);
        s.moved(None, Some(&mut w));
        assert!(s.log.is_empty());
    }

    #[test]
    fn file_promise_is_written_on_drop_and_cancel_ends_session() {
        let item = DraggingItem {
            types: vec!["public.utf8-plain-text".into()],
            promised_file: Some(PromisedFile { name: "a.txt".into(), contents: b"hi".to_vec() }),
        };
        let mut s = Session::begin(op::COPY, vec![item]).unwrap();
        let mut w = well(1, op::COPY, true);
        s.moved(None, Some(&mut w));
        s.drop_on(None, Some(&mut w));
        assert_eq!(s.written_files[0].name, "a.txt");

        let mut c = Session::begin(op::COPY, vec![text_item()]).unwrap();
        c.moved(None, Some(&mut w));
        c.cancel(Some(&mut w));
        assert_eq!(c.state(), State::Cancelled);
        assert_eq!(c.log.last(), Some(&Callback::Exited(1)));
        assert_eq!(c.drop_on(None, Some(&mut w)), op::NONE, "a finished session ignores more input");
    }
}
