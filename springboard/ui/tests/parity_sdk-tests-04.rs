//! Parity sdk-tests-04: headless tests for the scene light model. Each test evaluates the light arriving at points of a
//! scene with plain arithmetic; nothing is shaded, drawn or rendered.

use bifrost_ui::linalg::Vector3;
use bifrost_ui::scene_light::{AssetLight, AssetLightType};

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-3
}

fn irradiance(light: &AssetLight, point: Vector3) -> f32 {
    light.irradiance_at(point).expect("this light type is computed")
}

#[test]
fn headless_a_new_light_is_white_one_lumen_and_points_down_the_negative_z_axis() {
    let light = AssetLight::new(AssetLightType::Point);
    assert_eq!(light.color, [1.0, 1.0, 1.0]);
    assert!(near(light.lumens, 1.0));
    assert_eq!(light.position, Vector3::ZERO);
    assert_eq!(light.direction, Vector3::new(0.0, 0.0, -1.0));
    assert_eq!(light.attenuation_end, f32::INFINITY, "a new light does not fade");
}

#[test]
fn headless_a_point_light_without_falloff_is_as_bright_far_away_as_near() {
    let mut light = AssetLight::new(AssetLightType::Point);
    light.lumens = 60.0;
    assert!(near(irradiance(&light, Vector3::new(1000.0, 0.0, 0.0)), 60.0));
}

#[test]
fn headless_a_point_light_is_full_at_its_start_distance_and_gone_at_its_end() {
    let mut light = AssetLight::new(AssetLightType::Point);
    light.lumens = 10.0;
    light.attenuation_start = 3.0;
    light.attenuation_end = 3.0;
    assert!(near(irradiance(&light, Vector3::new(3.0, 0.0, 0.0)), 10.0), "exactly at the start distance is full strength");
    assert!(near(irradiance(&light, Vector3::new(3.1, 0.0, 0.0)), 0.0), "past a zero-width fade is nothing");
}

#[test]
fn headless_a_point_light_fades_linearly_across_its_range_in_every_direction() {
    let mut light = AssetLight::new(AssetLightType::Point);
    light.lumens = 80.0;
    light.position = Vector3::new(0.0, 0.0, 5.0);
    light.attenuation_start = 0.0;
    light.attenuation_end = 8.0;
    // Distance 2 from the light: 1 - 2/8 of full strength.
    assert!(near(irradiance(&light, Vector3::new(0.0, 2.0, 5.0)), 60.0));
    // Distance 8 or more: nothing.
    assert!(near(irradiance(&light, Vector3::new(0.0, 0.0, -3.0)), 0.0));
}

#[test]
fn headless_a_directional_light_is_the_same_at_every_point_whatever_its_position() {
    let mut light = AssetLight::new(AssetLightType::Directional);
    light.lumens = 25.0;
    light.position = Vector3::new(9.0, -9.0, 9.0);
    for p in [Vector3::ZERO, Vector3::new(-4.0, 0.0, 2.0), Vector3::new(500.0, 500.0, 500.0)] {
        assert!(near(irradiance(&light, p), 25.0));
    }
}

#[test]
fn headless_an_ambient_light_ignores_direction_and_position() {
    let mut light = AssetLight::new(AssetLightType::Ambient);
    light.lumens = 7.5;
    light.direction = Vector3::new(1.0, 0.0, 0.0);
    light.position = Vector3::new(-2.0, 2.0, 0.0);
    assert!(near(irradiance(&light, Vector3::new(3.0, 3.0, 3.0)), 7.5));
}

#[test]
fn headless_a_spot_light_keeps_its_cone_when_its_direction_is_scaled() {
    let mut light = AssetLight::new(AssetLightType::Spot);
    light.lumens = 80.0;
    light.inner_cone_deg = 30.0;
    light.outer_cone_deg = 60.0;
    light.attenuation_start = 100.0;
    light.attenuation_end = 200.0;
    light.direction = Vector3::new(0.0, 0.0, -10.0);
    assert!(near(irradiance(&light, Vector3::new(0.0, 0.0, -5.0)), 80.0), "on the axis the direction length is irrelevant");
    // 45 degrees off the axis: cone factor 1 - (45 - 30) / 30 = 0.5.
    assert!(near(irradiance(&light, Vector3::new(5.0, 0.0, -5.0)), 40.0));
}

#[test]
fn headless_a_spot_light_lights_its_own_position_fully() {
    let mut light = AssetLight::new(AssetLightType::Spot);
    light.lumens = 33.0;
    light.position = Vector3::new(5.0, 0.0, 0.0);
    light.inner_cone_deg = 10.0;
    light.outer_cone_deg = 20.0;
    light.attenuation_start = 1.0;
    light.attenuation_end = 2.0;
    assert!(near(irradiance(&light, Vector3::new(5.0, 0.0, 0.0)), 33.0), "a point at the light has no angle to test");
}

#[test]
fn headless_a_spot_light_gives_nothing_beyond_its_outer_cone_even_inside_its_range() {
    let mut light = AssetLight::new(AssetLightType::Spot);
    light.lumens = 50.0;
    light.inner_cone_deg = 10.0;
    light.outer_cone_deg = 20.0;
    light.attenuation_start = 100.0;
    light.attenuation_end = 200.0;
    assert!(near(irradiance(&light, Vector3::new(0.0, 0.0, -5.0)), 50.0));
    assert!(near(irradiance(&light, Vector3::new(4.0, 0.0, -1.0)), 0.0), "about 76 degrees off the axis is beyond the outer cone");
}

#[test]
fn headless_a_light_type_this_model_does_not_compute_returns_none() {
    for kind in [
        AssetLightType::Unknown,
        AssetLightType::Linear,
        AssetLightType::DiscArea,
        AssetLightType::RectangularArea,
        AssetLightType::SuperElliptical,
        AssetLightType::Photometric,
        AssetLightType::Probe,
        AssetLightType::Environment,
    ] {
        assert_eq!(AssetLight::new(kind).irradiance_at(Vector3::new(1.0, 2.0, 3.0)), None, "{kind:?}");
    }
}
