//! Parity sdk-tests-13: headless tests of the transfer model (`bifrost_app::transfer`): representation kinds, visibility,
//! proxies, pasteboard items, drags and drops, and file hand-over. No test touches the pasteboard, the shell or a
//! directory; the file tests only read paths that do not exist or that the crate itself ships.

use bifrost_app::dnd::{DragItem, Payload};
use bifrost_app::pasteboard_types::Item as PasteItem;
use bifrost_app::transfer::{
    drag_item, drop_values, from_pasteboard_item, paste_values, pasteboard_item, share_items, Kind, ReceivedTransferredFile,
    Representations, SentTransferredFile, Transferable, Viewer, Visibility, JSON,
};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

const PLAIN: &str = "public.utf8-plain-text";
const HTML: &str = "public.html";
const URL: &str = "public.url";
const PRIVATE: &str = "is.bifrost.note.private";
const DOC_FILE: &str = "is.bifrost.doc-file";

#[derive(Debug, Clone, PartialEq)]
struct Note {
    text: String,
}

#[derive(Debug, Clone, PartialEq)]
struct Wrapper(String);

#[derive(Debug, PartialEq)]
struct Doc(String);

fn note(text: &str) -> Note {
    Note { text: text.to_string() }
}

fn encode_json(n: &Note) -> Vec<u8> {
    format!("{{\"text\":\"{}\"}}", n.text).into_bytes()
}

fn decode_json(bytes: &[u8]) -> Option<Note> {
    let s = std::str::from_utf8(bytes).ok()?;
    let text = s.strip_prefix("{\"text\":\"")?.strip_suffix("\"}")?;
    Some(note(text))
}

fn export_plain(n: &Note) -> Vec<u8> {
    n.text.clone().into_bytes()
}

fn import_plain(bytes: &[u8]) -> Option<Note> {
    String::from_utf8(bytes.to_vec()).ok().map(|text| Note { text })
}

fn export_html(n: &Note) -> Vec<u8> {
    format!("<p>{}</p>", n.text).into_bytes()
}

fn export_link(n: &Note) -> Vec<u8> {
    format!("https://example.org/{}", n.text).into_bytes()
}

fn export_private(n: &Note) -> Vec<u8> {
    format!("private:{}", n.text).into_bytes()
}

/// JSON (all processes), plain text (team only), HTML (proxy), and a private proxy.
fn note_representations() -> Representations<Note> {
    Representations::new()
        .codable(JSON, Visibility::All, encode_json, decode_json)
        .data(PLAIN, Visibility::Team, export_plain, import_plain)
        .proxy(HTML, Visibility::All, export_html)
        .proxy(PRIVATE, Visibility::OwnProcess, export_private)
}

impl Transferable for Note {
    fn transfer_representations() -> Representations<Note> {
        note_representations()
    }
}

fn doc_file(d: &Doc) -> SentTransferredFile {
    SentTransferredFile::new(format!("/work/{}.txt", d.0))
}

fn doc_from_file(r: &ReceivedTransferredFile) -> Option<Doc> {
    r.file.file_stem()?.to_str().map(|s| Doc(s.to_owned()))
}

fn doc_representations() -> Representations<Doc> {
    Representations::new().file(DOC_FILE, Visibility::All, doc_file, doc_from_file)
}

impl Transferable for Doc {
    fn transfer_representations() -> Representations<Doc> {
        doc_representations()
    }
}

fn wrap_note(w: &Wrapper) -> Note {
    note(&w.0)
}

fn unwrap_note(n: Note) -> Wrapper {
    Wrapper(n.text)
}

fn wrap_doc(w: &Wrapper) -> Doc {
    Doc(w.0.clone())
}

fn unwrap_doc(d: Doc) -> Wrapper {
    Wrapper(d.0)
}

fn received(path: &str) -> ReceivedTransferredFile {
    ReceivedTransferredFile { file: PathBuf::from(path), is_original_file: false }
}

#[test]
fn headless_json_is_the_public_json_type_and_a_codable_representation_round_trips() {
    assert_eq!(JSON, "public.json");
    let reps = note_representations();
    assert_eq!(reps.kind_of(JSON), Some(Kind::Codable));
    let bytes = reps.export(&note("hi"), JSON, Viewer::Other).expect("JSON is visible to other processes");
    assert_eq!(bytes, b"{\"text\":\"hi\"}".to_vec());
    assert_eq!(reps.import(JSON, &bytes), Some(note("hi")));
    assert_eq!(reps.import(JSON, b"[1, 2]"), None, "a document that is not the encoded model is refused");
}

#[test]
fn headless_each_visibility_admits_only_the_viewers_it_names() {
    assert!(Visibility::All.is_visible_to(Viewer::Other));
    assert!(Visibility::All.is_visible_to(Viewer::Team));
    assert!(Visibility::All.is_visible_to(Viewer::OwnProcess));
    assert!(!Visibility::Team.is_visible_to(Viewer::Other));
    assert!(Visibility::Team.is_visible_to(Viewer::Team));
    assert!(Visibility::Team.is_visible_to(Viewer::OwnProcess));
    assert!(!Visibility::OwnProcess.is_visible_to(Viewer::Other));
    assert!(!Visibility::OwnProcess.is_visible_to(Viewer::Team));
    assert!(Visibility::OwnProcess.is_visible_to(Viewer::OwnProcess));
}

#[test]
fn headless_the_types_offered_to_each_viewer_keep_the_declaration_order() {
    let reps = note_representations();
    assert_eq!(reps.types_for(Viewer::Other), vec![JSON, HTML]);
    assert_eq!(reps.types_for(Viewer::Team), vec![JSON, PLAIN, HTML]);
    assert_eq!(reps.types_for(Viewer::OwnProcess), vec![JSON, PLAIN, HTML, PRIVATE]);
    assert_eq!(reps.exported_content_type(Viewer::Other), Some(JSON));
    let team_only = Representations::<Note>::new().data(PLAIN, Visibility::Team, export_plain, import_plain);
    assert_eq!(team_only.exported_content_type(Viewer::Other), None);
}

#[test]
fn headless_a_team_only_representation_is_not_exported_to_other_processes() {
    let reps = note_representations();
    assert_eq!(reps.export(&note("hi"), PLAIN, Viewer::Other), None);
    assert_eq!(reps.export(&note("hi"), PLAIN, Viewer::Team), Some(b"hi".to_vec()));
    assert_eq!(reps.export(&note("hi"), PRIVATE, Viewer::Team), None);
}

#[test]
fn headless_only_representations_that_import_are_listed_as_importable() {
    let reps = note_representations();
    assert_eq!(reps.imported_content_types(), vec![JSON, PLAIN]);
    assert_eq!(reps.kind_of(HTML), Some(Kind::Proxy));
    assert_eq!(reps.kind_of(PRIVATE), Some(Kind::Proxy));
    assert_eq!(reps.kind_of(PLAIN), Some(Kind::Data));
    assert_eq!(reps.kind_of("public.png"), None);
}

#[test]
fn headless_a_proxy_is_exported_but_never_read_back() {
    let reps = Representations::<Note>::new().proxy(HTML, Visibility::All, export_html);
    assert_eq!(reps.export(&note("x"), HTML, Viewer::Other), Some(b"<p>x</p>".to_vec()));
    assert_eq!(reps.import(HTML, b"<p>x</p>"), None);
    assert!(reps.imported_content_types().is_empty());
    assert_eq!(reps.best_import(&[HTML]), None);
}

#[test]
fn headless_best_import_follows_the_model_order_not_the_order_offered() {
    let reps = note_representations();
    assert_eq!(reps.best_import(&[HTML, PLAIN, JSON]), Some(JSON), "JSON is declared first, so it wins");
    assert_eq!(reps.best_import(&[HTML, PLAIN]), Some(PLAIN));
    assert_eq!(reps.best_import(&[HTML, PRIVATE, "public.png"]), None);
}

#[test]
fn headless_import_refuses_bytes_the_importer_cannot_read() {
    let reps = note_representations();
    assert_eq!(reps.import(PLAIN, &[0xff, 0xfe]), None);
    assert_eq!(reps.import(JSON, b"{\"text\":\"unterminated"), None);
    assert_eq!(reps.import("public.png", b"abc"), None);
}

#[test]
fn headless_a_pasteboard_item_holds_only_the_types_another_process_may_see() {
    let item = pasteboard_item(&note_representations(), &note("hi"));
    let utis: Vec<&str> = item.representations.iter().map(|r| r.uti.as_str()).collect();
    assert_eq!(utis, vec![JSON, HTML]);
    assert_eq!(item.data(JSON), Some(&b"{\"text\":\"hi\"}"[..]));
    assert_eq!(item.data(HTML), Some(&b"<p>hi</p>"[..]));
    assert_eq!(item.data(PLAIN), None, "team-only bytes stay inside the team");
    assert_eq!(item.data(PRIVATE), None, "own-process bytes never leave the process");
}

#[test]
fn headless_a_pasteboard_item_rebuilds_the_model_from_the_best_readable_type() {
    let reps = note_representations();
    let both = PasteItem::default().with(PLAIN, b"plain").with(JSON, b"{\"text\":\"json\"}");
    assert_eq!(from_pasteboard_item(&reps, &both), Some(note("json")));
    let html_and_plain = PasteItem::default().with(HTML, b"<p>hi</p>").with(PLAIN, b"hi");
    assert_eq!(from_pasteboard_item(&reps, &html_and_plain), Some(note("hi")));
    let proxy_only = PasteItem::default().with(HTML, b"<p>hi</p>");
    assert_eq!(from_pasteboard_item(&reps, &proxy_only), None);
    let bad = PasteItem::default().with(JSON, b"not json");
    assert_eq!(from_pasteboard_item(&reps, &bad), None);
}

#[test]
fn headless_paste_values_read_each_item_the_model_can_import() {
    let reps = Representations::<Note>::new().data(PLAIN, Visibility::All, export_plain, import_plain);
    let items = [
        PasteItem::default().with(PLAIN, b"a"),
        PasteItem::default().with(HTML, b"<p>b</p>"),
        PasteItem::default().with(PLAIN, b"c"),
    ];
    let texts: Vec<String> = paste_values(&reps, &items).into_iter().map(|n| n.text).collect();
    assert_eq!(texts, ["a", "c"]);
}

#[test]
fn headless_a_drag_item_needs_a_text_or_link_representation_visible_to_other_processes() {
    assert!(drag_item(&note_representations(), &note("hi")).is_none(), "the only text is team-only");
    let open = Representations::<Note>::new()
        .data(PLAIN, Visibility::All, export_plain, import_plain)
        .proxy(HTML, Visibility::All, export_html);
    let item = drag_item(&open, &note("hi")).expect("plain text is visible");
    assert_eq!(item.types, vec![PLAIN.to_string(), HTML.to_string()]);
    assert_eq!(item.payload, Payload::Text("hi".into()));
}

#[test]
fn headless_a_link_representation_becomes_a_link_payload_for_drag_and_share() {
    let reps = Representations::<Note>::new().proxy(URL, Visibility::All, export_link);
    let item = drag_item(&reps, &note("hi")).expect("the link is visible");
    assert_eq!(item.payload, Payload::Url("https://example.org/hi".into()));
    assert_eq!(share_items(&reps, &note("hi")), vec![Payload::Url("https://example.org/hi".into())]);
}

#[test]
fn headless_share_items_are_empty_without_a_text_or_link_representation() {
    let reps = Representations::<Note>::new().proxy(HTML, Visibility::All, export_html);
    assert!(share_items(&reps, &note("hi")).is_empty());
    let open = Representations::<Note>::new().data(PLAIN, Visibility::All, export_plain, import_plain);
    assert_eq!(share_items(&open, &note("hi")), vec![Payload::Text("hi".into())]);
}

#[test]
fn headless_a_drop_rebuilds_text_items_and_skips_links_and_files() {
    let reps = Representations::<Note>::new().data(PLAIN, Visibility::All, export_plain, import_plain);
    let items = vec![
        DragItem::new(Payload::Text("hi".into())),
        DragItem::new(Payload::Url("https://example.org".into())),
        DragItem::new(Payload::File { grant: [1; 16], name: "a.txt".into() }),
    ];
    assert_eq!(drop_values(&reps, &items), vec![note("hi")]);
}

#[test]
fn headless_a_file_representation_is_declared_as_a_file_and_hands_over_its_path() {
    let reps = doc_representations();
    assert_eq!(reps.kind_of(DOC_FILE), Some(Kind::File));
    assert_eq!(reps.imported_content_types(), vec![DOC_FILE]);
    let sent = reps.export_file(&Doc("memo".into()), DOC_FILE, Viewer::Other).expect("the file is handed over");
    assert_eq!(sent.file, PathBuf::from("/work/memo.txt"));
    assert!(!sent.allow_open_in_place);
    assert_eq!(reps.import_file(DOC_FILE, &received("/in/report.txt")), Some(Doc("report".into())));
}

#[test]
fn headless_a_file_representation_does_not_travel_as_bytes_it_does_not_have() {
    let reps = doc_representations();
    assert_eq!(reps.import(DOC_FILE, b"memo"), None, "a file is rebuilt from a file, not from bytes");
    assert_eq!(reps.export_file(&Doc("memo".into()), DOC_FILE, Viewer::Other).map(|s| s.file), Some(PathBuf::from("/work/memo.txt")));
    assert_eq!(reps.export(&Doc("memo".into()), DOC_FILE, Viewer::Other), None, "the file at /work/memo.txt does not exist here");
    let bytes_only = note_representations();
    assert!(bytes_only.export_file(&note("x"), PLAIN, Viewer::Team).is_none());
}

#[test]
fn headless_a_sent_file_opened_in_place_hands_over_the_original() {
    let sent = SentTransferredFile::new("/work/a.txt").allow_open_in_place(true);
    let got = sent.receive(Path::new("/unused")).expect("no copy is made");
    assert_eq!(got, ReceivedTransferredFile { file: PathBuf::from("/work/a.txt"), is_original_file: true });
}

#[test]
fn headless_a_sent_file_without_open_in_place_is_copied_and_a_missing_copy_source_fails() {
    let sent = SentTransferredFile::new("/definitely/missing/sdk-tests-13.txt");
    assert!(!sent.allow_open_in_place);
    assert!(sent.receive(Path::new("/definitely/missing")).is_err(), "there is nothing to copy");
    let nameless = SentTransferredFile::new("/");
    let err = nameless.receive(Path::new("/unused")).expect_err("a path without a file name is refused");
    assert_eq!(err.kind(), ErrorKind::InvalidInput);
}

#[test]
fn headless_a_proxy_of_another_model_forwards_its_bytes_and_reads_them_back() {
    let reps = Representations::<Wrapper>::new().proxy_of::<Note>(Visibility::All, wrap_note, Some(unwrap_note));
    assert_eq!(reps.kind_of(JSON), Some(Kind::Proxy));
    assert_eq!(reps.export(&Wrapper("hi".into()), JSON, Viewer::Other), Some(b"{\"text\":\"hi\"}".to_vec()));
    assert_eq!(reps.imported_content_types(), vec![JSON, PLAIN]);
    assert_eq!(reps.import(PLAIN, b"hi"), Some(Wrapper("hi".into())));
}

#[test]
fn headless_a_proxy_never_widens_the_visibility_of_what_it_forwards() {
    let reps = Representations::<Wrapper>::new().proxy_of::<Note>(Visibility::All, wrap_note, Some(unwrap_note));
    assert_eq!(reps.types_for(Viewer::Other), vec![JSON, HTML]);
    assert_eq!(reps.types_for(Viewer::Team), vec![JSON, PLAIN, HTML]);
    assert_eq!(reps.export(&Wrapper("hi".into()), PLAIN, Viewer::Other), None, "team-only bytes stay team-only");
    assert_eq!(reps.export(&Wrapper("hi".into()), PRIVATE, Viewer::Team), None);
    assert_eq!(reps.export(&Wrapper("hi".into()), PRIVATE, Viewer::OwnProcess), Some(b"private:hi".to_vec()));
}

#[test]
fn headless_a_team_proxy_of_a_file_is_hidden_from_other_processes_and_reads_back() {
    let reps = Representations::<Wrapper>::new().proxy_of::<Doc>(Visibility::Team, wrap_doc, Some(unwrap_doc));
    assert!(reps.types_for(Viewer::Other).is_empty(), "a team-only proxy is hidden from other processes");
    assert_eq!(reps.types_for(Viewer::Team), vec![DOC_FILE]);
    assert_eq!(reps.kind_of(DOC_FILE), Some(Kind::Proxy));
    assert_eq!(reps.export_file(&Wrapper("report".into()), DOC_FILE, Viewer::Other).map(|s| s.file), None);
    let sent = reps.export_file(&Wrapper("report".into()), DOC_FILE, Viewer::Team).expect("forwarded file");
    assert_eq!(sent.file, PathBuf::from("/work/report.txt"));
    assert_eq!(reps.import_file(DOC_FILE, &received("/in/memo.txt")), Some(Wrapper("memo".into())));
}

#[test]
fn headless_an_export_only_proxy_of_a_file_cannot_be_read_back() {
    let reps = Representations::<Wrapper>::new().proxy_of::<Doc>(Visibility::All, wrap_doc, None);
    assert_eq!(reps.types_for(Viewer::Other), vec![DOC_FILE]);
    assert!(reps.imported_content_types().is_empty());
    assert_eq!(reps.import_file(DOC_FILE, &received("/in/memo.txt")), None);
}

#[test]
fn headless_a_model_declares_its_representations_through_the_transferable_trait() {
    let reps = Note::transfer_representations();
    assert_eq!(reps.types_for(Viewer::Other), vec![JSON, HTML]);
    assert_eq!(Doc::transfer_representations().kind_of(DOC_FILE), Some(Kind::File));
}
