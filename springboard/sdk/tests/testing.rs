//! The testing kit (XCTest parity) on a tiny view tree: button, text field, label queries.
use bifrost_app::assert_element;
use bifrost_app::proto::KeyInput;
use bifrost_app::testing::TestWindow;
use bifrost_app::view::{Button, Label, ScrollView, Stack, TextField, View};
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

fn window() -> TestWindow<Model> {
    let root = Stack::vstack(8.0, vec![Box::new(Button::new("Save", "save")), Box::new(TextField::new("Name", "name")), Box::new(Label::new("Hello"))]);
    TestWindow::new(root, Model::default(), 320, 200)
}

#[test]
fn pressing_a_button_reaches_the_model() {
    let mut w = window();
    assert_element!(w, Role::Button, "Save");
    assert!(w.element(Role::Button, "Save").unwrap().press());
    assert!(w.model.actions.contains(&"save".to_string()));
    assert!(w.element(Role::Button, "Nope").is_none());
}

#[test]
fn clicking_the_frame_centre_hits_the_button() {
    let mut w = window();
    let f = w.element(Role::Button, "Save").unwrap().frame();
    assert!(f.w > 0.0 && f.h > 0.0);
    w.element(Role::Button, "Save").unwrap().click();
    assert!(w.model.actions.contains(&"save".to_string()));
}

#[test]
fn typing_into_a_text_field() {
    let mut w = window();
    w.element(Role::TextField, "Name").unwrap().click();
    w.type_text("Ada");
    assert_eq!(w.element(Role::TextField, "Name").unwrap().value(), "Ada");
    w.key(KeyInput::Code(8));
    assert_eq!(w.element(Role::TextField, "Name").unwrap().value(), "Ad");
    assert!(w.element(Role::TextField, "Name").unwrap().set_value("Grace"));
    assert_eq!(w.element(Role::TextField, "Name").unwrap().value(), "Grace");
}

#[test]
fn resize_and_draw_give_pixels() {
    let mut w = window();
    w.resize(200, 120);
    assert_eq!(w.draw().len(), 200 * 120);
}

#[test]
fn tap_at_a_point_presses_the_button_under_it() {
    let mut w = window();
    let f = w.element(Role::Button, "Save").unwrap().frame();
    assert!(w.tap(f.x + 1.0, f.y + 1.0));
    assert!(w.model.actions.contains(&"save".to_string()));
    assert!(!w.tap(-50.0, -50.0), "a point outside every view is not handled");
}

#[test]
fn type_into_taps_the_field_then_types() {
    let mut w = window();
    assert!(w.type_into(Role::TextField, "Name", "Ada"));
    assert_eq!(w.element(Role::TextField, "Name").unwrap().value(), "Ada");
    assert!(!w.type_into(Role::TextField, "Missing", "x"), "no such element: nothing is typed");
}

#[test]
fn swipe_pans_a_scroll_view_under_the_finger() {
    let rows: Vec<Box<dyn View>> = (0..40).map(|i| Box::new(Label::new(&format!("Row {i}"))) as Box<dyn View>).collect();
    let root = ScrollView::new(Box::new(Stack::vstack(8.0, rows)));
    let mut w = TestWindow::new(root, Model::default(), 320, 200);
    let offset = |w: &TestWindow<Model>| w.window.root().as_any().downcast_ref::<ScrollView>().unwrap().content_offset();
    assert_eq!(offset(&w), 0.0);
    assert!(w.swipe((100.0, 150.0), (100.0, 50.0)));
    assert!(offset(&w) > 0.0, "an upward swipe scrolls the content");
}
