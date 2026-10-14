# Metal Performance Shaders: parity notes

Status: partial (was partial). Matrix row: `docs/parity-matrix/data/metal-performance-shaders.json`.

## What was built

`springboard/render/src/filter.rs` (module `filter` in `springboard/render/src/lib.rs`), pure std,
CPU only, operating on an 8-bit single-channel `Plane`:

| Official API (names mirrored) | BifrOSt function | Notes |
|---|---|---|
| MPSImageThresholdBinary, ThresholdBinaryInverse, ThresholdToZero, ThresholdToZeroInverse, ThresholdTruncate | `threshold(src, Threshold, t, max)` | Five modes with the official per-pixel definitions. `t` and `max` are u8, not float. Colour transform is not supported. |
| MPSImageHistogram, MPSImageHistogramInfo | `HistogramInfo::new(entries, min, max)`, `histogram(src, info)` | Entries 1 to 256. Values outside min..=max count in the nearest end bin. Single channel, no alpha. |
| MPSImageHistogramEqualization | `equalization_table(bins)`, `equalize(src)` | Cumulative-distribution lookup over 0..=255. A flat image is returned unchanged. |
| MPSImageErode, MPSImageDilate, MPSImageAreaMin, MPSImageAreaMax | `area_filter(src, kw, kh, Extreme)`, `erode`, `dilate` | Rectangular min or max. Kernel sides must be odd. Edge pixels clamp. Separable (horizontal, then vertical). Erode and dilate are the same rectangle primitive as area min and max. |

## Official references

- Landing page REA evidence: `ev_90fad2adbd6e16ba3a74fb795ef31853ed7a40a1a449a915f08bd24e7079ffd4`
  (`https://developer.apple.com/documentation/metalperformanceshaders`, rendered by the REA web-page inspector).
- Image Filters outline (`docc.py metalperformanceshaders/image-filters`) lists the threshold, histogram,
  morphological and statistics sections used here.
- Symbol abstracts: MPSImageHistogram, MPSImageHistogramInfo (fields: numberOfHistogramEntries,
  histogramForAlpha, minPixelValue, maxPixelValue), MPSImageHistogramEqualization, MPSImageThresholdBinary,
  MPSImageErode, MPSImageAreaMin.

## Remaining gaps

- No app-facing SDK entry point. The filters are reachable only from Rust code in the render crate.
- Thresholds and histograms are 8-bit. The official classes work on float textures.
- Histogram specification, alpha histograms, and the threshold colour transform are missing.
- Erode and dilate take no structuring-element values, and no edge mode other than clamp.
- Still missing: integral images, region statistics (mean, variance, min and max), image arithmetic,
  reductions, convolution (Sobel, Laplacian, median), Lanczos scaling, neural network and recurrent layers,
  matrix kernels, and keyed archiving of kernels. The base trait exists (see below); the binary kernel base does not.
- No GPU path. The official classes run as compute kernels; `filter.rs` runs on the CPU.
- Verification has not been run in this round (no build, per the stage rules). The unit tests in
  `filter.rs` (10 tests) are the next thing to run.

## Kernel base trait (team A4)

- `springboard/render/src/kernel.rs`: `UnaryImageKernel` is the base of a kernel that reads one image and writes
  one of the same size (`label`, `encode`). It is the counterpart of MPSUnaryImageKernel. `ColorMap` adapts a
  straight-colour function to it, and `run_chain` runs a list of unary kernels in order, returning None when a
  kernel does not return one pixel per source pixel.
- Official references: https://developer.apple.com/documentation/metalperformanceshaders/mpsunaryimagekernel ,
  https://developer.apple.com/documentation/metalperformanceshaders/mpskernel
- Built on parity/team-a-04; not yet verified on Midget. The tests in `kernel.rs` (module `tests`) were written but
  not run in this round.
