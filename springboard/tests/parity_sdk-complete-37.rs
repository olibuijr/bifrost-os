//! Parity sdk-complete-37: headless tests for the offline download store. No test opens a connection or writes a file.

use springboard::media::offline::{OfflineStore, StoreError, TaskState};

fn finished(store: &mut OfflineStore, id: &str, size: u64) {
    store.enqueue(id, Some(size)).unwrap();
    store.start(id).unwrap();
    assert_eq!(store.receive(id, size), Ok(TaskState::Finished));
}

#[test]
fn a_new_download_starts_at_byte_zero_and_reports_its_fraction() {
    let mut store = OfflineStore::new(1000);
    store.enqueue("episode-1", Some(400)).unwrap();
    assert_eq!(store.start("episode-1"), Ok(0));
    assert_eq!(store.receive("episode-1", 100), Ok(TaskState::Downloading));
    assert_eq!(store.task("episode-1").unwrap().fraction(), Some(0.25));
}

#[test]
fn a_download_is_queued_once_per_identifier() {
    let mut store = OfflineStore::new(1000);
    store.enqueue("a", None).unwrap();
    assert_eq!(store.enqueue("a", None), Err(StoreError::Duplicate));
    assert_eq!(store.tasks().len(), 1);
}

#[test]
fn a_paused_download_resumes_from_the_bytes_it_has() {
    let mut store = OfflineStore::new(1000);
    store.enqueue("a", Some(300)).unwrap();
    store.start("a").unwrap();
    store.receive("a", 120).unwrap();
    store.pause("a").unwrap();
    assert_eq!(store.task("a").unwrap().state(), TaskState::Paused);
    assert_eq!(store.start("a"), Ok(120), "the request starts at the byte already stored");
}

#[test]
fn a_failed_download_keeps_its_bytes_and_restarts_from_them() {
    let mut store = OfflineStore::new(1000);
    store.enqueue("a", Some(300)).unwrap();
    store.start("a").unwrap();
    store.receive("a", 50).unwrap();
    store.fail("a").unwrap();
    assert_eq!(store.task("a").unwrap().state(), TaskState::Failed);
    assert_eq!(store.start("a"), Ok(50));
    assert_eq!(store.bytes_held(), 50);
}

#[test]
fn a_download_finishes_at_its_total_and_never_stores_more() {
    let mut store = OfflineStore::new(1000);
    store.enqueue("a", Some(100)).unwrap();
    store.start("a").unwrap();
    assert_eq!(store.receive("a", 250), Ok(TaskState::Finished));
    assert_eq!(store.task("a").unwrap().received(), 100);
    assert_eq!(store.start("a"), Err(StoreError::InvalidState), "a finished item is not downloaded again");
}

#[test]
fn a_download_of_unknown_size_is_never_finished_by_its_bytes() {
    let mut store = OfflineStore::new(1000);
    store.enqueue("stream", None).unwrap();
    store.start("stream").unwrap();
    assert_eq!(store.receive("stream", 5000), Ok(TaskState::Downloading));
    assert_eq!(store.task("stream").unwrap().fraction(), None);
}

#[test]
fn bytes_only_arrive_for_a_running_download() {
    let mut store = OfflineStore::new(1000);
    store.enqueue("a", Some(10)).unwrap();
    assert_eq!(store.receive("a", 1), Err(StoreError::InvalidState), "queued items take no bytes");
    assert_eq!(store.pause("a"), Err(StoreError::InvalidState));
    assert_eq!(store.receive("missing", 1), Err(StoreError::Unknown));
}

#[test]
fn the_budget_evicts_the_least_recently_played_finished_item_first() {
    let mut store = OfflineStore::new(300);
    finished(&mut store, "old", 100);
    finished(&mut store, "newer", 100);
    store.played("old").unwrap();
    store.played("newer").unwrap();
    store.played("old").unwrap(); // old is now the most recently played
    assert_eq!(store.make_room(150), Ok(vec!["newer".to_string()]));
    assert!(store.task("old").is_some());
    assert_eq!(store.bytes_held(), 100);
}

#[test]
fn an_item_never_played_goes_before_one_that_was() {
    let mut store = OfflineStore::new(200);
    finished(&mut store, "played", 100);
    finished(&mut store, "unplayed", 100);
    store.played("played").unwrap();
    assert_eq!(store.make_room(100), Ok(vec!["unplayed".to_string()]));
}

#[test]
fn room_that_fits_needs_no_eviction() {
    let mut store = OfflineStore::new(500);
    finished(&mut store, "a", 100);
    assert_eq!(store.make_room(400), Ok(Vec::new()));
    assert!(store.task("a").is_some());
}

#[test]
fn a_download_in_progress_is_never_evicted() {
    let mut store = OfflineStore::new(200);
    store.enqueue("running", Some(150)).unwrap();
    store.start("running").unwrap();
    store.receive("running", 100).unwrap();
    assert_eq!(store.make_room(150), Err(StoreError::NoRoom));
    assert!(store.task("running").is_some(), "a refused make_room removes nothing");
}

#[test]
fn make_room_refuses_without_evicting_when_the_bytes_can_never_fit() {
    let mut store = OfflineStore::new(200);
    finished(&mut store, "a", 100);
    assert_eq!(store.make_room(300), Err(StoreError::NoRoom));
    assert!(store.task("a").is_some());
}

#[test]
fn removing_an_item_frees_its_bytes() {
    let mut store = OfflineStore::new(100);
    finished(&mut store, "a", 100);
    assert!(store.remove("a"));
    assert!(!store.remove("a"));
    assert_eq!(store.bytes_held(), 0);
    assert_eq!(store.make_room(100), Ok(Vec::new()));
}

#[test]
fn only_a_finished_item_can_be_marked_as_played() {
    let mut store = OfflineStore::new(100);
    store.enqueue("a", Some(10)).unwrap();
    assert_eq!(store.played("a"), Err(StoreError::InvalidState));
    assert_eq!(store.played("nope"), Err(StoreError::Unknown));
}
