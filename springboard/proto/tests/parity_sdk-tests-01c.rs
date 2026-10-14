//! The preview wire messages (`ToShell::Preview` and `ToShell::PreviewClose`) through the public API of `bifrost_proto`:
//! a headless encode and decode round trip with the caps the shell relies on. No socket is opened.

use bifrost_proto::peek::{PreviewItem, MAX_ITEMS, MAX_TITLE, MAX_URL};
use bifrost_proto::{ToShell, MAX_PACKET};

fn item(url: &str, title: &str) -> PreviewItem {
    PreviewItem { url: url.into(), title: title.into() }
}

#[test]
fn a_preview_list_round_trips_with_its_index() {
    let items = vec![item("file:///a.txt", "A"), item("file:///b.png", "B")];
    let bytes = ToShell::Preview { items: items.clone(), index: 1 }.encode();
    assert!(bytes.len() <= MAX_PACKET);
    assert_eq!(ToShell::decode(&bytes), Ok(ToShell::Preview { items, index: 1 }));
}

#[test]
fn the_list_is_capped_and_the_index_follows_the_cap() {
    let many = vec![item("file:///x", "x"); MAX_ITEMS + 4];
    let decoded = ToShell::decode(&ToShell::Preview { items: many, index: 99 }.encode()).unwrap();
    let ToShell::Preview { items, index } = decoded else { panic!("a preview message") };
    assert_eq!(items.len(), MAX_ITEMS);
    assert_eq!(index, MAX_ITEMS as u32 - 1, "an index past the kept items is moved to the last kept one");
}

#[test]
fn an_index_past_a_short_list_is_moved_to_its_last_item() {
    let decoded = ToShell::decode(&ToShell::Preview { items: vec![item("file:///a", "a"), item("file:///b", "b")], index: 7 }.encode());
    let ToShell::Preview { index, .. } = decoded.unwrap() else { panic!("a preview message") };
    assert_eq!(index, 1);
}

#[test]
fn an_overlong_url_is_cut_to_the_url_cap() {
    let long = "x".repeat(MAX_URL + 50);
    let decoded = ToShell::decode(&ToShell::Preview { items: vec![item(&long, "t")], index: 0 }.encode()).unwrap();
    let ToShell::Preview { items, .. } = decoded else { panic!("a preview message") };
    assert_eq!(items[0].url.len(), MAX_URL);
    assert!(items[0].url.bytes().all(|b| b == b'x'));
}

#[test]
fn an_overlong_title_is_cut_on_a_character_boundary() {
    // Two-byte characters: the cap of 96 bytes is 48 characters exactly.
    let title = "é".repeat(100);
    let decoded = ToShell::decode(&ToShell::Preview { items: vec![item("file:///t", &title)], index: 0 }.encode()).unwrap();
    let ToShell::Preview { items, .. } = decoded else { panic!("a preview message") };
    assert_eq!(items[0].title.len(), MAX_TITLE);
    assert_eq!(items[0].title.chars().count(), MAX_TITLE / 2);
}

#[test]
fn an_empty_list_round_trips() {
    let decoded = ToShell::decode(&ToShell::Preview { items: Vec::new(), index: 0 }.encode());
    assert_eq!(decoded, Ok(ToShell::Preview { items: Vec::new(), index: 0 }));
}

#[test]
fn the_close_message_round_trips() {
    assert_eq!(ToShell::decode(&ToShell::PreviewClose.encode()), Ok(ToShell::PreviewClose));
}
