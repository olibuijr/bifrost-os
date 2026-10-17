//! Lookup-table pipelines of ICC profiles: the `mft1` (lut8Type) and `mft2` (lut16Type) tags and the `mAB ` / `mBA `
//! (lutAtoBType, lutBtoAType) tags. A pipeline is a list of stages applied in order: tone curves, a multidimensional
//! table (CLUT), or a 3x3 matrix with an offset. Values are normalised to 0..=1 between stages; `pcs_to_xyz` and
//! `xyz_to_pcs` turn the connection-space encodings into XYZ relative to D50 and back. Only the forms a matrix-free
//! RGB profile uses are covered: up to four channels in and out, and grids of at least two points per channel.

use super::colorspace::{self, Mat3};
use super::icc::Curve;

/// Which tag type a pipeline was read from. The type decides how the connection space is encoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// `mft1`: 8-bit tables, 8-bit CLUT, Lab 8-bit or XYZ.
    Lut8,
    /// `mft2`: 16-bit tables and CLUT, Lab 16-bit (v2 scaling) or XYZ.
    Lut16,
    /// `mAB ` and `mBA `: curves, CLUT, matrix and per-channel curves (v4 scaling).
    Mab,
}

/// A multidimensional table. `values` holds the normalised outputs of every grid point, the first input channel
/// varying slowest.
#[derive(Clone, Debug, PartialEq)]
pub struct Clut {
    /// Grid points per input channel (each at least 2).
    pub grid: Vec<usize>,
    /// Output channels per grid point.
    pub outputs: usize,
    pub values: Vec<f64>,
}

/// One stage of a pipeline.
#[derive(Clone, Debug, PartialEq)]
pub enum Stage {
    /// One tone curve per channel.
    Curves(Vec<Curve>),
    /// A multidimensional table.
    Clut(Clut),
    /// A 3x3 matrix (row-major) with an offset. When `xyz` is set the values are XYZ-encoded (1.0 is 0x8000), so the
    /// stage scales them to XYZ before the matrix and back afterwards.
    Matrix { m: Mat3, off: [f64; 3], xyz: bool },
}

/// A parsed pipeline ready to evaluate.
#[derive(Clone, Debug, PartialEq)]
pub struct Lut {
    pub kind: Kind,
    pub inputs: usize,
    pub outputs: usize,
    /// Whether the connection space is Lab (otherwise XYZ).
    pub pcs_lab: bool,
    pub stages: Vec<Stage>,
}

fn be_u16(data: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(data.get(at..at + 2)?.try_into().ok()?))
}

fn be_u32(data: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(data.get(at..at + 4)?.try_into().ok()?))
}

/// A signed 15.16 fixed-point number.
fn s15(data: &[u8], at: usize) -> Option<f64> {
    Some(i32::from_be_bytes(data.get(at..at + 4)?.try_into().ok()?) as f64 / 65536.0)
}

/// The nine matrix values at `at`, row-major.
fn matrix_at(data: &[u8], at: usize) -> Option<Mat3> {
    let v: Vec<f64> = (0..9).map(|i| s15(data, at + 4 * i)).collect::<Option<_>>()?;
    Some([[v[0], v[1], v[2]], [v[3], v[4], v[5]], [v[6], v[7], v[8]]])
}

fn grid_size(grid: usize, inputs: usize) -> Option<usize> {
    if !(1..=4).contains(&inputs) || grid < 2 {
        return None;
    }
    grid.checked_pow(inputs as u32)
}

/// Parses an `mft1` or `mft2` body (`wide` selects `mft2`). `b2a` tells a PCS-to-device table apart: its input is
/// the connection space, so the matrix applies first when that space is XYZ.
pub fn parse_mft(body: &[u8], wide: bool, b2a: bool, pcs_lab: bool) -> Option<Lut> {
    let inputs = *body.get(8)? as usize;
    let outputs = *body.get(9)? as usize;
    let grid = *body.get(10)? as usize;
    let count = grid_size(grid, inputs)?;
    if !(1..=8).contains(&outputs) {
        return None;
    }
    let width = if wide { 2 } else { 1 };
    let (in_entries, out_entries, mut at) = if wide {
        (be_u16(body, 48)? as usize, be_u16(body, 50)? as usize, 52)
    } else {
        (256, 256, 48)
    };
    if in_entries < 2 || out_entries < 2 {
        return None;
    }
    let sample = |at: usize| -> Option<f64> {
        if wide {
            be_u16(body, at).map(|v| v as f64 / 65535.0)
        } else {
            body.get(at).map(|&v| v as f64 / 255.0)
        }
    };
    let mut stages = Vec::new();
    if b2a && !pcs_lab && inputs == 3 {
        stages.push(Stage::Matrix { m: matrix_at(body, 12)?, off: [0.0; 3], xyz: true });
    }
    let mut input_curves = Vec::with_capacity(inputs);
    for _ in 0..inputs {
        let mut t = Vec::with_capacity(in_entries);
        for _ in 0..in_entries {
            t.push(sample(at)?);
            at += width;
        }
        input_curves.push(Curve::Table(t));
    }
    stages.push(Stage::Curves(input_curves));
    let mut values = Vec::with_capacity(count * outputs);
    for _ in 0..count * outputs {
        values.push(sample(at)?);
        at += width;
    }
    stages.push(Stage::Clut(Clut { grid: vec![grid; inputs], outputs, values }));
    let mut output_curves = Vec::with_capacity(outputs);
    for _ in 0..outputs {
        let mut t = Vec::with_capacity(out_entries);
        for _ in 0..out_entries {
            t.push(sample(at)?);
            at += width;
        }
        output_curves.push(Curve::Table(t));
    }
    stages.push(Stage::Curves(output_curves));
    let kind = if wide { Kind::Lut16 } else { Kind::Lut8 };
    Some(Lut { kind, inputs, outputs, pcs_lab, stages })
}

/// Reads `n` tone curves laid out back to back from `at`, each padded to four bytes.
fn curve_set(data: &[u8], at: usize, n: usize) -> Option<Vec<Curve>> {
    let mut at = at;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let (c, len) = super::icc::curve_at(data, at)?;
        out.push(c);
        at += len;
    }
    Some(out)
}

/// Parses an `mAB ` (device to PCS, `b2a` false) or `mBA ` (PCS to device, `b2a` true) body. The stages are the
/// A, CLUT, M, matrix and B blocks of the tag, present only when their offset is non-zero.
pub fn parse_mab(body: &[u8], b2a: bool, pcs_lab: bool) -> Option<Lut> {
    let inputs = *body.get(8)? as usize;
    let outputs = *body.get(9)? as usize;
    let off_b = be_u32(body, 12)? as usize;
    let off_matrix = be_u32(body, 16)? as usize;
    let off_m = be_u32(body, 20)? as usize;
    let off_clut = be_u32(body, 24)? as usize;
    let off_a = be_u32(body, 28)? as usize;
    if !(1..=4).contains(&inputs) || !(1..=8).contains(&outputs) {
        return None;
    }
    // Channel counts at each position. A2B: A (in), CLUT (in to out), M (out), matrix (out), B (out).
    // B2A: B (in), matrix (in), M (in), CLUT (in to out), A (out).
    let (a_n, m_n, b_n) = if b2a { (outputs, inputs, inputs) } else { (inputs, outputs, outputs) };
    let mut stages = Vec::new();
    let push_curves = |off: usize, n: usize, stages: &mut Vec<Stage>| -> Option<()> {
        if off != 0 {
            stages.push(Stage::Curves(curve_set(body, off, n)?));
        }
        Some(())
    };
    let push_clut = |stages: &mut Vec<Stage>| -> Option<()> {
        if off_clut == 0 {
            return Some(());
        }
        let mut grid = Vec::with_capacity(inputs);
        for i in 0..inputs {
            grid.push(*body.get(off_clut + i)? as usize);
        }
        if grid.iter().any(|&g| g < 2) {
            return None;
        }
        let precision = *body.get(off_clut + 16)?;
        let count: usize = grid.iter().product::<usize>() * outputs;
        let mut values = Vec::with_capacity(count);
        let mut at = off_clut + 20;
        for _ in 0..count {
            let v = match precision {
                1 => *body.get(at)? as f64 / 255.0,
                2 => be_u16(body, at)? as f64 / 65535.0,
                _ => return None,
            };
            values.push(v);
            at += precision as usize;
        }
        stages.push(Stage::Clut(Clut { grid, outputs, values }));
        Some(())
    };
    if b2a {
        push_curves(off_b, b_n, &mut stages)?;
        if off_matrix != 0 {
            stages.push(Stage::Matrix { m: matrix_at(body, off_matrix)?, off: offset_at(body, off_matrix + 36)?, xyz: false });
        }
        push_curves(off_m, m_n, &mut stages)?;
        push_clut(&mut stages)?;
        push_curves(off_a, a_n, &mut stages)?;
    } else {
        push_curves(off_a, a_n, &mut stages)?;
        push_clut(&mut stages)?;
        push_curves(off_m, m_n, &mut stages)?;
        if off_matrix != 0 {
            stages.push(Stage::Matrix { m: matrix_at(body, off_matrix)?, off: offset_at(body, off_matrix + 36)?, xyz: false });
        }
        push_curves(off_b, b_n, &mut stages)?;
    }
    let kind = Kind::Mab;
    Some(Lut { kind, inputs, outputs, pcs_lab, stages })
}

fn offset_at(data: &[u8], at: usize) -> Option<[f64; 3]> {
    Some([s15(data, at)?, s15(data, at + 4)?, s15(data, at + 8)?])
}

impl Lut {
    /// Runs the pipeline on `input` (normalised, one value per input channel). `None` when the input count is wrong.
    pub fn eval(&self, input: &[f64]) -> Option<Vec<f64>> {
        if input.len() != self.inputs {
            return None;
        }
        let mut v: Vec<f64> = input.iter().map(|x| x.clamp(0.0, 1.0)).collect();
        for stage in &self.stages {
            v = match stage {
                Stage::Curves(curves) => {
                    if curves.len() != v.len() {
                        return None;
                    }
                    v.iter().zip(curves).map(|(x, c)| c.eval(*x)).collect()
                }
                Stage::Clut(clut) => clut.eval(&v)?,
                Stage::Matrix { m, off, xyz } => {
                    if v.len() != 3 {
                        return None;
                    }
                    let (s_in, s_out) = if *xyz { (65535.0 / 32768.0, 32768.0 / 65535.0) } else { (1.0, 1.0) };
                    let x = [v[0] * s_in, v[1] * s_in, v[2] * s_in];
                    let y = colorspace::apply(*m, x);
                    (0..3).map(|i| (y[i] + off[i]) * s_out).collect()
                }
            };
        }
        Some(v)
    }

    /// The connection-space encoding of an output (normalised) as XYZ relative to D50.
    pub fn pcs_to_xyz(&self, e: [f64; 3]) -> [f64; 3] {
        if !self.pcs_lab {
            return e.map(|v| v * 65535.0 / 32768.0);
        }
        let (l, a, b) = match self.kind {
            Kind::Lut16 => (e[0] * 65535.0 / 65280.0 * 100.0, e[1] * 65535.0 / 256.0 - 128.0, e[2] * 65535.0 / 256.0 - 128.0),
            Kind::Lut8 | Kind::Mab => (e[0] * 100.0, e[1] * 255.0 - 128.0, e[2] * 255.0 - 128.0),
        };
        lab_to_xyz([l, a, b])
    }

    /// The encoding of an XYZ value (relative to D50) in this pipeline's connection space: the inverse of
    /// `pcs_to_xyz`, used as the input of a PCS-to-device table.
    pub fn xyz_to_pcs(&self, xyz: [f64; 3]) -> [f64; 3] {
        if !self.pcs_lab {
            return xyz.map(|v| v * 32768.0 / 65535.0);
        }
        let [l, a, b] = xyz_to_lab(xyz);
        match self.kind {
            Kind::Lut16 => [l / 100.0 * 65280.0 / 65535.0, (a + 128.0) * 256.0 / 65535.0, (b + 128.0) * 256.0 / 65535.0],
            Kind::Lut8 | Kind::Mab => [l / 100.0, (a + 128.0) / 255.0, (b + 128.0) / 255.0],
        }
    }
}

impl Clut {
    /// Multilinear interpolation of the table at `input` (normalised).
    pub fn eval(&self, input: &[f64]) -> Option<Vec<f64>> {
        let n = self.grid.len();
        if input.len() != n || n == 0 || n > 4 {
            return None;
        }
        let mut base = [0usize; 4];
        let mut frac = [0.0f64; 4];
        for d in 0..n {
            let g = self.grid[d];
            let pos = input[d].clamp(0.0, 1.0) * (g - 1) as f64;
            let i = (pos.floor() as usize).min(g - 2);
            base[d] = i;
            frac[d] = pos - i as f64;
        }
        let mut out = vec![0.0; self.outputs];
        for corner in 0..(1usize << n) {
            let mut w = 1.0;
            let mut idx = 0usize;
            for d in 0..n {
                let bit = (corner >> (n - 1 - d)) & 1;
                w *= if bit == 1 { frac[d] } else { 1.0 - frac[d] };
                idx = idx * self.grid[d] + base[d] + bit;
            }
            if w == 0.0 {
                continue;
            }
            let at = idx * self.outputs;
            for (o, slot) in out.iter_mut().enumerate() {
                *slot += w * self.values.get(at + o)?;
            }
        }
        Some(out)
    }
}

/// The D50 white in XYZ, the reference of the Lab connection space.
fn d50() -> [f64; 3] {
    colorspace::d50_xyz()
}

/// CIE Lab (D50) to XYZ relative to D50.
pub fn lab_to_xyz([l, a, b]: [f64; 3]) -> [f64; 3] {
    const DELTA: f64 = 6.0 / 29.0;
    let fy = (l + 16.0) / 116.0;
    let fx = fy + a / 500.0;
    let fz = fy - b / 200.0;
    let inv = |f: f64| if f > DELTA { f * f * f } else { 3.0 * DELTA * DELTA * (f - 4.0 / 29.0) };
    let w = d50();
    [w[0] * inv(fx), w[1] * inv(fy), w[2] * inv(fz)]
}

/// XYZ relative to D50 to CIE Lab (D50).
pub fn xyz_to_lab(xyz: [f64; 3]) -> [f64; 3] {
    const DELTA: f64 = 6.0 / 29.0;
    let w = d50();
    let f = |t: f64| if t > DELTA * DELTA * DELTA { t.cbrt() } else { t / (3.0 * DELTA * DELTA) + 4.0 / 29.0 };
    let (fx, fy, fz) = (f(xyz[0] / w[0]), f(xyz[1] / w[1]), f(xyz[2] / w[2]));
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bytes of an `mft2` body with 3 inputs, 3 outputs, grid 2, identity tables and an identity CLUT.
    fn identity_mft2() -> Vec<u8> {
        let mut b = Vec::new();
        b.extend(b"mft2");
        b.extend([0u8; 4]);
        b.extend([3u8, 3, 2, 0]);
        // Identity matrix: 1.0 is 0x00010000 in s15Fixed16.
        for i in 0..9 {
            let v: i32 = if i % 4 == 0 { 65536 } else { 0 };
            b.extend(v.to_be_bytes());
        }
        b.extend(2u16.to_be_bytes()); // input entries
        b.extend(2u16.to_be_bytes()); // output entries
        for _ in 0..3 {
            for v in [0u16, 65535] {
                b.extend(v.to_be_bytes());
            }
        }
        // CLUT: the first input varies slowest; output equals the input (r, g, b) at each corner.
        for r in 0..2u16 {
            for g in 0..2u16 {
                for bl in 0..2u16 {
                    for v in [r, g, bl] {
                        b.extend((v * 65535).to_be_bytes());
                    }
                }
            }
        }
        for _ in 0..3 {
            for v in [0u16, 65535] {
                b.extend(v.to_be_bytes());
            }
        }
        b
    }

    #[test]
    fn lut16_identity_table_returns_its_input() {
        let lut = parse_mft(&identity_mft2(), true, false, false).expect("mft2 parses");
        assert_eq!(lut.kind, Kind::Lut16);
        let out = lut.eval(&[0.25, 0.5, 0.75]).unwrap();
        for (got, want) in out.iter().zip([0.25, 0.5, 0.75]) {
            assert!((got - want).abs() < 1e-3, "{got} vs {want}");
        }
    }

    #[test]
    fn truncated_table_is_refused() {
        let mut b = identity_mft2();
        b.truncate(b.len() - 10);
        assert!(parse_mft(&b, true, false, false).is_none());
    }

    #[test]
    fn clut_interpolates_between_grid_points() {
        let clut = Clut { grid: vec![2], outputs: 1, values: vec![0.0, 1.0] };
        assert!((clut.eval(&[0.3]).unwrap()[0] - 0.3).abs() < 1e-12);
    }

    #[test]
    fn lab_round_trips_through_xyz() {
        let lab = [55.0, -20.0, 30.0];
        let back = xyz_to_lab(lab_to_xyz(lab));
        for i in 0..3 {
            assert!((back[i] - lab[i]).abs() < 1e-6, "{back:?}");
        }
        // The D50 white is L 100 with a and b zero.
        let white = xyz_to_lab(d50());
        assert!((white[0] - 100.0).abs() < 1e-9 && white[1].abs() < 1e-9 && white[2].abs() < 1e-9);
    }
}
