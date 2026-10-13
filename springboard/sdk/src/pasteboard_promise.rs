//! `NSPasteboardItemDataProvider` promises: a type is declared up front and its bytes are produced only when something
//! asks for them. The provider runs at most once per promise; the answer (even "no data") is kept.

use std::cell::RefCell;
use std::rc::Rc;

/// `NSPasteboardItemDataProvider` / `NSItemProvider` lazy loader: produces the bytes of one type on demand.
pub trait DataProvider {
    /// `pasteboard(_:item:provideDataForType:)`. `None` when the data cannot be produced.
    fn provide(&self, uti: &str) -> Option<Vec<u8>>;
}

struct Inner {
    provider: Rc<dyn DataProvider>,
    resolved: RefCell<Option<Option<Vec<u8>>>>,
}

/// A promised type. Clones share the one resolution.
#[derive(Clone)]
pub struct Promise {
    inner: Rc<Inner>,
}

impl Promise {
    pub fn new(provider: Rc<dyn DataProvider>) -> Self {
        Self { inner: Rc::new(Inner { provider, resolved: RefCell::new(None) }) }
    }

    /// The bytes of `uti`, asking the provider the first time only.
    pub fn resolve(&self, uti: &str) -> Option<Vec<u8>> {
        if let Some(done) = self.inner.resolved.borrow().as_ref() {
            return done.clone();
        }
        let data = self.inner.provider.provide(uti);
        *self.inner.resolved.borrow_mut() = Some(data.clone());
        data
    }

    /// True once the provider has been asked.
    pub fn is_resolved(&self) -> bool {
        self.inner.resolved.borrow().is_some()
    }
}
