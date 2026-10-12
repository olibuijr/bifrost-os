# Video Toolbox parity (wave 4)

## What was built

Three pure-Rust, std-only models in `springboard/render`. None touches a device, a GPU or the network,
and none is wired into the shell or the SDK yet.

- `springboard/render/src/video_transfer.rs`: pixel transfer and pixel rotation for decoded pictures.
  - `transfer_nv12(Nv12, ow, oh, Colorimetry)` converts an NV12 picture to opaque RGBA and scales it by
    nearest neighbour. `Colorimetry::for_height` picks BT.709 at 720 lines and up, BT.601 below.
  - `transfer_rgba(&Image, ow, oh)` scales an RGBA image by nearest neighbour.
  - `swap_red_blue(&mut Image)` converts between RGBA and BGRA byte order.
  - `mirror(&Image, horizontal, vertical)` flips an image.
  - `rotate_picture(&Image, Rotation, mirror_h, mirror_v)` mirrors, then turns by a quarter turn. The quarter
    turn is `gfx::rotate::rotated`, the existing renderer code, reused rather than copied.
- `springboard/render/src/block_motion.rs`: block motion estimation between two 8-bit planes.
  - `MotionEstimation::new(width, height, block, range)` is the session. `estimate(current, reference)`
    returns a `MotionField` with one `MotionVector { dx, dy, sad }` per block, row-major.
  - Full search over a square range. A candidate block must lie entirely inside the reference plane. Edge
    blocks are clipped. Ties go to the smaller offset (zero first), then to raster order.

The NV12 conversion uses the same integer BT.601 and BT.709 coefficients as `springboard/src/media/convert.rs`.
It is a separate copy because the media code sits in the binary crate and render cannot depend on it. Moving
it would be a refactor of the media path, which is out of scope for this wave.

## Official API names mirrored

- `VTPixelTransferSession` (Transformation), `VTPixelTransferSessionTransferImage`:
  https://developer.apple.com/documentation/videotoolbox/vtpixeltransfersession
- `VTPixelRotationSession` (Transformation), `VTPixelRotationSessionRotateImage`:
  https://developer.apple.com/documentation/videotoolbox/vtpixelrotationsession
- `VTMotionEstimationSession`, `init(width:height:motionVectorSize:useMultiPassSearch:label:)` and
  `motion(of:comparedTo:flags:)`, with `VTMotionEstimationSession.Motion`:
  https://developer.apple.com/documentation/videotoolbox/vtmotionestimationsession

Only the session shape and the transfer and rotation operations are mirrored. Not mirrored: the
`CVPixelBuffer` types and their attributes, the `flags` and `FrameFlags` argument, `useMultiPassSearch`
(accepted by Apple, not modelled here), the `label` and the `sourcePixelBufferAttributes` property.

## REA evidence

- `ev_7abd94f35a1740deba7318582fe6435ae6f21d947d296192f6ef415e43eacfd9`: rendered Video Toolbox landing page
  (`https://developer.apple.com/documentation/videotoolbox`). The page lists the Transformation section
  (VTPixel Transfer Session, VTPixel Rotation Session) and the Motion Estimation section (VTMotion Estimation
  Session). It was run once, as the brief asks. No per-symbol REA run was made for the three sessions.

## Matrix rows closed

These rows move to `partial`. Each one covers the model only, not the app-facing SDK.

| Framework | Feature (exactly as in data/video-toolbox.json) | New status | BifrOSt path |
|---|---|---|---|
| Video Toolbox | Pixel transfer (format conversion and scaling) | partial | springboard/render/src/video_transfer.rs (transfer_nv12, transfer_rgba, swap_red_blue) |
| Video Toolbox | Pixel rotation (VTPixelRotationSession) | partial | springboard/render/src/video_transfer.rs (rotate_picture, mirror) |
| Video Toolbox | Motion estimation | partial | springboard/render/src/block_motion.rs (MotionEstimation::estimate) |

## Gaps that remain

- No SDK wrapper: apps cannot reach these models yet. A wrapper would sit in `springboard/sdk/src`.
- No `CVPixelBuffer`-style buffer type, and no per-session attributes or labels.
- Multi-pass motion search and the `flags` argument are not modelled.
- Nearest-neighbour scaling only, with no filtered resampling.
- Not compiled or run: the wave brief forbids builds, so these tests are hand-worked and unverified.

## Tests

Each module has a `tests` block with at least five hand-worked cases. The expected values follow from the
rules in the code, for example `(298 * 110 + 128) >> 8 = 128` for luma 126 with neutral chroma, and
SAD = 4 * 3 = 12 for a 2x2 block that differs by 3 everywhere. Run them with `cargo test -p bifrost-render` once a
build is allowed.
