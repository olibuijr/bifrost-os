//! Parity tests for the shared file list model (recent items and favourites). Pure logic, no window or device: each test
//! drives the public `SharedList` API with concrete keys and checks the ordering, the cap and the favourite rules.

use bifrost_ui::shared_list::SharedList;

fn keys(list: &SharedList) -> Vec<String> {
    list.entries().iter().map(|i| i.key.clone()).collect()
}

#[test]
fn a_used_document_moves_to_the_front_with_its_new_title() {
    let mut list = SharedList::new(5);
    assert!(list.add("/a.txt", "a"));
    assert!(list.add("/b.txt", "b"));
    assert!(list.add("/a.txt", "A renamed"));
    assert_eq!(keys(&list), vec!["/a.txt", "/b.txt"]);
    assert_eq!(list.entries()[0].title, "A renamed");
    assert_eq!(list.len(), 2);
}

#[test]
fn an_empty_key_is_refused_and_the_list_stays_empty() {
    let mut list = SharedList::new(3);
    assert!(!list.add("", "nothing"));
    assert!(list.is_empty());
}

#[test]
fn recents_beyond_the_capacity_drop_the_oldest() {
    let mut list = SharedList::new(2);
    for k in ["/1", "/2", "/3"] {
        list.add(k, k);
    }
    assert_eq!(keys(&list), vec!["/3", "/2"]);
    assert!(!list.contains("/1"));
}

#[test]
fn a_zero_capacity_still_keeps_one_recent() {
    let mut list = SharedList::new(0);
    list.add("/x", "x");
    list.add("/y", "y");
    assert_eq!(keys(&list), vec!["/y"]);
}

#[test]
fn favourites_are_kept_when_recents_are_trimmed_and_listed_first() {
    let mut list = SharedList::new(1);
    list.add("/keep", "keep");
    assert!(list.set_favourite("/keep", true));
    list.add("/one", "one");
    list.add("/two", "two");
    assert_eq!(keys(&list), vec!["/keep", "/two"]);
    assert!(list.entries()[0].favourite);
    assert!(!list.entries()[1].favourite);
}

#[test]
fn unmarking_a_favourite_returns_it_to_the_recents_cap() {
    let mut list = SharedList::new(1);
    list.add("/a", "a");
    assert!(list.set_favourite("/a", true));
    list.add("/b", "b");
    assert!(list.set_favourite("/a", false));
    // /a is a recent again; with a capacity of one the older /a goes
    assert_eq!(keys(&list), vec!["/b"]);
    assert!(!list.set_favourite("/missing", true), "a key not in the list cannot become a favourite");
}

#[test]
fn clearing_recents_keeps_the_favourites() {
    let mut list = SharedList::new(4);
    list.add("/fav", "fav");
    assert!(list.set_favourite("/fav", true));
    list.add("/r1", "r1");
    list.add("/r2", "r2");
    list.clear_recents();
    assert_eq!(keys(&list), vec!["/fav"]);
    assert_eq!(list.len(), 1);
}

#[test]
fn remove_takes_out_one_entry_and_reports_absence() {
    let mut list = SharedList::new(4);
    list.add("/a", "a");
    list.add("/b", "b");
    assert!(list.remove("/a"));
    assert!(!list.remove("/a"));
    assert_eq!(keys(&list), vec!["/b"]);
}
