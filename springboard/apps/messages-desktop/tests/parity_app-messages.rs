//! Parity app-messages: headless tests through the model. The emoji picker's group strip, the reply outline, the drop
//! policy, the conversation list (search, pins, mutes, unread marks, leaving) and the composer's reply and edit states.
//! No test draws a frame or opens a window.

use bifrost_messages::model::{emoji_in_group, emoji_matches, Compose, EmojiGroup, Message, Messages, Overlay, Reaction, Room, EMOJI_CATALOG};
use bifrost_messages::protocol::Body;

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

#[test]
fn every_catalogue_emoji_belongs_to_exactly_one_group() {
    let total: usize = EmojiGroup::ALL.iter().map(|g| emoji_in_group(Some(*g)).len()).sum();
    assert_eq!(total, EMOJI_CATALOG.len(), "the six groups partition the catalogue");
    for g in EmojiGroup::ALL {
        assert!(!emoji_in_group(Some(g)).is_empty(), "the {} group is not empty", g.label());
    }
}

#[test]
fn all_lists_the_whole_catalogue_in_catalogue_order() {
    let catalogue: Vec<&str> = EMOJI_CATALOG.iter().map(|(_, emoji, _)| *emoji).collect();
    assert_eq!(emoji_in_group(None), catalogue);
}

#[test]
fn the_strip_has_six_groups_with_distinct_labels_in_strip_order() {
    let mut labels: Vec<&str> = EmojiGroup::ALL.iter().map(|g| g.label()).collect();
    assert_eq!(labels.len(), 6);
    labels.sort_unstable();
    labels.dedup();
    assert_eq!(labels.len(), 6, "no two groups share a label");
    assert_eq!(EmojiGroup::ALL[3].label(), "Food", "the fourth button after All is Food");
}

#[test]
fn a_group_offers_only_its_own_emoji_in_catalogue_order() {
    let mut m = Messages::default();
    m.open_emoji(None);
    m.emoji_group = Some(EmojiGroup::Food);
    assert_eq!(m.emoji_choices(), strings(&["\u{2615}", "\u{1F355}", "\u{1F382}"]));
}

#[test]
fn a_recent_emoji_of_the_group_comes_first_and_other_groups_do_not_show() {
    let mut m = Messages::default();
    m.prefs.recent_emoji = strings(&["\u{1F382}", "\u{1F525}"]);
    m.emoji_group = Some(EmojiGroup::Food);
    assert_eq!(m.emoji_choices(), strings(&["\u{1F382}", "\u{2615}", "\u{1F355}"]), "cake is recent; fire is Nature and stays out");
}

#[test]
fn all_puts_every_recent_emoji_first() {
    let mut m = Messages::default();
    m.prefs.recent_emoji = strings(&["\u{1F382}", "\u{1F525}"]);
    let choices = m.emoji_choices();
    assert_eq!(&choices[..2], &strings(&["\u{1F382}", "\u{1F525}"])[..]);
    assert_eq!(choices.len(), EMOJI_CATALOG.len(), "a recent emoji is listed once");
}

#[test]
fn a_search_finds_emoji_in_any_group_whatever_group_is_selected() {
    let mut m = Messages::default();
    m.emoji_group = Some(EmojiGroup::Food);
    m.emoji_query = "party".into();
    let found = m.emoji_choices();
    assert_eq!(found, emoji_matches("party"), "the search ignores the group");
    assert_eq!(found.len(), 2, "the party face (Faces) and the tada (Things)");
}

#[test]
fn opening_the_picker_selects_all_again() {
    let mut m = Messages::default();
    m.emoji_group = Some(EmojiGroup::Signs);
    m.open_emoji(None);
    assert_eq!(m.emoji_group, None, "a fresh picker shows All");
    assert_eq!(m.overlay, Overlay::Emoji(None));
}

fn message(id: &str, sender: &str, text: &str) -> Message {
    Message { id: id.into(), sender: sender.into(), ts: 1, body: Body::Text(text.into()), reply_to: None, edited: false, history: Vec::new(), reactions: Vec::new() }
}

fn room(id: &str, name: &str, timeline: Vec<Message>) -> Room {
    Room { id: id.into(), name: name.into(), timeline, ..Room::default() }
}

fn signed_in_with(rooms: Vec<Room>) -> Messages {
    Messages { user_id: "@me:x".into(), rooms, ..Messages::default() }
}

fn ids(m: &Messages) -> Vec<String> {
    m.visible_rooms().iter().map(|r| r.id.clone()).collect()
}

#[test]
fn a_quoted_message_is_outlined_until_the_next_press_or_another_conversation() {
    let mut m = signed_in_with(vec![room("!a:x", "Garden", vec![message("$1", "@bob:x", "hi"), message("$2", "@bob:x", "yo")]), room("!b:x", "Office", vec![])]);
    m.selected = Some("!a:x".into());
    assert!(m.highlight_message("$1"), "the jump lands on a message of the room on screen");
    assert!(m.is_highlighted("$1"));
    assert!(!m.is_highlighted("$2"), "only the quoted message is outlined");
    m.clear_highlight();
    assert!(!m.is_highlighted("$1"), "the next press takes the outline off");
    assert!(m.highlight_message("$2"));
    m.select("!b:x");
    assert!(!m.is_highlighted("$2"), "opening another conversation clears the outline");
}

#[test]
fn a_message_that_is_not_on_screen_is_not_outlined() {
    let mut m = signed_in_with(vec![room("!a:x", "Garden", vec![message("$1", "@bob:x", "hi")])]);
    assert!(!m.highlight_message("$1"), "no conversation is open yet");
    m.selected = Some("!a:x".into());
    assert!(!m.highlight_message("$9"), "a message the room does not hold");
    assert!(!m.is_highlighted("$9"));
}

#[test]
fn files_dropped_on_the_window_need_a_signed_in_account_and_an_open_conversation() {
    let mut m = Messages::default();
    m.rooms.push(room("!a:x", "Garden", vec![]));
    m.selected = Some("!a:x".into());
    assert!(!m.accepts_drop(), "a signed-out window takes no files");
    m.user_id = "@me:x".into();
    assert!(m.accepts_drop(), "a signed-in window with a conversation open takes the files");
    m.selected = None;
    assert!(!m.accepts_drop(), "a drop needs a conversation on screen");
}

#[test]
fn the_search_field_matches_conversation_names_and_loaded_text_in_list_order() {
    let mut m = signed_in_with(vec![
        room("!a:x", "Garden club", vec![]),
        room("!b:x", "Office", vec![message("$1", "@bob:x", "Bring the garden hose")]),
        room("!c:x", "Lunch", vec![]),
    ]);
    m.search = "GARDEN".into();
    assert_eq!(ids(&m), vec!["!a:x".to_string(), "!b:x".to_string()], "a name and a loaded message both match");
    m.search = "  ".into();
    assert_eq!(ids(&m).len(), 3, "a blank search shows every conversation");
}

#[test]
fn pinned_conversations_come_first_in_the_order_they_were_pinned() {
    let mut m = signed_in_with(vec![room("!a:x", "A", vec![]), room("!b:x", "B", vec![]), room("!c:x", "C", vec![])]);
    assert_eq!(m.toggle_pin("!c:x"), Ok(true));
    assert_eq!(m.toggle_pin("!b:x"), Ok(true));
    assert_eq!(ids(&m), vec!["!c:x", "!b:x", "!a:x"], "the first pin stays first");
    assert_eq!(m.toggle_pin("!c:x"), Ok(false), "a pinned conversation is unpinned by a second pin");
    assert_eq!(ids(&m), vec!["!b:x", "!a:x", "!c:x"]);
}

#[test]
fn hide_alerts_is_a_switch_per_conversation() {
    let mut m = signed_in_with(vec![room("!a:x", "A", vec![]), room("!b:x", "B", vec![])]);
    assert!(m.toggle_mute("!a:x"), "the first toggle hides the alerts");
    assert!(m.is_muted("!a:x"));
    assert!(!m.is_muted("!b:x"), "other conversations still alert");
    assert!(!m.toggle_mute("!a:x"), "the second toggle turns them back on");
}

#[test]
fn mark_as_unread_is_a_switch_that_the_server_state_can_set() {
    let mut m = signed_in_with(vec![room("!a:x", "A", vec![])]);
    assert!(m.toggle_unread_mark("!a:x"));
    assert!(m.is_marked_unread("!a:x"));
    assert!(m.set_unread_mark("!a:x", false), "the server clears the mark");
    assert!(!m.is_marked_unread("!a:x"));
    assert!(!m.set_unread_mark("!a:x", false), "a mark that is already gone changes nothing");
}

#[test]
fn leaving_a_conversation_forgets_its_pin_mute_and_unread_mark() {
    let mut m = signed_in_with(vec![room("!a:x", "A", vec![]), room("!b:x", "B", vec![])]);
    m.toggle_pin("!a:x").unwrap();
    m.toggle_mute("!a:x");
    m.toggle_unread_mark("!a:x");
    m.remove_room("!a:x");
    assert_eq!(ids(&m), vec!["!b:x".to_string()]);
    assert!(!m.is_pinned("!a:x") && !m.is_muted("!a:x") && !m.is_marked_unread("!a:x"));
}

#[test]
fn signing_out_forgets_the_account_and_its_rooms_but_keeps_the_choices() {
    let mut m = signed_in_with(vec![room("!a:x", "A", vec![])]);
    m.toggle_pin("!a:x").unwrap();
    m.sign_out();
    assert!(!m.signed_in());
    assert!(m.rooms.is_empty());
    assert_eq!(m.prefs.pinned, vec!["!a:x".to_string()], "the pins survive a sign-out");
}

#[test]
fn a_reply_and_an_edit_start_only_on_messages_that_allow_them() {
    let mut m = signed_in_with(vec![room("!a:x", "A", vec![message("$1", "@bob:x", "hi"), message("$2", "@me:x", "mine")])]);
    m.selected = Some("!a:x".into());
    assert!(m.begin_reply("$1"));
    assert_eq!(m.compose, Some(Compose::Reply("$1".into())));
    assert!(!m.begin_edit("$1"), "only my own text messages can be edited");
    assert!(m.begin_edit("$2"));
    assert_eq!(m.compose, Some(Compose::Edit("$2".into())));
    assert!(!m.begin_reply("$9"), "a message the room does not hold cannot be answered");
}

#[test]
fn my_reaction_is_found_by_its_emoji_and_its_event() {
    let mut reacted = message("$1", "@bob:x", "hi");
    reacted.reactions = vec![Reaction { key: "+1".into(), users: vec![("@me:x".into(), "$r".into()), ("@ann:x".into(), "$s".into())] }];
    assert_eq!(reacted.reaction_of("@me:x", "+1"), Some("$r"), "the event that took my reaction is the one to undo");
    assert_eq!(reacted.reaction_of("@ann:x", "+1"), Some("$s"));
    assert_eq!(reacted.reaction_of("@me:x", "heart"), None, "no reaction with another emoji");
}
