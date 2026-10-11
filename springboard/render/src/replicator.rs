//! Replicated instances of one source: each instance applies the transform once more than the previous one, waits a
//! fixed delay per step, and shifts the colour of the source by a per-instance offset. A pure model that gives the
//! per-instance matrix, delay and colour; the compositor draws the copies.

/// A row-major 4x4 matrix in f64. A point is a column vector: `m * [x, y, z, 1]`.
pub type Mat4 = [[f64; 4]; 4];

pub const IDENTITY: Mat4 = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];

/// A translation by (x, y, z).
pub fn translation(x: f64, y: f64, z: f64) -> Mat4 {
    let mut m = IDENTITY;
    m[0][3] = x;
    m[1][3] = y;
    m[2][3] = z;
    m
}

/// The matrix product `a * b`.
pub fn mul(a: &Mat4, b: &Mat4) -> Mat4 {
    let mut out = [[0.0; 4]; 4];
    for (r, row) in out.iter_mut().enumerate() {
        for (c, cell) in row.iter_mut().enumerate() {
            *cell = (0..4).map(|k| a[r][k] * b[k][c]).sum();
        }
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Replicator {
    /// The number of copies, including the source. Zero is treated as one.
    pub count: usize,
    /// Seconds between consecutive instances.
    pub delay: f64,
    /// Applied once per step: instance `i` is this matrix raised to the power `i`.
    pub transform: Mat4,
    /// The colour multiplied into the source for every instance, as RGBA in 0..=1.
    pub color: [f32; 4],
    /// Added per instance, as RGBA: instance `i` gets `i * offset` on top of its colour.
    pub color_offset: [f32; 4],
}

impl Default for Replicator {
    fn default() -> Self {
        Replicator { count: 1, delay: 0.0, transform: IDENTITY, color: [1.0; 4], color_offset: [0.0; 4] }
    }
}

impl Replicator {
    /// The number of instances drawn: `count`, and at least one.
    pub fn instances(&self) -> usize {
        self.count.max(1)
    }

    /// The matrix of instance `i`: the transform applied `i` times, with instance 0 the untransformed source.
    pub fn matrix(&self, i: usize) -> Mat4 {
        (0..i).fold(IDENTITY, |acc, _| mul(&self.transform, &acc))
    }

    /// The time offset of instance `i` in seconds.
    pub fn delay_of(&self, i: usize) -> f64 {
        i as f64 * self.delay
    }

    /// The colour of instance `i` for a source colour: the source times `color`, plus `i * color_offset`, clamped.
    pub fn color_of(&self, i: usize, source: [f32; 4]) -> [f32; 4] {
        let mut out = [0.0; 4];
        for c in 0..4 {
            let v = source[c] * self.color[c] + i as f32 * self.color_offset[c];
            out[c] = v.clamp(0.0, 1.0);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn a_zero_count_still_draws_the_source() {
        let r = Replicator { count: 0, ..Default::default() };
        assert_eq!(r.instances(), 1);
        assert_eq!(Replicator { count: 5, ..Default::default() }.instances(), 5);
    }

    #[test]
    fn instance_matrices_are_the_transform_raised_to_the_index() {
        let r = Replicator { count: 5, transform: translation(10.0, 0.0, 0.0), ..Default::default() };
        assert_eq!(r.matrix(0), IDENTITY);
        assert!(close(r.matrix(1)[0][3], 10.0));
        assert!(close(r.matrix(3)[0][3], 30.0));
        assert!(close(r.matrix(3)[1][3], 0.0));
    }

    #[test]
    fn composed_transforms_accumulate_in_the_matrix_product() {
        // One step moves by (1, 2, 0), so instance 2 is the step applied twice: (2, 4, 0).
        let r = Replicator { count: 3, transform: translation(1.0, 2.0, 0.0), ..Default::default() };
        let m = r.matrix(2);
        assert!(close(m[0][3], 2.0));
        assert!(close(m[1][3], 4.0));
    }

    #[test]
    fn instance_delays_step_linearly() {
        let r = Replicator { count: 4, delay: 0.5, ..Default::default() };
        assert!(close(r.delay_of(0), 0.0));
        assert!(close(r.delay_of(3), 1.5));
    }

    #[test]
    fn colour_offsets_grow_per_instance_and_clamp_at_one() {
        // The green offset reaches 1.0 at the last of five instances, as in a row that fades from black to green.
        let r = Replicator { count: 5, color_offset: [0.0, 0.25, 0.0, 0.0], ..Default::default() };
        let src = [0.0, 0.0, 0.0, 1.0];
        assert_eq!(r.color_of(0, src), [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(r.color_of(2, src), [0.0, 0.5, 0.0, 1.0]);
        assert_eq!(r.color_of(4, src), [0.0, 1.0, 0.0, 1.0]);
        let big = Replicator { color_offset: [0.5, 0.0, 0.0, 0.0], ..Default::default() };
        assert_eq!(big.color_of(4, src)[0], 1.0);
    }

    #[test]
    fn the_colour_multiplier_scales_the_source_before_the_offset() {
        let r = Replicator { color: [0.5, 1.0, 1.0, 1.0], color_offset: [0.0, 0.0, 0.0, -0.5], ..Default::default() };
        let out = r.color_of(1, [1.0, 1.0, 1.0, 1.0]);
        assert_eq!(out, [0.5, 1.0, 1.0, 0.5]);
    }
}
