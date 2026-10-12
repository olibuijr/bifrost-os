# Core Animation parity, wave 4

Two gap rows of the Core Animation framework are built as pure Rust models in `springboard/ui`. Both are
model-only: no painter, shell or app uses them yet, and they are not tested on the VM or Midget. Wave 4 runs no
builds, so the code is not compiled here; the tests were checked by hand.

## What was built

- `springboard/ui/src/stroke_style.rs`: the stroke settings a shape layer carries. `LineCap` (butt, round, square),
  `LineJoin` (miter, round, bevel), `StrokeStyle` (width, cap, join, miter limit, dash pattern, dash phase),
  `effective_join` (a miter falls back to a bevel past the miter limit, or at a zero angle) and `dash_polyline`
  (splits an open polyline into the pieces a dash pattern leaves on; an odd pattern reads as written twice; a corner
  inside a dash stays a corner of that dash; an empty, negative or zero-sum pattern leaves the line solid).
- `springboard/ui/src/shape.rs`: `Path::dashed(pattern, phase)` applies `dash_polyline` to every subpath. A closed
  subpath is walked back to its first point, so its closing edge takes part in the pattern.
- `springboard/ui/src/layer_style.rs`: the per-layer appearance rules. `CornerMask` (min-min, max-min, min-max and
  max-max corners, as a bit set), `effective_radius` (capped at half the shorter side; a non-positive or non-finite
  radius draws square corners), `corner_radii` (radius per masked corner), `Shadow` (opacity clamped to 0..=1, blur
  radius, offset) with `footprint` and `repaint_rect`, and `mask_contains` (point test for the frame with masked
  corners cut by circles).

The fill rule is not repeated: `render::bezier::WindingRule` already covers the non-zero and even-odd rules. The
stroke start and end are not repeated either: `shape::Path::trimmed` already covers them.

## Official API names mirrored

Source: developer.apple.com DocC pages, read with `tools/parity/docc.py`.

- `CAShapeLayer`: `lineCap`, `lineJoin`, `lineDashPattern`, `lineDashPhase`, `miterLimit`, `lineWidth`,
  `strokeStart`, `strokeEnd`, `fillRule` (https://developer.apple.com/documentation/quartzcore/cashapelayer)
- `CAShapeLayerLineCap` and `CAShapeLayerLineJoin` value sets, read from the enum pages.
- `CALayer`: `cornerRadius`, `masksToBounds`, `mask`, `maskedCorners` (`CACornerMask`), `shadowOpacity`,
  `shadowRadius`, `shadowOffset`, `shadowPath` (not modelled, see below)
  (https://developer.apple.com/documentation/quartzcore/calayer)
- `CALayerCornerCurve`: `circular` and `continuous` (not modelled, see below)

## REA evidence

- Core Animation landing page: `ev_e73ceff693c55970228ebf9650499824b96845c4cc6d3082fe4137f80b91261d`
- CAShapeLayer: `ev_bf5b819b5a47e95f334b325c47092235a38f58e26564016419aa049d072b9985`
- CALayer: `ev_d2efdbea94224ea3563b03e9a86d238d734311af4769d8cfe105314231ecdae0`

## Rows closed

| Framework slug | Feature (as in data/core-animation.json) | New status | BifrOSt path |
|---|---|---|---|
| core-animation | Shape layer (CAShapeLayer, Bezier paths) | partial | springboard/ui/src/stroke_style.rs, springboard/ui/src/shape.rs (`Path::dashed`) |
| core-animation | Corner radius, shadow and masks (CALayer cornerRadius, shadowPath, mask) | partial | springboard/ui/src/layer_style.rs |

Rows still open in this framework: the layer tree, shape and text layers as app objects, `shadowPath`,
`cornerCurve` (continuous), keyframe and group animation, and the Metal and render-server rows (`os`).

## Gaps and decisions

- The corner radius clamp, the miter rule (1 / sin(angle / 2) against the limit), the shadow footprint (the frame
  moved by the offset and grown by the blur radius on every side) and the mask hit test are BifrOSt rules. The
  official pages describe the properties, not these formulas, so they are stated here as this model's rules.
- `cornerCurve` (continuous corners) is not modelled: its shape is not given in the official abstract, so no curve
  is invented. Circular corners are the only shape drawn.
- `shadowPath` (a custom shadow shape) is not modelled. The shadow footprint is a rect.
- No app-facing SDK API exposes these models yet. Wiring them into `bifrost_app::view` is a later round.
