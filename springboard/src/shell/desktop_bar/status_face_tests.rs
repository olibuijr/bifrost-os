use super::*;

#[test]
fn a_title_wins_and_a_drawable_symbol_shows_as_an_icon() {
    assert_eq!(status_face("Quick", "square.and.pencil"), Some(StatusFace::Text("Quick".into())));
    assert_eq!(status_face("", "square.and.pencil"), Some(StatusFace::Icon("square.and.pencil".into())), "Notes' extra");
    assert_eq!(status_face("", "checklist"), Some(StatusFace::Icon("checklist".into())), "Reminders' extra");
    assert_eq!(status_face("", "wifi"), Some(StatusFace::Icon("wifi".into())), "a symbol of the shell's own set");
}

#[test]
fn an_item_with_no_title_and_no_symbol_the_bar_draws_is_not_shown() {
    assert_eq!(status_face("", ""), None, "no letter stands in for it");
    assert_eq!(status_face("", "unknown.symbol.name"), None);
}
