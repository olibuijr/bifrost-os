use super::*;
use crate::shell::helpbook::Hit;

fn entry(menu: Menu, row: usize, title: &str, enabled: bool) -> (Menu, usize, helpbook::MenuItem) {
    (menu, row, help_item(match menu { Menu::Edit => "Edit", Menu::Window => "Window", _ => "App" }, title, enabled))
}

#[test]
fn check_marks_and_indents_are_left_out_of_the_title() {
    let item = help_item("Edit", "\u{2713} Copy", true);
    assert_eq!(item.title, "Copy");
    assert_eq!(item.menu, "Edit");
    assert!(item.enabled);
    assert_eq!(help_item("View", "    Larger Icons", false).title, "Larger Icons");
}

#[test]
fn menu_items_group_before_topics_and_empty_groups_are_left_out() {
    assert_eq!(
        help_groups(&[3, 1], 2),
        vec![HelpRow::Header("Menu Items"), HelpRow::Item(3), HelpRow::Item(1), HelpRow::Header("Help Topics"), HelpRow::Topic(0), HelpRow::Topic(1)]
    );
    assert_eq!(help_groups(&[], 1), vec![HelpRow::Header("Help Topics"), HelpRow::Topic(0)], "no Menu Items heading without hits");
    assert_eq!(help_groups(&[0], 0), vec![HelpRow::Header("Menu Items"), HelpRow::Item(0)]);
    assert_eq!(help_groups(&[], 0), vec![HelpRow::Header("No Results")]);
}

#[test]
fn a_menu_query_resolves_to_the_row_that_holds_the_item() {
    let entries = vec![
        entry(Menu::Edit, 4, "Copy", true),
        entry(Menu::Edit, 5, "Paste", false),
        entry(Menu::Window, 2, "Copy Window", true),
    ];
    assert_eq!(help_hits(&entries, "copy"), vec![(Menu::Edit, 4), (Menu::Window, 2)], "whole-word titles in menu order");
    assert_eq!(help_hits(&entries, "paste"), vec![(Menu::Edit, 5)], "a disabled item is still found");
    assert!(help_hits(&entries, "print").is_empty());
    assert!(help_hits(&entries, "  ").is_empty());
}

#[test]
fn topics_come_from_the_front_book_and_the_system_book_only() {
    let hit = |app: &str, title: &str| Hit { app: app.into(), app_name: app.into(), slug: title.into(), title: title.into(), anchor: None, score: 1 };
    let hits = vec![hit("is.olibuijr.other", "A"), hit("com.front", "B"), hit(helpbook::SYSTEM_BOOK, "C"), hit("com.front", "D")];
    let kept: Vec<String> = help_topic_hits(hits.clone(), Some("com.front"), 8).into_iter().map(|h| h.title).collect();
    assert_eq!(kept, vec!["B", "C", "D"]);
    let one: Vec<String> = help_topic_hits(hits, None, 1).into_iter().map(|h| h.title).collect();
    assert_eq!(one, vec!["C"], "no front book: only the system book, limited");
}

#[test]
fn keyboard_steps_skip_headers_and_wrap() {
    let sel = [false, true, false, true];
    assert_eq!(step_index(&sel, None, 1), Some(1), "down from nothing lands on the first choice");
    assert_eq!(step_index(&sel, Some(1), 1), Some(3));
    assert_eq!(step_index(&sel, Some(3), 1), Some(1), "down wraps to the top");
    assert_eq!(step_index(&sel, None, -1), Some(3), "up from nothing lands on the last choice");
    assert_eq!(step_index(&sel, Some(1), -1), Some(3), "up wraps to the bottom");
    assert_eq!(step_index(&[false, false], None, 1), None, "no choices");
    assert_eq!(step_index(&[], Some(0), 1), None);
}
