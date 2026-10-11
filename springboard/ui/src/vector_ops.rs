//! Vector arithmetic over slices of `f32`: element-wise and vector-scalar operations, dot products and reductions, strided
//! access, clipping, decimal-to-decibel amplitude conversion, linear gap filling, decimation, and element-wise
//! transcendental functions. Pure functions over borrowed slices; only the returned vectors allocate.
//! Binary operations require equal lengths and report a mismatch instead of truncating to the shorter operand.

/// Why a vector operation was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VectorError {
    /// The two operands do not have the same length.
    LengthMismatch { left: usize, right: usize },
    /// A stride, decimation factor or reference value is zero or negative, or a strided range runs past the data.
    InvalidArgument,
}

fn same_len(a: &[f32], b: &[f32]) -> Result<(), VectorError> {
    if a.len() == b.len() {
        Ok(())
    } else {
        Err(VectorError::LengthMismatch { left: a.len(), right: b.len() })
    }
}

fn zip_with(a: &[f32], b: &[f32], op: impl Fn(f32, f32) -> f32) -> Result<Vec<f32>, VectorError> {
    same_len(a, b)?;
    Ok(a.iter().zip(b).map(|(x, y)| op(*x, *y)).collect())
}

/// Element-wise sum of two vectors.
pub fn add(a: &[f32], b: &[f32]) -> Result<Vec<f32>, VectorError> {
    zip_with(a, b, |x, y| x + y)
}

/// Element-wise difference `a - b`.
pub fn subtract(a: &[f32], b: &[f32]) -> Result<Vec<f32>, VectorError> {
    zip_with(a, b, |x, y| x - y)
}

/// Element-wise product of two vectors.
pub fn multiply(a: &[f32], b: &[f32]) -> Result<Vec<f32>, VectorError> {
    zip_with(a, b, |x, y| x * y)
}

/// Element-wise quotient `a / b`. A zero divisor yields the IEEE infinity or NaN, as the operator does.
pub fn divide(a: &[f32], b: &[f32]) -> Result<Vec<f32>, VectorError> {
    zip_with(a, b, |x, y| x / y)
}

/// Adds a scalar to every element.
pub fn add_scalar(a: &[f32], s: f32) -> Vec<f32> {
    a.iter().map(|x| x + s).collect()
}

/// Multiplies every element by a scalar.
pub fn multiply_scalar(a: &[f32], s: f32) -> Vec<f32> {
    a.iter().map(|x| x * s).collect()
}

/// Computes `scale * a + b` element-wise, the fused form used for accumulation.
pub fn multiply_add(a: &[f32], scale: f32, b: &[f32]) -> Result<Vec<f32>, VectorError> {
    zip_with(a, b, |x, y| scale * x + y)
}

/// Dot product of two vectors.
pub fn dot(a: &[f32], b: &[f32]) -> Result<f32, VectorError> {
    same_len(a, b)?;
    Ok(a.iter().zip(b).map(|(x, y)| x * y).sum())
}

/// Sum of all elements. An empty vector sums to zero.
pub fn sum(a: &[f32]) -> f32 {
    a.iter().sum()
}

/// Arithmetic mean, or `None` for an empty vector.
pub fn mean(a: &[f32]) -> Option<f32> {
    if a.is_empty() {
        None
    } else {
        Some(sum(a) / a.len() as f32)
    }
}

/// Smallest and largest element, or `None` for an empty vector. NaN elements are skipped.
pub fn min_max(a: &[f32]) -> Option<(f32, f32)> {
    a.iter().copied().filter(|x| !x.is_nan()).fold(None, |acc, x| match acc {
        None => Some((x, x)),
        Some((lo, hi)) => Some((lo.min(x), hi.max(x))),
    })
}

/// Absolute value of every element.
pub fn absolute(a: &[f32]) -> Vec<f32> {
    a.iter().map(|x| x.abs()).collect()
}

/// Clamps every element into `[low, high]`. If `low` is greater than `high`, every element becomes `high`.
pub fn clip(a: &[f32], low: f32, high: f32) -> Vec<f32> {
    a.iter().map(|x| x.max(low).min(high)).collect()
}

/// Reads `count` elements starting at `start`, `start + stride`, `start + 2 * stride` and so on.
/// Fails when the stride is zero or the last element falls outside the vector.
pub fn strided(a: &[f32], start: usize, stride: usize, count: usize) -> Result<Vec<f32>, VectorError> {
    if stride == 0 {
        return Err(VectorError::InvalidArgument);
    }
    if count == 0 {
        return Ok(Vec::new());
    }
    let last = count
        .checked_sub(1)
        .and_then(|n| n.checked_mul(stride))
        .and_then(|span| span.checked_add(start))
        .ok_or(VectorError::InvalidArgument)?;
    if last >= a.len() {
        return Err(VectorError::InvalidArgument);
    }
    Ok((0..count).map(|i| a[start + i * stride]).collect())
}

/// Keeps every `factor`-th element, starting with the first. This does not filter the signal first:
/// callers that need an antialiasing filter apply it before decimating.
pub fn decimate(a: &[f32], factor: usize) -> Result<Vec<f32>, VectorError> {
    if factor == 0 {
        return Err(VectorError::InvalidArgument);
    }
    Ok(a.iter().step_by(factor).copied().collect())
}

/// Replaces each run of NaN values with points on the straight line between its neighbours. A run at either end
/// of the vector has no neighbour on one side and is left as NaN.
pub fn fill_gaps(a: &[f32]) -> Vec<f32> {
    let mut out = a.to_vec();
    let mut i = 0;
    while i < out.len() {
        if !out[i].is_nan() {
            i += 1;
            continue;
        }
        let start = i;
        while i < out.len() && out[i].is_nan() {
            i += 1;
        }
        if start == 0 || i == out.len() {
            continue;
        }
        let (left, right) = (out[start - 1], out[i]);
        let steps = (i - start + 1) as f32;
        for k in start..i {
            let t = (k - start + 1) as f32 / steps;
            out[k] = left + (right - left) * t;
        }
    }
    out
}

/// Converts amplitudes to decibels relative to `zero_reference`: `20 * log10(|x| / reference)`.
/// A zero amplitude gives negative infinity. A reference that is not positive is refused.
pub fn amplitude_to_decibels(a: &[f32], zero_reference: f32) -> Result<Vec<f32>, VectorError> {
    if !(zero_reference > 0.0) {
        return Err(VectorError::InvalidArgument);
    }
    Ok(a.iter().map(|x| 20.0 * (x.abs() / zero_reference).log10()).collect())
}

/// An element-wise transcendental or rounding function applied by [`apply`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Function {
    Abs,
    Ceil,
    Floor,
    Sqrt,
    Reciprocal,
    Exp,
    Log,
    Log10,
    Sin,
    Cos,
    Tan,
    Asin,
    Acos,
    Atan,
    Sinh,
    Cosh,
    Tanh,
}

impl Function {
    /// The function's value at `x`.
    pub fn eval(self, x: f32) -> f32 {
        match self {
            Function::Abs => x.abs(),
            Function::Ceil => x.ceil(),
            Function::Floor => x.floor(),
            Function::Sqrt => x.sqrt(),
            Function::Reciprocal => 1.0 / x,
            Function::Exp => x.exp(),
            Function::Log => x.ln(),
            Function::Log10 => x.log10(),
            Function::Sin => x.sin(),
            Function::Cos => x.cos(),
            Function::Tan => x.tan(),
            Function::Asin => x.asin(),
            Function::Acos => x.acos(),
            Function::Atan => x.atan(),
            Function::Sinh => x.sinh(),
            Function::Cosh => x.cosh(),
            Function::Tanh => x.tanh(),
        }
    }
}

/// Applies `function` to every element. Out-of-domain inputs give NaN or infinity, as the scalar function does.
pub fn apply(a: &[f32], function: Function) -> Vec<f32> {
    a.iter().map(|x| function.eval(*x)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_operations_work_element_by_element() {
        let a = [1.0, 2.0, 3.0];
        let b = [4.0, 5.0, 6.0];
        assert_eq!(add(&a, &b).unwrap(), vec![5.0, 7.0, 9.0]);
        assert_eq!(subtract(&a, &b).unwrap(), vec![-3.0, -3.0, -3.0]);
        assert_eq!(multiply(&a, &b).unwrap(), vec![4.0, 10.0, 18.0]);
        assert_eq!(divide(&b, &a).unwrap(), vec![4.0, 2.5, 2.0]);
        assert_eq!(multiply_add(&a, 2.0, &b).unwrap(), vec![6.0, 9.0, 12.0]);
    }

    #[test]
    fn mismatched_lengths_are_refused_not_truncated() {
        let a = [1.0, 2.0];
        let b = [1.0];
        assert_eq!(add(&a, &b), Err(VectorError::LengthMismatch { left: 2, right: 1 }));
        assert_eq!(dot(&a, &b), Err(VectorError::LengthMismatch { left: 2, right: 1 }));
    }

    #[test]
    fn reductions_and_dot_product() {
        assert_eq!(dot(&[1.0, 2.0, 3.0], &[4.0, 5.0, 6.0]).unwrap(), 32.0);
        assert_eq!(sum(&[1.5, 2.5]), 4.0);
        assert_eq!(mean(&[1.0, 2.0, 6.0]), Some(3.0));
        assert_eq!(mean(&[]), None);
        assert_eq!(min_max(&[3.0, f32::NAN, -1.0, 7.0]), Some((-1.0, 7.0)));
        assert_eq!(min_max(&[]), None);
    }

    #[test]
    fn clipping_and_absolute_value() {
        assert_eq!(clip(&[-5.0, 0.5, 9.0], 0.0, 1.0), vec![0.0, 0.5, 1.0]);
        assert_eq!(clip(&[2.0], 3.0, 1.0), vec![1.0], "an inverted range clamps to its upper bound");
        assert_eq!(absolute(&[-2.0, 3.0]), vec![2.0, 3.0]);
    }

    #[test]
    fn strided_access_and_decimation() {
        let a: Vec<f32> = (0..10).map(|i| i as f32).collect();
        assert_eq!(strided(&a, 1, 3, 3).unwrap(), vec![1.0, 4.0, 7.0]);
        assert_eq!(strided(&a, 0, 0, 1), Err(VectorError::InvalidArgument));
        assert_eq!(strided(&a, 5, 3, 3), Err(VectorError::InvalidArgument), "last index 11 is past the end");
        assert_eq!(decimate(&a, 4).unwrap(), vec![0.0, 4.0, 8.0]);
        assert_eq!(decimate(&a, 0), Err(VectorError::InvalidArgument));
    }

    #[test]
    fn gaps_are_filled_linearly_and_edges_are_kept() {
        let filled = fill_gaps(&[f32::NAN, 0.0, f32::NAN, f32::NAN, 3.0, f32::NAN]);
        assert!(filled[0].is_nan(), "a leading gap has no left neighbour");
        assert_eq!(&filled[1..5], &[0.0, 1.0, 2.0, 3.0]);
        assert!(filled[5].is_nan(), "a trailing gap has no right neighbour");
    }

    #[test]
    fn decibels_follow_the_amplitude_formula() {
        let db = amplitude_to_decibels(&[1.0, 0.1, 0.0], 1.0).unwrap();
        assert!((db[0] - 0.0).abs() < 1e-6);
        assert!((db[1] + 20.0).abs() < 1e-4);
        assert_eq!(db[2], f32::NEG_INFINITY);
        assert_eq!(amplitude_to_decibels(&[1.0], 0.0), Err(VectorError::InvalidArgument));
    }

    #[test]
    fn elementwise_functions_match_the_scalar_functions() {
        let out = apply(&[0.0, 1.0], Function::Exp);
        assert!((out[0] - 1.0).abs() < 1e-6 && (out[1] - std::f32::consts::E).abs() < 1e-5);
        assert_eq!(apply(&[-1.5, 2.5], Function::Floor), vec![-2.0, 2.0]);
        assert!(apply(&[-1.0], Function::Sqrt)[0].is_nan());
        assert_eq!(apply(&[4.0], Function::Reciprocal), vec![0.25]);
    }
}
