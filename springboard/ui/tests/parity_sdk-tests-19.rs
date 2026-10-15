//! Parity sdk-tests-19: tests of the scene tree, cameras, look-at constraints, CPU lighting, surface materials,
//! shader entry points and window list records, through their public API. Every test is pure arithmetic or
//! bookkeeping; nothing here draws a frame.

use bifrost_core::json::Json;
use bifrost_ui::linalg::{Quaternion, Vector3};
use bifrost_ui::scene3d::{
    look_at_orientation, quaternion_from_axes, Camera, Constraint, Pose, ProjectionDirection, Scene,
};
use bifrost_ui::scene_material::{
    AssetMaterial, MaterialProperty, MaterialSemantic, MaterialValue, ShadingModel,
};
use bifrost_ui::shader_library::{
    is_valid_name, LibraryError, RenderPipelineDescriptor, ShaderLibrary, Stage, MAX_NAME_LEN,
};
use bifrost_ui::shading::{shade, Fog, Light, LightSource, Material, Rgb};
use bifrost_ui::window_list::{
    is_above, level_of_shell_window, list_from_json, list_to_json, make_opaque, redact_names,
    stacking_index, window_level, window_list, ImageOptions, SharingState, ShellWindowKind,
    WindowInfo, WindowLevelKey, WindowListOptions, WindowScope, NORMAL_LAYER,
};
use bifrost_ui::Rect;
use std::collections::HashSet;
use std::f32::consts::FRAC_PI_2;

fn v(x: f32, y: f32, z: f32) -> Vector3 {
    Vector3::new(x, y, z)
}

fn near(a: Vector3, b: Vector3) -> bool {
    (a.x - b.x).abs() < 1e-4 && (a.y - b.y).abs() < 1e-4 && (a.z - b.z).abs() < 1e-4
}

fn approx(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-4
}

fn at(position: Vector3) -> Pose {
    Pose {
        position,
        ..Pose::identity()
    }
}

fn rgb_near(a: Rgb, b: Rgb) -> bool {
    approx(a.r, b.r) && approx(a.g, b.g) && approx(a.b, b.b)
}

// ---- scene tree ----

#[test]
fn a_new_scene_holds_only_the_named_root() {
    let s = Scene::new();
    let root = s.root();
    assert_eq!(s.node_count(), 1);
    assert_eq!(s.name(root), Some("root"));
    assert_eq!(s.parent(root), None);
    assert!(s.children(root).is_empty());
    assert_eq!(s.find("root"), Some(root));
}

#[test]
fn added_children_keep_their_parent_and_their_order() {
    let mut s = Scene::new();
    let root = s.root();
    let a = s.add_child(root, "a").unwrap();
    let b = s.add_child(root, "b").unwrap();
    let c = s.add_child(a, "c").unwrap();
    assert_eq!(s.children(root).to_vec(), vec![a, b]);
    assert_eq!(s.children(a).to_vec(), vec![c]);
    assert_eq!(s.parent(c), Some(a));
    assert_eq!(s.node_count(), 4);
}

#[test]
fn find_returns_the_first_node_with_the_name() {
    let mut s = Scene::new();
    let root = s.root();
    let first = s.add_child(root, "dup").unwrap();
    let _second = s.add_child(root, "dup").unwrap();
    assert_eq!(s.find("dup"), Some(first));
    assert_eq!(s.find("missing"), None);
}

#[test]
fn a_handle_from_another_scene_is_refused_everywhere() {
    let mut big = Scene::new();
    let root = big.root();
    let far = big.add_child(root, "far").unwrap();
    let mut small = Scene::new();
    assert!(small.add_child(far, "x").is_none());
    assert!(!small.set_pose(far, Pose::identity()));
    assert!(!small.set_constraint(far, None));
    assert!(!small.set_camera(far, None));
    assert_eq!(small.local_pose(far), None);
    assert_eq!(small.world_pose(far), None);
    assert_eq!(small.name(far), None);
    assert_eq!(small.parent(far), None);
    assert!(small.children(far).is_empty());
}

#[test]
fn a_child_inherits_its_parents_position_and_scale() {
    // The parent sits at x = 10 and doubles its children's offsets; the child is 1 unit out along its own X.
    let mut s = Scene::new();
    let root = s.root();
    let parent = s.add_child(root, "parent").unwrap();
    s.set_pose(
        parent,
        Pose {
            position: v(10.0, 0.0, 0.0),
            scale: v(2.0, 2.0, 2.0),
            ..Pose::identity()
        },
    );
    let child = s.add_child(parent, "child").unwrap();
    s.set_pose(child, at(v(1.0, 0.0, 0.0)));
    assert!(near(s.world_position(child).unwrap(), v(12.0, 0.0, 0.0)));
    assert!(near(s.world_pose(child).unwrap().scale, v(2.0, 2.0, 2.0)));
    assert!(near(
        s.local_pose(child).unwrap().position,
        v(1.0, 0.0, 0.0)
    ));
}

#[test]
fn a_world_matrix_maps_local_points_to_world_points() {
    let mut s = Scene::new();
    let root = s.root();
    let n = s.add_child(root, "n").unwrap();
    s.set_pose(
        n,
        Pose {
            position: v(0.0, 0.0, 5.0),
            orientation: Quaternion::from_axis_angle(v(0.0, 0.0, 1.0), FRAC_PI_2),
            scale: v(2.0, 1.0, 1.0),
        },
    );
    // The local point (1, 0, 0) is stretched to (2, 0, 0), turned a quarter turn about Z to (0, 2, 0), then lifted to z = 5.
    let p = s
        .world_matrix(n)
        .unwrap()
        .transform_point(v(1.0, 0.0, 0.0))
        .unwrap();
    assert!(near(p, v(0.0, 2.0, 5.0)));
}

#[test]
fn the_identity_pose_leaves_a_child_pose_unchanged() {
    let child = Pose {
        position: v(1.0, 2.0, 3.0),
        orientation: Quaternion::from_axis_angle(v(0.0, 1.0, 0.0), 0.5),
        scale: v(2.0, 3.0, 4.0),
    };
    let composed = Pose::identity().compose(child);
    assert!(near(composed.position, child.position));
    assert!(near(composed.scale, child.scale));
    assert!(near(
        composed.orientation.rotate(v(1.0, 0.0, 0.0)),
        child.orientation.rotate(v(1.0, 0.0, 0.0))
    ));
}

#[test]
fn moving_the_target_turns_a_look_at_node_and_leaves_its_own_pose_alone() {
    let mut s = Scene::new();
    let root = s.root();
    let n = s.add_child(root, "n").unwrap();
    let t = s.add_child(root, "t").unwrap();
    s.set_pose(t, at(v(10.0, 0.0, 0.0)));
    s.set_constraint(
        n,
        Some(Constraint::LookAt {
            target: t,
            world_up: v(0.0, 1.0, 0.0),
        }),
    );
    let facing = s
        .world_pose(n)
        .unwrap()
        .orientation
        .rotate(v(0.0, 0.0, -1.0));
    assert!(near(facing, v(1.0, 0.0, 0.0)));
    s.set_pose(t, at(v(0.0, 0.0, 10.0)));
    let turned = s
        .world_pose(n)
        .unwrap()
        .orientation
        .rotate(v(0.0, 0.0, -1.0));
    assert!(near(turned, v(0.0, 0.0, 1.0)));
    assert_eq!(s.local_pose(n), Some(Pose::identity()));
}

#[test]
fn removing_a_look_at_constraint_restores_the_plain_pose() {
    let mut s = Scene::new();
    let root = s.root();
    let n = s.add_child(root, "n").unwrap();
    let t = s.add_child(root, "t").unwrap();
    s.set_pose(t, at(v(10.0, 0.0, 0.0)));
    assert!(s.set_constraint(
        n,
        Some(Constraint::LookAt {
            target: t,
            world_up: v(0.0, 1.0, 0.0)
        })
    ));
    assert!(s.set_constraint(n, None));
    let facing = s
        .world_pose(n)
        .unwrap()
        .orientation
        .rotate(v(0.0, 0.0, -1.0));
    assert!(near(facing, v(0.0, 0.0, -1.0)));
}

#[test]
fn a_cycle_of_look_at_constraints_still_resolves_each_node() {
    let mut s = Scene::new();
    let root = s.root();
    let a = s.add_child(root, "a").unwrap();
    let b = s.add_child(root, "b").unwrap();
    s.set_pose(a, at(v(5.0, 0.0, 0.0)));
    s.set_pose(b, at(v(0.0, 0.0, 5.0)));
    s.set_constraint(
        a,
        Some(Constraint::LookAt {
            target: b,
            world_up: v(0.0, 1.0, 0.0),
        }),
    );
    s.set_constraint(
        b,
        Some(Constraint::LookAt {
            target: a,
            world_up: v(0.0, 1.0, 0.0),
        }),
    );
    assert!(near(s.world_position(a).unwrap(), v(5.0, 0.0, 0.0)));
    assert!(near(s.world_position(b).unwrap(), v(0.0, 0.0, 5.0)));
}

// ---- cameras ----

#[test]
fn a_camera_is_stored_on_its_node_and_can_be_cleared() {
    let mut s = Scene::new();
    let root = s.root();
    let cam = s.add_child(root, "cam").unwrap();
    let lens = Camera::new(60.0, 1.5, 0.1, 50.0);
    assert!(s.set_camera(cam, Some(lens)));
    assert_eq!(s.camera(cam), Some(lens));
    assert!(s.camera_view(cam).is_some());
    assert!(s.set_camera(cam, None));
    assert_eq!(s.camera(cam), None);
    assert!(s.camera_view(cam).is_none());
}

#[test]
fn a_horizontal_angle_at_aspect_one_equals_the_vertical_angle() {
    let c = Camera::new(60.0, 1.0, 0.1, 50.0);
    assert_eq!(c.direction, ProjectionDirection::Horizontal);
    assert!(approx(
        c.vertical_fov_radians(),
        std::f32::consts::FRAC_PI_3
    ));
}

#[test]
fn a_vertical_angle_sets_the_vertical_focal_term_whatever_the_aspect() {
    // A 90 degree vertical angle has tan(45) = 1 for the vertical term; the horizontal term is that divided by the aspect.
    let c = Camera {
        field_of_view: 90.0,
        direction: ProjectionDirection::Vertical,
        aspect: 2.0,
        z_near: 0.1,
        z_far: 100.0,
    };
    let p = c.projection();
    assert!(approx(p.m[5], 1.0));
    assert!(approx(p.m[0], 0.5));
}

#[test]
fn the_camera_view_undoes_the_camera_world_matrix() {
    let mut s = Scene::new();
    let root = s.root();
    let cam = s.add_child(root, "cam").unwrap();
    s.set_pose(
        cam,
        Pose {
            position: v(3.0, -2.0, 10.0),
            orientation: Quaternion::from_axis_angle(v(0.0, 1.0, 0.0), 0.7),
            scale: v(1.0, 1.0, 1.0),
        },
    );
    s.set_camera(cam, Some(Camera::new(60.0, 1.5, 0.1, 50.0)));
    let world = s.world_matrix(cam).unwrap();
    let view = s.camera_view(cam).unwrap();
    let p = v(1.0, 2.0, 3.0);
    let round_trip = view
        .transform_point(world.transform_point(p).unwrap())
        .unwrap();
    assert!(near(round_trip, p));
}

// ---- orientation helpers ----

#[test]
fn axes_that_match_the_frame_give_the_identity_rotation() {
    let q = quaternion_from_axes(v(1.0, 0.0, 0.0), v(0.0, 1.0, 0.0), v(0.0, 0.0, 1.0));
    assert!(near(q.rotate(v(1.0, 2.0, 3.0)), v(1.0, 2.0, 3.0)));
}

#[test]
fn axes_of_a_quarter_turn_about_z_rotate_x_onto_y() {
    let q = quaternion_from_axes(v(0.0, 1.0, 0.0), v(-1.0, 0.0, 0.0), v(0.0, 0.0, 1.0));
    assert!(near(q.rotate(v(1.0, 0.0, 0.0)), v(0.0, 1.0, 0.0)));
    assert!(near(q.rotate(v(0.0, 1.0, 0.0)), v(-1.0, 0.0, 0.0)));
}

#[test]
fn axes_of_a_half_turn_about_x_flip_y_and_z() {
    // The trace is -1 here, which takes the branch where X is the dominant diagonal term.
    let q = quaternion_from_axes(v(1.0, 0.0, 0.0), v(0.0, -1.0, 0.0), v(0.0, 0.0, -1.0));
    assert!(near(q.rotate(v(0.0, 1.0, 0.0)), v(0.0, -1.0, 0.0)));
    assert!(near(q.rotate(v(0.0, 0.0, 1.0)), v(0.0, 0.0, -1.0)));
    assert!(near(q.rotate(v(1.0, 0.0, 0.0)), v(1.0, 0.0, 0.0)));
}

#[test]
fn look_at_orientation_points_the_front_axis_at_the_target() {
    let q = look_at_orientation(v(0.0, 0.0, 0.0), v(5.0, 0.0, 0.0), v(0.0, 1.0, 0.0)).unwrap();
    assert!(near(q.rotate(v(0.0, 0.0, -1.0)), v(1.0, 0.0, 0.0)));
    assert!(near(q.rotate(v(0.0, 1.0, 0.0)), v(0.0, 1.0, 0.0)));
}

#[test]
fn look_at_orientation_toward_the_back_is_the_identity_turn() {
    let q = look_at_orientation(v(0.0, 0.0, 0.0), v(0.0, 0.0, -5.0), v(0.0, 1.0, 0.0)).unwrap();
    assert!(near(q.rotate(v(0.0, 0.0, -1.0)), v(0.0, 0.0, -1.0)));
}

#[test]
fn look_at_orientation_is_undefined_for_coincident_points() {
    assert_eq!(
        look_at_orientation(v(1.0, 1.0, 1.0), v(1.0, 1.0, 1.0), v(0.0, 1.0, 0.0)),
        None
    );
}

#[test]
fn look_at_orientation_is_undefined_when_up_runs_along_the_view() {
    assert_eq!(
        look_at_orientation(v(0.0, 0.0, 0.0), v(0.0, 5.0, 0.0), v(0.0, 1.0, 0.0)),
        None
    );
}

// ---- lighting ----

fn white() -> Rgb {
    Rgb::WHITE
}

fn gray(x: f32) -> Rgb {
    Rgb::new(x, x, x)
}

fn matte(ambient: Rgb, diffuse: Rgb) -> Material {
    Material {
        ambient,
        diffuse,
        specular: Rgb::BLACK,
        shininess: 1.0,
    }
}

#[test]
fn rgb_colours_modulate_scale_add_and_clamp_per_component() {
    assert!(rgb_near(
        Rgb::new(0.5, 1.0, 0.2).modulate(gray(0.5)),
        Rgb::new(0.25, 0.5, 0.1)
    ));
    assert!(rgb_near(
        Rgb::new(0.2, 0.4, 0.6).scaled(0.5),
        Rgb::new(0.1, 0.2, 0.3)
    ));
    assert_eq!(Rgb::BLACK.added(Rgb::WHITE), Rgb::WHITE);
    assert_eq!(Rgb::new(1.5, -0.2, 0.5).clamped(), Rgb::new(1.0, 0.0, 0.5));
}

#[test]
fn ambient_light_alone_shows_on_a_surface_facing_away_from_the_light() {
    // The light travels up, so the direction towards it points down; the upward normal faces away.
    let light = Light {
        source: LightSource::Directional(v(0.0, 1.0, 0.0)),
        ambient: gray(0.2),
        diffuse: white(),
        specular: white(),
    };
    let material = Material {
        ambient: white(),
        diffuse: white(),
        specular: white(),
        shininess: 8.0,
    };
    let c = shade(
        &light,
        &material,
        v(0.0, 0.0, 0.0),
        v(0.0, 1.0, 0.0),
        v(0.0, 10.0, 0.0),
    );
    assert!(rgb_near(c, gray(0.2)));
}

#[test]
fn a_directional_light_from_above_adds_the_full_diffuse_term() {
    let light = Light {
        source: LightSource::Directional(v(0.0, -1.0, 0.0)),
        ambient: Rgb::BLACK,
        diffuse: white(),
        specular: Rgb::BLACK,
    };
    let c = shade(
        &light,
        &matte(Rgb::BLACK, gray(0.4)),
        v(0.0, 0.0, 0.0),
        v(0.0, 1.0, 0.0),
        v(0.0, 10.0, 0.0),
    );
    assert!(rgb_near(c, gray(0.4)));
}

#[test]
fn a_point_light_has_no_distance_falloff_in_this_model() {
    let light = |p: Vector3| Light {
        source: LightSource::Point(p),
        ambient: Rgb::BLACK,
        diffuse: white(),
        specular: Rgb::BLACK,
    };
    let material = matte(Rgb::BLACK, gray(0.4));
    let near_light = shade(
        &light(v(0.0, 2.0, 0.0)),
        &material,
        v(0.0, 0.0, 0.0),
        v(0.0, 1.0, 0.0),
        v(0.0, 10.0, 0.0),
    );
    let far_light = shade(
        &light(v(0.0, 500.0, 0.0)),
        &material,
        v(0.0, 0.0, 0.0),
        v(0.0, 1.0, 0.0),
        v(0.0, 10.0, 0.0),
    );
    assert!(rgb_near(near_light, gray(0.4)));
    assert!(rgb_near(far_light, gray(0.4)));
}

#[test]
fn a_light_from_the_side_is_dimmed_by_the_cosine_of_the_angle() {
    // The direction to the light is (1, 1, 0) / sqrt 2 and the normal is up, so the Lambert term is about 0.7071.
    let light = Light {
        source: LightSource::Point(v(1.0, 1.0, 0.0)),
        ambient: Rgb::BLACK,
        diffuse: white(),
        specular: Rgb::BLACK,
    };
    let c = shade(
        &light,
        &matte(Rgb::BLACK, gray(0.4)),
        v(0.0, 0.0, 0.0),
        v(0.0, 1.0, 0.0),
        v(0.0, 10.0, 0.0),
    );
    assert!(approx(c.r, 0.4 * 0.70710678));
}

#[test]
fn a_surface_facing_away_from_a_directional_light_gets_no_diffuse_term() {
    let light = Light {
        source: LightSource::Directional(v(0.0, 1.0, 0.0)),
        ambient: Rgb::BLACK,
        diffuse: white(),
        specular: Rgb::BLACK,
    };
    let c = shade(
        &light,
        &matte(Rgb::BLACK, gray(0.4)),
        v(0.0, 0.0, 0.0),
        v(0.0, 1.0, 0.0),
        v(0.0, 10.0, 0.0),
    );
    assert!(rgb_near(c, Rgb::BLACK));
}

#[test]
fn a_highlight_appears_when_the_eye_sits_on_the_mirror_path() {
    let light = Light {
        source: LightSource::Directional(v(0.0, -1.0, 0.0)),
        ambient: Rgb::BLACK,
        diffuse: Rgb::BLACK,
        specular: gray(0.5),
    };
    let material = Material {
        ambient: Rgb::BLACK,
        diffuse: Rgb::BLACK,
        specular: white(),
        shininess: 16.0,
    };
    let c = shade(
        &light,
        &material,
        v(0.0, 0.0, 0.0),
        v(0.0, 1.0, 0.0),
        v(0.0, 10.0, 0.0),
    );
    assert!(rgb_near(c, gray(0.5)));
}

#[test]
fn a_highlight_falls_away_as_the_eye_leaves_the_mirror_path() {
    // Half vector (1, 1, 0) / sqrt 2 gives n.h = 0.7071; raised to 16 that is 1/256, so the highlight is 0.5 / 256.
    let light = Light {
        source: LightSource::Directional(v(0.0, -1.0, 0.0)),
        ambient: Rgb::BLACK,
        diffuse: Rgb::BLACK,
        specular: gray(0.5),
    };
    let material = Material {
        ambient: Rgb::BLACK,
        diffuse: Rgb::BLACK,
        specular: white(),
        shininess: 16.0,
    };
    let c = shade(
        &light,
        &material,
        v(0.0, 0.0, 0.0),
        v(0.0, 1.0, 0.0),
        v(10.0, 0.0, 0.0),
    );
    assert!(approx(c.r, 0.5 / 256.0));
}

#[test]
fn an_unnormalised_normal_lights_the_same_as_its_unit_direction() {
    let light = Light {
        source: LightSource::Directional(v(0.0, -1.0, 0.0)),
        ambient: Rgb::BLACK,
        diffuse: white(),
        specular: Rgb::BLACK,
    };
    let material = matte(Rgb::BLACK, gray(0.4));
    let unit = shade(
        &light,
        &material,
        v(0.0, 0.0, 0.0),
        v(0.0, 1.0, 0.0),
        v(0.0, 10.0, 0.0),
    );
    let long = shade(
        &light,
        &material,
        v(0.0, 0.0, 0.0),
        v(0.0, 5.0, 0.0),
        v(0.0, 10.0, 0.0),
    );
    assert!(rgb_near(unit, long));
}

#[test]
fn lighting_is_clamped_to_the_displayable_range() {
    let light = Light {
        source: LightSource::Directional(v(0.0, -1.0, 0.0)),
        ambient: gray(0.8),
        diffuse: gray(0.8),
        specular: Rgb::BLACK,
    };
    let c = shade(
        &light,
        &matte(white(), white()),
        v(0.0, 0.0, 0.0),
        v(0.0, 1.0, 0.0),
        v(0.0, 10.0, 0.0),
    );
    assert!(rgb_near(c, white()));
}

#[test]
fn linear_fog_is_clear_before_its_start_and_full_after_its_end() {
    let fog = Fog::Linear {
        start: 10.0,
        end: 20.0,
    };
    assert!(approx(fog.amount(5.0), 0.0));
    assert!(approx(fog.amount(10.0), 0.0));
    assert!(approx(fog.amount(15.0), 0.5));
    assert!(approx(fog.amount(25.0), 1.0));
}

#[test]
fn a_linear_fog_with_equal_bounds_is_a_step() {
    let fog = Fog::Linear {
        start: 5.0,
        end: 5.0,
    };
    assert!(approx(fog.amount(4.9), 0.0));
    assert!(approx(fog.amount(5.0), 1.0));
}

#[test]
fn exponential_fog_grows_with_density_and_distance() {
    let fog = Fog::Exponential { density: 0.1 };
    assert!(approx(fog.amount(0.0), 0.0));
    assert!(approx(fog.amount(-5.0), 0.0));
    assert!(approx(fog.amount(10.0), 1.0 - (-1.0f32).exp()));
}

#[test]
fn fog_mixes_the_surface_colour_toward_the_fog_colour() {
    let fog = Fog::Linear {
        start: 0.0,
        end: 10.0,
    };
    assert!(rgb_near(fog.apply(white(), Rgb::BLACK, 5.0), gray(0.5)));
}

// ---- surface materials ----

fn f(name: &str, semantic: MaterialSemantic, value: f32) -> MaterialProperty {
    MaterialProperty::new(name, semantic, MaterialValue::Float(value))
}

#[test]
fn a_new_material_has_its_shading_model_and_no_base_or_properties() {
    let m = AssetMaterial::new("plain", ShadingModel::Basic);
    assert_eq!(m.name, "plain");
    assert_eq!(m.shading, ShadingModel::Basic);
    assert!(m.base.is_none());
    assert!(m.is_empty());
    assert!(m.property_named("anything").is_none());
}

#[test]
fn a_semantic_lookup_reaches_through_a_chain_of_bases() {
    let mut grand = AssetMaterial::new("grand", ShadingModel::PhysicallyPlausible);
    grand.set_property(f("sheen_amount", MaterialSemantic::Sheen, 0.6));
    let mut base = AssetMaterial::new("base", ShadingModel::PhysicallyPlausible);
    base.base = Some(Box::new(grand));
    let mut child = AssetMaterial::new("child", ShadingModel::PhysicallyPlausible);
    child.base = Some(Box::new(base));
    assert_eq!(
        child
            .property_with(MaterialSemantic::Sheen)
            .map(|p| p.name.as_str()),
        Some("sheen_amount")
    );
    assert!(child.is_empty());
}

#[test]
fn removing_an_own_property_reveals_the_base_value_of_that_name() {
    let mut base = AssetMaterial::new("base", ShadingModel::Basic);
    base.set_property(MaterialProperty::new(
        "tint",
        MaterialSemantic::BaseColor,
        MaterialValue::Color([0.1, 0.2, 0.3, 1.0]),
    ));
    let mut child = AssetMaterial::new("child", ShadingModel::Basic);
    child.base = Some(Box::new(base));
    child.set_property(MaterialProperty::new(
        "tint",
        MaterialSemantic::BaseColor,
        MaterialValue::Color([1.0, 0.0, 0.0, 1.0]),
    ));
    assert_eq!(
        child.property_named("tint").unwrap().value,
        MaterialValue::Color([1.0, 0.0, 0.0, 1.0])
    );
    assert!(child.remove("tint").is_some());
    assert_eq!(
        child.property_named("tint").unwrap().value,
        MaterialValue::Color([0.1, 0.2, 0.3, 1.0])
    );
}

#[test]
fn a_property_shadowed_by_name_hides_the_base_property_for_its_semantic_too() {
    let mut base = AssetMaterial::new("base", ShadingModel::Basic);
    base.set_property(f("roughness", MaterialSemantic::Roughness, 0.3));
    let mut child = AssetMaterial::new("child", ShadingModel::Basic);
    child.base = Some(Box::new(base));
    child.set_property(f("roughness", MaterialSemantic::Metallic, 1.0));
    assert!(child
        .properties_with(MaterialSemantic::Roughness)
        .is_empty());
    assert_eq!(child.properties_with(MaterialSemantic::Metallic).len(), 1);
}

#[test]
fn material_values_compare_by_kind_and_content() {
    assert_ne!(
        MaterialValue::Float3(v(1.0, 2.0, 3.0)),
        MaterialValue::Float3(v(1.0, 2.0, 4.0))
    );
    assert_ne!(
        MaterialValue::Color([1.0, 0.0, 0.0, 1.0]),
        MaterialValue::Float4([1.0, 0.0, 0.0, 1.0])
    );
    assert_eq!(
        MaterialValue::Path("brick.png".into()),
        MaterialValue::Path("brick.png".into())
    );
    assert_ne!(MaterialSemantic::UserDefined, MaterialSemantic::None);
}

// ---- shader entry points ----

#[test]
fn entry_point_names_follow_the_identifier_rule() {
    assert!(is_valid_name("quad"));
    assert!(is_valid_name("_a"));
    assert!(is_valid_name("Kernel2"));
    assert!(!is_valid_name("2fast"));
    assert!(!is_valid_name("a.b"));
    assert!(!is_valid_name("naive\u{00ef}"));
    assert!(!is_valid_name(""));
    assert!(is_valid_name(&"a".repeat(MAX_NAME_LEN)));
    assert!(!is_valid_name(&"a".repeat(MAX_NAME_LEN + 1)));
}

#[test]
fn a_library_keeps_entry_points_in_registration_order_and_refuses_repeats() {
    let mut lib = ShaderLibrary::with_label("effects");
    lib.add("vs", Stage::Vertex).unwrap();
    lib.add("fs", Stage::Fragment).unwrap();
    assert_eq!(
        lib.add("vs", Stage::Vertex),
        Err(LibraryError::Duplicate("vs".into()))
    );
    assert_eq!(
        lib.add("bad-name", Stage::Kernel),
        Err(LibraryError::InvalidName("bad-name".into()))
    );
    assert_eq!(lib.function_names(), vec!["vs", "fs"]);
    assert_eq!(lib.label.as_deref(), Some("effects"));
}

#[test]
fn a_lookup_returns_the_stage_or_reports_the_missing_name() {
    let mut lib = ShaderLibrary::new();
    lib.add("k", Stage::Kernel).unwrap();
    assert_eq!(lib.make_function("k").unwrap().stage, Stage::Kernel);
    assert_eq!(
        lib.make_function(""),
        Err(LibraryError::NotFound(String::new()))
    );
}

#[test]
fn a_render_pipeline_names_the_missing_function() {
    let mut lib = ShaderLibrary::new();
    lib.add("vs", Stage::Vertex).unwrap();
    assert_eq!(
        lib.check_render_pipeline("vs", "nope"),
        Err(LibraryError::NotFound("nope".into()))
    );
}

#[test]
fn a_source_scan_reads_each_stage_keyword_and_skips_other_lines() {
    let src = "float4 helper(\nkernel void k(\nvertex float4 vmain(uint i)\nfragment half4 fmain(float4 p)\nkernel void";
    let lib = ShaderLibrary::from_source(src).unwrap();
    assert_eq!(lib.function_names(), vec!["k", "vmain", "fmain"]);
    assert_eq!(lib.make_function("vmain").unwrap().stage, Stage::Vertex);
    assert!(lib.check_render_pipeline("vmain", "fmain").is_ok());
}

#[test]
fn a_source_scan_that_repeats_an_entry_point_fails() {
    assert_eq!(
        ShaderLibrary::from_source("kernel void k(\nkernel void k("),
        Err(LibraryError::Duplicate("k".into()))
    );
}

#[test]
fn a_default_render_descriptor_is_single_sampled_and_unbound() {
    let d = RenderPipelineDescriptor::default();
    assert_eq!(d.raster_sample_count, 1);
    assert_eq!(d.label, None);
    assert!(d.vertex_function.is_none() && d.fragment_function.is_none());
}

#[test]
fn a_render_descriptor_accepts_the_multisample_counts() {
    let mut lib = ShaderLibrary::new();
    lib.add("vs", Stage::Vertex).unwrap();
    lib.add("fs", Stage::Fragment).unwrap();
    let mut d = RenderPipelineDescriptor {
        vertex_function: Some("vs".into()),
        fragment_function: Some("fs".into()),
        ..Default::default()
    };
    for n in [1, 2, 4, 8] {
        d.raster_sample_count = n;
        assert_eq!(d.validate(&lib), Ok(()));
    }
}

#[test]
fn a_render_descriptor_refuses_other_sample_counts() {
    let mut lib = ShaderLibrary::new();
    lib.add("vs", Stage::Vertex).unwrap();
    lib.add("fs", Stage::Fragment).unwrap();
    let mut d = RenderPipelineDescriptor {
        vertex_function: Some("vs".into()),
        fragment_function: Some("fs".into()),
        ..Default::default()
    };
    d.raster_sample_count = 0;
    assert_eq!(d.validate(&lib), Err(LibraryError::InvalidSampleCount(0)));
    d.raster_sample_count = 16;
    assert_eq!(d.validate(&lib), Err(LibraryError::InvalidSampleCount(16)));
}

#[test]
fn a_render_descriptor_refuses_a_fragment_function_in_the_vertex_slot() {
    let mut lib = ShaderLibrary::new();
    lib.add("fs", Stage::Fragment).unwrap();
    lib.add("fs2", Stage::Fragment).unwrap();
    let d = RenderPipelineDescriptor {
        vertex_function: Some("fs".into()),
        fragment_function: Some("fs2".into()),
        ..Default::default()
    };
    assert_eq!(
        d.validate(&lib),
        Err(LibraryError::WrongStage {
            name: "fs".into(),
            found: Stage::Fragment
        })
    );
}

#[test]
fn a_compute_pipeline_accepts_only_a_kernel_from_the_library() {
    let mut lib = ShaderLibrary::new();
    lib.add("k", Stage::Kernel).unwrap();
    assert_eq!(lib.check_compute_pipeline("k"), Ok(()));
    assert_eq!(
        lib.check_compute_pipeline("other"),
        Err(LibraryError::NotFound("other".into()))
    );
}

// ---- window list records ----

fn win(number: u32, layer: i32, on_screen: bool) -> WindowInfo {
    WindowInfo {
        number,
        owner_pid: 100 + number,
        owner_name: format!("app{number}"),
        name: Some(format!("title{number}")),
        layer,
        bounds: Rect {
            x: 0.0,
            y: 0.0,
            w: 100.0,
            h: 80.0,
        },
        alpha: 1.0,
        on_screen,
        sharing: SharingState::None,
    }
}

/// Front to back: 1 (on screen), 2 (off screen), 3 (desktop element, on screen), 4 (on screen).
fn stack() -> Vec<WindowInfo> {
    vec![
        win(1, 0, true),
        win(2, 0, false),
        win(3, -20, true),
        win(4, 0, true),
    ]
}

fn numbers(list: &[WindowInfo]) -> Vec<u32> {
    list.iter().map(|w| w.number).collect()
}

#[test]
fn on_screen_windows_without_desktop_elements_are_the_visible_normal_windows() {
    let o = WindowListOptions {
        scope: WindowScope::OnScreenOnly,
        exclude_desktop_elements: true,
    };
    assert_eq!(numbers(&window_list(&stack(), o)), vec![1, 4]);
}

#[test]
fn an_above_scope_ignores_a_reference_window_that_is_off_screen() {
    // Window 2 is hidden but still has a stacking position, so only window 1 lies above it on screen.
    let o = WindowListOptions {
        scope: WindowScope::OnScreenAbove(2),
        exclude_desktop_elements: false,
    };
    assert_eq!(numbers(&window_list(&stack(), o)), vec![1]);
    assert_eq!(is_above(&stack(), 2, 1), Some(false));
}

#[test]
fn a_below_scope_from_the_back_window_is_empty() {
    let o = WindowListOptions {
        scope: WindowScope::OnScreenBelow(4),
        exclude_desktop_elements: false,
    };
    assert!(window_list(&stack(), o).is_empty());
}

#[test]
fn the_front_window_has_stacking_index_zero() {
    assert_eq!(stacking_index(&stack(), 1), Some(0));
    assert_eq!(stacking_index(&stack(), 0), None);
}

#[test]
fn only_windows_below_the_normal_layer_are_desktop_elements() {
    assert!(!bifrost_ui::window_list::is_desktop_element(&win(
        5,
        NORMAL_LAYER,
        true
    )));
    assert!(bifrost_ui::window_list::is_desktop_element(&win(
        5, -1, true
    )));
}

#[test]
fn a_list_on_the_wire_skips_records_that_do_not_parse() {
    let j = Json::Arr(vec![
        win(1, 0, true).to_json(),
        Json::Null,
        win(2, 0, false).to_json(),
    ]);
    assert_eq!(numbers(&list_from_json(&j)), vec![1, 2]);
    assert_eq!(
        numbers(&list_from_json(&list_to_json(&stack()))),
        vec![1, 2, 3, 4]
    );
}

#[test]
fn a_value_that_is_not_a_list_decodes_to_nothing() {
    assert!(list_from_json(&Json::Null).is_empty());
    assert_eq!(WindowInfo::from_json(&Json::Null), None);
}

#[test]
fn redaction_keeps_only_the_titles_of_the_callers_own_windows() {
    let mut s = stack();
    redact_names(&mut s, 103);
    assert_eq!(s[2].name.as_deref(), Some("title3"));
    assert!(s
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != 2)
        .all(|(_, w)| w.name.is_none()));
}

#[test]
fn sharing_codes_round_trip_and_unknown_codes_mean_no_sharing() {
    assert_eq!(SharingState::ReadWrite.code(), 2);
    assert_eq!(SharingState::from_code(2), SharingState::ReadWrite);
    assert_eq!(SharingState::from_code(1), SharingState::ReadOnly);
    assert_eq!(SharingState::from_code(7), SharingState::None);
}

#[test]
fn an_unknown_scope_name_on_the_wire_is_refused() {
    let j = Json::Obj(vec![(
        "scope".to_string(),
        Json::Str("sideways".to_string()),
    )]);
    assert_eq!(WindowListOptions::from_json(&j), None);
}

#[test]
fn a_missing_scope_on_the_wire_means_every_window() {
    assert_eq!(
        WindowListOptions::from_json(&Json::Null),
        Some(WindowListOptions::all())
    );
}

#[test]
fn an_above_scope_keeps_its_reference_across_the_wire() {
    let o = WindowListOptions {
        scope: WindowScope::OnScreenAbove(7),
        exclude_desktop_elements: true,
    };
    assert_eq!(WindowListOptions::from_json(&o.to_json()), Some(o));
}

#[test]
fn the_window_levels_are_ordered_and_have_distinct_names() {
    let levels: Vec<i32> = WindowLevelKey::ALL.iter().map(|k| k.level()).collect();
    assert_eq!(WindowLevelKey::ALL.len(), 20);
    assert!(levels.windows(2).all(|p| p[0] <= p[1]));
    let names: HashSet<&str> = WindowLevelKey::ALL.iter().map(|k| k.name()).collect();
    assert_eq!(names.len(), 20);
}

#[test]
fn floating_and_torn_off_menus_share_one_level() {
    assert_eq!(window_level(WindowLevelKey::Floating), 3);
    assert_eq!(
        window_level(WindowLevelKey::TornOffMenu),
        window_level(WindowLevelKey::Floating)
    );
    assert_eq!(
        WindowLevelKey::from_name("torn_off_menu"),
        Some(WindowLevelKey::TornOffMenu)
    );
}

#[test]
fn shell_windows_sit_on_the_levels_of_their_kind() {
    assert_eq!(level_of_shell_window(ShellWindowKind::MenuBar), 24);
    assert_eq!(level_of_shell_window(ShellWindowKind::Desktop), -20);
    assert_eq!(level_of_shell_window(ShellWindowKind::App), NORMAL_LAYER);
}

#[test]
fn capture_options_default_to_keeping_the_frame_and_the_alpha() {
    assert_eq!(
        ImageOptions::default(),
        ImageOptions {
            ignore_framing: false,
            opaque: false
        }
    );
    let o = ImageOptions {
        ignore_framing: false,
        opaque: true,
    };
    assert_eq!(ImageOptions::from_json(&o.to_json()), o);
}

#[test]
fn making_pixels_opaque_sets_only_the_alpha_byte_and_is_idempotent() {
    let mut px = [0x00ff_0000u32, 0x8000_0001];
    make_opaque(&mut px);
    assert_eq!(px, [0xffff_0000, 0xff00_0001]);
    make_opaque(&mut px);
    assert_eq!(px, [0xffff_0000, 0xff00_0001]);
}
