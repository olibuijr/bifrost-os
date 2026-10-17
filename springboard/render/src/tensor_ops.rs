//! N-dimensional array and dense matrix kernels (std only): the building blocks of a GPU compute framework's array
//! and matrix layers, as CPU references. `NdArray` is a row-major `f32` array with a shape. Operations: axis
//! permutation (transpose), reductions (sum, mean, max, min, product), arg-max, cumulative sum, gather, slice, concat,
//! NumPy-style broadcasting binary operations. Matrix operations: general matrix multiply with
//! transposes and `alpha`/`beta` scaling (`gemm`), matrix-vector product and triangular solves. LU and Cholesky
//! factorisation are in `bifrost-ui` (`linalg`); row-wise 2-D reductions, top-k and normalisation are in `kernel_array_ops`; neural-network
//! layers are in `nn_kernels`.
//!
//! Every function returns `None` for invalid shapes, axes, indices or lengths, so a bad graph is reported rather than
//! reading out of range. `matmul.comp` in the GPU shader directory is the GPU version of `gemm` without transposes.

/// A row-major array of `f32` with a shape. A rank-0 array (empty shape) holds one value.
#[derive(Clone, Debug, PartialEq)]
pub struct NdArray {
    pub shape: Vec<usize>,
    pub data: Vec<f32>,
}

fn count(shape: &[usize]) -> Option<usize> {
    shape.iter().try_fold(1usize, |a, &b| a.checked_mul(b))
}

fn strides(shape: &[usize]) -> Vec<usize> {
    let mut s = vec![1usize; shape.len()];
    for i in (0..shape.len().saturating_sub(1)).rev() {
        s[i] = s[i + 1] * shape[i + 1];
    }
    s
}

impl NdArray {
    /// An array from a shape and its values; `None` when the value count does not match.
    pub fn new(shape: &[usize], data: Vec<f32>) -> Option<NdArray> {
        (count(shape)? == data.len()).then(|| NdArray { shape: shape.to_vec(), data })
    }

    /// An array of zeros; `None` when the size overflows.
    pub fn zeros(shape: &[usize]) -> Option<NdArray> {
        Some(NdArray { shape: shape.to_vec(), data: vec![0.0; count(shape)?] })
    }

    pub fn rank(&self) -> usize {
        self.shape.len()
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
}

/// Reorders the axes: output axis `i` is input axis `perm[i]` (a matrix transpose is `[1, 0]`).
pub fn permute(a: &NdArray, perm: &[usize]) -> Option<NdArray> {
    let r = a.shape.len();
    if perm.len() != r {
        return None;
    }
    let mut seen = vec![false; r];
    for &p in perm {
        if p >= r || seen[p] {
            return None;
        }
        seen[p] = true;
    }
    let src = strides(&a.shape);
    let oshape: Vec<usize> = perm.iter().map(|&p| a.shape[p]).collect();
    let step: Vec<usize> = perm.iter().map(|&p| src[p]).collect();
    let n = a.data.len();
    let mut out = Vec::with_capacity(n);
    let mut idx = vec![0usize; r];
    for _ in 0..n {
        let off: usize = idx.iter().zip(&step).map(|(i, s)| i * s).sum();
        out.push(a.data[off]);
        for d in (0..r).rev() {
            idx[d] += 1;
            if idx[d] < oshape[d] {
                break;
            }
            idx[d] = 0;
        }
    }
    Some(NdArray { shape: oshape, data: out })
}

/// A reduction over one axis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Reduce {
    Sum,
    Mean,
    Max,
    Min,
    Prod,
}

/// Splits the shape around `axis` into (outer count, axis length, inner count).
fn split(shape: &[usize], axis: usize) -> Option<(usize, usize, usize)> {
    if axis >= shape.len() {
        return None;
    }
    Some((shape[..axis].iter().product(), shape[axis], shape[axis + 1..].iter().product()))
}

/// Reduces over `axis`, removing it from the shape. `Max` and `Min` of an empty axis give `None`.
pub fn reduce(a: &NdArray, axis: usize, op: Reduce) -> Option<NdArray> {
    let (outer, len, inner) = split(&a.shape, axis)?;
    if len == 0 && matches!(op, Reduce::Max | Reduce::Min | Reduce::Mean) {
        return None;
    }
    let mut out = Vec::with_capacity(outer * inner);
    for o in 0..outer {
        for i in 0..inner {
            let at = |k: usize| a.data[(o * len + k) * inner + i];
            let v = match op {
                Reduce::Sum => (0..len).map(at).sum::<f32>(),
                Reduce::Mean => (0..len).map(at).sum::<f32>() / len as f32,
                Reduce::Prod => (0..len).map(at).product::<f32>(),
                Reduce::Max => (1..len).fold(at(0), |m, k| m.max(at(k))),
                Reduce::Min => (1..len).fold(at(0), |m, k| m.min(at(k))),
            };
            out.push(v);
        }
    }
    let mut shape = a.shape.clone();
    shape.remove(axis);
    Some(NdArray { shape, data: out })
}

/// Index of the largest value along `axis` (the first one on ties), as `f32` values in an array without that axis.
pub fn argmax(a: &NdArray, axis: usize) -> Option<NdArray> {
    let (outer, len, inner) = split(&a.shape, axis)?;
    if len == 0 {
        return None;
    }
    let mut out = Vec::with_capacity(outer * inner);
    for o in 0..outer {
        for i in 0..inner {
            let mut best = 0usize;
            for k in 1..len {
                if a.data[(o * len + k) * inner + i] > a.data[(o * len + best) * inner + i] {
                    best = k;
                }
            }
            out.push(best as f32);
        }
    }
    let mut shape = a.shape.clone();
    shape.remove(axis);
    Some(NdArray { shape, data: out })
}

/// Inclusive running sum along `axis`, from the start or (with `reverse`) from the end.
pub fn cumsum(a: &NdArray, axis: usize, reverse: bool) -> Option<NdArray> {
    let (outer, len, inner) = split(&a.shape, axis)?;
    let mut out = a.data.clone();
    for o in 0..outer {
        for i in 0..inner {
            let mut run = 0.0f32;
            for step in 0..len {
                let k = if reverse { len - 1 - step } else { step };
                let p = (o * len + k) * inner + i;
                run += a.data[p];
                out[p] = run;
            }
        }
    }
    Some(NdArray { shape: a.shape.clone(), data: out })
}

/// Picks the given positions along `axis` (repeats allowed). `None` for an index out of range.
pub fn gather(a: &NdArray, axis: usize, indices: &[usize]) -> Option<NdArray> {
    let (outer, len, inner) = split(&a.shape, axis)?;
    if indices.iter().any(|&k| k >= len) {
        return None;
    }
    let mut out = Vec::with_capacity(outer * indices.len() * inner);
    for o in 0..outer {
        for &k in indices {
            let start = (o * len + k) * inner;
            out.extend_from_slice(&a.data[start..start + inner]);
        }
    }
    let mut shape = a.shape.clone();
    shape[axis] = indices.len();
    Some(NdArray { shape, data: out })
}

/// The positions `start..end` along `axis`. `None` unless `start <= end <= length`.
pub fn slice(a: &NdArray, axis: usize, start: usize, end: usize) -> Option<NdArray> {
    let (_, len, _) = split(&a.shape, axis)?;
    if start > end || end > len {
        return None;
    }
    let idx: Vec<usize> = (start..end).collect();
    gather(a, axis, &idx)
}

/// Joins arrays along `axis`; all other dimensions must agree. `None` for an empty list.
pub fn concat(parts: &[&NdArray], axis: usize) -> Option<NdArray> {
    let first = parts.first()?;
    split(&first.shape, axis)?;
    for p in parts {
        if p.shape.len() != first.shape.len() {
            return None;
        }
        for (d, (x, y)) in p.shape.iter().zip(&first.shape).enumerate() {
            if d != axis && x != y {
                return None;
            }
        }
    }
    let outer: usize = first.shape[..axis].iter().product();
    let inner: usize = first.shape[axis + 1..].iter().product();
    let total: usize = parts.iter().map(|p| p.shape[axis]).sum();
    let mut out = Vec::with_capacity(outer * total * inner);
    for o in 0..outer {
        for p in parts {
            let chunk = p.shape[axis] * inner;
            out.extend_from_slice(&p.data[o * chunk..(o + 1) * chunk]);
        }
    }
    let mut shape = first.shape.clone();
    shape[axis] = total;
    Some(NdArray { shape, data: out })
}

/// An element-wise binary operation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Binary {
    Add,
    Sub,
    Mul,
    Div,
    Max,
    Min,
}

/// Applies `op` with NumPy broadcasting: shapes are aligned at the right and each pair of dimensions must be equal or
/// have a 1. `None` when they are not compatible.
pub fn broadcast(a: &NdArray, b: &NdArray, op: Binary) -> Option<NdArray> {
    let r = a.shape.len().max(b.shape.len());
    let dim = |s: &[usize], d: usize| if d + s.len() >= r { s[d + s.len() - r] } else { 1 };
    let mut shape = Vec::with_capacity(r);
    for d in 0..r {
        let (x, y) = (dim(&a.shape, d), dim(&b.shape, d));
        shape.push(if x == y {
            x
        } else if x == 1 {
            y
        } else if y == 1 {
            x
        } else {
            return None;
        });
    }
    let stride_for = |s: &[usize]| -> Vec<usize> {
        let st = strides(s);
        (0..r)
            .map(|d| {
                if d + s.len() >= r {
                    let sd = d + s.len() - r;
                    if s[sd] == 1 {
                        0
                    } else {
                        st[sd]
                    }
                } else {
                    0
                }
            })
            .collect()
    };
    let (sa, sb) = (stride_for(&a.shape), stride_for(&b.shape));
    let n = count(&shape)?;
    let mut out = Vec::with_capacity(n);
    let mut idx = vec![0usize; r];
    for _ in 0..n {
        let oa: usize = idx.iter().zip(&sa).map(|(i, s)| i * s).sum();
        let ob: usize = idx.iter().zip(&sb).map(|(i, s)| i * s).sum();
        let (x, y) = (a.data[oa], b.data[ob]);
        out.push(match op {
            Binary::Add => x + y,
            Binary::Sub => x - y,
            Binary::Mul => x * y,
            Binary::Div => x / y,
            Binary::Max => x.max(y),
            Binary::Min => x.min(y),
        });
        for d in (0..r).rev() {
            idx[d] += 1;
            if idx[d] < shape[d] {
                break;
            }
            idx[d] = 0;
        }
    }
    Some(NdArray { shape, data: out })
}

/// General matrix multiply: `alpha * op(A) * op(B) + beta * C`. `op(A)` is `m x k` and `op(B)` is `k x n`; with
/// `trans_a` the stored `a` is `k x m` (row-major), with `trans_b` the stored `b` is `n x k`. `c` (`m x n`) is used only
/// when `beta` is not 0. `None` for wrong slice lengths.
#[allow(clippy::too_many_arguments)]
pub fn gemm(a: &[f32], b: &[f32], m: usize, k: usize, n: usize, trans_a: bool, trans_b: bool, alpha: f32, beta: f32, c: Option<&[f32]>) -> Option<Vec<f32>> {
    if a.len() != m.checked_mul(k)? || b.len() != k.checked_mul(n)? {
        return None;
    }
    let mn = m.checked_mul(n)?;
    if beta != 0.0 && c.is_some_and(|c| c.len() != mn) {
        return None;
    }
    let mut out = vec![0.0f32; mn];
    for i in 0..m {
        for j in 0..n {
            let mut sum = 0.0f32;
            for p in 0..k {
                let av = if trans_a { a[p * m + i] } else { a[i * k + p] };
                let bv = if trans_b { b[j * k + p] } else { b[p * n + j] };
                sum += av * bv;
            }
            let prior = if beta != 0.0 { c.map_or(0.0, |c| c[i * n + j]) } else { 0.0 };
            out[i * n + j] = alpha * sum + beta * prior;
        }
    }
    Some(out)
}

/// Matrix-vector product `A x` for a row-major `m x n` matrix.
pub fn matvec(a: &[f32], m: usize, n: usize, x: &[f32]) -> Option<Vec<f32>> {
    if x.len() != n {
        return None;
    }
    gemm(a, x, m, n, 1, false, false, 1.0, 0.0, None)
}

/// Solves `A X = B` for a triangular `n x n` matrix `A` (the lower or upper triangle is read, the other ignored) and
/// `B` of `n x nrhs`, by forward or back substitution. With `unit_diag` the diagonal is taken as 1. `None` for wrong
/// lengths or a zero diagonal entry.
pub fn solve_triangular(a: &[f32], n: usize, b: &[f32], nrhs: usize, lower: bool, unit_diag: bool) -> Option<Vec<f32>> {
    if a.len() != n.checked_mul(n)? || b.len() != n.checked_mul(nrhs)? {
        return None;
    }
    let mut x = b.to_vec();
    for col in 0..nrhs {
        for step in 0..n {
            let i = if lower { step } else { n - 1 - step };
            let mut v = x[i * nrhs + col];
            let range: Vec<usize> = if lower { (0..i).collect() } else { (i + 1..n).collect() };
            for j in range {
                v -= a[i * n + j] * x[j * nrhs + col];
            }
            if !unit_diag {
                let d = a[i * n + i];
                if d == 0.0 {
                    return None;
                }
                v /= d;
            }
            x[i * nrhs + col] = v;
        }
    }
    Some(x)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arr(shape: &[usize], data: &[f32]) -> NdArray {
        NdArray::new(shape, data.to_vec()).unwrap()
    }

    #[test]
    fn construction_checks_the_size() {
        assert!(NdArray::new(&[2, 3], vec![0.0; 5]).is_none());
        assert_eq!(NdArray::zeros(&[2, 2]).unwrap().len(), 4);
        assert_eq!(NdArray::new(&[], vec![7.0]).unwrap().rank(), 0);
        assert!(NdArray::zeros(&[0, 3]).unwrap().is_empty());
    }

    #[test]
    fn permute_transposes_and_reorders() {
        let a = arr(&[2, 3], &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        let t = permute(&a, &[1, 0]).unwrap();
        assert_eq!(t, arr(&[3, 2], &[1.0, 4.0, 2.0, 5.0, 3.0, 6.0]));
        let c = arr(&[2, 2, 2], &[0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0]);
        let p = permute(&c, &[2, 0, 1]).unwrap();
        assert_eq!(p.shape, vec![2, 2, 2]);
        assert_eq!(p.data, vec![0.0, 2.0, 4.0, 6.0, 1.0, 3.0, 5.0, 7.0]);
        assert_eq!(permute(&p, &[1, 2, 0]).unwrap(), c);
        assert!(permute(&a, &[0, 0]).is_none());
        assert!(permute(&a, &[0]).is_none());
        assert!(permute(&a, &[0, 2]).is_none());
    }

    #[test]
    fn reductions_remove_the_axis() {
        let a = arr(&[2, 3], &[1.0, 5.0, 3.0, 4.0, 2.0, 6.0]);
        assert_eq!(reduce(&a, 0, Reduce::Sum).unwrap(), arr(&[3], &[5.0, 7.0, 9.0]));
        assert_eq!(reduce(&a, 1, Reduce::Sum).unwrap(), arr(&[2], &[9.0, 12.0]));
        assert_eq!(reduce(&a, 1, Reduce::Mean).unwrap(), arr(&[2], &[3.0, 4.0]));
        assert_eq!(reduce(&a, 1, Reduce::Max).unwrap(), arr(&[2], &[5.0, 6.0]));
        assert_eq!(reduce(&a, 0, Reduce::Min).unwrap(), arr(&[3], &[1.0, 2.0, 3.0]));
        assert_eq!(reduce(&a, 1, Reduce::Prod).unwrap(), arr(&[2], &[15.0, 48.0]));
        assert!(reduce(&a, 2, Reduce::Sum).is_none());
        assert!(reduce(&NdArray::zeros(&[2, 0]).unwrap(), 1, Reduce::Max).is_none());
        assert_eq!(reduce(&NdArray::zeros(&[2, 0]).unwrap(), 1, Reduce::Sum).unwrap(), arr(&[2], &[0.0, 0.0]));
    }

    #[test]
    fn argmax_and_cumsum() {
        let a = arr(&[2, 3], &[1.0, 5.0, 5.0, 9.0, 2.0, 6.0]);
        assert_eq!(argmax(&a, 1).unwrap(), arr(&[2], &[1.0, 0.0]));
        assert_eq!(argmax(&a, 0).unwrap(), arr(&[3], &[1.0, 0.0, 1.0]));
        assert_eq!(cumsum(&a, 1, false).unwrap(), arr(&[2, 3], &[1.0, 6.0, 11.0, 9.0, 11.0, 17.0]));
        assert_eq!(cumsum(&a, 1, true).unwrap(), arr(&[2, 3], &[11.0, 10.0, 5.0, 17.0, 8.0, 6.0]));
        assert_eq!(cumsum(&a, 0, false).unwrap(), arr(&[2, 3], &[1.0, 5.0, 5.0, 10.0, 7.0, 11.0]));
    }

    #[test]
    fn gather_slice_and_concat() {
        let a = arr(&[2, 3], &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        assert_eq!(gather(&a, 1, &[2, 0, 0]).unwrap(), arr(&[2, 3], &[3.0, 1.0, 1.0, 6.0, 4.0, 4.0]));
        assert_eq!(gather(&a, 0, &[1]).unwrap(), arr(&[1, 3], &[4.0, 5.0, 6.0]));
        assert!(gather(&a, 1, &[3]).is_none());
        assert_eq!(slice(&a, 1, 1, 3).unwrap(), arr(&[2, 2], &[2.0, 3.0, 5.0, 6.0]));
        assert!(slice(&a, 1, 2, 1).is_none());
        assert!(slice(&a, 1, 0, 4).is_none());
        let b = arr(&[2, 1], &[10.0, 20.0]);
        assert_eq!(concat(&[&a, &b], 1).unwrap(), arr(&[2, 4], &[1.0, 2.0, 3.0, 10.0, 4.0, 5.0, 6.0, 20.0]));
        assert_eq!(concat(&[&a, &a], 0).unwrap().shape, vec![4, 3]);
        assert!(concat(&[&a, &b], 0).is_none());
        assert!(concat(&[], 0).is_none());
        // Splitting and joining round-trips.
        let left = slice(&a, 1, 0, 1).unwrap();
        let right = slice(&a, 1, 1, 3).unwrap();
        assert_eq!(concat(&[&left, &right], 1).unwrap(), a);
    }

    #[test]
    fn broadcasting_follows_numpy() {
        let col = arr(&[3, 1], &[1.0, 2.0, 3.0]);
        let row = arr(&[4], &[10.0, 20.0, 30.0, 40.0]);
        let s = broadcast(&col, &row, Binary::Add).unwrap();
        assert_eq!(s.shape, vec![3, 4]);
        assert_eq!(s.data[0..4], [11.0, 21.0, 31.0, 41.0]);
        assert_eq!(s.data[8..12], [13.0, 23.0, 33.0, 43.0]);
        let scalar = arr(&[], &[2.0]);
        assert_eq!(broadcast(&col, &scalar, Binary::Mul).unwrap(), arr(&[3, 1], &[2.0, 4.0, 6.0]));
        assert_eq!(broadcast(&row, &row, Binary::Sub).unwrap().data, vec![0.0; 4]);
        assert_eq!(broadcast(&col, &col, Binary::Max).unwrap(), col);
        assert!(broadcast(&arr(&[3], &[1.0, 2.0, 3.0]), &row, Binary::Add).is_none());
        let div = broadcast(&arr(&[2], &[1.0, 2.0]), &arr(&[1], &[4.0]), Binary::Div).unwrap();
        assert_eq!(div.data, vec![0.25, 0.5]);
    }

    #[test]
    fn gemm_with_transposes_and_scaling() {
        let a = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0]; // 2x3
        let b = [7.0, 8.0, 9.0, 10.0, 11.0, 12.0]; // 3x2
        let c = gemm(&a, &b, 2, 3, 2, false, false, 1.0, 0.0, None).unwrap();
        assert_eq!(c, vec![58.0, 64.0, 139.0, 154.0]);
        // The same product with A and B stored transposed.
        let at = [1.0, 4.0, 2.0, 5.0, 3.0, 6.0];
        let bt = [7.0, 9.0, 11.0, 8.0, 10.0, 12.0];
        assert_eq!(gemm(&at, &bt, 2, 3, 2, true, true, 1.0, 0.0, None).unwrap(), c);
        let prior = [1.0, 1.0, 1.0, 1.0];
        let scaled = gemm(&a, &b, 2, 3, 2, false, false, 2.0, 0.5, Some(&prior)).unwrap();
        assert_eq!(scaled, vec![116.5, 128.5, 278.5, 308.5]);
        assert!(gemm(&a, &b, 2, 3, 3, false, false, 1.0, 0.0, None).is_none());
        assert!(gemm(&a, &b, 2, 3, 2, false, false, 1.0, 1.0, Some(&prior[..3])).is_none());
    }

    #[test]
    fn matvec_multiplies() {
        assert_eq!(matvec(&[1.0, 2.0, 3.0, 4.0], 2, 2, &[1.0, 1.0]).unwrap(), vec![3.0, 7.0]);
        assert!(matvec(&[1.0, 2.0, 3.0, 4.0], 2, 2, &[1.0]).is_none());
    }

    #[test]
    fn triangular_solves() {
        let l = [2.0, 99.0, 1.0, 3.0];
        let x = solve_triangular(&l, 2, &[2.0, 7.0], 1, true, false).unwrap();
        assert_eq!(x, vec![1.0, 2.0]);
        let u = [2.0, 1.0, 99.0, 3.0];
        let x = solve_triangular(&u, 2, &[4.0, 6.0], 1, false, false).unwrap();
        assert_eq!(x, vec![1.0, 2.0]);
        // Unit diagonal ignores the stored diagonal; two right-hand sides.
        let x = solve_triangular(&[5.0, 0.0, 2.0, 5.0], 2, &[1.0, 2.0, 5.0, 6.0], 2, true, true).unwrap();
        assert_eq!(x, vec![1.0, 2.0, 3.0, 2.0]);
        assert!(solve_triangular(&[0.0, 0.0, 0.0, 1.0], 2, &[1.0, 1.0], 1, true, false).is_none());
        assert!(solve_triangular(&l, 2, &[1.0], 1, true, false).is_none());
    }
}
