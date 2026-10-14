//! The accessibility element map of `bifrost_ui::ax` through its public API: slugs and addresses, resolving, rotors,
//! announcements, text attributes, custom actions, settable controls and the parent links of recorded groups. All of it
//! is in memory (headless): no window, no bus and no display.

use bifrost_ui::ax::{
    begin_frame, begin_group, children_of, char_offset, end_group, index_in_parent, masked, parent_of, record, slug, take_frame,
    take_links, text_ax, Announcements, Map, Notification, Politeness, Priority, Request, Resolve, Settable, STANDARD_ROTORS,
};
use bifrost_ui::{Rect, Role};

fn r() -> Rect {
    Rect { x: 0., y: 0., w: 10., h: 10. }
}

#[test]
fn slugs_fold_icelandic_letters_and_turn_gaps_into_dashes() {
    assert_eq!(slug("Ólafur Búi"), "olafur-bui");
    assert_eq!(slug("Software Update"), "software-update");
    assert_eq!(slug("  Wi-Fi  "), "wi-fi");
    assert_eq!(slug("…"), "", "a name with no letters or digits has no slug");
}

#[test]
fn repeated_names_in_one_scope_are_numbered_in_order() {
    let mut m = Map::new();
    assert_eq!(m.add("os/x", "row", "Name", "", true, None, ""), "os/x/name");
    assert_eq!(m.add("os/x", "row", "Name", "", true, None, ""), "os/x/name-2");
    assert_eq!(m.add("os/x", "row", "Name", "", true, None, ""), "os/x/name-3");
    assert_eq!(m.add("os/y", "row", "Name", "", true, None, ""), "os/y/name", "another scope starts again");
    assert_eq!(m.len(), 4);
}

#[test]
fn a_name_resolves_by_suffix_or_is_ambiguous_when_two_elements_share_it() {
    let mut m = Map::new();
    m.add("os/settings/general", "row", "About", "", true, None, "1");
    m.add("os/settings/privacy", "row", "About", "", true, None, "2");
    assert!(matches!(m.resolve("general/about"), Resolve::One(e) if e.address == "os/settings/general/about"));
    assert_eq!(m.resolve("about"), Resolve::Ambiguous(vec!["os/settings/general/about", "os/settings/privacy/about"]));
    assert_eq!(m.resolve("nothing"), Resolve::None);
}

#[test]
fn role_words_are_the_ones_addresses_use() {
    let mut m = Map::new();
    let address = m.add("app/x", "segmented-control", "Mode", "", true, None, "");
    assert_eq!(address, "app/x/mode");
    assert_eq!(m.get(&address).map(|e| e.role.as_str()), Some("segmented-control"));
    assert_eq!(bifrost_ui::ax::role_name(Role::SegmentedControl), "segmented-control");
    assert_eq!(bifrost_ui::ax::role_name(Role::StaticText), "text");
}

#[test]
fn the_controls_rotor_steps_over_its_roles_and_wraps_both_ways() {
    let mut m = Map::new();
    m.add("app/x", "button", "Save", "", true, None, "1");
    m.add("app/x", "text", "Title", "", true, None, "2");
    m.add("app/x", "button", "Cancel", "", true, None, "3");
    let controls = &STANDARD_ROTORS[0];
    assert_eq!(controls.name, "Controls");
    let name = |step: Option<&bifrost_ui::ax::Element>| step.map(|e| e.name.clone());
    assert_eq!(name(m.rotor_step(controls, None, true)), Some("Save".into()), "the text element is skipped");
    assert_eq!(name(m.rotor_step(controls, Some("app/x/save"), true)), Some("Cancel".into()));
    assert_eq!(name(m.rotor_step(controls, Some("app/x/cancel"), true)), Some("Save".into()), "forward wraps");
    assert_eq!(name(m.rotor_step(controls, None, false)), Some("Cancel".into()));
    assert_eq!(name(m.rotor_step(controls, Some("app/x/save"), false)), Some("Cancel".into()), "backward wraps");
}

#[test]
fn the_progress_rotor_finds_nothing_on_a_screen_without_progress() {
    let mut m = Map::new();
    m.add("app/x", "button", "Save", "", true, None, "1");
    assert!(m.rotor_step(&STANDARD_ROTORS[5], None, true).is_none());
    assert_eq!(STANDARD_ROTORS[5].name, "Progress");
}

#[test]
fn a_slider_clamps_its_value_and_steps_stop_at_the_ends() {
    let mut s = Settable::slider(5.0, 0.0, 10.0, 2.5);
    assert_eq!(s.perform(&Request::Increment), vec![Notification::ValueChanged]);
    assert_eq!(s.value, 7.5);
    assert_eq!(s.perform(&Request::Increment), vec![Notification::ValueChanged]);
    assert_eq!(s.value, 10.0);
    assert!(s.perform(&Request::Increment).is_empty(), "already at the maximum: nothing to announce");
    assert_eq!(s.accessibility_value(), "10");
    assert_eq!(s.perform(&Request::Decrement), vec![Notification::ValueChanged]);
    assert_eq!(s.value, 7.5);
}

#[test]
fn a_slider_takes_a_typed_number_and_refuses_text_that_is_not_one() {
    let mut s = Settable::slider(0.0, 0.0, 10.0, 1.0);
    assert_eq!(s.perform(&Request::SetValue(" 3 ".into())), vec![Notification::ValueChanged]);
    assert_eq!(s.value, 3.0);
    assert_eq!(s.perform(&Request::SetValue("99".into())), vec![Notification::ValueChanged]);
    assert_eq!(s.value, 10.0, "a typed value is clamped too");
    assert!(s.perform(&Request::SetValue("loud".into())).is_empty());
    assert!(s.perform(&Request::SetValue("nan".into())).is_empty(), "a value that is not finite is refused");
    assert_eq!(s.value, 10.0);
}

#[test]
fn a_text_field_takes_a_value_and_its_focus_once() {
    let mut f = Settable::text_field("draft");
    assert_eq!(f.perform(&Request::SetValue("final".into())), vec![Notification::ValueChanged]);
    assert_eq!(f.accessibility_value(), "final");
    assert!(f.perform(&Request::SetValue("final".into())).is_empty(), "the same text changes nothing");
    assert!(f.perform(&Request::Increment).is_empty(), "a text field has no steps");
    assert_eq!(f.perform(&Request::SetFocused(true)), vec![Notification::FocusedUIElementChanged]);
    assert!(f.focused);
    assert!(f.perform(&Request::SetFocused(true)).is_empty(), "already focused");
    assert!(f.perform(&Request::SetFocused(false)).is_empty(), "losing focus is not announced");
    assert!(!f.focused);
}

#[test]
fn an_assertive_announcement_is_spoken_first_and_a_full_queue_drops_a_polite_one() {
    let mut q = Announcements::with_capacity(2);
    assert!(q.post("Saved", Politeness::Polite));
    assert!(!q.post("  ", Politeness::Polite), "blank text is not queued");
    assert!(!q.post("Saved", Politeness::Polite), "a repeat of the last text is not queued");
    assert!(q.post("Connection lost", Politeness::Assertive));
    assert!(q.post("Done", Politeness::Polite), "full: the oldest polite text makes room");
    assert_eq!(q.len(), 2);
    assert_eq!(q.take_next().map(|a| a.text), Some("Connection lost".into()));
    assert_eq!(q.take_next().map(|a| a.text), Some("Done".into()));
    assert!(q.is_empty());
}

#[test]
fn announcement_priority_maps_to_politeness() {
    assert_eq!(Priority::default(), Priority::Medium);
    assert_eq!(Priority::from_wire(2), Some(Priority::High));
    assert_eq!(Priority::from_wire(3), None);
    assert_eq!((Priority::Low.politeness(), Priority::Medium.politeness(), Priority::High.politeness()), (1, 1, 2));
}

#[test]
fn text_attributes_are_char_offsets_of_the_selection_and_the_caret() {
    let a = text_ax(3, "hello world", 6, 8, (0, 5), false);
    assert_eq!(a.caret, 8, "the caret is the moving end");
    assert_eq!(a.selected_range(), (6, 8));
    assert_eq!(a.selected_text("hello world"), "wo");
    assert_eq!((a.vis_start, a.vis_end, a.len), (0, 5, 11));
    let caret_only = text_ax(5, "abc", 1, 1, (0, 3), false);
    assert_eq!(caret_only.selected_range(), (1, 1));
    assert_eq!(caret_only.selected_text("abc"), "");
}

#[test]
fn offsets_count_characters_and_round_a_byte_inside_one_down() {
    assert_eq!(char_offset("aé b", 3), 2, "byte 3 is after the two-byte é");
    assert_eq!(char_offset("aé", 2), 1, "byte 2 is inside é, so the offset rounds down to 1");
    let inverted = text_ax(3, "hello", 0, 0, (4, 2), false);
    assert_eq!((inverted.vis_start, inverted.vis_end), (4, 4), "a visible range is never inverted");
}

#[test]
fn a_secure_field_exposes_only_its_length() {
    let a = text_ax(4, "hunter2", 0, 7, (0, 7), true);
    assert_eq!(a.selected_text("hunter2"), "", "the selected text of a secure field is empty");
    assert_eq!(a.len, 7);
    assert_eq!(masked(a.len), "\u{2022}".repeat(7));
}

#[test]
fn custom_actions_belong_to_one_element_and_refuse_duplicates() {
    let mut m = Map::new();
    let row = m.add("os/files", "row", "Report", "", true, None, "");
    let other = m.add("os/files", "row", "Invoice", "", true, None, "");
    assert!(m.add_custom_action(&row, "Move  up"), "whitespace collapses");
    assert!(m.add_custom_action(&row, "Archive"));
    assert!(!m.add_custom_action(&row, "move up"), "names compare without case");
    assert!(!m.add_custom_action(&row, "   "), "a blank name is refused");
    assert_eq!(m.custom_actions(&row).collect::<Vec<_>>(), ["Move up", "Archive"]);
    assert!(m.custom_actions(&other).next().is_none());
    assert!(m.has_custom_action(&row, " archive "));
}

#[test]
fn recorded_groups_make_a_parent_child_tree() {
    begin_frame();
    let list = begin_group(Role::List, "Inbox", r());
    let row = record(Role::Row, "Ada", "", r(), true);
    end_group();
    let top = record(Role::Button, "Compose", "", r(), true);
    let links = take_links();
    let nodes = take_frame();
    assert_eq!(parent_of(&links, row), Some(list));
    assert_eq!(parent_of(&links, top), None, "outside every group is a window child");
    assert_eq!(children_of(&nodes, &links, Some(list)), vec![row]);
    assert_eq!(children_of(&nodes, &links, None), vec![list, top]);
    assert_eq!(index_in_parent(&nodes, &links, row), Some(0));
}
