//! Parity sdk-tests-14: headless tests of the share sheet's item model. The vCard for a contact card, the names the
//! receiver sees, and the bytes sent. No test opens a sheet or draws a frame.

use springboard::shell::share::{vcard, Item};

#[test]
fn a_vcard_starts_and_ends_the_card_with_version_three() {
    let v = vcard("Ada Lovelace", "+354 555 0100", "ada@example.org", "");
    assert!(v.starts_with("BEGIN:VCARD\r\nVERSION:3.0\r\n"), "{v:?}");
    assert!(v.ends_with("END:VCARD\r\n"), "{v:?}");
    assert!(v.contains("FN:Ada Lovelace\r\n"));
    assert!(v.contains("N:Ada Lovelace;;;;\r\n"));
}

#[test]
fn a_vcard_leaves_out_the_fields_that_are_blank() {
    let v = vcard("Ada", "   ", "ada@example.org", "");
    assert!(!v.contains("TEL:"), "a blank phone is not written");
    assert!(v.contains("EMAIL:ada@example.org\r\n"));
    assert!(!v.contains("NOTE:"), "empty notes are not written");
}

#[test]
fn a_vcard_escapes_commas_semicolons_and_line_breaks_in_values() {
    let v = vcard("Lovelace, Ada", "", "", "Line one;\nline two");
    assert!(v.contains("FN:Lovelace\\, Ada\r\n"), "{v:?}");
    assert!(v.contains("NOTE:Line one\\;\\nline two\r\n"), "{v:?}");
}

#[test]
fn a_vcard_escapes_backslashes_before_the_other_escapes() {
    let v = vcard("A\\B", "", "", "");
    assert!(v.contains("FN:A\\\\B\r\n"), "{v:?}");
}

#[test]
fn a_card_item_is_titled_by_the_contact_and_named_as_a_vcf_file() {
    let item = Item::Card { name: "Ada Lovelace".into(), vcard: vcard("Ada Lovelace", "", "", "") };
    assert_eq!(item.title(), "Ada Lovelace");
    assert_eq!(item.file_name(), "Ada Lovelace.vcf");
    assert_eq!(item.subtitle(), "Contact card");
}

#[test]
fn a_card_item_with_an_unusable_name_falls_back_to_a_contact_file_name() {
    let item = Item::Card { name: "   ".into(), vcard: String::new() };
    assert_eq!(item.file_name(), "Contact.vcf");
}

#[test]
fn a_card_item_drops_path_separators_from_its_file_name() {
    let item = Item::Card { name: "../Ada/Lovelace".into(), vcard: String::new() };
    assert_eq!(item.file_name(), "AdaLovelace.vcf", "slashes and leading dots are removed from the name");
}

#[test]
fn a_card_item_sends_its_vcard_bytes_and_copies_the_same_text() {
    let text = vcard("Ada", "+354 555 0100", "", "");
    let item = Item::Card { name: "Ada".into(), vcard: text.clone() };
    assert_eq!(item.bytes(), Some(text.clone().into_bytes()));
    assert_eq!(item.clip_text(), text);
}

#[test]
fn a_file_item_has_no_bytes_because_it_is_streamed_from_disk() {
    let item = Item::File("/Users/u/Documents/report.pdf".into());
    assert_eq!(item.bytes(), None);
    assert_eq!(item.file_name(), "report.pdf");
    assert_eq!(item.subtitle(), "PDF");
}
