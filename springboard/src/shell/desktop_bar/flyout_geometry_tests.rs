use super::{clear_covered, flyout_origin, in_safe_triangle, Flyout, Menu, FLYOUT_OPEN_MS, FLYOUT_SAFE_MS};

const AREA: (i32, i32, i32, i32) = (0, 40, 1000, 700);

#[test]
fn a_submenu_opens_right_of_its_parent_with_its_first_row_level_with_the_header() {
    // a 200 px menu at x 100, header row at y 160, a 180 x 120 submenu
    assert_eq!(flyout_origin((100, 50, 200, 300), 160, (180, 120), AREA, 4, 8, true), (296, 152, true));
}

#[test]
fn it_opens_left_without_room_on_the_right_and_keeps_going_left() {
    assert_eq!(flyout_origin((700, 50, 200, 300), 160, (180, 120), AREA, 4, 8, true), (524, 152, false), "no room right of 900");
    // the next level keeps the left side while it has room, even though the right would fit too
    assert_eq!(flyout_origin((400, 50, 200, 300), 160, (180, 120), AREA, 4, 8, false), (224, 152, false));
    // and turns right again once the left runs out
    assert_eq!(flyout_origin((100, 50, 200, 300), 160, (180, 120), AREA, 4, 8, false), (296, 152, true));
}

#[test]
fn a_submenu_that_fits_on_neither_side_goes_where_there_is_more_room_inside_the_area() {
    let (x, _, right) = flyout_origin((300, 50, 500, 300), 160, (600, 120), AREA, 4, 8, true);
    assert!(!right, "300 px on the left, 200 on the right");
    assert_eq!(x, 0);
    let (x, _, right) = flyout_origin((100, 50, 500, 300), 160, (600, 120), AREA, 4, 8, true);
    assert!(right);
    assert_eq!(x, 400, "held inside the area: 1000 - 600");
}

#[test]
fn a_submenu_is_held_inside_the_area_vertically() {
    assert_eq!(flyout_origin((100, 50, 200, 700), 680, (180, 120), AREA, 4, 8, true).1, 620, "pushed up from the bottom edge");
    assert_eq!(flyout_origin((100, 50, 200, 300), 20, (180, 120), AREA, 4, 8, true).1, 40, "never above the area");
    assert_eq!(flyout_origin((100, 50, 200, 300), 20, (180, 900), AREA, 4, 8, true).1, 40, "a submenu taller than the area starts at its top");
}

#[test]
fn the_safe_triangle_covers_moves_toward_the_submenu_only() {
    let sub = (300., 100., 200., 300.);
    let apex = (150., 120.);
    assert!(in_safe_triangle(apex, (250., 200.), sub, true), "down and toward the near edge");
    assert!(in_safe_triangle(apex, (280., 110.), sub, true), "up and toward it, below its top corner");
    assert!(!in_safe_triangle(apex, (140., 200.), sub, true), "away from it");
    assert!(!in_safe_triangle(apex, (200., 60.), sub, true), "above its top corner");
    assert!(!in_safe_triangle(apex, (200., 400.), sub, true), "steeper than its bottom corner");
    // a submenu on the left: the near edge is its right side
    let left = (0., 100., 100., 300.);
    assert!(in_safe_triangle((250., 120.), (150., 200.), left, false));
    assert!(!in_safe_triangle((250., 120.), (260., 200.), left, false));
    // a degenerate triangle (the apex on the edge's line) holds nothing
    assert!(!in_safe_triangle((300., 120.), (300., 150.), sub, true));
    assert!(!in_safe_triangle((f32::NAN, 0.), (1., 1.), sub, true));
}

#[test]
fn the_part_a_submenu_surface_covers_is_cleared_and_nothing_else() {
    // a 10 x 6 surface at (100, 50); the submenu's surface from (106, 52), running past the surface's right edge
    let mut px = vec![1u32; 60];
    clear_covered(&mut px, (100, 50, 10, 6), (106, 52, 50, 3));
    for y in 0..6 {
        for x in 0..10 {
            let want = if x >= 6 && (2..5).contains(&y) { 0 } else { 1 };
            assert_eq!(px[y * 10 + x], want, "({x}, {y})");
        }
    }
    // a cover outside the surface changes nothing, nor does a buffer of the wrong size
    let mut px = vec![1u32; 60];
    clear_covered(&mut px, (100, 50, 10, 6), (300, 52, 50, 3));
    clear_covered(&mut px[..30], (100, 50, 10, 6), (100, 50, 10, 6));
    assert!(px.iter().all(|&p| p == 1));
}

#[test]
fn the_flyout_wakes_the_shell_at_the_earliest_of_its_timers() {
    let mut f = Flyout::default();
    assert_eq!(f.due(), None);
    f.pending = Some((Menu::File, Menu::Recent, 100.));
    assert_eq!(f.due(), Some(100. + FLYOUT_OPEN_MS));
    f.deferred = Some((Menu::File, Some(2), 50.));
    assert_eq!(f.due(), Some((50. + FLYOUT_SAFE_MS).min(100. + FLYOUT_OPEN_MS)));
}
