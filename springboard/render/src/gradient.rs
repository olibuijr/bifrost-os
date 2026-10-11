//! Gradient colour model: colour stops, interpolation, and the colour of a
//! linear or radial gradient at a point or over a pixel grid. Colours are the
//! packed words that `gfx::pack` and `gfx::rgba` produce. Channels interpolate
//! as stored, which is correct for premultiplied pixels and avoids dark edges.
//! Outside the stops the end colours extend.

/// One colour stop: a colour at a relative location in 0..=1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stop {
    pub location: f32,
    pub color: u32,
}

/// Colour stops sorted by location.
#[derive(Clone, Debug, PartialEq)]
pub struct Gradient {
    stops: Vec<Stop>,
}

impl Gradient {
    /// Builds a gradient from `(location, color)` pairs. Locations are clamped
    /// to 0..=1 and sorted. Returns `None` when there are no stops.
    pub fn new(stops: &[(f32, u32)]) -> Option<Gradient> {
        if stops.is_empty() {
            return None;
        }
        let mut v: Vec<Stop> = stops
            .iter()
            .map(|&(location, color)| Stop { location: location.clamp(0.0, 1.0), color })
            .collect();
        v.sort_by(|a, b| a.location.total_cmp(&b.location));
        Some(Gradient { stops: v })
    }

    /// Gradient running from `start` at location 0 to `end` at location 1.
    pub fn two_color(start: u32, end: u32) -> Gradient {
        Gradient {
            stops: vec![
                Stop { location: 0.0, color: start },
                Stop { location: 1.0, color: end },
            ],
        }
    }

    pub fn stops(&self) -> &[Stop] {
        &self.stops
    }

    /// Colour at relative location `t`. Locations beyond the first or last stop take that stop's colour.
    pub fn color_at(&self, t: f32) -> u32 {
        let first = self.stops[0];
        let last = self.stops[self.stops.len() - 1];
        if !(t > first.location) {
            return first.color;
        }
        if t >= last.location {
            return last.color;
        }
        for pair in self.stops.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            if t <= b.location {
                let span = b.location - a.location;
                let f = if span > 0.0 { (t - a.location) / span } else { 1.0 };
                return mix(a.color, b.color, f);
            }
        }
        last.color
    }

    /// Colour at `(x, y)` for a linear gradient from `start` (location 0) to `end` (location 1).
    /// A zero-length axis yields the first stop everywhere.
    pub fn linear_at(&self, start: (f32, f32), end: (f32, f32), x: f32, y: f32) -> u32 {
        let (dx, dy) = (end.0 - start.0, end.1 - start.1);
        let len2 = dx * dx + dy * dy;
        let t = if len2 > 0.0 {
            ((x - start.0) * dx + (y - start.1) * dy) / len2
        } else {
            0.0
        };
        self.color_at(t)
    }

    /// Colour at `(x, y)` for a radial gradient: location 0 at `center`, 1 at `radius`.
    /// A zero radius yields the last stop everywhere.
    pub fn radial_at(&self, center: (f32, f32), radius: f32, x: f32, y: f32) -> u32 {
        let t = if radius > 0.0 {
            ((x - center.0).powi(2) + (y - center.1).powi(2)).sqrt() / radius
        } else {
            1.0
        };
        self.color_at(t)
    }

    /// Linear gradient over a `w` by `h` grid, sampled at pixel centres, row-major.
    pub fn linear_pixels(&self, w: usize, h: usize, start: (f32, f32), end: (f32, f32)) -> Vec<u32> {
        self.grid(w, h, |x, y| self.linear_at(start, end, x, y))
    }

    /// Radial gradient over a `w` by `h` grid, sampled at pixel centres, row-major.
    pub fn radial_pixels(&self, w: usize, h: usize, center: (f32, f32), radius: f32) -> Vec<u32> {
        self.grid(w, h, |x, y| self.radial_at(center, radius, x, y))
    }

    fn grid(&self, w: usize, h: usize, f: impl Fn(f32, f32) -> u32) -> Vec<u32> {
        let mut out = Vec::with_capacity(w * h);
        for y in 0..h {
            for x in 0..w {
                out.push(f(x as f32 + 0.5, y as f32 + 0.5));
            }
        }
        out
    }
}

/// Channel-wise linear mix of two packed colours; `f` is clamped to 0..=1.
fn mix(a: u32, b: u32, f: f32) -> u32 {
    let f = f.clamp(0.0, 1.0);
    let mut out = 0u32;
    for shift in [0u32, 8, 16, 24] {
        let x = ((a >> shift) & 0xff) as f32;
        let y = ((b >> shift) & 0xff) as f32;
        let v = (x + (y - x) * f).round().clamp(0.0, 255.0) as u32;
        out |= v << shift;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gfx::pack;

    const BLACK: u32 = pack(0, 0, 0, 255);
    const WHITE: u32 = pack(255, 255, 255, 255);
    const RED: u32 = pack(255, 0, 0, 255);
    const GREEN: u32 = pack(0, 255, 0, 255);

    fn channel(c: u32, shift: u32) -> u32 {
        (c >> shift) & 0xff
    }

    #[test]
    fn two_color_interpolates_between_its_ends() {
        let g = Gradient::two_color(BLACK, WHITE);
        assert_eq!(g.color_at(0.0), BLACK);
        assert_eq!(g.color_at(1.0), WHITE);
        let mid = channel(g.color_at(0.5), 0);
        assert!(mid == 127 || mid == 128, "mid {mid}");
    }

    #[test]
    fn ends_extend_beyond_the_stops() {
        let g = Gradient::two_color(BLACK, WHITE);
        assert_eq!(g.color_at(-2.0), BLACK);
        assert_eq!(g.color_at(7.0), WHITE);
    }

    #[test]
    fn multi_stop_gradient_passes_through_its_middle_stop() {
        let g = Gradient::new(&[(0.0, RED), (0.5, GREEN), (1.0, BLACK)]).unwrap();
        assert_eq!(g.color_at(0.5), GREEN);
        let q = g.color_at(0.25);
        let r = channel(q, 0);
        let gr = channel(q, 8);
        assert!(r == 127 || r == 128, "red {r}");
        assert!(gr == 127 || gr == 128, "green {gr}");
    }

    #[test]
    fn new_sorts_clamps_locations_and_rejects_empty_input() {
        assert!(Gradient::new(&[]).is_none());
        let g = Gradient::new(&[(1.0, WHITE), (0.0, BLACK)]).unwrap();
        assert_eq!(g.stops()[0].location, 0.0);
        assert_eq!(g.color_at(0.0), BLACK);
        let clamped = Gradient::new(&[(5.0, WHITE)]).unwrap();
        assert_eq!(clamped.stops()[0].location, 1.0);
    }

    #[test]
    fn linear_at_follows_the_axis_and_ignores_the_perpendicular() {
        let g = Gradient::two_color(BLACK, WHITE);
        let start = (0.0, 0.0);
        let end = (10.0, 0.0);
        assert_eq!(g.linear_at(start, end, 0.0, 3.0), BLACK);
        assert_eq!(g.linear_at(start, end, 10.0, 3.0), WHITE);
        assert_eq!(g.linear_at(start, end, 5.0, 0.0), g.linear_at(start, end, 5.0, 99.0));
        assert_eq!(g.linear_at((1.0, 1.0), (1.0, 1.0), 3.0, 3.0), BLACK);
    }

    #[test]
    fn radial_at_extends_beyond_the_radius() {
        let g = Gradient::two_color(BLACK, WHITE);
        assert_eq!(g.radial_at((0.0, 0.0), 4.0, 0.0, 0.0), BLACK);
        // Distance 5 from the centre with radius 4 is past the last stop.
        assert_eq!(g.radial_at((0.0, 0.0), 4.0, 3.0, 4.0), WHITE);
        let half = channel(g.radial_at((0.0, 0.0), 4.0, 0.0, 2.0), 0);
        assert!(half == 127 || half == 128, "half {half}");
    }

    #[test]
    fn pixel_grid_is_row_major_and_sampled_at_pixel_centres() {
        let g = Gradient::two_color(BLACK, WHITE);
        let px = g.linear_pixels(4, 2, (0.0, 0.0), (4.0, 0.0));
        assert_eq!(px.len(), 8);
        let row = &px[..4];
        assert!(row.windows(2).all(|w| channel(w[0], 0) < channel(w[1], 0)));
        assert_eq!(&px[..4], &px[4..]);
    }
}
