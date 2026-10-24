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

## Round: colour lists, image objects, system colour names

Built on parity/team-k-appkit-graphics-printing; not yet verified on Midget. Host unit tests were written but not run
in this round; the lead runs them in the build stage.

- `springboard/sdk/src/color_list.rs` (`ColorList`, `ColorLists`): mirrors `NSColorList`. A list is named and keeps its
  keys in the order they were first set; setting an existing key replaces the colour in place; empty keys are refused.
  `ColorLists` is the registry an app keeps (`availableColorLists`): it refuses a second list with the same name and
  looks lists up by name. Colours are `graphics_color::Color`, so a list can hold any colour space.
- `springboard/sdk/src/image_object.rs` (`ImageObject`, `ImageRep`, `CapInsets`, `Slice`): mirrors `NSImage` as an
  object. Representations are bitmap (pixel grid and point size; 72 dpi by default) or vector (drawn at any size).
  `best_rep(target, scale)` prefers a vector representation, then the smallest bitmap with enough pixels for the target
  at the given scale, then the largest bitmap. The size is the explicit size, else the first representation's size.
  `template` is a flag. `resizable_slices(target)` gives the nine-slice layout for cap insets and returns `None` when the
  target is smaller than the caps. `from_encoded` reads the pixel size from the header of PNG, JPEG and the other codecs
  the imageio module reads (`graphics_imageio::pixel_size`).
- `springboard/render/src/ui.rs` (`system`): `WINDOW_FRAME` (windowFrameColor), `FIND_HIGHLIGHT` (findHighlightColor,
  a design approximation) and `SHADOW` (shadowColor).

### Row status

- "Color: colour spaces and lists": still `partial`. Colour spaces and conversions were already in `graphics_color`;
  the colour lists are new. No colour well or picker uses the lists yet.
- "Images and PDF: image objects and representations": still `partial`. The object model is new. Not used by the kit
  image widgets, and no per-codec representation classes or template tinting when drawn.
- "Color: semantic system colours": still `partial`. Three names added; user accent and the remaining names are open.
- Print sheet and page layout panel: unchanged. These need a shell-drawn sheet (the shell has no `run_modal`), so
  they are left as they are.
- Symbol catalogue, PDF rendering outside the Books engine, and the GPU rows: unchanged, as in the matrix.

### Remaining gaps

- Shell-drawn print sheet and page layout panel.
- Image drawing in the kit, per-codec representation classes, and template tinting.
- Colour well and picker modes that use colour lists; eyedropper.
- User accent colour feeding `controlAccentColor`.

## Round: measured colour names, colour well, printer description, image drawing (2026-10-24)

Branch `parity/appkit-graphics`. Details in `docs/bifrost-ui.md` ("NSColor semantic colours", "Table columns and shared
panels", "Printing", "AppKit drawing and print layout").

- `springboard/render/src/ui.rs` (`system`): the remaining semantic names (control, control text, disabled control
  text, selected control and text, unemphasized selection, focus ring, insertion point, header, menu item, highlight,
  window frame text, the quinary label, five system fills, alternating row backgrounds, control edges) with values
  measured from the reference's light and dark appearance catalogues (named-colour tables, read by number; nothing
  copied), plus `NAMED` / `named()` lookup by catalogue name. The accent already reached `controlAccentColor`
  (`Pair::following_accent`, `ToApp::Appearance` bits 8 to 11); that gap was stale.
- `springboard/ui/src/panels.rs`: picker modes with the reference raw values, the picker mask, `showsAlpha`,
  `isContinuous`, attached colour lists and slider components (grey, RGB, CMYK, HSB).
- `springboard/ui/src/color_well.rs`: the colour well (three styles), the shared panel protocol, the swatch popover grid
  and `draw_well`; `sdk/src/color_list.rs` `swatch_grid` and `pick_into_well`.
- `springboard/ui/src/printer_caps.rs`: make and model, document formats, resolutions, `language_level`,
  `device_description`.
- `springboard/sdk/src/image_object.rs`: `ImageObject::draw` (template tint, opacity, nine slices) and per-codec
  representations with frame counts.

REA evidence: ev_6c3ad0e07099189a330191100d5b1564f636b6064b3a70563d484aca64e11954 (colour panel modes),
ev_707cf40ef10ba0a4f1899622a486b7b1d441668d39a5c31a9cdc6fbef9e5e470 (colour well),
ev_718c7c2f272eff48acd3dad39768bd1d0c20bbe35ab21b643425783b55deedf1 (printer),
ev_54eacf1088c9c1e27dce9ec56f7386ff23618c8b28b9ed08b757acb7dca16b09 (deviceDescription),
ev_6a696e64493877091a178708ee554577c759c04ad87cb1546ee692e8f8fd131d (systemFill). Catalogue measurements: macOS 27
26A434 `SystemAppearance.bundle` `Aqua.car` (sha256 c8f2a0a714d5e935ded1abd136e6adede0df7d6bed24aff962d484fd274cb50c)
and `DarkAqua.car` (a158476559c537174a49cccb0ad952baaa021ed6e4f290dabb37dc5c4a6772e9), outside the repo.

Still open: the existing pairs were not re-valued to the measured 26A434 values (a lead decision with a Midget
comparison); no shell screen hosts a colour well or draws the panel pickers; eyedropper; Page Setup and accessory views
in the shell; the print sheet does not read the printer's lists; image DPI from files, HEIC; a kit widget that takes an
`ImageObject`; the symbol catalogue.
