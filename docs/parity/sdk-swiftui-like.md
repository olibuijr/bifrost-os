# parity(sdk-swiftui-like): declarative layout containers

Slice: SwiftUI view, layout and container rows (`python3 tools/bifrost.py pc parity show swiftui-view`).
Evidence used: the existing pure layout models in `springboard/ui/src/stack.rs` (`overlay`, `first_fitting`, `grid`,
`adaptive_columns`) and the retained view tree in `springboard/sdk/src/view/`. The macOS 27 SwiftUI reference was not
inspected this round (no REA session); behaviour follows the public SwiftUI names and the row notes.

## Rows advanced

| Row (`swiftui-view`) | API added | BifrOSt path | Done | Still needs Midget check |
|---|---|---|---|---|
| Layout fundamentals (stacks, grids) | `ZStack` (`anchored`, `inset`), `Grid` / `Columns` (fixed or adaptive, non-lazy) | `springboard/sdk/src/view/overlay.rs`, models in `springboard/ui/src/stack.rs` | Layers share one area aligned to an anchor; grid fills rows left to right in equal cells | Visual check of a ZStack overlay and a 2-up adaptive grid in a shell app |
| Layout fundamentals (ViewThatFits) | `ViewThatFits` (`constrained`, `shown`) | `springboard/sdk/src/view/overlay.rs` | Shows the first candidate that fits the offered rectangle, hides the rest, falls back to the last | Resize check on a live window |
| Layout adjustments (alignment, spacing, insets) | `ZStack` alignment anchors and insets | `springboard/sdk/src/view/overlay.rs` | Nine anchors and `EdgeInsets` honoured | Covered by the same Midget pass |

Not closed: lazy grids (`LazyVGrid` is laid out eagerly), ZStack/Grid draw nothing of their own, `ViewThatFits`
measures intrinsic sizes only (no content-driven re-layout while a child animates), and the declarative `View`
protocol / modifiers rows stay Gap.

## Shared-file edits

- `springboard/sdk/src/view/mod.rs`: `mod overlay;`, one `pub use overlay::{Columns, Grid, ViewThatFits, ZStack};`,
  and `#[cfg(test)] mod tests_overlay;`. Three small lines, no other change.

## Migration / deletion

None. The new views are the first app-facing users of the `overlay`, `first_fitting` and `grid` models; no older
copy existed in the SDK.

## Tests

`springboard/sdk/src/view/tests_overlay.rs`: ZStack alignment and insets, ViewThatFits choice, fallback and axes,
Grid wrap and cell placement, adaptive column count, zero row height. Not run (the brief forbids builds in this slice).
