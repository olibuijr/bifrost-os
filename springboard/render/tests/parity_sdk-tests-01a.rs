//! Parity sdk-tests-01a: headless tests for the gamma fade ramp and the glass look arithmetic. Both are pure functions over
//! numbers; no test touches a display, a gamma table or a GPU.

use bifrost_render::gamma_fade::{FadeDirection, GammaFade, DEFAULT_FADE_SECS};
use bifrost_render::glass::{Kind, Look};

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-5
}

fn close3(a: [f32; 3], b: [f32; 3]) -> bool {
    close(a[0], b[0]) && close(a[1], b[1]) && close(a[2], b[2])
}

#[test]
fn the_default_fade_is_half_a_second() {
    assert_eq!(DEFAULT_FADE_SECS, 0.5);
    assert_eq!(GammaFade::new(DEFAULT_FADE_SECS).duration(), 0.5);
}

#[test]
fn a_fade_in_rises_and_a_fade_out_falls_linearly() {
    let fade = GammaFade::new(0.5);
    assert_eq!(fade.level(FadeDirection::In, 0.25), 0.5);
    assert_eq!(fade.level(FadeDirection::Out, 0.25), 0.5);
    assert_eq!(fade.level(FadeDirection::In, 0.125), 0.25);
    assert_eq!(fade.level(FadeDirection::Out, 0.125), 0.75);
}

#[test]
fn elapsed_times_outside_the_fade_clamp_to_its_ends() {
    let fade = GammaFade::new(0.5);
    assert_eq!(fade.level(FadeDirection::In, -3.0), 0.0);
    assert_eq!(fade.level(FadeDirection::In, 9.0), 1.0);
    assert_eq!(fade.level(FadeDirection::Out, 9.0), 0.0);
    assert_eq!(fade.level(FadeDirection::In, f32::NAN), 0.0, "a NaN time counts as the start");
    assert_eq!(fade.level(FadeDirection::Out, f32::NAN), 1.0);
}

#[test]
fn a_fade_without_a_length_is_already_at_its_end() {
    assert_eq!(GammaFade::new(-1.0).duration(), 0.0);
    assert_eq!(GammaFade::new(f32::INFINITY).duration(), 0.0);
    assert_eq!(GammaFade::new(0.0).level(FadeDirection::In, 0.0), 1.0);
    assert_eq!(GammaFade::new(0.0).level(FadeDirection::Out, 0.0), 0.0);
}

#[test]
fn a_ramp_lists_evenly_spaced_levels_from_start_to_end() {
    let fade = GammaFade::new(0.5);
    assert_eq!(fade.ramp(FadeDirection::In, 5), vec![0.0, 0.25, 0.5, 0.75, 1.0]);
    assert_eq!(fade.ramp(FadeDirection::Out, 3), vec![1.0, 0.5, 0.0]);
    assert!(fade.ramp(FadeDirection::In, 0).is_empty());
    assert_eq!(fade.ramp(FadeDirection::In, 1), vec![1.0], "one step gives the end level alone");
}

fn identity() -> Look {
    Look { saturation: 1.0, brightness: 0.0, lum_amount: 0.0, lum_values: [0.0; 4] }
}

#[test]
fn an_identity_look_leaves_the_backdrop_unchanged() {
    assert!(close3(identity().apply([0.2, 0.4, 0.6]), [0.2, 0.4, 0.6]));
}

#[test]
fn zero_saturation_turns_the_backdrop_to_its_luma() {
    let grey = Look { saturation: 0.0, ..identity() };
    let luma = 1.0 * 0.2126;
    assert!(close3(grey.apply([1.0, 0.0, 0.0]), [luma, luma, luma]));
}

#[test]
fn brightness_is_added_and_clamped_to_full_scale() {
    let bright = Look { brightness: 0.5, ..identity() };
    assert!(close3(bright.apply([0.8, 0.8, 0.8]), [1.0, 1.0, 1.0]));
    assert!(close3(bright.apply([0.1, 0.2, 0.3]), [0.6, 0.7, 0.8]));
}

#[test]
fn mixing_looks_follows_the_scheme_position() {
    let dark = Look { saturation: 1.0, brightness: 0.0, lum_amount: 0.0, lum_values: [0.0, 0.0, 0.0, 0.0] };
    let light = Look { saturation: 2.0, brightness: 0.2, lum_amount: 1.0, lum_values: [1.0, 1.0, 1.0, 1.0] };
    let half = Look::mix(dark, light, 0.5);
    assert!(close(half.saturation, 1.5));
    assert!(close(half.brightness, 0.1));
    assert!(close(half.lum_amount, 0.5));
    assert!(close(half.lum_values[3], 0.5));
    let at_dark = Look::mix(dark, light, 0.0);
    assert!(close(at_dark.saturation, 1.0));
    assert!(close(Look::mix(dark, light, 1.0).saturation, 2.0));
}

#[test]
fn chrome_and_title_bars_share_the_title_bar_recipe_and_chrome_is_the_default() {
    assert_eq!(Kind::default(), Kind::Chrome);
    assert_eq!(Kind::Chrome.looks(), Kind::Titlebar.looks());
    let (dark, light) = Kind::Chrome.looks();
    assert!(close(dark.saturation, 2.0));
    assert!(close(light.saturation, 1.1));
    let (menu_dark, _) = Kind::Menu.looks();
    assert!(close(menu_dark.saturation, 1.5));
}

#[test]
fn sidebars_are_thinner_glass_than_window_backgrounds() {
    let (sidebar, _) = Kind::Sidebar.looks();
    let (content, _) = Kind::Content.looks();
    assert!(close(sidebar.lum_amount, 0.6));
    assert!(close(content.lum_amount, 0.88));
    assert!(sidebar.saturation < content.saturation + 0.2);
}
