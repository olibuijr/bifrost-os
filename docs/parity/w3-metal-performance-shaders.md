# Metal Performance Shaders: wave 3 (image filters, statistics, arithmetic)

Wave 3 closes three gap rows of the Metal Performance Shaders framework with pure Rust, std only, on the CPU.
The code is a host model: it is not wired into apps or the shell, and it has not been built, run or tested on
the VM or Midget yet. The official pages cited below describe the reference classes; BifrOSt mirrors their
operations, not their GPU execution.

## What was built

- `springboard/render/src/filter.rs` (extended from wave 1, same `Plane` type):
  - `convolve(src, kw, kh, weights, bias)`: a caller kernel with odd sides, applied without a flip, edge
    pixels repeated, sum plus bias rounded and clamped to 0..=255.
  - `laplacian(src, bias)`: the four-neighbour kernel `0 1 0 / 1 -4 1 / 0 1 0` built on `convolve`.
  - `sobel(src)`: gradient magnitude `sqrt(gx^2 + gy^2)` from the 3 by 3 Sobel kernels, clamped.
    Choice: the official page does not state the output encoding, so magnitude is our definition.
  - `median(src, diameter)`: median of an odd square window, edge pixels repeated.
  - `Arithmetic` and `arithmetic(a, b, op)`: element-wise Add, Subtract, Multiply and Divide on two planes of
    equal size. Values are read as fractions of 255: Multiply is `a * b / 255`, Divide is `a * 255 / b`
    (255 when `b` is 0), Add saturates at 255, Subtract at 0. Choice: the official pages do not give the
    integer rule, so the normalised-value reading is ours.
  - `transpose(src)`: pixel (x, y) moves to (y, x).
- `springboard/render/src/integral.rs` (new module, one `pub mod integral;` line in `render/src/lib.rs`):
  - `Integral::new(&Plane)`: summed-area tables for pixel sums and squared sums.
  - `Integral::rect_sum`, `rect_squares`, `mean_and_variance` (population variance): constant-time per rectangle.
  - `min_and_max(src, x, y, w, h)`: the smallest and largest pixel in a rectangle, scanned directly.

Every function returns `None` (or refuses) on bad input: even or zero kernel sides, a wrong weight count, a
rectangle outside the plane, an empty rectangle, or planes of different sizes. Tests live in
`#[cfg(test)] mod tests` in each file; expected values were worked out by hand from the rules above.

## Official APIs mirrored

| Official API (Metal Performance Shaders, macOS 10.13+) | BifrOSt function |
|---|---|
| MPSImageConvolution (init with kernelWidth, kernelHeight, weights, bias) | `filter::convolve` |
| MPSImageLaplacian (bias) | `filter::laplacian` |
| MPSImageSobel | `filter::sobel` |
| MPSImageMedian (kernelDiameter, odd) | `filter::median` |
| MPSImageIntegral, MPSImageIntegralOfSquares | `integral::Integral::rect_sum`, `rect_squares` |
| MPSImageStatisticsMeanAndVariance | `integral::Integral::mean_and_variance` |
| MPSImageStatisticsMinAndMax | `integral::min_and_max` |
| MPSImageAdd, MPSImageSubtract, MPSImageMultiply, MPSImageDivide | `filter::arithmetic` with `Arithmetic::{Add, Subtract, Multiply, Divide}` |
| MPSImageTranspose | `filter::transpose` |

Not mirrored: MPSImageConversion (colour space, alpha and pixel format), the GPU execution, the `MPSKernel`
base classes, the separable or pyramid variants, the sigma-based Gaussian, and any `MTLTexture` input.
MPSImageIntegral and MPSImageStatisticsMeanAndVariance write their results to destination images in Apple's
API; here the results are returned as values.

## REA evidence (rendered official pages)

- Framework landing page: `ev_3d1a6cb8557214e255b73ed10c5f1fa13429256c960daa66d5ef0070a1fcc0e3`
- MPSImageConvolution: `ev_c1a2c7db02d1e82b9311c67ef05c31015179c827a580adb1d5c4f8bff40fb3b4`
- MPSImageMedian: `ev_d2056e17162a125bfe0ee25ac6509882187bf6718c46473bc929b239071794e7`
- MPSImageSobel: `ev_c7633ffb6a8ea294d47ca9c8cda0b8d88443d10386196aadda0c7b0c3447a5c9`
- MPSImageLaplacian: `ev_5086ac34ff2b70685c012d164dc79ee62a7767bcd0151a0dafadf9f28595c02b` (rendered text
  is only the page header; the abstract was read with `docc.py`)
- MPSImageIntegral: `ev_097a4e9725bd2601cc0c6af96f048bf99b7d53f2ab06260ef5df43b77c55e9e1` (rendered text is
  page chrome; the abstract was read with `docc.py`)
- MPSImageStatisticsMeanAndVariance: `ev_8632d23ab9b19027f7f2293439b984b72902e872df58fc7b8dcd3f272518f9d4`
- MPSImageStatisticsMinAndMax: `ev_48b3ea293834edfcbd798fab63a05ae1ca54ec4b01934bea31b690aca3b65f69`
- MPSImageAdd: `ev_696a60b7c8518c49749884269886ac36ff5636683b4b90875cff604de3f7fd1d`
- MPSImageSubtract: `ev_aa5f628bafe43ebdd541c33343dbc7b6198fa8f7e8ed7f2f62443bd4fea5c98f`
- MPSImageMultiply: `ev_ee1c144fffc7d56707a40a419e04447061d493ade186720317dfa335a5464ffb`
- MPSImageDivide: `ev_6bc8162eea83167319306981ecf2a9d57c52f22bab6c24e34c0da898e6df502a`
- MPSImageTranspose: `ev_87a804b15d04b65e607c5aa9c0c18d19f31c5808b514a58d0662b14767b8b30f`

## Matrix rows closed

These rows move from `gap` to `partial` in `docs/parity-matrix/data/metal-performance-shaders.json` (the lead
updates the matrix centrally).

| Framework slug | Feature (as written in the matrix) | New status | BifrOSt path |
|---|---|---|---|
| metal-performance-shaders | Convolution, median, Sobel and Laplacian filters | partial | springboard/render/src/filter.rs |
| metal-performance-shaders | Integral images and statistics (mean, variance, min and max) | partial | springboard/render/src/integral.rs |
| metal-performance-shaders | Image arithmetic and pixel conversion | partial | springboard/render/src/filter.rs (arithmetic and transpose; pixel conversion not done) |

## Still open in this framework

Pixel conversion, Lanczos and Gaussian-sigma resampling, pyramid filters, reduction filters, distance transform,
guided filter, keypoints, neural network and recurrent layers, matrix kernels, and the kernel base classes.
