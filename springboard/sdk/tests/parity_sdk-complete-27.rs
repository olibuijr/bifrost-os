//! Parity sdk-complete-27: headless tests of the user-activity record of the workspace and of the edit commands of the
//! pasteboard. Only state and rules are checked: no display, no shell, no clock.

use bifrost_app::pasteboard::{offers_text, EditCommand, EditState, STRING};
use bifrost_app::workspace::Activity;

#[test]
fn a_dotted_activity_type_is_accepted_and_a_bare_name_is_not() {
    assert!(Activity::new("org.example.notes.editing", "Editing a note").is_some());
    assert!(Activity::new("notes", "Editing a note").is_none(), "a type needs at least two segments");
    assert!(Activity::new("org..notes", "x").is_none(), "an empty segment is refused");
    assert!(Activity::new("org.example notes", "x").is_none(), "a space is refused");
    assert!(Activity::new("org.example_notes", "x").is_none(), "an underscore is refused");
    assert!(Activity::new("", "x").is_none());
}

#[test]
fn an_activity_keeps_its_type_title_and_the_state_it_carries() {
    let a = Activity::new("org.example.notes.editing", "Untitled note")
        .unwrap()
        .with_info("document", "note-42")
        .with_info("cursor", "118");
    assert_eq!(a.activity_type(), "org.example.notes.editing");
    assert_eq!(a.title(), "Untitled note");
    assert_eq!(a.user_info().get("document").map(String::as_str), Some("note-42"));
    assert_eq!(a.user_info().get("cursor").map(String::as_str), Some("118"));
    assert_eq!(a.user_info().len(), 2);
}

#[test]
fn an_activity_becomes_current_once_and_resigns_once() {
    let mut a = Activity::new("org.example.notes.editing", "Note").unwrap();
    assert!(!a.is_current(), "a new activity is not current");
    assert!(a.become_current());
    assert!(!a.become_current(), "it is already current");
    assert!(a.is_current());
    assert!(a.resign());
    assert!(!a.resign(), "it has already resigned");
    assert!(!a.is_current());
}

#[test]
fn an_activity_is_offered_for_continuation_only_while_current_and_eligible() {
    let mut eligible = Activity::new("org.example.notes.editing", "Note").unwrap().eligible_for_continuation();
    assert!(!eligible.offers_continuation(), "not current yet");
    eligible.become_current();
    assert!(eligible.offers_continuation());
    eligible.resign();
    assert!(!eligible.offers_continuation(), "a resigned activity is not offered");

    let mut private = Activity::new("org.example.notes.editing", "Note").unwrap();
    private.become_current();
    assert!(!private.offers_continuation(), "without eligibility it is never offered");
}

#[test]
fn search_eligibility_is_separate_from_continuation() {
    let indexed = Activity::new("org.example.notes.editing", "Note").unwrap().eligible_for_search();
    assert!(indexed.offers_search());
    assert!(!indexed.offers_continuation());
    let plain = Activity::new("org.example.notes.editing", "Note").unwrap();
    assert!(!plain.offers_search());
}

#[test]
fn an_incoming_activity_resumes_only_one_of_the_same_type() {
    let a = Activity::new("org.example.notes.editing", "Note").unwrap();
    assert!(a.resumes("org.example.notes.editing"));
    assert!(!a.resumes("org.example.notes.reading"));
    assert!(!a.resumes("org.example.notes"));
}

#[test]
fn cut_copy_and_delete_need_a_selection() {
    let none = EditState::default();
    let selected = EditState { has_text: true, has_selection: true, board_has_text: false };
    for command in [EditCommand::Cut, EditCommand::Copy, EditCommand::Delete] {
        assert!(!command.enabled(none), "{command:?} without a selection");
        assert!(command.enabled(selected), "{command:?} with a selection");
    }
}

#[test]
fn paste_needs_text_on_the_board_and_select_all_needs_text_in_the_control() {
    let on_board = EditState { has_text: false, has_selection: false, board_has_text: true };
    assert!(EditCommand::Paste.enabled(on_board));
    assert!(!EditCommand::Paste.enabled(EditState::default()));
    assert!(!EditCommand::SelectAll.enabled(on_board), "an empty control has nothing to select");
    let with_text = EditState { has_text: true, has_selection: false, board_has_text: false };
    assert!(EditCommand::SelectAll.enabled(with_text));
    assert!(!EditCommand::Paste.enabled(with_text));
}

#[test]
fn a_board_offers_text_when_it_holds_the_plain_string_type() {
    assert!(offers_text(&[STRING.to_string()]));
    assert!(offers_text(&["public.png".to_string(), STRING.to_string()]));
    assert!(!offers_text(&["public.png".to_string(), "public.html".to_string()]));
    assert!(!offers_text(&[]));
}
