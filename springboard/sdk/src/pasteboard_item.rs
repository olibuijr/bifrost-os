//! `NSPasteboardItem`: one copied thing held as several types at once, each either plain data or a promise that a
//! [`DataProvider`] fills in when first read. [`NSPasteboardItem::to_item`] resolves everything into the wire item that
//! [`crate::pasteboard::NativePasteboard::write_items`] and [`crate::pasteboard_types::Board::write`] carry.

use crate::pasteboard_promise::{DataProvider, Promise};
use crate::pasteboard_types::Item;
use std::rc::Rc;

enum Slot {
    Data(Vec<u8>),
    Promised(Promise),
}

/// `NSPasteboardItem`.
#[derive(Default)]
pub struct NSPasteboardItem {
    slots: Vec<(String, Slot)>,
}

impl NSPasteboardItem {
    pub fn new() -> Self {
        Self::default()
    }

    fn put(&mut self, uti: &str, slot: Slot) {
        self.slots.retain(|(t, _)| t != uti);
        self.slots.push((uti.to_string(), slot));
    }

    /// `setData(_:forType:)`: replaces the type if present.
    pub fn set_data(&mut self, bytes: &[u8], uti: &str) {
        self.put(uti, Slot::Data(bytes.to_vec()));
    }

    /// `setString(_:forType:)`.
    pub fn set_string(&mut self, text: &str, uti: &str) {
        self.set_data(text.as_bytes(), uti);
    }

    /// `setDataProvider(_:forTypes:)`: promises `types`; `provider` is asked for each the first time it is read.
    pub fn set_data_provider(&mut self, provider: Rc<dyn DataProvider>, types: &[&str]) {
        for uti in types {
            self.put(uti, Slot::Promised(Promise::new(Rc::clone(&provider))));
        }
    }

    /// `types`: declaration order (a replaced type moves to the end).
    pub fn types(&self) -> Vec<String> {
        self.slots.iter().map(|(t, _)| t.clone()).collect()
    }

    /// True while `uti` is promised and nothing has asked for it yet.
    pub fn is_pending(&self, uti: &str) -> bool {
        self.slots.iter().any(|(t, s)| t == uti && matches!(s, Slot::Promised(p) if !p.is_resolved()))
    }

    /// `data(forType:)`: resolves a promise on first use.
    pub fn data(&self, uti: &str) -> Option<Vec<u8>> {
        match &self.slots.iter().find(|(t, _)| t == uti)?.1 {
            Slot::Data(d) => Some(d.clone()),
            Slot::Promised(p) => p.resolve(uti),
        }
    }

    /// `string(forType:)`.
    pub fn string(&self, uti: &str) -> Option<String> {
        self.data(uti).and_then(|d| String::from_utf8(d).ok())
    }

    /// The wire item: every type with its bytes. Promises are resolved now; a type whose provider has no data is left out.
    pub fn to_item(&self) -> Item {
        self.slots
            .iter()
            .filter_map(|(t, _)| self.data(t).map(|d| (t, d)))
            .fold(Item::default(), |item, (t, d)| item.with(t, &d))
    }

    /// An item holding the data of a wire item.
    pub fn from_item(item: &Item) -> Self {
        let mut out = Self::new();
        for r in &item.representations {
            out.set_data(&r.data, &r.uti);
        }
        out
    }
}
