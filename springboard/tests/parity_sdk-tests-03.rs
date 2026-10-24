//! Parity sdk-tests-03: headless tests of the accessibility vocabulary, the contacts store and its sidecar, the contacts
//! service and the calendar file. Each check runs on plain values or on files under the crate's target directory. No
//! test opens a window, a bus or a device.

use bifrost_core::json::Json;
use bifrost_proto::Role;
use bifrost_ui::ax::Element;
use bifrost_ui::contact_access::{Choice, Status};
use bifrost_ui::contact_fetch::{DateComponents, Key, LabeledValue, Value};
use springboard::shell::accessibility::walk::{in_rotor, rotor_count, rotor_name, rotor_role, speech, spoken_kind};
use springboard::shell::atspi::model::atspi_role;
use springboard::shell::calendar::store::{self as calendar_store, CalendarEvent};
use springboard::shell::calendar::Date;
use springboard::shell::contactextras::{self, Book, Extras};
use springboard::shell::contactstore::{self, Contact, LABELS, MAX_NAME, MAX_NOTES};
use springboard::shell::services::contacts::{
    access_of, answer, answer_prompt, answer_with, apply_save, apply_save_with, change_by_user, reads, record_of, record_with,
};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// A scratch file for one test, in a directory of its own under the crate's target directory.
fn scratch(test: &str, file: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/parity-sdk-tests-03").join(test);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(file)
}

fn contact(id: u64, name: &str, phone: &str, email: &str) -> Contact {
    Contact { id, name: name.into(), phone: phone.into(), email: email.into(), ..Default::default() }
}

fn obj(fields: Vec<(&str, Json)>) -> Json {
    Json::Obj(fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

fn text(value: &str) -> Json {
    Json::Str(value.into())
}

fn texts(values: &[&str]) -> Json {
    Json::Arr(values.iter().map(|v| text(v)).collect())
}

fn keys(names: &[&str]) -> Json {
    obj(vec![("keys", texts(names))])
}

fn element(role: &str, name: &str, value: &str, enabled: bool) -> Element {
    Element {
        address: format!("app/test/{name}"),
        name: name.into(),
        role: role.into(),
        value: value.into(),
        enabled,
        bounds: None,
        handle: String::new(),
    }
}

// --- accessibility: element role mapping ---------------------------------------------------------------------------------

#[test]
fn each_element_role_maps_to_its_own_atspi_role_number() {
    let roles = [
        Role::Button,
        Role::TextField,
        Role::List,
        Role::Row,
        Role::Toggle,
        Role::Checkbox,
        Role::Slider,
        Role::SegmentedControl,
        Role::StaticText,
        Role::MenuItem,
        Role::Window,
        Role::ProgressIndicator,
    ];
    let numbers: BTreeSet<u32> = roles.into_iter().map(atspi_role).collect();
    assert_eq!(numbers.len(), 12, "no two element roles share an AT-SPI role");
    assert!(numbers.iter().all(|n| *n > 0));
}

#[test]
fn a_spoken_kind_names_the_control_and_an_unknown_role_is_spoken_bare() {
    assert_eq!(spoken_kind("toggle"), "switch");
    assert_eq!(spoken_kind("page"), "button");
    assert_eq!(spoken_kind("item"), "menu item");
    assert_eq!(spoken_kind("segmented-control"), "segmented control");
    assert_eq!(spoken_kind("progress-indicator"), "progress indicator");
    assert_eq!(spoken_kind("text"), "", "a text row has no kind word");
    assert_eq!(spoken_kind("no-such-role"), "");
}

#[test]
fn a_walked_element_is_spoken_as_kind_name_value_and_unavailable() {
    assert_eq!(speech(&element("button", "Close", "", true)), "button, Close");
    assert_eq!(speech(&element("toggle", "Bold Text", "on", true)), "switch, Bold Text, on");
    assert_eq!(speech(&element("button", "Close", "", false)), "button, Close, unavailable");
    assert_eq!(speech(&element("text", "Message", "Hello", true)), "Message, Hello");
}

#[test]
fn settings_roles_are_matched_by_the_standard_rotor_roles() {
    assert_eq!(rotor_role("page"), "button");
    assert_eq!(rotor_role("choice"), "row");
    assert_eq!(rotor_role("item"), "menu-item");
    assert_eq!(rotor_role("menu"), "menu-item");
    assert_eq!(rotor_role("field"), "field", "a role without a mapping keeps its own word");
}

#[test]
fn the_rotors_are_all_then_the_six_standard_ones_and_their_indices_wrap() {
    assert_eq!(rotor_count(), 7);
    let names: Vec<&str> = (0..rotor_count()).map(rotor_name).collect();
    assert_eq!(names, ["All", "Controls", "Fields", "Lists", "Menu items", "Text", "Progress"]);
    assert_eq!(rotor_name(rotor_count()), "All", "an index past the last rotor wraps to All");
}

#[test]
fn a_rotor_walks_its_own_roles_and_all_walks_everything() {
    let button = element("button", "Save", "", true);
    let field = element("field", "Name", "Ada", true);
    let choice = element("choice", "Mobile", "selected", true);
    let menu = element("menu", "File", "", true);
    let segmented = element("segmented-control", "View", "", true);
    let text_row = element("text", "Note", "Hi", true);

    assert!(in_rotor(&button, 0) && in_rotor(&text_row, 0), "rotor 0 walks every element");
    assert!(in_rotor(&button, 1) && in_rotor(&segmented, 1), "Controls walks buttons and segmented controls");
    assert!(!in_rotor(&field, 1), "Controls does not walk fields");
    assert!(in_rotor(&field, 2), "Fields walks text fields");
    assert!(in_rotor(&choice, 3), "Lists walks a choice row, which maps to a list row");
    assert!(in_rotor(&menu, 4), "Menu items walks a menu, which maps to a menu item");
    assert!(in_rotor(&text_row, 5), "Text walks text rows");
    assert!(!in_rotor(&button, 6), "Progress walks progress indicators only");
}

// --- contacts store: the data model -----------------------------------------------------------------------------------

#[test]
fn a_contact_is_named_by_its_first_filled_identifying_field() {
    let company = Contact { company: "Acme".into(), phone: "555".into(), ..Default::default() };
    assert_eq!(company.display_name(), "Acme", "a company comes before a phone number");
    let phone_only = Contact { phone: "+354 555".into(), email: "a@x.is".into(), ..Default::default() };
    assert_eq!(phone_only.display_name(), "+354 555");
    let blank_name = Contact { name: "   ".into(), email: "a@x.is".into(), ..Default::default() };
    assert_eq!(blank_name.display_name(), "a@x.is", "a blank name is skipped");
    assert_eq!(Contact::default().display_name(), "No Name");
}

#[test]
fn initials_come_from_the_first_and_last_word_of_the_name() {
    assert_eq!(contact(1, "Ada Byron Lovelace", "", "").initials(), "AL");
    assert_eq!(contact(2, "grace", "", "").initials(), "G");
    assert_eq!(contact(3, "", "", "").initials(), "?", "no name and no company gives a question mark");
    let company_only = Contact { company: "acme corp".into(), ..Default::default() };
    assert_eq!(company_only.initials(), "AC", "without a name the company stands in");
}

#[test]
fn a_contact_needs_one_identifying_field_to_be_valid() {
    assert!(!Contact::default().valid());
    assert!(!contact(1, "   ", "  ", "").valid(), "blank text is not an identifying field");
    let company_only = Contact { company: "Acme".into(), ..Default::default() };
    assert!(company_only.valid());
    assert!(contact(1, "Ada", "", "").valid());
}

#[test]
fn label_indexes_must_name_a_label_of_their_row() {
    assert_eq!(LABELS[0][0], "mobile");
    assert_eq!(LABELS.map(|set| set.len()), [4, 3, 3]);
    let last = Contact { labels: [3, 2, 2], ..contact(1, "Ada", "", "") };
    assert!(last.valid(), "the last label of each row is in range");
    let past_phone = Contact { labels: [4, 0, 0], ..contact(1, "Ada", "", "") };
    assert!(!past_phone.valid(), "a phone label index past the phone labels is refused");
    let past_email = Contact { labels: [0, 3, 0], ..contact(1, "Ada", "", "") };
    assert!(!past_email.valid());
}

#[test]
fn notes_are_limited_to_the_note_length_and_keep_line_breaks() {
    assert_eq!(MAX_NOTES, 4000);
    let at_limit = Contact { notes: "n".repeat(MAX_NOTES), ..contact(1, "Ada", "", "") };
    assert!(at_limit.valid(), "a note of exactly the limit is valid");
    let over = Contact { notes: "n".repeat(MAX_NOTES + 1), ..contact(1, "Ada", "", "") };
    assert!(!over.valid(), "a note one character over the limit is refused");
    let lines = Contact { notes: "line one\nline two\tcolumn".into(), ..contact(1, "Ada", "", "") };
    assert!(lines.valid(), "line breaks and tabs are allowed in notes");
    let bell = Contact { notes: "ring\u{7}".into(), ..contact(1, "Ada", "", "") };
    assert!(!bell.valid(), "other control characters are refused");
}

#[test]
fn a_name_is_limited_to_the_name_length() {
    let longest = Contact { name: "n".repeat(MAX_NAME), ..Default::default() };
    assert!(longest.valid());
    let too_long = Contact { name: "n".repeat(MAX_NAME + 1), ..Default::default() };
    assert!(!too_long.valid());
}

#[test]
fn contacts_sort_by_display_name_ignoring_case_then_by_id() {
    let mut book = vec![contact(3, "bob", "", ""), contact(1, "Alice", "", ""), contact(2, "BOB", "", "")];
    contactstore::sort(&mut book);
    let order: Vec<u64> = book.iter().map(|c| c.id).collect();
    assert_eq!(order, vec![1, 2, 3]);
}

#[test]
fn search_matches_any_text_field_without_regard_to_case() {
    let book = vec![
        Contact { name: "Ada".into(), company: "Analytical Engines".into(), ..Default::default() },
        Contact { name: "Grace".into(), address: "12 Harbour Road".into(), ..Default::default() },
        Contact { name: "Alan".into(), notes: "Prefers mornings".into(), ..Default::default() },
    ];
    assert_eq!(contactstore::search(&book, "ENGINES"), vec![0]);
    assert_eq!(contactstore::search(&book, "harbour road"), vec![1]);
    assert_eq!(contactstore::search(&book, " mornings "), vec![2], "the query is trimmed");
    assert_eq!(contactstore::search(&book, "zzz"), Vec::<usize>::new());
    assert_eq!(contactstore::search(&book, ""), vec![0, 1, 2], "a blank query lists every contact");
}

#[test]
fn new_ids_start_at_one_and_fill_the_first_gap() {
    assert_eq!(contactstore::new_id(&[]), Some(1));
    let book = vec![contact(1, "A", "", ""), contact(2, "B", "", ""), contact(4, "D", "", "")];
    assert_eq!(contactstore::new_id(&book), Some(3));
}

#[test]
fn the_contacts_file_sits_in_the_contacts_app_data_folder() {
    let path = contactstore::store_path(Path::new("/data"));
    assert_eq!(path, Path::new("/data/appdata/is.olibuijr.contacts/Contacts/contacts.tsv"));
}

// --- contacts store: access to the file -------------------------------------------------------------------------------

#[test]
fn a_missing_contacts_file_loads_as_an_empty_book() {
    let path = scratch("missing_file", "contacts.tsv");
    assert_eq!(contactstore::load_checked(&path).unwrap(), Vec::<Contact>::new());
    assert!(contactstore::load(&path).is_empty());
}

#[test]
fn a_saved_book_loads_back_with_its_labels_flags_and_company() {
    let path = scratch("round_trip", "contacts.tsv");
    let ada = Contact {
        id: 7,
        name: "Ada Byron Lovelace".into(),
        phone: "+354 555 0100".into(),
        email: "ada@example.com".into(),
        notes: "Analyst\nFirst programmer".into(),
        favorite: true,
        company: "Analytical Engines".into(),
        address: "1 Mill Lane".into(),
        uid: String::new(),
        labels: [2, 1, 2],
    };
    let grace = contact(9, "Grace Hopper", "", "grace@example.com");
    contactstore::save(&path, &[ada.clone(), grace.clone()]).unwrap();
    assert_eq!(contactstore::load_checked(&path).unwrap(), vec![ada, grace]);
}

#[test]
fn the_saved_file_starts_with_the_contacts_format_header() {
    let path = scratch("header", "contacts.tsv");
    contactstore::save(&path, &[contact(1, "Ada", "", "")]).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert_eq!(text.lines().next(), Some("springboard-contacts-v1"));
}

#[test]
fn a_book_with_two_contacts_of_one_id_is_not_saved() {
    let path = scratch("duplicate_ids", "contacts.tsv");
    let error = contactstore::save(&path, &[contact(1, "Ada", "", ""), contact(1, "Grace", "", "")]).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    assert!(!path.exists(), "a refused save writes nothing");
}

#[test]
fn a_file_with_another_header_is_refused_and_loads_as_empty() {
    let path = scratch("foreign", "contacts.tsv");
    std::fs::write(&path, "springboard-calendar-v1\n1\tAda\t\t\t\n").unwrap();
    let error = contactstore::load_checked(&path).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    assert!(contactstore::load(&path).is_empty());
}

#[test]
fn rows_with_the_wrong_column_count_or_a_bad_id_are_skipped() {
    let text = "springboard-contacts-v1\n1\tAda\t\t\t\n2\tToo\tmany\t\t\t0\t\t\t0\t0\t0\t9\t\t\n3\tOk\t\t\t\nx\tBad id\t\t\t\n";
    let book = contactstore::parse(text);
    let ids: Vec<u64> = book.iter().map(|c| c.id).collect();
    assert_eq!(ids, vec![1, 3]);
}

#[test]
fn an_older_file_with_five_columns_loads_with_the_newer_fields_empty() {
    let book = contactstore::parse("springboard-contacts-v1\n4\tAnna\t1\t\t\n");
    assert_eq!(book.len(), 1);
    assert_eq!(book[0].phone, "1");
    assert!(book[0].company.is_empty() && book[0].address.is_empty());
    assert_eq!(book[0].labels, [0, 0, 0]);
}

// --- contacts sidecar: multi-value fields -----------------------------------------------------------------------------

#[test]
fn the_sidecar_is_written_beside_the_contacts_file_and_loads_back() {
    let contacts_file = scratch("sidecar_round_trip", "contacts.tsv");
    let sidecar = contactextras::path_for(&contacts_file);
    let mut book = Book::new();
    book.insert(
        4,
        Extras {
            phones: vec![LabeledValue::new("work", "+354 555 0101"), LabeledValue::new("home", "+354 555 0102")],
            emails: vec![LabeledValue::new("other", "b@x.is")],
            urls: vec![LabeledValue::new("work", "https://x.is/a")],
            birthday: DateComponents::new(None, 12, 10),
            dates: vec![("anniversary".into(), DateComponents::new(Some(2001), 5, 6).unwrap())],
            ..Default::default()
        },
    );
    contactextras::save(&sidecar, &book).unwrap();
    assert_eq!(sidecar.file_name().unwrap(), "contacts-extra.tsv");
    assert_eq!(contactextras::load(&sidecar), book);
}

#[test]
fn a_missing_sidecar_is_an_empty_book() {
    let sidecar = scratch("sidecar_missing", "contacts-extra.tsv");
    assert!(contactextras::load(&sidecar).is_empty());
}

#[test]
fn the_sidecar_header_and_a_missing_header_are_checked() {
    let mut book = Book::new();
    book.insert(1, Extras { emails: vec![LabeledValue::new("home", "a@x.is")], ..Default::default() });
    let text = contactextras::render(&book);
    assert_eq!(text.lines().next(), Some("springboard-contact-extras-v1"));
    assert!(contactextras::parse("1\temail\thome\ta@x.is\n").is_empty(), "text without the header is ignored");
}

#[test]
fn a_bad_sidecar_value_leaves_its_field_empty_and_keeps_the_contact() {
    let book = contactextras::parse("springboard-contact-extras-v1\n2\tbirthday\t\t--02-30x\n2\tphone\tmobile\t+354 1\n");
    assert_eq!(book[&2].birthday, None);
    assert_eq!(book[&2].phones.len(), 1);
    let unknown = contactextras::parse("springboard-contact-extras-v1\n5\tshoe_size\t\t42\n");
    assert_eq!(unknown[&5], Extras::default(), "an unknown kind adds nothing");
}

// --- contacts service: access, reads and the record a contact is seen as --------------------------------------------

#[test]
fn a_new_app_has_no_access_and_reads_nothing() {
    let id = "is.example.sdk-tests-03.new";
    assert_eq!(access_of(id).status(), Status::NotDetermined);
    assert!(!reads(id));
    let reply = answer(id, "access", &Json::Null, &[]).unwrap();
    assert_eq!(reply.get("status").and_then(Json::as_str), Some("not_determined"));
    assert_eq!(answer(id, "fetch", &keys(&["given_name"]), &[]).err(), Some("contacts access is not granted".to_string()));
}

#[test]
fn a_request_reports_the_status_and_changes_nothing() {
    let id = "is.example.sdk-tests-03.request";
    let reply = answer(id, "request", &Json::Null, &[]).unwrap();
    assert_eq!(reply.get("status").and_then(Json::as_str), Some("not_determined"));
    assert_eq!(access_of(id).status(), Status::NotDetermined, "a request never grants access");
}

#[test]
fn a_refused_app_reads_and_writes_nothing_until_the_person_changes_it_in_settings() {
    let id = "is.example.sdk-tests-03.refused";
    let book = vec![contact(1, "Ada Lovelace", "", "")];
    assert_eq!(answer_prompt(id, Choice::Deny), Ok(Status::Denied));
    assert!(!reads(id));
    assert!(answer(id, "fetch", &keys(&["given_name"]), &book).is_err());
    assert_eq!(apply_save(id, &obj(vec![("add", Json::Arr(vec![obj(vec![("given_name", text("Grace"))])]))]), &book).err(), Some("contacts write access is not granted".to_string()));
    assert_eq!(answer_prompt(id, Choice::All), Err(bifrost_ui::contact_access::Error::AlreadyDecided), "the prompt is shown once");
    assert_eq!(change_by_user(id, Choice::All), Ok(Status::Authorized), "a Settings change can grant access later");
}

#[test]
fn a_limited_grant_reads_only_the_picked_contacts_and_cannot_save() {
    let id = "is.example.sdk-tests-03.limited";
    let book = vec![contact(1, "Ada Lovelace", "", ""), contact(2, "Grace Hopper", "", ""), contact(3, "Alan Turing", "", "")];
    assert_eq!(answer_prompt(id, Choice::Picked([2u64, 3].into_iter().collect())), Ok(Status::Limited));
    assert!(reads(id));
    assert!(access_of(id).can_read(3) && !access_of(id).can_read(1));
    let reply = answer(id, "fetch", &keys(&["family_name"]), &book).unwrap();
    let ids: Vec<u64> = reply.get("contacts").and_then(Json::as_array).unwrap().iter().filter_map(|r| r.get("id")?.as_u64()).collect();
    assert_eq!(ids, vec![3, 2], "the picked contacts come back sorted by name");
    let refused = apply_save(id, &obj(vec![("delete", Json::Arr(vec![obj(vec![("id", Json::Num(2.0))])]))]), &book);
    assert_eq!(refused.err(), Some("a limited contacts grant only reads; saving needs full access".to_string()));
}

#[test]
fn full_access_fetches_the_named_fields_only_and_blank_values_are_null() {
    let id = "is.example.sdk-tests-03.full";
    let book = vec![contact(1, "Ada Lovelace", "", "ada@example.com"), contact(2, "Alan Turing", "", "")];
    assert_eq!(answer_prompt(id, Choice::All), Ok(Status::Authorized));
    let reply = answer(id, "fetch", &keys(&["family_name", "emails"]), &book).unwrap();
    let rows = reply.get("contacts").and_then(Json::as_array).unwrap();
    assert!(rows[0].get("given_name").is_none(), "a key that was not named is not returned");
    assert_eq!(rows[1].get("emails"), Some(&Json::Null));
    assert_eq!(rows[0].get("emails").and_then(Json::as_array).map(|values| values.len()), Some(1));
}

#[test]
fn a_fetch_needs_known_keys_and_a_known_method() {
    let id = "is.example.sdk-tests-03.keys";
    answer_prompt(id, Choice::All).unwrap();
    assert!(answer(id, "fetch", &keys(&[]), &[]).is_err(), "an empty key list is an error");
    assert_eq!(answer(id, "fetch", &keys(&["nickname"]), &[]).err(), Some("no contact field nickname".to_string()));
    assert_eq!(answer(id, "sing", &Json::Null, &[]).err(), Some("contacts has no method sing".to_string()));
    assert!(answer(id, "save", &Json::Null, &[]).is_err(), "a save goes through the shell that stores the book");
}

#[test]
fn a_query_filters_the_readable_contacts_by_any_text() {
    let id = "is.example.sdk-tests-03.query";
    answer_prompt(id, Choice::All).unwrap();
    let book = vec![contact(1, "Ada Lovelace", "", ""), contact(2, "Alan Turing", "", "")];
    let params = obj(vec![("keys", texts(&["family_name"])), ("query", text("turing"))]);
    let reply = answer(id, "fetch", &params, &book).unwrap();
    let rows = reply.get("contacts").and_then(Json::as_array).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].get("family_name").and_then(Json::as_str), Some("Turing"));
}

#[test]
fn a_contact_name_is_seen_as_given_and_family_name_split_on_the_last_space() {
    let r = record_of(&contact(1, "Ada Byron Lovelace", "", ""));
    assert_eq!(r.get(Key::GivenName), Some(&Value::Text("Ada".into())));
    assert_eq!(r.get(Key::MiddleName), Some(&Value::Text("Byron".into())));
    assert_eq!(r.get(Key::FamilyName), Some(&Value::Text("Lovelace".into())));
    let single = record_of(&contact(2, "Ada", "", ""));
    assert_eq!(single.get(Key::GivenName), Some(&Value::Text("Ada".into())));
    assert_eq!(single.get(Key::FamilyName), None);
}

#[test]
fn the_file_phone_comes_first_and_the_sidecar_phones_follow_with_their_labels() {
    let c = Contact { phone: "+354 555".into(), labels: [2, 0, 0], ..contact(1, "Ada", "", "") };
    let extras = Extras { phones: vec![LabeledValue::new("home", "+354 556")], ..Default::default() };
    let r = record_with(&c, &extras);
    let Some(Value::Labeled(items)) = r.get(Key::Phones) else { panic!("phones are labeled") };
    let shape: Vec<(&str, &str, &str)> = items.iter().map(|i| (i.label.as_str(), i.value.as_str(), i.identifier.as_str())).collect();
    assert_eq!(shape, vec![("work", "+354 555", "1"), ("home", "+354 556", "2")]);
}

#[test]
fn the_sidecar_birthday_and_dates_reach_the_record() {
    let extras = Extras {
        birthday: DateComponents::new(None, 12, 10),
        dates: vec![("anniversary".into(), DateComponents::new(Some(2001), 5, 6).unwrap())],
        ..Default::default()
    };
    let r = record_with(&contact(1, "Ada", "", ""), &extras);
    assert_eq!(r.get(Key::Birthday), Some(&Value::Date(DateComponents::new(None, 12, 10).unwrap())));
    assert_eq!(r.get(Key::Dates), Some(&Value::Dates(vec![("anniversary".into(), DateComponents::new(Some(2001), 5, 6).unwrap())])));
}

#[test]
fn a_fetch_returns_labeled_entries_with_identifiers_and_a_birthday_without_a_year() {
    let id = "is.example.sdk-tests-03.labeled";
    answer_prompt(id, Choice::All).unwrap();
    let book = vec![contact(1, "Ada Lovelace", "+354 1", "ada@x.is"), contact(2, "Alan Turing", "", "")];
    let mut sidecar = Book::new();
    sidecar.insert(1, Extras { emails: vec![LabeledValue::new("work", "b@x.is")], birthday: DateComponents::new(None, 12, 10), ..Default::default() });
    let params = keys(&["phones", "emails", "birthday"]);
    let reply = answer_with(id, "fetch", &params, &book, &sidecar).unwrap();
    let rows = reply.get("contacts").and_then(Json::as_array).unwrap();
    let phones = rows[0].get("phones").and_then(Json::as_array).unwrap();
    assert_eq!(phones, &[obj(vec![("label", text("mobile")), ("value", text("+354 1")), ("identifier", text("1"))])]);
    let emails = rows[0].get("emails").and_then(Json::as_array).unwrap();
    assert_eq!(emails[1].get("identifier"), Some(&text("2")));
    assert_eq!(emails[1].get("label"), Some(&text("work")));
    assert_eq!(rows[0].get("birthday"), Some(&obj(vec![("month", Json::Num(12.0)), ("day", Json::Num(10.0))])));
    assert_eq!(rows[1].get("birthday"), Some(&Json::Null), "a contact without a sidecar birthday reads as null");
}

// --- contacts service: save requests -----------------------------------------------------------------------------------

#[test]
fn an_add_gets_the_next_free_id_and_splits_its_name() {
    let id = "is.example.sdk-tests-03.add";
    answer_prompt(id, Choice::All).unwrap();
    let book = vec![contact(1, "Ada Lovelace", "", "")];
    let params = obj(vec![(
        "add",
        Json::Arr(vec![obj(vec![
            ("given_name", text("Grace")),
            ("family_name", text("Hopper")),
            ("phones", texts(&["+1 555"])),
            ("emails", texts(&["g@x.is"])),
        ])]),
    )]);
    let (reply, next) = apply_save(id, &params, &book).unwrap();
    assert_eq!(reply.get("added"), Some(&Json::Arr(vec![Json::Num(2.0)])));
    assert_eq!(next.len(), 2);
    // the stable uid is made when the card is added
    assert!(next[1].uid.ends_with("@bifrost"), "a new card gets a uid");
    assert_eq!(next[1], Contact { id: 2, name: "Grace Hopper".into(), phone: "+1 555".into(), email: "g@x.is".into(), uid: next[1].uid.clone(), ..Default::default() });
}

#[test]
fn an_update_changes_the_named_fields_and_keeps_the_flags_labels_and_the_rest() {
    let id = "is.example.sdk-tests-03.update";
    answer_prompt(id, Choice::All).unwrap();
    let original = Contact { favorite: true, labels: [2, 1, 2], company: "Acme".into(), phone: "+354 1".into(), ..contact(1, "Ada Lovelace", "", "") };
    let params = obj(vec![("update", Json::Arr(vec![obj(vec![("id", Json::Num(1.0)), ("family_name", text("Byron"))])]))]);
    let (_, next) = apply_save(id, &params, std::slice::from_ref(&original)).unwrap();
    assert_eq!(next[0].name, "Ada Byron");
    assert!(next[0].favorite);
    assert_eq!(next[0].labels, [2, 1, 2]);
    assert_eq!((next[0].company.as_str(), next[0].phone.as_str()), ("Acme", "+354 1"));
}

#[test]
fn a_null_clears_one_field_and_an_update_of_an_unknown_contact_is_refused() {
    let id = "is.example.sdk-tests-03.clear";
    answer_prompt(id, Choice::All).unwrap();
    let book = vec![contact(1, "Ada", "+354 1", "ada@x.is")];
    let clear = obj(vec![("update", Json::Arr(vec![obj(vec![("id", Json::Num(1.0)), ("emails", Json::Null)])]))]);
    let (_, next) = apply_save(id, &clear, &book).unwrap();
    assert_eq!(next[0].email, "");
    assert_eq!(next[0].phone, "+354 1");
    let unknown = obj(vec![("update", Json::Arr(vec![obj(vec![("id", Json::Num(9.0)), ("family_name", text("X"))])]))]);
    assert_eq!(apply_save(id, &unknown, &book).err(), Some("no contact 9".to_string()));
}

#[test]
fn a_save_applies_all_its_changes_or_none_of_them() {
    let id = "is.example.sdk-tests-03.atomic";
    answer_prompt(id, Choice::All).unwrap();
    let book = vec![contact(1, "Ada", "", "ada@x.is")];
    let params = obj(vec![
        ("add", Json::Arr(vec![obj(vec![("given_name", text("Grace"))])])),
        ("delete", Json::Arr(vec![obj(vec![("id", Json::Num(9.0))])])),
    ]);
    assert_eq!(apply_save(id, &params, &book).err(), Some("no contact 9".to_string()));
    assert_eq!(book.len(), 1, "the caller keeps the old book when a save fails");
}

#[test]
fn a_save_keeps_the_first_phone_in_the_file_and_the_rest_in_the_sidecar_but_refuses_two_addresses() {
    let id = "is.example.sdk-tests-03.two";
    answer_prompt(id, Choice::All).unwrap();
    let params = obj(vec![("add", Json::Arr(vec![obj(vec![("given_name", text("Ada")), ("phones", texts(&["1", "2"]))])]))]);
    let (_, next, extras) = apply_save_with(id, &params, &[], &Book::new()).unwrap();
    assert_eq!(next[0].phone, "1");
    assert_eq!(extras[&next[0].id].phones, vec![LabeledValue::new("", "2")]);
    let two = obj(vec![("add", Json::Arr(vec![obj(vec![("given_name", text("Ada")), ("address", texts(&["a", "b"]))])]))]);
    assert_eq!(apply_save(id, &two, &[]).err(), Some("address takes one value per contact".to_string()));
}

#[test]
fn an_empty_save_and_an_unknown_field_are_refused() {
    let id = "is.example.sdk-tests-03.empty";
    answer_prompt(id, Choice::All).unwrap();
    assert_eq!(apply_save(id, &obj(vec![]), &[]).err(), Some("save needs add, update or delete".to_string()));
    let bad = obj(vec![("add", Json::Arr(vec![obj(vec![("shoe_size", text("42"))])]))]);
    assert_eq!(apply_save(id, &bad, &[]).err(), Some("no contact field shoe_size".to_string()));
}

#[test]
fn a_new_app_cannot_save_contacts_until_the_person_grants_full_access() {
    let fresh = "is.example.sdk-tests-03.not-granted";
    let params = obj(vec![("add", Json::Arr(vec![obj(vec![("given_name", text("Ada"))])]))]);
    assert_eq!(apply_save(fresh, &params, &[]).err(), Some("contacts write access is not granted".to_string()));
}

// --- calendar file: the versioned format the core-data row names --------------------------------------------------------

fn event(id: u64, minute: u16, title: &str) -> CalendarEvent {
    CalendarEvent {
        id,
        date: Date { year: 2026, month: 10, day: 14 },
        minute,
        duration: 60,
        all_day: false,
        title: title.into(),
        location: String::new(),
        notes: String::new(),
        repeat: 0,
        alert: 0, uid: String::new(),
    }
}

#[test]
fn a_calendar_saves_and_loads_its_events_in_time_order() {
    let path = scratch("calendar_round_trip", "calendar.tsv");
    calendar_store::save(&path, &[event(2, 600, "Lunch"), event(1, 540, "Standup")]).unwrap();
    let loaded = calendar_store::load(&path).unwrap();
    let titles: Vec<&str> = loaded.iter().map(|e| e.title.as_str()).collect();
    assert_eq!(titles, vec!["Standup", "Lunch"]);
    assert_eq!(loaded[0], event(1, 540, "Standup"));
}

#[test]
fn the_calendar_file_starts_with_its_format_header_and_a_foreign_one_is_refused() {
    let path = scratch("calendar_header", "calendar.tsv");
    calendar_store::save(&path, &[event(1, 540, "Standup")]).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap().lines().next(), Some("springboard-calendar-v1"));
    std::fs::write(&path, "springboard-contacts-v1\n").unwrap();
    assert_eq!(calendar_store::load(&path).unwrap_err().kind(), std::io::ErrorKind::InvalidData);
}

#[test]
fn a_missing_calendar_file_is_an_empty_calendar() {
    let path = scratch("calendar_missing", "calendar.tsv");
    assert_eq!(calendar_store::load(&path).unwrap(), Vec::new());
}

#[test]
fn a_calendar_save_refuses_duplicate_ids_and_events_shorter_than_fifteen_minutes() {
    let path = scratch("calendar_refused", "calendar.tsv");
    let twice = calendar_store::save(&path, &[event(1, 540, "A"), event(1, 600, "B")]).unwrap_err();
    assert_eq!(twice.kind(), std::io::ErrorKind::InvalidInput);
    let short = CalendarEvent { duration: 10, ..event(3, 540, "Short") };
    assert_eq!(calendar_store::save(&path, &[short]).unwrap_err().kind(), std::io::ErrorKind::InvalidInput);
    assert!(!path.exists(), "a refused save writes nothing");
}

#[test]
fn a_damaged_calendar_row_is_skipped_and_the_rest_of_the_calendar_loads() {
    let path = scratch("calendar_damaged", "calendar.tsv");
    calendar_store::save(&path, &[event(1, 540, "Standup")]).unwrap();
    let mut text = std::fs::read_to_string(&path).unwrap();
    text.push_str("2\t2026\t10\t14\t600\t60\t2\t4c756e6368\n");
    std::fs::write(&path, text).unwrap();
    let ids: Vec<u64> = calendar_store::load(&path).unwrap().iter().map(|e| e.id).collect();
    assert_eq!(ids, vec![1], "a row whose all-day flag is not 0 or 1 is dropped");
}

#[test]
fn a_new_calendar_id_follows_the_largest_id_in_use() {
    assert_eq!(calendar_store::next_id(&[]), 1);
    assert_eq!(calendar_store::next_id(&[event(4, 540, "A"), event(9, 600, "B")]), 10);
}
