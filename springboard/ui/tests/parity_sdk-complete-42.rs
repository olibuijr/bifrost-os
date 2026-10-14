//! Parity sdk-complete-42: headless tests of the 3D view camera model. The camera is checked through its public
//! fields and methods only: validity, the look and framing operations, and the matrices it builds. No test renders a
//! scene or opens a display.

use bifrost_ui::linalg::Vector3;
use bifrost_ui::view_camera::{Projection, ViewCamera};

const EPS: f32 = 1e-3;

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < EPS
}

#[test]
fn a_new_camera_looks_down_the_negative_z_axis_from_five_units_out() {
    let cam = ViewCamera::default();
    assert_eq!(cam.position, Vector3::new(0.0, 0.0, 5.0));
    assert_eq!(cam.target, Vector3::ZERO);
    assert_eq!(cam.projection, Projection::Perspective);
    assert!(cam.is_valid());
}

#[test]
fn look_at_turns_the_camera_and_keeps_its_position() {
    let mut cam = ViewCamera::default();
    cam.look_at(Vector3::new(2.0, 0.0, 0.0));
    assert_eq!(cam.position, Vector3::new(0.0, 0.0, 5.0));
    assert_eq!(cam.target, Vector3::new(2.0, 0.0, 0.0));
}

#[test]
fn look_from_moves_the_camera_and_points_it_at_the_target() {
    let mut cam = ViewCamera::default();
    cam.look_from(Vector3::new(0.0, 10.0, 0.0), Vector3::new(0.0, 0.0, -1.0));
    assert_eq!(cam.position, Vector3::new(0.0, 10.0, 0.0));
    assert_eq!(cam.target, Vector3::new(0.0, 0.0, -1.0));
}

#[test]
fn a_field_of_view_of_zero_or_180_degrees_or_more_is_not_valid() {
    let mut cam = ViewCamera::default();
    cam.field_of_view = 0.0;
    assert!(!cam.is_valid());
    assert_eq!(cam.projection_matrix(1.0), None);
    cam.field_of_view = 180.0;
    assert!(!cam.is_valid());
    cam.field_of_view = 179.0;
    assert!(cam.is_valid());
}

#[test]
fn an_orthographic_camera_needs_a_positive_height_and_ignores_the_field_of_view() {
    let mut cam = ViewCamera::default();
    cam.projection = Projection::Orthographic;
    cam.field_of_view = 0.0;
    assert!(cam.is_valid(), "the field of view applies to the perspective projection only");
    cam.orthographic_height = 0.0;
    assert!(!cam.is_valid());
}

#[test]
fn a_projection_needs_a_positive_aspect_ratio() {
    let cam = ViewCamera::default();
    assert!(cam.projection_matrix(16.0 / 9.0).is_some());
    assert_eq!(cam.projection_matrix(0.0), None);
    assert_eq!(cam.projection_matrix(-1.0), None);
}

#[test]
fn the_view_matrix_puts_the_target_five_units_in_front_of_the_default_camera() {
    let cam = ViewCamera::default();
    let at_target = cam.view_matrix().transform_point(cam.target).unwrap();
    assert!(close(at_target.x, 0.0));
    assert!(close(at_target.y, 0.0));
    assert!(close(at_target.z, -5.0), "the eye looks down -z, so the target lies at z = -5");
}

#[test]
fn the_view_projection_is_missing_exactly_when_the_camera_is_invalid() {
    let mut cam = ViewCamera::default();
    assert!(cam.view_projection(1.0).is_some());
    cam.far = cam.near;
    assert_eq!(cam.view_projection(1.0), None);
}

#[test]
fn framing_a_perspective_box_puts_the_camera_at_twice_the_radius_for_a_sixty_degree_field() {
    // Box from (-1,-1,-1) to (1,1,1): centre at the origin, radius sqrt(3). With a 60 degree field the distance is
    // radius / sin(30 degrees) = 2 * radius. The default camera sits on the +z side, so it stays there.
    let mut cam = ViewCamera::default();
    let radius = 3.0_f32.sqrt();
    assert!(cam.frame_bounding_box(Vector3::new(-1.0, -1.0, -1.0), Vector3::new(1.0, 1.0, 1.0)));
    assert!(close(cam.position.z, 2.0 * radius));
    assert!(close(cam.target.length(), 0.0));
    assert!(close(cam.near, radius));
    assert!(close(cam.far, 3.0 * radius));
}

#[test]
fn framing_an_orthographic_box_sets_the_visible_height_to_the_box_diameter() {
    let mut cam = ViewCamera::default();
    cam.projection = Projection::Orthographic;
    let radius = 3.0_f32.sqrt();
    assert!(cam.frame_bounding_box(Vector3::new(-1.0, -1.0, -1.0), Vector3::new(1.0, 1.0, 1.0)));
    assert!(close(cam.orthographic_height, 2.0 * radius));
    assert!(close(cam.position.z, radius));
    assert!(cam.is_valid());
}

#[test]
fn framing_a_flat_box_still_frames_it_and_a_point_is_refused() {
    let mut cam = ViewCamera::default();
    assert!(cam.frame_bounding_box(Vector3::new(0.0, 0.0, 0.0), Vector3::new(2.0, 0.0, 0.0)));
    assert!(close(cam.target.x, 1.0));
    let before = cam;
    assert!(!cam.frame_bounding_box(Vector3::new(4.0, 4.0, 4.0), Vector3::new(4.0, 4.0, 4.0)));
    assert_eq!(cam, before, "a box with no size leaves the camera where it was");
}
