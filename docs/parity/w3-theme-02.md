# Parity wave 3, theme 02: shader functions and image kernels

Theme "Shader functions and libraries" (40 gap rows across 12 frameworks). This round builds the CPU image-kernel
part of the theme: the reference image-processing kernels (convolution, median, gradients, Laplacian, reductions,
integral images, statistics and element-wise arithmetic). The GPU shader-library part (compiling and loading
app-supplied shader functions) is not built here; no GPU or hardware access was used, and nothing was compiled or run.

## What was built

| Module | What it does |
|---|---|
| `springboard/render/src/filter.rs` (extended) | `convolve` (odd kernel, bias, edge repeat, clamped output), `median` (odd diameter), `sobel` (`Gradient::Horizontal` / `Vertical`, positive part kept), `laplacian` (3 by 3, centre 4, edges -1, plus bias). Sits next to the existing threshold, histogram, equalization, erode and dilate code. |
| `springboard/render/src/plane_stats.rs` (new) | `reduce_rows` / `reduce_columns` with `Reduce::{Min, Max, Sum, Mean}` returning `f32` per line; `Integral` (summed-area table, `new`, `of_squares`, `rect_sum` with clipping); `statistics` (exact integer mean and population variance, min and max, clipped region, `None` when empty). |
| `springboard/render/src/plane_arith.rs` (new) | `combine` with `Op::{Add, Subtract, Multiply, Divide}` on two equal-size planes (fractions of 255, rounded, clamped; divide by zero gives 255); `transpose`. |

Each new module has `#[cfg(test)] mod tests`. `filter.rs` gained eight tests in its existing test module.
`springboard/render/src/lib.rs` gained two `pub mod` lines (`plane_stats`, `plane_arith`).

## Official API names mirrored

Apple reference pages (DocC JSON, read with `tools/parity/docc.py`):

- `MPSImageConvolution` (`init(device:kernelWidth:kernelHeight:weights:)`, `bias`), `MPSImageMedian` (`kernelDiameter`),
  `MPSImageSobel`, `MPSImageLaplacian` (`bias`), `MPSImageIntegral`, `MPSImageStatisticsMeanAndVariance`,
  `MPSImageReduceRowMax` (and the other row and column reductions), `MPSImageArithmetic` (`primaryScale`,
  `secondaryScale`, `bias`, `minimumValue`, `maximumValue`), `MPSImageAdd`, `MPSImageTranspose`.

Only the operations are mirrored. Apple's scale, bias and minimum or maximum properties on the arithmetic base class,
the storage format, the offset and clip-rectangle properties, and the GPU encode path are not modelled. The Sobel
output convention (signed, clamped) and the Laplacian coefficients are this module's choice; the official page
gives only the abstract.

## REA evidence

- `ev_196beed97724378c78d753ee0e60ea4d33f60e265294946b478112c035903514`: rendered page
  `https://developer.apple.com/documentation/metalperformanceshaders` (title "Metal Performance Shaders | Apple
  Developer Documentation"). The framework page lists "Image Filters" and "Matrices and Vectors" topics that these
  kernels belong to.

## Matrix rows closed

Status is `partial` for each row: the operations exist as CPU models, with no app-facing API and no GPU path.

| Framework slug | Feature (as in `docs/parity-matrix/data/<slug>.json`) | New status | BifrOSt path |
|---|---|---|---|
| metal-performance-shaders | Convolution, median, Sobel and Laplacian filters | partial | springboard/render/src/filter.rs (`convolve`, `median`, `sobel`, `laplacian`) |
| metal-performance-shaders | Reduction filters (row and column min, max, sum, mean) | partial | springboard/render/src/plane_stats.rs (`reduce_rows`, `reduce_columns`) |
| metal-performance-shaders | Integral images and statistics (mean, variance, min and max) | partial | springboard/render/src/plane_stats.rs (`Integral`, `statistics`) |
| metal-performance-shaders | Image arithmetic and pixel conversion | partial | springboard/render/src/plane_arith.rs (`combine`, `transpose`); pixel-format conversion not built |

## Gaps left open

- No pixel-format conversion (the conversion half of the arithmetic row). Planes are 8-bit single channel only.
- No GPU or app-facing shader API: the shader-library rows (`metal`, `scenekit`, `glkit`) and the GPU kernel path stay gaps.
- Reductions return `f32` per line, not a stored image, so the storage-format clamp is not modelled.
- Euclidean distance transform, guided filter and keypoints (`metal-performance-shaders`) are not built.
- Statistics use the population variance; the official page does not state which variance it computes, so this is
  the implementation's choice.
- Nothing is wired into apps or the shell. Not built, compiled or run in this round.
