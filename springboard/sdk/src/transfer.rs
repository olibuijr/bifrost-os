//! `Transferable` parity (`TransferRepresentation`, `DataRepresentation`, `ProxyRepresentation`,
//! `TransferRepresentationVisibility`): one declaration per app model type of the representations it moves as through
//! copy, paste, drag and share. Each representation is a UTType id with an export function and, when the value can be
//! read back, an import function. Declaration order is the preference order.
//!
//! The conversions below are pure; [`begin_drag`] is the one call that talks to the shell. The bridges are [`pasteboard_item`]
//! and [`from_pasteboard_item`] (named pasteboards), [`drag_item`], [`begin_drag`] and [`drop_values`] (typed drag and drop). The pasteboard ([`crate::pasteboard`]), drag and drop
//! ([`crate::dnd`]) and the share picker ([`crate::share`]) take the bytes a representation exports and hand back the
//! bytes they received, so a model type needs one declaration instead of hand-written conversions at every site.
//!
//! Remaining gaps: no sender and receiver roles for file transfers (`SentTransferredFile`), and visibility only separates
//! the owning process from everyone else (no team level).

use crate::dnd::{self, DragItem, DragPreview, Payload};
use crate::pasteboard_types::Item as PasteItem;
use crate::Ctx;

/// `TransferRepresentationVisibility`: who may receive a representation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Visibility {
    /// `.all`: any app or process.
    All,
    /// `.ownProcess`: only the process that wrote the item.
    OwnProcess,
}

/// The party asking for a representation: the process that declared the model, or another one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Viewer {
    OwnProcess,
    Other,
}

impl Visibility {
    /// True when `viewer` may receive a representation with this visibility.
    pub fn is_visible_to(self, viewer: Viewer) -> bool {
        match self {
            Visibility::All => true,
            Visibility::OwnProcess => viewer == Viewer::OwnProcess,
        }
    }
}

/// One representation of `T`: a UTType id, how to export `T` to it and, when present, how to rebuild `T` from it.
/// A representation without an importer is a proxy: it can be offered but never read back.
pub struct Representation<T> {
    pub uti: &'static str,
    pub visibility: Visibility,
    export: fn(&T) -> Vec<u8>,
    import: Option<fn(&[u8]) -> Option<T>>,
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

impl<T> Representations<T> {
    pub fn new() -> Self {
        Self::default()
    }

    /// `DataRepresentation(contentType:exporting:importing:)`: both directions.
    pub fn data(
        mut self,
        uti: &'static str,
        visibility: Visibility,
        export: fn(&T) -> Vec<u8>,
        import: fn(&[u8]) -> Option<T>,
    ) -> Self {
        self.items.push(Representation { uti, visibility, export, import: Some(import) });
        self
    }

    /// `ProxyRepresentation(exporting:)`: export only.
    pub fn proxy(mut self, uti: &'static str, visibility: Visibility, export: fn(&T) -> Vec<u8>) -> Self {
        self.items.push(Representation { uti, visibility, export, import: None });
        self
    }

    /// The UTType ids `viewer` may see, in preference order: what a pasteboard or a drag offers it.
    pub fn types_for(&self, viewer: Viewer) -> Vec<&'static str> {
        self.items.iter().filter(|r| r.visibility.is_visible_to(viewer)).map(|r| r.uti).collect()
    }

    /// Exports `value` as `uti` for `viewer`. `None` when there is no such representation or `viewer` may not see it.
    pub fn export(&self, value: &T, uti: &str, viewer: Viewer) -> Option<Vec<u8>> {
        self.items
            .iter()
            .find(|r| r.uti == uti && r.visibility.is_visible_to(viewer))
            .map(|r| (r.export)(value))
    }

    /// Rebuilds a value from `bytes` of `uti`. `None` for an unknown type, a proxy, or bytes the importer rejects.
    pub fn import(&self, uti: &str, bytes: &[u8]) -> Option<T> {
        self.items
            .iter()
            .find(|r| r.uti == uti)
            .and_then(|r| r.import)
            .and_then(|f| f(bytes))
    }

    /// The first representation in preference order that the pasteboard or drop `offered` and this model can read back.
    pub fn best_import(&self, offered: &[&str]) -> Option<&'static str> {
        self.items
            .iter()
            .filter(|r| r.import.is_some())
            .find(|r| offered.contains(&r.uti))
            .map(|r| r.uti)
    }
}

/// A model type that declares how it moves through copy, paste, drag and share (`Transferable`).
pub trait Transferable: Sized {
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
    let mut payload = None;
    for uti in [TEXT_PAYLOAD, URL_PAYLOAD] {
        if let Some(bytes) = reps.export(value, uti, Viewer::Other) {
            if let Ok(text) = String::from_utf8(bytes) {
                payload = Some(if uti == URL_PAYLOAD { Payload::Url(text) } else { Payload::Text(text) });
                break;
            }
        }
    }
    Some(DragItem { types, payload: payload? })
}

/// Starts a drag of `value` (`beginDraggingSession`) and returns the session id. `None` when [`drag_item`] has nothing to
/// carry. The answer comes through the app's `App::drag` events, as for any session.
pub fn begin_drag<T>(ctx: &Ctx, allowed: u8, value: &T, reps: &Representations<T>, preview: DragPreview) -> Option<u32> {
    let item = drag_item(reps, value)?;
    Some(dnd::begin(ctx, allowed, vec![item], preview))
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
}
