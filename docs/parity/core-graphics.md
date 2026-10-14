# Core Graphics parity

Official reference: https://developer.apple.com/documentation/coregraphics

REA evidence of the rendered framework page: `ev_02cfd76ddb7b87442642d82d0a01dd8c3631f0e987cd5f40faef56bae6c62821`
(page: "Core Graphics | Apple Developer Documentation").

Matrix row data: `docs/parity-matrix/data/core-graphics.json`.

## What was built

Three pure models in `bifrost-ui` (`springboard/ui/src`), std only, no drawing calls yet:

| Module | Mirrors (official names) | Contents | Tests |
|---|---|---|---|
| `affine.rs` | `CGAffineTransform`: `CGAffineTransformMake`, `MakeTranslation`, `MakeScale`, `MakeRotation`, `Concat`, `Invert`, `CGPointApplyAffineTransform`, `CGRectApplyAffineTransform`, `CGAffineTransformIsIdentity` | `AffineTransform` (`a b c d tx ty`): `IDENTITY`, `translate`, `scale`, `rotate`, `then` (apply self, then next), `apply`, `apply_rect` (bounds of the transformed corners), `invert` (`None` when singular), `is_identity` | 6 |
| `path.rs` | `CGPath`, `CGMutablePath`: `move`, `line`, `cubic` and `close` elements; `init(rect:)`, `init(ellipseIn:)`, `copy(using:)`, `boundingBox`, `isEmpty` | `Path` with `move_to`, `line_to`, `cubic_to`, `close`, `add_rect`, `add_ellipse_in` (four cubic quarter arcs), `transformed`, `polylines(tolerance)` (curves split by estimated length; a lone move draws nothing), `bounds` | 6 |
| `gradient.rs` | `CGGradient`: colour stops and locations; the linear and radial ramp positions that `CGContextDrawLinearGradient` and `CGContextDrawRadialGradient` evaluate | `Gradient` and `Stop` (`at`, straight 8-bit RGBA): `new` (sorts and clamps stops), `color_at` (per-channel interpolation, end colours held outside the ramp, transparent when empty), `linear_t`, `radial_t` | 7 |

`bifrost-ui/src/lib.rs` gains three `pub mod` lines (`affine`, `path`, `gradient`). No other crate changed.

## Matrix rows changed

- Path construction and stroking: gap to partial.
- Affine transforms and transform stack: os to partial. The compositor's window transforms are unchanged.
- Gradients (linear and radial): gap to partial.

The framework status stays `partial`. Shapes, clipping, shadings, patterns and the drawing context are unchanged.

## Not done (remaining gaps)

- No `Surface` call fills or strokes a `Path`, applies an `AffineTransform`, or fills with a `Gradient`. The models have no drawing path into `springboard/render/src/gfx.rs` yet.
- Paths have no dash, stroked outline, winding rule or contains-point test.
- No clip rectangle or clip path on the Surface (`docs/bifrost-ui.md:867`).
- No line, ellipse or arc draw call on the Surface. `add_ellipse_in` only builds geometry.
- No shading, function or pattern fills (`CGShading`, `CGFunction`, `CGPattern`).
- No colour space object or wide-gamut conversion. `CGLayer`, the PDF reader and display and window services are unchanged.
- The browser's private gradient painter (`springboard/src/shell/browser/tiles.rs`) is not moved onto `Gradient`.
- Apple's bit-level layouts (`CGPathElement` and similar) are not reproduced; the models use plain Rust enums.

## Verification

Not built or tested here, by instruction. The stage that runs `cargo test -p bifrost-ui` must run the 20 new tests. `python3 tools/check-names.py` reports no hits for the new files.

## Team-k refresh (branch parity/team-k-quartz)

Not built or run on this branch; the lead builds and checks it. Verified on Midget: no.

The earlier section above describes the pure models from the first Core Graphics waves. The render crate has moved on,
and the matrix rows in `docs/parity-matrix/data/core-graphics.json` now reflect the code on the `arm` branch:

- The whole render `cg` tree is public to apps as `bifrost_app::kit::cg` (`springboard/sdk/src/lib.rs`, the `kit` module).
- `cg::surface` draws lines, ellipses and arcs, and fills and strokes paths, on the Surface.
- `cg::paint` holds colour ramps, linear, radial and conic gradients, shadings and functions, and patterns.
- `cg::image` holds `CgImage` with cropping, masks and premultiplied conversion.
- The SDK wrappers `graphics_pdf`, `graphics_imageio`, `graphics_color`, `graphics_stroke`, `graphics_gradient` and
  `graphics_context` sit on top of these models.
- Apps that use the tree today: Freeform (`cg::paint` gradient ramp), Preview (`cg::pdf` read, draw and writer, with annotated save).

Rows updated: bitmap context, path construction and stroking, primitive shapes, affine transforms, colour, gradients,
shadings, patterns, images, image decoding and encoding, PDF documents, PDF creation. All remain `partial`.

Remaining gaps are listed in the matrix row `gaps` field. The main ones: most of the tree has no app caller yet, PDF
pages cannot be drawn into a PDF context, and no Boolean path operations exist.
