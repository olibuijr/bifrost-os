//! Headless tests for the SDK models behind the parity rows: local search indexing, the index ledger,
//! notifications, menu-bar extras, local authentication, sharing, the preview panel, thumbnail requests, the notify bus
//! and the keychain. Everything runs on the host with an in-memory `Ctx` (no shell, no display, no network) and
//! asserts on the messages the SDK queues for the shell and on the values the models return.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use bifrost_app::intents::Intent;
use bifrost_app::local_auth::{decode_reply, BiometryType, Device, LAContext, LAError, LAPolicy};
use bifrost_app::notification::{DistributedNotificationCenter, NotificationCenter};
use bifrost_app::notify_bus::{self, Mode, Notified};
use bifrost_app::peek::{ItemState, PreviewDataSource, PreviewItem, PreviewPanel, MAX_ITEMS};
use bifrost_app::proto::quicksearch::{DELETE_ALL, DELETE_DOMAINS, DELETE_IDS, MAX_DOMAIN, MAX_ID};
use bifrost_app::proto::{Configure, KeychainOp, Payload, ToShell};
use bifrost_app::quicksearch::{
    CSCustomAttributeKey, CSSearchableIndex, CSSearchableIndexDelegate, CSSearchableItem, CSSearchableItemAttributeSet,
    IndexError, LocalizedString, Person, SearchQuery, UserActivity, ACTION_TYPE,
};
use bifrost_app::search_index_batch::{
    CSIndexError, CSIndexErrorCode, IndexOp, SearchIndexLedger, MAX_ATTRIBUTE_BYTES, MAX_CLIENT_STATE_BYTES, MAX_INDEXED_ITEMS,
};
use bifrost_app::share::{text_from_url, Candidate, SharingService, COMPOSE, COPY, INBOUND};
use bifrost_app::status_item::{MenuItem, StatusBar, StatusItemLength};
use bifrost_app::thumbnails::{self, edge_px, RequestError, State, ThumbKind, ThumbRequest, MIN_PX};
use bifrost_app::Ctx;
use bifrost_core::json::Json;
use bifrost_render::font::Font;
use bifrost_render::ui::Ui;

/// An in-memory context: what the app sends to the shell is collected by `take_sent`.
fn ctx() -> Ctx {
    let data: &'static [u8] = Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/SFUI.ttf")).unwrap().into_boxed_slice());
    let cfg = Configure { w: 100, h: 100, scale: 1.0, safe_top: 0.0, safe_bottom: 0.0, dark: true, text_scale: 1.0, reduce_motion: false };
    Ctx::testing(Ui::new(Font::parse(data).unwrap(), 1.0), cfg)
}

fn ids(items: &[&CSSearchableItem]) -> Vec<String> {
    items.iter().map(|i| i.unique_identifier.clone()).collect()
}

fn found(q: &str, items: &[CSSearchableItem]) -> Vec<String> {
    ids(&search_items_or_panic(q, items))
}

fn search_items_or_panic<'a>(q: &str, items: &'a [CSSearchableItem]) -> Vec<&'a CSSearchableItem> {
    bifrost_app::quicksearch::search_items(&SearchQuery::new(q), items).expect("a non-empty query runs")
}

// --- Local search: searchable items and attribute sets ----------------------------------------------------------------

#[test]
fn attribute_set_builder_records_each_attribute_it_is_given() {
    let set = CSSearchableItemAttributeSet::new("Report")
        .display_name("Q3 Report")
        .content_description("Quarterly figures")
        .keywords(&["finance"])
        .thumbnail_path("/home/u/.local/share/app/t.png")
        .content_url("file:///home/u/report.txt")
        .content_type("public.text")
        .content_creation_date(1_700_000_000)
        .content_modification_date(0)
        .author_names(&["Ada Lovelace"])
        .ranking_hint(5);
    assert_eq!(set.title, "Report");
    assert_eq!(set.display_name, "Q3 Report");
    assert_eq!(set.content_description, "Quarterly figures");
    assert_eq!(set.keywords, ["finance"]);
    assert_eq!(set.thumbnail_path, "/home/u/.local/share/app/t.png");
    assert_eq!(set.content_url, "file:///home/u/report.txt");
    assert_eq!(set.content_type, "public.text");
    assert_eq!(set.content_creation_date, Some(1_700_000_000));
    assert_eq!(set.content_modification_date, Some(0));
    assert_eq!(set.author_names, ["Ada Lovelace"]);
    assert_eq!(set.ranking_hint, Some(5));
}

#[test]
fn dates_content_type_authors_and_display_name_are_searchable() {
    let items = [
        CSSearchableItem::new("r", "docs", CSSearchableItemAttributeSet::new("Report").display_name("Q3 Report").content_type("public.text").content_creation_date(1_700_000_000).content_modification_date(0).author_names(&["Ada Lovelace"])),
        CSSearchableItem::new("o", "docs", CSSearchableItemAttributeSet::new("Other")),
    ];
    assert_eq!(found("lovelace", &items), ["r"]);
    assert_eq!(found("public.text", &items), ["r"]);
    assert_eq!(found("2023-11-14", &items), ["r"], "the creation date is indexed as YYYY-MM-DD");
    assert_eq!(found("1970-01-01", &items), ["r"], "the modification date is indexed as YYYY-MM-DD");
    assert_eq!(found("q3", &items), ["r"], "the display name stands in for the title");
}

#[test]
fn searchable_custom_values_are_found_and_unsearchable_ones_are_not() {
    let tag = CSCustomAttributeKey::new("tag", true);
    let pin = CSCustomAttributeKey::new("pin", false);
    assert!(tag.searchable && !pin.searchable);
    let set = CSSearchableItemAttributeSet::new("Note").custom_value(&tag, "finance").custom_value(&pin, "9999");
    assert_eq!(set.custom.len(), 2);
    assert!(!set.custom[0].unsearchable && set.custom[1].unsearchable);
    let items = [CSSearchableItem::new("n", "notes", set)];
    assert_eq!(found("finance", &items), ["n"]);
    assert!(found("9999", &items).is_empty());
}

#[test]
fn persons_are_found_by_name_and_by_handle() {
    let person = Person {
        display_name: "Grace Hopper".into(),
        handles: vec!["grace@example.test".into()],
        handle_identifier: "email".into(),
        contact_identifier: "contact-7".into(),
    };
    let items = [CSSearchableItem::new("m", "mail", CSSearchableItemAttributeSet::new("Compiler").person(person))];
    assert_eq!(found("hopper", &items), ["m"]);
    assert_eq!(found("example", &items), ["m"]);
    assert!(found("lovelace", &items).is_empty());
}

#[test]
fn localized_titles_pick_the_locale_then_the_language_then_the_first_text() {
    let t = LocalizedString::new(&[("en_US", "Milk"), ("is_IS", "Mjólk")]);
    assert_eq!(t.localized_string(&bifrost_app::locale::Locale::from_env("is_IS", None)), Some("Mjólk"));
    assert_eq!(t.localized_string(&bifrost_app::locale::Locale::from_env("is", None)), Some("Mjólk"));
    assert_eq!(t.localized_string(&bifrost_app::locale::Locale::from_env("de_DE", None)), Some("Milk"));
    assert_eq!(LocalizedString::default().localized_string(&bifrost_app::locale::Locale::from_env("en_US", None)), None);
}

#[test]
fn an_item_ranks_a_title_match_above_a_keyword_match() {
    let items = [
        CSSearchableItem::new("keyword", "notes", CSSearchableItemAttributeSet::new("Bread").keywords(&["milk"])),
        CSSearchableItem::new("title", "notes", CSSearchableItemAttributeSet::new("Milk")),
    ];
    assert_eq!(found("milk", &items), ["title", "keyword"]);
}

// --- Local search: indexing, errors and deletes ------------------------------------------------------------------------

#[test]
fn index_checked_refuses_bad_identifiers_and_sends_the_rest() {
    let c = ctx();
    let long_id = "x".repeat(MAX_ID + 1);
    let long_domain = "d".repeat(MAX_DOMAIN + 1);
    let items = [
        CSSearchableItem::new("a", "notes", CSSearchableItemAttributeSet::new("Milk")),
        CSSearchableItem::new("", "notes", CSSearchableItemAttributeSet::new("Empty")),
        CSSearchableItem::new(&long_id, "notes", CSSearchableItemAttributeSet::new("Long")),
        CSSearchableItem::new("n", &long_domain, CSSearchableItemAttributeSet::new("Domain")),
        CSSearchableItem::new("b", "notes", CSSearchableItemAttributeSet::new("Bread")),
    ];
    let refused = CSSearchableIndex::default_index().index_checked(&c, &items);
    assert_eq!(refused, vec![(1, IndexError::EmptyIdentifier), (2, IndexError::IdentifierTooLong), (3, IndexError::DomainTooLong)]);
    let sent = c.take_sent();
    assert_eq!(sent.len(), 1);
    let ToShell::QuickSearchIndex(batch) = &sent[0] else { panic!("expected an index message") };
    assert_eq!(batch.iter().map(|i| i.unique_id.as_str()).collect::<Vec<_>>(), ["a", "b"]);
    assert_eq!(batch[0].title, "Milk");
    assert_eq!(batch[1].domain, "notes");
}

#[test]
fn index_error_codes_and_domain_are_stable() {
    assert_eq!(IndexError::EmptyIdentifier.code(), 1);
    assert_eq!(IndexError::IdentifierTooLong.code(), 2);
    assert_eq!(IndexError::DomainTooLong.code(), 3);
    assert_eq!(IndexError::DOMAIN, "bifrost.quicksearch.index");
}

#[test]
fn deletes_are_sent_by_identifier_domain_and_all() {
    let c = ctx();
    let ix = CSSearchableIndex::default_index();
    ix.delete_searchable_items(&c, &["a", "b"]);
    ix.delete_searchable_items_with_domain_identifiers(&c, &["notes"]);
    ix.delete_all_searchable_items(&c);
    assert_eq!(
        c.take_sent(),
        [
            ToShell::QuickSearchDelete { kind: DELETE_IDS, values: vec!["a".into(), "b".into()] },
            ToShell::QuickSearchDelete { kind: DELETE_DOMAINS, values: vec!["notes".into()] },
            ToShell::QuickSearchDelete { kind: DELETE_ALL, values: vec![] },
        ]
    );
}

struct Store(Vec<CSSearchableItem>);

impl CSSearchableIndexDelegate for Store {
    fn reindex_all_searchable_items(&self, _ctx: &Ctx) -> Vec<CSSearchableItem> {
        self.0.clone()
    }
    fn searchable_items_for(&self, _ctx: &Ctx, identifiers: &[&str]) -> Vec<CSSearchableItem> {
        self.0.iter().filter(|i| identifiers.contains(&i.unique_identifier.as_str())).cloned().collect()
    }
}

#[test]
fn reindex_all_clears_the_index_and_indexes_the_delegate_items() {
    let c = ctx();
    let store = Store(vec![CSSearchableItem::new("a", "notes", CSSearchableItemAttributeSet::new("A")), CSSearchableItem::new("b", "notes", CSSearchableItemAttributeSet::new("B"))]);
    assert!(CSSearchableIndex::default_index().reindex_all_with(&c, &store).is_empty());
    let sent = c.take_sent();
    assert_eq!(sent[0], ToShell::QuickSearchDelete { kind: DELETE_ALL, values: vec![] });
    let ToShell::QuickSearchIndex(batch) = &sent[1] else { panic!("expected the reindexed batch") };
    assert_eq!(batch.iter().map(|i| i.unique_id.as_str()).collect::<Vec<_>>(), ["a", "b"]);
}

#[test]
fn reindex_identifiers_indexes_only_the_asked_items_and_deletes_nothing() {
    let c = ctx();
    let store = Store(vec![CSSearchableItem::new("a", "notes", CSSearchableItemAttributeSet::new("A")), CSSearchableItem::new("b", "notes", CSSearchableItemAttributeSet::new("B"))]);
    CSSearchableIndex::default_index().reindex_identifiers_with(&c, &["b", "zzz"], &store);
    let sent = c.take_sent();
    assert_eq!(sent.len(), 1);
    let ToShell::QuickSearchIndex(batch) = &sent[0] else { panic!("expected an index message") };
    assert_eq!(batch.len(), 1);
    assert_eq!(batch[0].unique_id, "b");
}

#[test]
fn an_open_result_carries_the_action_type_and_the_item_identifier() {
    let activity = UserActivity { activity_type: ACTION_TYPE.into(), unique_identifier: "n1".into(), domain_identifier: "notes".into() };
    assert_eq!(activity.activity_type, "bifrost.quicksearch.item");
    assert_eq!(activity.unique_identifier, "n1");
    assert_eq!(activity.domain_identifier, "notes");
}

// --- Local search: the index ledger (acknowledged writes) --------------------------------------------------------------

fn item(id: &str) -> CSSearchableItem {
    CSSearchableItem::new(id, "notes", CSSearchableItemAttributeSet::new("Milk"))
}

#[test]
fn the_ledger_refuses_oversized_items_and_names_the_offending_position() {
    let mut l = SearchIndexLedger::new();
    let big = CSSearchableItem::new("big", "notes", CSSearchableItemAttributeSet::new("t").content_description(&"x".repeat(MAX_ATTRIBUTE_BYTES + 1)));
    let e = l.index(&[item("ok"), big]).unwrap_err();
    assert_eq!((e.code, e.item_position), (CSIndexErrorCode::InvalidItemError, Some(1)));
    assert_eq!(l.item_count(), 0, "a refused call indexes nothing");
    assert_eq!(e.code.raw_value(), -1001);
}

#[test]
fn the_ledger_caps_the_item_count_with_quota_exceeded() {
    let mut l = SearchIndexLedger::new();
    let all: Vec<CSSearchableItem> = (0..MAX_INDEXED_ITEMS).map(|i| item(&format!("n{i}"))).collect();
    l.index(&all).unwrap();
    let e = l.index(&[item("extra")]).unwrap_err();
    assert_eq!(e.code, CSIndexErrorCode::QuotaExceeded);
    assert_eq!(e.code.raw_value(), -1004);
    assert_eq!(l.item_count(), MAX_INDEXED_ITEMS);
    assert!(l.index(&[item("n0")]).is_ok(), "replacing an item is not a new item");
}

#[test]
fn a_batch_commits_all_its_calls_with_the_client_state() {
    let mut l = SearchIndexLedger::new();
    l.begin_batch().unwrap();
    assert!(l.index(&[item("a")]).unwrap().is_empty(), "calls inside a batch are held back");
    l.index(&[item("b")]).unwrap();
    let ops = l.end_batch(b"v1").unwrap();
    assert_eq!(ops.len(), 2);
    assert!(matches!(ops[0], IndexOp::Index(_)));
    assert_eq!(l.fetch_last_client_state().unwrap(), b"v1");
    assert!(l.contains("a") && l.contains("b"));
}

#[test]
fn a_batch_with_one_failed_call_commits_none_of_them() {
    let mut l = SearchIndexLedger::new();
    l.index(&[item("keep")]).unwrap();
    l.begin_batch().unwrap();
    l.index(&[item("a")]).unwrap();
    l.delete_all().unwrap();
    assert!(l.index(&[item("")]).is_err());
    let e: CSIndexError = l.end_batch(b"v2").unwrap_err();
    assert_eq!(e.code, CSIndexErrorCode::InvalidItemError);
    assert!(l.contains("keep") && !l.contains("a"), "the index is restored to its state before the batch");
    assert!(l.fetch_last_client_state().unwrap().is_empty());
}

#[test]
fn client_state_must_be_small_and_a_batch_must_be_open() {
    let mut l = SearchIndexLedger::new();
    assert_eq!(l.end_batch(b"x").unwrap_err().code, CSIndexErrorCode::InvalidClientStateError);
    l.begin_batch().unwrap();
    assert_eq!(l.begin_batch().unwrap_err().code, CSIndexErrorCode::InvalidClientStateError);
    assert_eq!(l.end_batch(&vec![0u8; MAX_CLIENT_STATE_BYTES + 1]).unwrap_err().code, CSIndexErrorCode::InvalidClientStateError);
}

#[test]
fn a_closed_ledger_answers_index_unavailable() {
    let mut l = SearchIndexLedger::new();
    l.close();
    assert_eq!(l.index(&[item("a")]).unwrap_err().code, CSIndexErrorCode::IndexUnavailableError);
    assert_eq!(l.fetch_last_client_state().unwrap_err().code, CSIndexErrorCode::IndexUnavailableError);
}

#[test]
fn the_completion_form_sends_only_what_the_ledger_accepts() {
    let c = ctx();
    let mut l = SearchIndexLedger::new();
    let outcome = Cell::new(None);
    CSSearchableIndex::default_index().index_searchable_items_with_completion(&c, &mut l, &[item("a")], |r| outcome.set(Some(r)));
    assert_eq!(outcome.get(), Some(Ok(())));
    assert_eq!(c.take_sent().len(), 1);
    let refused = Cell::new(None);
    CSSearchableIndex::default_index().index_searchable_items_with_completion(&c, &mut l, &[item("")], |r| refused.set(Some(r)));
    assert_eq!(refused.get().map(|r| r.unwrap_err().code), Some(CSIndexErrorCode::InvalidItemError));
    assert!(c.take_sent().is_empty());
}

// --- Notifications (in process and distributed) -----------------------------------------------------------------------

#[test]
fn an_observer_hears_its_name_and_its_object_only() {
    let center = NotificationCenter::default();
    let seen: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let s = seen.clone();
    let token = center.add_observer("refresh", Some(7), move |n| s.borrow_mut().push(n.name.clone()));
    center.post("refresh", Some(7), HashMap::new());
    center.post("refresh", Some(8), HashMap::new());
    center.post("other", Some(7), HashMap::new());
    assert_eq!(seen.borrow().clone(), vec!["refresh".to_string()]);
    drop(token);
    center.post("refresh", Some(7), HashMap::new());
    assert_eq!(seen.borrow().len(), 1, "dropping the token removes the observer");
}

#[test]
fn an_observer_without_an_object_hears_every_object_and_gets_the_user_info() {
    let center = NotificationCenter::default();
    let got: Rc<RefCell<Vec<(Option<u64>, Option<String>)>>> = Rc::new(RefCell::new(Vec::new()));
    let g = got.clone();
    let _token = center.add_observer("sync", None, move |n| g.borrow_mut().push((n.object, n.user_info.get("k").cloned())));
    let mut info = HashMap::new();
    info.insert("k".to_string(), "v".to_string());
    center.post("sync", Some(1), info);
    center.post("sync", None, HashMap::new());
    assert_eq!(got.borrow().clone(), vec![(Some(1), Some("v".to_string())), (None, None)]);
}

#[test]
fn a_distributed_post_reaches_local_observers_and_is_queued_for_the_shell_sorted() {
    let c = ctx();
    let heard: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let h = heard.clone();
    let _token = DistributedNotificationCenter.add_observer("bifrost.test.ping", move |n| *h.borrow_mut() = n.user_info.get("who").cloned());
    let mut info = HashMap::new();
    info.insert("who".to_string(), "ada".to_string());
    info.insert("a".to_string(), "1".to_string());
    DistributedNotificationCenter.post_with(&c, "bifrost.test.ping", info);
    assert_eq!(heard.borrow().clone(), Some("ada".to_string()));
    assert_eq!(
        c.take_sent(),
        [ToShell::DistPost { name: "bifrost.test.ping".into(), user_info: vec![("a".into(), "1".into()), ("who".into(), "ada".into())] }]
    );
}

#[test]
fn a_process_local_distributed_post_sends_nothing_to_the_shell() {
    let c = ctx();
    DistributedNotificationCenter.post("bifrost.test.local", HashMap::new());
    assert!(c.take_sent().is_empty());
}

// --- Notify bus helpers (token, flag and state replies) ---------------------------------------------------------------

fn obj(fields: Vec<(&str, Json)>) -> Json {
    Json::Obj(fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

#[test]
fn register_reply_yields_a_positive_integer_token() {
    assert_eq!(notify_bus::token_from_reply(&Ok(obj(vec![("token", Json::Num(7.0))]))), Ok(7));
    assert!(notify_bus::token_from_reply(&Ok(obj(vec![("token", Json::Num(2.5))]))).is_err(), "a fraction is not a token");
    assert!(notify_bus::token_from_reply(&Ok(obj(vec![("token", Json::Num(0.0))]))).is_err(), "zero is never a token");
    assert_eq!(notify_bus::token_from_reply(&Err("boom".into())), Err("boom".to_string()));
}

#[test]
fn flag_and_state_replies_are_read_strictly() {
    assert_eq!(notify_bus::flag_from_reply(&Ok(obj(vec![("pending", Json::Bool(true))])), "pending"), Ok(true));
    assert!(notify_bus::flag_from_reply(&Ok(obj(vec![])), "pending").is_err());
    assert_eq!(notify_bus::state_from_reply(&Ok(obj(vec![("state", Json::Num(42.0))]))), Ok(42));
    assert!(notify_bus::state_from_reply(&Ok(obj(vec![("state", Json::Num(-1.0))]))).is_err());
}

#[test]
fn a_delivered_name_decodes_only_from_the_notify_bus_event() {
    let body = obj(vec![("token", Json::Num(3.0)), ("name", Json::Str("refresh".into()))]);
    assert_eq!(notify_bus::notified(notify_bus::SERVICE, notify_bus::EVENT, &body), Some(Notified { token: 3, name: "refresh".into() }));
    assert_eq!(notify_bus::notified("other", notify_bus::EVENT, &body), None);
    assert_eq!(notify_bus::notified(notify_bus::SERVICE, "other", &body), None);
    assert_eq!(notify_bus::notified(notify_bus::SERVICE, notify_bus::EVENT, &obj(vec![("name", Json::Str("x".into()))])), None);
    assert_eq!(notify_bus::APPEARANCE_CHANGE, "system.appearance.change");
    assert_ne!(Mode::Event, Mode::Poll);
}

#[test]
fn notify_bus_calls_go_to_the_notify_bus_service_with_their_method() {
    let c = ctx();
    let r = notify_bus::register(&c, "refresh", Mode::Poll);
    let p = notify_bus::post(&c, "refresh");
    let sent = c.take_sent();
    let ToShell::ServiceCall { request, service, method, body } = &sent[0] else { panic!("expected a service call") };
    assert_eq!((*request, service.as_str(), method.as_str()), (r, notify_bus::SERVICE, "register"));
    assert!(body.contains("poll"), "a poll token asks for the poll mode");
    let ToShell::ServiceCall { request, method, .. } = &sent[1] else { panic!("expected a service call") };
    assert_eq!((*request, method.as_str()), (p, "post"));
}

// --- Menu-bar extras (status items) -----------------------------------------------------------------------------------

#[test]
fn a_status_item_sends_its_whole_state_on_every_change() {
    let c = ctx();
    let item = StatusBar::system().status_item(StatusItemLength::Variable);
    item.set_title(&c, "42%");
    item.set_menu(&c, &[MenuItem::new(1, "Open"), MenuItem::separator(), MenuItem::new(2, "Quit").enabled(false).checked(true)]);
    item.set_visible(&c, false);
    let sent = c.take_sent();
    assert_eq!(sent.len(), 3);
    let ToShell::StatusItemSet { item: id, visible, title, menu, .. } = &sent[1] else { panic!("expected a status item update") };
    assert_eq!(*id, item.id());
    assert!(*visible);
    assert_eq!(title.as_str(), "42%");
    assert_eq!(menu.len(), 3);
    assert_eq!(menu[0].title.as_str(), "Open");
    assert_eq!(menu[0].id, 1);
    assert!(menu[1].separator);
    assert!(!menu[2].enabled && menu[2].checked);
    let ToShell::StatusItemSet { visible, .. } = &sent[2] else { panic!("expected a status item update") };
    assert!(!visible, "hiding the item is sent as visible false");
}

#[test]
fn status_item_ids_are_distinct_and_removal_names_the_item() {
    let c = ctx();
    let a = StatusBar::system().status_item(StatusItemLength::Square);
    let b = StatusBar::system().status_item(StatusItemLength::Variable);
    assert_ne!(a.id(), b.id());
    let id = b.id();
    b.remove(&c);
    assert_eq!(c.take_sent(), [ToShell::StatusItemRemove { item: id }]);
}

// --- Local authentication ---------------------------------------------------------------------------------------------

fn reply(ok: bool, error: Option<&str>) -> Json {
    let mut fields = vec![("ok", Json::Bool(ok))];
    if let Some(e) = error {
        fields.push(("error", Json::Str(e.into())));
    }
    obj(fields)
}

#[test]
fn the_device_owner_policy_needs_a_password_and_biometry_is_never_available() {
    let with_password = LAContext::new(Device { password_set: true });
    assert_eq!(with_password.can_evaluate_policy(LAPolicy::DeviceOwnerAuthentication), Ok(()));
    assert_eq!(with_password.can_evaluate_policy(LAPolicy::DeviceOwnerAuthenticationWithBiometrics), Err(LAError::BiometryNotAvailable));
    assert_eq!(with_password.biometry_type(), BiometryType::None);
    let without = LAContext::new(Device { password_set: false });
    assert_eq!(without.can_evaluate_policy(LAPolicy::DeviceOwnerAuthentication), Err(LAError::PasscodeNotSet));
}

#[test]
fn an_invalidated_context_fails_with_invalid_context() {
    let mut c = LAContext::new(Device { password_set: true });
    c.invalidate();
    assert_eq!(c.can_evaluate_policy(LAPolicy::DeviceOwnerAuthentication), Err(LAError::InvalidContext));
}

#[test]
fn a_success_inside_the_reuse_window_does_not_prompt_again() {
    let mut c = LAContext::new(Device { password_set: true });
    c.touch_id_authentication_allowable_reuse_duration = 60;
    let asked = Cell::new(0);
    let prompt = |asked: &Cell<u32>| {
        asked.set(asked.get() + 1);
        reply(true, None)
    };
    assert_eq!(c.evaluate_policy(LAPolicy::DeviceOwnerAuthentication, "Reveal", 100, |_| prompt(&asked)), Ok(()));
    assert_eq!(c.evaluate_policy(LAPolicy::DeviceOwnerAuthentication, "Reveal", 130, |_| prompt(&asked)), Ok(()));
    assert_eq!(asked.get(), 1, "the second call reuses the success");
    assert_eq!(c.evaluate_policy(LAPolicy::DeviceOwnerAuthentication, "Reveal", 200, |_| prompt(&asked)), Ok(()));
    assert_eq!(asked.get(), 2, "after the window the prompt shows again");
}

#[test]
fn a_refused_prompt_reports_the_shell_error_and_unknown_errors_fall_back_to_authentication_failed() {
    let mut c = LAContext::new(Device { password_set: true });
    assert_eq!(decode_reply(&reply(false, Some("user_cancel"))), Err(LAError::UserCancel));
    assert_eq!(c.evaluate_policy(LAPolicy::DeviceOwnerAuthentication, "r", 1, |_| reply(false, Some("biometry_lockout"))), Err(LAError::BiometryLockout));
    assert_eq!(decode_reply(&reply(false, Some("gremlins"))), Err(LAError::AuthenticationFailed));
    assert_eq!(decode_reply(&Json::Null), Err(LAError::AuthenticationFailed));
}

#[test]
fn every_error_wire_name_round_trips_and_policies_have_their_names() {
    let all = [
        LAError::AuthenticationFailed,
        LAError::UserCancel,
        LAError::UserFallback,
        LAError::SystemCancel,
        LAError::PasscodeNotSet,
        LAError::BiometryNotAvailable,
        LAError::BiometryNotEnrolled,
        LAError::BiometryLockout,
        LAError::AppCancel,
        LAError::InvalidContext,
        LAError::NotInteractive,
    ];
    for e in all {
        assert_eq!(LAError::from_wire(e.wire()), Some(e));
    }
    assert_eq!(LAError::from_wire("nope"), None);
    assert_eq!(LAPolicy::DeviceOwnerAuthentication.wire(), "device_owner_authentication");
}

// --- Sharing ----------------------------------------------------------------------------------------------------------

#[test]
fn the_share_menu_lists_apps_that_take_the_items_then_compose_and_copy() {
    let handles = vec!["public.utf8-plain-text".to_string()];
    let notes = Candidate { id: "notes", name: "Notes", handles: &handles, schemes: &[] };
    let items = [Payload::Text("hello".into())];
    let services = SharingService::sharing_services_for_items(&items, &[notes]);
    let names: Vec<&str> = services.iter().map(|s| s.name()).collect();
    assert_eq!(names, ["notes", COMPOSE, COPY]);
    assert_eq!(services[0].title(), "Notes");
    assert!(SharingService::sharing_services_for_items(&[], &[]).is_empty(), "nothing shared offers nothing");
}

#[test]
fn a_service_performs_only_when_it_takes_every_item() {
    let handles = vec!["public.utf8-plain-text".to_string()];
    let notes = Candidate { id: "notes", name: "Notes", handles: &handles, schemes: &["https".to_string()] };
    let text = [Payload::Text("hi".into())];
    let services = SharingService::sharing_services_for_items(&text, &[notes]);
    assert!(services[0].can_perform(&text));
    assert!(!services[0].can_perform(&[]));
    let mixed = [Payload::Text("hi".into()), Payload::Url("ftp://example.test".into())];
    assert!(!services[0].can_perform(&mixed));
}

#[test]
fn copy_puts_the_items_on_the_clipboard_one_per_line() {
    let c = ctx();
    let items = [Payload::Text("a".into()), Payload::Url("https://b.test".into())];
    let copy = SharingService::named(COPY).expect("Copy is built in");
    assert!(copy.can_perform(&items));
    assert!(copy.perform(&c, &items));
    assert_eq!(c.take_sent(), [ToShell::SetClipboard { text: "a\nhttps://b.test".into() }]);
    let compose = SharingService::named(COMPOSE).expect("Compose Post is built in");
    assert!(!compose.perform(&c, &items), "Compose Post is reached through the menu");
    assert!(c.take_sent().is_empty());
    assert!(SharingService::named("no.such.service").is_none());
}

#[test]
fn inbound_requests_are_flagged_and_text_urls_decode() {
    assert!(bifrost_app::share::is_inbound(INBOUND | 5));
    assert!(!bifrost_app::share::is_inbound(5));
    assert_eq!(text_from_url("data:text/plain;charset=utf-8,hi%20there"), Some("hi there".to_string()));
    assert_eq!(text_from_url("https://example.test"), None);
}

// --- Preview panel ----------------------------------------------------------------------------------------------------

fn previews(n: usize) -> Vec<PreviewItem> {
    (0..n).map(|i| PreviewItem { url: format!("file:///docs/{i}.txt"), title: format!("Doc {i}") }).collect()
}

#[test]
fn the_preview_index_stays_inside_the_list() {
    let mut p = PreviewPanel::new();
    p.set_items(previews(3));
    p.step(5);
    assert_eq!(p.current_preview_item_index(), 2, "stepping past the end stops at the last item");
    p.step(-9);
    assert_eq!(p.current_preview_item_index(), 0, "stepping before the start stops at the first item");
    p.set_current_preview_item_index(10);
    assert_eq!(p.current_item().map(|i| i.title.as_str()), Some("Doc 2"));
    p.set_items(previews(1));
    assert_eq!(p.current_preview_item_index(), 0, "a shorter list clamps the index");
}

#[test]
fn each_item_keeps_its_display_state_and_a_new_list_resets_them() {
    let mut p = PreviewPanel::new();
    p.set_items(previews(2));
    assert_eq!(p.display_state(0), Some(ItemState::Ready), "items start ready");
    assert!(p.set_display_state(1, ItemState::Failed));
    assert_eq!(p.display_state(1), Some(ItemState::Failed));
    assert!(!p.set_display_state(9, ItemState::Loading), "past the end changes nothing");
    p.set_items(previews(2));
    assert_eq!(p.display_state(1), Some(ItemState::Ready));
    assert_eq!(ItemState::default(), ItemState::Ready);
}

#[test]
fn the_data_source_fills_at_most_the_item_cap() {
    struct Many(usize);
    impl PreviewDataSource for Many {
        fn number_of_items(&self) -> usize {
            self.0
        }
        fn item_at(&self, index: usize) -> Option<PreviewItem> {
            (index < self.0).then(|| PreviewItem { url: format!("file:///{index}"), title: index.to_string() })
        }
    }
    let mut p = PreviewPanel::new();
    p.load_from(&Many(MAX_ITEMS + 3));
    assert_eq!(p.items().len(), MAX_ITEMS);
    p.load_from(&previews(2));
    assert_eq!(p.items().len(), 2);
}

#[test]
fn the_panel_shows_and_hides_through_the_shell_and_an_empty_panel_stays_hidden() {
    let c = ctx();
    let mut empty = PreviewPanel::new();
    assert!(!empty.toggle(&c), "nothing to show stays hidden");
    assert!(c.take_sent().is_empty());
    let mut p = PreviewPanel::new();
    p.set_items(previews(2));
    assert!(p.toggle(&c), "the first space shows the panel");
    assert!(p.is_visible());
    let ToShell::Preview { items, index } = c.take_sent().remove(0) else { panic!("expected the preview list") };
    assert_eq!((items.len(), index), (2, 0));
    assert!(!p.toggle(&c), "the next space hides it");
    assert_eq!(c.take_sent(), [ToShell::PreviewClose]);
}

// --- Thumbnails -------------------------------------------------------------------------------------------------------

#[test]
fn thumbnail_edges_scale_and_clamp_to_the_pixel_limits() {
    assert_eq!(edge_px(100, 2.0), 200);
    assert_eq!(edge_px(4, 1.0), MIN_PX);
    assert_eq!(edge_px(4000, 1.0), 1024);
    assert_eq!(ThumbRequest::new("/home/u/a.png", 64, 2.0, ThumbKind::Thumbnail).unwrap().px(), 128);
}

#[test]
fn a_thumbnail_request_needs_an_absolute_clean_path_and_a_sane_size() {
    assert_eq!(ThumbRequest::new("relative.png", 64, 1.0, ThumbKind::Icon), Err(RequestError::BadPath));
    assert_eq!(ThumbRequest::new("/home/u/../x.png", 64, 1.0, ThumbKind::Icon), Err(RequestError::BadPath));
    assert_eq!(ThumbRequest::new("/home/u/a.png", 0, 1.0, ThumbKind::Icon), Err(RequestError::BadSize));
    assert_eq!(ThumbRequest::new("/home/u/a.png", 64, 9.0, ThumbKind::Icon), Err(RequestError::BadSize));
    assert_eq!(ThumbRequest::new("/home/u/a.png", 64, f32::NAN, ThumbKind::Icon), Err(RequestError::BadSize));
    assert_eq!(ThumbKind::parse("thumbnail"), Some(ThumbKind::Thumbnail));
    assert_eq!(ThumbKind::parse("poster"), None);
}

#[test]
fn a_thumbnail_request_is_sent_to_the_thumbnails_service() {
    let c = ctx();
    let r = thumbnails::request(&c, "/home/u/a.png", 64, 2.0, ThumbKind::Thumbnail);
    let sent = c.take_sent();
    let ToShell::ServiceCall { request, service, method, body } = &sent[0] else { panic!("expected a service call") };
    assert_eq!(*request, r);
    assert_eq!((service.as_str(), method.as_str()), (thumbnails::SERVICE, "request"));
    assert!(body.contains("thumbnail") && body.contains("/home/u/a.png"));
}

#[test]
fn thumbnail_answers_decode_into_their_states() {
    let base = |state: &str| vec![("file", Json::Str("/home/u/a.png".into())), ("px", Json::Num(128.0)), ("state", Json::Str(state.into()))];
    let pending = thumbnails::reply(&obj(base("pending"))).unwrap();
    assert_eq!(pending.state, State::Pending);
    assert_eq!(pending.px, 128);

    let mut pixels = base("ready");
    pixels.push(("art", Json::Str("pixels".into())));
    pixels.push(("image", Json::Str("/cache/x.raw".into())));
    pixels.push(("w", Json::Num(128.0)));
    pixels.push(("h", Json::Num(64.0)));
    assert_eq!(thumbnails::reply(&obj(pixels)).unwrap().state, State::Pixels { image: "/cache/x.raw".into(), w: 128, h: 64 });

    let mut icon = base("ready");
    icon.push(("art", Json::Str("icon".into())));
    icon.push(("type", Json::Str("public.png".into())));
    assert_eq!(thumbnails::reply(&obj(icon)).unwrap().state, State::Icon { type_id: "public.png".into(), badge: None });

    let mut failed = base("failed");
    failed.push(("error", Json::Str("unreadable".into())));
    assert_eq!(thumbnails::reply(&obj(failed)).unwrap().state, State::Failed("unreadable".into()));

    assert!(thumbnails::reply(&obj(base("weird"))).is_none(), "an unknown state is not an answer");
}

#[test]
fn only_the_thumbnail_ready_and_failed_events_are_answers() {
    let body = obj(vec![("file", Json::Str("/home/u/a.png".into())), ("px", Json::Num(16.0)), ("state", Json::Str("failed".into())), ("error", Json::Str("x".into()))]);
    assert!(thumbnails::event(thumbnails::SERVICE, thumbnails::READY_EVENT, &body).is_some());
    assert!(thumbnails::event(thumbnails::SERVICE, thumbnails::FAILED_EVENT, &body).is_some());
    assert!(thumbnails::event(thumbnails::SERVICE, "later", &body).is_none());
    assert!(thumbnails::event("other", thumbnails::READY_EVENT, &body).is_none());
}

// --- Keychain and intents ---------------------------------------------------------------------------------------------

#[test]
fn a_keychain_add_is_sent_with_its_service_account_and_secret() {
    let c = ctx();
    let r = bifrost_app::keychain::add(&c, "bifrost.mail", "ada", b"token-1");
    let sent = c.take_sent();
    let ToShell::Keychain { request, op } = &sent[0] else { panic!("expected a keychain call") };
    assert_eq!(*request, r);
    assert_eq!(*op, KeychainOp::Add { service: "bifrost.mail".into(), account: "ada".into(), value: b"token-1".to_vec() });
}

#[test]
fn an_intent_serialises_its_identifier_and_title_into_the_catalog() {
    let intent = Intent::new("open-note", "Open Note", "Opens a note");
    let json = intent.to_json();
    assert_eq!(json.str_field("title").as_deref(), Some("Open Note"));
    assert_eq!(json.str_field("description").as_deref(), Some("Opens a note"));
    assert!(!intent.method().is_empty());
}

// --- Tool tips (the widget material model) ----------------------------------------------------------------------------

#[test]
fn the_tool_tip_material_is_listed_maps_to_its_window_model_kind_and_blurs_six_points() {
    use bifrost_app::widgets::Material;
    assert_eq!(Material::ALL.len(), 14);
    assert!(Material::ALL.contains(&Material::ToolTip));
    assert_eq!(Material::from_model(Material::ToolTip.model_kind()), Material::ToolTip);
    assert_eq!(Material::ToolTip.blur_pt(), 6.0);
}
