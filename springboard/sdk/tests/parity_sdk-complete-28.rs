//! Parity sdk-complete-28: the media intents an app uses through `bifrost_app::media_intents`. A playback request is
//! answered against the app's catalogue, the plan is applied to the app's queue and the dialog is returned. Headless:
//! no playback starts and no speech is recognised.

use bifrost_app::media_intents::{handle_request, MediaItem, MediaKind, PlaybackQueue, PlaybackRequest};

fn catalogue() -> [MediaItem; 2] {
    [MediaItem::new("s1", MediaKind::Song, "Blue Hour"), MediaItem::new("s2", MediaKind::Song, "Blue Tide")]
}

#[test]
fn a_play_request_starts_the_queue_and_names_the_first_title_in_the_dialog() {
    let catalogue = catalogue();
    let mut queue = PlaybackQueue::new();
    let dialog = handle_request(&mut queue, &PlaybackRequest::parse("play blue"), &catalogue, 10);
    assert_eq!(queue.current().map(|item| item.id.as_str()), Some("s1"));
    assert_eq!(queue.len(), 2);
    assert!(dialog.contains("Blue Hour"), "the dialog names the item now playing: {dialog}");
}

#[test]
fn a_request_with_no_match_leaves_the_queue_empty_and_still_answers() {
    let catalogue = catalogue();
    let mut queue = PlaybackQueue::new();
    let dialog = handle_request(&mut queue, &PlaybackRequest::parse("play zzzz"), &catalogue, 10);
    assert!(queue.is_empty());
    assert!(!dialog.is_empty(), "the person always gets an answer");
}

#[test]
fn a_queue_request_appends_to_what_is_already_playing() {
    let catalogue = catalogue();
    let mut queue = PlaybackQueue::new();
    handle_request(&mut queue, &PlaybackRequest::parse("play blue hour"), &catalogue, 10);
    handle_request(&mut queue, &PlaybackRequest::parse("queue blue tide"), &catalogue, 10);
    assert_eq!(queue.current().map(|item| item.id.as_str()), Some("s1"));
    assert_eq!(queue.upcoming().first().map(|item| item.id.as_str()), Some("s2"));
}
