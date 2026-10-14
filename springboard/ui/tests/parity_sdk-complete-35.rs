//! Parity sdk-complete-35: headless tests for the scene node tree (`bifrost_ui::sk_node`), the per-key timing of
//! keyframe sequences (`bifrost_ui::sk_action`) and the other models completed in this round. Every test is pure and
//! deterministic: no clock, no display, no device.

use bifrost_ui::anim::Curve;
use bifrost_ui::sk_action::{Action, Key, Keyframes};
use bifrost_ui::sk_node::{Content, Frame, NodeTree, Point, Size};

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

fn assert_point(p: Point, x: f64, y: f64) {
    assert!(near(p.x, x) && near(p.y, y), "expected ({x}, {y}), got ({}, {})", p.x, p.y);
}

fn assert_frame(f: Frame, x: f64, y: f64, w: f64, h: f64) {
    assert!(
        near(f.origin.x, x) && near(f.origin.y, y) && near(f.size.width, w) && near(f.size.height, h),
        "expected frame ({x}, {y}, {w}, {h}), got ({}, {}, {}, {})",
        f.origin.x,
        f.origin.y,
        f.size.width,
        f.size.height
    );
}

fn square(side: f64) -> Content {
    Content::Rect(Size { width: side, height: side })
}

// --- sk_node: the tree, transforms and alpha ------------------------------------------------------------------------

#[test]
fn a_new_tree_has_a_root_with_no_parent_that_cannot_be_removed() {
    let mut tree = NodeTree::new();
    let root = tree.root();
    assert_eq!(tree.parent(root), None);
    assert_eq!(tree.children(root).map(|c| c.len()), Some(0));
    assert!(!tree.remove(root), "the scene root is not a removable node");
}

#[test]
fn a_child_takes_its_position_and_scale_from_the_parent() {
    let mut tree = NodeTree::new();
    let parent = tree.add(tree.root(), Content::Empty).unwrap();
    tree.set_position(parent, Point { x: 100.0, y: 50.0 });
    tree.set_scale(parent, Point { x: 2.0, y: 2.0 });
    let child = tree.add(parent, Content::Empty).unwrap();
    tree.set_position(child, Point { x: 10.0, y: 0.0 });
    assert_point(tree.world_position(child).unwrap(), 120.0, 50.0);
    assert_eq!(tree.parent(child), Some(parent));
}

#[test]
fn a_parent_rotation_turns_the_positions_of_its_children() {
    let mut tree = NodeTree::new();
    let parent = tree.add(tree.root(), Content::Empty).unwrap();
    tree.set_rotation(parent, std::f64::consts::FRAC_PI_2);
    let child = tree.add(parent, Content::Empty).unwrap();
    tree.set_position(child, Point { x: 10.0, y: 0.0 });
    assert_point(tree.world_position(child).unwrap(), 0.0, 10.0);
}

#[test]
fn alpha_multiplies_down_the_tree_and_is_clamped_to_one() {
    let mut tree = NodeTree::new();
    let parent = tree.add(tree.root(), Content::Empty).unwrap();
    let child = tree.add(parent, Content::Empty).unwrap();
    assert!(tree.set_alpha(parent, 0.5));
    assert!(tree.set_alpha(child, 0.5));
    assert!(near(tree.world_alpha(child).unwrap(), 0.25));
    assert!(tree.set_alpha(child, 1.5));
    assert!(near(tree.world_alpha(child).unwrap(), 0.5));
}

#[test]
fn edits_to_an_unknown_node_are_refused() {
    let mut tree = NodeTree::new();
    let gone = tree.add(tree.root(), Content::Empty).unwrap();
    assert!(tree.remove(gone));
    assert!(!tree.set_position(gone, Point { x: 1.0, y: 1.0 }));
    assert!(!tree.set_alpha(gone, 0.2));
    assert_eq!(tree.world_position(gone), None);
    assert_eq!(tree.add(gone, Content::Empty), None);
}

#[test]
fn removing_a_node_removes_its_subtree_and_its_running_actions() {
    let mut tree = NodeTree::new();
    let parent = tree.add(tree.root(), Content::Empty).unwrap();
    let child = tree.add(parent, Content::Empty).unwrap();
    tree.run_action(child, &Action::Report(4)).unwrap();
    assert!(tree.remove(parent));
    assert_eq!(tree.world_position(child), None);
    assert_eq!(tree.children(tree.root()).map(|c| c.len()), Some(0));
    assert_eq!(tree.advance_actions(1.0), Vec::<(bifrost_ui::sk_node::NodeId, u32)>::new());
}

// --- sk_node: z order and visibility --------------------------------------------------------------------------------

#[test]
fn draw_order_sorts_siblings_by_z_then_by_insertion() {
    let mut tree = NodeTree::new();
    let root = tree.root();
    let a = tree.add(root, square(1.0)).unwrap();
    let b = tree.add(root, square(1.0)).unwrap();
    let c = tree.add(root, square(1.0)).unwrap();
    tree.set_z_position(a, 2.0);
    tree.set_z_position(c, 0.0);
    let order: Vec<_> = tree.draw_list(Size { width: 100.0, height: 100.0 }).iter().map(|i| i.node).collect();
    assert_eq!(order, vec![b, c, a], "equal depths keep insertion order");
}

#[test]
fn a_hidden_node_hides_its_whole_subtree() {
    let mut tree = NodeTree::new();
    let parent = tree.add(tree.root(), square(4.0)).unwrap();
    tree.add(parent, square(2.0)).unwrap();
    assert_eq!(tree.draw_list(Size { width: 50.0, height: 50.0 }).len(), 2);
    assert!(tree.set_hidden(parent, true));
    assert!(tree.draw_list(Size { width: 50.0, height: 50.0 }).is_empty());
}

#[test]
fn a_node_with_no_alpha_or_no_content_is_not_listed_for_drawing() {
    let mut tree = NodeTree::new();
    let faint = tree.add(tree.root(), square(4.0)).unwrap();
    tree.add(tree.root(), Content::Empty).unwrap();
    tree.set_alpha(faint, 0.0);
    assert!(tree.draw_list(Size { width: 50.0, height: 50.0 }).is_empty());
}

// --- sk_node: frames of the content ---------------------------------------------------------------------------------

#[test]
fn a_rectangle_frame_is_centred_on_its_node_and_follows_a_quarter_turn() {
    let mut tree = NodeTree::new();
    let node = tree.add(tree.root(), Content::Rect(Size { width: 10.0, height: 4.0 })).unwrap();
    assert_frame(tree.world_frame(node).unwrap(), -5.0, -2.0, 10.0, 4.0);
    tree.set_rotation(node, std::f64::consts::FRAC_PI_2);
    let turned = tree.world_frame(node).unwrap();
    assert_frame(turned, -2.0, -5.0, 4.0, 10.0);
}

#[test]
fn a_circle_frame_follows_the_non_uniform_scale_of_its_node() {
    let mut tree = NodeTree::new();
    let node = tree.add(tree.root(), Content::Circle(3.0)).unwrap();
    tree.set_scale(node, Point { x: 2.0, y: 1.0 });
    assert_frame(tree.world_frame(node).unwrap(), -6.0, -3.0, 12.0, 6.0);
}

#[test]
fn an_empty_node_has_no_frame() {
    let mut tree = NodeTree::new();
    let node = tree.add(tree.root(), Content::Empty).unwrap();
    assert_eq!(tree.world_frame(node), None);
}

// --- sk_node: camera and view mapping -------------------------------------------------------------------------------

#[test]
fn without_a_camera_the_scene_origin_is_the_view_centre() {
    let tree = NodeTree::new();
    let view = Size { width: 200.0, height: 100.0 };
    assert_point(tree.view_point(Point { x: 0.0, y: 0.0 }, view).unwrap(), 100.0, 50.0);
    assert_point(tree.view_point(Point { x: 30.0, y: -10.0 }, view).unwrap(), 130.0, 40.0);
}

#[test]
fn a_camera_centres_its_position_in_the_view() {
    let mut tree = NodeTree::new();
    let camera = tree.add(tree.root(), Content::Empty).unwrap();
    tree.set_position(camera, Point { x: 100.0, y: 100.0 });
    assert!(tree.set_camera(Some(camera)));
    let view = Size { width: 200.0, height: 100.0 };
    assert_point(tree.view_point(Point { x: 100.0, y: 100.0 }, view).unwrap(), 100.0, 50.0);
}

#[test]
fn a_camera_scale_of_two_shows_twice_the_scene() {
    let mut tree = NodeTree::new();
    let camera = tree.add(tree.root(), Content::Empty).unwrap();
    tree.set_scale(camera, Point { x: 2.0, y: 2.0 });
    tree.set_camera(Some(camera));
    let view = Size { width: 200.0, height: 100.0 };
    assert_point(tree.view_point(Point { x: 20.0, y: 0.0 }, view).unwrap(), 110.0, 50.0);
    let back = tree.scene_point(Point { x: 110.0, y: 50.0 }, view).unwrap();
    assert_point(back, 20.0, 0.0);
}

#[test]
fn a_camera_must_belong_to_the_tree() {
    let mut tree = NodeTree::new();
    let node = tree.add(tree.root(), Content::Empty).unwrap();
    tree.remove(node);
    assert!(!tree.set_camera(Some(node)));
    assert!(tree.set_camera(None));
}

#[test]
fn draw_list_maps_each_frame_through_the_camera_and_keeps_paint_order() {
    let mut tree = NodeTree::new();
    let root = tree.root();
    let rect = tree.add(root, square(10.0)).unwrap();
    let dot = tree.add(root, Content::Circle(5.0)).unwrap();
    tree.set_position(dot, Point { x: 50.0, y: 0.0 });
    tree.set_z_position(dot, -1.0);
    tree.set_alpha(dot, 0.5);
    let items = tree.draw_list(Size { width: 100.0, height: 100.0 });
    assert_eq!(items.iter().map(|i| i.node).collect::<Vec<_>>(), vec![dot, rect]);
    assert_frame(items[0].frame, 95.0, 45.0, 10.0, 10.0);
    assert!(near(items[0].alpha, 0.5));
    assert_frame(items[1].frame, 45.0, 45.0, 10.0, 10.0);
    assert!(near(items[1].alpha, 1.0));
}

// --- sk_node: per-node actions --------------------------------------------------------------------------------------

#[test]
fn actions_run_per_node_and_report_the_node_that_fired() {
    let mut tree = NodeTree::new();
    let a = tree.add(tree.root(), Content::Empty).unwrap();
    let b = tree.add(tree.root(), Content::Empty).unwrap();
    let script = Action::Sequence(vec![Action::Report(1), Action::Wait(0.5), Action::Report(2)]);
    let handle = tree.run_action(a, &script).unwrap();
    tree.run_action(b, &Action::Report(9)).unwrap();
    assert_eq!(tree.advance_actions(0.2), vec![(a, 1), (b, 9)]);
    assert_eq!(tree.advance_actions(0.5), vec![(a, 2)]);
    assert_eq!(tree.advance_actions(0.5), Vec::new());
    assert!(!tree.remove_action(a, handle));
}

#[test]
fn a_running_action_can_be_removed_before_it_fires() {
    let mut tree = NodeTree::new();
    let node = tree.add(tree.root(), Content::Empty).unwrap();
    let handle = tree.run_action(node, &Action::Wait(1.0)).unwrap();
    assert!(tree.remove_action(node, handle));
    assert_eq!(tree.advance_actions(2.0), Vec::new());
    assert!(!tree.remove_action(node, handle), "a removed action is no longer running");
}

#[test]
fn an_action_cannot_be_started_on_a_removed_node() {
    let mut tree = NodeTree::new();
    let node = tree.add(tree.root(), Content::Empty).unwrap();
    tree.remove(node);
    assert_eq!(tree.run_action(node, &Action::Report(1)), None);
}

// --- sk_action: keyframe timing per key -----------------------------------------------------------------------------

#[test]
fn a_key_eases_the_segment_that_starts_at_it() {
    let keys = Keyframes::from_keys(vec![Key::linear(0.0, 0.0).eased(Curve::EaseIn), Key::linear(1.0, 10.0)]).unwrap();
    let expected = 10.0 * f64::from(Curve::EaseIn.at(0.5));
    assert!(near(keys.value_at(0.5), expected));
    assert!(expected < 5.0, "an ease-in segment is behind the straight line at its midpoint");
}

#[test]
fn each_key_chooses_how_its_own_segment_is_filled() {
    let keys = Keyframes::from_keys(vec![Key::hold(0.0, 0.0), Key::linear(1.0, 10.0), Key::hold(2.0, 20.0)]).unwrap();
    assert!(near(keys.value_at(0.5), 0.0), "the first segment holds its value");
    assert!(near(keys.value_at(1.0), 10.0));
    assert!(near(keys.value_at(1.5), 15.0), "the second segment interpolates linearly");
    assert!(near(keys.value_at(3.0), 20.0));
}

#[test]
fn keyframes_from_keys_reject_empty_or_unordered_keys() {
    assert!(Keyframes::from_keys(vec![]).is_none());
    assert!(Keyframes::from_keys(vec![Key::linear(1.0, 0.0), Key::linear(0.5, 1.0)]).is_none());
    assert!(Keyframes::from_keys(vec![Key::linear(0.0, f64::INFINITY)]).is_none());
}

#[test]
fn the_plain_constructor_still_builds_linear_keys_with_no_easing() {
    let keys = Keyframes::new(bifrost_ui::sk_action::Interpolation::Linear, vec![(0.0, 0.0), (2.0, 4.0)]).unwrap();
    assert!(near(keys.value_at(1.0), 2.0));
}
