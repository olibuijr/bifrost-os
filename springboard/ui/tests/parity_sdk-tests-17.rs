//! Parity sdk-tests-17: headless tests of the layer tree (`bifrost_ui::layer_tree`, `bifrost_ui::layer_layout`) and of
//! the location authorization model (`bifrost_ui::location_auth`). Layers are flattened into render commands in
//! memory and compared exactly; no window, display or device is used.

use bifrost_ui::layer_layout::{
    Constraint, ConstraintAttr, ConstraintLayoutManager, ConstraintSource, LayerAction, LayerContext, LayerDelegate,
};
use bifrost_ui::layer_tree::{Layer, LayerCmd, LayerError, LayerValue};
use bifrost_ui::location_auth::{
    AccuracyAuthorization, AuthorizationStatus, Level, LocationError, RequestOutcome, UsageDescriptions, UsageError,
    ERRORS,
};
use bifrost_ui::transform3d::Transform3D;
use bifrost_ui::Rect;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

const ALL_STATUSES: [AuthorizationStatus; 5] = [
    AuthorizationStatus::NotDetermined,
    AuthorizationStatus::Restricted,
    AuthorizationStatus::Denied,
    AuthorizationStatus::AuthorizedAlways,
    AuthorizationStatus::AuthorizedWhenInUse,
];

fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect { x, y, w, h }
}

// ---- authorization status and usage texts --------------------------------------------------------------------

#[test]
fn a_request_prompts_only_when_the_person_has_not_answered() {
    for status in ALL_STATUSES {
        let expected = match status {
            AuthorizationStatus::NotDetermined => RequestOutcome::Prompt,
            AuthorizationStatus::AuthorizedAlways | AuthorizationStatus::AuthorizedWhenInUse => RequestOutcome::AlreadyGranted,
            AuthorizationStatus::Restricted | AuthorizationStatus::Denied => RequestOutcome::Refused,
        };
        assert_eq!(status.request_outcome(), expected, "{status:?}");
    }
}

#[test]
fn a_restricted_or_denied_app_cannot_start_location_in_use_or_in_the_background() {
    for status in [AuthorizationStatus::Restricted, AuthorizationStatus::Denied] {
        assert!(!status.is_authorized(), "{status:?}");
        assert!(!status.may_start(true), "{status:?} in use");
        assert!(!status.may_start(false), "{status:?} in the background");
    }
}

#[test]
fn an_unanswered_app_cannot_start_location_until_the_person_answers() {
    assert!(!AuthorizationStatus::NotDetermined.is_authorized());
    assert!(!AuthorizationStatus::NotDetermined.may_start(true));
}

#[test]
fn an_always_grant_starts_in_the_background_and_a_when_in_use_grant_only_in_the_foreground() {
    assert!(AuthorizationStatus::AuthorizedAlways.may_start(false));
    assert!(AuthorizationStatus::AuthorizedWhenInUse.may_start(true));
    assert!(!AuthorizationStatus::AuthorizedWhenInUse.may_start(false));
}

#[test]
fn only_full_accuracy_reads_precise_positions() {
    assert!(AccuracyAuthorization::FullAccuracy.is_full());
    assert!(!AccuracyAuthorization::ReducedAccuracy.is_full());
    assert_ne!(AccuracyAuthorization::FullAccuracy, AccuracyAuthorization::ReducedAccuracy);
}

#[test]
fn each_level_has_its_own_usage_text_and_surrounding_spaces_are_removed() {
    let texts = UsageDescriptions::new("  Find the nearest stop  ", "Keep the stop list current");
    assert_eq!(texts.for_level(Level::WhenInUse), Ok("Find the nearest stop"));
    assert_eq!(texts.for_level(Level::Always), Ok("Keep the stop list current"));
}

#[test]
fn a_level_without_a_usage_text_is_reported_missing_so_no_prompt_has_no_reason() {
    let texts = UsageDescriptions::new("Find the nearest stop", " \t ");
    assert_eq!(texts.for_level(Level::Always), Err(UsageError::Missing(Level::Always)));
    assert_eq!(
        UsageDescriptions::default().for_level(Level::WhenInUse),
        Err(UsageError::Missing(Level::WhenInUse))
    );
}

// ---- location errors -----------------------------------------------------------------------------------------

#[test]
fn error_codes_follow_the_declaration_order_and_round_trip() {
    assert_eq!(LocationError::DOMAIN, "bifrost.location");
    assert_eq!(ERRORS.len(), 19);
    for (i, error) in ERRORS.iter().enumerate() {
        assert_eq!(error.code(), i as u32, "{error:?}");
        assert_eq!(LocationError::from_code(i as u32), Some(*error));
    }
    assert_eq!(LocationError::from_code(ERRORS.len() as u32), None);
}

#[test]
fn a_refused_permission_needs_the_person_and_a_network_failure_is_retried() {
    assert!(LocationError::Denied.needs_user_action());
    assert!(LocationError::PromptDeclined.needs_user_action());
    assert!(LocationError::RegionMonitoringDenied.needs_user_action());
    assert!(!LocationError::Denied.is_transient());
    assert!(LocationError::Network.is_transient());
    assert!(!LocationError::Network.needs_user_action());
}

#[test]
fn a_canceled_geocode_is_neither_a_user_action_nor_a_retry() {
    assert!(!LocationError::GeocodeCanceled.needs_user_action());
    assert!(!LocationError::GeocodeCanceled.is_transient());
}

#[test]
fn no_error_both_needs_the_person_and_is_worth_retrying() {
    for error in ERRORS.iter() {
        assert!(!(error.needs_user_action() && error.is_transient()), "{error:?}");
    }
}

// ---- layer geometry ------------------------------------------------------------------------------------------

#[test]
fn a_new_layer_is_visible_opaque_unanchored_and_needs_display() {
    let layer = Layer::new();
    assert_eq!(layer.position(), (0.0, 0.0));
    assert_eq!(layer.anchor_point(), (0.5, 0.5));
    assert_eq!(layer.opacity(), 1.0);
    assert!(!layer.is_hidden());
    assert!(!layer.masks_to_bounds());
    assert_eq!(layer.background_color(), None);
    assert!(layer.sublayers().is_empty());
    assert!(layer.superlayer().is_none());
    assert!(layer.needs_display());
}

#[test]
fn a_layer_made_from_a_frame_reports_that_frame_back() {
    let frame = rect(10.0, 20.0, 100.0, 50.0);
    let layer = Layer::with_frame(frame);
    assert_eq!(layer.bounds(), rect(0.0, 0.0, 100.0, 50.0));
    assert_eq!(layer.position(), (60.0, 45.0), "the centre anchor puts the middle at the frame's middle");
    assert_eq!(layer.frame(), frame);
}

#[test]
fn a_scaled_layer_has_the_bounding_box_of_its_transformed_bounds_as_its_frame() {
    let layer = Layer::new();
    layer.set_bounds(rect(0.0, 0.0, 10.0, 10.0));
    layer.set_transform(Transform3D::make_scale(2.0, 2.0, 1.0));
    layer.set_position(50.0, 50.0);
    assert_eq!(layer.frame(), rect(40.0, 40.0, 20.0, 20.0));
}

#[test]
fn points_and_rectangles_convert_between_layers_through_their_positions() {
    let root = Layer::new();
    let panel = Layer::new();
    panel.set_anchor_point(0.0, 0.0);
    panel.set_bounds(rect(0.0, 0.0, 300.0, 200.0));
    panel.set_position(100.0, 50.0);
    root.add_sublayer(&panel);
    let button = Layer::new();
    button.set_anchor_point(0.0, 0.0);
    button.set_bounds(rect(0.0, 0.0, 40.0, 20.0));
    button.set_position(10.0, 10.0);
    panel.add_sublayer(&button);

    assert_eq!(button.convert_point_to((0.0, 0.0), &root), (110.0, 60.0));
    assert_eq!(button.convert_point_from((110.0, 60.0), &root), (0.0, 0.0));
    assert_eq!(button.convert_rect_to(rect(0.0, 0.0, 40.0, 20.0), &root), rect(110.0, 60.0, 40.0, 20.0));
}

// ---- tree structure ------------------------------------------------------------------------------------------

#[test]
fn sublayers_keep_their_order_and_insertions_go_above_or_below_a_sibling() {
    let parent = Layer::new();
    let (a, b, c) = (Layer::new(), Layer::new(), Layer::new());
    parent.set_sublayers(&[a.clone(), b.clone(), c.clone()]);
    let x = Layer::new();
    parent.insert_sublayer_above(&x, &a);
    assert_eq!(parent.sublayers(), vec![a.clone(), x.clone(), b.clone(), c.clone()]);
    let y = Layer::new();
    parent.insert_sublayer_below(&y, &a);
    assert_eq!(parent.sublayers(), vec![y.clone(), a.clone(), x.clone(), b.clone(), c.clone()]);
    assert_eq!(y.superlayer(), Some(parent.clone()));
}

#[test]
fn adding_a_layer_moves_it_out_of_its_old_superlayer() {
    let (first, second, child) = (Layer::new(), Layer::new(), Layer::new());
    first.add_sublayer(&child);
    second.add_sublayer(&child);
    assert!(first.sublayers().is_empty());
    assert_eq!(child.superlayer(), Some(second.clone()));
    child.remove_from_superlayer();
    assert!(second.sublayers().is_empty());
    assert!(child.superlayer().is_none());
}

#[test]
fn replace_sublayer_takes_the_old_place_and_ignores_a_layer_that_is_not_a_sublayer() {
    let parent = Layer::new();
    let (a, b, c, outsider) = (Layer::new(), Layer::new(), Layer::new(), Layer::new());
    parent.set_sublayers(&[a.clone(), b.clone()]);
    parent.replace_sublayer(&a, &c);
    assert_eq!(parent.sublayers(), vec![c.clone(), b.clone()]);
    assert!(a.superlayer().is_none());
    parent.replace_sublayer(&outsider, &a);
    assert_eq!(parent.sublayers(), vec![c.clone(), b.clone()]);
}

#[test]
fn a_layer_cannot_become_its_own_ancestor() {
    let parent = Layer::new();
    let child = Layer::new();
    parent.add_sublayer(&child);
    child.add_sublayer(&parent);
    assert!(child.sublayers().is_empty());
    assert!(parent.superlayer().is_none());
    assert_eq!(child.root(), parent);
}

#[test]
fn a_hit_test_returns_the_front_most_visible_layer_under_the_point() {
    let root = Layer::with_frame(rect(0.0, 0.0, 200.0, 200.0));
    let back = Layer::with_frame(rect(0.0, 0.0, 100.0, 100.0));
    let front = Layer::with_frame(rect(50.0, 50.0, 100.0, 100.0));
    root.add_sublayer(&back);
    root.add_sublayer(&front);
    front.set_z_position(1.0);

    assert_eq!(root.hit_test((60.0, 60.0)), Some(front.clone()), "the higher layer wins where both cover");
    assert_eq!(root.hit_test((10.0, 10.0)), Some(back.clone()));
    assert_eq!(root.hit_test((180.0, 180.0)), Some(root.clone()), "only the root covers this point");
    assert_eq!(root.hit_test((300.0, 300.0)), None);

    front.set_hidden(true);
    assert_eq!(root.hit_test((60.0, 60.0)), Some(back.clone()), "a hidden layer cannot be hit");
    back.set_opacity(0.0);
    assert_eq!(root.hit_test((60.0, 60.0)), Some(root.clone()), "a fully transparent layer cannot be hit");
}

#[test]
fn a_change_marks_the_tree_dirty_until_the_tree_is_cleared() {
    let parent = Layer::new();
    let child = Layer::new();
    parent.add_sublayer(&child);
    parent.clear_dirty_tree();
    assert!(!parent.tree_dirty());
    child.set_opacity(0.5);
    assert!(parent.tree_dirty());
    parent.clear_dirty_tree();
    assert!(!parent.tree_dirty());
}

// ---- properties, observers and key paths --------------------------------------------------------------------

#[test]
fn an_observer_hears_each_real_change_once_with_its_old_and_new_value() {
    let layer = Layer::new();
    let heard: Rc<RefCell<Vec<(String, LayerValue, LayerValue)>>> = Rc::new(RefCell::new(Vec::new()));
    let sink = heard.clone();
    layer.add_observer(Rc::new(move |_: &Layer, key: &str, old: &LayerValue, new: &LayerValue| {
        sink.borrow_mut().push((key.to_string(), old.clone(), new.clone()));
    }));
    layer.set_opacity(0.25);
    layer.set_opacity(0.25);
    layer.set_hidden(true);
    assert_eq!(
        *heard.borrow(),
        vec![
            ("opacity".to_string(), LayerValue::Float(1.0), LayerValue::Float(0.25)),
            ("hidden".to_string(), LayerValue::Bool(false), LayerValue::Bool(true)),
        ]
    );
}

#[test]
fn key_paths_read_and_write_built_in_properties_and_refuse_a_wrong_type() {
    let layer = Layer::new();
    layer.set_value("bounds.size.width", LayerValue::Float(80.0)).unwrap();
    assert_eq!(layer.value("bounds.size.width"), Some(LayerValue::Float(80.0)));
    assert_eq!(layer.bounds().w, 80.0);
    layer.set_value("position", LayerValue::Point(3.0, 4.0)).unwrap();
    assert_eq!(layer.value("position.y"), Some(LayerValue::Float(4.0)));
    assert_eq!(
        layer.set_value("opacity", LayerValue::Bool(true)),
        Err(LayerError::TypeMismatch("opacity".to_string()))
    );
    assert_eq!(layer.opacity(), 1.0, "a refused value changes nothing");
}

#[test]
fn an_unknown_key_is_kept_as_an_extra_value_and_is_not_a_built_in() {
    let layer = Layer::new();
    layer.set_value("cornerStyle.radius", LayerValue::Float(8.0)).unwrap();
    assert_eq!(layer.value("cornerStyle.radius"), Some(LayerValue::Float(8.0)));
    assert_eq!(layer.extra_keys(), vec!["cornerStyle.radius".to_string()]);
    assert!(!Layer::is_builtin_key("cornerStyle.radius"));
    assert!(Layer::is_builtin_key("position.x"));
    assert_eq!(layer.value("cornerStyle.missing"), None);
}

struct Quiet;

impl LayerDelegate for Quiet {
    fn action_for(&self, _layer: &Layer, key: &str) -> Option<LayerAction> {
        (key == "position").then_some(LayerAction::Null)
    }
}

#[test]
fn the_delegate_answers_an_action_before_the_layer_action_table() {
    let layer = Layer::new();
    layer.set_action("opacity", Some(LayerAction::Named("fade".to_string())));
    assert_eq!(layer.action_for_key("opacity"), Some(LayerAction::Named("fade".to_string())));
    assert_eq!(layer.action_for_key("position"), None);
    layer.set_delegate(Some(Rc::new(Quiet)));
    assert_eq!(layer.action_for_key("position"), Some(LayerAction::Null));
    layer.set_action("position", Some(LayerAction::Named("slide".to_string())));
    assert_eq!(layer.action_for_key("position"), Some(LayerAction::Null), "the delegate answers first");
    layer.set_action("opacity", None);
    assert_eq!(layer.action_for_key("opacity"), None);
}

// ---- drawing and the render list -----------------------------------------------------------------------------

struct Painter {
    colour: u32,
    draws: Rc<Cell<u32>>,
}

impl LayerDelegate for Painter {
    fn draw_layer(&self, _layer: &Layer, ctx: &mut LayerContext) {
        self.draws.set(self.draws.get() + 1);
        let (w, h) = ctx.size();
        ctx.fill_rect(rect(0.0, 0.0, w, h), self.colour);
    }
}

#[test]
fn a_delegate_draws_once_and_its_operations_stay_until_the_layer_is_displayed_again() {
    let layer = Layer::with_frame(rect(0.0, 0.0, 30.0, 20.0));
    let draws = Rc::new(Cell::new(0));
    layer.set_delegate(Some(Rc::new(Painter { colour: 0xff12_3456, draws: draws.clone() })));
    let cmds = layer.render_list();
    assert_eq!(cmds.len(), 3);
    assert!(matches!(cmds[0], LayerCmd::Push { opacity, .. } if opacity == 1.0));
    assert_eq!(cmds[1], LayerCmd::Fill { rect: rect(0.0, 0.0, 30.0, 20.0), color: 0xff12_3456 });
    assert_eq!(cmds[2], LayerCmd::Pop);
    assert!(!layer.needs_display());
    assert_eq!(layer.render_list(), cmds);
    assert_eq!(draws.get(), 1);
    layer.set_needs_display();
    layer.render_list();
    assert_eq!(draws.get(), 2);
}

#[test]
fn a_hidden_layer_emits_nothing_and_a_masked_bordered_layer_clips_and_outlines_itself() {
    let layer = Layer::with_frame(rect(0.0, 0.0, 30.0, 20.0));
    let bounds = rect(0.0, 0.0, 30.0, 20.0);
    layer.set_background_color(Some(0xff00_ff00));
    layer.set_masks_to_bounds(true);
    layer.set_border_width(2.0);
    layer.set_border_color(0xff00_00ff);
    let cmds = layer.render_list();
    assert_eq!(cmds[1], LayerCmd::PushClip(bounds));
    assert_eq!(cmds[2], LayerCmd::Fill { rect: bounds, color: 0xff00_ff00 });
    assert!(cmds.contains(&LayerCmd::Stroke { rect: bounds, width: 2.0, color: 0xff00_00ff }));
    assert_eq!(cmds[cmds.len() - 2], LayerCmd::PopClip);
    layer.set_hidden(true);
    assert!(layer.render_list().is_empty());
}

#[test]
fn sublayers_are_drawn_back_to_front_by_their_z_position() {
    let parent = Layer::with_frame(rect(0.0, 0.0, 100.0, 100.0));
    let front = Layer::with_frame(rect(0.0, 0.0, 10.0, 10.0));
    front.set_background_color(Some(0xff00_0001));
    front.set_z_position(2.0);
    let back = Layer::with_frame(rect(0.0, 0.0, 10.0, 10.0));
    back.set_background_color(Some(0xff00_0002));
    back.set_z_position(1.0);
    parent.add_sublayer(&front);
    parent.add_sublayer(&back);
    let cmds = parent.render_list();
    let at = |color: u32| {
        cmds.iter()
            .position(|cmd| *cmd == LayerCmd::Fill { rect: rect(0.0, 0.0, 10.0, 10.0), color })
            .expect("the fill is in the list")
    };
    assert!(at(0xff00_0002) < at(0xff00_0001), "the lower z position is drawn first");
}

// ---- constraint layout ---------------------------------------------------------------------------------------

#[test]
fn constraints_lay_out_sublayers_against_the_superlayer_and_a_named_sibling() {
    let parent = Layer::new();
    parent.set_bounds(rect(0.0, 0.0, 200.0, 100.0));
    parent.set_layout_manager(Some(Rc::new(ConstraintLayoutManager)));

    let content = Layer::new();
    content.set_name("content");
    content.set_constraints(vec![
        Constraint::new(ConstraintAttr::MinX, ConstraintSource::Superlayer, ConstraintAttr::MinX).with_offset(10.0),
        Constraint::new(ConstraintAttr::MaxX, ConstraintSource::Superlayer, ConstraintAttr::MaxX).with_offset(-10.0),
        Constraint::new(ConstraintAttr::MinY, ConstraintSource::Superlayer, ConstraintAttr::MinY).with_offset(5.0),
        Constraint::new(ConstraintAttr::MaxY, ConstraintSource::Superlayer, ConstraintAttr::MaxY).with_offset(-5.0),
    ]);

    let label = Layer::new();
    label.set_name("label");
    label.set_constraints(vec![
        Constraint::new(
            ConstraintAttr::MinX,
            ConstraintSource::Sibling("content".to_string()),
            ConstraintAttr::MaxX,
        )
        .with_offset(4.0),
        Constraint::new(ConstraintAttr::Width, ConstraintSource::Superlayer, ConstraintAttr::Width).with_scale(0.5),
        Constraint::new(
            ConstraintAttr::MinY,
            ConstraintSource::Sibling("content".to_string()),
            ConstraintAttr::MinY,
        ),
        Constraint::new(ConstraintAttr::Height, ConstraintSource::Superlayer, ConstraintAttr::Height).with_scale(0.1),
    ]);

    parent.set_sublayers(&[content.clone(), label.clone()]);
    parent.layout_if_needed();
    assert_eq!(content.frame(), rect(10.0, 5.0, 180.0, 90.0));
    assert_eq!(label.frame(), rect(194.0, 5.0, 100.0, 10.0));
}

#[test]
fn a_constraint_to_a_missing_sibling_leaves_the_frame_alone() {
    let parent = Layer::new();
    parent.set_bounds(rect(0.0, 0.0, 200.0, 100.0));
    parent.set_layout_manager(Some(Rc::new(ConstraintLayoutManager)));
    let orphan = Layer::with_frame(rect(7.0, 9.0, 30.0, 40.0));
    orphan.set_constraints(vec![Constraint::new(
        ConstraintAttr::MinX,
        ConstraintSource::Sibling("missing".to_string()),
        ConstraintAttr::MinX,
    )]);
    parent.set_sublayers(&[orphan.clone()]);
    parent.layout_if_needed();
    assert_eq!(orphan.frame(), rect(7.0, 9.0, 30.0, 40.0));
}
