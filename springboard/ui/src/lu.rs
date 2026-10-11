//! Dense linear systems: LU factorisation with partial pivoting, then solves and determinants from the factors.
//! Matrices are square and row-major (`a[row * n + column]`) and work in `f64`. This is the dense core only: no sparse
//! storage, no iterative solvers, and no least-squares or eigenvalue routines.

/// Why a factorisation or solve was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LuError {
    /// The matrix does not hold `n * n` values, or the right-hand side does not hold `n` values.
    DimensionMismatch,
    /// No pivot is large enough: the matrix is singular to working precision.
    Singular,
}

/// An LU factorisation with row pivoting: `P * A = L * U`, with `L` unit lower triangular.
#[derive(Clone, Debug, PartialEq)]
pub struct Lu {
    n: usize,
    /// Packed factors: the strict lower triangle holds `L` (its unit diagonal is implicit), the upper triangle holds `U`.
    packed: Vec<f64>,
    /// `order[i]` is the original row that ended up in row `i` of the factors.
    order: Vec<usize>,
    /// `+1` or `-1`: the parity of the row swaps, which the determinant needs.
    sign: f64,
}

impl Lu {
    /// Factorises the `n x n` row-major matrix `a`. Fails when a column has no usable pivot.
    pub fn factor(a: &[f64], n: usize) -> Result<Lu, LuError> {
        if a.len() != n.checked_mul(n).ok_or(LuError::DimensionMismatch)? {
            return Err(LuError::DimensionMismatch);
        }
        let mut packed = a.to_vec();
        let mut order: Vec<usize> = (0..n).collect();
        let mut sign = 1.0;
        let scale = packed.iter().fold(0.0_f64, |m, v| m.max(v.abs()));
        if n > 0 && (!scale.is_finite() || scale == 0.0) {
            return Err(LuError::Singular);
        }
        let tolerance = scale * n as f64 * f64::EPSILON;
        for k in 0..n {
            let mut pivot_row = k;
            for i in (k + 1)..n {
                if packed[i * n + k].abs() > packed[pivot_row * n + k].abs() {
                    pivot_row = i;
                }
            }
            let pivot = packed[pivot_row * n + k];
            if !(pivot.abs() > tolerance) {
                return Err(LuError::Singular);
            }
            if pivot_row != k {
                for j in 0..n {
                    packed.swap(k * n + j, pivot_row * n + j);
                }
                order.swap(k, pivot_row);
                sign = -sign;
            }
            let pivot = packed[k * n + k];
            for i in (k + 1)..n {
                let factor = packed[i * n + k] / pivot;
                packed[i * n + k] = factor;
                for j in (k + 1)..n {
                    packed[i * n + j] -= factor * packed[k * n + j];
                }
            }
        }
        Ok(Lu { n, packed, order, sign })
    }

    /// Solves `A x = b` for `x`, using the stored factors.
    pub fn solve(&self, b: &[f64]) -> Result<Vec<f64>, LuError> {
        let n = self.n;
        if b.len() != n {
            return Err(LuError::DimensionMismatch);
        }
        // Forward substitution with the permuted right-hand side: L y = P b.
        let mut y: Vec<f64> = self.order.iter().map(|&row| b[row]).collect();
        for i in 0..n {
            for j in 0..i {
                y[i] -= self.packed[i * n + j] * y[j];
            }
        }
        // Back substitution: U x = y.
        for i in (0..n).rev() {
            for j in (i + 1)..n {
                y[i] -= self.packed[i * n + j] * y[j];
            }
            y[i] /= self.packed[i * n + i];
        }
        Ok(y)
    }

    /// The determinant of the factored matrix: the product of `U`'s diagonal, negated for each row swap.
    pub fn determinant(&self) -> f64 {
        (0..self.n).fold(self.sign, |d, i| d * self.packed[i * self.n + i])
    }
}

/// Solves `a x = b` for a square `n x n` matrix in one call.
pub fn solve(a: &[f64], n: usize, b: &[f64]) -> Result<Vec<f64>, LuError> {
    Lu::factor(a, n)?.solve(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: &[f64], b: &[f64]) -> bool {
        a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-9)
    }

    #[test]
    fn solves_a_three_by_three_system() {
        // 2x + y - z = 8, -3x - y + 2z = -11, -2x + y + 2z = -3 has the solution (2, 3, -1).
        let a = [2.0, 1.0, -1.0, -3.0, -1.0, 2.0, -2.0, 1.0, 2.0];
        let x = solve(&a, 3, &[8.0, -11.0, -3.0]).unwrap();
        assert!(close(&x, &[2.0, 3.0, -1.0]), "got {x:?}");
    }

    #[test]
    fn a_zero_leading_pivot_is_handled_by_row_swapping() {
        let a = [0.0, 1.0, 1.0, 0.0];
        let x = solve(&a, 2, &[3.0, 5.0]).unwrap();
        assert!(close(&x, &[5.0, 3.0]), "got {x:?}");
    }

    #[test]
    fn determinant_tracks_row_swaps() {
        let lu = Lu::factor(&[0.0, 1.0, 1.0, 0.0], 2).unwrap();
        assert!((lu.determinant() + 1.0).abs() < 1e-12);
        let lu = Lu::factor(&[4.0, 3.0, 6.0, 3.0], 2).unwrap();
        assert!((lu.determinant() + 6.0).abs() < 1e-12);
    }

    #[test]
    fn a_singular_matrix_is_reported() {
        assert_eq!(solve(&[1.0, 2.0, 2.0, 4.0], 2, &[1.0, 1.0]), Err(LuError::Singular));
        assert_eq!(solve(&[0.0, 0.0, 0.0, 0.0], 2, &[1.0, 1.0]), Err(LuError::Singular));
    }

    #[test]
    fn mismatched_shapes_are_refused() {
        assert_eq!(solve(&[1.0, 2.0, 3.0], 2, &[1.0, 1.0]), Err(LuError::DimensionMismatch));
        assert_eq!(solve(&[1.0, 0.0, 0.0, 1.0], 2, &[1.0]), Err(LuError::DimensionMismatch));
    }

    #[test]
    fn the_identity_solves_to_the_right_hand_side() {
        let a = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
        let x = solve(&a, 3, &[7.0, -2.0, 0.5]).unwrap();
        assert!(close(&x, &[7.0, -2.0, 0.5]));
    }
}
