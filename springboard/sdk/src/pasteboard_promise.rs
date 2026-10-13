//! Promised pasteboard data: `PasteboardItemDataProvider`, `PasteboardItem.setDataProvider(_:forTypes:)`,
//! `Pasteboard.declareTypes(_:owner:)` with `PasteboardTypeOwner`, and `FilePromiseProvider`.
//!
//! A promised type is listed in `types` without bytes. The first read asks the provider once and keeps the answer
//! (even "no data"); a read of nothing fails with [`PromiseError::NoData`]. Providers hear
//! `pasteboardFinishedWithDataProvider(_:)` once, when the board is cleared, another app takes over or the item stops
//! using them. The items themselves are [`crate::pasteboard_item::PasteboardItem`]; the board is
//! [`crate::pasteboard::NativePasteboard`] (`declare_types`).
//!
//! parity gap: promises live in the process that made them; the wire to other apps still carries resolved bytes only.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// Why a read of promised data failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PromiseError {
    /// The item or board has no such type.
    UnknownType(String),
    /// The provider returned nothing for the type.
    NoData(String),
    /// A file promise could not be written.
    Write(String),
}

/// `PasteboardItemDataProvider`: produces the bytes of a promised type on demand.
pub trait PasteboardItemDataProvider {
    /// `pasteboard(_:item:provideDataForType:)`. `None` when the data cannot be produced.
    fn provide_data(&self, uti: &str) -> Option<Vec<u8>>;
    /// `pasteboardFinishedWithDataProvider(_:)`.
    fn finished_with_data_provider(&self) {}
}

/// `PasteboardTypeOwner`: the owner named in `declareTypes(_:owner:)`.
pub trait PasteboardTypeOwner {
    /// `pasteboard(_:provideDataForType:)`.
    fn provide_data(&self, uti: &str) -> Option<Vec<u8>>;
    /// `pasteboardChangedOwner(_:)`.
    fn changed_owner(&self) {}
}

/// Serves the types of a declared-types owner through the item provider machinery.
pub(crate) struct OwnerProvider(pub Rc<dyn PasteboardTypeOwner>);

impl PasteboardItemDataProvider for OwnerProvider {
    fn provide_data(&self, uti: &str) -> Option<Vec<u8>> {
        self.0.provide_data(uti)
    }
    fn finished_with_data_provider(&self) {
        self.0.changed_owner();
    }
}

struct HandleInner {
    provider: Rc<dyn PasteboardItemDataProvider>,
    finished: Cell<bool>,
}

/// A provider shared by the promises it backs, so that it is told it is finished once, however many types it served.
#[derive(Clone)]
pub struct ProviderHandle {
    inner: Rc<HandleInner>,
}

impl ProviderHandle {
    pub fn new(provider: Rc<dyn PasteboardItemDataProvider>) -> Self {
        Self { inner: Rc::new(HandleInner { provider, finished: Cell::new(false) }) }
    }

    /// Tells the provider it is finished; idempotent.
    pub fn finish(&self) {
        if !self.inner.finished.replace(true) {
            self.inner.provider.finished_with_data_provider();
        }
    }

    /// True when both handles are the same provider registration.
    pub fn same(&self, other: &ProviderHandle) -> bool {
        Rc::ptr_eq(&self.inner, &other.inner)
    }
}

struct PromiseInner {
    handle: ProviderHandle,
    resolved: RefCell<Option<Option<Vec<u8>>>>,
}

/// A promised type. Clones share the one resolution.
#[derive(Clone)]
pub struct Promise {
    inner: Rc<PromiseInner>,
}

impl Promise {
    pub fn new(provider: Rc<dyn PasteboardItemDataProvider>) -> Self {
        Self::with_handle(ProviderHandle::new(provider))
    }

    /// A promise served by an already registered provider.
    pub fn with_handle(handle: ProviderHandle) -> Self {
        Self { inner: Rc::new(PromiseInner { handle, resolved: RefCell::new(None) }) }
    }

    pub fn handle(&self) -> &ProviderHandle {
        &self.inner.handle
    }

    /// The bytes of `uti`, asking the provider the first time only.
    pub fn resolve(&self, uti: &str) -> Option<Vec<u8>> {
        if let Some(done) = self.inner.resolved.borrow().as_ref() {
            return done.clone();
        }
        let data = self.inner.handle.inner.provider.provide_data(uti);
        *self.inner.resolved.borrow_mut() = Some(data.clone());
        data
    }

    /// True once the provider has been asked.
    pub fn is_resolved(&self) -> bool {
        self.inner.resolved.borrow().is_some()
    }

    /// The kept answer, without asking: `None` while unresolved or when the provider had no data.
    pub fn resolved_data(&self) -> Option<Vec<u8>> {
        self.inner.resolved.borrow().clone().flatten()
    }
}

/// `FilePromiseProviderDelegate`.
pub trait FilePromiseProviderDelegate {
    /// `filePromiseProvider(_:fileNameForType:)`.
    fn file_name_for_type(&self, file_type: &str) -> String;
    /// `filePromiseProvider(_:writePromiseTo:completionHandler:)`: write the file at `url`.
    fn write_promise_to(&mut self, url: &Path) -> Result<(), String>;
}

/// `FilePromiseProvider`.
pub struct FilePromiseProvider {
    pub file_type: String,
    delegate: Box<dyn FilePromiseProviderDelegate>,
}

impl FilePromiseProvider {
    pub fn new(file_type: &str, delegate: Box<dyn FilePromiseProviderDelegate>) -> Self {
        FilePromiseProvider { file_type: file_type.to_string(), delegate }
    }

    /// The name the delegate picks for the promised file.
    pub fn file_name(&self) -> String {
        self.delegate.file_name_for_type(&self.file_type)
    }

    /// Fulfils the promise inside `dir` and returns the file's path. File names that would leave `dir` are refused.
    pub fn write_promise(&mut self, dir: &Path) -> Result<PathBuf, PromiseError> {
        let name = self.file_name();
        if name.is_empty() || name.contains('/') || name == "." || name == ".." {
            return Err(PromiseError::Write(format!("bad file name {name:?}")));
        }
        let url = dir.join(&name);
        self.delegate.write_promise_to(&url).map_err(PromiseError::Write)?;
        Ok(url)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Delegate {
        name: String,
    }

    impl FilePromiseProviderDelegate for Delegate {
        fn file_name_for_type(&self, _file_type: &str) -> String {
            self.name.clone()
        }
        fn write_promise_to(&mut self, url: &Path) -> Result<(), String> {
            std::fs::write(url, b"promised").map_err(|e| e.to_string())
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("target").join("promise-tests").join(name);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn file_promise_writes_into_the_destination_with_the_provider_name() {
        let dir = scratch("write");
        let mut p = FilePromiseProvider::new("public.png", Box::new(Delegate { name: "shot.png".into() }));
        let path = p.write_promise(&dir).unwrap();
        assert_eq!(path, dir.join("shot.png"));
        assert_eq!(std::fs::read(path).unwrap(), b"promised");
    }

    #[test]
    fn file_promise_refuses_names_that_escape_the_destination() {
        let dir = scratch("escape");
        let mut p = FilePromiseProvider::new("public.png", Box::new(Delegate { name: "../evil".into() }));
        assert!(matches!(p.write_promise(&dir), Err(PromiseError::Write(_))));
    }

    struct Counting(Cell<u32>, Cell<u32>);

    impl PasteboardItemDataProvider for Counting {
        fn provide_data(&self, _uti: &str) -> Option<Vec<u8>> {
            self.0.set(self.0.get() + 1);
            Some(b"x".to_vec())
        }
        fn finished_with_data_provider(&self) {
            self.1.set(self.1.get() + 1);
        }
    }

    #[test]
    fn a_promise_asks_the_provider_once_and_a_handle_finishes_once() {
        let provider = Rc::new(Counting(Cell::new(0), Cell::new(0)));
        let handle = ProviderHandle::new(provider.clone());
        let (a, b) = (Promise::with_handle(handle.clone()), Promise::with_handle(handle.clone()));
        assert_eq!(a.resolved_data(), None);
        assert_eq!(a.resolve("public.html"), Some(b"x".to_vec()));
        assert_eq!(a.resolve("public.html"), Some(b"x".to_vec()));
        assert_eq!(b.resolve("public.png"), Some(b"x".to_vec()));
        assert_eq!(provider.0.get(), 2, "once per promise");
        assert!(a.handle().same(b.handle()));
        a.handle().finish();
        b.handle().finish();
        assert_eq!(provider.1.get(), 1, "finished once for the shared provider");
    }
}
