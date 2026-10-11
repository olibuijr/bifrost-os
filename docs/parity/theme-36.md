# Theme 36: vectors, matrices, quaternions and simd

Scope: Accelerate's vector arithmetic, transcendental functions, numerical quadrature and dense linear algebra, plus the
existing vector, matrix and quaternion types. Pure Rust, std only, in `springboard/ui/src/` (`bifrost_ui`).
No build or test run was done in this round (the theme rules forbid it), so the new tests are unverified.

## What was built

| Module | Mirrors (official names) | What it does |
|---|---|---|
| `springboard/ui/src/vector_ops.rs` (new) | vDSP vector-vector and vector-scalar operations, strided access, decimation, linear interpolation, amplitude to decibels, clipping; vForce element-wise transcendental functions | `add`, `subtract`, `multiply`, `divide`, `multiply_add` (`scale * a + b`), `add_scalar`, `multiply_scalar`, `dot`, `sum`, `mean`, `min_max`, `absolute`, `clip`, `strided`, `decimate`, `fill_gaps` (linear interpolation over NaN runs), `amplitude_to_decibels`, and `apply` over a `Function` enum (exp, log, log10, trig, inverse trig, hyperbolic, sqrt, rounding, reciprocal). Length mismatches are refused, never truncated. |
| `springboard/ui/src/quadrature.rs` (new) | Quadrature (`integrate(over:integrand:)`, absolute and relative tolerance, estimated absolute error) | Adaptive Simpson rule over finite intervals. Infinite ends are mapped onto a finite interval (`x = a + t/(1-t)`, and reflection for a lower bound of negative infinity). Returns `Integral { value, estimated_error }` or `NotConverged` with the best estimate. |
| `springboard/ui/src/lu.rs` (new) | Solving systems of linear equations (dense core of BLAS/LAPACK-style solvers) | LU factorisation with partial pivoting (`Lu::factor`), `solve`, `determinant`, and a one-call `solve`. Singular systems and shape mismatches are reported. Dense only. |
| `springboard/ui/src/linalg.rs` (existing, wave 1) | Working with Vectors, Working with Matrices, Working with Quaternions | `Vector3`, `Vector4`, `Matrix4` (column-major, perspective, look-at, inverse), `Quaternion` (slerp, rotate, to_matrix4). Not changed in this round. |

Known limits: no SIMD vector types (simd), no sparse storage or iterative solvers, no FFT, DFT or DCT, no vector
integrand for quadrature, and no CMYK or planar pixel conversion. The vForce-style functions are scalar loops over slices.

## Official API evidence

- Accelerate landing page, Vectors, Matrices, and Quaternions topics (simd, vForce): https://developer.apple.com/documentation/accelerate
  - REA evidence: `ev_ae742bb7fdf9ed378db7e14ffc27706ffe1617dc91f3e737a7061f2c3bff1c92`
- vDSP (type methods: add, multiply, absolute, clip, convert amplitude to decibels): https://developer.apple.com/documentation/accelerate/vdsp
  - REA evidence: `ev_0fc23bf31027f79d8fd50e9e75e7ca321daa7c8929b48951206d3f94ffdb4b4e`
  - DocC outline also lists "Controlling vDSP operations with stride", "Using linear interpolation to construct new data points" and "Resampling a signal with decimation".
- Quadrature (initialiser with integrator and tolerances, `integrate(over:integrand:)` returning integral result and estimated absolute error): https://developer.apple.com/documentation/accelerate/quadrature
  - REA evidence: `ev_6919372ce0559ef001981f6c8b56184b7c9e824d56748783b6be1ee62d5878f3`
- vForce (element-wise acos, asin, atan, ceil, exp and others): https://developer.apple.com/documentation/accelerate/vforce
  - DocC only; no REA capture was taken for this page.
- Linear solver topic: the DocC path `accelerate/solving-systems-of-linear-equations` returned 404, so the solver is cited
  to the Accelerate landing page only.

## Matrix rows closed

| Framework slug | Feature (as in data JSON) | New status | BifrOSt path |
|---|---|---|---|
| accelerate | vDSP vector arithmetic and stride operations | partial | springboard/ui/src/vector_ops.rs |
| accelerate | Numerical quadrature | partial | springboard/ui/src/quadrature.rs |
| accelerate | Linear algebra (BLAS, LAPACK, sparse solvers) | partial | springboard/ui/src/lu.rs |
| accelerate | Vectors, matrices, quaternions and simd | partial | springboard/ui/src/linalg.rs, springboard/ui/src/vector_ops.rs |

## Already built before this round (matrix still says gap)

These modules exist in the tree from wave 1. They were not changed here, and the matrix data should be updated by the lead:

| Framework slug | Feature | Existing path |
|---|---|---|
| core-ml | Tensors | springboard/ui/src/ml_tensor.rs (row-major f32 tensor, reshape, range slicing, pointwise min and max) |
| evaluations | Metrics, evaluators and aggregation | springboard/ui/src/evaluation.rs (Metric, Evaluator, MetricsAggregator, Summary) |

## Tests

Each new module has a `#[cfg(test)] mod tests` with at least five behaviour tests:
- `vector_ops.rs`: 8 tests (binary ops, length mismatch, reductions, clip and abs, strided and decimation, gap filling, decibels, element-wise functions).
- `quadrature.rs`: 9 tests (polynomial, sine, exponential tail, Gaussian over the whole line, lower infinite bound, bad bounds, bad tolerance, non-finite integrand, empty interval).
- `lu.rs`: 6 tests (3x3 solve, pivoting, determinant sign, singular, shape mismatch, identity).

Not run: no cargo or test command was executed, per the theme rules.

## Checks

- `python3 tools/check-names.py`: exit 0 (no hits).
- `springboard/ui/src/lib.rs`: one `pub mod` line added for each of `vector_ops`, `quadrature` and `lu`.
