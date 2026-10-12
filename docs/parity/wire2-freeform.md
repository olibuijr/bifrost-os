# Wire 2: Freeform (1.4.0) wires three parity models

Branch `wire2/freeform`, on top of main (release 0.5.37). Freeform is the native app at
`springboard/apps/freeform-desktop/`. The change touches only that folder plus this note. Nothing was built or run
(the lead builds once). Host tests were written by hand, not run.

## What is now wired

| Matrix slug | Feature (as written in the matrix) | BifrOSt path | How a user reaches it |
|---|---|---|---|
| tipkit | Inline tip view and style | `springboard/ui/src/tips.rs` (`TipView`, `TipViewStyle::Mini`, `Datastore`) | First launch of Freeform shows a hint card at the bottom-left of the board: "Select, then arrange". A press on the board dismisses it for good. |
| swiftui-views-layout | Custom layout (Layout protocol) | `springboard/ui/src/layout_protocol.rs` (`FlowLayout`, `Layout`) | Choose Lasso, select strokes, press the new Arrange button in the toolbar (beside Copy). The strokes line up in a row from the selection's top-left corner and wrap at the board's right edge. |

### Details

- **Tip.** `LassoHint` implements `tips::Tip` (id `is.olibuijr.freeform.tip.lasso`). The datastore lives at
  `$HOME/.local/share/is.olibuijr.freeform.desktop/tips.txt` (`ConfigOption::DatastorePath`). If that file cannot be
  read, the tips fall back to an in-memory store. `TipView::sections` decides what shows, `mark_shown` counts the
  first draw per run, and `invalidate` runs on the first press (`Freeform::dismiss_tip`). Only the title and message
  are drawn (the Mini style), with the existing toast fill and white text.
- **Arrange.** `Freeform::arrange_selection` takes the selected strokes' bounds, places them with `FlowLayout` (gap
  12 pt, scaled with `ui.pt`, proposal width = board width minus the selection's left edge), and moves each stroke
  with `Stroke::transform(AffineTransform::translate(...))`. It is one undo step, and the selection stays on the same
  strokes. Stroke shapes are not changed.

## Not wired, and why

- **Symbol weights, scales and rendering modes** (`springboard/ui/src/symbol_layers.rs`). Freeform has no symbol
  library: its Rectangle and Oval tools draw plain outline strokes, and the toolbar icons are drawn by the kit. No
  glyph surface in the app takes a weight or a scale, so there is nothing to wire it to. Left for a later round that
  adds a symbol-based surface.
- **3D coordinate spaces** (`springboard/ui/src/coordinate_space3d.rs`). Skipped: the board is a flat 2D canvas, so
  the 3D conversions would not apply. The focus note said to skip it if it does not fit.
- **TipGroup and popover anchoring.** Only the inline view is wired. There is one tip, so no group ordering is
  needed yet, and no popover is anchored to a toolbar control.

## How to verify on Midget

1. Build and publish as usual (`bifrost pc publish` after the app is built with `tools/pc-app.py`), then
   `bifrost pc midget ota` until `phase=accepted`.
2. Remove the tip file first on Midget so the hint shows: `rm -f ~/.local/share/is.olibuijr.freeform.desktop/tips.txt`.
   Open Freeform: the card "Select, then arrange" shows at the bottom-left of the board. Press the board: it goes away
   and stays away after a relaunch.
3. Draw two or three strokes apart. Choose Lasso, drag a loop around all of them. Press Arrange: the strokes line up
   from the top-left of the selection. Press Undo: they go back.
4. Screenshot with `bifrost pc midget screen PATH`. Press controls with `bifrost pc midget tap-window Freeform X Y`
   (window-local pixels). The Arrange button is greyed out with no selection.
5. Check Light and Dark (`bifrost pc midget appearance light|dark`).

## REA evidence (official pages, rendered)

- `ev_7a07966f9170573a582fc44ed3b13bbc7c5dafad633d424ba781b6cd7fd01bf1`: TipView (developer.apple.com/documentation/tipkit/tipview).
  It states that a tip view takes a type conforming to `Tip`, an optional arrow edge and an action handler.
- `ev_35c58c7ded45a13e000677e5c980bac862684ff70c7be27b1232a0adcf210893`: Custom layout
  (developer.apple.com/documentation/swiftui/custom-layout). It states that a custom layout container asks for its
  subviews' sizes and then indicates where to place them within its bounds.
- docc: `python3 -I tools/parity/docc.py tipkit/tipview` and `swiftui/layout` (symbol lists read, no extra claims).

## Remaining gaps

- The tip shows only once per install unless the datastore file is removed. There is no action button and no
  "Show Me" flow.
- Arrange places strokes in one row or wraps them; there is no grid choice, no spacing setting and no alignment
  option. The layout protocol's `size_that_fits` and `Layout` trait are not exposed as a reusable SDK call.
- Symbol weights and scales are not used anywhere in Freeform.
- The compliance snapshot `tests/snapshots/freeform-desktop*` will differ: the toolbar gains a button and the first
  launch draws the tip card. The lead must run the compliance test with `BIFROST_BLESS=1` once to refresh the golden.
- Not tested on Midget yet, so the matrix rows stay `partial` until a Midget run is recorded.

## Matrix rows (for the lead to update in docs/parity-matrix/)

| Slug | Feature | Suggested status | BifrOSt path |
|---|---|---|---|
| tipkit | Inline tip view and style | partial (Midget pending) | springboard/ui/src/tips.rs, springboard/apps/freeform-desktop/src/freeform.rs |
| swiftui-views-layout | Custom layout (Layout protocol) | partial (Midget pending) | springboard/ui/src/layout_protocol.rs, springboard/apps/freeform-desktop/src/freeform.rs |
| symbols | Symbol weights, scales and rendering modes | unchanged (not wired) | springboard/ui/src/symbol_layers.rs |

## Files changed

- `springboard/apps/freeform-desktop/src/freeform.rs`: `LassoHint`, `now_seconds`, tip state (`with_tips`,
  `tip_sections`, `dismiss_tip`), Arrange (`arrange_selection`, `Target::Arrange`, `BAR_ARRANGE` = 13), tip card in
  `draw`, and host tests (`arrange_lines_the_selected_strokes_up_in_a_row_from_their_corner`,
  `the_first_use_hint_shows_until_the_board_is_pressed`).
- `springboard/apps/freeform-desktop/src/main.rs`: `tip_store`, passed to `Freeform` in `configure`.
- `springboard/apps/freeform-desktop/Cargo.toml`, `manifest.toml`: version 1.4.0.
- `springboard/apps/freeform-desktop/CHANGELOG.md`: 1.4.0 entry.
- `docs/parity/wire2-freeform.md`: this note.

## Checks

- `python3 tools/check-names.py`: no hits.
- Hand-worked arrange values: scale 1.5625 (1200/768), so the gap is 12 * 1.5625 = 18.75 px. Strokes of 20 x 10 at
  (10,10) and 40 x 20 at (100,50) give a union of (10,10) 130 x 60; the second stroke moves by (-51.25, -40) to
  (48.75, 10) and (88.75, 30).
