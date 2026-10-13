//! `PasteboardItem`: one copied thing held as several typed representations at once. Types are UTType ids; a
//! pasteboard holds a list of items (`pasteboard::NativePasteboard::write_items`). Each type is either plain data or a
//! promise that a [`PasteboardItemDataProvider`] fills in the first time it is read (`setDataProvider(_:forTypes:)`).
//! [`PasteboardItem::to_item`] resolves everything into the wire item that
//! [`crate::pasteboard::NativePasteboard::write_items`] and [`crate::pasteboard_types::Board::write`] carry.

use crate::pasteboard_promise::{PasteboardItemDataProvider, Promise, ProviderHandle, PromiseError};
use crate::pasteboard_types::Item;
use bifrost_ui::uttype;
use std::fmt;
use std::rc::Rc;

#[derive(Clone)]
enum Slot {
    Data(Vec<u8>),
    Promised(Promise),
}

/// What a lookup of one type found, without asking any provider.
pub(crate) enum Found {
    Data(Vec<u8>),
    Promise(Promise),
}

/// `PasteboardItem`.
#[derive(Clone, Default)]
pub struct PasteboardItem {
    slots: Vec<(String, Slot)>,
}

impl fmt::Debug for PasteboardItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let slots = self.slots.iter().map(|(t, s)| (t, matches!(s, Slot::Promised(_)))).collect::<Vec<_>>();
        f.debug_struct("PasteboardItem").field("types (type, promised)", &slots).finish()
    }
}

impl PasteboardItem {
    pub fn new() -> Self {
        Self::default()
    }

    /// Builds an item from flat (type, data) pairs.
    pub fn from_representations(reps: Vec<(String, Vec<u8>)>) -> Self {
        let mut item = Self::new();
        for (t, d) in reps {
            item.set_data(&d, &t);
        }
        item
    }

    /// An item holding the data of a wire item.
    pub fn from_item(item: &Item) -> Self {
        let mut out = Self::new();
        for r in &item.representations {
            out.set_data(&r.data, &r.uti);
        }
        out
    }

    /// Puts `slot` under `uti`, replacing (and moving to the end) a type that is already there. A provider that
    /// no longer backs any type of the item is told it is finished.
    fn put(&mut self, uti: &str, slot: Slot) {
        let at = self.slots.iter().position(|(t, _)| t == uti);
        let old = at.map(|i| self.slots.remove(i).1);
        self.slots.push((uti.to_string(), slot));
        if let Some(Slot::Promised(p)) = old {
            if !self.uses(p.handle()) {
                p.handle().finish();
            }
        }
    }

    fn uses(&self, handle: &ProviderHandle) -> bool {
        self.slots.iter().any(|(_, s)| matches!(s, Slot::Promised(p) if p.handle().same(handle)))
    }

    /// `setData(_:forType:)`: replaces the data of `uti`; returns true.
    pub fn set_data(&mut self, bytes: &[u8], uti: &str) -> bool {
        self.put(uti, Slot::Data(bytes.to_vec()));
        true
    }

    /// `setString(_:forType:)`.
    pub fn set_string(&mut self, text: &str, uti: &str) -> bool {
        self.set_data(text.as_bytes(), uti)
    }

    /// `setDataProvider(_:forTypes:)`: promises `types`; `provider` is asked for each the first time it is read.
    pub fn set_data_provider(&mut self, provider: Rc<dyn PasteboardItemDataProvider>, types: &[&str]) {
        let handle = ProviderHandle::new(provider);
        for uti in types {
            self.put(uti, Slot::Promised(Promise::with_handle(handle.clone())));
        }
    }

    /// `types`, in the order they were set (a replaced type moves to the end). Promised types are listed too.
    pub fn types(&self) -> Vec<String> {
        self.slots.iter().map(|(t, _)| t.clone()).collect()
    }

    /// True while `uti` is promised and nothing has asked for it yet.
    pub fn is_pending(&self, uti: &str) -> bool {
        self.slots.iter().any(|(t, s)| t == uti && matches!(s, Slot::Promised(p) if !p.is_resolved()))
    }

    /// `data(forType:)`: resolves a promise on first use. A type the item lacks is [`PromiseError::UnknownType`]; a
    /// provider with nothing for it is [`PromiseError::NoData`].
    pub fn data_for_type(&self, uti: &str) -> Result<Vec<u8>, PromiseError> {
        match self.lookup(uti).ok_or_else(|| PromiseError::UnknownType(uti.to_string()))? {
            Found::Data(d) => Ok(d),
            Found::Promise(p) => p.resolve(uti).ok_or_else(|| PromiseError::NoData(uti.to_string())),
        }
    }

    /// `data(forType:)` as an option.
    pub fn data(&self, uti: &str) -> Option<Vec<u8>> {
        self.data_for_type(uti).ok()
    }

    /// `string(forType:)`.
    pub fn string(&self, uti: &str) -> Option<String> {
        self.data(uti).and_then(|d| String::from_utf8(d).ok())
    }

    /// The slot of `uti`, cloned, with no provider asked. Lets a caller resolve outside its own borrows.
    pub(crate) fn lookup(&self, uti: &str) -> Option<Found> {
        self.slots.iter().find(|(t, _)| t == uti).map(|(_, s)| match s {
            Slot::Data(d) => Found::Data(d.clone()),
            Slot::Promised(p) => Found::Promise(p.clone()),
        })
    }

    /// The flat (type, data) pairs with every promise resolved now. A type whose provider has no data is left out.
    pub fn representations(&self) -> Vec<(String, Vec<u8>)> {
        self.slots.iter().filter_map(|(t, _)| self.data(t).map(|d| (t.clone(), d))).collect()
    }

    /// The flat pairs that exist without asking anyone: plain data and promises already answered. This is what goes
    /// to the shell, so a promise stays lazy until something reads it.
    pub fn available_representations(&self) -> Vec<(String, Vec<u8>)> {
        self.slots
            .iter()
            .filter_map(|(t, s)| match s {
                Slot::Data(d) => Some((t.clone(), d.clone())),
                Slot::Promised(p) => p.resolved_data().map(|d| (t.clone(), d)),
            })
            .collect()
    }

    /// The wire item: every type with its bytes. Promises are resolved now; a type whose provider has no data is left out.
    pub fn to_item(&self) -> Item {
        self.representations().into_iter().fold(Item::default(), |item, (t, d)| item.with(&t, &d))
    }

    /// Tells every provider behind this item that it is finished (the board was cleared or another app took over).
    pub fn finish_providers(&self) {
        for (_, s) in &self.slots {
            if let Slot::Promised(p) = s {
                p.handle().finish();
            }
        }
    }

    /// True when any type of the item is, or descends from, one of `ancestors`.
    pub fn conforms_to_any(&self, ancestors: &[&str]) -> bool {
        self.slots.iter().any(|(t, _)| ancestors.iter().any(|a| uttype::conforms(t, a)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    #[test]
    fn an_item_keeps_one_representation_per_type_in_order() {
        let mut i = PasteboardItem::new();
        i.set_string("a", "public.utf8-plain-text");
        i.set_data(b"<b>a</b>", "public.html");
        i.set_string("b", "public.utf8-plain-text");
        assert_eq!(i.types(), ["public.html", "public.utf8-plain-text"]);
        assert_eq!(i.string("public.utf8-plain-text").as_deref(), Some("b"));
        assert_eq!(i.data("public.png"), None);
    }

    #[test]
    fn conformance_uses_the_type_hierarchy() {
        let mut i = PasteboardItem::new();
        i.set_string("a", "public.utf8-plain-text");
        assert!(i.conforms_to_any(&["public.text"]));
        assert!(!i.conforms_to_any(&["public.image"]));
    }

    #[test]
    fn an_item_round_trips_through_the_wire_item() {
        let mut item = PasteboardItem::new();
        item.set_string("hello", "public.utf8-plain-text");
        item.set_data(b"<b>hello</b>", "public.html");
        let back = PasteboardItem::from_item(&item.to_item());
        assert_eq!(back.string("public.utf8-plain-text").as_deref(), Some("hello"));
        assert_eq!(back.data("public.html"), Some(b"<b>hello</b>".to_vec()));
        assert_eq!(PasteboardItem::from_representations(item.representations()).types(), item.types());
    }

    #[derive(Default)]
    struct Log {
        calls: Vec<String>,
        finished: u32,
    }

    struct Provider {
        log: Rc<RefCell<Log>>,
        answer: Option<Vec<u8>>,
    }

    impl PasteboardItemDataProvider for Provider {
        fn provide_data(&self, uti: &str) -> Option<Vec<u8>> {
            self.log.borrow_mut().calls.push(uti.to_string());
            self.answer.clone()
        }
        fn finished_with_data_provider(&self) {
            self.log.borrow_mut().finished += 1;
        }
    }

    fn promised(answer: Option<&[u8]>) -> (PasteboardItem, Rc<RefCell<Log>>) {
        let log = Rc::new(RefCell::new(Log::default()));
        let mut item = PasteboardItem::new();
        item.set_data_provider(Rc::new(Provider { log: log.clone(), answer: answer.map(|a| a.to_vec()) }), &["public.png"]);
        (item, log)
    }

    #[test]
    fn promised_types_are_listed_without_calling_the_provider() {
        let (item, log) = promised(Some(b"png"));
        assert_eq!(item.types(), vec!["public.png".to_string()]);
        assert!(item.is_pending("public.png"));
        assert!(item.available_representations().is_empty(), "nothing exists yet for the shell");
        assert!(log.borrow().calls.is_empty());
    }

    #[test]
    fn first_read_calls_the_provider_once_and_keeps_the_answer() {
        let (item, log) = promised(Some(b"png"));
        assert_eq!(item.data_for_type("public.png"), Ok(b"png".to_vec()));
        assert_eq!(item.data_for_type("public.png"), Ok(b"png".to_vec()));
        assert_eq!(log.borrow().calls, vec!["public.png".to_string()]);
        assert!(!item.is_pending("public.png"));
        assert_eq!(item.available_representations(), vec![("public.png".to_string(), b"png".to_vec())]);
    }

    #[test]
    fn a_provider_with_nothing_fails_cleanly_and_unknown_types_are_reported() {
        let (item, log) = promised(None);
        assert_eq!(item.data_for_type("public.png"), Err(PromiseError::NoData("public.png".into())));
        assert_eq!(item.data_for_type("public.png"), Err(PromiseError::NoData("public.png".into())));
        assert_eq!(log.borrow().calls.len(), 1, "the empty answer is kept too");
        assert_eq!(item.data_for_type("public.html"), Err(PromiseError::UnknownType("public.html".into())));
        assert!(item.to_item().representations.is_empty(), "a type without data is left out of the wire item");
    }

    #[test]
    fn finishing_tells_a_shared_provider_once() {
        let log = Rc::new(RefCell::new(Log::default()));
        let mut item = PasteboardItem::new();
        item.set_data_provider(Rc::new(Provider { log: log.clone(), answer: None }), &["public.png", "public.html"]);
        item.finish_providers();
        item.finish_providers();
        assert_eq!(log.borrow().finished, 1);
    }

    #[test]
    fn replacing_every_type_of_a_provider_finishes_it() {
        let (mut item, log) = promised(Some(b"x"));
        item.set_data_provider(Rc::new(Provider { log: Rc::new(RefCell::new(Log::default())), answer: None }), &["public.png"]);
        assert_eq!(log.borrow().finished, 1);
        let (mut item, log) = promised(Some(b"x"));
        item.set_data(b"plain", "public.png");
        assert_eq!(log.borrow().finished, 1, "plain data replacing the promise ends it as well");
    }

    struct Counting(Cell<u32>);

    impl PasteboardItemDataProvider for Counting {
        fn provide_data(&self, uti: &str) -> Option<Vec<u8>> {
            self.0.set(self.0.get() + 1);
            (uti != "public.png").then(|| format!("made {uti}").into_bytes())
        }
    }

    #[test]
    fn a_provider_serving_two_types_is_asked_per_type_once() {
        let provider = Rc::new(Counting(Cell::new(0)));
        let mut item = PasteboardItem::new();
        item.set_data_provider(provider.clone(), &["public.html", "public.png"]);
        assert_eq!(item.string("public.html").as_deref(), Some("made public.html"));
        assert_eq!(item.string("public.html").as_deref(), Some("made public.html"));
        assert_eq!(provider.0.get(), 1);
        assert_eq!(item.to_item().representations.len(), 1, "public.png has no data and is left out");
        item.to_item();
        assert_eq!(provider.0.get(), 2, "the empty answer was kept");
    }
}
