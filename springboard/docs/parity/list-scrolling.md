# List scrolling (round 75)

Owner goal: scrolling in long lists stays smooth on the lab PC (Intel GPU). This slice removes per-frame work for rows
that are off screen and repeated text rasterisation. No Apple reference rows were closed by this slice.

## Changes

| Area | Path | What changed | Status |
| --- | --- | --- | --- |
| SDK clipping views (ScrollView and any clipping parent) | `springboard/sdk/src/view/mod.rs` (`draw_subtree`, `draw_dirty`, `in_rows`, `reaches_rows`, `has_layer`) | A child whose painted area (`dirty_rect`) misses the parent's rows is skipped, and its dirty flags are cleared. A child with a layer in its subtree is always drawn. Before this, a scroll repaint walked and painted every child of the document, including rows far outside the clip. | Done in code, unit test for the overlap predicate. Needs Midget check: scroll a long Files, Mail or Settings list and confirm no row tearing or stale rows when rows scroll back in. |
| Files list rows | `springboard/apps/files/src/views/list.rs`, `views/mod.rs` (`cached_text`), `main.rs` (`row_text` field) | Name, date, size and kind of each visible row draw through the SDK `TextCache` instead of rasterising on every repaint. | Done in code. Files 1.27.0. Needs Midget check of scrolling in a 10k folder. |

## Known remaining work (not done here)

- Mail list (`springboard/apps/mail-desktop/src/draw.rs`, `state.rs` `visible_lines`): `visible_lines()` clones every row and
  regroups threads on every paint, and `fit()` measures per character for each visible row. Needs a cached line list
  keyed on the mailbox generation, and a row span so only visible lines are walked.
- Files `visible()` builds a full index vector per paint (50k rows allocate on each scroll frame).
- Momentum: `ScrollView` already runs fling and bounce on the frame clock (`tick`); no wheel input exists yet because the
  proto delivers only Down, Move, Up and Cancel.
- Music library, Photos grid and Settings were not measured in this round.

## Evidence

- Reviewed by reading only: `sdk/src/view/mod.rs` draw traversal, `sdk/src/view/scroll.rs` physics, `apps/files/src/window.rs`
  paint loop (already windows rows with `row_span`), `apps/mail-desktop/src/draw.rs` `draw_list`.
- Nothing was built or run in this round (the lead builds and tests).
