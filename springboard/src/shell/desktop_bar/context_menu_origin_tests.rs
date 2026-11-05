use super::context_menu_origin;

// the area under the bar and above the Dock: 1000 x 600 px, its top at y 30
const AREA: (i32, i32, i32, i32) = (0, 30, 1000, 600);

#[test]
fn a_menu_that_fits_opens_with_its_corner_at_the_pointer() {
    assert_eq!(context_menu_origin((300., 200.), (264, 400), AREA), (300, 200));
}

#[test]
fn a_menu_flips_left_at_the_right_edge_and_up_at_the_bottom_edge() {
    // 1000 - 264 = 736: a pointer at 800 leaves no room to the right
    assert_eq!(context_menu_origin((800., 200.), (264, 100), AREA), (536, 200), "its right edge is the pointer");
    // the area ends at 630: a 100 px menu at 560 would end at 660
    assert_eq!(context_menu_origin((300., 560.), (264, 100), AREA), (300, 460), "its bottom edge is the pointer");
    assert_eq!(context_menu_origin((800., 560.), (264, 100), AREA), (536, 460), "both at once");
    // a menu that ends exactly at the edge stays where it opened
    assert_eq!(context_menu_origin((736., 530.), (264, 100), AREA), (736, 530));
}

#[test]
fn a_menu_larger_than_the_area_is_held_inside_it_from_the_top_left() {
    assert_eq!(context_menu_origin((300., 300.), (264, 700), AREA), (300, 30), "too tall to flip: held at the top");
    assert_eq!(context_menu_origin((10., 300.), (2000, 10), AREA), (0, 300), "wider than the screen: held at the left");
}

#[test]
fn a_pointer_outside_the_area_is_pulled_in() {
    assert_eq!(context_menu_origin((-20., 5.), (264, 100), AREA), (0, 30));
}
