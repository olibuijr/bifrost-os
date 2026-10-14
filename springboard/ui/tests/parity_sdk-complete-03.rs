//! Parity sdk-complete-03: headless tests for contact search over directory records and for the composition rules of
//! the input method model. Nothing here opens a directory server, a socket or a display.

use bifrost_ui::directory_contacts::{contact_filter, escape_filter_value, search_contacts, Contact};
use bifrost_ui::directory_query::DirRecord;
use bifrost_ui::input_method::{Event, Outcome, Registration, Server};
use bifrost_ui::keys::{Key, KeyCombo};

fn person(name: &str, cn: &str, mails: &[&str], phones: &[&str]) -> DirRecord {
    let mut r = DirRecord::new("users", name);
    r.set_values("cn", vec![cn.to_string()]).unwrap();
    r.set_values("mail", mails.iter().map(|m| m.to_string()).collect()).unwrap();
    r.set_values("telephoneNumber", phones.iter().map(|p| p.to_string()).collect()).unwrap();
    r
}

fn directory() -> Vec<DirRecord> {
    let mut staff = DirRecord::new("groups", "staff");
    staff.set_values("cn", vec!["Lind staff".to_string()]).unwrap();
    vec![
        person("anna", "Anna Lind", &["anna@example.is"], &["+354 555 0101"]),
        person("bjarni", "Bjarni Olafsson", &["bjarni@example.is"], &[]),
        person("carla", "Carla Lind", &["carla@example.com", "carla.lind@example.com"], &["+354 555 0303"]),
        staff,
    ]
}

fn refs(db: &[DirRecord]) -> Vec<&DirRecord> {
    db.iter().collect()
}

fn names(contacts: &[Contact]) -> Vec<String> {
    contacts.iter().map(|c| c.name.clone()).collect()
}

#[test]
fn escaping_replaces_the_filter_specials_with_hex_escapes() {
    assert_eq!(escape_filter_value("a*b(c)d\\e"), "a\\2ab\\28c\\29d\\5ce");
    assert_eq!(escape_filter_value("x\0y"), "x\\00y");
    assert_eq!(escape_filter_value("Ásdís Ólafs"), "Ásdís Ólafs", "ordinary text, including letters outside ASCII, is kept");
}

#[test]
fn a_blank_contact_query_builds_no_filter_and_finds_nothing() {
    assert_eq!(contact_filter("   "), None);
    assert_eq!(contact_filter(""), None);
    let db = directory();
    assert!(search_contacts(&refs(&db), "  ", 10).is_empty());
}

#[test]
fn the_contact_filter_matches_each_contact_attribute_as_a_substring() {
    assert_eq!(
        contact_filter(" Lind ").as_deref(),
        Some("(&(objectClass=person)(|(cn=*Lind*)(givenName=*Lind*)(sn=*Lind*)(mail=*Lind*)(telephoneNumber=*Lind*)))"),
        "the text is trimmed and every contact attribute is an OR term"
    );
}

#[test]
fn the_contact_filter_escapes_user_text_so_it_cannot_add_terms() {
    assert_eq!(
        contact_filter("a*(b").as_deref(),
        Some("(&(objectClass=person)(|(cn=*a\\2a\\28b*)(givenName=*a\\2a\\28b*)(sn=*a\\2a\\28b*)(mail=*a\\2a\\28b*)(telephoneNumber=*a\\2a\\28b*)))")
    );
}

#[test]
fn a_contact_query_returns_matching_people_sorted_by_name_and_never_groups() {
    let db = directory();
    let found = search_contacts(&refs(&db), "lind", 10);
    assert_eq!(names(&found), vec!["Anna Lind", "Carla Lind"], "the group named Lind staff is not a person");
    assert_eq!(found[1].emails, vec!["carla@example.com".to_string(), "carla.lind@example.com".to_string()]);
    assert_eq!(found[0].phones, vec!["+354 555 0101".to_string()]);
}

#[test]
fn a_contact_query_matches_any_address_and_ignores_case() {
    let db = directory();
    assert_eq!(names(&search_contacts(&refs(&db), "EXAMPLE.IS", 10)), vec!["Anna Lind", "Bjarni Olafsson"]);
    assert_eq!(names(&search_contacts(&refs(&db), "carla.lind@", 10)), vec!["Carla Lind"], "the second address matches too");
}

#[test]
fn a_contact_query_keeps_at_most_the_limit_after_sorting() {
    let db = directory();
    assert_eq!(names(&search_contacts(&refs(&db), "example", 1)), vec!["Anna Lind"]);
    assert!(search_contacts(&refs(&db), "example", 0).is_empty());
}

#[test]
fn a_contact_without_a_full_name_takes_the_record_name() {
    let mut r = DirRecord::new("users", "aaron");
    r.set_values("mail", vec!["aaron@example.is".to_string()]).unwrap();
    let contact = Contact::from_record(&r);
    assert_eq!(contact.name, "aaron");
    assert_eq!(contact.emails, vec!["aaron@example.is".to_string()]);
    assert!(contact.phones.is_empty());
}

fn controller_server() -> Server {
    let reg = Registration::new("PinyinController").mode("zh-pinyin", "Pinyin").mode("ja-romaji", "Romaji");
    Server::new(reg).unwrap()
}

#[test]
fn a_shortcut_during_a_composition_commits_it_before_the_client_acts() {
    let mut server = controller_server();
    let session = server.connect(1).unwrap();
    session.set_composition(0, "ni", "你", 3);
    let copy = Event::Key { text: "c".to_string(), combo: KeyCombo { cmd: true, ..KeyCombo::new(Key::Char('c')) } };
    assert_eq!(session.handle(&copy), Outcome::Committed("你".to_string()));
    assert!(session.composition().is_none(), "the shortcut ends the conversion");
}

#[test]
fn a_tab_and_a_function_key_during_a_composition_commit_it() {
    let mut server = controller_server();
    let session = server.connect(1).unwrap();
    session.set_composition(0, "ma", "吗", 3);
    let tab = Event::Key { text: "\t".to_string(), combo: KeyCombo::new(Key::Tab) };
    assert_eq!(session.handle(&tab), Outcome::Committed("吗".to_string()));
    session.set_composition(0, "ma", "妈", 3);
    let f5 = Event::Key { text: String::new(), combo: KeyCombo::new(Key::F(5)) };
    assert_eq!(session.handle(&f5), Outcome::Committed("妈".to_string()));
}

#[test]
fn an_arrow_key_with_no_candidate_window_ends_the_composition() {
    let mut server = controller_server();
    let session = server.connect(1).unwrap();
    session.set_composition(0, "ma", "ma", 2);
    assert!(!session.candidates().is_visible());
    let down = Event::Key { text: String::new(), combo: KeyCombo::new(Key::Down) };
    assert_eq!(session.handle(&down), Outcome::Committed("ma".to_string()));
}

#[test]
fn an_inactive_session_passes_a_shortcut_through_during_a_composition() {
    let mut server = controller_server();
    let session = server.connect(1).unwrap();
    session.set_composition(0, "ni", "你", 3);
    session.deactivate();
    let copy = Event::Key { text: "c".to_string(), combo: KeyCombo { cmd: true, ..KeyCombo::new(Key::Char('c')) } };
    assert_eq!(session.handle(&copy), Outcome::PassThrough);
}
