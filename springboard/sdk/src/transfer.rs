//! `Transferable` parity (`TransferRepresentation`, `DataRepresentation`, `ProxyRepresentation`,
//! `TransferRepresentationVisibility`): one declaration per app model type of the representations it moves as through
//! copy, paste, drag and share. Each representation is a UTType id with an export function and, when the value can be
//! read back, an import function. Declaration order is the preference order.
//!
//! This is the pure model and does not talk to the shell. The pasteboard ([`crate::pasteboard`]), drag and drop
//! ([`crate::dnd`]) and the share picker ([`crate::share`]) take the bytes a representation exports and hand back the
//! bytes they received, so a model type needs one declaration instead of hand-written conversions at every site.
//!
//! Remaining gaps: no sender and receiver roles for file transfers (`SentTransferredFile`), and visibility only separates
//! the owning process from everyone else (no team level).

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
}
