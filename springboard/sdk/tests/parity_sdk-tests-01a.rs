//! Parity sdk-tests-01a: headless tests for the SDK models behind the QuickLook, keychain, Spotlight, workspace and
//! thumbnail APIs. Each test runs the pure part of a module through its public API and asserts concrete values. None
//! opens a socket, reads the clock or needs a display.

use bifrost_app::keychain::{Error as KeychainError, Reply as KeychainReply};
use bifrost_app::keychain_query::{decode, decode_item, encode, matches, select, wire_key, Attributes, Item, ItemClass, MatchLimit, Query, Status};
use bifrost_app::peek::{ItemState, PreviewDataSource, PreviewItem, PreviewPanel, PreviewView, ViewStyle, MAX_ITEMS};
use bifrost_app::proto::WorkspaceEventKind;
use bifrost_app::quicksearch::{search_items, CSSearchableItem, CSSearchableItemAttributeSet, IndexError, SearchQuery};
use bifrost_app::thumbnails::{event, reply, State, FAILED_EVENT, READY_EVENT, SERVICE};
use bifrost_app::workspace::{icon_for_file, notification_name, DID_LAUNCH_APPLICATION};
use bifrost_core::json::Json;

fn item(n: &str) -> PreviewItem {
    PreviewItem { url: format!("file:///{n}"), title: n.into() }
}

/// A data source that makes `n` items on demand, as an app's `QLPreviewPanelDataSource` would.
struct Generated {
    n: usize,
}

impl PreviewDataSource for Generated {
    fn number_of_items(&self) -> usize {
        self.n
    }

    fn item_at(&self, index: usize) -> Option<PreviewItem> {
        (index < self.n).then(|| item(&format!("doc{index}")))
    }
}

#[test]
fn panel_pulls_a_data_source_and_stops_at_the_item_cap() {
    let mut panel = PreviewPanel::new();
    panel.load_from(&Generated { n: 40 });
    assert_eq!(panel.items().len(), MAX_ITEMS);
    assert_eq!(panel.items()[0].title, "doc0");
    assert_eq!(panel.items()[MAX_ITEMS - 1].title, format!("doc{}", MAX_ITEMS - 1));
    assert_eq!(panel.display_state(0), Some(ItemState::Ready));
}

#[test]
fn panel_steps_through_the_list_without_wrapping() {
    let mut panel = PreviewPanel::new();
    panel.set_items(vec![item("a"), item("b"), item("c")]);
    panel.step(1);
    panel.step(1);
    panel.step(1);
    assert_eq!(panel.current_preview_item_index(), 2, "the last item stays current");
    panel.step(-1);
    assert_eq!(panel.current_item().map(|i| i.title.as_str()), Some("b"));
    panel.step(-9);
    assert_eq!(panel.current_preview_item_index(), 0, "the first item stays current");
    assert!(!panel.is_visible(), "stepping alone does not show the panel");
}

#[test]
fn panel_display_state_is_kept_per_item_and_reset_by_a_new_list() {
    let mut panel = PreviewPanel::new();
    panel.set_items(vec![item("a"), item("b")]);
    assert!(panel.set_display_state(1, ItemState::Failed));
    assert_eq!(panel.display_state(1), Some(ItemState::Failed));
    assert_eq!(panel.display_state(0), Some(ItemState::Ready));
    assert!(!panel.set_display_state(2, ItemState::Loading), "no item at index 2");
    assert_eq!(panel.display_state(2), None);
    panel.set_items(vec![item("c")]);
    assert_eq!(panel.display_state(0), Some(ItemState::Ready));
    assert_eq!(panel.display_state(1), None);
}

#[test]
fn panel_index_follows_the_list_when_it_shrinks() {
    let mut panel = PreviewPanel::new();
    panel.set_items(vec![item("a"), item("b"), item("c")]);
    panel.set_current_preview_item_index(2);
    panel.set_items(vec![item("x")]);
    assert_eq!(panel.current_preview_item_index(), 0);
    assert_eq!(panel.current_item().map(|i| i.url.as_str()), Some("file:///x"));
}

#[test]
fn embedded_view_loads_an_autostarting_item_until_the_app_reports_ready() {
    let mut view = PreviewView::new(ViewStyle::Compact);
    assert_eq!(view.style(), ViewStyle::Compact);
    view.set_preview_item(Some(item("photo")));
    assert_eq!(view.display_state(), ItemState::Loading);
    view.set_display_state(ItemState::Ready);
    assert_eq!(view.display_state(), ItemState::Ready);
    assert_eq!(view.preview_item().map(|i| i.title.as_str()), Some("photo"));
}

#[test]
fn embedded_view_without_autostart_waits_at_ready() {
    let mut view = PreviewView::new(ViewStyle::Normal);
    view.set_autostarts(false);
    view.set_preview_item(Some(item("clip")));
    assert_eq!(view.display_state(), ItemState::Ready);
    assert_eq!(view.refresh_count(), 0);
    assert!(view.refresh_preview_item());
    assert_eq!(view.refresh_count(), 1);
    assert_eq!(view.close().map(|i| i.title), Some("clip".to_string()));
    assert!(!view.refresh_preview_item(), "nothing to refresh after close");
}

#[test]
fn embedded_view_closes_with_its_window_only_when_asked() {
    let mut kept = PreviewView::new(ViewStyle::Normal);
    kept.set_closes_with_window(false);
    kept.set_preview_item(Some(item("keep")));
    assert!(!kept.window_closed());
    assert!(kept.preview_item().is_some());
    let mut gone = PreviewView::new(ViewStyle::Normal);
    gone.set_preview_item(Some(item("drop")));
    assert!(gone.window_closed());
    assert!(gone.preview_item().is_none());
}

#[test]
fn a_thumbnail_answer_names_its_cached_picture() {
    let body = Json::Obj(vec![
        ("state".into(), Json::Str("ready".into())),
        ("file".into(), Json::Str("/home/me/a.png".into())),
        ("px".into(), Json::Num(128.0)),
        ("art".into(), Json::Str("pixels".into())),
        ("image".into(), Json::Str("/cache/abc.raw".into())),
        ("w".into(), Json::Num(64.0)),
        ("h".into(), Json::Num(48.0)),
    ]);
    let answer = reply(&body).expect("a thumbnail answer");
    assert_eq!(answer.px, 128);
    assert_eq!(answer.state, State::Pixels { image: "/cache/abc.raw".into(), w: 64, h: 48 });
}

#[test]
fn a_thumbnail_event_is_read_only_from_its_own_service() {
    let body = Json::Obj(vec![
        ("state".into(), Json::Str("failed".into())),
        ("file".into(), Json::Str("/a/b.pdf".into())),
        ("px".into(), Json::Num(96.0)),
        ("error".into(), Json::Str("unreadable".into())),
    ]);
    let answer = event(SERVICE, FAILED_EVENT, &body).expect("failed event");
    assert_eq!(answer.state, State::Failed("unreadable".into()));
    assert!(event(SERVICE, READY_EVENT, &body).is_some());
    assert!(event("thumbnails-other", READY_EVENT, &body).is_none());
}

fn generic(service: &str, account: &str, data: &[u8]) -> Item {
    Item {
        class: ItemClass::GenericPassword,
        attributes: Attributes { service: Some(service.into()), account: Some(account.into()), ..Attributes::default() },
        data: data.to_vec(),
    }
}

#[test]
fn generic_password_wire_key_is_service_and_account() {
    let a = Attributes { service: Some("mail".into()), account: Some("ada".into()), ..Attributes::default() };
    assert_eq!(wire_key(ItemClass::GenericPassword, &a), Some(("mail".into(), "ada".into())));
    let no_account = Attributes { service: Some("mail".into()), ..Attributes::default() };
    assert_eq!(wire_key(ItemClass::GenericPassword, &no_account), None);
}

#[test]
fn internet_password_wire_key_carries_server_port_and_protocol() {
    let a = Attributes {
        server: Some("example.com".into()),
        port: Some(443),
        protocol: Some("https".into()),
        account: Some("ada".into()),
        ..Attributes::default()
    };
    assert_eq!(wire_key(ItemClass::InternetPassword, &a), Some(("inet:example.com:443:https".into(), "ada".into())));
}

#[test]
fn an_item_round_trips_through_its_stored_record() {
    let stored_item = Item {
        class: ItemClass::InternetPassword,
        attributes: Attributes {
            service: None,
            account: Some("ada".into()),
            server: Some("example.com".into()),
            protocol: Some("https".into()),
            port: Some(8443),
            label: Some("Work".into()),
            comment: None,
            creation_date: Some(1_700_000_000),
            modification_date: Some(1_700_000_500),
            access_group: Some("group.bifrost".into()),
        },
        data: b"hunter2".to_vec(),
    };
    let stored = encode(&stored_item);
    assert!(stored.starts_with(b"SIQ1"));
    assert_eq!(decode(&stored), Ok(stored_item));
}

#[test]
fn a_legacy_value_decodes_as_a_generic_password_with_only_its_secret() {
    let legacy = decode(b"plain-secret").unwrap();
    assert_eq!(legacy.class, ItemClass::GenericPassword);
    assert_eq!(legacy.attributes, Attributes::default());
    assert_eq!(legacy.data, b"plain-secret".to_vec());
}

#[test]
fn a_damaged_record_is_a_decode_status() {
    assert_eq!(decode(b"SIQ1\x00"), Err(Status::Decode), "a record cut short after its class byte");
    assert_eq!(decode(b"SIQ1\x00\x09"), Err(Status::Decode), "a presence byte other than 0 or 1 is refused");
}

#[test]
fn decode_item_maps_a_failed_get_to_its_status() {
    let ok = KeychainReply { request: 1, result: Ok(encode(&generic("mail", "ada", b"s3cret"))) };
    assert_eq!(decode_item(&ok).map(|i| i.data), Ok(b"s3cret".to_vec()));
    let missing = KeychainReply { request: 2, result: Err(KeychainError::ItemNotFound) };
    assert_eq!(decode_item(&missing), Err(Status::ItemNotFound));
    let denied = KeychainReply { request: 3, result: Err(KeychainError::NotPermitted) };
    assert_eq!(decode_item(&denied), Err(Status::NotAvailable));
}

#[test]
fn osstatus_codes_match_the_security_framework_values() {
    assert_eq!(Status::Success.code(), 0);
    assert_eq!(Status::DuplicateItem.code(), -25299);
    assert_eq!(Status::ItemNotFound.code(), -25300);
    assert_eq!(Status::NotAvailable.code(), -25291);
    assert_eq!(Status::Param.code(), -50);
    assert_eq!(Status::Io.code(), -36);
    assert_eq!(Status::from_error(&KeychainError::Io("disk".into())), Status::Io);
    assert_eq!(KeychainError::DuplicateItem.code(), Status::DuplicateItem.code());
}

#[test]
fn a_query_matches_only_the_attributes_it_sets_and_the_same_class() {
    let stored = generic("mail", "ada", b"x");
    let mut by_service = Query::new(ItemClass::GenericPassword);
    by_service.attributes.service = Some("mail".into());
    assert!(matches(&by_service, &stored));
    by_service.attributes.service = Some("web".into());
    assert!(!matches(&by_service, &stored));
    let wrong_class = Query::new(ItemClass::InternetPassword);
    assert!(!matches(&wrong_class, &stored));
}

#[test]
fn match_limit_one_takes_the_first_hit_and_all_takes_every_hit() {
    let items = vec![generic("mail", "ada", b"1"), generic("mail", "grace", b"2"), generic("mail", "linus", b"3")];
    let mut q = Query::new(ItemClass::GenericPassword);
    q.attributes.service = Some("mail".into());
    q.return_data = true;
    let one = select(&q, &items);
    assert_eq!(one.len(), 1);
    assert_eq!(one[0].data, b"1".to_vec());
    q.limit = MatchLimit::All;
    assert_eq!(select(&q, &items).len(), 3);
}

#[test]
fn select_drops_the_secret_and_the_attributes_the_query_did_not_ask_for() {
    let items = vec![generic("mail", "ada", b"s3cret")];
    let mut q = Query::new(ItemClass::GenericPassword);
    q.limit = MatchLimit::All;
    let hidden = select(&q, &items);
    assert!(hidden[0].data.is_empty(), "no secret without return_data");
    assert_eq!(hidden[0].attributes, Attributes::default(), "no attributes without return_attributes");
    q.return_data = true;
    q.return_attributes = true;
    let shown = select(&q, &items);
    assert_eq!(shown[0].data, b"s3cret".to_vec());
    assert_eq!(shown[0].attributes.account.as_deref(), Some("ada"));
}

fn note(id: &str, domain: &str, title: &str) -> CSSearchableItem {
    CSSearchableItem::new(id, domain, CSSearchableItemAttributeSet::new(title))
}

#[test]
fn an_indexed_item_needs_an_identifier_within_its_limits() {
    assert_eq!(IndexError::check(&note("", "notes", "Empty")), Err(IndexError::EmptyIdentifier));
    assert_eq!(IndexError::check(&note(&"x".repeat(129), "notes", "Long")), Err(IndexError::IdentifierTooLong));
    assert_eq!(IndexError::check(&note(&"x".repeat(128), "notes", "Exact")), Ok(()));
    assert_eq!(IndexError::check(&note("n1", &"d".repeat(65), "Domain")), Err(IndexError::DomainTooLong));
    assert_eq!(IndexError::DomainTooLong.code(), 3);
    assert_eq!(IndexError::DOMAIN, "bifrost.quicksearch.index");
}

#[test]
fn a_search_over_the_app_items_finds_the_matching_title() {
    let items = vec![note("n1", "notes", "Garden notes"), note("n2", "notes", "Budget 2026")];
    let hits = search_items(&SearchQuery::new("garden"), &items).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].unique_identifier, "n1");
}

#[test]
fn workspace_events_map_to_their_notification_names() {
    assert_eq!(notification_name(WorkspaceEventKind::DidLaunchApplication), DID_LAUNCH_APPLICATION);
    assert_eq!(DID_LAUNCH_APPLICATION, "workspace.didLaunchApplication");
    assert_eq!(notification_name(WorkspaceEventKind::ScreensDidWake), "workspace.screensDidWake");
}

#[test]
fn file_icons_follow_the_type_of_the_extension() {
    assert_eq!(icon_for_file("photo.jpg"), "photo");
    assert_eq!(icon_for_file("paper.pdf"), "doc.richtext");
    assert_eq!(icon_for_file("notes.unknownext"), "doc");
    assert_eq!(icon_for_file("no-extension"), "doc");
    assert_eq!(FAILED_EVENT, "failed");
}
