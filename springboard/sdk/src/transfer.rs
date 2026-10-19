//! `Transferable` parity (`TransferRepresentation`, `CodableRepresentation`, `DataRepresentation`, `FileRepresentation`,
//! `ProxyRepresentation`, `TransferRepresentationVisibility`, `SentTransferredFile`, `ReceivedTransferredFile`): one
//! declaration per app model type of the representations it moves as through copy, paste, drag and share. Each
//! representation is a UTType id with an export function and, when the value can be read back, an import function.
//! Declaration order is the preference order (`exportedContentType` is the first, `importedContentTypes` the importable ones).
//!
//! The conversions below are pure; [`begin_drag`] is the one call that talks to the shell. The bridges are [`pasteboard_item`]
//! and [`from_pasteboard_item`] (named pasteboards), [`drag_item`], [`begin_drag`] and [`drop_values`] (typed drag and drop). The pasteboard ([`crate::pasteboard`]), drag and drop
//! ([`crate::dnd`]), the item provider ([`crate::item_provider`]) and the share picker ([`crate::share`]) take the bytes a
//! representation exports and hand back the bytes they received, so a model type needs one declaration instead of
//! hand-written conversions at every site.
//!
//! Several values move as several items ([`drag_items`], [`begin_drag_all`], [`share_items_all`]), and a drop that brings
//! files is read back through the file representations ([`drop_values_with_files`]).
//!
//! Remaining gap: a file representation travels as bytes on a pasteboard item (the shell has no file promise yet).

use crate::dnd::{self, DragItem, DragPreview, Payload};
use crate::pasteboard_types::Item as PasteItem;
use crate::Ctx;
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// `TransferRepresentationVisibility`: who may receive a representation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Visibility {
    /// `.all`: any app or process.
    All,
    /// `.group` / team: processes of the same team, and the owner.
    Team,
    /// `.ownProcess`: only the process that wrote the item.
    OwnProcess,
}

/// The party asking for a representation: the process that declared the model, a process of the same team, or another one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Viewer {
    OwnProcess,
    Team,
    Other,
}

impl Visibility {
    /// True when `viewer` may receive a representation with this visibility.
    pub fn is_visible_to(self, viewer: Viewer) -> bool {
        match self {
            Visibility::All => true,
            Visibility::Team => viewer != Viewer::Other,
            Visibility::OwnProcess => viewer == Viewer::OwnProcess,
        }
    }

    /// The stricter of two visibilities (a proxy is never more visible than the type it forwards).
    fn narrow(self, other: Visibility) -> Visibility {
        let rank = |v: Visibility| match v {
            Visibility::All => 0,
            Visibility::Team => 1,
            Visibility::OwnProcess => 2,
        };
        if rank(other) > rank(self) {
            other
        } else {
            self
        }
    }
}

/// `SentTransferredFile`: a file the sender hands over. The receiver gets a copy unless `allow_open_in_place` is set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SentTransferredFile {
    pub file: PathBuf,
    pub allow_open_in_place: bool,
}

/// `ReceivedTransferredFile`: the file the receiver was given. `is_original_file` is true only when it is the sender's own file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReceivedTransferredFile {
    pub file: PathBuf,
    pub is_original_file: bool,
}

impl SentTransferredFile {
    pub fn new(file: impl Into<PathBuf>) -> Self {
        Self { file: file.into(), allow_open_in_place: false }
    }

    /// `init(_:allowAccessingOriginalFile:)`.
    pub fn allow_open_in_place(mut self, allow: bool) -> Self {
        self.allow_open_in_place = allow;
        self
    }

    /// The receiving side: with `allow_open_in_place` the receiver gets the original; otherwise the file is copied into
    /// `dest_dir` (same name) and the sender's file stays untouched.
    pub fn receive(&self, dest_dir: &Path) -> std::io::Result<ReceivedTransferredFile> {
        if self.allow_open_in_place {
            return Ok(ReceivedTransferredFile { file: self.file.clone(), is_original_file: true });
        }
        let name = self.file.file_name().ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "no file name"))?;
        let dest = dest_dir.join(name);
        std::fs::copy(&self.file, &dest)?;
        Ok(ReceivedTransferredFile { file: dest, is_original_file: false })
    }
}

/// Which `TransferRepresentation` a [`Representation`] was declared as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Codable,
    Data,
    File,
    Proxy,
}

/// The public JSON type a `CodableRepresentation` defaults to.
pub const JSON: &str = "public.json";

type Export<T> = Rc<dyn Fn(&T) -> Vec<u8>>;
type Import<T> = Rc<dyn Fn(&[u8]) -> Option<T>>;

type FileExport<T> = Rc<dyn Fn(&T) -> Option<SentTransferredFile>>;
type FileImport<T> = Rc<dyn Fn(&ReceivedTransferredFile) -> Option<T>>;

enum Io<T> {
    Bytes { export: Export<T>, import: Option<Import<T>> },
    /// A file representation. `export` gives `None` when the forwarded value has no file to hand over; `import` is `None`
    /// for an export-only file.
    File { export: FileExport<T>, import: Option<FileImport<T>> },
}

/// One representation of `T`: a UTType id, how to export `T` to it and, when present, how to rebuild `T` from it.
/// A representation without an importer is a proxy: it can be offered but never read back.
pub struct Representation<T> {
    pub uti: &'static str,
    pub visibility: Visibility,
    pub kind: Kind,
    io: Io<T>,
}

impl<T> Representation<T> {
    fn export_bytes(&self, value: &T) -> Option<Vec<u8>> {
        match &self.io {
            Io::Bytes { export, .. } => Some(export(value)),
            Io::File { export, .. } => std::fs::read(export(value)?.file).ok(),
        }
    }
    fn imports_bytes(&self) -> bool {
        matches!(&self.io, Io::Bytes { import: Some(_), .. })
    }
    fn imports(&self) -> bool {
        self.imports_bytes() || matches!(&self.io, Io::File { import: Some(_), .. })
    }
}

/// The ordered representations of one model type (`TransferRepresentationBuilder`).
pub struct Representations<T> {
    items: Vec<Representation<T>>,
}

impl<T> Default for Representations<T> {
    fn default() -> Self {
        Self { items: Vec::new() }
    }
}

impl<T: 'static> Representations<T> {
    pub fn new() -> Self {
        Self::default()
    }

    fn push_bytes(mut self, kind: Kind, uti: &'static str, visibility: Visibility, export: Export<T>, import: Option<Import<T>>) -> Self {
        self.items.push(Representation { uti, visibility, kind, io: Io::Bytes { export, import } });
        self
    }

    /// `DataRepresentation(contentType:exporting:importing:)`: both directions.
    pub fn data(self, uti: &'static str, visibility: Visibility, export: fn(&T) -> Vec<u8>, import: fn(&[u8]) -> Option<T>) -> Self {
        self.push_bytes(Kind::Data, uti, visibility, Rc::new(export), Some(Rc::new(import)))
    }

    /// `CodableRepresentation(contentType:)`: the model is encoded and decoded as structured text (JSON by default, [`JSON`]).
    pub fn codable(self, uti: &'static str, visibility: Visibility, encode: fn(&T) -> Vec<u8>, decode: fn(&[u8]) -> Option<T>) -> Self {
        self.push_bytes(Kind::Codable, uti, visibility, Rc::new(encode), Some(Rc::new(decode)))
    }

    /// `ProxyRepresentation(exporting:)`: export only.
    pub fn proxy(self, uti: &'static str, visibility: Visibility, export: fn(&T) -> Vec<u8>) -> Self {
        self.push_bytes(Kind::Proxy, uti, visibility, Rc::new(export), None)
    }

    /// `ProxyRepresentation(exporting:importing:)` through another `Transferable` `U`: every representation of `U` (bytes
    /// and file) is offered for `T` (converted with `to_proxy`), and read back through `from_proxy` when given. A proxy is
    /// never more visible than the representation it forwards.
    pub fn proxy_of<U: Transferable>(mut self, visibility: Visibility, to_proxy: fn(&T) -> U, from_proxy: Option<fn(U) -> T>) -> Self {
        let inner = Rc::new(U::transfer_representations());
        for rep in &inner.items {
            let uti = rep.uti;
            let vis = visibility.narrow(rep.visibility);
            let via = Rc::clone(&inner);
            match &rep.io {
                Io::Bytes { import, .. } => {
                    let export: Export<T> = Rc::new(move |t: &T| via.export(&to_proxy(t), uti, Viewer::OwnProcess).unwrap_or_default());
                    let import: Option<Import<T>> = match (from_proxy, import.is_some()) {
                        (Some(back), true) => {
                            let back_via = Rc::clone(&inner);
                            let f: Import<T> = Rc::new(move |b: &[u8]| back_via.import(uti, b).map(back));
                            Some(f)
                        }
                        _ => None,
                    };
                    self = self.push_bytes(Kind::Proxy, uti, vis, export, import);
                }
                Io::File { import, .. } => {
                    let export: FileExport<T> = Rc::new(move |t: &T| via.export_file(&to_proxy(t), uti, Viewer::OwnProcess));
                    let import: Option<FileImport<T>> = match (from_proxy, import.is_some()) {
                        (Some(back), true) => {
                            let back_via = Rc::clone(&inner);
                            let f: FileImport<T> = Rc::new(move |file: &ReceivedTransferredFile| back_via.import_file(uti, file).map(back));
                            Some(f)
                        }
                        _ => None,
                    };
                    self.items.push(Representation { uti, visibility: vis, kind: Kind::Proxy, io: Io::File { export, import } });
                }
            }
        }
        self
    }

    /// `FileRepresentation(contentType:exporting:importing:)`: the model travels as a file. The sender names the file; the
    /// receiver is handed a copy (see [`SentTransferredFile::receive`]) and rebuilds the model from it.
    pub fn file(
        mut self,
        uti: &'static str,
        visibility: Visibility,
        export: fn(&T) -> SentTransferredFile,
        import: fn(&ReceivedTransferredFile) -> Option<T>,
    ) -> Self {
        let export: FileExport<T> = Rc::new(move |t: &T| Some(export(t)));
        let import: FileImport<T> = Rc::new(move |file: &ReceivedTransferredFile| import(file));
        self.items.push(Representation { uti, visibility, kind: Kind::File, io: Io::File { export, import: Some(import) } });
        self
    }

}

impl<T> Representations<T> {
    /// The UTType ids `viewer` may see, in preference order: what a pasteboard or a drag offers it.
    pub fn types_for(&self, viewer: Viewer) -> Vec<&'static str> {
        self.items.iter().filter(|r| r.visibility.is_visible_to(viewer)).map(|r| r.uti).collect()
    }

    /// `exportedContentType`: the most preferred type `viewer` may see.
    pub fn exported_content_type(&self, viewer: Viewer) -> Option<&'static str> {
        self.types_for(viewer).first().copied()
    }

    /// `importedContentTypes`: every type the model can be rebuilt from, in preference order.
    pub fn imported_content_types(&self) -> Vec<&'static str> {
        self.items.iter().filter(|r| r.imports()).map(|r| r.uti).collect()
    }

    /// Exports `value` as `uti` for `viewer`. `None` when there is no such representation or `viewer` may not see it.
    /// A file representation exports the bytes of its file.
    pub fn export(&self, value: &T, uti: &str, viewer: Viewer) -> Option<Vec<u8>> {
        self.items.iter().find(|r| r.uti == uti && r.visibility.is_visible_to(viewer)).and_then(|r| r.export_bytes(value))
    }

    /// The file a file representation of `uti` hands over to `viewer`.
    pub fn export_file(&self, value: &T, uti: &str, viewer: Viewer) -> Option<SentTransferredFile> {
        self.items.iter().find(|r| r.uti == uti && r.visibility.is_visible_to(viewer)).and_then(|r| match &r.io {
            Io::File { export, .. } => export(value),
            Io::Bytes { .. } => None,
        })
    }

    /// Rebuilds a value from `bytes` of `uti`. `None` for an unknown type, a proxy, or bytes the importer rejects.
    pub fn import(&self, uti: &str, bytes: &[u8]) -> Option<T> {
        self.items.iter().find(|r| r.uti == uti).and_then(|r| match &r.io {
            Io::Bytes { import: Some(f), .. } => f(bytes),
            _ => None,
        })
    }

    /// Rebuilds a value from the file a sender handed over, through the file representation of `uti`.
    pub fn import_file(&self, uti: &str, file: &ReceivedTransferredFile) -> Option<T> {
        self.items.iter().find(|r| r.uti == uti).and_then(|r| match &r.io {
            Io::File { import: Some(import), .. } => import(file),
            _ => None,
        })
    }

    /// The first representation in preference order that the pasteboard or drop `offered` and this model can read back from bytes.
    pub fn best_import(&self, offered: &[&str]) -> Option<&'static str> {
        self.items.iter().filter(|r| r.imports_bytes()).find(|r| offered.contains(&r.uti)).map(|r| r.uti)
    }

    /// The first file representation, in preference order, that can rebuild the model from a received file of type
    /// `offered` (the file's own type conforms to the representation's type) and that is not export-only.
    pub fn best_file_import(&self, offered: &str) -> Option<&'static str> {
        self.items
            .iter()
            .find(|r| matches!(&r.io, Io::File { import: Some(_), .. }) && bifrost_ui::uttype::conforms(offered, r.uti))
            .map(|r| r.uti)
    }

    /// The `TransferRepresentation` kind declared for `uti`.
    pub fn kind_of(&self, uti: &str) -> Option<Kind> {
        self.items.iter().find(|r| r.uti == uti).map(|r| r.kind)
    }
}

/// A model type that declares how it moves through copy, paste, drag and share (`Transferable`).
pub trait Transferable: Sized + 'static {
    fn transfer_representations() -> Representations<Self>;
}

/// The payload kinds a drag item can carry, by the UTType it is natural as.
const TEXT_PAYLOAD: &str = "public.utf8-plain-text";
const URL_PAYLOAD: &str = "public.url";

/// The pasteboard item for `value`: one representation per type another app may see, in declaration order.
pub fn pasteboard_item<T>(reps: &Representations<T>, value: &T) -> PasteItem {
    let mut item = PasteItem::default();
    for uti in reps.types_for(Viewer::Other) {
        if let Some(bytes) = reps.export(value, uti, Viewer::Other) {
            item = item.with(uti, &bytes);
        }
    }
    item
}

/// Rebuilds a value from a pasteboard item (a paste, or a read of a named board): the first representation in the model's
/// preference order that the item holds and the model can import.
pub fn from_pasteboard_item<T>(reps: &Representations<T>, item: &PasteItem) -> Option<T> {
    let offered: Vec<&str> = item.representations.iter().map(|r| r.uti.as_str()).collect();
    let uti = reps.best_import(&offered)?;
    reps.import(uti, item.data(uti)?)
}

/// The drag item for `value`: offered as every type another app may see, carrying its text (or, failing that, its link) as
/// the payload. `None` when neither is visible, since a drag item carries text, a link or a file grant only.
pub fn drag_item<T>(reps: &Representations<T>, value: &T) -> Option<DragItem> {
    let types: Vec<String> = reps.types_for(Viewer::Other).iter().map(|t| t.to_string()).collect();
    Some(DragItem { types, payload: text_payload(reps, value)? })
}

/// The text (or, failing that, link) `value` exports to another app, as a drag or share payload.
fn text_payload<T>(reps: &Representations<T>, value: &T) -> Option<Payload> {
    for uti in [TEXT_PAYLOAD, URL_PAYLOAD] {
        if let Some(bytes) = reps.export(value, uti, Viewer::Other) {
            if let Ok(text) = String::from_utf8(bytes) {
                return Some(if uti == URL_PAYLOAD { Payload::Url(text) } else { Payload::Text(text) });
            }
        }
    }
    None
}

/// `ShareLink(item:)`: the items the share menu is shown for, taken from what `value` exports to another app. Empty when
/// no visible representation is text or a link, since a share carries text, links and file grants only.
pub fn share_items<T>(reps: &Representations<T>, value: &T) -> Vec<Payload> {
    text_payload(reps, value).into_iter().collect()
}

/// Starts a drag of `value` (`beginDraggingSession`) and returns the session id. `None` when [`drag_item`] has nothing to
/// carry. The answer comes through the app's `App::drag` events, as for any session.
pub fn begin_drag<T>(ctx: &Ctx, allowed: u8, value: &T, reps: &Representations<T>, preview: DragPreview) -> Option<u32> {
    let item = drag_item(reps, value)?;
    Some(dnd::begin(ctx, allowed, vec![item], preview))
}

/// The first available pasteboard item in `items`, read back through the model's preferred representation.
pub fn paste_values<T>(reps: &Representations<T>, items: &[PasteItem]) -> Vec<T> {
    items.iter().filter_map(|item| from_pasteboard_item(reps, item)).collect()
}

/// The values a drop carries, rebuilt by the model. An item is read through the representation its payload is natural as
/// (text or link) when the model can import it; file items and items the model cannot read are skipped.
pub fn drop_values<T>(reps: &Representations<T>, items: &[DragItem]) -> Vec<T> {
    items
        .iter()
        .filter_map(|item| {
            let (uti, text): (&str, &str) = match &item.payload {
                Payload::Text(t) => (TEXT_PAYLOAD, t.as_str()),
                Payload::Url(u) => (URL_PAYLOAD, u.as_str()),
                Payload::File { .. } => return None,
            };
            let uti = reps.best_import(&[uti])?;
            reps.import(uti, text.as_bytes())
        })
        .collect()
}

/// The drag items for several values, one item each, in order. A value that has no text or link to carry is left out,
/// and so is everything past the eight items a drag session holds.
pub fn drag_items<T>(reps: &Representations<T>, values: &[T]) -> Vec<DragItem> {
    values.iter().filter_map(|v| drag_item(reps, v)).take(MAX_SESSION_ITEMS).collect()
}

/// Starts a drag of several values as one session (a multi-selection drag) and returns the session id. `None` when no
/// value has anything to carry.
pub fn begin_drag_all<T>(ctx: &Ctx, allowed: u8, values: &[T], reps: &Representations<T>, preview: DragPreview) -> Option<u32> {
    let items = drag_items(reps, values);
    if items.is_empty() {
        return None;
    }
    Some(dnd::begin(ctx, allowed, items, preview))
}

/// The share menu's items for several values (`ShareLink` over a selection): the text or link each exports to another
/// app, in order, up to the eight items a share holds.
pub fn share_items_all<T>(reps: &Representations<T>, values: &[T]) -> Vec<Payload> {
    values.iter().filter_map(|v| text_payload(reps, v)).take(MAX_SESSION_ITEMS).collect()
}

/// The values a drop carries, like [`drop_values`], and also the files: a file item is read through the first file
/// representation that accepts its type. `resolve` turns the item's document grant and name into the path of the
/// arrived file (the app keeps the `App::document` events of the drop; see [`crate::dnd`]); an item whose file it
/// cannot find is skipped. The received file is the receiver's own copy, never the sender's original.
pub fn drop_values_with_files<T>(
    reps: &Representations<T>,
    items: &[DragItem],
    resolve: &dyn Fn(&crate::proto::GrantId, &str) -> Option<PathBuf>,
) -> Vec<T> {
    items
        .iter()
        .filter_map(|item| match &item.payload {
            Payload::File { grant, name } => {
                let uti = reps.best_file_import(&item.payload.uti())?;
                let file = resolve(grant, name)?;
                reps.import_file(uti, &ReceivedTransferredFile { file, is_original_file: false })
            }
            _ => drop_values(reps, std::slice::from_ref(item)).pop(),
        })
        .collect()
}

/// The items a drag or share session holds (the shell keeps the first eight).
const MAX_SESSION_ITEMS: usize = 8;

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq)]
    struct Note {
        text: String,
    }

    const PLAIN: &str = "public.utf8-plain-text";
    const HTML: &str = "public.html";
    const PRIVATE: &str = "is.bifrost.note.private";

    fn export_text(n: &Note) -> Vec<u8> {
        n.text.as_bytes().to_vec()
    }
    fn import_text(b: &[u8]) -> Option<Note> {
        String::from_utf8(b.to_vec()).ok().map(|text| Note { text })
    }
    fn export_html(n: &Note) -> Vec<u8> {
        format!("<p>{}</p>", n.text).into_bytes()
    }
    fn export_private(n: &Note) -> Vec<u8> {
        format!("secret:{}", n.text).into_bytes()
    }

    fn note_representations() -> Representations<Note> {
        Representations::new()
            .data(PLAIN, Visibility::All, export_text, import_text)
            .proxy(HTML, Visibility::All, export_html)
            .proxy(PRIVATE, Visibility::OwnProcess, export_private)
    }

    impl Transferable for Note {
        fn transfer_representations() -> Representations<Note> {
            note_representations()
        }
    }

    #[test]
    fn types_are_offered_in_declaration_order_and_filtered_by_visibility() {
        let reps = note_representations();
        assert_eq!(reps.types_for(Viewer::Other), vec![PLAIN, HTML]);
        assert_eq!(reps.types_for(Viewer::OwnProcess), vec![PLAIN, HTML, PRIVATE]);
    }

    #[test]
    fn own_process_visibility_hides_the_representation_from_other_processes() {
        let reps = note_representations();
        let note = Note { text: "hi".into() };
        assert_eq!(reps.export(&note, PRIVATE, Viewer::Other), None);
        assert_eq!(reps.export(&note, PRIVATE, Viewer::OwnProcess), Some(b"secret:hi".to_vec()));
    }

    #[test]
    fn data_representation_round_trips_export_and_import() {
        let reps = note_representations();
        let note = Note { text: "groceries".into() };
        let bytes = reps.export(&note, PLAIN, Viewer::Other).expect("plain text is exported");
        assert_eq!(reps.import(PLAIN, &bytes), Some(note));
    }

    #[test]
    fn proxy_representation_is_export_only() {
        let reps = note_representations();
        let note = Note { text: "x".into() };
        assert_eq!(reps.export(&note, HTML, Viewer::Other), Some(b"<p>x</p>".to_vec()));
        assert_eq!(reps.import(HTML, b"<p>x</p>"), None);
    }

    #[test]
    fn best_import_picks_the_first_declared_type_the_drop_offers() {
        let reps = note_representations();
        // HTML is offered first by the source, but the model declares plain text first and it can read that back.
        assert_eq!(reps.best_import(&[HTML, PLAIN]), Some(PLAIN));
        // A proxy is never chosen for import, and unknown types are ignored.
        assert_eq!(reps.best_import(&[HTML, "public.png"]), None);
    }

    #[test]
    fn import_rejects_bytes_the_importer_cannot_read() {
        let reps = note_representations();
        assert_eq!(reps.import(PLAIN, &[0xff, 0xfe]), None);
        assert_eq!(reps.import("public.png", b"abc"), None);
    }

    #[test]
    fn transferable_model_declares_its_representations() {
        let reps = Note::transfer_representations();
        assert_eq!(reps.types_for(Viewer::Other), vec![PLAIN, HTML]);
    }

    #[test]
    fn a_pasteboard_item_holds_every_type_another_app_may_see() {
        let item = pasteboard_item(&note_representations(), &Note { text: "hi".into() });
        let utis: Vec<&str> = item.representations.iter().map(|r| r.uti.as_str()).collect();
        assert_eq!(utis, vec![PLAIN, HTML]);
        assert_eq!(item.data(PLAIN), Some(&b"hi"[..]));
        assert_eq!(item.data(HTML), Some(&b"<p>hi</p>"[..]));
        assert_eq!(item.data(PRIVATE), None, "own-process types never leave the process");
    }

    #[test]
    fn a_pasteboard_item_rebuilds_the_value_from_the_importable_type() {
        let reps = note_representations();
        let item = PasteItem::default().with(HTML, b"<p>hi</p>").with(PLAIN, b"hi");
        assert_eq!(from_pasteboard_item(&reps, &item), Some(Note { text: "hi".into() }));
        // only the proxy (HTML) is on the item: nothing can be read back
        let proxy_only = PasteItem::default().with(HTML, b"<p>hi</p>");
        assert_eq!(from_pasteboard_item(&reps, &proxy_only), None);
        // bytes the importer rejects give nothing
        let bad = PasteItem::default().with(PLAIN, &[0xff, 0xfe]);
        assert_eq!(from_pasteboard_item(&reps, &bad), None);
    }

    #[test]
    fn a_drag_item_is_offered_as_the_visible_types_and_carries_its_text() {
        let item = drag_item(&note_representations(), &Note { text: "hi".into() }).unwrap();
        assert_eq!(item.types, vec![PLAIN.to_string(), HTML.to_string()]);
        assert_eq!(item.payload, Payload::Text("hi".into()));
        // a value with only own-process representations has nothing to drag
        let private = Representations::new().proxy(PRIVATE, Visibility::OwnProcess, export_private);
        assert!(drag_item(&private, &Note { text: "x".into() }).is_none());
    }

    #[test]
    fn share_items_carry_the_exported_text() {
        let reps = note_representations();
        assert_eq!(share_items(&reps, &Note { text: "hi".into() }), vec![Payload::Text("hi".into())]);
        let private = Representations::new().proxy(PRIVATE, Visibility::OwnProcess, export_private);
        assert!(share_items(&private, &Note { text: "x".into() }).is_empty());
    }

    #[test]
    fn paste_values_read_every_item_the_model_can_import() {
        let reps = note_representations();
        let items = [PasteItem::default().with(PLAIN, b"a"), PasteItem::default().with(HTML, b"<p>b</p>"), PasteItem::default().with(PLAIN, b"c")];
        let texts: Vec<String> = paste_values(&reps, &items).into_iter().map(|n| n.text).collect();
        assert_eq!(texts, ["a", "c"]);
    }

    const DOC_FILE: &str = "is.bifrost.doc-file";

    #[derive(Debug, PartialEq)]
    struct Doc(String);
    #[derive(Debug, PartialEq)]
    struct Wrapper(String);

    fn doc_file(d: &Doc) -> SentTransferredFile {
        SentTransferredFile::new(format!("/work/{}.txt", d.0))
    }
    fn doc_from_file(r: &ReceivedTransferredFile) -> Option<Doc> {
        r.file.file_stem().and_then(|s| s.to_str()).map(|s| Doc(s.to_owned()))
    }
    fn wrap(w: &Wrapper) -> Doc {
        Doc(w.0.clone())
    }
    fn unwrap_doc(d: Doc) -> Wrapper {
        Wrapper(d.0)
    }
    impl Transferable for Doc {
        fn transfer_representations() -> Representations<Doc> {
            Representations::new().file(DOC_FILE, Visibility::All, doc_file, doc_from_file)
        }
    }

    #[test]
    fn proxy_of_forwards_file_representations_with_the_stricter_visibility() {
        let reps = Representations::<Wrapper>::new().proxy_of::<Doc>(Visibility::Team, wrap, Some(unwrap_doc));
        assert!(reps.types_for(Viewer::Other).is_empty(), "a team-only proxy is hidden from other processes");
        assert_eq!(reps.types_for(Viewer::Team), vec![DOC_FILE]);
        assert_eq!(reps.kind_of(DOC_FILE), Some(Kind::Proxy));
        assert_eq!(reps.export_file(&Wrapper("report".into()), DOC_FILE, Viewer::Other), None);
        let sent = reps.export_file(&Wrapper("report".into()), DOC_FILE, Viewer::Team).expect("forwarded file");
        assert_eq!(sent.file, PathBuf::from("/work/report.txt"));
        assert_eq!(reps.imported_content_types(), vec![DOC_FILE]);
        let received = ReceivedTransferredFile { file: PathBuf::from("/in/memo.txt"), is_original_file: false };
        assert_eq!(reps.import_file(DOC_FILE, &received), Some(Wrapper("memo".into())));
    }

    #[test]
    fn export_only_forwarded_file_cannot_be_read_back() {
        let reps = Representations::<Wrapper>::new().proxy_of::<Doc>(Visibility::All, wrap, None);
        assert_eq!(reps.types_for(Viewer::Other), vec![DOC_FILE]);
        assert!(reps.imported_content_types().is_empty());
        let received = ReceivedTransferredFile { file: PathBuf::from("/in/memo.txt"), is_original_file: false };
        assert_eq!(reps.import_file(DOC_FILE, &received), None);
    }

    #[test]
    fn a_drop_rebuilds_text_items_and_skips_the_rest() {
        let reps = note_representations();
        let items = vec![
            DragItem::new(Payload::Text("hi".into())),
            // a link: the model has no importer for public.url
            DragItem::new(Payload::Url("https://example.org".into())),
            DragItem::new(Payload::File { grant: [1; 16], name: "a.txt".into() }),
        ];
        assert_eq!(drop_values(&reps, &items), vec![Note { text: "hi".into() }]);
    }

    #[test]
    fn several_values_become_several_drag_and_share_items_in_order() {
        let reps = note_representations();
        let notes: Vec<Note> = ["a", "b", "c"].iter().map(|t| Note { text: (*t).into() }).collect();
        let items = drag_items(&reps, &notes);
        assert_eq!(items.len(), 3);
        assert_eq!(items[1].payload, Payload::Text("b".into()));
        assert_eq!(share_items_all(&reps, &notes), vec![Payload::Text("a".into()), Payload::Text("b".into()), Payload::Text("c".into())]);
    }

    #[test]
    fn a_session_holds_at_most_eight_items_and_skips_values_with_nothing_to_carry() {
        let reps = note_representations();
        let many: Vec<Note> = (0..12).map(|i| Note { text: i.to_string() }).collect();
        assert_eq!(drag_items(&reps, &many).len(), 8);
        assert_eq!(share_items_all(&reps, &many).len(), 8);
        // a model whose only representation is private has nothing another app may receive
        let private: Representations<Note> = Representations::new().proxy(PRIVATE, Visibility::OwnProcess, export_private);
        assert!(drag_items(&private, &many).is_empty());
        assert!(share_items_all(&private, &many).is_empty());
    }

    #[test]
    fn a_dropped_file_is_read_through_the_file_representation_that_accepts_its_type() {
        let reps = Representations::new().file("public.plain-text", Visibility::All, |d: &Doc| SentTransferredFile::new(format!("/work/{}.txt", d.0)), |r| {
            r.file.file_stem().and_then(|s| s.to_str()).map(|s| Doc(s.to_owned()))
        });
        let items = vec![
            DragItem::new(Payload::File { grant: [3; 16], name: "memo.txt".into() }),
            // a picture is not plain text: no file representation takes it
            DragItem::new(Payload::File { grant: [4; 16], name: "photo.png".into() }),
            // a file whose grant the app cannot resolve is skipped
            DragItem::new(Payload::File { grant: [5; 16], name: "lost.txt".into() }),
        ];
        let resolve = |grant: &crate::proto::GrantId, name: &str| (grant[0] != 5).then(|| PathBuf::from(format!("/in/{name}")));
        assert_eq!(drop_values_with_files(&reps, &items, &resolve), vec![Doc("memo".into())]);
        assert_eq!(reps.best_file_import("public.png"), None);
        assert_eq!(reps.best_file_import("public.plain-text"), Some("public.plain-text"));
    }

    #[test]
    fn text_items_in_a_mixed_drop_are_still_read_and_an_export_only_file_representation_reads_nothing() {
        let reps = note_representations();
        let items = vec![DragItem::new(Payload::Text("hi".into())), DragItem::new(Payload::File { grant: [1; 16], name: "a.txt".into() })];
        let resolve = |_: &crate::proto::GrantId, name: &str| Some(PathBuf::from(format!("/in/{name}")));
        assert_eq!(drop_values_with_files(&reps, &items, &resolve), vec![Note { text: "hi".into() }]);
    }
}
