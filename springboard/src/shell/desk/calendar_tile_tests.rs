use super::calendar_face_text;

#[test]
fn the_calendar_tile_shows_the_day_it_is_not_a_fixed_picture() {
    // Monday 12 October 2026: the tile reads "Mon" over "12", never the baked "Tue 1"
    assert_eq!(calendar_face_text(1, 12), ("Mon", "12".to_string()));
    assert_eq!(calendar_face_text(2, 1), ("Tue", "1".to_string()));
    // weekday 0 is Sunday; a negative index wraps to Saturday rather than panicking
    assert_eq!(calendar_face_text(0, 4), ("Sun", "4".to_string()));
    assert_eq!(calendar_face_text(-1, 31), ("Sat", "31".to_string()));
}
