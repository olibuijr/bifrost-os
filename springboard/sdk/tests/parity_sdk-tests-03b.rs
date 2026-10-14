//! Parity sdk-tests-03: headless tests of the SDK contacts module (the reply decoders, the typed errors, the access
//! preview and the field names) and of the restorable state file. Every check decodes or encodes plain values or uses a
//! scratch folder under the crate's target directory. No test calls the shell or opens a window.

use bifrost_app::contacts::{
    decode_contacts, decode_error, decode_picked, decode_saved, decode_status, preview, Choice, ContactsError, DateComponents,
    Error, Key, LabeledValue, Record, Status, Value, LABEL_HOME, LABEL_OTHER, LABEL_PHONE_MAIN, LABEL_PHONE_MOBILE, LABEL_WORK,
};
use bifrost_app::restoration::{self, RestorableState};
use bifrost_core::json::Json;
use std::collections::BTreeSet;
use std::path::PathBuf;

fn obj(fields: Vec<(&str, Json)>) -> Json {
    Json::Obj(fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

fn reply(rows: Vec<Json>) -> Json {
    obj(vec![("contacts", Json::Arr(rows))])
}

fn entry(label: &str, value: &str, identifier: &str) -> Json {
    obj(vec![("label", Json::Str(label.into())), ("value", Json::Str(value.into())), ("identifier", Json::Str(identifier.into()))])
}

/// A scratch folder for one test, emptied first, under the crate's target directory.
fn scratch(test: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/parity-sdk-tests-03").join(test);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

// --- status and access -------------------------------------------------------------------------------------------------

#[test]
fn every_status_decodes_from_its_wire_name_and_an_unknown_name_does_not() {
    let all = [Status::NotDetermined, Status::Restricted, Status::Denied, Status::Authorized, Status::Limited];
    for status in all {
        let r = obj(vec![("status", Json::Str(status.name().into()))]);
        assert_eq!(decode_status(&r), Some(status), "{}", status.name());
        assert_eq!(Status::from_name(status.name()), Some(status));
    }
    assert_eq!(decode_status(&obj(vec![("status", Json::Str("all".into()))])), None);
    assert_eq!(decode_status(&Json::Null), None);
}

#[test]
fn the_picked_contacts_decode_and_a_reply_without_them_holds_none() {
    let r = obj(vec![("picked", Json::Arr(vec![Json::Num(4.0), Json::Num(9.0), Json::Str("x".into())]))]);
    assert_eq!(decode_picked(&r), [4u64, 9].into_iter().collect::<BTreeSet<u64>>(), "a non-number is ignored");
    assert!(decode_picked(&Json::Null).is_empty());
}

#[test]
fn the_access_preview_applies_the_same_rules_as_the_shell() {
    assert_eq!(preview(Choice::All), Ok(Status::Authorized));
    assert_eq!(preview(Choice::Deny), Ok(Status::Denied));
    assert_eq!(preview(Choice::Picked([1u64].into_iter().collect())), Ok(Status::Limited));
    assert_eq!(preview(Choice::Picked(BTreeSet::new())), Err(Error::NothingPicked));
}

// --- fetch replies -----------------------------------------------------------------------------------------------------

#[test]
fn a_fetch_reply_decodes_to_records_and_a_missing_value_is_no_field() {
    let people = decode_contacts(&reply(vec![
        obj(vec![("id", Json::Num(1.0)), ("given_name", Json::Str("Ada".into()))]),
        obj(vec![("id", Json::Num(2.0)), ("given_name", Json::Null)]),
    ]))
    .unwrap();
    assert_eq!(people.len(), 2);
    assert_eq!(people[0].id, 1);
    assert_eq!(people[0].get(Key::GivenName), Some(&Value::Text("Ada".into())));
    assert_eq!(people[1].get(Key::GivenName), None);
}

#[test]
fn a_record_without_an_id_fails_the_whole_reply() {
    let bad = reply(vec![obj(vec![("given_name", Json::Str("No id".into()))])]);
    assert_eq!(decode_contacts(&bad), None);
    assert_eq!(decode_contacts(&Json::Null), None);
    assert_eq!(decode_contacts(&reply(vec![])), Some(vec![]), "an empty book is an empty list");
}

#[test]
fn labeled_entries_keep_their_identifiers_when_the_reply_has_them() {
    let people = decode_contacts(&reply(vec![obj(vec![("id", Json::Num(3.0)), ("emails", Json::Arr(vec![entry("home", "a@x.is", "7")]))])])).unwrap();
    let expected = Value::Labeled(vec![LabeledValue { label: "home".into(), value: "a@x.is".into(), identifier: "7".into() }]);
    assert_eq!(people[0].get(Key::Emails), Some(&expected));
}

#[test]
fn labeled_entries_without_identifiers_are_numbered_by_position() {
    let people = decode_contacts(&reply(vec![obj(vec![
        ("id", Json::Num(4.0)),
        ("emails", Json::Arr(vec![entry("home", "a@x.is", ""), Json::Str("old@x.is".into())])),
    ])]))
    .unwrap();
    let Some(Value::Labeled(items)) = people[0].get(Key::Emails) else { panic!("emails are labeled") };
    let shape: Vec<(&str, &str, &str)> = items.iter().map(|i| (i.label.as_str(), i.value.as_str(), i.identifier.as_str())).collect();
    assert_eq!(shape, vec![("home", "a@x.is", "1"), ("", "old@x.is", "2")], "a plain string is a labeled entry with no label");
}

#[test]
fn a_birthday_and_labeled_dates_decode_and_a_bad_date_is_left_out() {
    let people = decode_contacts(&reply(vec![
        obj(vec![
            ("id", Json::Num(5.0)),
            ("birthday", obj(vec![("month", Json::Num(12.0)), ("day", Json::Num(10.0))])),
            (
                "dates",
                Json::Arr(vec![obj(vec![
                    ("label", Json::Str("anniversary".into())),
                    ("year", Json::Num(2001.0)),
                    ("month", Json::Num(5.0)),
                    ("day", Json::Num(6.0)),
                ])]),
            ),
        ]),
        obj(vec![("id", Json::Num(6.0)), ("birthday", obj(vec![("month", Json::Num(13.0)), ("day", Json::Num(1.0))]))]),
    ]))
    .unwrap();
    assert_eq!(people[0].get(Key::Birthday), Some(&Value::Date(DateComponents::new(None, 12, 10).unwrap())));
    assert_eq!(
        people[0].get(Key::Dates),
        Some(&Value::Dates(vec![("anniversary".into(), DateComponents::new(Some(2001), 5, 6).unwrap())]))
    );
    assert_eq!(people[1].get(Key::Birthday), None, "month 13 is not a date");
}

#[test]
fn a_record_sets_and_replaces_its_fields_and_a_blank_value_removes_the_field() {
    let mut r = Record::new(8);
    r.set(Key::Note, Value::Text("first".into()));
    r.set(Key::Note, Value::Text("second".into()));
    assert_eq!(r.get(Key::Note), Some(&Value::Text("second".into())));
    r.set(Key::Note, Value::Text("   ".into()));
    assert_eq!(r.get(Key::Note), None);
    r.set(Key::Emails, Value::List(vec![" ".into()]));
    assert_eq!(r.get(Key::Emails), None, "a list of blanks is blank");
}

// --- saves ---------------------------------------------------------------------------------------------------------------

#[test]
fn a_save_reply_decodes_to_the_added_ids_in_order() {
    let r = obj(vec![("added", Json::Arr(vec![Json::Num(4.0), Json::Num(5.0)]))]);
    assert_eq!(decode_saved(&r), Some(vec![4, 5]));
    assert_eq!(decode_saved(&obj(vec![("added", Json::Arr(vec![Json::Str("4".into())]))])), None);
    assert_eq!(decode_saved(&Json::Null), None);
}

// --- typed errors --------------------------------------------------------------------------------------------------------

#[test]
fn a_shell_error_message_maps_to_its_typed_error_and_any_other_stays_as_text() {
    assert_eq!(ContactsError::from_message("contacts access is not granted"), ContactsError::NotGranted);
    assert_eq!(
        ContactsError::from_message("a limited contacts grant only reads; saving needs full access"),
        ContactsError::ReadOnly
    );
    assert_eq!(ContactsError::from_message("fetch needs keys"), ContactsError::Other("fetch needs keys".into()));
}

#[test]
fn an_access_error_maps_to_the_matching_typed_error() {
    assert_eq!(ContactsError::from(Error::Restricted), ContactsError::Restricted);
    assert_eq!(ContactsError::from(Error::AlreadyDecided), ContactsError::AlreadyDecided);
    assert_eq!(ContactsError::from(Error::NothingPicked), ContactsError::NothingPicked);
    assert_eq!(ContactsError::from(Error::ReadOnly), ContactsError::ReadOnly);
    assert_eq!(ContactsError::from(Error::NotAuthorized), ContactsError::NotGranted);
}

#[test]
fn a_failed_reply_decodes_to_its_typed_error_and_a_successful_one_to_none() {
    assert_eq!(decode_error(&Err("contacts access is not granted".into())), Some(ContactsError::NotGranted));
    assert_eq!(decode_error(&Ok(Json::Null)), None);
}

// --- field names and labels ------------------------------------------------------------------------------------------------

#[test]
fn every_field_name_round_trips_and_an_unknown_name_is_refused() {
    for key in Key::ALL {
        assert_eq!(Key::from_name(key.name()), Some(key));
    }
    assert_eq!(Key::Phones.name(), "phones");
    assert_eq!(Key::GivenName.name(), "given_name");
    assert_eq!(Key::from_name("nickname"), None);
}

#[test]
fn the_label_names_are_the_plain_words_the_book_stores() {
    assert_eq!((LABEL_HOME, LABEL_WORK, LABEL_OTHER), ("home", "work", "other"));
    assert_eq!((LABEL_PHONE_MOBILE, LABEL_PHONE_MAIN), ("mobile", "main"));
}

// --- restorable state file -----------------------------------------------------------------------------------------------

#[test]
fn a_saved_state_loads_back_for_its_window_and_only_once_the_state_changes() {
    let dir = scratch("restoration_round_trip");
    let state = RestorableState::new();
    assert_eq!(restoration::save(&dir, "main", (800, 600), &state, 1).unwrap(), true);
    assert_eq!(restoration::save(&dir, "main", (800, 600), &state, 2).unwrap(), false, "an unchanged state costs no write");
    assert_eq!(restoration::load(&dir, "main"), Some((state.clone(), (800, 600))));
}

#[test]
fn a_state_of_another_window_is_not_loaded_and_is_discarded() {
    let dir = scratch("restoration_other_window");
    restoration::save(&dir, "main", (800, 600), &RestorableState::new(), 1).unwrap();
    assert_eq!(restoration::load(&dir, "inspector"), None);
    assert!(!restoration::path(&dir).exists(), "a state that does not match is removed");
}

#[test]
fn a_corrupt_state_file_is_ignored_and_removed() {
    let dir = scratch("restoration_corrupt");
    std::fs::write(restoration::path(&dir), "{ not a record").unwrap();
    assert_eq!(restoration::load(&dir, "main"), None);
    assert!(!restoration::path(&dir).exists());
}

#[test]
fn discarding_removes_the_saved_state() {
    let dir = scratch("restoration_discard");
    restoration::save(&dir, "main", (640, 480), &RestorableState::new(), 1).unwrap();
    restoration::discard(&dir);
    assert_eq!(restoration::load(&dir, "main"), None);
}
