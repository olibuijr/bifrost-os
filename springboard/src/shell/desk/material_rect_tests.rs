use super::*;
use bifrost_render::glass::Kind;
#[test]
fn an_app_sidebar_region_reaches_the_window_edge_and_grows_inside() {
    // sidebar at the window's left edge under a 28 px title bar, window 400x300, radius 10
    let g = expand_material((0.0, 28.0, 120.0, 272.0, 0.0, Kind::Sidebar, None), (400.0, 300.0), false, 10.0);
    assert_eq!((g.x, g.y, g.w, g.h), (0.0, 18.0, 130.0, 282.0));
    assert_eq!(g.r, 10.0);
}
#[test]
fn a_light_dark_switch_changes_the_cached_window_look() {
    let _lock = crate::test_support::global_lock();
    bifrost_render::appearance::jump_to(false);
    let dark = window_look();
    bifrost_render::appearance::jump_to(true);
    let light = window_look();
    bifrost_render::appearance::jump_to(false);
    assert_ne!(dark, light, "a window cached in Dark is not reused in Light (E2E13 finding 3)");
    assert_eq!(window_look(), dark, "the same look reuses the cache");
}
#[test]
fn a_maximized_window_keeps_the_declared_rect() {
    let g = expand_material((0.0, 28.0, 120.0, 272.0, 6.0, Kind::Sidebar, None), (400.0, 300.0), true, 10.0);
    assert_eq!((g.x, g.y, g.w, g.h, g.r), (0.0, 28.0, 120.0, 272.0, 6.0));
}
