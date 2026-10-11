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
}
