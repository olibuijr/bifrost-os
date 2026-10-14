//! Parity app-messages: headless tests of the emoji picker's group strip (All, then six groups) through the model.
//! The picker is data and state only here; no test draws it or opens a window.

use bifrost_messages::model::{emoji_in_group, emoji_matches, EmojiGroup, Messages, Overlay, EMOJI_CATALOG};

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
