# Parity wave 3: Core Graphics

Three gap rows of the Core Graphics framework are now partial. Each has a pure Rust model in
`springboard/render/src`, with host unit tests. None is wired into `Surface`, the SDK or the shell yet,
and none has been built or run on the VM or Midget (the wave ran without builds).

## What was built

| Module | What it does | Official API it mirrors |
|---|---|---|
| `springboard/render/src/clip.rs` | `Rect` and `ClipStack`: a clip rectangle that only narrows, with save and restore, a per-pixel `contains` test and a per-row `clip_span` trim | `CGContextClip`, `CGContextClipToRect`, graphics state save and restore |
| `springboard/render/src/pattern.rs` | `Pattern`: a row-major ARGB tile repeated on a grid with a step and a phase, transparent between tiles, and a `fill` block sampler | `CGPattern`, `CGContextSetFillPattern` (tiling with `xStep`/`yStep`) |
| `springboard/render/src/function.rs` | `Function`: a piecewise-linear colour function over a domain with evenly spaced samples, and `axial`, which evaluates it at a point's offset along a line | `CGFunction`, `CGShading` (axial shading), `CGContextDrawShading` |

Design notes:

- `ClipStack::clip_to` always intersects, so a clip can only shrink. `restore` returns false when nothing was saved.
- `Pattern` gives each pixel to exactly one cell. A step smaller than the tile cuts the tile at the step; it does not overlap.
- `Function` clamps inputs to the domain and treats NaN as the domain start. It reuses `gradient::linear_t` for the axial projection and `colorfx::Rgb::mix` for interpolation, so no new colour type is added.
- The Core Graphics names appear in this report only. The code uses neutral names, per the owner rule.

## Official references

- https://developer.apple.com/documentation/coregraphics (framework landing page; DocC via `tools/parity/docc.py coregraphics`)
- https://developer.apple.com/documentation/coregraphics/cgpattern (`init(info:bounds:matrix:xStep:yStep:tiling:isColored:callbacks:)`)
- https://developer.apple.com/documentation/coregraphics/cgshading (`init(axialSpace:start:end:function:extendStart:extendEnd:)`)
- https://developer.apple.com/documentation/coregraphics/cgfunction (`init(info:domainDimension:domain:rangeDimension:range:callbacks:)`)

REA evidence (rendered official landing page): `ev_441181dd5a738489ca0d0a5c959c4cefb2989fa323085a5ffca32811377a6335`

## Matrix rows closed

Status is `partial` in every row. The lead updates `docs/parity-matrix/` centrally.

| Framework slug | Feature (as written in `docs/parity-matrix/data/core-graphics.json`) | New status | BifrOSt path |
|---|---|---|---|
| core-graphics | Clipping | partial | `springboard/render/src/clip.rs` |
| core-graphics | Patterns (CGPattern) | partial | `springboard/render/src/pattern.rs` |
| core-graphics | Shadings and gradient functions (CGShading, CGFunction) | partial | `springboard/render/src/function.rs` |

## Still open in this framework

- Surface integration: `clip.rs` is not yet applied to `Surface` drawing calls, and `docs/bifrost-ui.md:867` still reports the missing clip.
- SDK exposure of patterns and shadings for apps.
- Layers (CGLayer), window services and event services remain gap rows.

## Verification

- `python3 tools/check-names.py` exits 0 (no Apple or product names in the `.rs` files).
- Hand-worked test expectations are in each module's `#[cfg(test)]` block: 5 tests in `clip.rs`, 5 in `pattern.rs`, 5 in `function.rs`.
- No cargo or rustc run (wave rule).
