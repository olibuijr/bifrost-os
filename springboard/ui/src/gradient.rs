//! Gradients: colour stops along a ramp, and the ramp position of a point for a linear or radial gradient (the gradient
//! type of the reference SDK, without the context that fills with it). The caller samples `color_at` per pixel.
//! Colours are straight (non-premultiplied) 8-bit RGBA and interpolate per channel.

/// One colour on the ramp.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stop {
    /// Position along the ramp: 0.0 at the start, 1.0 at the end.
    pub at: f32,
    pub rgba: [u8; 4],
}

#[derive(Clone, Debug, PartialEq)]
pub struct Gradient {
    stops: Vec<Stop>,
}

impl Gradient {
    /// Sorts the stops by position and clamps each one to 0..=1. Non-finite positions become 0.
    pub fn new(mut stops: Vec<Stop>) -> Self {
        for s in &mut stops {
            s.at = if s.at.is_finite() { s.at.clamp(0.0, 1.0) } else { 0.0 };
        }
        stops.sort_by(|a, b| a.at.partial_cmp(&b.at).unwrap_or(std::cmp::Ordering::Equal));
        Gradient { stops }
    }

    pub fn stops(&self) -> &[Stop] {
        &self.stops
    }

    /// The colour at ramp position `t`, clamped to 0..=1. Between two stops the channels interpolate linearly;
    /// outside the first and last stops the end colours hold. A gradient with no stops is transparent black.
    pub fn color_at(&self, t: f32) -> [u8; 4] {
        let (first, last) = match (self.stops.first(), self.stops.last()) {
            (Some(f), Some(l)) => (*f, *l),
            _ => return [0, 0, 0, 0],
        };
        let t = if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.0 };
        if t <= first.at {
            return first.rgba;
        }
        if t >= last.at {
            return last.rgba;
        }
        for pair in self.stops.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            if t <= b.at {
                let span = b.at - a.at;
                let f = if span > 0.0 { (t - a.at) / span } else { 1.0 };
                return mix(a.rgba, b.rgba, f);
            }
        }
        last.rgba
    }

    /// The ramp position of `(x, y)` projected onto the line from `start` to `end`, clamped to 0..=1.
    /// A zero-length line gives 0.
    pub fn linear_t(start: (f32, f32), end: (f32, f32), x: f32, y: f32) -> f32 {
        let (dx, dy) = (end.0 - start.0, end.1 - start.1);
        let len2 = dx * dx + dy * dy;
        if len2 <= 0.0 {
            return 0.0;
        }
        (((x - start.0) * dx + (y - start.1) * dy) / len2).clamp(0.0, 1.0)
    }

    /// The ramp position of `(x, y)` as its distance from `center` over `radius`, clamped to 0..=1.
    /// A zero or negative radius gives 1, so every point lies outside the ramp's circle.
    pub fn radial_t(center: (f32, f32), radius: f32, x: f32, y: f32) -> f32 {
        if radius <= 0.0 {
            return 1.0;
        }
        let d = ((x - center.0).powi(2) + (y - center.1).powi(2)).sqrt();
        (d / radius).clamp(0.0, 1.0)
    }

    /// The ramp position of `(x, y)` as its clockwise angle from `start_angle` around `center`, as a fraction of a full
    /// turn in 0..1. `start_angle` is in radians, measured from the positive x axis towards positive y. A point at the
    /// centre has no direction of its own and is taken as angle 0 before `start_angle` is applied.
    pub fn conic_t(center: (f32, f32), start_angle: f32, x: f32, y: f32) -> f32 {
        let angle = (y - center.1).atan2(x - center.0) - start_angle;
        angle.rem_euclid(std::f32::consts::TAU) / std::f32::consts::TAU
    }

    /// The ramp position of `(x, y)` for a layer of `kind`. The rule matches the reference SDK's layer, with the radius
    /// and the start angle taken from the end point: axial projects onto the line from `start` to `end`; radial uses
    /// `start` as the centre and the distance to `end` as the radius (a circle, not an ellipse); conic starts at the
    /// direction of `end` from `start`.
    pub fn layer_t(kind: GradientType, start: (f32, f32), end: (f32, f32), x: f32, y: f32) -> f32 {
        match kind {
            GradientType::Axial => Gradient::linear_t(start, end, x, y),
            GradientType::Radial => {
                let radius = ((end.0 - start.0).powi(2) + (end.1 - start.1).powi(2)).sqrt();
                Gradient::radial_t(start, radius, x, y)
            }
            GradientType::Conic => {
                let start_angle = (end.1 - start.1).atan2(end.0 - start.0);
                Gradient::conic_t(start, start_angle, x, y)
            }
        }
    }
}

/// The style of gradient a layer draws (the reference SDK's gradient layer type).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GradientType {
    /// Varies along the axis between the start and end points.
    Axial,
    /// Varies with the distance from the start point, out to the end point.
    Radial,
    /// Varies with the angle around the start point, starting in the direction of the end point.
    Conic,
}

fn mix(a: [u8; 4], b: [u8; 4], f: f32) -> [u8; 4] {
    let mut out = [0u8; 4];
    for i in 0..4 {
        let v = a[i] as f32 + (b[i] as f32 - a[i] as f32) * f;
        out[i] = v.round().clamp(0.0, 255.0) as u8;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const BLACK: [u8; 4] = [0, 0, 0, 255];
    const WHITE: [u8; 4] = [255, 255, 255, 255];

    #[test]
    fn new_sorts_stops_and_clamps_their_positions() {
        let g = Gradient::new(vec![
            Stop { at: 1.5, rgba: WHITE },
            Stop { at: -0.2, rgba: BLACK },
            Stop { at: f32::NAN, rgba: BLACK },
        ]);
        let at: Vec<f32> = g.stops().iter().map(|s| s.at).collect();
        assert_eq!(at, vec![0.0, 0.0, 1.0]);
    }

    #[test]
    fn two_stop_midpoint_is_the_average_of_the_ends() {
        let g = Gradient::new(vec![Stop { at: 0.0, rgba: BLACK }, Stop { at: 1.0, rgba: WHITE }]);
        assert_eq!(g.color_at(0.5), [128, 128, 128, 255]);
        assert_eq!(g.color_at(0.0), BLACK);
        assert_eq!(g.color_at(1.0), WHITE);
    }

    #[test]
    fn positions_outside_the_ramp_hold_the_end_colours() {
        let g = Gradient::new(vec![Stop { at: 0.2, rgba: BLACK }, Stop { at: 0.8, rgba: WHITE }]);
        assert_eq!(g.color_at(-1.0), BLACK);
        assert_eq!(g.color_at(0.1), BLACK);
        assert_eq!(g.color_at(2.0), WHITE);
        assert_eq!(g.color_at(f32::NAN), BLACK);
    }

    #[test]
    fn three_stops_interpolate_within_the_segment() {
        let red = [255, 0, 0, 255];
        let green = [0, 255, 0, 255];
        let blue = [0, 0, 255, 255];
        let g = Gradient::new(vec![
            Stop { at: 0.0, rgba: red },
            Stop { at: 0.5, rgba: green },
            Stop { at: 1.0, rgba: blue },
        ]);
        assert_eq!(g.color_at(0.25), [128, 128, 0, 255]);
        assert_eq!(g.color_at(0.5), green);
        assert_eq!(g.color_at(0.75), [0, 128, 128, 255]);
    }

    #[test]
    fn alpha_interpolates_with_the_colour() {
        let g = Gradient::new(vec![Stop { at: 0.0, rgba: [0, 0, 0, 0] }, Stop { at: 1.0, rgba: [0, 0, 0, 255] }]);
        assert_eq!(g.color_at(0.5)[3], 128);
    }

    #[test]
    fn empty_gradient_is_transparent() {
        let g = Gradient::new(Vec::new());
        assert_eq!(g.color_at(0.5), [0, 0, 0, 0]);
    }

    #[test]
    fn linear_position_projects_onto_the_axis() {
        let (a, b) = ((0.0, 0.0), (10.0, 0.0));
        assert_eq!(Gradient::linear_t(a, b, 5.0, 0.0), 0.5);
        assert_eq!(Gradient::linear_t(a, b, 5.0, 100.0), 0.5, "distance off the axis does not matter");
        assert_eq!(Gradient::linear_t(a, b, -5.0, 0.0), 0.0);
        assert_eq!(Gradient::linear_t(a, b, 50.0, 0.0), 1.0);
        assert_eq!(Gradient::linear_t(a, a, 3.0, 3.0), 0.0, "a zero-length line gives the start");
    }

    #[test]
    fn radial_position_is_the_distance_over_the_radius() {
        let c = (0.0, 0.0);
        assert_eq!(Gradient::radial_t(c, 10.0, 3.0, 4.0), 0.5);
        assert_eq!(Gradient::radial_t(c, 10.0, 30.0, 40.0), 1.0);
        assert_eq!(Gradient::radial_t(c, 10.0, 0.0, 0.0), 0.0);
        assert_eq!(Gradient::radial_t(c, 0.0, 0.0, 0.0), 1.0, "a zero radius is outside the ramp");
    }

    #[test]
    fn conic_position_is_the_clockwise_fraction_of_a_turn() {
        // Straight down (positive y) from the centre is a quarter turn from the positive x axis.
        let t = Gradient::conic_t((0.0, 0.0), 0.0, 0.0, 1.0);
        assert!((t - 0.25).abs() < 1e-6, "got {t}");
        // Up and to the right is -45 degrees, which is 7/8 of a turn clockwise.
        let t = Gradient::conic_t((0.0, 0.0), 0.0, 1.0, -1.0);
        assert!((t - 0.875).abs() < 1e-6, "got {t}");
    }

    #[test]
    fn conic_start_angle_moves_the_seam() {
        // With the seam at straight down (pi/2), the point on the positive x axis is three quarters of a turn on.
        let t = Gradient::conic_t((0.0, 0.0), std::f32::consts::FRAC_PI_2, 1.0, 0.0);
        assert!((t - 0.75).abs() < 1e-6, "got {t}");
    }

    #[test]
    fn conic_centre_is_position_zero_with_no_start_offset() {
        assert_eq!(Gradient::conic_t((4.0, 4.0), 0.0, 4.0, 4.0), 0.0);
    }

    #[test]
    fn layer_axial_matches_the_linear_position() {
        let (a, b) = ((0.0, 0.0), (10.0, 0.0));
        assert_eq!(Gradient::layer_t(GradientType::Axial, a, b, 5.0, 3.0), 0.5);
        assert_eq!(Gradient::layer_t(GradientType::Axial, a, b, 5.0, 3.0), Gradient::linear_t(a, b, 5.0, 3.0));
    }

    #[test]
    fn layer_radial_takes_its_radius_from_the_end_point() {
        // The end point (3, 4) is 5 away from the start, so the ramp runs out to a radius of 5.
        let (start, end) = ((0.0, 0.0), (3.0, 4.0));
        assert_eq!(Gradient::layer_t(GradientType::Radial, start, end, 0.0, 0.0), 0.0);
        assert_eq!(Gradient::layer_t(GradientType::Radial, start, end, 3.0, 4.0), 1.0);
        assert_eq!(Gradient::layer_t(GradientType::Radial, start, end, 1.5, 2.0), 0.5);
    }
}
