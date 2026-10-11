//! A three-dimensional colour lookup table. Each input colour selects a point in a cube of output
//! colours, and the points between the cube's samples are trilinearly interpolated. The caller
//! builds the table (a grading preset, a channel curve baked into a cube, or `identity` as a start);
//! the lookup is pure and allocation-free.

use crate::colorfx::Rgb;

/// Largest supported side: 64 samples per axis, 262,144 entries.
pub const MAX_SIZE: usize = 64;

/// A colour cube with `size` samples per axis. Entry `(r, g, b)` is stored at
/// `r + g * size + b * size * size`, so red varies fastest.
#[derive(Clone, Debug, PartialEq)]
pub struct ColorCube {
    size: usize,
    data: Vec<Rgb>,
}

impl ColorCube {
    /// A cube from `size^3` entries in red-fastest order. `None` when `size` is outside 2..=MAX_SIZE
    /// or the entry count does not match.
    pub fn new(size: usize, data: Vec<Rgb>) -> Option<Self> {
        if !(2..=MAX_SIZE).contains(&size) || data.len() != size * size * size {
            return None;
        }
        Some(ColorCube { size, data })
    }

    /// The cube that maps every colour to itself. `None` when `size` is out of range.
    pub fn identity(size: usize) -> Option<Self> {
        if !(2..=MAX_SIZE).contains(&size) {
            return None;
        }
        let n = (size - 1) as f32;
        let mut data = Vec::with_capacity(size * size * size);
        for b in 0..size {
            for g in 0..size {
                for r in 0..size {
                    data.push(Rgb::new(r as f32 / n, g as f32 / n, b as f32 / n));
                }
            }
        }
        ColorCube::new(size, data)
    }

    /// Samples per axis.
    pub fn size(&self) -> usize {
        self.size
    }

    fn at(&self, r: usize, g: usize, b: usize) -> Rgb {
        self.data[r + g * self.size + b * self.size * self.size]
    }

    /// The table's colour for `c`. Each channel is clamped to 0..=1 and the result is interpolated
    /// between the eight nearest samples.
    pub fn lookup(&self, c: Rgb) -> Rgb {
        self.sample(c, true)
    }

    /// Like `lookup`, but channels outside 0..=1 are not clamped. The cube is extended linearly from
    /// its edge samples, as the `extrapolate` option of a colour cube filter does.
    pub fn lookup_extrapolated(&self, c: Rgb) -> Rgb {
        self.sample(c, false)
    }

    fn sample(&self, c: Rgb, clamp: bool) -> Rgb {
        let top = (self.size - 1) as f32;
        // (index of the lower sample, fraction towards the next one)
        let axis = |v: f32| {
            let v = if clamp { v.clamp(0.0, 1.0) } else { v };
            let f = v * top;
            let i = (f.floor().max(0.0) as usize).min(self.size - 2);
            (i, f - i as f32)
        };
        let (r0, tr) = axis(c.r);
        let (g0, tg) = axis(c.g);
        let (b0, tb) = axis(c.b);
        let c00 = self.at(r0, g0, b0).mix(self.at(r0 + 1, g0, b0), tr);
        let c10 = self.at(r0, g0 + 1, b0).mix(self.at(r0 + 1, g0 + 1, b0), tr);
        let c01 = self.at(r0, g0, b0 + 1).mix(self.at(r0 + 1, g0, b0 + 1), tr);
        let c11 = self.at(r0, g0 + 1, b0 + 1).mix(self.at(r0 + 1, g0 + 1, b0 + 1), tr);
        c00.mix(c10, tg).mix(c01.mix(c11, tg), tb)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn identity_cube_returns_its_input() {
        let cube = ColorCube::identity(9).unwrap();
        for c in [Rgb::new(0.0, 0.0, 0.0), Rgb::new(1.0, 1.0, 1.0), Rgb::new(0.13, 0.57, 0.91), Rgb::new(0.5, 0.25, 0.75)] {
            let out = cube.lookup(c);
            assert!(close(out.r, c.r) && close(out.g, c.g) && close(out.b, c.b), "{c:?} -> {out:?}");
        }
    }

    #[test]
    fn inverted_cube_inverts_colours() {
        let identity = ColorCube::identity(5).unwrap();
        let data: Vec<Rgb> = identity.data.iter().map(|c| Rgb::new(1.0 - c.r, 1.0 - c.g, 1.0 - c.b)).collect();
        let cube = ColorCube::new(5, data).unwrap();
        let out = cube.lookup(Rgb::new(0.2, 0.0, 1.0));
        assert!(close(out.r, 0.8) && close(out.g, 1.0) && close(out.b, 0.0));
    }

    #[test]
    fn lookup_interpolates_between_samples() {
        // A 2-sample cube whose red axis runs black to white and whose other axes are flat:
        // the midpoint of red must give half intensity.
        let data = vec![
            Rgb::new(0.0, 0.0, 0.0),
            Rgb::new(1.0, 0.0, 0.0),
            Rgb::new(0.0, 1.0, 0.0),
            Rgb::new(1.0, 1.0, 0.0),
            Rgb::new(0.0, 0.0, 1.0),
            Rgb::new(1.0, 0.0, 1.0),
            Rgb::new(0.0, 1.0, 1.0),
            Rgb::new(1.0, 1.0, 1.0),
        ];
        let cube = ColorCube::new(2, data).unwrap();
        let mid = cube.lookup(Rgb::new(0.5, 0.0, 0.0));
        assert!(close(mid.r, 0.5) && close(mid.g, 0.0) && close(mid.b, 0.0));
        let corner = cube.lookup(Rgb::new(1.0, 1.0, 1.0));
        assert!(close(corner.r, 1.0) && close(corner.g, 1.0) && close(corner.b, 1.0));
    }

    #[test]
    fn lookup_clamps_out_of_range_inputs() {
        let cube = ColorCube::identity(4).unwrap();
        let out = cube.lookup(Rgb::new(2.0, -1.0, 0.5));
        assert!(close(out.r, 1.0) && close(out.g, 0.0) && close(out.b, 0.5));
    }

    #[test]
    fn new_rejects_bad_sizes_and_counts() {
        assert!(ColorCube::new(1, vec![Rgb::new(0.0, 0.0, 0.0)]).is_none());
        assert!(ColorCube::new(2, vec![Rgb::new(0.0, 0.0, 0.0); 7]).is_none());
        assert!(ColorCube::new(2, vec![Rgb::new(0.0, 0.0, 0.0); 8]).is_some());
        assert!(ColorCube::identity(MAX_SIZE + 1).is_none());
        assert!(ColorCube::identity(MAX_SIZE).is_some());
    }

    #[test]
    fn extrapolated_lookup_continues_past_the_edges() {
        // Red runs 0 to 1 along the cube, so red 1.5 extrapolates to 1.5 and red -0.5 to -0.5.
        let identity = ColorCube::identity(3).unwrap();
        let hi = identity.lookup_extrapolated(Rgb::new(1.5, 0.5, 0.25));
        assert!(close(hi.r, 1.5) && close(hi.g, 0.5) && close(hi.b, 0.25), "{hi:?}");
        let lo = identity.lookup_extrapolated(Rgb::new(-0.5, 0.5, 0.25));
        assert!(close(lo.r, -0.5), "{lo:?}");
        // The clamped lookup of the same colour stays inside the cube.
        assert!(close(identity.lookup(Rgb::new(1.5, 0.5, 0.25)).r, 1.0));
    }

    #[test]
    fn size_reports_samples_per_axis() {
        assert_eq!(ColorCube::identity(6).unwrap().size(), 6);
    }
}
