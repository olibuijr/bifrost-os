//! Document-based apps (`NativeDocument`, `NativeDocumentController`).
//!
//! An app implements [`Document`] (read bytes, write bytes) and owns a [`DocumentController`], which keeps the open
//! documents, the recent documents, the "Untitled N" numbering, the edited flag, autosave in place and the standard File
//! menu. The controller talks to the shell through [`DocumentHost`] (implemented for `Ctx`), so it is testable without a
//! shell. Typical wiring:
//!
//! ```ignore
//! fn document(&mut self, ctx: &Ctx, ev: DocumentEvent) -> bool { self.docs.handle(ctx, ev).is_some() }
//! fn main_menu(&self, _: &Ctx) -> Vec<AppMenu> { vec![self.docs.file_menu()] }
//! fn menu_action(&mut self, ctx: &Ctx, id: u32) -> bool { self.docs.menu_action(ctx, id).is_some() }
//! fn tick(&mut self, ctx: &Ctx, t: f64) -> bool { self.docs.tick(ctx, t); false }
//! fn next_wake(&self, _: f64) -> Option<f64> { self.docs.next_wake() }
//! fn suspend(&mut self, ctx: &Ctx) { self.docs.suspend(ctx) }
//! // after each edit: self.docs.note_change(id, self.undo.change_count())
//! ```
//!
//! What is the app's: the UI that shows a document, and presenting [`DocumentController::alert`] (a `bifrost_ui::nav::Alert`)
//! while it is `Some`, answering with [`DocumentController::answer_alert`]. Gaps against the native toolkit: docs/bifrost-ui.md "Documents (NativeDocument)".

use crate::proto::{AppMenu, AppMenuItem, DocErrorCode, DocMode, GrantId};
use crate::{DocumentEvent, OpenOptions};
use bifrost_ui::nav::{Alert, AlertStyle};
use crate::document_package;
use bifrost_ui::undo::ChangeCount;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// A failed read or write (`NativeError` for `read(from:ofType:)` / `data(ofType:)`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocumentError(pub String);

impl DocumentError {
    pub fn new(message: &str) -> Self {
        DocumentError(message.into())
    }
}

/// The model of one document (`NativeDocument`): turn bytes into it and it into bytes. Types are UTIs (`uttype`).
pub trait Document: Sized {
    /// `read(from:ofType:)`
    fn read(bytes: &[u8], uttype: &str) -> Result<Self, DocumentError>;
    /// `data(ofType:)`
    fn write(&self, uttype: &str) -> Result<Vec<u8>, DocumentError>;
    /// A new empty document (`File > New`).
    fn untitled() -> Self;
    /// `readableTypes`: the open panel's filters.
    fn readable_types() -> &'static [&'static str];
    /// The type of a new document and the one Save As offers.
    fn default_type() -> &'static str;
    /// Reads a folder package of `uttype` from its files (name and bytes, as `document_package::read_package_dir` gives
    /// them). Only the package types override it; the default refuses.
    fn read_package(_files: &[(String, Vec<u8>)], _uttype: &str) -> Result<Self, DocumentError> {
        Err(DocumentError::new("This document is not a package"))
    }
    /// The files of a folder package of `uttype` (`TXT.rtf` and the rest), which a save writes with
    /// `document_package::write_package_dir`. Only the package types override it; the default refuses.
    fn package_files(&self, _uttype: &str) -> Result<Vec<(String, Vec<u8>)>, DocumentError> {
        Err(DocumentError::new("This document is not a package"))
    }
}

/// Identifies an open document for the life of the controller.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DocId(pub u32);

/// What the controller needs from the runtime. `Ctx` implements it; tests use a recording stand-in.
pub trait DocumentHost {
    fn open_panel(&self, options: &OpenOptions) -> u32;
    fn save_panel(&self, suggested_name: &str, filters: &[&str]) -> u32;
    fn save_in_place(&self, grant: GrantId) -> u32;
    fn commit(&self, request: u32);
    fn abort(&self, request: u32);
    fn reopen(&self, grant: GrantId) -> u32;
    fn release(&self, grant: GrantId);
    fn set_document_state(&self, title: &str, edited: bool, represented: &str);
    fn data_dir(&self) -> PathBuf;
    fn pref(&self, key: &str) -> Option<String>;
    fn set_pref(&self, key: &str, value: &str);
}

impl DocumentHost for crate::Ctx {
    fn open_panel(&self, options: &OpenOptions) -> u32 {
        crate::Ctx::open_panel(self, options)
    }
    fn save_panel(&self, suggested_name: &str, filters: &[&str]) -> u32 {
        self.save_document(suggested_name, filters)
    }
    fn save_in_place(&self, grant: GrantId) -> u32 {
        self.save_document_in_place(grant)
    }
    fn commit(&self, request: u32) {
        self.commit_document(request)
    }
    fn abort(&self, request: u32) {
        self.abort_document(request)
    }
    fn reopen(&self, grant: GrantId) -> u32 {
        self.reopen_document(grant)
    }
    fn release(&self, grant: GrantId) {
        self.release_document(grant)
    }
    fn set_document_state(&self, title: &str, edited: bool, represented: &str) {
        crate::Ctx::set_document_state(self, title, edited, represented)
    }
    fn data_dir(&self) -> PathBuf {
        self.data_dir.clone()
    }
    fn pref(&self, key: &str) -> Option<String> {
        self.defaults().string(key)
    }
    fn set_pref(&self, key: &str, value: &str) {
        self.defaults_mut().set_string(key, value)
    }
}

/// Which command started a save panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveOp {
    /// Save of an untitled document, or Save As: the document now lives at the chosen file
    SaveAs,
    /// Rename: the document moves to the chosen name (the old file is not deleted yet, see the docs)
    Rename,
    /// Move To
    MoveTo,
    /// Save in place over the document's own file
    Save,
    /// Autosave in place
    Autosave,
}

/// What happened, for the app to react to (redraw, switch the visible document, reset its undo manager).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Notice {
    /// a new untitled document (`File > New`, or a duplicate)
    Created(DocId),
    /// a document opened from a file, or an untitled one restored after a crash or quit
    Opened(DocId),
    /// saved; for `SaveOp::SaveAs`/`Rename`/`MoveTo` the document now has the new name
    Saved(DocId, SaveOp),
    /// `Revert To Saved` replaced the content: reset the app's undo manager and views
    Reverted(DocId),
    Closed(DocId),
    /// a request failed or was cancelled (`code` is `Cancelled` when the user closed a panel)
    Failed { doc: Option<DocId>, code: Option<DocErrorCode>, message: String },
}

/// `NativeDocumentController.recentDocumentURLs`: the grant reopens the file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecentDocument {
    pub grant: GrantId,
    pub name: String,
}

/// Menu item ids of [`DocumentController::file_menu`]; pass the id the app receives in `menu_action` to
/// [`DocumentController::menu_action`]. [`is_menu_id`] tells whether an id is one of these.
pub mod item {
    pub const NEW: u32 = 0x4D01;
    pub const OPEN: u32 = 0x4D02;
    pub const CLOSE: u32 = 0x4D03;
    pub const SAVE: u32 = 0x4D04;
    pub const SAVE_AS: u32 = 0x4D05;
    pub const DUPLICATE: u32 = 0x4D06;
    pub const RENAME: u32 = 0x4D07;
    pub const MOVE_TO: u32 = 0x4D08;
    pub const REVERT: u32 = 0x4D09;
    pub const CLEAR_RECENTS: u32 = 0x4D0A;
    /// Open Recent entry `i` is `RECENT + i`
    pub const RECENT: u32 = 0x4D10;
}

pub fn is_menu_id(id: u32) -> bool {
    (item::NEW..=item::CLEAR_RECENTS).contains(&id) || (item::RECENT..item::RECENT + MAX_RECENTS as u32).contains(&id)
}

/// Alert button ids (`NativeAlert` response codes) of the close confirmation.
pub mod alert {
    pub const SAVE: u32 = 1;
    pub const CANCEL: u32 = 2;
    pub const DONT_SAVE: u32 = 3;
}

pub const MAX_RECENTS: usize = 10;
/// Autosave runs this long (ms) after the first unsaved change (`NativeDocument.autosavingDelay`).
pub const AUTOSAVE_DELAY_MS: f64 = 5000.0;
const RECENTS_KEY: &str = "document.recents";

/// A state of the document's content: its undo position plus manual `mark_edited` touches.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
struct Stamp {
    count: ChangeCount,
    touch: u32,
}

struct Entry<D> {
    id: DocId,
    doc: D,
    /// `Some(n)` for an untitled document: "Untitled" (1) or "Untitled n"
    untitled: Option<u32>,
    name: String,
    grant: Option<GrantId>,
    uttype: String,
    now: Stamp,
    saved: Stamp,
    /// when the first unsaved change was noticed (ms, the `tick` clock)
    edit_since: Option<f64>,
    /// the state an autosave last tried, so a failing one is not retried until the next change
    tried: Option<Stamp>,
    /// the folder a package document was read from, which a save writes back to (None for a file)
    package: Option<PathBuf>,
}

impl<D> Entry<D> {
    fn edited(&self) -> bool {
        self.now != self.saved
    }
    fn title(&self) -> String {
        match self.untitled {
            Some(1) => "Untitled".into(),
            Some(n) => format!("Untitled {n}"),
            None => self.name.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum After {
    Nothing,
    Close,
}

enum Pending {
    Open,
    /// reopening a recent document by its grant
    Recent(GrantId),
    Revert(DocId),
    Save { doc: DocId, op: SaveOp, after: After, stamp: Stamp },
}

struct CloseAlert {
    doc: DocId,
    alert: Alert,
}

/// `NativeDocumentController` plus the per-document bookkeeping of `NativeDocument`.
pub struct DocumentController<D: Document> {
    docs: Vec<Entry<D>>,
    active: Option<DocId>,
    next_id: u32,
    recents: Vec<RecentDocument>,
    recents_loaded: bool,
    pending: HashMap<u32, Pending>,
    alert: Option<CloseAlert>,
    sent: Option<(String, bool, String)>,
    /// autosave delay in ms; `AUTOSAVE_DELAY_MS` unless changed
    pub autosave_delay: f64,
    now: f64,
}

impl<D: Document> Default for DocumentController<D> {
    fn default() -> Self {
        Self::new()
    }
}

impl<D: Document> DocumentController<D> {
    pub fn new() -> Self {
        DocumentController { docs: Vec::new(), active: None, next_id: 0, recents: Vec::new(), recents_loaded: false, pending: HashMap::new(), alert: None, sent: None, autosave_delay: AUTOSAVE_DELAY_MS, now: 0.0 }
    }

    // ---- the document list ----

    pub fn ids(&self) -> Vec<DocId> {
        self.docs.iter().map(|e| e.id).collect()
    }
    pub fn len(&self) -> usize {
        self.docs.len()
    }
    pub fn is_empty(&self) -> bool {
        self.docs.is_empty()
    }
    pub fn get(&self, id: DocId) -> Option<&D> {
        self.entry(id).map(|e| &e.doc)
    }
    /// The model for editing. Tell the controller about the edit with `note_change` or `mark_edited`.
    pub fn get_mut(&mut self, id: DocId) -> Option<&mut D> {
        self.docs.iter_mut().find(|e| e.id == id).map(|e| &mut e.doc)
    }
    /// The document of the key window (`NativeDocumentController.currentDocument`).
    pub fn active(&self) -> Option<DocId> {
        self.active
    }
    pub fn set_active(&mut self, id: DocId) {
        if self.entry(id).is_some() {
            self.active = Some(id);
        }
    }
    /// `displayName`: "Untitled", "Untitled 2" or the file name.
    pub fn display_name(&self, id: DocId) -> Option<String> {
        self.entry(id).map(Entry::title)
    }
    /// The window title: the display name, with " - Edited" while the document has unsaved changes (the shell adds the
    /// suffix itself from `DocumentState`; this is for apps that draw their own title).
    pub fn window_title(&self, id: DocId) -> Option<String> {
        self.entry(id).map(|e| if e.edited() { format!("{} \u{2013} Edited", e.title()) } else { e.title() })
    }
    /// `isDocumentEdited`
    pub fn is_edited(&self, id: DocId) -> bool {
        self.entry(id).is_some_and(Entry::edited)
    }
    /// True when any open document has unsaved changes (ask before quitting).
    pub fn has_unsaved(&self) -> bool {
        self.docs.iter().any(Entry::edited)
    }
    /// The file name of a saved document (`representedFilename`); `None` for an untitled one.
    pub fn file_name(&self, id: DocId) -> Option<&str> {
        self.entry(id).filter(|e| e.untitled.is_none()).map(|e| e.name.as_str())
    }
    pub fn uttype(&self, id: DocId) -> Option<&str> {
        self.entry(id).map(|e| e.uttype.as_str())
    }

    fn entry(&self, id: DocId) -> Option<&Entry<D>> {
        self.docs.iter().find(|e| e.id == id)
    }
    fn entry_mut(&mut self, id: DocId) -> Option<&mut Entry<D>> {
        self.docs.iter_mut().find(|e| e.id == id)
    }

    fn add(&mut self, doc: D, untitled: Option<u32>, name: String, grant: Option<GrantId>, uttype: &str) -> DocId {
        self.next_id += 1;
        let id = DocId(self.next_id);
        self.docs.push(Entry { id, doc, untitled, name, grant, uttype: uttype.into(), now: Stamp::default(), saved: Stamp::default(), edit_since: None, tried: None, package: None });
        self.active = Some(id);
        id
    }

    fn free_untitled(&self) -> u32 {
        (1..).find(|n| !self.docs.iter().any(|e| e.untitled == Some(*n))).unwrap()
    }

    // ---- change tracking ----

    /// `updateChangeCount`: give the undo manager's position (`UndoManager::change_count`) after every edit, undo or redo.
    /// The document is edited exactly when it differs from the position at the last save, so undoing back to the saved
    /// state clears the flag.
    pub fn note_change(&mut self, id: DocId, count: ChangeCount) {
        if let Some(e) = self.entry_mut(id) {
            e.now.count = count;
        }
    }
    /// `updateChangeCount(.changeDone)` for apps without an undo manager.
    pub fn mark_edited(&mut self, id: DocId) {
        if let Some(e) = self.entry_mut(id) {
            e.now.touch += 1;
        }
    }

    // ---- new, open, recents ----

    /// `File > New`.
    pub fn new_document(&mut self) -> DocId {
        let n = self.free_untitled();
        self.add(D::untitled(), Some(n), String::new(), None, D::default_type())
    }

    /// `File > Open…`: the open panel; several files may be chosen. The answer arrives in `handle`.
    pub fn open_panel(&mut self, host: &dyn DocumentHost) {
        let request = host.open_panel(&OpenOptions {
            mode: DocMode::Write,
            title: "Open".into(),
            filters: D::readable_types().iter().map(|t| t.to_string()).collect(),
            multiple: true,
            ..Default::default()
        });
        self.pending.insert(request, Pending::Open);
    }

    /// `NativeDocumentController.recentDocumentURLs`, newest first.
    pub fn recents(&mut self, host: &dyn DocumentHost) -> &[RecentDocument] {
        self.load_recents(host);
        &self.recents
    }

    /// `File > Open Recent > name`. An already open document just becomes active.
    pub fn open_recent(&mut self, host: &dyn DocumentHost, index: usize) {
        self.load_recents(host);
        let Some(r) = self.recents.get(index).cloned() else { return };
        if let Some(id) = self.docs.iter().find(|e| e.grant == Some(r.grant)).map(|e| e.id) {
            self.active = Some(id);
            return;
        }
        let request = host.reopen(r.grant);
        self.pending.insert(request, Pending::Recent(r.grant));
    }

    /// `File > Open Recent > Clear Menu` (`clearRecentDocuments`).
    pub fn clear_recents(&mut self, host: &dyn DocumentHost) {
        self.load_recents(host);
        for r in std::mem::take(&mut self.recents) {
            if !self.docs.iter().any(|e| e.grant == Some(r.grant)) {
                host.release(r.grant);
            }
        }
        self.store_recents(host);
    }

    fn load_recents(&mut self, host: &dyn DocumentHost) {
        if self.recents_loaded {
            return;
        }
        self.recents_loaded = true;
        self.recents = host.pref(RECENTS_KEY).map(|t| parse_recents(&t)).unwrap_or_default();
    }

    fn store_recents(&self, host: &dyn DocumentHost) {
        host.set_pref(RECENTS_KEY, &format_recents(&self.recents));
    }

    /// `noteNewRecentDocumentURL`
    fn note_recent(&mut self, host: &dyn DocumentHost, grant: GrantId, name: &str) {
        self.load_recents(host);
        self.recents.retain(|r| r.grant != grant);
        self.recents.insert(0, RecentDocument { grant, name: name.into() });
        while self.recents.len() > MAX_RECENTS {
            let old = self.recents.pop().unwrap();
            if !self.docs.iter().any(|e| e.grant == Some(old.grant)) {
                host.release(old.grant);
            }
        }
        self.store_recents(host);
    }

    fn forget_recent(&mut self, host: &dyn DocumentHost, grant: GrantId) {
        self.load_recents(host);
        self.recents.retain(|r| r.grant != grant);
        self.store_recents(host);
        host.release(grant);
    }

    // ---- saving ----

    /// `File > Save` (`save(_:)`): in place for a saved document, the save panel for an untitled one.
    pub fn save(&mut self, host: &dyn DocumentHost, id: DocId) {
        self.save_then(host, id, After::Nothing);
    }

    fn save_then(&mut self, host: &dyn DocumentHost, id: DocId, after: After) {
        match self.entry(id).map(|e| e.grant) {
            Some(Some(grant)) => self.start_in_place(host, id, grant, SaveOp::Save, after),
            Some(None) => self.start_panel(host, id, SaveOp::SaveAs, after),
            None => {}
        }
    }

    /// `File > Save As…`
    pub fn save_as(&mut self, host: &dyn DocumentHost, id: DocId) {
        self.start_panel(host, id, SaveOp::SaveAs, After::Nothing);
    }
    /// `File > Rename…` (saved documents only)
    pub fn rename(&mut self, host: &dyn DocumentHost, id: DocId) {
        if self.entry(id).is_some_and(|e| e.untitled.is_none()) {
            self.start_panel(host, id, SaveOp::Rename, After::Nothing);
        }
    }
    /// `File > Move To…` (saved documents only)
    pub fn move_to(&mut self, host: &dyn DocumentHost, id: DocId) {
        if self.entry(id).is_some_and(|e| e.untitled.is_none()) {
            self.start_panel(host, id, SaveOp::MoveTo, After::Nothing);
        }
    }

    /// `File > Duplicate` (`duplicate(_:)`): a new untitled document with the same content, edited, in the same type.
    pub fn duplicate(&mut self, id: DocId) -> Notice {
        let Some(e) = self.entry(id) else { return failed(None, None, "no such document") };
        let uttype = e.uttype.clone();
        let copy = e.doc.write(&uttype).and_then(|bytes| D::read(&bytes, &uttype));
        match copy {
            Ok(doc) => {
                let n = self.free_untitled();
                let new = self.add(doc, Some(n), String::new(), None, &uttype);
                if let Some(e) = self.entry_mut(new) {
                    e.now.touch = 1;
                }
                Notice::Created(new)
            }
            Err(DocumentError(m)) => failed(Some(id), None, &m),
        }
    }

    /// `File > Revert To Saved`: reread the file and drop the changes. Answered with `Notice::Reverted`.
    pub fn revert(&mut self, host: &dyn DocumentHost, id: DocId) {
        let Some(grant) = self.entry(id).and_then(|e| e.grant) else { return };
        let request = host.reopen(grant);
        self.pending.insert(request, Pending::Revert(id));
    }

    fn start_panel(&mut self, host: &dyn DocumentHost, id: DocId, op: SaveOp, after: After) {
        let Some(e) = self.entry(id) else { return };
        let stamp = e.now;
        let ext = bifrost_ui::uttype::preferred_extension(&e.uttype).map(|x| format!(".{x}")).unwrap_or_default();
        let suggested = if e.untitled.is_some() || e.name.is_empty() { format!("{}{ext}", e.title()) } else { e.name.clone() };
        let uttype = e.uttype.clone();
        let request = host.save_panel(&suggested, &[uttype.as_str()]);
        self.pending.insert(request, Pending::Save { doc: id, op, after, stamp });
    }

    fn start_in_place(&mut self, host: &dyn DocumentHost, id: DocId, grant: GrantId, op: SaveOp, after: After) {
        let Some(e) = self.entry_mut(id) else { return };
        let stamp = e.now;
        e.tried = Some(stamp);
        e.edit_since = None;
        let request = host.save_in_place(grant);
        self.pending.insert(request, Pending::Save { doc: id, op, after, stamp });
    }

    fn saving(&self, id: DocId) -> bool {
        self.pending.values().any(|p| matches!(p, Pending::Save { doc, .. } if *doc == id))
    }

    // ---- closing ----

    /// `close()` with `canClose(withDelegate:)`: a clean document closes at once; an edited one raises the alert
    /// ([`DocumentController::alert`]) which the app shows and answers with `answer_alert`.
    pub fn close(&mut self, host: &dyn DocumentHost, id: DocId) -> Vec<Notice> {
        let Some(e) = self.entry(id) else { return Vec::new() };
        if !e.edited() {
            return self.finish_close(host, id).into_iter().collect();
        }
        let name = e.title();
        let alert = Alert::new(&format!("Do you want to save the changes made to the document \u{201c}{name}\u{201d}?"), "Your changes will be lost if you don\u{2019}t save them.")
            .action(alert::SAVE, "Save", AlertStyle::Default)
            .action(alert::CANCEL, "Cancel", AlertStyle::Cancel)
            .action(alert::DONT_SAVE, "Don\u{2019}t Save", AlertStyle::Destructive);
        self.active = Some(id);
        self.alert = Some(CloseAlert { doc: id, alert });
        Vec::new()
    }

    /// The close confirmation to present (Save is the default button; Golden Gate order Don't Save, Cancel, Save comes from
    /// `Alert::button_order`). `None` when no close is waiting.
    pub fn alert(&self) -> Option<&Alert> {
        self.alert.as_ref().map(|a| &a.alert)
    }

    /// The user pressed button `choice` (`alert::SAVE`, `CANCEL` or `DONT_SAVE`; the id of an `AlertAction`).
    pub fn answer_alert(&mut self, host: &dyn DocumentHost, choice: u32) -> Vec<Notice> {
        let Some(CloseAlert { doc, .. }) = self.alert.take() else { return Vec::new() };
        match choice {
            alert::SAVE => {
                self.save_then(host, doc, After::Close);
                Vec::new()
            }
            alert::DONT_SAVE => self.finish_close(host, doc).into_iter().collect(),
            _ => Vec::new(),
        }
    }

    fn finish_close(&mut self, host: &dyn DocumentHost, id: DocId) -> Option<Notice> {
        let i = self.docs.iter().position(|e| e.id == id)?;
        let e = self.docs.remove(i);
        if let Some(n) = e.untitled {
            let _ = std::fs::remove_file(autosave_path(&host.data_dir(), n));
        }
        self.pending.retain(|_, p| !matches!(p, Pending::Save { doc, .. } if *doc == id));
        if self.active == Some(id) {
            self.active = self.docs.last().map(|e| e.id);
        }
        Some(Notice::Closed(id))
    }

    // ---- answers from the shell ----

    /// Feeds a `DocumentEvent` in. `None` means the request was not the controller's (the app's own panel, say);
    /// otherwise the notices to react to (possibly empty while a save is half done).
    pub fn handle(&mut self, host: &dyn DocumentHost, ev: DocumentEvent) -> Option<Vec<Notice>> {
        let out = match ev {
            DocumentEvent::Opened(file) => self.on_opened(host, vec![file])?,
            DocumentEvent::OpenedMany { request, docs } => {
                if !self.pending.contains_key(&request) {
                    return None;
                }
                if docs.is_empty() {
                    self.pending.remove(&request);
                    vec![failed(None, Some(DocErrorCode::Cancelled), "nothing chosen")]
                } else {
                    self.on_opened(host, docs)?
                }
            }
            DocumentEvent::SaveTo(file) => self.on_save_to(host, file)?,
            DocumentEvent::Saved(file) => self.on_saved(host, file)?,
            DocumentEvent::Failed { request, code } => self.on_failed(host, request, code)?,
        };
        self.sync(host);
        Some(out)
    }

    fn on_opened(&mut self, host: &dyn DocumentHost, files: Vec<crate::Document>) -> Option<Vec<Notice>> {
        let request = files.first()?.request;
        let pending = self.pending.remove(&request)?;
        let mut out = Vec::new();
        for file in files {
            // a folder grant is a package (the panels hand over folders only for package types)
            if file.file.metadata().is_ok_and(|m| m.is_dir()) {
                out.push(self.open_package(host, file));
                continue;
            }
            let mut bytes = Vec::new();
            if let Err(e) = (&file.file).read_to_end(&mut bytes) {
                out.push(failed(None, None, &format!("cannot read {}: {e}", file.name)));
                continue;
            }
            let uttype = bifrost_ui::uttype::type_for_extension(&file.name).unwrap_or(D::default_type()).to_string();
            if let Pending::Revert(id) = pending {
                match D::read(&bytes, &uttype) {
                    Ok(doc) => {
                        if let Some(e) = self.entry_mut(id) {
                            e.doc = doc;
                            e.uttype = uttype;
                            e.saved = e.now;
                            e.edit_since = None;
                            out.push(Notice::Reverted(id));
                        }
                    }
                    Err(DocumentError(m)) => out.push(failed(Some(id), None, &m)),
                }
                continue;
            }
            if let Some(id) = self.docs.iter().find(|e| e.grant == Some(file.grant)).map(|e| e.id) {
                host.release(file.grant);
                self.active = Some(id);
                continue;
            }
            match D::read(&bytes, &uttype) {
                Ok(doc) => {
                    let id = self.add(doc, None, file.name.clone(), Some(file.grant), &uttype);
                    self.note_recent(host, file.grant, &file.name);
                    out.push(Notice::Opened(id));
                }
                Err(DocumentError(m)) => {
                    host.release(file.grant);
                    out.push(failed(None, None, &format!("{}: {m}", file.name)));
                }
            }
        }
        Some(out)
    }

    /// Opens a package folder: its files are read, and the document keeps the folder so a save writes it back.
    fn open_package(&mut self, host: &dyn DocumentHost, file: crate::Document) -> Notice {
        let name = file.name.clone();
        let uttype = bifrost_ui::uttype::type_for_extension(&name).unwrap_or(D::default_type()).to_string();
        let read = package_path(&file.file)
            .ok_or_else(|| "the package cannot be reached".to_string())
            .and_then(|path| {
                let files = document_package::read_package_dir(&path).map_err(|e| e.to_string())?;
                let doc = D::read_package(&files, &uttype).map_err(|DocumentError(m)| m)?;
                Ok((doc, path))
            });
        match read {
            Ok((doc, path)) => {
                let id = self.add(doc, None, name.clone(), Some(file.grant), &uttype);
                if let Some(e) = self.entry_mut(id) {
                    e.package = Some(path);
                }
                self.note_recent(host, file.grant, &name);
                Notice::Opened(id)
            }
            Err(m) => {
                host.release(file.grant);
                failed(None, None, &format!("{name}: {m}"))
            }
        }
    }

    /// A save of a package document: the package folder is written in place, atomically, and the temporary file the
    /// shell made for the save is discarded.
    fn save_package(&mut self, host: &dyn DocumentHost, request: u32, doc: DocId, op: SaveOp, stamp: Stamp, path: &Path) -> Vec<Notice> {
        self.pending.remove(&request);
        let written = match self.entry(doc) {
            Some(e) => e.doc.package_files(&e.uttype).map_err(|DocumentError(m)| m).and_then(|files| {
                let refs: Vec<(&str, &[u8])> = files.iter().map(|(n, b)| (n.as_str(), b.as_slice())).collect();
                document_package::write_package_dir(path, &refs).map_err(|err| err.to_string())
            }),
            None => Err("no such document".to_string()),
        };
        host.abort(request);
        match written {
            Ok(()) => {
                if let Some(e) = self.entry_mut(doc) {
                    e.saved = stamp;
                    e.edit_since = None;
                }
                vec![Notice::Saved(doc, op)]
            }
            Err(m) => vec![failed(Some(doc), None, &m)],
        }
    }

    fn on_save_to(&mut self, host: &dyn DocumentHost, file: crate::Document) -> Option<Vec<Notice>> {
        let request = file.request;
        let Some(Pending::Save { doc, op, stamp, .. }) = self.pending.get(&request) else { return None };
        let (doc, op, stamp) = (*doc, *op, *stamp);
        // a save in place of a package writes the folder, not the file the shell made
        if op == SaveOp::Save {
            if let Some(path) = self.entry(doc).filter(|e| bifrost_ui::uttype::is_package(&e.uttype)).and_then(|e| e.package.clone()) {
                return Some(self.save_package(host, request, doc, op, stamp, &path));
            }
        }
        // the type follows the chosen name when it has a known extension
        let named = bifrost_ui::uttype::type_for_extension(&file.name);
        let Some(e) = self.entry(doc) else {
            host.abort(request);
            self.pending.remove(&request);
            return Some(Vec::new());
        };
        let uttype = named.map(str::to_string).unwrap_or_else(|| e.uttype.clone());
        let result = if file.file.metadata().is_ok_and(|m| m.is_dir()) {
            // a package name: the shell made a folder for the save; the package's files go into it and commit puts it in place
            package_path(&file.file).ok_or_else(|| "the package cannot be reached".to_string()).and_then(|path| {
                let files = e.doc.package_files(&uttype).map_err(|DocumentError(m)| m)?;
                let refs: Vec<(&str, &[u8])> = files.iter().map(|(n, b)| (n.as_str(), b.as_slice())).collect();
                document_package::write_package_contents(&path, &refs).map_err(|err| err.to_string())
            })
        } else {
            e.doc.write(&uttype).map_err(|DocumentError(m)| m).and_then(|bytes| {
                let mut f = &file.file;
                f.set_len(0).and_then(|_| f.write_all(&bytes)).and_then(|_| f.sync_all()).map_err(|err| err.to_string())
            })
        };
        match result {
            Ok(()) => {
                host.commit(request);
                Some(Vec::new())
            }
            Err(m) => {
                host.abort(request);
                self.pending.remove(&request);
                Some(vec![failed(Some(doc), None, &m)])
            }
        }
    }

    fn on_saved(&mut self, host: &dyn DocumentHost, file: crate::Document) -> Option<Vec<Notice>> {
        let Some(Pending::Save { doc, op, after, stamp }) = self.pending.remove(&file.request) else { return None };
        let mut out = Vec::new();
        let Some(e) = self.entry_mut(doc) else { return Some(out) };
        e.saved = stamp;
        e.edit_since = None;
        let old = e.grant;
        let untitled = e.untitled.take();
        e.grant = Some(file.grant);
        e.name = file.name.clone();
        if let Some(t) = bifrost_ui::uttype::type_for_extension(&file.name) {
            e.uttype = t.to_string();
        }
        // a package keeps its folder, which a later save writes back to; a saved file has none
        e.package = if file.file.metadata().is_ok_and(|m| m.is_dir()) { package_path(&file.file) } else { None };
        if let Some(n) = untitled {
            let _ = std::fs::remove_file(autosave_path(&host.data_dir(), n));
        }
        if op != SaveOp::Autosave {
            self.note_recent(host, file.grant, &file.name);
        }
        if let Some(old) = old {
            if old != file.grant {
                host.release(old);
            }
        }
        out.push(Notice::Saved(doc, op));
        if after == After::Close && !self.is_edited(doc) {
            out.extend(self.finish_close(host, doc));
        }
        Some(out)
    }

    fn on_failed(&mut self, host: &dyn DocumentHost, request: u32, code: DocErrorCode) -> Option<Vec<Notice>> {
        let pending = self.pending.remove(&request)?;
        let doc = match pending {
            Pending::Save { doc, .. } => Some(doc),
            Pending::Revert(id) => Some(id),
            Pending::Open | Pending::Recent(_) => None,
        };
        // a recent whose file is gone or replaced leaves the list
        if let Pending::Recent(grant) = pending {
            if matches!(code, DocErrorCode::NotFound | DocErrorCode::Stale | DocErrorCode::Denied | DocErrorCode::Revoked) {
                self.forget_recent(host, grant);
            }
        }
        Some(vec![failed(doc, Some(code), &format!("{code:?}"))])
    }

    // ---- autosave ----

    /// Call every loop turn (`App::tick`) with the clock `t` (ms). Autosaves documents whose first unsaved change is
    /// older than `autosave_delay`: in place for saved documents (temporary file and rename through the grant), a copy
    /// in the app's data directory for untitled ones. Also keeps the shell's window state in step.
    pub fn tick(&mut self, host: &dyn DocumentHost, t: f64) {
        self.now = t;
        let delay = self.autosave_delay;
        let due: Vec<DocId> = self
            .docs
            .iter_mut()
            .filter_map(|e| {
                if !e.edited() {
                    e.edit_since = None;
                    return None;
                }
                let since = *e.edit_since.get_or_insert(t);
                (t - since >= delay && e.tried != Some(e.now)).then_some(e.id)
            })
            .collect();
        for id in due {
            self.autosave(host, id);
        }
        self.sync(host);
    }

    /// When `tick` next has work (ms), for `App::next_wake`.
    pub fn next_wake(&self) -> Option<f64> {
        self.docs.iter().filter(|e| e.edited() && e.tried != Some(e.now)).map(|e| e.edit_since.unwrap_or(self.now) + self.autosave_delay).reduce(f64::min)
    }

    /// `App::suspend`: write everything that is unsaved now.
    pub fn suspend(&mut self, host: &dyn DocumentHost) {
        let ids: Vec<DocId> = self.docs.iter().filter(|e| e.edited() && e.tried != Some(e.now)).map(|e| e.id).collect();
        for id in ids {
            self.autosave(host, id);
        }
    }

    fn autosave(&mut self, host: &dyn DocumentHost, id: DocId) {
        if self.saving(id) {
            return;
        }
        let Some(e) = self.entry(id) else { return };
        match (e.grant, e.untitled) {
            (Some(grant), _) => self.start_in_place(host, id, grant, SaveOp::Autosave, After::Nothing),
            (None, Some(n)) => {
                let stamp = e.now;
                let result = e.doc.write(&e.uttype).map_err(|DocumentError(m)| m).and_then(|bytes| write_autosave(&host.data_dir(), n, &e.uttype, &bytes).map_err(|err| err.to_string()));
                if let Some(e) = self.entry_mut(id) {
                    e.tried = Some(stamp);
                    e.edit_since = None;
                }
                let _ = result;
            }
            _ => {}
        }
    }

    /// Reopens the untitled documents that were autosaved before the app quit or crashed (`restoreWindow`). Call once
    /// at launch; the restored documents are edited, so closing them still asks. Returns what to show.
    pub fn restore(&mut self, host: &dyn DocumentHost) -> Vec<Notice> {
        let dir = host.data_dir().join(AUTOSAVE_DIR);
        let mut found: Vec<(u32, PathBuf)> = std::fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                let n = name.strip_prefix("untitled-")?.strip_suffix(".autosave")?.parse::<u32>().ok()?;
                (n >= 1).then(|| (n, e.path()))
            })
            .collect();
        found.sort();
        let mut out = Vec::new();
        for (n, path) in found {
            let Ok(raw) = std::fs::read(&path) else { continue };
            let Some(nl) = raw.iter().position(|b| *b == b'\n') else { continue };
            let uttype = String::from_utf8_lossy(&raw[..nl]).into_owned();
            match D::read(&raw[nl + 1..], &uttype) {
                Ok(doc) if !self.docs.iter().any(|e| e.untitled == Some(n)) => {
                    let id = self.add(doc, Some(n), String::new(), None, &uttype);
                    if let Some(e) = self.entry_mut(id) {
                        e.now.touch = 1;
                    }
                    out.push(Notice::Opened(id));
                }
                _ => {
                    let _ = std::fs::remove_file(&path);
                }
            }
        }
        self.sync(host);
        out
    }

    // ---- window state ----

    /// Sends the active document's title, edited flag and file name to the shell when they changed (the dirty dot in the
    /// close light and " - Edited" in the title bar). `tick` and `handle` call it; call it after your own edits too.
    pub fn sync(&mut self, host: &dyn DocumentHost) {
        let state = match self.active.and_then(|id| self.entry(id)) {
            Some(e) => (e.title(), e.edited(), if e.untitled.is_none() { e.name.clone() } else { String::new() }),
            None => (String::new(), false, String::new()),
        };
        if self.sent.as_ref() != Some(&state) {
            host.set_document_state(&state.0, state.1, &state.2);
            self.sent = Some(state);
        }
    }

    // ---- the File menu ----

    /// The standard File menu (`New`, `Open…`, `Open Recent`, `Close`, `Save`, `Save As…`, `Duplicate`, `Rename…`,
    /// `Move To…`, `Revert To Saved`), enabled for the active document. Return it from `App::main_menu`. Recents are
    /// the ones loaded so far (`recents()` or `restore` loads them).
    pub fn file_menu(&self) -> AppMenu {
        let active = self.active.and_then(|id| self.entry(id));
        let has = active.is_some();
        let saved = active.is_some_and(|e| e.untitled.is_none());
        let mut items = vec![
            AppMenuItem::new(item::NEW, "New").key("cmd+n"),
            AppMenuItem::new(item::OPEN, "Open\u{2026}").key("cmd+o"),
            AppMenuItem::submenu("Open Recent"),
        ];
        for (i, r) in self.recents.iter().enumerate() {
            items.push(AppMenuItem::new(item::RECENT + i as u32, &r.name).depth(1));
        }
        if !self.recents.is_empty() {
            items.push(AppMenuItem::separator().depth(1));
        }
        items.push(AppMenuItem::new(item::CLEAR_RECENTS, "Clear Menu").enabled(!self.recents.is_empty()).depth(1));
        items.extend([
            AppMenuItem::separator(),
            AppMenuItem::new(item::CLOSE, "Close").key("cmd+w").enabled(has),
            AppMenuItem::new(item::SAVE, "Save\u{2026}").key("cmd+s").enabled(has),
            AppMenuItem::new(item::SAVE_AS, "Save As\u{2026}").key("cmd+alt+shift+s").enabled(has),
            AppMenuItem::new(item::DUPLICATE, "Duplicate").key("cmd+shift+s").enabled(has),
            AppMenuItem::new(item::RENAME, "Rename\u{2026}").enabled(saved),
            AppMenuItem::new(item::MOVE_TO, "Move To\u{2026}").enabled(saved),
            AppMenuItem::new(item::REVERT, "Revert To Saved").enabled(saved && active.is_some_and(Entry::edited)),
        ]);
        // "Save…" only while an untitled document needs the panel, as the native toolkit's validateUserInterfaceItem retitles it
        if saved {
            if let Some(s) = items.iter_mut().find(|i| i.id == item::SAVE) {
                s.title = "Save".into();
            }
        }
        AppMenu { title: "File".into(), items }
    }

    /// Runs a File menu item chosen by the user (an id for which `is_menu_id` is true). `None` when the id is not ours;
    /// otherwise the notices (for `New` and `Duplicate`, the created document).
    pub fn menu_action(&mut self, host: &dyn DocumentHost, id: u32) -> Option<Vec<Notice>> {
        if !is_menu_id(id) {
            return None;
        }
        self.load_recents(host);
        let active = self.active;
        let mut out = Vec::new();
        match id {
            item::NEW => out.push(Notice::Created(self.new_document())),
            item::OPEN => self.open_panel(host),
            item::CLEAR_RECENTS => self.clear_recents(host),
            i if i >= item::RECENT => self.open_recent(host, (i - item::RECENT) as usize),
            _ => {
                let doc = active?;
                match id {
                    item::CLOSE => out = self.close(host, doc),
                    item::SAVE => self.save(host, doc),
                    item::SAVE_AS => self.save_as(host, doc),
                    item::DUPLICATE => out.push(self.duplicate(doc)),
                    item::RENAME => self.rename(host, doc),
                    item::MOVE_TO => self.move_to(host, doc),
                    item::REVERT => self.revert(host, doc),
                    _ => {}
                }
            }
        }
        self.sync(host);
        Some(out)
    }
}

fn failed(doc: Option<DocId>, code: Option<DocErrorCode>, message: &str) -> Notice {
    Notice::Failed { doc, code, message: message.into() }
}

// ---- autosave copies of untitled documents ----

const AUTOSAVE_DIR: &str = "autosave";

/// The path a granted descriptor was opened from (Linux keeps it in `/proc/self/fd`); `None` when it has none.
fn package_path(file: &std::fs::File) -> Option<PathBuf> {
    use std::os::unix::io::AsRawFd;
    std::fs::read_link(format!("/proc/self/fd/{}", file.as_raw_fd())).ok()
}

fn autosave_path(data_dir: &Path, n: u32) -> PathBuf {
    data_dir.join(AUTOSAVE_DIR).join(format!("untitled-{n}.autosave"))
}

/// First line the type, then the content; written to a temporary name and renamed, so a crash leaves the old copy.
fn write_autosave(data_dir: &Path, n: u32, uttype: &str, bytes: &[u8]) -> std::io::Result<()> {
    let path = autosave_path(data_dir, n);
    std::fs::create_dir_all(path.parent().unwrap())?;
    let tmp = path.with_extension("autosave.tmp");
    let mut f = std::fs::File::create(&tmp)?;
    f.write_all(uttype.as_bytes())?;
    f.write_all(b"\n")?;
    f.write_all(bytes)?;
    f.sync_all()?;
    std::fs::rename(&tmp, &path)
}

// ---- recents persistence ----

fn hex(g: &GrantId) -> String {
    g.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Option<GrantId> {
    let mut out = [0u8; 16];
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(s.get(2 * i..2 * i + 2)?, 16).ok()?;
    }
    Some(out)
}

fn format_recents(r: &[RecentDocument]) -> String {
    r.iter().map(|r| format!("{}\t{}\n", hex(&r.grant), r.name.replace(['\n', '\t'], " "))).collect()
}

fn parse_recents(t: &str) -> Vec<RecentDocument> {
    t.lines().filter_map(|l| l.split_once('\t').and_then(|(g, n)| Some(RecentDocument { grant: unhex(g)?, name: n.into() }))).take(MAX_RECENTS).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    #[derive(Debug, PartialEq)]
    struct Text(String);

    impl Document for Text {
        fn read(bytes: &[u8], _t: &str) -> Result<Self, DocumentError> {
            String::from_utf8(bytes.to_vec()).map(Text).map_err(|_| DocumentError::new("not UTF-8"))
        }
        fn write(&self, _t: &str) -> Result<Vec<u8>, DocumentError> {
            Ok(self.0.clone().into_bytes())
        }
        fn untitled() -> Self {
            Text(String::new())
        }
        fn readable_types() -> &'static [&'static str] {
            &["public.plain-text"]
        }
        fn default_type() -> &'static str {
            "public.plain-text"
        }
    }

    struct Mock {
        next: Cell<u32>,
        calls: RefCell<Vec<String>>,
        prefs: RefCell<HashMap<String, String>>,
        dir: PathBuf,
        state: RefCell<Option<(String, bool, String)>>,
    }

    impl Mock {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("bifrost-doc-test-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Mock { next: Cell::new(0), calls: RefCell::new(Vec::new()), prefs: RefCell::new(HashMap::new()), dir, state: RefCell::new(None) }
        }
        fn req(&self, what: &str) -> u32 {
            self.next.set(self.next.get() + 1);
            self.calls.borrow_mut().push(format!("{what}#{}", self.next.get()));
            self.next.get()
        }
        fn log(&self) -> Vec<String> {
            self.calls.borrow().clone()
        }
        /// A granted file with `content`, as the shell would hand it over.
        fn file(&self, request: u32, name: &str, grant: u8, content: &str) -> crate::Document {
            let path = self.dir.join(format!("{request}-{name}"));
            std::fs::write(&path, content).unwrap();
            let file = std::fs::OpenOptions::new().read(true).write(true).open(&path).unwrap();
            crate::Document { request, grant: [grant; 16], name: name.into(), size: content.len() as u64, mode: DocMode::Write, file }
        }
    }

    impl DocumentHost for Mock {
        fn open_panel(&self, _o: &OpenOptions) -> u32 {
            self.req("open")
        }
        fn save_panel(&self, name: &str, _f: &[&str]) -> u32 {
            self.req(&format!("panel:{name}"))
        }
        fn save_in_place(&self, g: GrantId) -> u32 {
            self.req(&format!("inplace:{}", g[0]))
        }
        fn commit(&self, r: u32) {
            self.calls.borrow_mut().push(format!("commit#{r}"));
        }
        fn abort(&self, r: u32) {
            self.calls.borrow_mut().push(format!("abort#{r}"));
        }
        fn reopen(&self, g: GrantId) -> u32 {
            self.req(&format!("reopen:{}", g[0]))
        }
        fn release(&self, g: GrantId) {
            self.calls.borrow_mut().push(format!("release:{}", g[0]));
        }
        fn set_document_state(&self, t: &str, e: bool, r: &str) {
            *self.state.borrow_mut() = Some((t.into(), e, r.into()));
        }
        fn data_dir(&self) -> PathBuf {
            self.dir.clone()
        }
        fn pref(&self, k: &str) -> Option<String> {
            self.prefs.borrow().get(k).cloned()
        }
        fn set_pref(&self, k: &str, v: &str) {
            self.prefs.borrow_mut().insert(k.into(), v.into());
        }
    }

    fn count(position: i64) -> ChangeCount {
        ChangeCount { position, epoch: 0 }
    }

    /// Opens a file named `name` with `content` through the controller's open panel.
    fn open(c: &mut DocumentController<Text>, m: &Mock, name: &str, grant: u8, content: &str) -> DocId {
        c.open_panel(m);
        let req = m.next.get();
        let file = m.file(req, name, grant, content);
        let notices = c.handle(m, DocumentEvent::OpenedMany { request: req, docs: vec![file] }).unwrap();
        let Notice::Opened(id) = notices[0] else { panic!("{notices:?}") };
        id
    }

    /// A document with a package form: its main file is `TXT.rtf`, its text is that file's contents.
    #[derive(Debug, PartialEq)]
    struct Pkg(String);

    impl Document for Pkg {
        fn read(bytes: &[u8], _t: &str) -> Result<Self, DocumentError> {
            String::from_utf8(bytes.to_vec()).map(Pkg).map_err(|_| DocumentError::new("not UTF-8"))
        }
        fn write(&self, _t: &str) -> Result<Vec<u8>, DocumentError> {
            Ok(self.0.clone().into_bytes())
        }
        fn untitled() -> Self {
            Pkg(String::new())
        }
        fn readable_types() -> &'static [&'static str] {
            &["public.rtfd"]
        }
        fn default_type() -> &'static str {
            "public.rtfd"
        }
        fn read_package(files: &[(String, Vec<u8>)], _t: &str) -> Result<Self, DocumentError> {
            let (_, main) = files.iter().find(|(n, _)| n == "TXT.rtf").ok_or_else(|| DocumentError::new("no TXT.rtf"))?;
            Self::read(main, "public.rtfd")
        }
        fn package_files(&self, _t: &str) -> Result<Vec<(String, Vec<u8>)>, DocumentError> {
            Ok(vec![("TXT.rtf".into(), self.0.clone().into_bytes())])
        }
    }

    #[test]
    fn a_package_folder_opens_as_one_document_and_keeps_its_folder() {
        let m = Mock::new("pkgopen");
        let pkg = m.dir.join("Notes.rtfd");
        std::fs::create_dir_all(&pkg).unwrap();
        std::fs::write(pkg.join("TXT.rtf"), "hello").unwrap();
        let mut c = DocumentController::<Pkg>::new();
        c.open_panel(&m);
        let req = m.next.get();
        // the grant of a folder is a read-only descriptor of the folder itself
        let file = crate::Document { request: req, grant: [9; 16], name: "Notes.rtfd".into(), size: 0, mode: DocMode::Read, file: std::fs::File::open(&pkg).unwrap() };
        let notices = c.handle(&m, DocumentEvent::OpenedMany { request: req, docs: vec![file] }).unwrap();
        let Notice::Opened(id) = notices[0] else { panic!("{notices:?}") };
        assert_eq!(c.get(id), Some(&Pkg("hello".into())));
        assert_eq!(c.uttype(id), Some("public.rtfd"));
        assert_eq!(c.file_name(id), Some("Notes.rtfd"));
        assert_eq!(c.entry(id).and_then(|e| e.package.as_ref()).and_then(|p| p.file_name()).map(|n| n.to_owned()), Some("Notes.rtfd".into()), "a save writes back to this folder");
    }

    #[test]
    fn save_as_to_a_package_name_writes_into_the_folder_the_shell_made_and_commits_it() {
        let m = Mock::new("pkgsave");
        let mut c = DocumentController::<Pkg>::new();
        let id = c.new_document();
        *c.get_mut(id).unwrap() = Pkg("draft".into());
        c.save_as(&m, id);
        let req = m.next.get();
        // the shell answers the panel with the temporary folder it made for "Notes.rtfd"
        let staging = m.dir.join("staging-Notes.rtfd");
        std::fs::create_dir_all(&staging).unwrap();
        let folder = crate::Document { request: req, grant: [4; 16], name: "Notes.rtfd".into(), size: 0, mode: DocMode::Write, file: std::fs::File::open(&staging).unwrap() };
        assert_eq!(c.handle(&m, DocumentEvent::SaveTo(folder)).unwrap(), Vec::new());
        assert_eq!(std::fs::read(staging.join("TXT.rtf")).unwrap(), b"draft", "the package's files are in the folder");
        assert!(m.log().contains(&format!("commit#{req}")), "{:?}", m.log());
        // the shell has put the folder in place and answers with it
        let done = crate::Document { request: req, grant: [5; 16], name: "Notes.rtfd".into(), size: 0, mode: DocMode::Write, file: std::fs::File::open(&staging).unwrap() };
        assert_eq!(c.handle(&m, DocumentEvent::Saved(done)).unwrap(), vec![Notice::Saved(id, SaveOp::SaveAs)]);
        assert_eq!(c.file_name(id), Some("Notes.rtfd"));
        assert_eq!(c.uttype(id), Some("public.rtfd"));
        assert!(c.entry(id).is_some_and(|e| e.package.is_some()), "the next save writes the folder");
        assert!(!c.is_edited(id));
    }

    #[test]
    fn untitled_documents_are_numbered_and_numbers_are_reused() {
        let mut c = DocumentController::<Text>::new();
        let (a, b, d) = (c.new_document(), c.new_document(), c.new_document());
        assert_eq!([c.display_name(a).unwrap(), c.display_name(b).unwrap(), c.display_name(d).unwrap()], ["Untitled", "Untitled 2", "Untitled 3"]);
        let m = Mock::new("num");
        c.close(&m, b);
        let e = c.new_document();
        assert_eq!(c.display_name(e).unwrap(), "Untitled 2", "the freed number comes back");
    }

    #[test]
    fn the_edited_flag_follows_the_undo_position_and_the_title_gets_a_suffix() {
        let mut c = DocumentController::<Text>::new();
        let id = c.new_document();
        assert!(!c.is_edited(id));
        c.note_change(id, count(1));
        assert!(c.is_edited(id));
        assert_eq!(c.window_title(id).unwrap(), "Untitled \u{2013} Edited");
        c.note_change(id, count(0));
        assert!(!c.is_edited(id), "undone back to the saved state");
        c.mark_edited(id);
        assert!(c.is_edited(id));
    }

    #[test]
    fn the_shell_is_told_the_title_the_dirty_flag_and_the_file_name_only_when_they_change() {
        let m = Mock::new("state");
        let mut c = DocumentController::<Text>::new();
        let id = c.new_document();
        c.sync(&m);
        assert_eq!(m.state.borrow().clone(), Some(("Untitled".into(), false, String::new())));
        c.note_change(id, count(1));
        c.sync(&m);
        assert_eq!(m.state.borrow().clone(), Some(("Untitled".into(), true, String::new())));
        *m.state.borrow_mut() = None;
        c.sync(&m);
        assert!(m.state.borrow().is_none(), "unchanged state is not sent again");
    }

    #[test]
    fn opening_a_file_reads_it_adds_a_recent_and_persists_it() {
        let m = Mock::new("open");
        let mut c = DocumentController::<Text>::new();
        let id = open(&mut c, &m, "notes.txt", 7, "hello");
        assert_eq!(c.get(id), Some(&Text("hello".into())));
        assert_eq!(c.display_name(id).unwrap(), "notes.txt");
        assert_eq!(c.file_name(id), Some("notes.txt"));
        assert!(!c.is_edited(id));
        let mut again = DocumentController::<Text>::new();
        let recents = again.recents(&m).to_vec();
        assert_eq!(recents, vec![RecentDocument { grant: [7; 16], name: "notes.txt".into() }]);
        // a request the controller did not make is not its own
        assert!(c.handle(&m, DocumentEvent::Failed { request: 999, code: DocErrorCode::Cancelled }).is_none());
    }

    #[test]
    fn opening_a_document_that_is_already_open_activates_it() {
        let m = Mock::new("dup");
        let mut c = DocumentController::<Text>::new();
        let a = open(&mut c, &m, "a.txt", 1, "a");
        let _b = open(&mut c, &m, "b.txt", 2, "b");
        c.open_panel(&m);
        let req = m.next.get();
        let n = c.handle(&m, DocumentEvent::OpenedMany { request: req, docs: vec![m.file(req, "a.txt", 1, "a")] }).unwrap();
        assert!(n.is_empty());
        assert_eq!(c.active(), Some(a));
        assert_eq!(c.len(), 2);
    }

    #[test]
    fn a_failed_read_is_reported_and_releases_the_grant() {
        let m = Mock::new("bad");
        let mut c = DocumentController::<Text>::new();
        c.open_panel(&m);
        let req = m.next.get();
        let file = m.file(req, "x.txt", 5, "");
        std::fs::write(m.dir.join(format!("{req}-x.txt")), [0xff, 0xfe]).unwrap();
        let n = c.handle(&m, DocumentEvent::Opened(file)).unwrap();
        assert!(matches!(n[0], Notice::Failed { .. }));
        assert!(m.log().contains(&"release:5".to_string()));
        assert!(c.is_empty());
    }

    #[test]
    fn saving_an_untitled_document_uses_the_panel_then_the_commit_and_clears_the_flag() {
        let m = Mock::new("saveas");
        let mut c = DocumentController::<Text>::new();
        let id = c.new_document();
        c.get_mut(id).unwrap().0 = "body".into();
        c.note_change(id, count(1));
        c.save(&m, id);
        let req = m.next.get();
        assert!(m.log()[0].starts_with("panel:Untitled.txt"), "{:?}", m.log());
        let to = m.file(req, "Letter.txt", 3, "");
        let (read_back, _) = (to.file.try_clone().unwrap(), ());
        assert_eq!(c.handle(&m, DocumentEvent::SaveTo(to)).unwrap(), vec![]);
        assert_eq!(m.log().last().unwrap(), &format!("commit#{req}"));
        let mut text = String::new();
        let mut f = read_back;
        std::io::Seek::seek(&mut f, std::io::SeekFrom::Start(0)).unwrap();
        f.read_to_string(&mut text).unwrap();
        assert_eq!(text, "body");
        let n = c.handle(&m, DocumentEvent::Saved(m.file(req, "Letter.txt", 3, "body"))).unwrap();
        assert_eq!(n, vec![Notice::Saved(id, SaveOp::SaveAs)]);
        assert!(!c.is_edited(id));
        assert_eq!(c.display_name(id).unwrap(), "Letter.txt");
        assert!(c.file_name(id).is_some());
    }

    #[test]
    fn an_edit_during_the_save_leaves_the_document_edited() {
        let m = Mock::new("race");
        let mut c = DocumentController::<Text>::new();
        let id = open(&mut c, &m, "a.txt", 1, "a");
        c.note_change(id, count(1));
        c.save(&m, id);
        let req = m.next.get();
        assert_eq!(m.log().last().unwrap(), &format!("inplace:1#{req}"));
        c.note_change(id, count(2));
        c.handle(&m, DocumentEvent::SaveTo(m.file(req, "a.txt", 1, ""))).unwrap();
        c.handle(&m, DocumentEvent::Saved(m.file(req, "a.txt", 1, ""))).unwrap();
        assert!(c.is_edited(id));
    }

    #[test]
    fn a_cancelled_save_panel_keeps_the_document_edited() {
        let m = Mock::new("cancel");
        let mut c = DocumentController::<Text>::new();
        let id = c.new_document();
        c.mark_edited(id);
        c.save_as(&m, id);
        let req = m.next.get();
        let n = c.handle(&m, DocumentEvent::Failed { request: req, code: DocErrorCode::Cancelled }).unwrap();
        assert!(matches!(n[0], Notice::Failed { code: Some(DocErrorCode::Cancelled), .. }));
        assert!(c.is_edited(id) && c.len() == 1);
    }

    #[test]
    fn closing_a_clean_document_closes_and_an_edited_one_asks() {
        let m = Mock::new("close");
        let mut c = DocumentController::<Text>::new();
        let clean = c.new_document();
        assert_eq!(c.close(&m, clean), vec![Notice::Closed(clean)]);
        let id = c.new_document();
        c.mark_edited(id);
        assert!(c.close(&m, id).is_empty());
        let a = c.alert().expect("the alert");
        assert!(a.title.contains("Untitled"));
        let labels: Vec<&str> = a.button_order().iter().map(|&i| a.actions()[i].label.as_str()).collect();
        assert_eq!(labels, ["Don\u{2019}t Save", "Cancel", "Save"], "Golden Gate button order");
        assert_eq!(a.default_action().unwrap().id, alert::SAVE);
        assert_eq!(a.cancel_action().unwrap().id, alert::CANCEL);
        // Cancel keeps it
        assert!(c.answer_alert(&m, alert::CANCEL).is_empty());
        assert_eq!(c.len(), 1);
        assert!(c.alert().is_none());
        // Don't Save discards it
        c.close(&m, id);
        assert_eq!(c.answer_alert(&m, alert::DONT_SAVE), vec![Notice::Closed(id)]);
        assert!(c.is_empty());
    }

    #[test]
    fn save_in_the_close_alert_saves_then_closes() {
        let m = Mock::new("closesave");
        let mut c = DocumentController::<Text>::new();
        let id = open(&mut c, &m, "a.txt", 4, "a");
        c.get_mut(id).unwrap().0 = "b".into();
        c.note_change(id, count(1));
        c.close(&m, id);
        assert!(c.answer_alert(&m, alert::SAVE).is_empty());
        let req = m.next.get();
        c.handle(&m, DocumentEvent::SaveTo(m.file(req, "a.txt", 4, ""))).unwrap();
        let n = c.handle(&m, DocumentEvent::Saved(m.file(req, "a.txt", 4, "b"))).unwrap();
        assert_eq!(n, vec![Notice::Saved(id, SaveOp::Save), Notice::Closed(id)]);
        assert!(c.is_empty());
    }

    #[test]
    fn duplicate_makes_an_edited_untitled_copy() {
        let m = Mock::new("duplicate");
        let mut c = DocumentController::<Text>::new();
        let id = open(&mut c, &m, "a.txt", 4, "text");
        let Notice::Created(copy) = c.duplicate(id) else { panic!() };
        assert_eq!(c.get(copy), Some(&Text("text".into())));
        assert_eq!(c.display_name(copy).unwrap(), "Untitled");
        assert!(c.is_edited(copy) && !c.is_edited(id));
    }

    #[test]
    fn revert_rereads_the_file_and_clears_the_flag() {
        let m = Mock::new("revert");
        let mut c = DocumentController::<Text>::new();
        let id = open(&mut c, &m, "a.txt", 4, "old");
        c.get_mut(id).unwrap().0 = "new".into();
        c.note_change(id, count(3));
        c.revert(&m, id);
        let req = m.next.get();
        let n = c.handle(&m, DocumentEvent::Opened(m.file(req, "a.txt", 4, "old"))).unwrap();
        assert_eq!(n, vec![Notice::Reverted(id)]);
        assert_eq!(c.get(id), Some(&Text("old".into())));
        assert!(!c.is_edited(id));
    }

    #[test]
    fn rename_and_move_to_are_for_saved_documents_and_use_the_panel() {
        let m = Mock::new("rename");
        let mut c = DocumentController::<Text>::new();
        let untitled = c.new_document();
        c.rename(&m, untitled);
        c.move_to(&m, untitled);
        assert!(m.log().is_empty());
        let id = open(&mut c, &m, "a.txt", 4, "x");
        let before = m.log().len();
        c.rename(&m, id);
        assert!(m.log()[before].starts_with("panel:a.txt"));
        let req = m.next.get();
        c.handle(&m, DocumentEvent::SaveTo(m.file(req, "b.txt", 9, ""))).unwrap();
        let n = c.handle(&m, DocumentEvent::Saved(m.file(req, "b.txt", 9, "x"))).unwrap();
        assert_eq!(n, vec![Notice::Saved(id, SaveOp::Rename)]);
        assert_eq!(c.display_name(id).unwrap(), "b.txt");
        assert!(m.log().contains(&"release:4".to_string()), "the old grant is released");
    }

    #[test]
    fn autosave_saves_a_saved_document_in_place_after_the_delay() {
        let m = Mock::new("autosave");
        let mut c = DocumentController::<Text>::new();
        let id = open(&mut c, &m, "a.txt", 6, "a");
        c.note_change(id, count(1));
        c.tick(&m, 1000.0);
        assert_eq!(c.next_wake(), Some(1000.0 + AUTOSAVE_DELAY_MS));
        c.tick(&m, 1000.0 + AUTOSAVE_DELAY_MS - 1.0);
        assert!(!m.log().iter().any(|l| l.starts_with("inplace")));
        c.tick(&m, 1000.0 + AUTOSAVE_DELAY_MS);
        let req = m.next.get();
        assert_eq!(m.log().last().unwrap(), &format!("inplace:6#{req}"));
        // the edit is still in flight; a second tick does not start a second save
        c.tick(&m, 20000.0);
        assert_eq!(m.next.get(), req);
        c.handle(&m, DocumentEvent::SaveTo(m.file(req, "a.txt", 6, ""))).unwrap();
        let n = c.handle(&m, DocumentEvent::Saved(m.file(req, "a.txt", 6, "a"))).unwrap();
        assert_eq!(n, vec![Notice::Saved(id, SaveOp::Autosave)]);
        assert!(!c.is_edited(id));
        assert!(c.next_wake().is_none());
    }

    #[test]
    fn a_failed_autosave_is_not_retried_until_the_next_change() {
        let m = Mock::new("autofail");
        let mut c = DocumentController::<Text>::new();
        let id = open(&mut c, &m, "a.txt", 6, "a");
        c.note_change(id, count(1));
        c.tick(&m, 0.0);
        c.tick(&m, AUTOSAVE_DELAY_MS);
        let req = m.next.get();
        c.handle(&m, DocumentEvent::Failed { request: req, code: DocErrorCode::Stale }).unwrap();
        c.tick(&m, 100000.0);
        assert_eq!(m.next.get(), req, "no retry");
        c.note_change(id, count(2));
        c.tick(&m, 100001.0);
        c.tick(&m, 100001.0 + AUTOSAVE_DELAY_MS);
        assert_eq!(m.next.get(), req + 1, "a new change is autosaved");
    }

    #[test]
    fn an_untitled_document_is_autosaved_to_a_copy_and_restored_on_the_next_launch() {
        let m = Mock::new("restore");
        let mut c = DocumentController::<Text>::new();
        let _first = c.new_document();
        let second = c.new_document();
        c.get_mut(second).unwrap().0 = "draft".into();
        c.mark_edited(second);
        c.suspend(&m);
        assert!(autosave_path(&m.dir, 2).exists());
        assert!(!autosave_path(&m.dir, 1).exists(), "an untouched document leaves no copy");
        // the next launch
        let mut next = DocumentController::<Text>::new();
        let n = next.restore(&m);
        assert_eq!(n.len(), 1);
        let Notice::Opened(id) = n[0] else { panic!() };
        assert_eq!(next.get(id), Some(&Text("draft".into())));
        assert_eq!(next.display_name(id).unwrap(), "Untitled 2");
        assert!(next.is_edited(id), "closing it still asks");
        // discarding it removes the copy
        next.close(&m, id);
        next.answer_alert(&m, alert::DONT_SAVE);
        assert!(!autosave_path(&m.dir, 2).exists());
    }

    #[test]
    fn the_file_menu_enables_items_for_the_active_document() {
        let m = Mock::new("menu");
        let mut c = DocumentController::<Text>::new();
        let enabled = |menu: &AppMenu, id: u32| menu.items.iter().find(|i| i.id == id).map(|i| i.enabled);
        let menu = c.file_menu();
        assert_eq!(menu.title, "File");
        assert_eq!(enabled(&menu, item::NEW), Some(true));
        assert_eq!(enabled(&menu, item::SAVE), Some(false), "no document");
        let u = c.new_document();
        let menu = c.file_menu();
        assert_eq!((enabled(&menu, item::SAVE), enabled(&menu, item::RENAME), enabled(&menu, item::REVERT)), (Some(true), Some(false), Some(false)));
        let id = open(&mut c, &m, "a.txt", 2, "x");
        c.close(&m, u);
        assert_eq!(c.active(), Some(id));
        c.note_change(id, count(1));
        let menu = c.file_menu();
        assert_eq!((enabled(&menu, item::RENAME), enabled(&menu, item::MOVE_TO), enabled(&menu, item::REVERT)), (Some(true), Some(true), Some(true)));
        let recent = menu.items.iter().find(|i| i.id == item::RECENT).expect("an Open Recent entry");
        assert_eq!((recent.title.as_str(), recent.depth), ("a.txt", 1));
        assert!(menu.items.iter().any(|i| i.submenu && i.title == "Open Recent"));
        assert!(menu.items.iter().filter(|i| i.id != 0).all(|i| is_menu_id(i.id)));
    }

    #[test]
    fn menu_items_drive_the_controller() {
        let m = Mock::new("menuact");
        let mut c = DocumentController::<Text>::new();
        assert_eq!(c.menu_action(&m, 7), None, "not ours");
        let n = c.menu_action(&m, item::NEW).unwrap();
        let Notice::Created(id) = n[0] else { panic!() };
        assert_eq!(c.active(), Some(id));
        c.menu_action(&m, item::SAVE_AS).unwrap();
        assert!(m.log()[0].starts_with("panel:"));
        c.menu_action(&m, item::OPEN).unwrap();
        assert!(m.log()[1].starts_with("open#"));
        let open_id = open(&mut c, &m, "r.txt", 8, "r");
        c.menu_action(&m, item::CLOSE).unwrap();
        assert!(!c.ids().contains(&open_id), "Close closed the clean active document");
        c.menu_action(&m, item::RECENT).unwrap();
        assert!(m.log().iter().any(|l| l.starts_with("reopen:8")));
        c.menu_action(&m, item::CLEAR_RECENTS).unwrap();
        assert!(c.recents(&m).is_empty());
        assert!(m.log().contains(&"release:8".to_string()));
    }

    #[test]
    fn recents_are_bounded_newest_first_and_survive_a_round_trip() {
        let m = Mock::new("recents");
        let mut c = DocumentController::<Text>::new();
        for i in 1..=12u8 {
            let id = open(&mut c, &m, &format!("f{i}.txt"), i, "x");
            c.close(&m, id);
        }
        let r = c.recents(&m).to_vec();
        assert_eq!(r.len(), MAX_RECENTS);
        assert_eq!(r[0].name, "f12.txt");
        assert!(m.log().contains(&"release:1".to_string()), "a grant that left the list is released");
        assert_eq!(parse_recents(&format_recents(&r)), r);
        assert_eq!(parse_recents("zz\tbad\n"), vec![]);
    }
}
