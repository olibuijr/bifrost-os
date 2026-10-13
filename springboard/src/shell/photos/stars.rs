//! Star ratings: the rating a photo keeps (0 to 5 stars), what a tap on a star row means, and the shape of a
//! star as a signed distance for the glyph painter.

/// The most stars a photo can have.
pub const MAX_STARS: u8 = 5;

/// The rating after a tap on star `tapped` (1..=5) of a photo rated `current`: tapping the star it already
/// has clears the rating, any other star sets it. Anything outside 1..=5 leaves the rating.
pub fn after_tap(current: u8, tapped: u8) -> u8 {
    if !(1..=MAX_STARS).contains(&tapped) {
        current.min(MAX_STARS)
    } else if current == tapped {
        0
    } else {
        tapped
    }
}

/// Which star (1..=5) a tap at `x` lands on in a row of five equal slots from `x0` to `x1`.
pub fn star_at(x: f32, x0: f32, x1: f32) -> Option<u8> {
    if x1 <= x0 || x < x0 || x >= x1 {
        return None;
    }
    Some((((x - x0) / (x1 - x0) * MAX_STARS as f32) as u8 + 1).min(MAX_STARS))
}

/// The stored value as a rating: a number 0..=5, anything else is no rating.
pub fn parse(value: &str) -> u8 {
    value.parse::<u8>().ok().filter(|&r| r <= MAX_STARS).unwrap_or(0)
}

/// Signed distance (negative inside) from `(px, py)` to a five-pointed star centred on the origin, with the
/// point up on a screen whose y axis points down. `r` is the outer radius and `inner` (0..1) shapes the
/// notches between the points.
pub fn star_distance(px: f32, py: f32, r: f32, inner: f32) -> f32 {
    // the usual star construction works in a y-up plane, so flip y
    let (mut x, mut y) = (px.abs(), -py);
    let k1 = (0.809_017_f32, -0.587_785_25_f32);
    let k2 = (-k1.0, k1.1);
    let dot = |a: (f32, f32), bx: f32, by: f32| a.0 * bx + a.1 * by;
    let d1 = 2.0 * dot(k1, x, y).max(0.0);
    x -= d1 * k1.0;
    y -= d1 * k1.1;
    let d2 = 2.0 * dot(k2, x, y).max(0.0);
    x -= d2 * k2.0;
    y -= d2 * k2.1;
    x = x.abs();
    y -= r;
    let ba = (inner * -k1.1, inner * k1.0 - 1.0);
    let h = (dot(ba, x, y) / dot(ba, ba.0, ba.1)).clamp(0.0, r);
    let (dx, dy) = (x - ba.0 * h, y - ba.1 * h);
    (dx * dx + dy * dy).sqrt() * (y * ba.0 - x * ba.1).signum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tap_sets_or_clears_the_rating() {
        assert_eq!(after_tap(0, 3), 3);
        assert_eq!(after_tap(3, 3), 0, "the same star clears");
        assert_eq!(after_tap(3, 5), 5);
        assert_eq!(after_tap(2, 0), 2);
        assert_eq!(after_tap(2, 6), 2);
        assert_eq!(after_tap(9, 0), 5, "a corrupt rating is capped");
    }

    #[test]
    fn taps_map_to_five_equal_slots() {
        assert_eq!(star_at(0.0, 0.0, 100.0), Some(1));
        assert_eq!(star_at(19.9, 0.0, 100.0), Some(1));
        assert_eq!(star_at(20.0, 0.0, 100.0), Some(2));
        assert_eq!(star_at(99.9, 0.0, 100.0), Some(5));
        assert_eq!(star_at(100.0, 0.0, 100.0), None);
        assert_eq!(star_at(-1.0, 0.0, 100.0), None);
        assert_eq!(star_at(5.0, 10.0, 10.0), None);
    }

    #[test]
    fn stored_values_parse_to_ratings() {
        assert_eq!((parse("4"), parse("0"), parse("6"), parse("x"), parse("")), (4, 0, 0, 0, 0));
    }

    #[test]
    fn the_star_is_inside_at_the_centre_and_outside_far_away() {
        assert!(star_distance(0.0, 0.0, 10.0, 0.5) < 0.0);
        assert!(star_distance(30.0, 0.0, 10.0, 0.5) > 0.0);
        assert!(star_distance(0.0, -9.0, 10.0, 0.5) < 0.0, "the top point is up");
        assert!(star_distance(0.0, 9.0, 10.0, 0.5) > 0.0, "nothing points straight down");
    }
}
