# G2 `kit-canvas-views`: the SDK view system draws through `Canvas`

Part of wave G2 in `docs/graphics-waves.md` (row 10) and the contract in `docs/graphics-architecture.md` ("SDK",
"Canvas, DisplayList"). The retained view system (`springboard/sdk/src/view/`) no longer paints into a `Surface`
directly. Its draw walk takes `&mut dyn Canvas`, so the same view tree can be drawn into a CPU `Surface` (the
shared-memory path), recorded into a `DisplayList` (the GPU executor's input), or drawn by `bifrost-2d` later, without
any view knowing which one.

## What is implemented

### Draw walk (`view/mod.rs`)

- `View::draw` and `View::draw_overlay` take `&mut dyn Canvas` (was `&mut Surface`). `Window::draw` takes
  `&mut dyn Canvas` and reads the target size from `Canvas::size()`. Existing `&mut Surface` callers coerce, so the
  apps and the tests keep their calls.
- `draw_subtree` maps the old pixel operations onto the vocabulary:
  - group opacity: `push_opacity(alpha)` / `pop_opacity()` around the subtree (was: save the pixels under the frame,
    blend them back). The subtree is now one group, not a copy of the frame.
  - a view's `background`: `fill_rrect(frame, 0, colour)`, which composites source-over like the old `over` loop.
  - a `clips_children` view (a scroll view's rows): `push_clip(full width x frame rows, 0)` / `pop_clip()` (was
    `Surface::rows`).
- `draw_dirty` clears a dirty frame with `fill_rect` in the window background colour and keeps the damage bookkeeping
  (`Damage::add`) unchanged.
- `image_key(&Image) -> u64`: a content hash of the size and the pixels, used as `ImageRef::id`, so equal pixels give
  equal ids and the GPU side can reuse a texture.
- `blit(&mut dyn Canvas, &Image, x, y)`: draws a rendered image (text, glyph images) at its top-left corner with
  `Canvas::image` at full opacity. Replaces `Surface::blit(.., 255, 255)`.
- `px_rf(ui, rect)`: the device-pixel rectangle of a point rectangle, as a `RectF`.

### Controls and forms (`view/controls.rs`, `view/forms.rs`)

- Every `draw` takes `&mut dyn Canvas`. `fill_rrect` / `stroke_rrect` calls take a `RectF` (the same numbers as before,
  built with `RectF::new`). `fill_rect` calls take a `RectF` built from the old `usize` values with `trunc()`.
- Text is still rendered by `Ui::text` into an `Image` and drawn with `blit`. Nothing about text layout changed.
- `draw_text`, `focus_ring`, `dot`, `chevron`, and the control draws are unchanged in logic.

### Animation (`view/animation.rs`)

- The `Backdrop` save-and-blend helper is removed. Group opacity is now `push_opacity` (see above), so the CPU and
  the GPU path share one behaviour. Its test was rewritten to check a 50% white square over black through
  `Canvas::push_opacity` on a `Surface`: the result is mid grey (0x80 within 1), and pixels outside it are untouched.

### App adapter (`view/app.rs`)

- `ViewApp::draw`, `ViewApp::draw_at` take `&mut dyn Canvas`. An app with views returns `true` from `draws_canvas` and
  forwards `draw_canvas` to `draw_at`; the doc example shows the three lines. Apps that still implement `draw` with a
  `&mut Surface` keep working (the `Surface` coerces).

### Tests (`view/mod.rs`, module `canvas_tests`)

Seven tests, expected values worked out by hand in the test bodies:

| Test | What it fixes |
|---|---|
| `image_key_is_a_hash_of_the_size_and_the_pixels` | equal images give equal keys; a changed pixel or a changed shape gives a different key |
| `blit_draws_source_over_at_the_top_left_corner` | a 2x2 white image at (1, 1) on a 4x4 black surface paints pixels (1..3, 1..3) only |
| `blit_past_the_edge_is_clipped_not_wrapped` | at (3, 3) only the corner pixel changes, the next row is not touched |
| `a_background_fills_the_frame_opaque` | a root view's background covers the whole 4x4 frame |
| `a_hidden_or_fully_transparent_view_paints_nothing` | hidden and opacity 0 leave only the window background |
| `group_opacity_mixes_the_subtree_with_what_is_behind_it` | 50% white over the window background gives (255 + bg) / 2 per channel, within 2 |
| `replaying_the_recorded_tree_matches_drawing_it_directly` | a tree recorded into a `DisplayList` and replayed into a `Surface` equals the direct draw, pixel for pixel |

The existing tests in `view/tests*.rs` (scroll clipping and offset, dirty repaint, popup overlays, progress animation,
focus, undo) are unchanged except for the `Surface` import they need. They draw through `Surface`, which now runs the
`Canvas` path, so they cover the new route too.

## Contract changes

Breaking for in-tree code only where a signature changed, and every caller in the tree compiles through coercion:

- `View::draw(&mut self, s: &mut dyn Canvas, ui: &Ui)` and `View::draw_overlay(...)`: was `&mut Surface`.
- `Window::draw(&mut self, c: &mut dyn Canvas, ui: &Ui) -> Damage`: was `&mut Surface`.
- `ViewApp::draw`, `ViewApp::draw_at`: `&mut dyn Canvas`.
- New `pub(super)` helpers in `view/mod.rs`: `image_key`, `blit`, `px_rf`. Removed: `animation::Backdrop`.
- `scroll.rs` (owned by `sdk-layer-views`) changed only its `draw_overlay` signature and the knob's `fill_rrect` call,
  because the trait changed. Its layout and behaviour are untouched.

## Known gap: the build needs the kit-canvas merge

`cargo check -p bifrost-app --tests` on this branch fails at three call sites in `view/forms.rs` (lines 507, 793, 801):
`widgets::outline_if_borders`, `widgets::popup_button` and `widgets::menu` still take `&mut Surface`. They are owned by
`kit-canvas` (`sdk/src/widgets.rs`), which changes them to `&mut dyn Canvas` with the same arguments. Those are the only
errors; the rest of the crate, including all tests, type-checks. This branch was not merged with that work, so it
cannot pass the check alone. After both branches merge the check passes.

## todo_left

1. `kit-canvas` (widgets) must land first, see above.
2. The SDK runtime (`sdk/src/runtime.rs`, `sdk-direct-runtime`) must call `App::draw_canvas` when `draws_canvas()` is
   true, on the direct path with a `bifrost-2d` target and on the shared-memory path with a `Surface`. This branch does
   not touch the runtime.
3. No app returns `draws_canvas() == true` yet (`app-canvas-pilot` does the first one).
4. `image_key` hashes the pixels on every draw. Text images are rebuilt by `Ui::text` on each frame; a keyed text cache
   (the glyph atlas, `bifrost-2d`) makes this a lookup. The hash is O(pixels) per draw of a text image.
5. The CPU `Surface` implements `push_clip` and `push_opacity` with a scratch layer per op (see
   `docs/graphics/g1-canvas.md`). A scroll view's rows now cost one scratch composite per child op instead of a band
   copy. Measure it on Midget before G3 moves the scroll views to the GPU path.
6. Group opacity used to cost one copy of the frame. It now costs one scratch layer per faded subtree, sized to the
   draw's device box. Same order of magnitude; not measured.
7. `Window::draw` still repaints the whole window on any wide dirty view (menus, pop-ups), as before. Layer-backed
   views (`sdk-layer-views`) remove this.
8. The three widget call sites above still need the `kit-canvas` signatures; until then the view module cannot be
   built alone.

## How to verify

Host (after the kit-canvas merge; no `cargo test` was run on this branch, by the wave rules):

```
cd springboard
CARGO_TARGET_DIR=... cargo check --offline -j 2 -p bifrost-app --tests
CARGO_TARGET_DIR=... cargo test --offline -p bifrost-app view::
python3 -I ../tools/check-names.py
```

The expected results are the test expectations in `view/mod.rs` (`canvas_tests`) and the existing `view/tests*.rs`
suite, all passing. `view::canvas_tests::replaying_the_recorded_tree_matches_drawing_it_directly` is the key check: a
recorded view tree must replay identically to a direct draw.

Midget (Intel HD 530, Mesa ANV, Vulkan 1.4). The session runs labwc unless `/etc/bifrost/compositor` says
`bifrost-wm`. Visual change must be zero for the existing apps, since they still draw through `Surface`:

1. `bifrost pc publish`, then `bifrost pc midget ota` until `phase=accepted`.
2. Open Settings and another view-based app and take `bifrost pc midget screen /tmp/g2-views.png`. Compare
   with the previous release: the pixels match, including the focus ring, the Switch and Segmented control, the
   pop-up menu, the text field caret and the scroll knob.
3. Scroll a long list (Settings or Files) and drag a scroll view: the content clips at the frame edges as before.
4. Open a sheet with a fade (menu or a pop-up) and check the group opacity: the faded area blends into the window with
   no seam.

Once the SDK runtime runs `draw_canvas` on the direct path (todo 2), repeat step 2 with `BIFROST_DIRECT=1` and compare.
