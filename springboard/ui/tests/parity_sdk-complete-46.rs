//! Parity sdk-complete-46: headless tests of the angle trigonometry and the pane link router. No test opens Settings.

use bifrost_ui::angle::Angle;
use bifrost_ui::pane_link::{pane_url, route, PaneLink, Route};

const SECURITY: &str = "com.apple.preference.security";
const DISPLAYS: &str = "com.apple.preference.displays";

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-3
}

#[test]
fn headless_atan2_reads_the_direction_of_a_vector_in_degrees() {
    assert!(close(Angle::atan2(1.0, 1.0).as_degrees(), 45.0));
    assert!(close(Angle::atan2(-1.0, 0.0).as_degrees(), -90.0));
}

#[test]
fn headless_asin_and_acos_refuse_values_outside_minus_one_to_one() {
    assert_eq!(Angle::asin(2.0), None);
    assert_eq!(Angle::acos(-1.5), None);
    assert!(close(Angle::asin(1.0).unwrap().as_degrees(), 90.0));
    assert!(close(Angle::acos(-1.0).unwrap().as_degrees(), 180.0));
}

#[test]
fn headless_sin_cos_matches_sin_and_cos_for_one_angle() {
    let a = Angle::degrees(90.0);
    let (s, c) = a.sin_cos();
    assert!(close(s, 1.0));
    assert!(close(c, 0.0));
    assert!(close(s, a.sin()) && close(c, a.cos()));
}

#[test]
fn headless_lerp_moves_between_two_angles_and_extrapolates_past_them() {
    let from = Angle::degrees(0.0);
    let to = Angle::degrees(90.0);
    assert!(close(from.lerp(to, 0.5).as_degrees(), 45.0));
    assert!(close(from.lerp(to, 2.0).as_degrees(), 180.0));
}

#[test]
fn headless_division_and_compound_assignment_keep_the_unit() {
    let mut a = Angle::degrees(30.0);
    a += Angle::degrees(60.0);
    assert!(close(a.as_degrees(), 90.0));
    a -= Angle::degrees(30.0);
    assert!(close(a.as_degrees(), 60.0));
    assert!(close((a / 2.0).as_degrees(), 30.0));
}

#[test]
fn headless_a_pane_link_to_an_open_pane_routes_to_it_with_its_anchor() {
    let url = pane_url(SECURITY, Some("Privacy_Camera")).unwrap();
    assert_eq!(
        route(&url, &[DISPLAYS, SECURITY]),
        Route::Open(PaneLink { identifier: SECURITY.into(), anchor: Some("Privacy_Camera".into()) })
    );
}

#[test]
fn headless_a_well_formed_link_to_an_unlisted_pane_is_unknown_not_invalid() {
    let url = pane_url(SECURITY, None).unwrap();
    assert_eq!(route(&url, &[DISPLAYS]), Route::UnknownPane(PaneLink { identifier: SECURITY.into(), anchor: None }));
}

#[test]
fn headless_text_that_is_not_a_pane_link_is_invalid() {
    assert_eq!(route("https://example.com", &[DISPLAYS]), Route::Invalid);
    assert_eq!(route(concat!("x-", "ap", "ple.systempreferences:"), &[DISPLAYS]), Route::Invalid);
}
