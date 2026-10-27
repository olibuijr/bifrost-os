use super::{popup_damage_rect, popup_surface_rect};

const SCREEN: (usize, usize) = (1000, 800);

#[test]
fn the_exported_surface_is_the_popup_and_its_shadow() {
    // the damage rectangle of the popup at (100, 34) 200 x 150 is (88, 28, 224, 170)
    let (surface, menu) = popup_surface_rect((100, 34, 200, 150), 28, 12, 2, SCREEN);
    assert_eq!(surface, (88, 28, 224, 170));
    assert_eq!(menu, [12, 6, 200, 150], "the popup inside it: 12 px of shadow to its left, 6 px up to the bar");
}

#[test]
fn the_surface_is_clipped_to_the_screen_and_the_menu_rectangle_follows() {
    // at the left edge the shadow is cut at x 0, so the popup starts 5 px in
    let (surface, menu) = popup_surface_rect((5, 34, 100, 40), 28, 12, 2, SCREEN);
    assert_eq!(surface, (0, 28, 117, 60));
    assert_eq!(menu, [5, 6, 100, 40]);
    // at the right edge it is cut at the screen's width: 888 + 112 = 1000
    let (surface, menu) = popup_surface_rect((900, 34, 90, 40), 28, 12, 2, SCREEN);
    assert_eq!(surface, (888, 28, 112, 60));
    assert_eq!(menu, [12, 6, 90, 40]);
    // at the bottom it is cut at the screen's height: 748 + 52 = 800
    let (surface, menu) = popup_surface_rect((100, 760, 200, 30), 28, 12, 2, SCREEN);
    assert_eq!(surface, (88, 748, 224, 52));
    assert_eq!(menu, [12, 12, 200, 30]);
}

#[test]
fn the_menu_rectangle_stays_inside_the_surface_and_is_smaller_than_it() {
    for popup in [(100, 200, 264, 150), (0, 34, 300, 100), (700, 600, 300, 200)] {
        let ((sx, sy, sw, sh), menu) = popup_surface_rect(popup, 28, 12, 2, SCREEN);
        assert!(menu[0] >= 0 && menu[1] >= 0, "{popup:?}: the menu starts inside the surface");
        assert!(menu[0] + menu[2] <= sw as i32 && menu[1] + menu[3] <= sh as i32, "{popup:?}: and ends inside it");
        assert_eq!((sx as i32 + menu[0], sy as i32 + menu[1]), (popup.0 as i32, popup.1 as i32), "{popup:?}: at the popup's screen position");
        assert!(sw as i32 > menu[2] && sh as i32 > menu[3], "{popup:?}: there is shadow to show");
    }
}

// A menu bar 28 px tall, a shadow reach of 12 px (10 pt + 2 at scale 1) and a 2 px lip.
#[test]
fn a_menu_below_the_bar_owes_its_shadow_up_to_the_bar() {
    // popup at (100, 34) 200 x 150: the top shadow would start at y 22, clamped to the bar's bottom (28);
    // left 88, right 100 + 200 + 12 = 312 (width 224); bottom 34 + 150 + 12 + 2 = 198 (height 170)
    assert_eq!(popup_damage_rect((100, 34, 200, 150), 28, 12, 2), (88, 28, 224, 170));
}

#[test]
fn a_popup_lower_on_screen_starts_its_damage_above_its_top_edge() {
    // top shadow starts at 70 - 12 = 58, which is inside the gap under the bar, so it is damaged
    let (_, top, _, _) = popup_damage_rect((10, 70, 100, 80), 28, 12, 2);
    assert_eq!(top, 58);
}

#[test]
fn a_menu_at_the_left_edge_is_clamped_to_the_screen() {
    // x 5 - 12 is negative, so the rectangle starts at 0 and reaches 5 + 100 + 12 = 117 (width 117);
    // top 34 - 12 = 22 clamps to 28; bottom 34 + 40 + 12 + 2 = 88 (height 60)
    assert_eq!(popup_damage_rect((5, 34, 100, 40), 28, 12, 2), (0, 28, 117, 60));
}

#[test]
fn the_rectangle_always_contains_the_popup() {
    let (x, y, w, h) = popup_damage_rect((300, 34, 180, 220), 28, 12, 2);
    assert!(x <= 300 && y <= 34);
    assert!(x + w >= 300 + 180 && y + h >= 34 + 220);
}
