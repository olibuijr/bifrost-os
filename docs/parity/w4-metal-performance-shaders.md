# Wave 4: Metal Performance Shaders gap rows

Framework: Metal Performance Shaders (`metal-performance-shaders`), official page
https://developer.apple.com/documentation/metalperformanceshaders

Two gap rows are closed with pure Rust, std only, no device, no unsafe. Both are CPU models; neither is wired
into apps or the shell yet, and neither has been run on the VM or Midget.

## What was built

1. Dense decompositions in `springboard/ui/src/linalg.rs` (new section, extends the existing 3D math module).
   - `cholesky` factors a symmetric positive definite `n x n` matrix as `A = L Lᵀ`; `cholesky_solve` solves from
     the factor. Mirrors `MPSMatrixDecompositionCholesky`.
   - `lu` factors with partial pivoting (`P A = L U`) and `Lu::solve` solves from the factor. Mirrors
     `MPSMatrixDecompositionLU`.
   - Refusals are `DenseError::{Shape, NotPositiveDefinite, Singular}`.
   - Matrix multiplication and softmax were not added here: `springboard/ui/src/graph.rs` (`matmul`) and
     `springboard/ui/src/ml_model.rs` (`softmax`) already provide them as tensor operations.
2. Euclidean distance transform in `springboard/render/src/distance.rs` (`euclidean_distance`). Exact squared
   distances from the Felzenszwalb and Huttenlocher separable lower-envelope method, then a square root. Optional
   search limit. Mirrors `MPSImageEuclideanDistanceTransform` (`searchLimitRadius`).
   - The reference page says the limit is a search radius but does not state the value for a pixel outside it.
     BifrOSt returns `f32::INFINITY` there. This is a BifrOSt choice, not a confirmed Apple behaviour.
3. Guided filter in `springboard/render/src/guided.rs` (`guided_filter`). Single-call He et al. guided filter on
   `f32` planes with an odd `diameter` and `epsilon`, using summed-area box means that clamp at the border.
   Mirrors `MPSImageGuidedFilter`. The reference splits the work into `encodeRegression` and
   `encodeReconstruction`; BifrOSt runs both steps in one call.

Keypoint detection (`MPSImageFindKeypoints`) is not built, so the third feature of its row stays open.

## Official API names mirrored

- `MPSMatrixDecompositionCholesky` (https://developer.apple.com/documentation/metalperformanceshaders/mpsmatrixdecompositioncholesky)
- `MPSMatrixDecompositionLU` (https://developer.apple.com/documentation/metalperformanceshaders/mpsmatrixdecompositionlu)
- `MPSImageEuclideanDistanceTransform` (https://developer.apple.com/documentation/metalperformanceshaders/mpsimageeuclideandistancetransform)
- `MPSImageGuidedFilter` (https://developer.apple.com/documentation/metalperformanceshaders/mpsimageguidedfilter)
- Framework landing page: https://developer.apple.com/documentation/metalperformanceshaders

## REA evidence

- Framework landing page: `ev_1c0ebb65db931630886715955e21ff2188f76a60c3477bf269f4438c21d51010`
- `MPSMatrixDecompositionCholesky`: `ev_67b09fb474be96d77e1b29eb4ab6491a04aa93bbe313ca18f6c7d93e24729872`
- `MPSMatrixDecompositionLU`: `ev_a0e1961029cc3ee8f8e02f12a004f3761201cbca439ae7229b209277ca4316af`
- `MPSImageEuclideanDistanceTransform`: `ev_1ce3489e27cf5bacefecc7f32018372d6d8ddf03fb483d7347f2da5cfa181f89`
- `MPSImageGuidedFilter`: `ev_6ef13972cc01ac92036db8f300fb48ba8b4a13053d1e53975a3d29764c4f79fb`

## Matrix rows closed

| Framework slug | Feature | New status | BifrOSt path |
|---|---|---|---|
| metal-performance-shaders | Matrix and vector kernels (multiply, decomposition, solve, softmax) | partial | springboard/ui/src/linalg.rs (cholesky, cholesky_solve, lu, Lu::solve); multiply and softmax in springboard/ui/src/graph.rs and springboard/ui/src/ml_model.rs |
| metal-performance-shaders | Euclidean distance transform, guided filter, keypoints | partial | springboard/render/src/distance.rs (euclidean_distance); springboard/render/src/guided.rs (guided_filter); keypoints not built |

## Tests

Host unit tests were written by hand from the rules the code implements (no build was run in this wave):

- `springboard/ui/src/linalg.rs`: Cholesky factor of `[[4,2],[2,3]]`; Cholesky solve giving `x = 1.25, y = 1.5`;
  refusal of a non-positive-definite matrix; LU pivoting past a zero diagonal; LU solve of a 3x3 system; refusals
  for singular and misshapen input.
- `springboard/render/src/distance.rs`: row distances `[2, 1, 0]`; 3x3 centre seed with edge 1 and corner sqrt 2;
  diagonal seeds on 5x5; all-infinity without seeds; search limit cut-off; empty plane.
- `springboard/render/src/guided.rs`: zero radius returns the input; flat guide gives clamped window means
  `[2.25, 3, 3.75]`; constant input stays constant; even and zero diameter refused; mismatched sizes refused; clamped
  box mean `[3, 4, 5]`.
