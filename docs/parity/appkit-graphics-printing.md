# AppKit graphics, drawing, colour and printing: parity notes

Framework row: `docs/parity-matrix/data/appkit-graphics-printing.json`.
Official reference: https://developer.apple.com/documentation/appkit (Bezier paths, gradients and shadows at
https://developer.apple.com/documentation/appkit/nsbezierpath, https://developer.apple.com/documentation/appkit/nsgradient,
https://developer.apple.com/documentation/appkit/nsshadow).

## What was built

Two pure render models, std only, no unsafe, no new crates. Neither touches a pixel buffer yet.

- `springboard/render/src/bezier.rs` (`BezierPath`, `WindingRule`, `Point`): mirrors the path-construction
  part of NSBezierPath. Commands: `move_to`, `line_to`, `curve_to` (cubic), `close`, `append`. Builders:
  `rect`, `rounded_rect` (radii clamp to half the box), `oval`. Queries: `flatten(tolerance)` to polylines,
  `bounds` as (x, y, w, h), `contains(x, y, rule)`, and `fill_mask(w, h, rule)` (hard-edged, pixel centres).
  Winding rules `NonZero` and `EvenOdd` follow the fill semantics documented for NSBezierPath.
- `springboard/render/src/gradient.rs` (`Gradient`, `Stop`): mirrors NSGradient colour stops. `new` from
  `(location, colour)` pairs (clamped and sorted, `None` when empty), `two_color`, `color_at(t)` with end
  colours extended outside the stops, `linear_at` and `radial_at` sampling, `linear_pixels` and
  `radial_pixels` grids. Colours are the packed words from `gfx::pack`/`gfx::rgba`; channels interpolate as
  stored, which is correct for premultiplied pixels.
- `springboard/render/src/lib.rs`: one `pub mod` line per new module.

Tests: 7 in `bezier.rs` and 7 in `gradient.rs`, covering area and bounds of fills, the even-odd hole, radius
clamping, curve flattening endpoints and peak, subpath restart after close, stop interpolation and clamping,
and radial and linear sampling. Nothing was built or run in this round; the build stage runs them.

## Row status

- "Drawing: Bezier paths, gradients, shadows": `gap` to `partial`. Path and gradient models exist; no shadow
  model; not painted onto `Surface`; no SDK entry point.
- Framework status stays `partial`.

## REA evidence

- NSBezierPath page, rendered through the REA Chrome DevTools provider: `ev_409f5b9cb512d8ab3d5645ec22e4ed85b8f0a69173ab890b5396e326a163007f`
- NSGradient page: `ev_bda51480254f369ddb7a1b24a150eec17663889cddc7940363a21a9af26aced4`
- Framework landing page (earlier round): `ev_c13e34ac0a796f139c7afc1ac6d88343b2a61a27ec40e0a76c53e92c372eebaf`

Official API shape came from `docc.py`: NSBezierPath (`init(rect:)`, `init(roundedRect:xRadius:yRadius:)`,
`init(ovalIn:)`, `move(to:)`, `line(to:)`, `curve(to:controlPoint1:controlPoint2:)`, `close()`, `append(_:)`,
`fill()`, `flattened`, `windingRule`); NSGradient (`init(colors:atLocations:colorSpace:)`,
`interpolatedColor(atLocation:)`, `draw(from:to:options:)`, `draw(fromCenter:radius:toCenter:radius:options:)`);
NSShadow (`shadowOffset`, `shadowBlurRadius`, `shadowColor`, `set()`).

## Remaining gaps

- Shadow model (offset, blur radius, colour) for drawing. NSShadow is not implemented.
- Stroke styling (width, joins, caps, dash) and clipping paths. NSBezierPath stroke and `addClip()` are not implemented.
- Painting paths and gradients onto `Surface`, and an SDK entry point in `springboard/sdk/src/` for apps.
- Print sheet and page layout panel, colour spaces and colour lists, NSImage-style representations, PDF
  rendering outside the Books engine (unchanged from the matrix).
