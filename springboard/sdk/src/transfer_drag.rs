//! Declarative drag and drop for `Transferable` models (`draggable(_:)`, `dropDestination(for:action:isTargeted:)`), on top of
//! the drag session state machine in `bifrost_ui::drag_session`.
//!
//! [`draggable`] turns a model value into the session's dragging item plus the data each type carries. [`DropTarget`] is the
//! destination: it registers for the types its model can import, answers the negotiated operation, and on drop rebuilds the
//! values and hands them to its action. `is_targeted` mirrors the `isTargeted` binding.
//!
//! The session items carry type lists only, so the data travels beside them in a [`Dragged`]; the host that runs the session
//! hands it to the target with [`DropTarget::deliver`] before the drop. Inside one process that is a plain move; across
//! processes the shell wire carries the same pasteboard items.

use crate::pasteboard_types::Item as PasteItem;
use crate::transfer::{pasteboard_item, Representations, Viewer};
use bifrost_ui::drag_session::{op, Destination, DraggingItem, PromisedFile};

/// What a drag source puts into a session: the items the session negotiates over, and the data each one carries.
pub struct Dragged {
    pub items: Vec<DraggingItem>,
    pub data: Vec<PasteItem>,
}

/// `draggable(_:)`: one dragging item offering every type `value` exports to another app. A file representation is also
/// promised under the file's name, so a destination that wants a file gets one written on drop. `None` when the model has
/// nothing visible to export.
pub fn draggable<T>(reps: &Representations<T>, value: &T) -> Option<Dragged> {
    let types: Vec<String> = reps.types_for(Viewer::Other).iter().map(|t| t.to_string()).collect();
    if types.is_empty() {
        return None;
    }
    let promised_file = types.iter().find_map(|uti| {
        let sent = reps.export_file(value, uti, Viewer::Other)?;
        let name = sent.file.file_name()?.to_string_lossy().into_owned();
        let contents = std::fs::read(&sent.file).ok()?;
        Some(PromisedFile { name, contents })
    });
    Some(Dragged { items: vec![DraggingItem { types, promised_file }], data: vec![pasteboard_item(reps, value)] })
}

/// `dropDestination(for:action:isTargeted:)`: accepts drops of `T` and calls `action` with the values and the pointer's
/// position class. The action returns true when it took the drop.
pub struct DropTarget<T> {
    id: u64,
    reps: Representations<T>,
    types: Vec<String>,
    wanted: u8,
    targeted: bool,
    incoming: Vec<PasteItem>,
    action: Box<dyn FnMut(Vec<T>) -> bool>,
}

impl<T> DropTarget<T> {
    /// A target registered for every type `reps` can import. It asks for a copy; see [`Self::wanting`].
    pub fn new(id: u64, reps: Representations<T>, action: impl FnMut(Vec<T>) -> bool + 'static) -> Self {
        let types = reps.imported_content_types().iter().map(|t| t.to_string()).collect();
        DropTarget { id, reps, types, wanted: op::COPY, targeted: false, incoming: Vec::new(), action: Box::new(action) }
    }

    /// The operations this target answers with (`draggingEntered` result), as `drag_session::op` bits.
    pub fn wanting(mut self, operations: u8) -> Self {
        self.wanted = operations;
        self
    }

    /// The `isTargeted` binding: true while a drag that this target accepts is over it.
    pub fn is_targeted(&self) -> bool {
        self.targeted
    }

    /// Hands the data of the drag to the target before the drop (see the module comment).
    pub fn deliver(&mut self, data: &[PasteItem]) {
        self.incoming = data.to_vec();
    }
}

impl<T> Destination for DropTarget<T> {
    fn id(&self) -> u64 {
        self.id
    }

    fn registered_types(&self) -> &[String] {
        &self.types
    }

    fn operation(&mut self, _source_mask: u8) -> u8 {
        self.targeted = true;
        self.wanted
    }

    fn perform(&mut self, _items: &[DraggingItem], _written: &mut Vec<PromisedFile>) -> bool {
        let values = crate::transfer::paste_values(&self.reps, &std::mem::take(&mut self.incoming));
        !values.is_empty() && (self.action)(values)
    }

    fn exited(&mut self) {
        self.targeted = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transfer::{Transferable, Visibility};
    use bifrost_ui::drag_session::{Callback, Session, State};
    use std::cell::RefCell;
    use std::rc::Rc;

    #[derive(Debug, PartialEq)]
    struct Note(String);

    const PLAIN: &str = "public.utf8-plain-text";
    const HTML: &str = "public.html";

    impl Transferable for Note {
        fn transfer_representations() -> Representations<Note> {
            Representations::new()
                .data(PLAIN, Visibility::All, |n: &Note| n.0.clone().into_bytes(), |b| String::from_utf8(b.to_vec()).ok().map(Note))
                .proxy(HTML, Visibility::All, |n| format!("<p>{}</p>", n.0).into_bytes())
        }
    }

    fn inbox() -> (Rc<RefCell<Vec<Note>>>, DropTarget<Note>) {
        let got = Rc::new(RefCell::new(Vec::new()));
        let sink = Rc::clone(&got);
        let target = DropTarget::new(1, Note::transfer_representations(), move |v| {
            sink.borrow_mut().extend(v);
            true
        });
        (got, target)
    }

    #[test]
    fn draggable_offers_the_visible_types_with_their_data() {
        let d = draggable(&Note::transfer_representations(), &Note("hi".into())).expect("a note is draggable");
        assert_eq!(d.items[0].types, [PLAIN, HTML]);
        assert_eq!(d.data[0].data(PLAIN), Some(&b"hi"[..]));
        assert!(draggable(&Representations::<Note>::new(), &Note("x".into())).is_none());
    }

    #[test]
    fn a_target_registers_only_for_importable_types() {
        let (_, target) = inbox();
        assert_eq!(target.registered_types().to_vec(), vec![PLAIN.to_string()]);
    }

    #[test]
    fn a_note_drags_and_drops_between_two_hosts() {
        let (got, mut target) = inbox();
        let dragged = draggable(&Note::transfer_representations(), &Note("hello".into())).unwrap();
        let mut session = Session::begin(op::COPY | op::MOVE, dragged.items).expect("a session begins");
        session.moved(None, Some(&mut target));
        assert!(target.is_targeted());
        assert_eq!(session.operation(), op::COPY);
        target.deliver(&dragged.data);
        assert_eq!(session.drop_on(None, Some(&mut target)), op::COPY);
        assert_eq!(session.state(), State::Dropped);
        assert_eq!(session.log, [Callback::Entered(1), Callback::Perform(1), Callback::Conclude(1)]);
        assert_eq!(*got.borrow(), [Note("hello".into())]);
        assert!(!target.is_targeted(), "the drop clears isTargeted");
    }

    #[test]
    fn leaving_a_target_clears_is_targeted_and_a_refusing_action_cancels() {
        let mut target = DropTarget::new(2, Note::transfer_representations(), |_| false);
        let dragged = draggable(&Note::transfer_representations(), &Note("x".into())).unwrap();
        let mut session = Session::begin(op::COPY, dragged.items).unwrap();
        session.moved(None, Some(&mut target));
        session.moved(Some(&mut target), None);
        assert!(!target.is_targeted());
        session.moved(None, Some(&mut target));
        target.deliver(&dragged.data);
        assert_eq!(session.drop_on(None, Some(&mut target)), op::NONE);
        assert_eq!(session.state(), State::Cancelled);
    }

    #[test]
    fn a_target_wanting_move_gets_move_only_when_the_source_allows_it() {
        let (_, target) = inbox();
        let mut target = target.wanting(op::MOVE);
        let dragged = draggable(&Note::transfer_representations(), &Note("x".into())).unwrap();
        let mut copy_only = Session::begin(op::COPY, dragged.items.clone()).unwrap();
        copy_only.moved(None, Some(&mut target));
        assert_eq!(copy_only.operation(), op::NONE);
        let mut movable = Session::begin(op::MOVE, dragged.items).unwrap();
        movable.moved(None, Some(&mut target));
        assert_eq!(movable.operation(), op::MOVE);
    }
}
