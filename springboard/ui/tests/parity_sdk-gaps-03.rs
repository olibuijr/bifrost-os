//! Parity sdk-gaps-03: headless model tests for markup adornments, ignored accessibility cells and the dictation entry.
//! Each test drives a pure model through its public API and asserts concrete values. Nothing here needs a display, a
//! device, the network or the clock.

use bifrost_ui::ax::Element;
use bifrost_ui::ax_ignored::{exposed, ignored_count, Cell, CellKind};
use bifrost_ui::dictation_entry::{Availability, Dictation, Reason, State};
use bifrost_ui::markup_adornment::{hit_test, place_all, Adornment, Anchor};
use bifrost_ui::Rect;

fn canvas() -> Rect {
    Rect { x: 0.0, y: 0.0, w: 400.0, h: 300.0 }
}

fn element(name: &str) -> Element {
    Element {
        address: format!("app/list/{name}"),
        name: name.into(),
        role: "row".into(),
        value: String::new(),
        enabled: true,
        bounds: None,
        handle: String::new(),
    }
}

fn cell(kind: CellKind, name: &str) -> Cell {
    Cell { kind, element: element(name) }
}

// Markup adornments: placement inside the canvas.

#[test]
fn a_top_leading_adornment_sits_at_its_inset_corner() {
    let a = Adornment::new(Anchor::TopLeading, 50.0, 20.0).with_inset(8.0);
    assert_eq!(a.frame_in(canvas()), Rect { x: 8.0, y: 8.0, w: 50.0, h: 20.0 });
}

#[test]
fn a_bottom_trailing_adornment_sits_at_its_inset_corner() {
    let a = Adornment::new(Anchor::BottomTrailing, 50.0, 20.0).with_inset(8.0);
    assert_eq!(a.frame_in(canvas()), Rect { x: 342.0, y: 272.0, w: 50.0, h: 20.0 });
}

#[test]
fn a_centred_adornment_ignores_its_inset() {
    let a = Adornment::new(Anchor::Center, 100.0, 60.0).with_inset(40.0);
    assert_eq!(a.frame_in(canvas()), Rect { x: 150.0, y: 120.0, w: 100.0, h: 60.0 });
}

#[test]
fn an_edge_anchor_centres_along_the_other_axis() {
    let top = Adornment::new(Anchor::TopCenter, 100.0, 20.0).frame_in(canvas());
    assert_eq!((top.x, top.y), (150.0, 0.0));
    let trailing = Adornment::new(Anchor::CenterTrailing, 100.0, 20.0).frame_in(canvas());
    assert_eq!((trailing.x, trailing.y), (300.0, 140.0));
}

#[test]
fn an_oversized_adornment_is_clipped_to_the_canvas() {
    let a = Adornment::new(Anchor::BottomCenter, 900.0, 500.0).with_inset(10.0);
    assert_eq!(a.frame_in(canvas()), Rect { x: 0.0, y: 0.0, w: 400.0, h: 300.0 });
}

#[test]
fn a_negative_inset_counts_as_no_inset() {
    let a = Adornment::new(Anchor::TopLeading, 10.0, 10.0).with_inset(-30.0);
    let f = a.frame_in(canvas());
    assert_eq!((f.x, f.y), (0.0, 0.0));
}

#[test]
fn a_canvas_offset_moves_every_adornment_frame() {
    let offset = Rect { x: 100.0, y: 50.0, w: 200.0, h: 100.0 };
    let a = Adornment::new(Anchor::TopLeading, 20.0, 10.0);
    assert_eq!(a.frame_in(offset), Rect { x: 100.0, y: 50.0, w: 20.0, h: 10.0 });
}

#[test]
fn place_all_returns_the_frames_in_the_order_given() {
    let list = [Adornment::new(Anchor::TopLeading, 10.0, 10.0), Adornment::new(Anchor::BottomTrailing, 20.0, 5.0)];
    let frames = place_all(&list, canvas());
    assert_eq!(frames.len(), 2);
    assert_eq!((frames[0].x, frames[0].y), (0.0, 0.0));
    assert_eq!((frames[1].x, frames[1].y), (380.0, 295.0));
}

#[test]
fn a_hit_on_overlapping_adornments_picks_the_one_drawn_last() {
    let frames = [Rect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 }, Rect { x: 50.0, y: 50.0, w: 100.0, h: 100.0 }];
    assert_eq!(hit_test(&frames, 60.0, 60.0), Some(1));
    assert_eq!(hit_test(&frames, 10.0, 10.0), Some(0));
    assert_eq!(hit_test(&frames, 500.0, 500.0), None);
}

// Ignored accessibility cells: image and text-field cells are hidden, the rest stay exposed.

#[test]
fn image_and_text_field_cells_are_ignored_by_assistive_technology() {
    assert!(CellKind::Image.is_ignored());
    assert!(CellKind::TextField.is_ignored());
    assert!(!CellKind::Text.is_ignored());
    assert!(!CellKind::Button.is_ignored());
}

#[test]
fn exposed_drops_ignored_cells_and_keeps_the_list_order() {
    let cells = [cell(CellKind::Image, "icon"), cell(CellKind::Text, "Office"), cell(CellKind::TextField, "Name"), cell(CellKind::Button, "Forget")];
    let names: Vec<&str> = exposed(&cells).iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, vec!["Office", "Forget"]);
}

#[test]
fn ignored_count_reports_how_many_cells_are_hidden() {
    let cells = [cell(CellKind::Image, "a"), cell(CellKind::Image, "b"), cell(CellKind::Text, "c")];
    assert_eq!(ignored_count(&cells), 2);
    assert_eq!(ignored_count(&[]), 0);
}

#[test]
fn a_list_of_only_image_cells_exposes_nothing() {
    let cells = [cell(CellKind::Image, "a")];
    assert!(exposed(&cells).is_empty());
}

// Dictation entry: listening starts only when dictation is available, and text is kept until it is taken.

#[test]
fn a_new_dictation_session_is_idle_with_no_text() {
    let d = Dictation::new();
    assert_eq!(d.state(), State::Idle);
    assert_eq!(d.text(), "");
}

#[test]
fn starting_while_dictation_is_unavailable_reports_the_reason() {
    let mut d = Dictation::new();
    assert_eq!(d.start(Availability::Unavailable(Reason::NoRecognizer)), Err(Reason::NoRecognizer));
    assert_eq!(d.start(Availability::Unavailable(Reason::Disabled)), Err(Reason::Disabled));
    assert_eq!(d.state(), State::Idle);
}

#[test]
fn starting_while_available_begins_listening() {
    let mut d = Dictation::new();
    assert_eq!(d.start(Availability::Available), Ok(()));
    assert_eq!(d.state(), State::Listening);
}

#[test]
fn phrases_are_joined_with_single_spaces_while_listening() {
    let mut d = Dictation::new();
    d.start(Availability::Available).unwrap();
    assert!(d.deliver("  hello "));
    assert!(d.deliver("world"));
    assert!(d.deliver("   "));
    assert_eq!(d.text(), "hello world");
}

#[test]
fn phrases_delivered_while_idle_are_dropped() {
    let mut d = Dictation::new();
    assert!(!d.deliver("early words"));
    assert_eq!(d.text(), "");
}

#[test]
fn stopping_keeps_the_text_until_it_is_taken() {
    let mut d = Dictation::new();
    d.start(Availability::Available).unwrap();
    d.deliver("draft");
    d.stop();
    assert_eq!(d.state(), State::Idle);
    assert_eq!(d.text(), "draft");
    assert_eq!(d.take_text(), "draft");
    assert_eq!(d.text(), "");
}

#[test]
fn a_second_start_while_listening_keeps_the_text_collected() {
    let mut d = Dictation::new();
    d.start(Availability::Available).unwrap();
    d.deliver("one");
    d.start(Availability::Available).unwrap();
    d.deliver("two");
    assert_eq!(d.text(), "one two");
}
