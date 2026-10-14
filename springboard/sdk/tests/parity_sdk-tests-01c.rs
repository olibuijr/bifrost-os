//! Quick Look preview models and system thumbnail answers through the public API of `bifrost_app`. Headless: the panel
//! and view are pure state, and the thumbnail answers are decoded from JSON bodies; nothing is sent to a shell.

use bifrost_app::peek::{ItemState, PreviewDataSource, PreviewItem, PreviewPanel, PreviewView, ViewStyle, MAX_ITEMS};
use bifrost_app::share;
use bifrost_app::thumbnails::{self, State, FAILED_EVENT, READY_EVENT, SERVICE};
use bifrost_core::json::Json;

fn item(title: &str) -> PreviewItem {
    PreviewItem { url: format!("file:///{title}.txt"), title: title.into() }
}

/// A data source that makes its items on demand, like an app with a large folder.
struct Generated(usize);

impl PreviewDataSource for Generated {
    fn number_of_items(&self) -> usize {
        self.0
    }

    fn item_at(&self, index: usize) -> Option<PreviewItem> {
        (index < self.0).then(|| item(&format!("g{index}")))
    }
}

fn obj(pairs: &[(&str, Json)]) -> Json {
    Json::Obj(pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect())
}

fn text(v: &str) -> Json {
    Json::Str(v.into())
}

#[test]
fn the_panel_steps_without_wrapping_and_the_index_stays_in_range() {
    let mut p = PreviewPanel::new();
    p.set_items(vec![item("a"), item("b"), item("c")]);
    p.set_current_preview_item_index(9);
    assert_eq!(p.current_item().unwrap().title, "c", "an index past the end clamps to the last item");
    p.step(1);
    assert_eq!(p.current_preview_item_index(), 2, "no step past the last item");
    p.step(-5);
    assert_eq!(p.current_preview_item_index(), 0, "no step before the first item");
    p.set_items(vec![item("x")]);
    assert_eq!(p.current_preview_item_index(), 0);
    p.set_items(Vec::new());
    assert!(p.current_item().is_none());
    p.step(3);
    assert_eq!(p.current_preview_item_index(), 0, "an empty panel has nothing to step to");
}

#[test]
fn a_data_source_pull_keeps_its_order_and_is_capped_at_the_wire_limit() {
    let mut p = PreviewPanel::new();
    p.load_from(&Generated(3));
    let titles: Vec<&str> = p.items().iter().map(|i| i.title.as_str()).collect();
    assert_eq!(titles, ["g0", "g1", "g2"]);
    p.load_from(&Generated(MAX_ITEMS + 9));
    assert_eq!(p.items().len(), MAX_ITEMS);
    assert_eq!(p.items().last().unwrap().title, format!("g{}", MAX_ITEMS - 1));
    p.load_from(&vec![item("v")]);
    assert_eq!(p.items().len(), 1, "a Vec is a data source too");
}

#[test]
fn a_pull_clamps_the_current_index_into_the_new_list() {
    let mut p = PreviewPanel::new();
    p.load_from(&Generated(5));
    p.set_current_preview_item_index(4);
    p.load_from(&Generated(2));
    assert_eq!(p.current_preview_item_index(), 1);
    assert_eq!(p.current_item().unwrap().title, "g1");
}

#[test]
fn each_item_starts_ready_and_a_new_list_resets_the_states() {
    let mut p = PreviewPanel::new();
    p.set_items(vec![item("a"), item("b")]);
    assert_eq!(p.display_state(0), Some(ItemState::Ready));
    assert!(p.set_display_state(1, ItemState::Failed));
    assert_eq!(p.display_state(1), Some(ItemState::Failed));
    assert_eq!(p.display_state(0), Some(ItemState::Ready), "the other item keeps its state");
    p.set_items(vec![item("a"), item("b")]);
    assert_eq!(p.display_state(1), Some(ItemState::Ready));
}

#[test]
fn a_display_state_past_the_end_is_refused() {
    let mut p = PreviewPanel::new();
    p.set_items(vec![item("a")]);
    assert!(!p.set_display_state(3, ItemState::Loading));
    assert_eq!(p.display_state(3), None);
}

#[test]
fn a_hidden_panel_has_no_visible_state_until_it_is_shown() {
    let p = PreviewPanel::new();
    assert!(!p.is_visible());
    assert!(p.items().is_empty());
}

#[test]
fn an_embedded_view_loads_a_new_item_until_the_app_reports_ready() {
    let mut v = PreviewView::new(ViewStyle::Compact);
    assert_eq!(v.style(), ViewStyle::Compact);
    assert!(v.autostarts() && v.closes_with_window());
    v.set_preview_item(Some(item("notes")));
    assert_eq!(v.display_state(), ItemState::Loading);
    assert_eq!(v.preview_item().map(|i| i.title.as_str()), Some("notes"));
    v.set_display_state(ItemState::Ready);
    assert_eq!(v.display_state(), ItemState::Ready);
}

#[test]
fn without_autostart_an_embedded_item_waits_at_ready() {
    let mut v = PreviewView::new(ViewStyle::Normal);
    v.set_autostarts(false);
    v.set_preview_item(Some(item("photo")));
    assert_eq!(v.display_state(), ItemState::Ready);
}

#[test]
fn a_refresh_needs_an_item_and_is_counted() {
    let mut v = PreviewView::new(ViewStyle::Normal);
    assert!(!v.refresh_preview_item());
    v.set_preview_item(Some(item("a")));
    assert!(v.refresh_preview_item());
    assert!(v.refresh_preview_item());
    assert_eq!(v.refresh_count(), 2);
}

#[test]
fn closing_a_view_returns_its_item_and_resets_the_state() {
    let mut v = PreviewView::new(ViewStyle::Normal);
    v.set_preview_item(Some(item("doc")));
    let gone = v.close().unwrap();
    assert_eq!(gone.title, "doc");
    assert!(v.preview_item().is_none());
    assert_eq!(v.display_state(), ItemState::Ready);
    assert!(v.close().is_none());
}

#[test]
fn a_window_close_closes_the_view_only_when_the_switch_is_on() {
    let mut v = PreviewView::new(ViewStyle::Normal);
    v.set_closes_with_window(false);
    v.set_preview_item(Some(item("kept")));
    assert!(!v.window_closed());
    assert!(v.preview_item().is_some());
    v.set_closes_with_window(true);
    assert!(v.window_closed());
    assert!(v.preview_item().is_none());
}

#[test]
fn a_picture_answer_names_the_cached_image_and_its_size() {
    let body = obj(&[
        ("state", text("ready")),
        ("file", text("/a/b.png")),
        ("px", Json::Num(128.0)),
        ("art", text("pixels")),
        ("image", text("/c/abc.raw")),
        ("w", Json::Num(64.0)),
        ("h", Json::Num(32.0)),
    ]);
    let reply = thumbnails::reply(&body).expect("a thumbnail answer");
    assert_eq!((reply.file.as_str(), reply.px), ("/a/b.png", 128));
    assert_eq!(reply.state, State::Pixels { image: "/c/abc.raw".into(), w: 64, h: 32 });
}

#[test]
fn an_icon_answer_keeps_the_type_and_the_badge_and_pending_has_neither() {
    let icon = obj(&[
        ("state", text("ready")),
        ("file", text("/a/b.docx")),
        ("px", Json::Num(64.0)),
        ("art", text("icon")),
        ("type", text("org.openxmlformats.wordprocessingml.document")),
        ("badge", text("DOCX")),
    ]);
    assert_eq!(
        thumbnails::reply(&icon).unwrap().state,
        State::Icon { type_id: "org.openxmlformats.wordprocessingml.document".into(), badge: Some("DOCX".into()) }
    );
    let pending = obj(&[("state", text("pending")), ("file", text("/a/b.pdf")), ("px", Json::Num(96.0))]);
    assert_eq!(thumbnails::reply(&pending).unwrap().state, State::Pending);
}

#[test]
fn a_failed_answer_carries_the_reason() {
    let failed = obj(&[("state", text("failed")), ("file", text("/a/b.pdf")), ("px", Json::Num(96.0)), ("error", text("gone"))]);
    assert_eq!(thumbnails::reply(&failed).unwrap().state, State::Failed("gone".into()));
}

#[test]
fn other_bodies_are_not_thumbnail_answers() {
    assert!(thumbnails::reply(&obj(&[("state", text("unknown")), ("file", text("/a")), ("px", Json::Num(1.0))])).is_none());
    assert!(thumbnails::reply(&text("ok")).is_none());
    assert!(thumbnails::reply(&obj(&[("state", text("pending")), ("px", Json::Num(1.0))])).is_none(), "a file is required");
}

#[test]
fn only_the_thumbnail_events_of_the_thumbnail_service_are_read() {
    let body = obj(&[("state", text("failed")), ("file", text("/a")), ("px", Json::Num(8.0)), ("error", text("x"))]);
    assert_eq!(SERVICE, "thumbnails");
    assert!(thumbnails::event(SERVICE, FAILED_EVENT, &body).is_some());
    assert!(thumbnails::event(SERVICE, READY_EVENT, &body).is_some());
    assert!(thumbnails::event(SERVICE, "closed", &body).is_none());
    assert!(thumbnails::event("quick-look", READY_EVENT, &body).is_none());
}

#[test]
fn a_shared_text_arrives_as_a_data_url_that_decodes_to_the_text() {
    assert_eq!(share::text_from_url("data:text/plain;charset=utf-8,hello%20world%21"), Some("hello world!".into()));
    assert_eq!(share::text_from_url("https://example.com/"), None, "a web address is a URL, not shared text");
    assert_eq!(share::text_from_url("data:text/plain;charset=utf-8,bad%zz"), None, "an escape that is not hex is refused");
}

#[test]
fn a_document_request_with_the_inbound_bit_is_one_someone_shared_with_the_app() {
    assert!(share::is_inbound(share::INBOUND | 7));
    assert!(!share::is_inbound(7), "a request the app made itself is not inbound");
    assert_eq!(share::COPY, "copy");
}
