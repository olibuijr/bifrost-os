# cg-paint: colour, colour spaces, gradients, shadings, functions, patterns

Code: `springboard/render/src/cg/color.rs` and `springboard/render/src/cg/paint.rs` (module list in `cg/mod.rs`).
Nothing here has been compiled or run yet (build agents do not run cargo); everything needs the lead's build and a Midget check.

| Row | BifrOSt path | Done | Still to check |
|---|---|---|---|
| Colour (create generic RGB/gray/CMYK, alpha copy, convert, equality, white/black/clear) | `cg::color::Color` | all of it, with unit tests | Midget: freeform gradient fill still looks the same |
| Colour spaces (device and generic, sRGB, extended, linear, wide gamut, Rec.709/2020, gray 2.2, Lab, indexed, pattern; name, model, component count, wide-gamut flag) | `cg::color::ColorSpace` | all listed spaces | generic RGB/gray are our own definition (sRGB primaries, 2.2 power curve) |
| ICC matrix/TRC profiles | `ColorSpace::from_icc` | RGB matrix with curv/para curves, gray kTRC; lookup-table profiles return `ProfileError::Unsupported` | a real display profile on Midget |
| Conversion, rendering intents, conversion info | `ColorConversionInfo`, `Color::converted` | XYZ (D50) chain, absolute scaling by media white; perceptual and saturation behave as relative | gamut mapping is not modelled |
| Gradients (linear, radial with radii, extend start/end, colours and locations, colour space) | `LinearGradient`, `RadialGradient`, `ColorRamp`, `ConicGradient` | all; interpolation in the ramp's own space, 1024-entry sRGB table | |
| Shadings and functions (sampled, exponential, stitching) | `Shading`, `Function` | axial and radial shadings; 1-input functions | calculator-style functions and multi-input sampled functions are not done |
| Patterns (coloured, uncoloured, tiling, step, cell callback) | `Pattern`, `PatternPaint`, `Tiling` | cell drawn by a callback into a surface; step, matrix, three tiling modes | the pattern matrix is relative to user space, not the page |
| `Paint` and fills | `Paint::{shade, shade_row, fill_coverage, fill_rect}` | `fill_coverage` is written against the raster contract (`Coverage` with `x, y, w, h, data`; `Affine` with `a b c d tx ty`) and `blend::Mode` | lead: reconcile field names in `affine_parts` / `coverage_parts` at the bottom of `paint.rs`, and the `BlendMode` alias |

Adopted and deleted: `ui/src/gradient.rs` (freeform now uses `cg::paint`), `render/src/function.rs`, `render/src/pattern.rs`;
private gradient painters in `src/shell/browser/tiles.rs` (`stop_color`, `linear`, `radial` now call `cg::paint`) and in
`src/deildu/library.rs` and `src/deildu/player_view.rs`.
