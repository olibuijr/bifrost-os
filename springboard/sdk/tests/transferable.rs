//! Contract test for `Transferable` (Core Transferable) with `PasteboardItem` promises and `ItemProvider`.

use bifrost_app::item_provider::{ItemProviderError, ItemProvider};
use bifrost_app::pasteboard_item::PasteboardItem;
use bifrost_app::pasteboard_promise::PasteboardItemDataProvider;
use bifrost_app::pasteboard_types::Item;
use bifrost_app::transfer::{
    from_pasteboard_item, pasteboard_item, Kind, ReceivedTransferredFile, Representations, SentTransferredFile, Transferable, Viewer,
    Visibility, JSON,
};
use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

const NOTE_FILE: &str = "is.bifrost.note-file";
const PLAIN: &str = "public.utf8-plain-text";
const HTML: &str = "public.html";
const PRIVATE: &str = "is.bifrost.note.private";
const TEAM: &str = "is.bifrost.note.team";

#[derive(Debug, Clone, PartialEq)]
struct Note {
    text: String,
}

fn note(t: &str) -> Note {
    Note { text: t.into() }
}
fn to_json(n: &Note) -> Vec<u8> {
    format!("{{\"text\":\"{}\"}}", n.text).into_bytes()
}
fn from_json(b: &[u8]) -> Option<Note> {
    let s = std::str::from_utf8(b).ok()?;
    let t = s.strip_prefix("{\"text\":\"")?.strip_suffix("\"}")?;
    Some(note(t))
}
fn to_text(n: &Note) -> Vec<u8> {
    n.text.clone().into_bytes()
}
fn from_text(b: &[u8]) -> Option<Note> {
    String::from_utf8(b.to_vec()).ok().map(|text| Note { text })
}
fn to_html(n: &Note) -> Vec<u8> {
    format!("<p>{}</p>", n.text).into_bytes()
}
fn note_path(n: &Note) -> SentTransferredFile {
    let dir = std::env::temp_dir().join(format!("bifrost-transferable-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join(format!("{}.note", n.text));
    std::fs::write(&f, &n.text).unwrap();
    SentTransferredFile::new(f)
}
fn note_from_file(r: &ReceivedTransferredFile) -> Option<Note> {
    std::fs::read_to_string(&r.file).ok().map(|text| Note { text })
}

impl Transferable for Note {
    fn transfer_representations() -> Representations<Note> {
        Representations::new()
            .codable(JSON, Visibility::All, to_json, from_json)
            .data(PLAIN, Visibility::All, to_text, from_text)
            .file(NOTE_FILE, Visibility::All, note_path, note_from_file)
            .proxy(HTML, Visibility::All, to_html)
            .proxy(TEAM, Visibility::Team, to_text)
            .proxy(PRIVATE, Visibility::OwnProcess, to_text)
    }
}

#[test]
fn representations_are_exported_in_preference_order() {
    let reps = Note::transfer_representations();
    assert_eq!(reps.types_for(Viewer::Other), vec![JSON, PLAIN, NOTE_FILE, HTML]);
    assert_eq!(reps.exported_content_type(Viewer::Other), Some(JSON));
    assert_eq!(reps.kind_of(JSON), Some(Kind::Codable));
    assert_eq!(reps.kind_of(PLAIN), Some(Kind::Data));
    assert_eq!(reps.kind_of(NOTE_FILE), Some(Kind::File));
    assert_eq!(reps.kind_of(HTML), Some(Kind::Proxy));
}

#[test]
fn imported_content_types_are_the_ones_the_model_can_read_back() {
    assert_eq!(Note::transfer_representations().imported_content_types(), vec![JSON, PLAIN, NOTE_FILE]);
}

#[test]
fn import_uses_the_best_available_type() {
    let reps = Note::transfer_representations();
    let item = Item::default().with(HTML, b"<p>a</p>").with(PLAIN, b"plain").with(JSON, b"{\"text\":\"json\"}");
    assert_eq!(from_pasteboard_item(&reps, &item), Some(note("json")), "codable is declared first");
    let plain_only = Item::default().with(HTML, b"<p>a</p>").with(PLAIN, b"plain");
    assert_eq!(from_pasteboard_item(&reps, &plain_only), Some(note("plain")));
}

#[test]
fn visibility_filters_all_team_and_own_process() {
    let reps = Note::transfer_representations();
    assert_eq!(reps.types_for(Viewer::Team), vec![JSON, PLAIN, NOTE_FILE, HTML, TEAM]);
    assert_eq!(reps.types_for(Viewer::OwnProcess), vec![JSON, PLAIN, NOTE_FILE, HTML, TEAM, PRIVATE]);
    assert_eq!(reps.export(&note("x"), TEAM, Viewer::Other), None);
    assert_eq!(reps.export(&note("x"), TEAM, Viewer::Team), Some(b"x".to_vec()));
    assert_eq!(reps.export(&note("x"), PRIVATE, Viewer::Team), None);
    let item = pasteboard_item(&reps, &note("x"));
    assert!(item.data(TEAM).is_none() && item.data(PRIVATE).is_none());
}

#[test]
fn a_file_transfer_has_sender_and_receiver_roles_with_copy_semantics() {
    let reps = Note::transfer_representations();
    let sent = reps.export_file(&note("report"), NOTE_FILE, Viewer::Other).expect("file representation");
    assert!(!sent.allow_open_in_place);
    let dest: PathBuf = std::env::temp_dir().join(format!("bifrost-transferable-recv-{}", std::process::id()));
    std::fs::create_dir_all(&dest).unwrap();
    let received = sent.receive(&dest).unwrap();
    assert!(!received.is_original_file, "the receiver holds a copy");
    assert_ne!(received.file, sent.file);
    std::fs::remove_file(&sent.file).unwrap();
    assert_eq!(reps.import_file(NOTE_FILE, &received), Some(note("report")), "the copy outlives the sender's file");
    // opening in place hands over the original
    let original = SentTransferredFile::new("/some/file.note").allow_open_in_place(true).receive(&dest).unwrap();
    assert!(original.is_original_file);
    assert_eq!(original.file, PathBuf::from("/some/file.note"));
}

#[test]
fn proxy_of_forwards_another_transferables_representations() {
    #[derive(Debug, PartialEq)]
    struct Task(String);
    fn task_note(t: &Task) -> Note {
        note(&t.0)
    }
    fn note_task(n: Note) -> Task {
        Task(n.text)
    }
    impl Transferable for Task {
        fn transfer_representations() -> Representations<Task> {
            Representations::new().proxy_of::<Note>(Visibility::All, task_note, Some(note_task))
        }
    }
    let reps = Task::transfer_representations();
    assert_eq!(reps.types_for(Viewer::Other), vec![JSON, PLAIN, HTML]);
    assert_eq!(reps.types_for(Viewer::OwnProcess), vec![JSON, PLAIN, HTML, TEAM, PRIVATE], "inner visibility is kept");
    assert_eq!(reps.export(&Task("t".into()), PLAIN, Viewer::Other), Some(b"t".to_vec()));
    assert_eq!(reps.import(PLAIN, b"t"), Some(Task("t".into())));
    assert_eq!(reps.import(HTML, b"<p>t</p>"), None, "an export-only inner type stays export-only");
}

struct Counting(Cell<u32>);
impl PasteboardItemDataProvider for Counting {
    fn provide_data(&self, uti: &str) -> Option<Vec<u8>> {
        self.0.set(self.0.get() + 1);
        (uti != "public.png").then(|| format!("made {uti}").into_bytes())
    }
}

#[test]
fn a_pasteboard_item_with_several_types_round_trips() {
    let mut item = PasteboardItem::new();
    item.set_string("hello", PLAIN);
    item.set_data(b"<b>hello</b>", HTML);
    assert_eq!(item.types(), [PLAIN, HTML]);
    let wire = item.to_item();
    let back = PasteboardItem::from_item(&wire);
    assert_eq!(back.string(PLAIN).as_deref(), Some("hello"));
    assert_eq!(back.data(HTML), Some(b"<b>hello</b>".to_vec()));
    item.set_string("again", PLAIN);
    assert_eq!(item.types(), [HTML, PLAIN], "setting a type again replaces it");
}

#[test]
fn promised_data_is_resolved_lazily_and_once() {
    let provider = Rc::new(Counting(Cell::new(0)));
    let mut item = PasteboardItem::new();
    item.set_data_provider(provider.clone(), &[HTML, "public.png"]);
    assert!(item.is_pending(HTML));
    assert_eq!(provider.0.get(), 0);
    assert_eq!(item.string(HTML).as_deref(), Some("made public.html"));
    assert_eq!(item.string(HTML).as_deref(), Some("made public.html"));
    assert_eq!(provider.0.get(), 1, "second read uses the kept answer");
    assert!(!item.is_pending(HTML));
    // a type with no data is left out of the wire item, and asked for only once
    let wire = item.to_item();
    assert_eq!(wire.representations.len(), 1);
    item.to_item();
    assert_eq!(provider.0.get(), 2);
}

#[test]
fn an_item_provider_loads_a_transferable() {
    let p = ItemProvider::with_transferable(&note("shared"));
    assert_eq!(p.registered_type_identifiers(), [JSON, PLAIN, NOTE_FILE, HTML]);
    assert!(p.has_item_conforming_to("public.text"));
    assert!(p.can_load_transferable::<Note>());
    assert_eq!(p.load_transferable::<Note>(), Ok(note("shared")));
}

#[test]
fn an_item_provider_registers_data_and_lazy_representations() {
    let mut p = ItemProvider::new();
    assert_eq!(p.load_transferable::<Note>(), Err(ItemProviderError::UnavailableCoercion));
    let lazy = Rc::new(Counting(Cell::new(0)));
    p.register_lazy_representation(PLAIN, lazy.clone());
    assert_eq!(lazy.0.get(), 0);
    assert_eq!(p.load_transferable::<Note>(), Ok(note("made public.utf8-plain-text")));
    assert_eq!(p.load_data_representation(PLAIN), Ok(b"made public.utf8-plain-text".to_vec()));
    assert_eq!(lazy.0.get(), 1);
    p.register_data_representation(JSON, b"not json".to_vec());
    assert_eq!(p.load_transferable::<Note>(), Err(ItemProviderError::UnexpectedValue), "the best type wins, and its bytes are bad");
    assert_eq!(p.load_data_representation("public.png"), Err(ItemProviderError::UnavailableCoercion));
}
