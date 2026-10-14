//! Parity tests for the preview row (sdk-tests-09): a view is rendered headless, offscreen, into an in-memory pixel
//! buffer and inspected without running the shell or an app process. The tests assert buffer sizes, pixel output and
//! accessibility elements, with a pinned clock.

use bifrost_app::testing::{pin, unix_secs, TestWindow};
use bifrost_app::view::{Button, Label, Stack};
use bifrost_core::pin::PINNED_UNIX_SECS;
use bifrost_ui::responder::Responder;
use bifrost_ui::Role;

#[derive(Default)]
struct Model {
    actions: Vec<String>,
}

impl Responder for Model {
    fn perform(&mut self, action: &str) -> bool {
        self.actions.push(action.into());
        true
    }
}

fn button_window(w: u32, h: u32) -> TestWindow<Model> {
    let root = Stack::vstack(8.0, vec![Box::new(Button::new("Save", "save"))]);
    TestWindow::new(root, Model::default(), w, h)
}

fn empty_window(w: u32, h: u32) -> TestWindow<Model> {
    TestWindow::new(Stack::vstack(8.0, Vec::new()), Model::default(), w, h)
}

#[test]
fn a_headless_preview_renders_the_view_into_a_pixel_buffer_of_the_window_size() {
    let mut w = button_window(320, 200);
    let px = w.draw();
    assert_eq!(px.len(), 320 * 200, "one packed pixel per window point");
}

#[test]
fn the_offscreen_render_of_a_button_differs_from_an_empty_window() {
    let with_button = button_window(320, 200).draw();
    let without = empty_window(320, 200).draw();
    assert_ne!(with_button, without, "the button leaves pixels behind");
}

#[test]
fn the_same_view_renders_the_same_pixels_twice() {
    let mut w = button_window(160, 90);
    let first = w.draw();
    let second = w.draw();
    assert_eq!(first, second, "a headless render of an unchanged view is deterministic");
}

#[test]
fn resizing_the_preview_resizes_the_pixel_buffer() {
    let mut w = button_window(320, 200);
    w.resize(200, 120);
    assert_eq!(w.draw().len(), 200 * 120);
}

#[test]
fn a_preview_exposes_its_accessibility_elements_without_drawing_them() {
    let mut w = button_window(320, 200);
    assert!(w.exists(Role::Button, "Save"));
    assert!(!w.exists(Role::Button, "Cancel"));
    assert_eq!(w.elements().iter().filter(|n| n.role == Role::Button).count(), 1);
}

#[test]
fn a_headless_preview_of_a_label_keeps_its_text_out_of_the_responder() {
    let root = Stack::vstack(8.0, vec![Box::new(Label::new("Hello"))]);
    let mut w = TestWindow::new(root, Model::default(), 120, 40);
    assert_eq!(w.draw().len(), 120 * 40);
    assert!(w.model.actions.is_empty(), "drawing performs no action");
}

#[test]
fn a_pinned_preview_reads_the_pinned_clock() {
    pin();
    assert_eq!(unix_secs(), PINNED_UNIX_SECS);
}
