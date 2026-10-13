# SwiftUI views and layout: parity work

Framework: https://developer.apple.com/documentation/swiftui (Views, View layout). Matrix row data:
`docs/parity-matrix/data/swiftui-views-layout.json`.

## What was built

Pure models in `bifrost-ui`, std only, with unit tests. No view API yet.

- `springboard/ui/src/stack.rs`
  - `Anchor` (nine alignments) and `overlay(bounds, insets, anchor, layers)`: lays layers over one another, each
    aligned to the anchor, clamped to the area. Mirrors the `ZStack` layout. Leading and trailing are the left and
    right edges.
  - `first_fitting(available, candidates, axes)`: index of the first candidate, in preference order, that fits the
    available size on the constrained axes, or `None`. Mirrors the `ViewThatFits` selection. The caller chooses the
    fallback when nothing fits.
  - Tests: `overlay_aligns_every_layer_to_the_anchor`, `overlay_clamps_oversized_layers_inside_the_area`,
    `first_fitting_takes_the_first_candidate_in_preference_order`, `first_fitting_ignores_axes_that_are_not_constrained`.
- `springboard/ui/src/shape.rs` (new module, one `pub mod shape;` line in `lib.rs`)
  - `Point`, `Subpath`, `Path`: straight-segment subpaths with `move_to`, `line_to`, `close`, `add_rect`,
    `add_rounded_rect` (radius clamped to half the shorter side), `add_ellipse` (flattened), `bounds`, `length`,
    `trimmed(from, to)` for fractions of the outline (clamped, the result is open polylines in path order).
  - `Shape` enum: `Rectangle`, `RoundedRectangle(r)`, `Capsule`, `Ellipse`, each giving a `Path` inside a `Rect`.
    Mirrors the `Shape` protocol's built-in shapes and the `trim(from:to:)` behaviour of its path.
  - Tests: rectangle perimeter and bounds, empty path, trimming within one edge and across a corner, empty and
    reversed ranges, ellipse bounds and flattened length, rounded radius clamping, capsule, and `line_to` /
    `close` semantics.

## Official references

- ZStack: https://developer.apple.com/documentation/swiftui/zstack
- ViewThatFits: https://developer.apple.com/documentation/swiftui/viewthatfits. Its page says it "selects the first
  child whose ideal size on the constrained axes fits within the proposed size", with the constrained axes given by
  `init(in: Axis.Set, content:)`.
- Shape: https://developer.apple.com/documentation/swiftui/shape

## REA evidence

Captured with `rea capture-browser-scenario` (REA 6.4.0, Chromium 152, headless, accessibility tree at end):

- `ev_78d14297af9f8430a941c441102a8a2843f64006ec3feb5e216535deb7862f91`: ZStack page
- `ev_9584889b85aff243b91aaa0806dd0e5afd1c6b82b889800691ad872c750cb630`: ViewThatFits page (overlay and
  first-fit wording above is taken from this capture)
- Shape page: the capture returned `cleanup_incomplete` with the page content in `partial_observation`, so it has no
  complete evidence id. The Shape statements above rest on the official page only and should be re-captured.

The matrix's own row evidence (`ev_1a882830e38c95f440f0bd336bd00c74fb8bc2a84fc97d1342e354040edb662e`) is kept
unchanged.

## Matrix changes

- Shapes: `gap` to `partial`, bifrost path `springboard/ui/src/shape.rs`.
- Layout fundamentals: path extended with `stack.rs` overlay and first-fit models; status stays `partial`.
- Framework status stays `partial`. Gaps updated: the Shape row now names the missing renderer and app API, and the
  Layout row names the missing view API for ZStack and ViewThatFits.

## Remaining gaps

- No declarative `View` protocol, generic modifiers or state-driven body. Environment values exist (see below).
- No `ZStack` or `ViewThatFits` view types; only the layout and selection models exist.
- No `Shape` protocol for apps, no stroke or fill style, no renderer for `Path`.
- No `Canvas`, no `LazyVGrid`, no `Layout` protocol.
- `ScrollViewReader`, `TabView`, `GroupBox`, `ControlGroup` and swipe-action lists are untouched.

## Team A8 round: view configuration

- Environment values (`EnvironmentValues`): `springboard/ui/src/environment.rs` (`Environment`, `EnvValue`). A node sets
  a value, its descendants read it, and a nested scope overrides it until the scope is left. The parity row now
  points there as `partial`. No view reads the environment yet. Generic modifiers and state-driven bindings are still missing, so the row is not `parity`.

## Verification

Not built or run in this stage, as the workflow requires. The name guard `python3 tools/check-names.py` exits 0.
