//! The testing kit (XCTest parity) on a tiny view tree: button, text field, label queries.
use bifrost_app::assert_element;
use bifrost_app::proto::KeyInput;
use bifrost_app::testing::TestWindow;
use bifrost_app::view::{Button, Label, Stack, TextField};
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
