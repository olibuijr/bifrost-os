//! Procedural noise: sources that return a value in -1..=1 for every point of the plane, combinations of them into a
//! fractal, and a grid sampler that turns a source into a map of values.
//!
//! The sources are deterministic: the same seed and point always give the same value, so a map can be rebuilt
//! later without storing it. The gradient (Perlin) source is zero on the integer lattice, which the tests rely on.
//! A fractal sums octaves of a base source; each octave is at `lacunarity` times the frequency and `persistence`
//! times the weight of the one before. `Billow` and `Ridged` change how each octave is folded before the sum, and
//! every output stays in -1..=1. Voronoi (cellular) noise returns the distance to the nearest feature point, cut at
//! 1 and mapped to -1..=1. Nothing here draws: the caller turns the values into colours or heights.

use std::f64::consts::SQRT_2;

/// Parameters shared by the fractal sources.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fractal {
    pub seed: u64,
    /// Cycles per unit distance for the first octave.
    pub frequency: f64,
    /// Number of octaves summed. Zero gives a value of 0.
    pub octaves: u32,
    /// Frequency multiplier from one octave to the next, usually 2.
    pub lacunarity: f64,
    /// Weight multiplier from one octave to the next, usually 0.5.
    pub persistence: f64,
}

/// A procedural noise generator.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NoiseSource {
    /// The same value everywhere.
    Constant(f64),
    /// Alternating +1 and -1 squares of the given side length. A side of zero or less gives 0.
    Checkerboard { size: f64 },
    /// Single-octave gradient noise, zero on the integer lattice.
    Perlin { seed: u64, frequency: f64 },
    /// Fractal sum of gradient noise.
    Fractal(Fractal),
    /// Fractal sum of folded gradient noise, `2|n| - 1` per octave, soft bumps.
    Billow(Fractal),
    /// Fractal sum of `1 - 2|n|` per octave, sharp ridges.
    Ridged(Fractal),
    /// Distance from each point to the nearest random feature point, one per unit cell times `frequency`.
    Voronoi { seed: u64, frequency: f64 },
}

impl NoiseSource {
    /// The noise value at a point, always in -1..=1.
    pub fn value(&self, x: f64, y: f64) -> f64 {
        match *self {
            NoiseSource::Constant(c) => clamp_unit(c),
            NoiseSource::Checkerboard { size } => {
                if !(size > 0.0) {
                    return 0.0;
                }
                let cells = (x / size).floor() as i64 + (y / size).floor() as i64;
                if cells.rem_euclid(2) == 0 {
                    1.0
                } else {
                    -1.0
                }
            }
            NoiseSource::Perlin { seed, frequency } => perlin(seed, x * frequency, y * frequency),
            NoiseSource::Fractal(f) => fractal(&f, x, y, |n| n),
            NoiseSource::Billow(f) => fractal(&f, x, y, |n| 2.0 * n.abs() - 1.0),
            NoiseSource::Ridged(f) => fractal(&f, x, y, |n| 1.0 - 2.0 * n.abs()),
            NoiseSource::Voronoi { seed, frequency } => voronoi(seed, x * frequency, y * frequency),
        }
    }
}

/// A grid of noise values sampled from one source. Row `y` holds the `width` values for that row, row-major.
#[derive(Clone, Debug, PartialEq)]
pub struct NoiseMap {
    width: usize,
    height: usize,
    values: Vec<f64>,
}

impl NoiseMap {
    /// Samples `width` by `height` points starting at `origin`, `sample_distance` apart along both axes.
    /// A zero size gives an empty map.
    pub fn sample(source: &NoiseSource, width: usize, height: usize, origin: (f64, f64), sample_distance: f64) -> Self {
        let mut values = Vec::with_capacity(width * height);
        for row in 0..height {
            let y = origin.1 + row as f64 * sample_distance;
            for col in 0..width {
                let x = origin.0 + col as f64 * sample_distance;
                values.push(source.value(x, y));
            }
        }
        NoiseMap { width, height, values }
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    /// The value at a grid position, or None when it is outside the map.
    pub fn get(&self, col: usize, row: usize) -> Option<f64> {
        if col < self.width && row < self.height {
            Some(self.values[row * self.width + col])
        } else {
            None
        }
    }

    /// Smallest and largest value in the map, or None for an empty map.
    pub fn range(&self) -> Option<(f64, f64)> {
        let first = *self.values.first()?;
        Some(self.values.iter().fold((first, first), |(lo, hi), &v| (lo.min(v), hi.max(v))))
    }
}

fn clamp_unit(v: f64) -> f64 {
    v.clamp(-1.0, 1.0)
}

/// Sums `octaves` of a base function. `fold` maps each raw gradient value in -1..=1 to the octave's contribution.
fn fractal(f: &Fractal, x: f64, y: f64, fold: impl Fn(f64) -> f64) -> f64 {
    let mut sum = 0.0;
    let mut total = 0.0;
    let mut amp = 1.0;
    let mut freq = f.frequency;
    for i in 0..f.octaves {
        let n = perlin(f.seed.wrapping_add(i as u64), x * freq, y * freq);
        sum += fold(n) * amp;
        total += amp;
        amp *= f.persistence;
        freq *= f.lacunarity;
    }
    if total > 0.0 {
        clamp_unit(sum / total)
    } else {
        0.0
    }
}

/// SplitMix-style hash of a seed and a lattice point. The same inputs always give the same output.
fn hash(seed: u64, x: i64, y: i64) -> u64 {
    let mut h = seed ^ (x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (y as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    h ^= h >> 33;
    h = h.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    h ^= h >> 33;
    h = h.wrapping_mul(0xC4CE_B9FE_1A85_EC53);
    h ^= h >> 33;
    h
}

/// A hash value mapped to [0, 1).
fn unit(h: u64) -> f64 {
    (h >> 11) as f64 / (1u64 << 53) as f64
}

/// 1 / sqrt(2), the components of a diagonal unit vector.
const DIAG: f64 = 0.707_106_781_186_547_6;

/// The eight unit directions, the four diagonals scaled to unit length.
const GRADIENTS: [(f64, f64); 8] = [
    (1.0, 0.0),
    (-1.0, 0.0),
    (0.0, 1.0),
    (0.0, -1.0),
    (DIAG, DIAG),
    (-DIAG, DIAG),
    (DIAG, -DIAG),
    (-DIAG, -DIAG),
];

fn gradient(seed: u64, x: i64, y: i64) -> (f64, f64) {
    GRADIENTS[(hash(seed, x, y) % 8) as usize]
}

fn fade(t: f64) -> f64 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

/// Single-octave 2D gradient noise. The raw value is scaled by sqrt(2), which makes its range -1..=1.
fn perlin(seed: u64, x: f64, y: f64) -> f64 {
    let x0 = x.floor();
    let y0 = y.floor();
    let (xi, yi) = (x0 as i64, y0 as i64);
    let (fx, fy) = (x - x0, y - y0);

    let dot = |gx: i64, gy: i64, dx: f64, dy: f64| {
        let (ux, uy) = gradient(seed, gx, gy);
        ux * dx + uy * dy
    };
    let n00 = dot(xi, yi, fx, fy);
    let n10 = dot(xi + 1, yi, fx - 1.0, fy);
    let n01 = dot(xi, yi + 1, fx, fy - 1.0);
    let n11 = dot(xi + 1, yi + 1, fx - 1.0, fy - 1.0);

    let u = fade(fx);
    let v = fade(fy);
    let raw = lerp(lerp(n00, n10, u), lerp(n01, n11, u), v);
    clamp_unit(raw * SQRT_2)
}

/// Distance from (x, y) to the nearest feature point. Each unit cell holds one point at a hashed offset, and the
/// nine cells around the sample are searched. The distance is cut at 1 and mapped so 0 gives -1 and 1 gives 1.
fn voronoi(seed: u64, x: f64, y: f64) -> f64 {
    let cx = x.floor() as i64;
    let cy = y.floor() as i64;
    let mut best = f64::INFINITY;
    for dy in -1..=1 {
        for dx in -1..=1 {
            let (gx, gy) = (cx + dx, cy + dy);
            let px = gx as f64 + unit(hash(seed, gx, gy));
            let py = gy as f64 + unit(hash(seed.wrapping_add(0x5555), gx, gy));
            let d = ((px - x) * (px - x) + (py - y) * (py - y)).sqrt();
            if d < best {
                best = d;
            }
        }
    }
    clamp_unit(2.0 * best.min(1.0) - 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOL: f64 = 1e-12;

    fn fractal_of(octaves: u32) -> Fractal {
        Fractal { seed: 7, frequency: 1.0, octaves, lacunarity: 2.0, persistence: 0.5 }
    }

    #[test]
    fn constant_source_returns_its_value_clamped() {
        assert_eq!(NoiseSource::Constant(0.25).value(3.0, -9.0), 0.25);
        assert_eq!(NoiseSource::Constant(4.0).value(0.0, 0.0), 1.0);
    }

    #[test]
    fn checkerboard_alternates_by_cell_parity_including_negative_cells() {
        let s = NoiseSource::Checkerboard { size: 2.0 };
        assert_eq!(s.value(0.5, 0.5), 1.0, "cell (0, 0)");
        assert_eq!(s.value(2.5, 0.5), -1.0, "cell (1, 0)");
        assert_eq!(s.value(2.5, 2.5), 1.0, "cell (1, 1)");
        assert_eq!(s.value(-0.5, 0.5), -1.0, "cell (-1, 0) floors, it does not truncate toward zero");
        assert_eq!(NoiseSource::Checkerboard { size: 0.0 }.value(1.0, 1.0), 0.0);
    }

    #[test]
    fn perlin_is_zero_on_integer_lattice_points() {
        for &(x, y) in &[(0.0, 0.0), (3.0, 4.0), (-2.0, 7.0)] {
            let v = NoiseSource::Perlin { seed: 42, frequency: 1.0 }.value(x, y);
            assert!(v.abs() < TOL, "value at ({x}, {y}) was {v}");
        }
    }

    #[test]
    fn every_source_stays_in_the_unit_range_and_is_deterministic() {
        let sources = [
            NoiseSource::Perlin { seed: 1, frequency: 0.37 },
            NoiseSource::Fractal(fractal_of(5)),
            NoiseSource::Billow(fractal_of(4)),
            NoiseSource::Ridged(fractal_of(4)),
            NoiseSource::Voronoi { seed: 9, frequency: 0.5 },
        ];
        for s in sources.iter() {
            for i in 0..40 {
                let (x, y) = (i as f64 * 0.73 - 11.0, i as f64 * 0.41 + 3.0);
                let v = s.value(x, y);
                assert!((-1.0..=1.0).contains(&v), "{s:?} gave {v}");
                assert_eq!(v, s.value(x, y), "same point, same value");
            }
        }
    }

    #[test]
    fn zero_octaves_gives_zero_and_one_octave_is_the_base_gradient_noise() {
        assert_eq!(NoiseSource::Fractal(fractal_of(0)).value(1.3, 2.7), 0.0);
        let base = NoiseSource::Perlin { seed: 7, frequency: 1.0 }.value(1.3, 2.7);
        let one = NoiseSource::Fractal(fractal_of(1)).value(1.3, 2.7);
        assert!((one - base).abs() < TOL, "one octave, weight 1, is the base: {one} vs {base}");
    }

    #[test]
    fn ridged_folds_zero_crossings_to_the_top_and_billow_to_the_bottom() {
        // A raw value of 0 is folded to +1 by Ridged and to -1 by Billow. Both are the fold of the base at lattice points.
        let base = NoiseSource::Perlin { seed: 7, frequency: 1.0 };
        assert_eq!(base.value(2.0, 5.0), 0.0);
        let one = fractal_of(1);
        assert_eq!(NoiseSource::Ridged(one).value(2.0, 5.0), 1.0);
        assert_eq!(NoiseSource::Billow(one).value(2.0, 5.0), -1.0);
    }

    #[test]
    fn checkerboard_map_has_the_expected_grid_values() {
        let map = NoiseMap::sample(&NoiseSource::Checkerboard { size: 1.0 }, 3, 2, (0.5, 0.5), 1.0);
        assert_eq!((map.width(), map.height()), (3, 2));
        assert_eq!(map.get(0, 0), Some(1.0));
        assert_eq!(map.get(1, 0), Some(-1.0));
        assert_eq!(map.get(2, 0), Some(1.0));
        assert_eq!(map.get(0, 1), Some(-1.0));
        assert_eq!(map.get(3, 0), None, "column past the width");
        assert_eq!(map.range(), Some((-1.0, 1.0)));
    }

    #[test]
    fn empty_map_has_no_range() {
        let map = NoiseMap::sample(&NoiseSource::Constant(0.5), 0, 4, (0.0, 0.0), 1.0);
        assert_eq!(map.range(), None);
        assert_eq!(map.get(0, 0), None);
    }
}
