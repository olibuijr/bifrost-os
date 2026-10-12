//! Procedural noise for game worlds: a seeded value-noise lattice, layered into fractal octaves, with three
//! shapes (smooth, billowy and ridged) and two exact patterns (checkerboard and constant). All work is f64 math
//! with no state. With a value or constant basis and a smooth shape, every sample lies in [-1, 1].

/// What the lattice holds. `Value` is seeded random noise; the other two are exact patterns.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Basis {
    /// Smooth random values on an integer lattice, derived from `seed`.
    Value { seed: u32 },
    /// Alternating squares of side `square` in world units. Samples are 1.0 or -1.0.
    Checkerboard { square: f64 },
    /// The same value everywhere.
    Constant(f64),
}

/// How each octave value is shaped before the octaves are summed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// The octave value as it is.
    Smooth,
    /// `2|n| - 1`: soft, rounded features with a dark crease where the value crosses zero.
    Billow,
    /// `1 - |n|`: sharp ridges along the zero contours.
    Ridged,
}

/// A noise field: a basis, a shape, and the octave layering that combines them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Noise {
    pub basis: Basis,
    pub shape: Shape,
    /// Octaves summed. Values below 1 are treated as 1.
    pub octaves: u32,
    /// Lattice cells per world unit for the first octave.
    pub frequency: f64,
    /// Amplitude multiplier per octave, typically below 1.
    pub persistence: f64,
    /// Frequency multiplier per octave, typically 2.
    pub lacunarity: f64,
}

impl Noise {
    /// One smooth octave at frequency 1 with persistence 0.5 and lacunarity 2.
    pub fn new(basis: Basis) -> Self {
        Self {
            basis,
            shape: Shape::Smooth,
            octaves: 1,
            frequency: 1.0,
            persistence: 0.5,
            lacunarity: 2.0,
        }
    }

    /// The field at world point (`x`, `y`). Octave amplitudes are divided by their sum, so the result keeps the
    /// range of a single octave.
    pub fn sample(&self, x: f64, y: f64) -> f64 {
        let octaves = self.octaves.max(1);
        let mut freq = self.frequency;
        let mut amp = 1.0_f64;
        let mut sum = 0.0_f64;
        let mut norm = 0.0_f64;
        for _ in 0..octaves {
            sum += amp * self.shape_value(self.base(x * freq, y * freq));
            norm += amp;
            amp *= self.persistence;
            freq *= self.lacunarity;
        }
        sum / norm
    }

    fn base(&self, x: f64, y: f64) -> f64 {
        match self.basis {
            Basis::Value { seed } => value_noise(seed, x, y),
            Basis::Checkerboard { square } => {
                let side = if square > 0.0 { square } else { 1.0 };
                let cx = (x / side).floor() as i64;
                let cy = (y / side).floor() as i64;
                if (cx + cy).rem_euclid(2) == 0 {
                    1.0
                } else {
                    -1.0
                }
            }
            Basis::Constant(v) => v,
        }
    }

    fn shape_value(&self, n: f64) -> f64 {
        match self.shape {
            Shape::Smooth => n,
            Shape::Billow => 2.0 * n.abs() - 1.0,
            Shape::Ridged => 1.0 - n.abs(),
        }
    }
}

/// The lattice value at integer point (`x`, `y`) for `seed`, in [-1, 1].
pub fn lattice_value(seed: u32, x: i64, y: i64) -> f64 {
    let h = mix(seed, x, y);
    h as f64 / u32::MAX as f64 * 2.0 - 1.0
}

/// Integer hash of a lattice point. It mixes the seed and both coordinates into 32 bits, so neighbouring
/// points get unrelated values.
fn mix(seed: u32, x: i64, y: i64) -> u32 {
    let mut h = seed ^ (x as u32).wrapping_mul(0x27d4_eb2d) ^ (y as u32).wrapping_mul(0x1656_67b1);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297a_2d39);
    h ^= h >> 15;
    h
}

/// Smoothstep: 0 at 0, 1 at 1, and 0.5 at 0.5.
fn fade(t: f64) -> f64 {
    t * t * (3.0 - 2.0 * t)
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

/// Bilinear blend of the four surrounding lattice values, weighted with `fade`.
fn value_noise(seed: u32, x: f64, y: f64) -> f64 {
    let x0 = x.floor();
    let y0 = y.floor();
    let (ix, iy) = (x0 as i64, y0 as i64);
    let sx = fade(x - x0);
    let sy = fade(y - y0);
    let top = lerp(lattice_value(seed, ix, iy), lattice_value(seed, ix + 1, iy), sx);
    let bottom = lerp(lattice_value(seed, ix, iy + 1), lattice_value(seed, ix + 1, iy + 1), sx);
    lerp(top, bottom, sy)
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f64 = 1e-12;

    #[test]
    fn constant_basis_with_smooth_shape_keeps_its_value() {
        let n = Noise {
            basis: Basis::Constant(0.5),
            shape: Shape::Smooth,
            octaves: 4,
            frequency: 1.0,
            persistence: 0.5,
            lacunarity: 2.0,
        };
        assert!((n.sample(3.7, -2.2) - 0.5).abs() < EPS);
        assert!((Noise::new(Basis::Constant(0.5)).sample(0.0, 0.0) - 0.5).abs() < EPS);
    }

    #[test]
    fn ridged_and_billow_shapes_of_a_constant() {
        // Ridged: 1 - |0.25| = 0.75 on every octave, so the weighted mean is 0.75.
        let ridged = Noise {
            shape: Shape::Ridged,
            octaves: 3,
            ..Noise::new(Basis::Constant(0.25))
        };
        assert!((ridged.sample(1.0, 2.0) - 0.75).abs() < EPS);
        // Billow: 2 * 0.25 - 1 = -0.5 on every octave.
        let billow = Noise {
            shape: Shape::Billow,
            octaves: 3,
            ..Noise::new(Basis::Constant(0.25))
        };
        assert!((billow.sample(1.0, 2.0) + 0.5).abs() < EPS);
    }

    #[test]
    fn checkerboard_alternates_by_square_parity() {
        let n = Noise::new(Basis::Checkerboard { square: 1.0 });
        assert_eq!(n.sample(0.5, 0.5), 1.0); // cell (0, 0): even
        assert_eq!(n.sample(1.5, 0.5), -1.0); // cell (1, 0): odd
        assert_eq!(n.sample(1.5, 1.5), 1.0); // cell (1, 1): even
        assert_eq!(n.sample(-0.5, 0.5), -1.0); // cell (-1, 0): odd, negative cells included
    }

    #[test]
    fn value_noise_is_the_lattice_value_at_integer_points() {
        let n = Noise::new(Basis::Value { seed: 7 });
        // At an integer point the fade weight is 0, so the sample is the lattice value exactly.
        assert!((n.sample(2.0, 3.0) - lattice_value(7, 2, 3)).abs() < EPS);
    }

    #[test]
    fn value_noise_halfway_between_two_lattice_points_is_their_mean() {
        let n = Noise::new(Basis::Value { seed: 7 });
        // fade(0.5) = 0.5, and the y weight is 0 on the lattice row y = 0.
        let mean = (lattice_value(7, 0, 0) + lattice_value(7, 1, 0)) / 2.0;
        assert!((n.sample(0.5, 0.0) - mean).abs() < EPS);
    }

    #[test]
    fn fractal_value_noise_stays_in_range() {
        let n = Noise {
            octaves: 3,
            ..Noise::new(Basis::Value { seed: 99 })
        };
        for i in 0..200 {
            for j in 0..20 {
                let v = n.sample(i as f64 * 0.37 - 20.0, j as f64 * 0.41 - 4.0);
                assert!(v >= -1.0 - EPS && v <= 1.0 + EPS, "sample {v} out of range");
            }
        }
    }
}
