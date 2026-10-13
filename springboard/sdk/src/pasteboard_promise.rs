//! Promised pasteboard data: `NSPasteboardItemDataProvider`, `NSPasteboardItem.setDataProvider(_:forTypes:)`,
//! `NSPasteboard.declareTypes(_:owner:)` with `NSPasteboardTypeOwner`, and `NSFilePromiseProvider`.
//!
//! A promised type is listed in `types` without bytes. The first read asks the provider once and caches the answer;
//! a provider that returns nothing makes the read fail with [`PromiseError::NoData`]. Providers hear
//! `pasteboardFinishedWithDataProvider(_:)` when the board is cleared or another owner takes over.
//!
//! parity gap: promises live in the process that made them; the wire to other apps still carries resolved bytes only.

use std::path::{Path, PathBuf};

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

/// `NSPasteboardItemDataProvider`.
pub trait PasteboardItemDataProvider {
    /// `pasteboard(_:item:provideDataForType:)`.
    fn provide_data(&mut self, uti: &str) -> Option<Vec<u8>>;
    /// `pasteboardFinishedWithDataProvider(_:)`.
    fn finished_with_data_provider(&mut self) {}
}

/// `NSPasteboardItem` with lazily provided types.
#[derive(Default)]
pub struct PasteboardItem {
    data: Vec<(String, Vec<u8>)>,
    promised: Vec<String>,
    provider: Option<Box<dyn PasteboardItemDataProvider>>,
}

impl PasteboardItem {
    pub fn new() -> Self {
        Self::default()
    }

    /// `setData(_:forType:)`.
    pub fn set_data(&mut self, bytes: &[u8], uti: &str) {
        self.promised.retain(|t| t != uti);
        self.data.retain(|(t, _)| t != uti);
        self.data.push((uti.to_string(), bytes.to_vec()));
    }

    /// `setDataProvider(_:forTypes:)`: promises `types`; a previous provider is told it is finished.
    pub fn set_data_provider(&mut self, provider: Box<dyn PasteboardItemDataProvider>, types: &[&str]) {
        self.finish_provider();
        for t in types {
            self.data.retain(|(d, _)| d != t);
            if !self.promised.iter().any(|p| p == t) {
                self.promised.push((*t).to_string());
            }
        }
        self.provider = Some(provider);
    }

    /// `types`: stored and promised types, without resolving anything.
    pub fn types(&self) -> Vec<String> {
        self.data.iter().map(|(t, _)| t.clone()).chain(self.promised.iter().cloned()).collect()
    }

    /// `data(forType:)`: resolves a promise on the first read and caches it.
    pub fn data_for_type(&mut self, uti: &str) -> Result<Vec<u8>, PromiseError> {
        if let Some((_, d)) = self.data.iter().find(|(t, _)| t == uti) {
            return Ok(d.clone());
        }
        if !self.promised.iter().any(|p| p == uti) {
            return Err(PromiseError::UnknownType(uti.to_string()));
        }
        let provided = self.provider.as_mut().and_then(|p| p.provide_data(uti));
        match provided {
            Some(bytes) => {
                self.promised.retain(|p| p != uti);
                self.data.push((uti.to_string(), bytes.clone()));
                Ok(bytes)
            }
            None => Err(PromiseError::NoData(uti.to_string())),
        }
    }

    /// Tells the provider it is finished (board cleared or changed owner); idempotent.
    pub fn finish_provider(&mut self) {
        if let Some(mut p) = self.provider.take() {
            p.finished_with_data_provider();
        }
    }
}

/// `NSPasteboardTypeOwner`.
pub trait PasteboardTypeOwner {
    /// `pasteboard(_:provideDataForType:)`.
    fn provide_data(&mut self, uti: &str) -> Option<Vec<u8>>;
    /// `pasteboardChangedOwner(_:)`.
    fn changed_owner(&mut self) {}
}

/// A pasteboard holding items and an optional declared-types owner.
#[derive(Default)]
pub struct PromiseBoard {
    items: Vec<PasteboardItem>,
    declared: Vec<String>,
    owner: Option<Box<dyn PasteboardTypeOwner>>,
    change_count: u32,
}

impl PromiseBoard {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn change_count(&self) -> u32 {
        self.change_count
    }

    /// `clearContents`: providers and the owner are told, the change count advances.
    pub fn clear_contents(&mut self) -> u32 {
        for item in &mut self.items {
            item.finish_provider();
        }
        self.items.clear();
        self.declared.clear();
        if let Some(mut o) = self.owner.take() {
            o.changed_owner();
        }
        self.change_count = self.change_count.wrapping_add(1);
        self.change_count
    }

    /// `writeObjects`: adds an item.
    pub fn write_item(&mut self, item: PasteboardItem) {
        self.items.push(item);
    }

    /// `declareTypes(_:owner:)`: replaces the contents with promised `types` served by `owner`.
    pub fn declare_types(&mut self, types: &[&str], owner: Box<dyn PasteboardTypeOwner>) -> u32 {
        let n = self.clear_contents();
        self.declared = types.iter().map(|t| t.to_string()).collect();
        self.owner = Some(owner);
        n
    }

    /// `types`.
    pub fn types(&self) -> Vec<String> {
        let mut out = self.declared.clone();
        for item in &self.items {
            for t in item.types() {
                if !out.contains(&t) {
                    out.push(t);
                }
            }
        }
        out
    }

    /// `data(forType:)`: items first, then the owner once (cached as an item).
    pub fn data_for_type(&mut self, uti: &str) -> Result<Vec<u8>, PromiseError> {
        for item in &mut self.items {
            if item.types().iter().any(|t| t == uti) {
                return item.data_for_type(uti);
            }
        }
        if !self.declared.iter().any(|t| t == uti) {
            return Err(PromiseError::UnknownType(uti.to_string()));
        }
        let bytes = self
            .owner
            .as_mut()
            .and_then(|o| o.provide_data(uti))
            .ok_or_else(|| PromiseError::NoData(uti.to_string()))?;
        let mut item = PasteboardItem::new();
        item.set_data(&bytes, uti);
        self.items.push(item);
        self.declared.retain(|t| t != uti);
        Ok(bytes)
    }
}

/// `NSFilePromiseProviderDelegate`.
pub trait FilePromiseProviderDelegate {
    /// `filePromiseProvider(_:fileNameForType:)`.
    fn file_name_for_type(&self, file_type: &str) -> String;
    /// `filePromiseProvider(_:writePromiseTo:completionHandler:)`: write the file at `url`.
    fn write_promise_to(&mut self, url: &Path) -> Result<(), String>;
}

/// `NSFilePromiseProvider`.
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
    use std::cell::RefCell;
    use std::rc::Rc;

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
        fn provide_data(&mut self, uti: &str) -> Option<Vec<u8>> {
            self.log.borrow_mut().calls.push(uti.to_string());
            self.answer.clone()
        }
        fn finished_with_data_provider(&mut self) {
            self.log.borrow_mut().finished += 1;
        }
    }

    fn promised(answer: Option<&[u8]>) -> (PasteboardItem, Rc<RefCell<Log>>) {
        let log = Rc::new(RefCell::new(Log::default()));
        let mut item = PasteboardItem::new();
        item.set_data_provider(Box::new(Provider { log: log.clone(), answer: answer.map(|a| a.to_vec()) }), &["public.png"]);
        (item, log)
    }

    #[test]
    fn promised_types_are_listed_without_calling_the_provider() {
        let (item, log) = promised(Some(b"png"));
        assert_eq!(item.types(), vec!["public.png".to_string()]);
        assert!(log.borrow().calls.is_empty());
    }

    #[test]
    fn first_read_calls_the_provider_once_and_caches() {
        let (mut item, log) = promised(Some(b"png"));
        assert_eq!(item.data_for_type("public.png"), Ok(b"png".to_vec()));
        assert_eq!(item.data_for_type("public.png"), Ok(b"png".to_vec()));
        assert_eq!(log.borrow().calls, vec!["public.png".to_string()]);
    }

    #[test]
    fn provider_returning_nothing_fails_cleanly_and_unknown_types_are_reported() {
        let (mut item, _) = promised(None);
        assert_eq!(item.data_for_type("public.png"), Err(PromiseError::NoData("public.png".into())));
        assert_eq!(item.data_for_type("public.html"), Err(PromiseError::UnknownType("public.html".into())));
    }

    #[test]
    fn clearing_the_board_finishes_the_provider_once() {
        let (item, log) = promised(Some(b"x"));
        let mut board = PromiseBoard::new();
        board.write_item(item);
        assert_eq!(board.clear_contents(), 1);
        board.clear_contents();
        assert_eq!(log.borrow().finished, 1);
    }

    #[test]
    fn replacing_the_provider_finishes_the_old_one() {
        let (mut item, log) = promised(Some(b"x"));
        item.set_data_provider(Box::new(Provider { log: Rc::new(RefCell::new(Log::default())), answer: None }), &["public.html"]);
        assert_eq!(log.borrow().finished, 1);
    }

    struct Owner {
        log: Rc<RefCell<Log>>,
        changed: Rc<RefCell<u32>>,
    }

    impl PasteboardTypeOwner for Owner {
        fn provide_data(&mut self, uti: &str) -> Option<Vec<u8>> {
            self.log.borrow_mut().calls.push(uti.to_string());
            Some(b"owned".to_vec())
        }
        fn changed_owner(&mut self) {
            *self.changed.borrow_mut() += 1;
        }
    }

    #[test]
    fn declared_types_are_served_by_the_owner_once_and_owner_hears_of_change() {
        let log = Rc::new(RefCell::new(Log::default()));
        let changed = Rc::new(RefCell::new(0));
        let mut board = PromiseBoard::new();
        board.declare_types(&["public.html"], Box::new(Owner { log: log.clone(), changed: changed.clone() }));
        assert_eq!(board.types(), vec!["public.html".to_string()]);
        assert_eq!(board.data_for_type("public.html"), Ok(b"owned".to_vec()));
        assert_eq!(board.data_for_type("public.html"), Ok(b"owned".to_vec()));
        assert_eq!(log.borrow().calls.len(), 1);
        board.clear_contents();
        assert_eq!(*changed.borrow(), 1);
    }

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
}
