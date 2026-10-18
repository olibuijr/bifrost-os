//! General array-operation kernels: the compute library an array framework dispatches to. Row-major 2-D
//! `f32` tensors: matrix multiply and transpose, elementwise maps (exp, log, tanh, sigmoid, SiLU, GELU),
//! broadcast add of a row vector, reductions along an axis (sum, mean, max, argmax), numerically stable
//! softmax and log-softmax, layer and RMS normalisation, running sum, row gather and top-k. The layer
//! types that use them (convolution, dense, recurrent) are in `bifrost-ui` `cnn_layers`; device placement
//! and dispatch geometry are in `compute` and `compute_dispatch`. The GPU twins are
//! `springboard/gpu/shaders/matmul.comp` and `array_softmax.comp`. Written from the standard definitions.

/// `a` (`m` x `k`) times `b` (`k` x `n`) as an `m` x `n` matrix. `None` on a size mismatch.
pub fn matmul(a: &[f32], m: usize, k: usize, b: &[f32], n: usize) -> Option<Vec<f32>> {
    if a.len() != m.checked_mul(k)? || b.len() != k.checked_mul(n)? {
        return None;
    }
    let mut out = vec![0.0f32; m * n];
    for i in 0..m {
        for p in 0..k {
            let av = a[i * k + p];
            for j in 0..n {
                out[i * n + j] += av * b[p * n + j];
            }
        }
    }
    Some(out)
}

/// The transpose of an `r` x `c` matrix (a `c` x `r` matrix).
pub fn transpose(a: &[f32], r: usize, c: usize) -> Option<Vec<f32>> {
    if a.len() != r.checked_mul(c)? {
        return None;
    }
    let mut out = vec![0.0f32; r * c];
    for i in 0..r {
        for j in 0..c {
            out[j * r + i] = a[i * c + j];
        }
    }
    Some(out)
}

/// A unary elementwise function.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Unary {
    Exp,
    Log,
    Tanh,
    Sigmoid,
    Silu,
    /// The tanh approximation of GELU.
    Gelu,
    Relu,
}

impl Unary {
    pub fn apply(self, x: f32) -> f32 {
        match self {
            Unary::Exp => x.exp(),
            Unary::Log => x.ln(),
            Unary::Tanh => x.tanh(),
            Unary::Sigmoid => 1.0 / (1.0 + (-x).exp()),
            Unary::Silu => x / (1.0 + (-x).exp()),
            Unary::Gelu => 0.5 * x * (1.0 + (0.797_884_6 * (x + 0.044_715 * x * x * x)).tanh()),
            Unary::Relu => x.max(0.0),
        }
    }
}

/// Applies `f` to every element.
pub fn map_unary(a: &[f32], f: Unary) -> Vec<f32> {
    a.iter().map(|x| f.apply(*x)).collect()
}

/// Adds the row vector `bias` (length `c`) to every row of an `r` x `c` matrix.
pub fn add_row_bias(a: &[f32], r: usize, c: usize, bias: &[f32]) -> Option<Vec<f32>> {
    if a.len() != r.checked_mul(c)? || bias.len() != c {
        return None;
    }
    Some((0..r * c).map(|i| a[i] + bias[i % c]).collect())
}

/// The axis a reduction runs along: `Rows` reduces each row to one value (length `r`), `Columns`
/// reduces each column (length `c`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Axis {
    Rows,
    Columns,
}

fn reduce(a: &[f32], r: usize, c: usize, axis: Axis, init: f32, f: impl Fn(f32, f32) -> f32) -> Option<Vec<f32>> {
    if a.len() != r.checked_mul(c)? {
        return None;
    }
    Some(match axis {
        Axis::Rows => (0..r).map(|i| a[i * c..(i + 1) * c].iter().fold(init, |acc, v| f(acc, *v))).collect(),
        Axis::Columns => (0..c).map(|j| (0..r).fold(init, |acc, i| f(acc, a[i * c + j]))).collect(),
    })
}

/// Sum along an axis.
pub fn sum(a: &[f32], r: usize, c: usize, axis: Axis) -> Option<Vec<f32>> {
    reduce(a, r, c, axis, 0.0, |x, y| x + y)
}

/// Mean along an axis; `None` when the reduced length is zero.
pub fn mean(a: &[f32], r: usize, c: usize, axis: Axis) -> Option<Vec<f32>> {
    let n = match axis {
        Axis::Rows => c,
        Axis::Columns => r,
    };
    if n == 0 {
        return None;
    }
    let s = sum(a, r, c, axis)?;
    Some(s.into_iter().map(|v| v / n as f32).collect())
}

/// Maximum along an axis; `None` when the reduced length is zero.
pub fn max(a: &[f32], r: usize, c: usize, axis: Axis) -> Option<Vec<f32>> {
    if r == 0 || c == 0 {
        return None;
    }
    reduce(a, r, c, axis, f32::NEG_INFINITY, f32::max)
}

/// The index of the largest value in each row (the first one on a tie).
pub fn argmax_rows(a: &[f32], r: usize, c: usize) -> Option<Vec<usize>> {
    if c == 0 || a.len() != r.checked_mul(c)? {
        return None;
    }
    Some(
        (0..r)
            .map(|i| {
                let row = &a[i * c..(i + 1) * c];
                let mut best = 0;
                for (j, v) in row.iter().enumerate() {
                    if *v > row[best] {
                        best = j;
                    }
                }
                best
            })
            .collect(),
    )
}

/// Row-wise softmax with the row maximum subtracted first, so large logits do not overflow.
pub fn softmax_rows(a: &[f32], r: usize, c: usize) -> Option<Vec<f32>> {
    if c == 0 || a.len() != r.checked_mul(c)? {
        return None;
    }
    let mut out = vec![0.0f32; r * c];
    for i in 0..r {
        let row = &a[i * c..(i + 1) * c];
        let m = row.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let mut total = 0.0f32;
        for j in 0..c {
            let e = (row[j] - m).exp();
            out[i * c + j] = e;
            total += e;
        }
        for j in 0..c {
            out[i * c + j] /= total;
        }
    }
    Some(out)
}

/// Row-wise log-softmax: `x - max - ln(sum(exp(x - max)))`.
pub fn log_softmax_rows(a: &[f32], r: usize, c: usize) -> Option<Vec<f32>> {
    if c == 0 || a.len() != r.checked_mul(c)? {
        return None;
    }
    let mut out = vec![0.0f32; r * c];
    for i in 0..r {
        let row = &a[i * c..(i + 1) * c];
        let m = row.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let lse = row.iter().map(|v| (v - m).exp()).sum::<f32>().ln();
        for j in 0..c {
            out[i * c + j] = row[j] - m - lse;
        }
    }
    Some(out)
}

/// Layer normalisation of each row: zero mean, unit variance (plus `eps`), then `gamma * x + beta`.
pub fn layer_norm_rows(a: &[f32], r: usize, c: usize, gamma: &[f32], beta: &[f32], eps: f32) -> Option<Vec<f32>> {
    if c == 0 || a.len() != r.checked_mul(c)? || gamma.len() != c || beta.len() != c {
        return None;
    }
    let mut out = vec![0.0f32; r * c];
    for i in 0..r {
        let row = &a[i * c..(i + 1) * c];
        let mu = row.iter().sum::<f32>() / c as f32;
        let var = row.iter().map(|v| (v - mu) * (v - mu)).sum::<f32>() / c as f32;
        let inv = 1.0 / (var + eps).sqrt();
        for j in 0..c {
            out[i * c + j] = (row[j] - mu) * inv * gamma[j] + beta[j];
        }
    }
    Some(out)
}

/// RMS normalisation of each row: `x / sqrt(mean(x^2) + eps) * gamma`.
pub fn rms_norm_rows(a: &[f32], r: usize, c: usize, gamma: &[f32], eps: f32) -> Option<Vec<f32>> {
    if c == 0 || a.len() != r.checked_mul(c)? || gamma.len() != c {
        return None;
    }
    let mut out = vec![0.0f32; r * c];
    for i in 0..r {
        let row = &a[i * c..(i + 1) * c];
        let ms = row.iter().map(|v| v * v).sum::<f32>() / c as f32;
        let inv = 1.0 / (ms + eps).sqrt();
        for j in 0..c {
            out[i * c + j] = row[j] * inv * gamma[j];
        }
    }
    Some(out)
}

/// The running (inclusive) sum along each row.
pub fn cumsum_rows(a: &[f32], r: usize, c: usize) -> Option<Vec<f32>> {
    if a.len() != r.checked_mul(c)? {
        return None;
    }
    let mut out = a.to_vec();
    for i in 0..r {
        for j in 1..c {
            out[i * c + j] += out[i * c + j - 1];
        }
    }
    Some(out)
}

/// Gathers whole rows of an `r` x `c` matrix by index; `None` for an index out of range.
pub fn gather_rows(a: &[f32], r: usize, c: usize, indices: &[usize]) -> Option<Vec<f32>> {
    if a.len() != r.checked_mul(c)? || indices.iter().any(|i| *i >= r) {
        return None;
    }
    let mut out = Vec::with_capacity(indices.len() * c);
    for i in indices {
        out.extend_from_slice(&a[i * c..(i + 1) * c]);
    }
    Some(out)
}

/// The `k` largest values of a vector with their indices, largest first (ties keep the lower index first).
pub fn top_k(values: &[f32], k: usize) -> Vec<(usize, f32)> {
    let mut idx: Vec<usize> = (0..values.len()).collect();
    idx.sort_by(|a, b| values[*b].partial_cmp(&values[*a]).unwrap_or(std::cmp::Ordering::Equal).then(a.cmp(b)));
    idx.into_iter().take(k).map(|i| (i, values[i])).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn matmul_multiplies_and_checks_shapes() {
        // [1 2 3; 4 5 6] x [7 8; 9 10; 11 12]
        let out = matmul(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0], 2, 3, &[7.0, 8.0, 9.0, 10.0, 11.0, 12.0], 2).unwrap();
        assert_eq!(out, vec![58.0, 64.0, 139.0, 154.0]);
        assert!(matmul(&[1.0; 5], 2, 3, &[1.0; 6], 2).is_none());
    }

    #[test]
    fn transpose_swaps_axes_and_twice_is_identity() {
        let a = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
        let t = transpose(&a, 2, 3).unwrap();
        assert_eq!(t, vec![1.0, 4.0, 2.0, 5.0, 3.0, 6.0]);
        assert_eq!(transpose(&t, 3, 2).unwrap(), a.to_vec());
    }

    #[test]
    fn unary_maps_match_known_values() {
        assert!(close(Unary::Sigmoid.apply(0.0), 0.5));
        assert!(close(Unary::Tanh.apply(0.0), 0.0));
        assert!(close(Unary::Silu.apply(0.0), 0.0));
        assert!(close(Unary::Gelu.apply(0.0), 0.0));
        assert!(Unary::Gelu.apply(3.0) > 2.99);
        assert_eq!(map_unary(&[-1.0, 2.0], Unary::Relu), vec![0.0, 2.0]);
        assert!(close(Unary::Log.apply(Unary::Exp.apply(1.5)), 1.5));
    }

    #[test]
    fn bias_broadcasts_over_rows() {
        let out = add_row_bias(&[0.0; 6], 2, 3, &[1.0, 2.0, 3.0]).unwrap();
        assert_eq!(out, vec![1.0, 2.0, 3.0, 1.0, 2.0, 3.0]);
        assert!(add_row_bias(&[0.0; 6], 2, 3, &[1.0]).is_none());
    }

    #[test]
    fn reductions_along_both_axes() {
        let a = [1.0, 5.0, 3.0, 4.0, 2.0, 6.0];
        assert_eq!(sum(&a, 2, 3, Axis::Rows).unwrap(), vec![9.0, 12.0]);
        assert_eq!(sum(&a, 2, 3, Axis::Columns).unwrap(), vec![5.0, 7.0, 9.0]);
        assert_eq!(mean(&a, 2, 3, Axis::Rows).unwrap(), vec![3.0, 4.0]);
        assert_eq!(max(&a, 2, 3, Axis::Columns).unwrap(), vec![4.0, 5.0, 6.0]);
        assert_eq!(argmax_rows(&a, 2, 3).unwrap(), vec![1, 2]);
        assert!(max(&[], 0, 3, Axis::Rows).is_none());
        assert!(mean(&a, 2, 0, Axis::Rows).is_none());
    }

    #[test]
    fn softmax_rows_sum_to_one_and_survive_large_logits() {
        let out = softmax_rows(&[1.0, 2.0, 3.0, 1000.0, 1000.0, 999.0], 2, 3).unwrap();
        assert!(close(out[0..3].iter().sum::<f32>(), 1.0));
        assert!(close(out[3..6].iter().sum::<f32>(), 1.0));
        assert!(out.iter().all(|v| v.is_finite()));
        assert!(out[2] > out[1] && out[1] > out[0]);
        assert!(close(out[3], out[4]));
        let log = log_softmax_rows(&[1.0, 2.0, 3.0], 1, 3).unwrap();
        for j in 0..3 {
            assert!(close(log[j].exp(), out[j]));
        }
    }

    #[test]
    fn layer_norm_gives_zero_mean_and_unit_variance() {
        let out = layer_norm_rows(&[1.0, 2.0, 3.0, 4.0], 1, 4, &[1.0; 4], &[0.0; 4], 1e-9).unwrap();
        let mu: f32 = out.iter().sum::<f32>() / 4.0;
        let var: f32 = out.iter().map(|v| (v - mu) * (v - mu)).sum::<f32>() / 4.0;
        assert!(mu.abs() < 1e-5 && close(var, 1.0));
        assert!(layer_norm_rows(&[1.0; 4], 1, 4, &[1.0; 3], &[0.0; 4], 1e-5).is_none());
    }

    #[test]
    fn rms_norm_has_unit_rms() {
        let out = rms_norm_rows(&[3.0, 4.0], 1, 2, &[1.0, 1.0], 0.0).unwrap();
        let rms = ((out[0] * out[0] + out[1] * out[1]) / 2.0).sqrt();
        assert!(close(rms, 1.0));
    }

    #[test]
    fn cumsum_gather_and_top_k() {
        assert_eq!(cumsum_rows(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0], 2, 3).unwrap(), vec![1.0, 3.0, 6.0, 4.0, 9.0, 15.0]);
        let a = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
        assert_eq!(gather_rows(&a, 3, 2, &[2, 0]).unwrap(), vec![5.0, 6.0, 1.0, 2.0]);
        assert!(gather_rows(&a, 3, 2, &[3]).is_none());
        assert_eq!(top_k(&[0.1, 0.9, 0.9, 0.5], 3), vec![(1, 0.9), (2, 0.9), (3, 0.5)]);
        assert_eq!(top_k(&[1.0], 5).len(), 1);
    }
}
