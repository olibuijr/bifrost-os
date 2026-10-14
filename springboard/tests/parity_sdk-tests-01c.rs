//! Accessibility element mapping through the public API of `springboard`: the AT-SPI role and name of each element, its
//! states, the secure text field masking and the text edits a screen reader is told about. These are pure model checks:
//! no bus, no window and no display. The bridge that exports them is behind the `atspi` feature.

use bifrost_proto::{AxMeta, AxText, Node, Role, UiRect as Rect};
use springboard::shell::atspi::model::{atspi_role, build_ax, role, role_name, state, states, text_edits, window_states, TextEdit};

fn field(id: u64, label: &str, value: &str) -> Node {
    Node { id, role: Role::TextField, label: label.into(), value: value.into(), bounds: Rect::default(), enabled: true }
}

fn node(id: u64, kind: Role, value: &str) -> Node {
    Node { id, role: kind, label: "Item".into(), value: value.into(), bounds: Rect::default(), enabled: true }
}

fn has(bits: [u32; 2], bit: u32) -> bool {
    ((bits[(bit / 32) as usize] >> (bit % 32)) & 1) == 1
}

#[test]
fn each_element_role_maps_to_one_atspi_role_and_role_name() {
    let table = [
        (Role::Button, role::PUSH_BUTTON, "push button"),
        (Role::TextField, role::ENTRY, "entry"),
        (Role::List, role::LIST, "list"),
        (Role::Row, role::LIST_ITEM, "list item"),
        (Role::Toggle, role::TOGGLE_BUTTON, "toggle button"),
        (Role::Checkbox, role::CHECK_BOX, "check box"),
        (Role::Slider, role::SLIDER, "slider"),
        (Role::SegmentedControl, role::PAGE_TAB_LIST, "page tab list"),
        (Role::StaticText, role::LABEL, "label"),
        (Role::MenuItem, role::MENU_ITEM, "menu item"),
        (Role::Window, role::FRAME, "frame"),
        (Role::ProgressIndicator, role::PROGRESS_BAR, "progress bar"),
    ];
    for (kind, code, name) in table {
        assert_eq!(atspi_role(kind), code, "{kind:?} exports the AT-SPI role code");
        assert_eq!(role_name(kind), name, "{kind:?} is named by its AT-SPI role name");
    }
}

#[test]
fn a_secure_field_reads_as_password_and_its_value_is_only_bullets() {
    let text = AxText { id: 7, caret: 3, sel_start: 1, sel_end: 4, vis_start: 0, vis_end: 7, secure: true, len: 7 };
    let built = build_ax(&[field(7, "Password", "hunter2")], &[], &[text], &[]);
    assert_eq!(built.len(), 1);
    assert_eq!(built[0].value, "\u{2022}".repeat(7), "one bullet per character, no character of the secret");
    assert!(!built[0].value.contains("hunter"));
    assert_eq!(built[0].atspi_role(), role::PASSWORD_TEXT);
}

#[test]
fn a_plain_field_stays_an_entry_and_keeps_its_value() {
    let text = AxText { id: 7, caret: 0, sel_start: 0, sel_end: 0, vis_start: 0, vis_end: 3, secure: false, len: 3 };
    let built = build_ax(&[field(7, "Name", "Ada")], &[], &[text], &[]);
    assert_eq!(built[0].atspi_role(), role::ENTRY);
    assert_eq!(built[0].value, "Ada");
}

#[test]
fn the_name_falls_back_to_the_title_and_help_is_kept_apart() {
    let metas = [AxMeta { id: 7, title: "Search".into(), help: "Finds mail".into() }];
    let untitled = build_ax(&[field(7, "", "")], &[], &[], &metas);
    assert_eq!(untitled[0].accessible_name(), "Search", "no label: the visible title is the name");
    assert_eq!(untitled[0].help, "Finds mail");
    let labelled = build_ax(&[field(7, "Mail search", "")], &[], &[], &metas);
    assert_eq!(labelled[0].accessible_name(), "Mail search", "the label wins over the title");
    assert_eq!(labelled[0].title, "Search");
}

#[test]
fn text_edits_report_an_insert_a_delete_and_a_replace() {
    assert_eq!(text_edits("hello", "hello world"), vec![TextEdit { insert: true, at: 5, len: 6, text: " world".into() }]);
    assert_eq!(text_edits("hello world", "hello"), vec![TextEdit { insert: false, at: 5, len: 6, text: " world".into() }]);
    assert_eq!(
        text_edits("abcdef", "abXYef"),
        vec![TextEdit { insert: false, at: 2, len: 2, text: "cd".into() }, TextEdit { insert: true, at: 2, len: 2, text: "XY".into() }],
        "a replace is a delete, then an insert, at the same offset"
    );
    assert!(text_edits("same", "same").is_empty());
}

#[test]
fn an_enabled_button_is_visible_focusable_and_sensitive() {
    let built = build_ax(&[node(1, Role::Button, "")], &[], &[], &[]);
    let bits = states(&built[0]);
    for bit in [state::VISIBLE, state::SHOWING, state::ENABLED, state::SENSITIVE, state::FOCUSABLE] {
        assert!(has(bits, bit), "state bit {bit} is set");
    }
    assert!(!has(bits, state::CHECKABLE), "a button is not checkable");
    assert!(!has(bits, state::EDITABLE));
}

#[test]
fn static_text_is_not_focusable_and_a_disabled_control_is_not_enabled() {
    let text = build_ax(&[node(2, Role::StaticText, "")], &[], &[], &[]);
    assert!(!has(states(&text[0]), state::FOCUSABLE));
    let mut off = node(3, Role::Button, "");
    off.enabled = false;
    let built = build_ax(&[off], &[], &[], &[]);
    let bits = states(&built[0]);
    assert!(!has(bits, state::ENABLED) && !has(bits, state::FOCUSABLE));
    assert!(has(bits, state::VISIBLE), "a disabled control is still shown");
}

#[test]
fn a_checkbox_reads_its_checked_state_from_the_value_text() {
    let on = build_ax(&[node(4, Role::Checkbox, "on")], &[], &[], &[]);
    let bits = states(&on[0]);
    assert!(has(bits, state::CHECKABLE) && has(bits, state::CHECKED));
    let off = build_ax(&[node(5, Role::Checkbox, "off")], &[], &[], &[]);
    let bits = states(&off[0]);
    assert!(has(bits, state::CHECKABLE) && !has(bits, state::CHECKED));
}

#[test]
fn a_window_frame_is_active_and_showing() {
    let bits = window_states();
    assert!(has(bits, state::ACTIVE) && has(bits, state::SHOWING) && has(bits, state::VISIBLE));
    assert!(!has(bits, state::EDITABLE));
}
