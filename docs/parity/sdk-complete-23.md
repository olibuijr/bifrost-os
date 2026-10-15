# Parity sdk-complete-23: photo metadata, file system resources and image filters

Branch `parity/w8-sdk-complete-23`. Headless tests only; no build or test was run in this round, so the expected
values were worked out by hand and the lead runs the suites after merging.

## What was built

- `springboard/render/src/filter.rs`: convolution along one axis (`convolve_axis`, `Axis`) and per-channel RGB
  convolution (`convolve_rgb`, `RgbImage`) with odd kernel sides from 3 to 9 (`CONVOLUTION_SIDES`,
  `is_convolution_side`); histogram specification (`specification_table`, `specify`).
- `springboard/ui/src/fs_resource.rs`: the `Resource` model for file system modules: a block device node under
  `/dev`, an absolute path without dot parts, and a generic URL with a scheme. Registered in `springboard/ui/src/lib.rs`.
- `springboard/src/shell/photos/exif.rs`: `parse_png` reads the `eXIf` chunk and the `IHDR` size; `parse_image`
  chooses the JPEG or PNG reader by signature. The orientation and the info sheet use `parse_image`.

## Tests

- `springboard/render/tests/parity_sdk-complete-23.rs`: thresholds, histograms, specification, morphology,
  convolution, median, Sobel, Laplacian, arithmetic and transpose with concrete pixel values.
- `springboard/ui/tests/parity_sdk-complete-23.rs`: resource kinds, the maintenance task log and completion, and
  the photo library change details.
- `springboard/tests/parity_sdk-complete-23.rs`: PNG metadata built in memory.

## Matrix rows

Rows that gained code or tests: core-image convolution filters; metal-performance-shaders morphology, convolution,
thresholding and histogram rows; fskit resources and tasks; photokit change details and PNG metadata.

Rows left as they were, with the reason:
- photokit fetch options: moments, years and user albums need a library model in the shell first.
- photokit smart albums: the built-in albums are not exposed through the SDK, which needs a new service answer.
- photokit image caching: video thumbnails need a decoder, and the manager has no progress handler for apps.
- photokit editing: a third-party editor host does not exist yet.
