//! Headless tests for the parity rows of the SDK model layer: contact access, partial contact fetches, labeled values
//! and dates. Everything here is pure in-memory state: no display, no device, no store file and no clock.

use bifrost_ui::contact_access::{Access, Choice, Error as AccessError, Status};
use bifrost_ui::contact_fetch::{
    project, DateComponents, Error as FetchError, Key, Keys, LabeledValue, Record, Value, LABEL_HOME, LABEL_OTHER,
    LABEL_PHONE_MAIN, LABEL_PHONE_MOBILE, LABEL_WORK,
};
use std::collections::BTreeSet;

fn picked(ids: &[u64]) -> Choice {
    Choice::Picked(ids.iter().copied().collect())
}

#[test]
fn headless_status_wire_names_are_fixed() {
    let names: Vec<&str> =
        [Status::NotDetermined, Status::Restricted, Status::Denied, Status::Authorized, Status::Limited]
            .into_iter()
            .map(|s| s.name())
            .collect();
    assert_eq!(names, ["not_determined", "restricted", "denied", "authorized", "limited"]);
}

#[test]
fn headless_a_default_app_starts_undecided_with_nothing_picked() {
    assert_eq!(Access::default(), Access::new());
    assert_eq!(Access::default().status(), Status::NotDetermined);
    assert!(Access::default().picked().is_empty());
}

#[test]
fn headless_a_full_grant_can_be_taken_back_in_settings() {
    let mut a = Access::new();
    assert_eq!(a.request(Choice::All), Ok(Status::Authorized));
    assert_eq!(a.change_in_settings(Choice::Deny), Ok(Status::Denied));
    assert!(!a.can_read(1));
    assert_eq!(a.check_write(), Err(AccessError::NotAuthorized));
    assert_eq!(a.change_in_settings(picked(&[4])), Ok(Status::Limited));
    assert_eq!(a.visible(&[4, 5]), vec![4]);
}

#[test]
fn headless_a_limited_grant_becomes_full_access_only_in_settings() {
    let mut a = Access::new();
    a.request(picked(&[1])).unwrap();
    assert_eq!(a.request(Choice::All), Err(AccessError::AlreadyDecided));
    assert_eq!(a.change_in_settings(Choice::All), Ok(Status::Authorized));
    assert_eq!(a.check_write(), Ok(()));
    assert!(a.picked().is_empty());
}

#[test]
fn headless_a_new_pick_in_settings_replaces_the_old_one() {
    let mut a = Access::new();
    a.request(picked(&[1, 2])).unwrap();
    assert_eq!(a.change_in_settings(picked(&[9])), Ok(Status::Limited));
    assert_eq!(a.visible(&[1, 2, 9]), vec![9]);
    assert_eq!(a.picked().iter().copied().collect::<Vec<u64>>(), vec![9]);
}

#[test]
fn headless_an_empty_settings_pick_keeps_the_current_grant() {
    let mut a = Access::new();
    a.request(picked(&[3])).unwrap();
    assert_eq!(a.change_in_settings(picked(&[])), Err(AccessError::NothingPicked));
    assert_eq!(a.status(), Status::Limited);
    assert!(a.can_read(3));
}

#[test]
fn headless_a_denied_app_sees_no_contacts() {
    let mut a = Access::new();
    a.request(Choice::Deny).unwrap();
    assert!(a.visible(&[1, 2, 3]).is_empty());
    assert_eq!(a.check_write(), Err(AccessError::NotAuthorized));
}

#[test]
fn headless_visible_keeps_the_order_given_and_drops_unpicked_ids() {
    let mut a = Access::new();
    a.request(picked(&[8, 2, 5])).unwrap();
    assert_eq!(a.visible(&[5, 1, 8, 2, 9]), vec![5, 8, 2]);
    let all: BTreeSet<u64> = a.picked().clone();
    assert_eq!(all.len(), 3);
}

#[test]
fn headless_fetch_keys_need_one_key_and_duplicates_collapse() {
    assert_eq!(Keys::new(&[]), Err(FetchError::NoKeys));
    assert_eq!(Keys::new(&[Key::Phones, Key::Phones]), Keys::new(&[Key::Phones]));
    assert!(Keys::all().contains(Key::Dates));
}

#[test]
fn headless_key_wire_names_are_fixed_and_read_back() {
    let names: Vec<&str> = Key::ALL.iter().map(|k| k.name()).collect();
    assert_eq!(
        names,
        [
            "given_name",
            "family_name",
            "organization",
            "phones",
            "emails",
            "address",
            "note",
            "urls",
            "birthday",
            "dates"
        ]
    );
    assert_eq!(Key::from_name("phones"), Some(Key::Phones));
    assert_eq!(Key::from_name("nickname"), None);
}

fn ada() -> Record {
    let mut r = Record::new(12);
    r.set(Key::GivenName, Value::Text("Ada".into()));
    r.set(
        Key::Phones,
        Value::Labeled(LabeledValue::numbered(vec![
            LabeledValue::new(LABEL_PHONE_MOBILE, "+354 555 0100"),
            LabeledValue::new(LABEL_PHONE_MAIN, "+354 555 0101"),
        ])),
    );
    r.set(Key::Organization, Value::Text("   ".into()));
    r.set(Key::Birthday, Value::Date(DateComponents::new(None, 12, 10).unwrap()));
    r
}

#[test]
fn headless_a_partial_record_holds_only_the_fetched_fields() {
    let keys = Keys::new(&[Key::Phones, Key::GivenName]).unwrap();
    let p = project(&ada(), &keys);
    assert_eq!(p.id, 12);
    assert_eq!(p.fetched().collect::<Vec<Key>>(), vec![Key::GivenName, Key::Phones]);
    assert_eq!(p.text(Key::GivenName), Ok("Ada"));
    assert_eq!(p.get(Key::Birthday), Err(FetchError::NotFetched(Key::Birthday)));
    assert_eq!(p.text(Key::Organization), Err(FetchError::NotFetched(Key::Organization)));
}

#[test]
fn headless_a_fetched_field_the_record_lacks_reads_as_absent() {
    let p = project(&ada(), &Keys::new(&[Key::Organization, Key::Note]).unwrap());
    assert_eq!(p.get(Key::Organization), Ok(None));
    assert_eq!(p.text(Key::Note), Ok(""));
}

#[test]
fn headless_all_keys_fetch_every_stored_field_and_blank_ones_stay_absent() {
    let p = project(&ada(), &Keys::all());
    assert_eq!(p.fetched().count(), Key::ALL.len());
    assert_eq!(p.get(Key::Organization), Ok(None));
    assert_eq!(p.get(Key::Birthday), Ok(Some(&Value::Date(DateComponents::new(None, 12, 10).unwrap()))));
}

#[test]
fn headless_setting_a_blank_value_removes_the_field() {
    let mut r = ada();
    r.set(Key::GivenName, Value::Text("  ".into()));
    assert_eq!(r.get(Key::GivenName), None);
    r.set(Key::Emails, Value::List(vec!["".into(), " ".into()]));
    assert_eq!(r.get(Key::Emails), None);
    r.set(Key::Dates, Value::Dates(vec![]));
    assert_eq!(r.get(Key::Dates), None);
    r.set(Key::Phones, Value::Labeled(vec![LabeledValue::new(LABEL_HOME, "")]));
    assert_eq!(r.get(Key::Phones), None);
}

#[test]
fn headless_labeled_values_number_from_one_and_keep_their_labels() {
    let items = LabeledValue::numbered(vec![
        LabeledValue::new(LABEL_WORK, "ada@example.com"),
        LabeledValue::new("lab", "ada@lab.example"),
        LabeledValue::new(LABEL_OTHER, "ada@other.example"),
    ]);
    let ids: Vec<&str> = items.iter().map(|i| i.identifier.as_str()).collect();
    assert_eq!(ids, ["1", "2", "3"]);
    assert_eq!(items[1].label, "lab");
    assert_eq!(items[2].value, "ada@other.example");
}

#[test]
fn headless_predefined_labels_are_plain_words() {
    assert_eq!(LABEL_HOME, "home");
    assert_eq!(LABEL_WORK, "work");
    assert_eq!(LABEL_OTHER, "other");
    assert_eq!(LABEL_PHONE_MOBILE, "mobile");
    assert_eq!(LABEL_PHONE_MAIN, "main");
}

#[test]
fn headless_date_components_reject_out_of_range_parts() {
    assert_eq!(DateComponents::new(Some(2026), 0, 1), None);
    assert_eq!(DateComponents::new(Some(2026), 13, 1), None);
    assert_eq!(DateComponents::new(Some(2026), 1, 0), None);
    assert_eq!(DateComponents::new(Some(2026), 1, 32), None);
    assert!(DateComponents::new(Some(2026), 1, 1).is_some());
}

#[test]
fn headless_date_text_round_trips_with_and_without_a_year() {
    let dated = DateComponents::new(Some(1815), 12, 10).unwrap();
    assert_eq!(dated.to_text(), "1815-12-10");
    assert_eq!(DateComponents::from_text("1815-12-10"), Some(dated));
    let yearless = DateComponents::new(None, 3, 7).unwrap();
    assert_eq!(yearless.to_text(), "--03-07");
    assert_eq!(DateComponents::from_text("--03-07"), Some(yearless));
    assert_eq!(DateComponents::from_text("1815-13-10"), None);
    assert_eq!(DateComponents::from_text("soon"), None);
}

#[test]
fn headless_access_and_fetch_errors_are_distinct_typed_variants() {
    assert_ne!(AccessError::ReadOnly, AccessError::NotAuthorized);
    assert_ne!(AccessError::NothingPicked, AccessError::AlreadyDecided);
    assert_eq!(FetchError::NotFetched(Key::Note), FetchError::NotFetched(Key::Note));
    assert_ne!(FetchError::NotFetched(Key::Note), FetchError::NotFetched(Key::Urls));
}
