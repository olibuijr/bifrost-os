//! Parity sdk-tests-18: headless tests of the pure UI models through their public API. Each model is a plain value
//! with no window, device or GPU behind it, so every test runs without a display. The transition and parameter view
//! models describe where things go and draw nothing; the tests assert their values, not pixels.

use bifrost_ui::anim::Curve;
use bifrost_ui::controller_power::{Battery, BatteryState, Color, Light};
use bifrost_ui::gamepad::{
    element, Axis, AxisRange, Button, Change, ControllerInput, Element, Gamepad, EV_ABS, EV_KEY, QUEUE_LIMIT,
};
use bifrost_ui::linalg::{Matrix4, Vector3};
use bifrost_ui::ml_model::{Layer, Model};
use bifrost_ui::ml_update::{update, Parameters, Report, Sample, UpdateError};
use bifrost_ui::param_view::{DisplayFlags, GenericView, ParamSpec, Scale};
use bifrost_ui::scene_object::{NodeId, SceneObjects, TransformOp};
use bifrost_ui::transition::{Ease, Kind, Run, Subtype, Transition, REDUCED_DURATION};
use std::cell::RefCell;
use std::rc::Rc;

const BTN_SOUTH: u16 = 0x130;
const BTN_EAST: u16 = 0x131;
const BTN_NORTH: u16 = 0x133;
const BTN_WEST: u16 = 0x134;
const BTN_TL: u16 = 0x136;
const BTN_TR: u16 = 0x137;
const BTN_TL2: u16 = 0x138;
const BTN_TR2: u16 = 0x139;
const BTN_SELECT: u16 = 0x13a;
const BTN_START: u16 = 0x13b;
const BTN_MODE: u16 = 0x13c;
const BTN_THUMBL: u16 = 0x13d;
const BTN_THUMBR: u16 = 0x13e;
const BTN_LEFT: u16 = 0x110;
const ABS_X: u16 = 0x00;
const ABS_Y: u16 = 0x01;
const ABS_Z: u16 = 0x02;
const ABS_RX: u16 = 0x03;
const ABS_RY: u16 = 0x04;
const ABS_RZ: u16 = 0x05;
const ABS_HAT0X: u16 = 0x10;
const ABS_HAT0Y: u16 = 0x11;

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-4
}

// ---- gamepad elements -------------------------------------------------------------------------------------------

#[test]
fn the_face_buttons_and_menu_keys_map_to_button_elements() {
    assert_eq!(element(EV_KEY, BTN_SOUTH), Some(Element::Button(Button::South)));
    assert_eq!(element(EV_KEY, BTN_EAST), Some(Element::Button(Button::East)));
    assert_eq!(element(EV_KEY, BTN_NORTH), Some(Element::Button(Button::North)));
    assert_eq!(element(EV_KEY, BTN_WEST), Some(Element::Button(Button::West)));
    assert_eq!(element(EV_KEY, BTN_START), Some(Element::Button(Button::Start)));
    assert_eq!(element(EV_KEY, BTN_SELECT), Some(Element::Button(Button::Select)));
    assert_eq!(element(EV_KEY, BTN_MODE), Some(Element::Button(Button::Home)));
}

#[test]
fn shoulders_triggers_and_stick_clicks_map_to_their_own_elements() {
    assert_eq!(element(EV_KEY, BTN_TL), Some(Element::Button(Button::LeftShoulder)));
    assert_eq!(element(EV_KEY, BTN_TR), Some(Element::Button(Button::RightShoulder)));
    assert_eq!(element(EV_KEY, BTN_TL2), Some(Element::Button(Button::LeftTrigger)));
    assert_eq!(element(EV_KEY, BTN_TR2), Some(Element::Button(Button::RightTrigger)));
    assert_eq!(element(EV_KEY, BTN_THUMBL), Some(Element::Button(Button::LeftStick)));
    assert_eq!(element(EV_KEY, BTN_THUMBR), Some(Element::Button(Button::RightStick)));
}

#[test]
fn sticks_and_analogue_triggers_map_to_axis_elements() {
    assert_eq!(element(EV_ABS, ABS_X), Some(Element::Axis(Axis::LeftX)));
    assert_eq!(element(EV_ABS, ABS_Y), Some(Element::Axis(Axis::LeftY)));
    assert_eq!(element(EV_ABS, ABS_RX), Some(Element::Axis(Axis::RightX)));
    assert_eq!(element(EV_ABS, ABS_RY), Some(Element::Axis(Axis::RightY)));
    assert_eq!(element(EV_ABS, ABS_Z), Some(Element::Axis(Axis::LeftTrigger)));
    assert_eq!(element(EV_ABS, ABS_RZ), Some(Element::Axis(Axis::RightTrigger)));
}

#[test]
fn the_direction_pad_hat_maps_to_two_dpad_elements() {
    assert_eq!(element(EV_ABS, ABS_HAT0X), Some(Element::DpadX));
    assert_eq!(element(EV_ABS, ABS_HAT0Y), Some(Element::DpadY));
}

#[test]
fn codes_that_are_not_gamepad_elements_map_to_nothing() {
    assert_eq!(element(EV_KEY, BTN_LEFT), None, "a pointer button is not a gamepad element");
    assert_eq!(element(EV_ABS, 0x06), None);
    assert_eq!(element(2, 0), None, "a record of another type is not an element");
}

#[test]
fn the_element_lists_hold_every_button_and_axis_once() {
    assert_eq!(Button::ALL.len(), 13);
    assert_eq!(Axis::ALL.len(), 6);
    for (i, a) in Button::ALL.iter().enumerate() {
        assert!(!Button::ALL[i + 1..].contains(a), "{a:?} is listed twice");
    }
}

#[test]
fn a_button_reports_each_edge_once_and_ignores_key_repeats() {
    let mut pad = Gamepad::new();
    assert_eq!(pad.feed(EV_KEY, BTN_SOUTH, 1), Some(Change::Button { button: Button::South, pressed: true }));
    assert!(pad.is_pressed(Button::South));
    assert_eq!(pad.feed(EV_KEY, BTN_SOUTH, 2), None, "value 2 is a repeat");
    assert_eq!(pad.feed(EV_KEY, BTN_SOUTH, 1), None, "a held button reports nothing");
    assert_eq!(pad.feed(EV_KEY, BTN_SOUTH, 0), Some(Change::Button { button: Button::South, pressed: false }));
    assert!(!pad.is_pressed(Button::South));
    assert!(!pad.is_pressed(Button::East));
}

#[test]
fn a_centred_stick_maps_its_ends_to_full_deflection_and_its_middle_to_rest() {
    let mut pad = Gamepad::new();
    pad.set_range(Axis::LeftX, AxisRange::stick(0, 200, 0.2));
    assert_eq!(pad.feed(EV_ABS, ABS_X, 0), Some(Change::Axis { axis: Axis::LeftX, value: -1.0 }));
    assert_eq!(pad.feed(EV_ABS, ABS_X, 200), Some(Change::Axis { axis: Axis::LeftX, value: 1.0 }));
    assert_eq!(pad.feed(EV_ABS, ABS_X, 100), Some(Change::Axis { axis: Axis::LeftX, value: 0.0 }));
}

#[test]
fn a_stick_past_its_dead_zone_is_rescaled_with_no_step_at_the_edge() {
    let mut pad = Gamepad::new();
    pad.set_range(Axis::LeftX, AxisRange::stick(0, 200, 0.2));
    // raw 150 is v = 0.5; (0.5 - 0.2) / 0.8 = 0.375
    pad.feed(EV_ABS, ABS_X, 150);
    assert!(close(pad.axis(Axis::LeftX), 0.375));
    // raw 121 is v = 0.21, just past the dead zone: (0.21 - 0.2) / 0.8 = 0.0125
    pad.feed(EV_ABS, ABS_X, 121);
    assert!(close(pad.axis(Axis::LeftX), 0.0125));
}

#[test]
fn a_degenerate_axis_range_reads_as_rest() {
    assert_eq!(AxisRange::stick(5, 5, 0.1).normalize(5), 0.0);
    assert_eq!(AxisRange::trigger(9, 3, 0.0).normalize(4), 0.0);
}

#[test]
fn the_default_stick_range_is_signed_sixteen_bit() {
    let mut pad = Gamepad::new();
    pad.feed(EV_ABS, ABS_RX, 32767);
    assert!(close(pad.axis(Axis::RightX), 1.0));
    pad.feed(EV_ABS, ABS_RX, -32768);
    assert!(close(pad.axis(Axis::RightX), -1.0));
}

#[test]
fn a_trigger_is_unipolar_and_reads_zero_inside_its_dead_zone() {
    let mut pad = Gamepad::new();
    assert_eq!(pad.feed(EV_ABS, ABS_Z, 255), Some(Change::Axis { axis: Axis::LeftTrigger, value: 1.0 }));
    // 6 / 255 is under the 5 per cent dead zone
    pad.feed(EV_ABS, ABS_Z, 6);
    assert_eq!(pad.axis(Axis::LeftTrigger), 0.0);
    // 128 / 255 rescaled over the 0.95 past the dead zone
    pad.feed(EV_ABS, ABS_Z, 128);
    assert!(close(pad.axis(Axis::LeftTrigger), (128.0 / 255.0 - 0.05) / 0.95));
}

#[test]
fn an_axis_reports_only_when_its_normalised_value_moves() {
    let mut pad = Gamepad::new();
    assert_eq!(pad.feed(EV_ABS, ABS_RX, 1000), None, "inside the dead zone the value stays at rest");
    assert_eq!(pad.feed(EV_ABS, ABS_RX, 1200), None);
    assert_eq!(pad.axis(Axis::RightX), 0.0);
    assert!(pad.feed(EV_ABS, ABS_RX, 16000).is_some());
    assert_eq!(pad.feed(EV_ABS, ABS_RX, 16000), None, "the same raw value is not a change");
}

#[test]
fn the_direction_pad_reports_each_change_and_clamps_out_of_range_values() {
    let mut pad = Gamepad::new();
    assert_eq!(pad.feed(EV_ABS, ABS_HAT0X, -1), Some(Change::Dpad { x: -1, y: 0 }));
    assert_eq!(pad.feed(EV_ABS, ABS_HAT0Y, 1), Some(Change::Dpad { x: -1, y: 1 }));
    assert_eq!(pad.feed(EV_ABS, ABS_HAT0X, -1), None);
    assert_eq!(pad.feed(EV_ABS, ABS_HAT0X, 0), Some(Change::Dpad { x: 0, y: 1 }));
    assert_eq!(pad.dpad(), (0, 1));
    assert_eq!(pad.feed(EV_ABS, ABS_HAT0Y, 5), None, "5 clamps to 1, which is already held");
}

#[test]
fn a_controller_input_calls_back_once_per_change_in_feed_order() {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let mut input = ControllerInput::new();
    let sink = Rc::clone(&seen);
    input.on_change(move |c| sink.borrow_mut().push(*c));
    input.feed(EV_KEY, BTN_SOUTH, 1);
    input.feed(EV_KEY, BTN_SOUTH, 2);
    input.feed(EV_KEY, BTN_SOUTH, 1);
    input.feed(EV_KEY, BTN_SOUTH, 0);
    assert_eq!(
        seen.borrow().clone(),
        vec![
            Change::Button { button: Button::South, pressed: true },
            Change::Button { button: Button::South, pressed: false },
        ]
    );
}

#[test]
fn polling_returns_queued_changes_in_order_and_then_nothing() {
    let mut input = ControllerInput::new();
    assert_eq!(input.feed(EV_ABS, ABS_HAT0X, -1), Some(Change::Dpad { x: -1, y: 0 }));
    assert_eq!(input.feed(EV_ABS, ABS_HAT0Y, 1), Some(Change::Dpad { x: -1, y: 1 }));
    assert_eq!(input.pending(), 2);
    assert_eq!(input.poll(), Some(Change::Dpad { x: -1, y: 0 }));
    assert_eq!(input.poll(), Some(Change::Dpad { x: -1, y: 1 }));
    assert_eq!(input.poll(), None);
    assert_eq!(input.pending(), 0);
}

#[test]
fn a_full_queue_drops_the_oldest_changes_and_keeps_the_newest() {
    let mut input = ControllerInput::new();
    // presses and releases alternate, so change i is a press when i is even
    for i in 0..300 {
        let value = if i % 2 == 0 { 1 } else { 0 };
        assert!(input.feed(EV_KEY, BTN_SOUTH, value).is_some());
    }
    assert_eq!(input.pending(), QUEUE_LIMIT);
    // changes 0 to 43 were dropped, so the oldest left is change 44, a press
    assert_eq!(input.poll(), Some(Change::Button { button: Button::South, pressed: true }));
}

#[test]
fn a_captured_snapshot_does_not_follow_later_changes() {
    let mut input = ControllerInput::new();
    input.feed(EV_KEY, BTN_SOUTH, 1);
    let snapshot = input.capture();
    input.feed(EV_KEY, BTN_SOUTH, 0);
    assert!(snapshot.is_pressed(Button::South));
    assert!(!input.gamepad().is_pressed(Button::South));
}

// ---- battery and light ------------------------------------------------------------------------------------------

#[test]
fn a_reported_percentage_becomes_a_level_and_reads_back_as_a_percentage() {
    let half = Battery::from_percent(72, BatteryState::Discharging);
    assert_eq!(half.percent(), Some(72));
    assert_eq!(half.state, BatteryState::Discharging);
    let over = Battery::from_percent(250, BatteryState::Full);
    assert_eq!(over.level, Some(1.0), "a percentage above 100 reads as 100");
}

#[test]
fn a_battery_with_no_report_has_no_level_and_no_percentage() {
    let unknown = Battery::unknown();
    assert_eq!(unknown.level, None);
    assert_eq!(unknown.state, BatteryState::Unknown);
    assert_eq!(unknown.percent(), None);
}

#[test]
fn a_battery_level_rounds_to_the_nearest_whole_percent() {
    assert_eq!(Battery { level: Some(0.625), state: BatteryState::Charging }.percent(), Some(63));
    assert_eq!(Battery { level: Some(0.004), state: BatteryState::Discharging }.percent(), Some(0));
}

#[test]
fn charging_and_full_are_different_battery_states() {
    assert_ne!(BatteryState::Charging, BatteryState::Full);
}

#[test]
fn colour_components_clamp_to_the_unit_range_and_nan_becomes_zero() {
    let c = Color::new(1.5, -0.2, 0.5);
    assert_eq!(c, Color { red: 1.0, green: 0.0, blue: 0.5 });
    assert_eq!(Color::new(f32::NAN, 0.25, 0.0).red, 0.0);
}

#[test]
fn eight_bit_colour_components_round_trip_and_round_half_away_from_zero() {
    assert_eq!(Color::from_8bit(255, 0, 128).to_8bit(), [255, 0, 128]);
    // 0.5 * 255 = 127.5 rounds up to 128
    assert_eq!(Color::new(0.5, 0.5, 0.5).to_8bit(), [128, 128, 128]);
}

#[test]
fn a_light_starts_black_and_takes_the_colour_it_is_set_to() {
    let mut light = Light::new();
    assert_eq!(light.color.to_8bit(), [0, 0, 0]);
    light.set_color(Color::from_8bit(0, 0, 255));
    assert_eq!(light.color.to_8bit(), [0, 0, 255]);
    light.set_color(Color::new(2.0, 0.0, 0.0));
    assert_eq!(light.color.red, 1.0, "the stored colour is the clamped one");
}

// ---- 3D object hierarchy and transforms -------------------------------------------------------------------------

#[test]
fn a_child_moves_with_its_parent_in_the_world_but_not_in_its_local_space() {
    let mut s = SceneObjects::new();
    let parent = s.add_root("body").unwrap();
    s.push_op(parent, TransformOp::Translate(Vector3::new(1.0, 0.0, 0.0)));
    let child = s.add_child(parent, "arm").unwrap();
    s.push_op(child, TransformOp::Translate(Vector3::new(0.0, 2.0, 0.0)));
    let world = s.world_matrix(child).unwrap().transform_point(Vector3::ZERO).unwrap();
    assert!(close(world.x, 1.0) && close(world.y, 2.0) && close(world.z, 0.0));
    let local = s.local_matrix(child).unwrap().transform_point(Vector3::ZERO).unwrap();
    assert!(close(local.y, 2.0) && close(local.x, 0.0));
}

#[test]
fn the_first_listed_operation_is_the_outermost_one() {
    let mut s = SceneObjects::new();
    let r = s.add_root("a").unwrap();
    s.push_op(r, TransformOp::Translate(Vector3::new(1.0, 0.0, 0.0)));
    s.push_op(r, TransformOp::Scale(Vector3::new(2.0, 2.0, 2.0)));
    // the scale is applied first: x = 1 becomes 2, then the translation gives 3
    let p = s.local_matrix(r).unwrap().transform_point(Vector3::new(1.0, 0.0, 0.0)).unwrap();
    assert!(close(p.x, 3.0));
}

#[test]
fn a_quarter_turn_about_z_takes_x_to_y() {
    let mut s = SceneObjects::new();
    let r = s.add_root("spin").unwrap();
    s.push_op(r, TransformOp::Rotate { radians: std::f32::consts::FRAC_PI_2, axis: Vector3::new(0.0, 0.0, 1.0) });
    let p = s.local_matrix(r).unwrap().transform_point(Vector3::new(1.0, 0.0, 0.0)).unwrap();
    assert!(close(p.x, 0.0) && close(p.y, 1.0));
}

#[test]
fn a_matrix_operation_is_applied_exactly_as_given() {
    let mut s = SceneObjects::new();
    let r = s.add_root("m").unwrap();
    let m = Matrix4::translation(5.0, 0.0, 0.0);
    s.push_op(r, TransformOp::Matrix(m));
    assert_eq!(s.local_matrix(r), Some(m));
}

#[test]
fn a_node_with_no_operations_has_the_identity_transform() {
    let mut s = SceneObjects::new();
    let r = s.add_root("empty").unwrap();
    assert_eq!(s.local_matrix(r), Some(Matrix4::identity()));
    assert_eq!(s.world_matrix(r), Some(Matrix4::identity()));
}

#[test]
fn node_paths_join_the_names_from_the_root_and_resolve_back() {
    let mut s = SceneObjects::new();
    let world = s.add_root("world").unwrap();
    let car = s.add_child(world, "car").unwrap();
    let wheel = s.add_child(car, "wheel").unwrap();
    assert_eq!(s.path(wheel).as_deref(), Some("world/car/wheel"));
    assert_eq!(s.find("world/car/wheel"), Some(wheel));
    assert_eq!(s.node(wheel).map(|n| n.name.as_str()), Some("wheel"));
}

#[test]
fn find_returns_nothing_for_a_missing_segment_an_empty_path_or_a_double_slash() {
    let mut s = SceneObjects::new();
    let world = s.add_root("world").unwrap();
    s.add_child(world, "car").unwrap();
    assert_eq!(s.find("world/missing"), None);
    assert_eq!(s.find(""), None);
    assert_eq!(s.find("world//car"), None);
}

#[test]
fn a_hidden_node_hides_everything_under_it_until_it_is_shown_again() {
    let mut s = SceneObjects::new();
    let room = s.add_root("room").unwrap();
    let lamp = s.add_child(room, "lamp").unwrap();
    assert_eq!(s.is_rendered(lamp), Some(true));
    assert!(s.set_hidden(room, true));
    assert_eq!(s.is_rendered(lamp), Some(false));
    assert!(s.set_hidden(room, false));
    assert_eq!(s.is_rendered(lamp), Some(true));
}

#[test]
fn descendants_come_depth_first_in_child_order_without_the_node_itself() {
    let mut s = SceneObjects::new();
    let r = s.add_root("r").unwrap();
    let c1 = s.add_child(r, "c1").unwrap();
    let g = s.add_child(c1, "g").unwrap();
    let c2 = s.add_child(r, "c2").unwrap();
    assert_eq!(s.descendants(r), vec![c1, g, c2]);
    assert!(s.descendants(c2).is_empty());
    assert_eq!(s.len(), 4);
    assert!(!s.is_empty());
}

#[test]
fn names_with_a_slash_or_no_characters_are_refused() {
    let mut s = SceneObjects::new();
    assert_eq!(s.add_root(""), None);
    assert_eq!(s.add_root("a/b"), None);
    let r = s.add_root("ok").unwrap();
    assert_eq!(s.add_child(r, ""), None);
    assert_eq!(s.len(), 1);
}

#[test]
fn a_missing_node_cannot_take_a_parent_an_operation_or_a_visibility_change() {
    let mut s = SceneObjects::new();
    let r = s.add_root("ok").unwrap();
    assert_eq!(s.add_child(NodeId(99), "x"), None);
    assert!(!s.push_op(NodeId(7), TransformOp::Scale(Vector3::new(1.0, 1.0, 1.0))));
    assert!(!s.set_hidden(NodeId(7), true));
    assert_eq!(s.local_matrix(NodeId(7)), None);
    assert_eq!(s.is_rendered(r), Some(true));
}

// ---- on-device model updates ------------------------------------------------------------------------------------

fn one_dense(weights: Vec<f32>, bias: Vec<f32>) -> Model {
    Model::new("x", 2).unwrap().with_output("y").dense(2, weights, bias).unwrap()
}

fn sample(input: Vec<f32>, label: usize) -> Sample {
    Sample { input, label }
}

fn dense_of(model: &Model, index: usize) -> (Vec<f32>, Vec<f32>) {
    match &model.layers()[index] {
        Layer::Dense { weights, bias, .. } => (weights.clone(), bias.clone()),
        other => panic!("expected a dense layer, found {other:?}"),
    }
}

#[test]
fn one_step_moves_the_weights_and_bias_by_the_softmax_gradient() {
    // zero weights give p = [0.5, 0.5]; with rate 1 and input [1, 0] and label 0 the step is
    // bias [0.5, -0.5] and weight row 0 [0.5, -0.5], with row 1 still zero
    let model = one_dense(vec![0.0; 4], vec![0.0; 2]);
    let params = Parameters { learning_rate: 1.0, epochs: 1 };
    let (out, _) = update(&model, &[sample(vec![1.0, 0.0], 0)], &params).unwrap();
    assert_eq!(dense_of(&out, 0), (vec![0.5, -0.5, 0.0, 0.0], vec![0.5, -0.5]));
}

#[test]
fn the_report_gives_the_loss_before_and_after_one_step() {
    // uniform two-class loss is ln 2; after one step the logits are [1, -1], loss ln(e + 1/e) - 1
    let model = one_dense(vec![0.0; 4], vec![0.0; 2]);
    let params = Parameters { learning_rate: 1.0, epochs: 1 };
    let (_, report) = update(&model, &[sample(vec![1.0, 0.0], 0)], &params).unwrap();
    assert!((report.loss_before - 0.6931).abs() < 1e-3, "before {}", report.loss_before);
    assert!((report.loss_after - 0.1269).abs() < 1e-3, "after {}", report.loss_after);
    assert_eq!((report.samples, report.epochs), (1, 1));
}

#[test]
fn a_second_epoch_lowers_the_loss_further() {
    let model = one_dense(vec![0.0; 4], vec![0.0; 2]);
    let samples = [sample(vec![1.0, 0.0], 0)];
    let (_, one) = update(&model, &samples, &Parameters { learning_rate: 1.0, epochs: 1 }).unwrap();
    let (_, two) = update(&model, &samples, &Parameters { learning_rate: 1.0, epochs: 2 }).unwrap();
    assert!(two.loss_after < one.loss_after);
    assert!((two.loss_after - 0.0807).abs() < 2e-3, "after {}", two.loss_after);
}

#[test]
fn an_update_returns_a_new_model_and_leaves_the_old_one_unchanged() {
    let model = one_dense(vec![0.0; 4], vec![0.0; 2]);
    let params = Parameters { learning_rate: 1.0, epochs: 1 };
    let (out, _) = update(&model, &[sample(vec![1.0, 0.0], 0)], &params).unwrap();
    assert_eq!(dense_of(&model, 0), (vec![0.0; 4], vec![0.0; 2]));
    assert_ne!(dense_of(&out, 0), dense_of(&model, 0));
}

#[test]
fn the_same_model_samples_and_parameters_always_give_the_same_report() {
    let model = one_dense(vec![0.1, -0.2, 0.3, 0.4], vec![0.0, 0.0]);
    let samples = [sample(vec![1.0, 0.5], 1), sample(vec![-1.0, 0.25], 0)];
    let params = Parameters { learning_rate: 0.5, epochs: 3 };
    let first: Report = update(&model, &samples, &params).unwrap().1;
    let second: Report = update(&model, &samples, &params).unwrap().1;
    assert_eq!(first, second);
}

#[test]
fn frozen_layers_before_the_trained_layer_are_kept_as_they_were() {
    let model = Model::new("x", 2)
        .unwrap()
        .with_output("y")
        .dense(2, vec![1.0, 0.0, 0.0, 1.0], vec![0.0, 0.0])
        .unwrap()
        .relu()
        .dense(2, vec![0.0; 4], vec![0.0; 2])
        .unwrap();
    let params = Parameters { learning_rate: 1.0, epochs: 1 };
    let (out, _) = update(&model, &[sample(vec![1.0, 0.0], 0)], &params).unwrap();
    assert_eq!(out.layers()[0], model.layers()[0]);
    assert_eq!(out.layers()[1], Layer::Relu);
    assert_eq!(dense_of(&out, 2), (vec![0.5, -0.5, 0.0, 0.0], vec![0.5, -0.5]));
    assert_eq!((out.input_name(), out.output_name()), ("x", Some("y")));
}

#[test]
fn bad_samples_and_parameters_are_rejected_with_the_reason() {
    let model = one_dense(vec![0.0; 4], vec![0.0; 2]);
    let good = Parameters::default();
    assert_eq!(update(&model, &[], &good).unwrap_err(), UpdateError::Empty);
    let zero_epochs = Parameters { learning_rate: 0.1, epochs: 0 };
    assert_eq!(update(&model, &[sample(vec![1.0, 0.0], 0)], &zero_epochs).unwrap_err(), UpdateError::Epochs);
    let zero_rate = Parameters { learning_rate: 0.0, epochs: 1 };
    assert_eq!(update(&model, &[sample(vec![1.0, 0.0], 0)], &zero_rate).unwrap_err(), UpdateError::LearningRate);
    let short = [sample(vec![1.0, 0.0], 0), sample(vec![1.0], 1)];
    assert_eq!(
        update(&model, &short, &good).unwrap_err(),
        UpdateError::InputLength { sample: 1, expected: 2, found: 1 }
    );
    let bad_label = [sample(vec![1.0, 0.0], 2)];
    assert_eq!(update(&model, &bad_label, &good).unwrap_err(), UpdateError::Label { sample: 0, label: 2, classes: 2 });
}

#[test]
fn a_model_without_a_dense_layer_cannot_be_updated() {
    let model = Model::new("x", 2).unwrap().relu();
    assert_eq!(
        update(&model, &[sample(vec![1.0, 0.0], 0)], &Parameters::default()).unwrap_err(),
        UpdateError::NoDenseLayer
    );
}

// ---- transitions ------------------------------------------------------------------------------------------------

#[test]
fn a_fade_changes_only_the_incoming_opacity() {
    let t = Transition::fade().ease(Ease::Linear);
    let p = t.placement(0.25, 100.0, 50.0);
    assert_eq!((p.under_x, p.over_x, p.under_alpha, p.over_alpha), (0.0, 0.0, 1.0, 0.25));
    assert_eq!(t.placement(1.0, 100.0, 50.0).over_alpha, 1.0);
}

#[test]
fn a_move_in_slides_the_incoming_content_from_the_named_side() {
    let from_bottom = Transition::move_in(Subtype::FromBottom).ease(Ease::Linear);
    let start = from_bottom.placement(0.0, 100.0, 50.0);
    assert_eq!((start.over_x, start.over_y, start.under_y), (0.0, 50.0, 0.0));
    assert_eq!(from_bottom.placement(1.0, 100.0, 50.0).over_y, 0.0);
    let from_left = Transition::move_in(Subtype::FromLeft).ease(Ease::Linear).placement(0.5, 100.0, 50.0);
    assert_eq!(from_left.over_x, -50.0);
    let from_top = Transition::move_in(Subtype::FromTop).ease(Ease::Linear).placement(0.0, 100.0, 50.0);
    assert_eq!(from_top.over_y, -50.0);
}

#[test]
fn a_push_moves_both_pieces_in_opposite_directions() {
    let p = Transition::push(Subtype::FromRight).ease(Ease::Linear).placement(0.5, 100.0, 50.0);
    assert_eq!((p.over_x, p.under_x), (50.0, -50.0));
    assert!(p.over_on_top);
}

#[test]
fn a_reveal_moves_the_outgoing_piece_and_draws_the_incoming_one_below() {
    let p = Transition::reveal(Subtype::FromRight).ease(Ease::Linear).placement(0.5, 100.0, 50.0);
    assert_eq!((p.under_x, p.over_x), (-50.0, 0.0));
    assert!(!p.over_on_top);
}

#[test]
fn a_reversed_push_is_a_pop_with_parallax_and_shade() {
    let pop = Transition::push(Subtype::FromRight).parallax(0.3, 0.12).ease(Ease::OutQuartic).reversed();
    let start = pop.placement(0.0, 1000.0, 0.0);
    assert_eq!((start.under_x.round(), start.over_x, start.shade_u8()), (-300.0, 0.0, 30));
    let end = pop.placement(1.0, 1000.0, 0.0);
    assert_eq!((end.under_x, end.over_x, end.shade_u8()), (0.0, 1000.0, 0));
}

#[test]
fn a_range_is_clamped_to_the_unit_interval_and_a_nan_start_is_zero() {
    let t = Transition::fade().range(-1.0, 3.0);
    assert_eq!((t.start_progress, t.end_progress), (0.0, 1.0));
    assert_eq!(Transition::fade().range(f32::NAN, 0.5).start_progress, 0.0);
}

#[test]
fn progress_follows_the_clock_and_a_zero_duration_is_already_done() {
    let t = Transition::fade().duration(2.0);
    assert_eq!(t.progress(1.0, 0.0), 0.5);
    assert_eq!(t.progress(10.0, 0.0), 1.0);
    assert_eq!(t.progress(-5.0, 0.0), 0.0);
    assert_eq!(Transition::fade().duration(0.0).progress(0.0, 0.0), 1.0);
    assert_eq!(Transition::fade().duration(f32::NAN).duration, 0.0);
    assert_eq!(Transition::fade().duration(-3.0).duration, 0.0);
}

#[test]
fn every_ease_runs_from_zero_to_one_and_out_cubic_leads_linear() {
    for e in [Ease::Linear, Ease::OutCubic, Ease::OutQuartic, Ease::Curve(Curve::EaseInOut)] {
        assert!(e.at(0.0).abs() < 1e-6 && (e.at(1.0) - 1.0).abs() < 1e-6, "{e:?}");
    }
    assert!(close(Ease::OutCubic.at(0.5), 0.875));
    assert_eq!(Ease::Linear.at(f32::NAN), 0.0);
}

#[test]
fn a_run_is_done_once_its_duration_has_passed() {
    let run = Run::new(Transition::fade().duration(0.5).ease(Ease::Linear), 10.0);
    assert!(!run.is_done(10.2));
    assert!(run.is_done(10.5));
    assert!(close(run.placement(10.25, 100.0, 50.0).over_alpha, 0.5), "halfway through a linear fade is half opaque");
}

#[test]
fn the_reduced_motion_transition_is_a_short_linear_fade() {
    let reduced = Transition::reduced();
    assert_eq!(reduced.kind, Kind::Fade);
    assert_eq!(reduced.duration, REDUCED_DURATION);
    assert_eq!(reduced.ease, Ease::Linear);
}

#[test]
fn parallax_clamps_its_shift_and_its_dim() {
    let t = Transition::push(Subtype::FromRight).parallax(-1.0, 2.0);
    assert_eq!((t.under_shift, t.under_dim), (0.0, 1.0));
}

#[test]
fn subtypes_point_from_the_centre_to_their_side() {
    assert_eq!(Subtype::FromRight.direction(), (1.0, 0.0));
    assert_eq!(Subtype::FromTop.direction(), (0.0, -1.0));
    assert!(Subtype::FromLeft.is_horizontal());
    assert!(!Subtype::FromBottom.is_horizontal());
}

// ---- generic parameter view -------------------------------------------------------------------------------------

fn gain() -> ParamSpec {
    ParamSpec::linear(1, "Gain", 0.0, 10.0, 5.0)
}

fn cutoff() -> ParamSpec {
    ParamSpec {
        id: 2,
        name: "Cutoff".to_string(),
        min: 20.0,
        max: 20000.0,
        default: 1000.0,
        value: 1000.0,
        scale: Scale::Logarithmic,
        expert: false,
    }
}

#[test]
fn expert_parameters_are_hidden_until_the_display_flag_is_set() {
    let bias = ParamSpec { id: 3, expert: true, ..ParamSpec::linear(3, "Bias", -1.0, 1.0, 0.0) };
    let mut view = GenericView::new(vec![gain(), bias], DisplayFlags::default());
    let ids: Vec<u32> = view.visible().iter().map(|p| p.id).collect();
    assert_eq!(ids, vec![1]);
    view.set_flags(DisplayFlags { shows_expert: true });
    let ids: Vec<u32> = view.visible().iter().map(|p| p.id).collect();
    assert_eq!(ids, vec![1, 3]);
    assert!(view.flags().shows_expert);
}

#[test]
fn set_value_clamps_into_the_range_and_refuses_unknown_ids() {
    let mut view = GenericView::new(vec![gain()], DisplayFlags::default());
    assert!(view.set_value(1, 12.0));
    assert_eq!(view.visible()[0].value, 10.0);
    assert!(view.set_value(1, -1.0));
    assert_eq!(view.visible()[0].value, 0.0);
    assert!(!view.set_value(99, 1.0));
}

#[test]
fn a_linear_slider_position_is_the_fraction_of_the_range() {
    let mut view = GenericView::new(vec![gain()], DisplayFlags::default());
    view.set_value(1, 2.5);
    assert!(close(view.position(1).unwrap(), 0.25));
    assert_eq!(view.position(99), None);
}

#[test]
fn a_logarithmic_slider_position_is_the_fraction_of_the_log_range() {
    // ln(200 / 20) / ln(20000 / 20) = 1 / 3
    let mut view = GenericView::new(vec![cutoff()], DisplayFlags::default());
    view.set_value(2, 200.0);
    assert!(close(view.position(2).unwrap(), 1.0 / 3.0));
}

#[test]
fn half_way_on_a_logarithmic_slider_is_the_geometric_mean_of_its_ends() {
    // sqrt(20 * 20000) = 632.456
    let mut view = GenericView::new(vec![cutoff()], DisplayFlags::default());
    assert!(view.set_position(2, 0.5));
    let value = view.visible()[0].value;
    assert!((value - 632.455_5).abs() < 0.01, "got {value}");
}

#[test]
fn a_parameter_with_an_unusable_range_is_dropped_when_the_view_is_built() {
    let bad_log = ParamSpec { scale: Scale::Logarithmic, ..ParamSpec::linear(4, "Bad", 0.0, 1.0, 0.5) };
    let flat = ParamSpec::linear(5, "Flat", 1.0, 1.0, 1.0);
    let not_finite = ParamSpec::linear(6, "Nan", 0.0, f32::NAN, 0.5);
    assert!(!bad_log.is_valid() && !flat.is_valid() && !not_finite.is_valid());
    let view = GenericView::new(vec![gain(), bad_log, flat, not_finite], DisplayFlags::default());
    assert_eq!(view.visible().len(), 1);
}

#[test]
fn reset_restores_the_default_and_refuses_unknown_ids() {
    let mut view = GenericView::new(vec![gain()], DisplayFlags::default());
    view.set_value(1, 9.0);
    assert!(view.reset(1));
    assert_eq!(view.visible()[0].value, 5.0);
    assert!(!view.reset(4));
}

#[test]
fn a_nan_value_clamps_to_the_default_and_a_nan_position_to_the_minimum() {
    let g = gain();
    assert_eq!(g.clamp(f32::NAN), 5.0);
    assert_eq!(g.value_at(f32::NAN), 0.0);
    assert_eq!(g.value_at(2.0), 10.0, "a position above one is the maximum");
}
