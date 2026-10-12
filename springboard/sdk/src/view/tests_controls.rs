//! Contract tests for the the native toolkit-style controls in `controls.rs` and `forms.rs` that `tests.rs` does not cover:
//! `NativeButton`, `NativeTextField` (label), `NativeSwitch`, `NativeSlider`, `NativeSegmentedControl`, `NativeStackView`, grouped forms.
use super::*;
use crate::kit::{Font, Surface};
use crate::layout::ROW_PT;
use crate::proto::InputTraits;
use bifrost_ui::geometry::EdgeInsets;
use bifrost_ui::{values, Role};

fn ui() -> Ui {
    let data: &'static [u8] = Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/SFUI.ttf")).unwrap().into_boxed_slice());
    Ui::new(Font::parse(data).unwrap(), 1.0)
}

fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect { x, y, w, h }
}

#[derive(Default)]
struct Model {
    log: Vec<String>,
}
impl Responder for Model {
    fn perform(&mut self, action: &str) -> bool {
        self.log.push(action.into());
        true
    }
}

fn click(w: &mut Window<Model>, m: &mut Model, x: f32, y: f32) {
    w.event(&ViewEvent::Pointer(Event::Down(x, y)), m);
    w.event(&ViewEvent::Pointer(Event::Up(x, y)), m);
}

fn key(w: &mut Window<Model>, m: &mut Model, k: KeyInput) {
    w.event(&ViewEvent::Key(k), m);
}

fn text(s: &str) -> KeyInput {
    KeyInput::Text(s.into())
}

fn one<V: View + 'static>(v: V, size: (f32, f32)) -> (Window<Model>, u64) {
    let id = v.id();
    let mut w: Window<Model> = Window::new(v, 1);
    w.layout(&ui(), rect(0.0, 0.0, size.0, size.1));
    (w, id)
}

fn draws(w: &mut Window<Model>, wd: usize, ht: usize) {
    let mut px = vec![0u32; wd * ht];
    let mut s = Surface { w: wd, h: ht, stride: wd, px: &mut px, y0: 0 };
    w.draw(&mut s, &ui());
}

// ---- Label ----

#[test]
fn a_label_exposes_static_text_and_marks_itself_dirty_only_on_change() {
    let (mut w, id) = one(Label::new("Hello"), (100.0, 20.0));
    draws(&mut w, 100, 20);
    assert!(!w.needs_display());
    w.view_mut::<Label>(id).unwrap().set_text("Hello");
    assert!(!w.needs_display(), "same text: no redraw");
    w.view_mut::<Label>(id).unwrap().set_text("World");
    assert!(w.needs_display());
    assert_eq!(w.view::<Label>(id).unwrap().text(), "World");
    let n = &w.ax_nodes()[0];
    assert_eq!((n.role, n.label.as_str(), n.value.as_str()), (Role::StaticText, "World", ""));
    w.view_mut::<Label>(id).unwrap().secondary = true;
    w.view_mut::<Label>(id).unwrap().base_mut().enabled = false;
    draws(&mut w, 100, 20);
}

// ---- Button ----

#[test]
fn a_button_sends_on_click_return_and_space_through_the_chain() {
    let (mut w, id) = one(Button::new("Go", "go"), (80.0, 28.0));
    let mut m = Model::default();
    click(&mut w, &mut m, 10.0, 10.0);
    assert_eq!(w.take_sent(), vec![(id, "go".to_string())]);
    assert_eq!(m.log, vec!["go".to_string()]);
    w.make_first_responder(Some(id));
    key(&mut w, &mut m, KeyInput::Code(13));
    key(&mut w, &mut m, KeyInput::Code(32));
    key(&mut w, &mut m, text(" "));
    assert_eq!(w.take_sent().len(), 3);
    key(&mut w, &mut m, KeyInput::Code(65));
    assert!(w.take_sent().is_empty(), "other keys do nothing");
}

#[test]
fn a_button_cancelled_or_released_outside_does_not_send() {
    let (mut w, _) = one(Button::new("Go", "go"), (80.0, 28.0));
    let mut m = Model::default();
    w.event(&ViewEvent::Pointer(Event::Down(10.0, 10.0)), &mut m);
    w.event(&ViewEvent::Cancel, &mut m);
    w.event(&ViewEvent::Pointer(Event::Up(10.0, 10.0)), &mut m);
    assert!(w.take_sent().is_empty());
    w.event(&ViewEvent::Pointer(Event::Down(10.0, 10.0)), &mut m);
    w.event(&ViewEvent::Pointer(Event::Move(200.0, 10.0)), &mut m);
    w.event(&ViewEvent::Pointer(Event::Up(10.0, 10.0)), &mut m);
    assert!(w.take_sent().is_empty(), "dragged off the button: the press was dropped");
}

#[test]
fn a_disabled_button_ignores_pointer_and_keys_and_leaves_the_key_loop() {
    let mut b = Button::new("Go", "go");
    b.base_mut().enabled = false;
    let (mut w, _) = one(b, (80.0, 28.0));
    let mut m = Model::default();
    click(&mut w, &mut m, 10.0, 10.0);
    key(&mut w, &mut m, KeyInput::Code(13));
    assert!(w.take_sent().is_empty() && m.log.is_empty());
    assert!(w.key_view_loop().is_empty());
    let n = &w.ax_nodes()[0];
    assert!(!n.enabled && n.role == Role::Button && n.label == "Go");
    draws(&mut w, 80, 28);
}

#[test]
fn a_button_title_is_readable_settable_and_exposed() {
    let (mut w, id) = one(Button::new("Go", "go"), (80.0, 28.0));
    draws(&mut w, 80, 28);
    w.view_mut::<Button>(id).unwrap().set_title("Stop");
    assert!(w.needs_display());
    assert_eq!(w.view::<Button>(id).unwrap().title(), "Stop");
    assert_eq!(w.ax_nodes()[0].label, "Stop");
    w.make_first_responder(Some(id));
    draws(&mut w, 80, 28);
}

// ---- Switch ----

#[test]
fn a_switch_toggles_on_click_and_keys_and_reports_toggle_state() {
    let (mut w, id) = one(Switch::new("Wi-Fi", false, "wifi"), (120.0, 28.0));
    let mut m = Model::default();
    click(&mut w, &mut m, 10.0, 10.0);
    assert!(w.view::<Switch>(id).unwrap().on());
    assert_eq!(w.take_sent(), vec![(id, "wifi".to_string())]);
    w.make_first_responder(Some(id));
    key(&mut w, &mut m, KeyInput::Code(32));
    assert!(!w.view::<Switch>(id).unwrap().on());
    key(&mut w, &mut m, KeyInput::Code(13));
    assert!(w.view::<Switch>(id).unwrap().on());
    assert_eq!(w.take_sent().len(), 2);
    let n = &w.ax_nodes()[0];
    assert_eq!((n.role, n.label.as_str(), n.value.as_str()), (Role::Toggle, "Wi-Fi", "on"));
    assert_eq!(w.ax_ext()[0].checked, Some(true));
    draws(&mut w, 120, 28);
}

#[test]
fn a_switch_set_on_is_silent_and_release_outside_does_not_toggle() {
    let (mut w, id) = one(Switch::new("Wi-Fi", false, "wifi"), (120.0, 28.0));
    let mut m = Model::default();
    w.view_mut::<Switch>(id).unwrap().set_on(true);
    assert!(w.take_sent().is_empty(), "programmatic changes send no action");
    assert_eq!(w.ax_nodes()[0].value, "on");
    w.event(&ViewEvent::Pointer(Event::Down(10.0, 10.0)), &mut m);
    w.event(&ViewEvent::Pointer(Event::Up(500.0, 10.0)), &mut m);
    assert!(w.view::<Switch>(id).unwrap().on() && w.take_sent().is_empty());
}

#[test]
fn a_disabled_switch_ignores_input_and_a_bad_ax_value_is_rejected() {
    let mut sw = Switch::new("Wi-Fi", false, "wifi");
    sw.base_mut().enabled = false;
    let (mut w, id) = one(sw, (120.0, 28.0));
    let mut m = Model::default();
    click(&mut w, &mut m, 10.0, 10.0);
    key(&mut w, &mut m, KeyInput::Code(32));
    assert!(!w.view::<Switch>(id).unwrap().on() && w.take_sent().is_empty());
    let mut s = Switch::new("x", false, "x");
    assert!(s.ax_set_value("on") && s.on());
    assert!(s.ax_set_value("on") && s.on(), "setting the current value is a no-op");
    assert!(!s.ax_set_value("banana"));
}

// ---- Slider ----

fn slider(step: Option<f32>, value: f32) -> (Window<Model>, u64) {
    one(Slider::new("Volume", values::Slider::new(0.0, 10.0, value, step), "volume"), (118.0, 28.0))
}

#[test]
fn a_slider_drags_clamps_and_sends_only_when_the_value_changes() {
    let (mut w, id) = slider(None, 5.0);
    let mut m = Model::default();
    w.event(&ViewEvent::Pointer(Event::Down(117.0, 14.0)), &mut m);
    assert_eq!(w.view::<Slider>(id).unwrap().value(), 10.0, "clamped to max");
    w.event(&ViewEvent::Pointer(Event::Move(-300.0, 14.0)), &mut m);
    assert_eq!(w.view::<Slider>(id).unwrap().value(), 0.0, "clamped to min");
    w.event(&ViewEvent::Pointer(Event::Move(-400.0, 14.0)), &mut m);
    w.event(&ViewEvent::Pointer(Event::Up(-400.0, 14.0)), &mut m);
    let sent = w.take_sent();
    assert_eq!(sent.len(), 2, "the last move left the value at 0");
    assert!(sent.iter().all(|(i, a)| *i == id && a == "volume"));
}

#[test]
fn a_slider_arrows_nudge_one_step_and_stop_at_the_ends() {
    let (mut w, id) = slider(Some(2.0), 4.0);
    let mut m = Model::default();
    w.make_first_responder(Some(id));
    key(&mut w, &mut m, KeyInput::Code(39));
    assert_eq!(w.view::<Slider>(id).unwrap().value(), 6.0);
    for _ in 0..4 {
        key(&mut w, &mut m, KeyInput::Code(37));
    }
    assert_eq!(w.view::<Slider>(id).unwrap().value(), 0.0);
    assert_eq!(w.take_sent().len(), 4, "4 -> 6 -> 4 -> 2 -> 0; the fourth left press stayed at 0 but 6 -> 0 took 3 presses");
}

#[test]
fn a_slider_snaps_to_its_step_and_exposes_value_through_ax() {
    let (mut w, id) = slider(Some(2.0), 0.0);
    let mut m = Model::default();
    w.event(&ViewEvent::Pointer(Event::Down(59.0, 14.0)), &mut m);
    let v = w.view::<Slider>(id).unwrap().value();
    assert_eq!(v % 2.0, 0.0, "value {v} is a multiple of the step");
    let n = &w.ax_nodes()[0];
    assert_eq!((n.role, n.label.as_str()), (Role::Slider, "Volume"));
    assert_eq!(n.value, format!("{v}"));
    let s = w.view_mut::<Slider>(id).unwrap();
    assert!(s.ax_set_value("99") && s.value() == 10.0, "ax value clamps");
    assert!(!s.ax_set_value("loud"));
    draws(&mut w, 118, 28);
}

#[test]
fn a_disabled_slider_ignores_pointer_and_arrows() {
    let mut sl = Slider::new("Volume", values::Slider::new(0.0, 10.0, 5.0, None), "volume");
    sl.base_mut().enabled = false;
    let (mut w, id) = one(sl, (118.0, 28.0));
    let mut m = Model::default();
    click(&mut w, &mut m, 109.0, 14.0);
    key(&mut w, &mut m, KeyInput::Code(39));
    assert_eq!(w.view::<Slider>(id).unwrap().value(), 5.0);
    assert!(w.take_sent().is_empty());
    assert!(!w.ax_nodes()[0].enabled);
    draws(&mut w, 118, 28);
}

// ---- Segmented ----

fn segmented(sel: Option<usize>) -> (Window<Model>, u64) {
    one(Segmented::new(&["One", "Two", "Three"], sel, "seg"), (150.0, 28.0))
}

#[test]
fn a_segmented_control_selects_by_click_and_arrows() {
    let (mut w, id) = segmented(Some(0));
    let mut m = Model::default();
    click(&mut w, &mut m, 75.0, 14.0);
    assert_eq!(w.view::<Segmented>(id).unwrap().selected(), Some(1));
    click(&mut w, &mut m, 140.0, 14.0);
    assert_eq!(w.view::<Segmented>(id).unwrap().selected(), Some(2));
    w.make_first_responder(Some(id));
    key(&mut w, &mut m, KeyInput::Code(37));
    assert_eq!(w.view::<Segmented>(id).unwrap().selected(), Some(1));
    key(&mut w, &mut m, KeyInput::Code(39));
    assert_eq!(w.view::<Segmented>(id).unwrap().selected(), Some(2));
    let sent = w.take_sent();
    assert_eq!(sent.len(), 4);
    assert!(sent.iter().all(|(i, a)| *i == id && a == "seg"));
}

#[test]
fn a_segmented_click_on_the_current_segment_sends_nothing_and_cancel_clears_the_press() {
    let (mut w, id) = segmented(Some(1));
    let mut m = Model::default();
    click(&mut w, &mut m, 75.0, 14.0);
    assert!(w.take_sent().is_empty());
    w.event(&ViewEvent::Pointer(Event::Down(10.0, 14.0)), &mut m);
    w.event(&ViewEvent::Cancel, &mut m);
    w.event(&ViewEvent::Pointer(Event::Up(10.0, 14.0)), &mut m);
    assert_eq!(w.view::<Segmented>(id).unwrap().selected(), Some(1));
    assert!(w.take_sent().is_empty());
}

#[test]
fn a_segmented_control_exposes_labels_and_the_selected_one_through_ax() {
    let (mut w, _) = segmented(Some(2));
    let n = &w.ax_nodes()[0];
    assert_eq!((n.role, n.label.as_str(), n.value.as_str()), (Role::SegmentedControl, "One, Two, Three", "Three"));
    let (w2, _) = segmented(None);
    assert_eq!(w2.ax_nodes()[0].value, "");
    draws(&mut w, 150, 28);
}

#[test]
fn a_disabled_segmented_control_ignores_input() {
    let mut sg = Segmented::new(&["One", "Two"], Some(0), "seg");
    sg.base_mut().enabled = false;
    let (mut w, id) = one(sg, (100.0, 28.0));
    let mut m = Model::default();
    click(&mut w, &mut m, 75.0, 14.0);
    key(&mut w, &mut m, KeyInput::Code(39));
    assert_eq!(w.view::<Segmented>(id).unwrap().selected(), Some(0));
    assert!(w.take_sent().is_empty());
    draws(&mut w, 100, 28);
}

// ---- forms.rs gaps: disabled state, ax, clamping ----

#[test]
fn a_disabled_stepper_checkbox_and_text_field_ignore_input() {
    let mut m = Model::default();
    let mut st = Stepper::new("N", values::Stepper::new(1.0, 0.0, 5.0, 1.0), "n");
    st.base_mut().enabled = false;
    let (mut w, id) = one(st, (15.0, 22.0));
    click(&mut w, &mut m, 7.0, 4.0);
    w.make_first_responder(Some(id));
    key(&mut w, &mut m, KeyInput::Code(38));
    assert_eq!(w.view::<Stepper>(id).unwrap().value(), 1.0);

    let mut cb = Checkbox::new("C", false, "c");
    cb.base_mut().enabled = false;
    let (mut w, id) = one(cb, (100.0, ROW_PT));
    click(&mut w, &mut m, 5.0, 10.0);
    key(&mut w, &mut m, KeyInput::Code(32));
    assert!(!w.view::<Checkbox>(id).unwrap().checked() && w.take_sent().is_empty());

    let mut tf = TextField::new("Name", "name");
    tf.base_mut().enabled = false;
    let (mut w, id) = one(tf, (100.0, ROW_PT));
    click(&mut w, &mut m, 5.0, 10.0);
    key(&mut w, &mut m, text("a"));
    assert_eq!(w.view::<TextField>(id).unwrap().text(), "");
    assert!(m.log.is_empty());
}

#[test]
fn a_text_field_exposes_placeholder_and_value_and_set_text_is_silent() {
    let (mut w, id) = one(TextField::new("Name", "name").with_traits(InputTraits::default()), (120.0, ROW_PT));
    let n = &w.ax_nodes()[0];
    assert_eq!((n.role, n.label.as_str(), n.value.as_str()), (Role::TextField, "Name", ""));
    w.view_mut::<TextField>(id).unwrap().set_text("Ada");
    assert!(w.take_sent().is_empty());
    assert_eq!(w.ax_nodes()[0].value, "Ada");
    draws(&mut w, 120, ROW_PT as usize);
    w.make_first_responder(Some(id));
    draws(&mut w, 120, ROW_PT as usize);
}

#[test]
fn a_popup_button_exposes_title_and_open_state_and_disabled_does_not_open() {
    let (mut w, id) = one(PopUpButton::new(&["One", "Two"], 1, "p"), (100.0, ROW_PT));
    let n = &w.ax_nodes()[0];
    assert_eq!((n.role, n.label.as_str(), n.value.as_str()), (Role::Button, "Two", ""));
    w.view_mut::<PopUpButton>(id).unwrap().open_menu();
    assert_eq!(w.ax_nodes()[0].value, "open");
    w.view_mut::<PopUpButton>(id).unwrap().close_menu();
    assert!(!w.view::<PopUpButton>(id).unwrap().is_open());

    let mut p = PopUpButton::new(&["One", "Two"], 0, "p");
    p.base_mut().enabled = false;
    let (mut w, id) = one(p, (100.0, ROW_PT));
    let mut m = Model::default();
    click(&mut w, &mut m, 10.0, 10.0);
    assert!(!w.view::<PopUpButton>(id).unwrap().is_open());
    assert_eq!(w.view::<PopUpButton>(id).unwrap().selected(), 0);
}

#[test]
fn a_progress_indicator_clamps_its_fraction_and_exposes_percent() {
    let (mut w, id) = one(ProgressIndicator::bar(0.25), (100.0, 10.0));
    assert_eq!(w.ax_nodes()[0].role, Role::ProgressIndicator);
    assert_eq!(w.ax_nodes()[0].value, "25%");
    w.view_mut::<ProgressIndicator>(id).unwrap().set_fraction(7.0);
    assert_eq!(w.view::<ProgressIndicator>(id).unwrap().model.fraction(), Some(1.0));
    w.view_mut::<ProgressIndicator>(id).unwrap().set_fraction(-3.0);
    assert_eq!(w.view::<ProgressIndicator>(id).unwrap().model.fraction(), Some(0.0));
    assert!(!w.view::<ProgressIndicator>(id).unwrap().is_animating());
    w.view_mut::<ProgressIndicator>(id).unwrap().start_animation();
    assert!(w.view::<ProgressIndicator>(id).unwrap().is_animating());
    draws(&mut w, 100, 10);
    assert_eq!(ProgressIndicator::indeterminate_bar().model.label(), "");
    assert!(ProgressIndicator::spinner().is_animating());
}

// ---- Stack ----

#[test]
fn a_stack_aligns_children_across_its_axis() {
    let ui = ui();
    let mk = |a: Align| {
        let mut s = Stack::vstack(0.0, vec![Box::new(Button::new("OK", "ok"))]).aligned(a);
        s.layout(&ui, rect(0.0, 0.0, 300.0, 100.0));
        s.children()[0].frame()
    };
    let lead = mk(Align::Leading);
    assert_eq!(lead.x, 0.0);
    assert!(lead.w < 300.0);
    assert!((mk(Align::Center).x - (300.0 - lead.w) / 2.0).abs() < 1e-3);
    assert!((mk(Align::Trailing).right() - 300.0).abs() < 1e-3);
    assert_eq!(mk(Align::Fill).w, 300.0);
    assert_eq!((Align::TOP, Align::BOTTOM), (Align::Leading, Align::Trailing));
}

#[test]
fn a_stack_with_insets_and_spacing_stacks_children_in_order_and_routes_clicks() {
    let ui = ui();
    let (a, b) = (Button::new("A", "a"), Button::new("B", "b"));
    let (ia, ib) = (a.id(), b.id());
    let mut s = Stack::vstack(10.0, vec![Box::new(a), Box::new(b)]);
    s.insets = EdgeInsets { top: 5.0, ..EdgeInsets::ZERO };
    let mut w: Window<Model> = Window::new(s, 1);
    w.layout(&ui, rect(0.0, 0.0, 200.0, 200.0));
    let (fa, fb) = (w.view::<Button>(ia).unwrap().frame(), w.view::<Button>(ib).unwrap().frame());
    assert!(fa.y >= 5.0 && (fb.y - (fa.bottom() + 10.0)).abs() < 1e-3);
    let mut m = Model::default();
    click(&mut w, &mut m, fb.x + 2.0, fb.y + 2.0);
    assert_eq!(w.take_sent(), vec![(ib, "b".to_string())]);
    draws(&mut w, 200, 200);
}

// ---- Section and Form ----

#[test]
fn a_form_stacks_inset_sections_and_exposes_their_rows_to_ax() {
    let ui = ui();
    let (sw, cb) = (Switch::new("Wi-Fi", true, "wifi"), Checkbox::new("Share", false, "share"));
    let (si, ci) = (sw.id(), cb.id());
    let form = Form::new(vec![
        Section::new(vec![Box::new(sw)]).header("Network"),
        Section::new(vec![Box::new(cb), Box::new(Button::new("Reset", "reset"))]).footer("Careful"),
    ]);
    let mut w: Window<Model> = Window::new(form, 1);
    w.layout(&ui, rect(0.0, 0.0, 400.0, 400.0));
    let nodes = w.ax_nodes();
    assert_eq!(nodes.iter().map(|n| n.role).collect::<Vec<_>>(), vec![Role::Toggle, Role::Checkbox, Role::Button]);
    assert!(nodes[0].bounds.x > 0.0, "inset by the detail margin");
    assert!(nodes[1].bounds.y > nodes[0].bounds.bottom(), "sections stack in order");
    let mut m = Model::default();
    let c = w.view::<Switch>(si).unwrap().frame();
    click(&mut w, &mut m, c.x + 2.0, c.y + 2.0);
    assert!(!w.view::<Switch>(si).unwrap().on());
    let c = w.view::<Checkbox>(ci).unwrap().frame();
    click(&mut w, &mut m, c.x + 2.0, c.y + 2.0);
    assert!(w.view::<Checkbox>(ci).unwrap().checked());
    let actions: Vec<_> = w.take_sent().into_iter().map(|(_, a)| a).collect();
    assert_eq!(actions, vec!["wifi", "share"]);
    draws(&mut w, 400, 400);
}

#[test]
fn a_section_card_excludes_header_and_footer_and_grows_with_its_rows() {
    let ui = ui();
    let mut plain = Section::new(vec![Box::new(Label::new("a")), Box::new(Label::new("b"))]);
    plain.layout(&ui, rect(0.0, 0.0, 300.0, 0.0));
    let mut framed = Section::new(vec![Box::new(Label::new("a")), Box::new(Label::new("b"))]).header("H").footer("F");
    framed.layout(&ui, rect(0.0, 0.0, 300.0, 0.0));
    let (p, f) = (plain.card(), framed.card());
    assert_eq!(p.y, 0.0);
    assert!(f.y > 0.0, "the header sits above the card");
    assert_eq!(p.h, f.h);
    assert!(p.h >= 2.0 * ROW_PT);
    assert!(framed.frame().bottom() > f.bottom(), "the footer sits below the card");
    assert_eq!(plain.frame().h, p.h);
    assert_eq!(framed.intrinsic(&ui).1, framed.frame().h);
}

#[test]
fn a_form_with_no_sections_lays_out_and_draws() {
    let ui = ui();
    let f = Form::new(Vec::new());
    assert_eq!(f.intrinsic(&ui).1, 0.0);
    let mut w: Window<Model> = Window::new(f, 1);
    w.layout(&ui, rect(0.0, 0.0, 100.0, 100.0));
    draws(&mut w, 100, 100);
    assert!(w.ax_nodes().is_empty());
}
