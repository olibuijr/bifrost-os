# G2 `app-canvas-pilot`: the Calculator draws through Canvas

Wave G2 of the graphics core (`docs/graphics-architecture.md`, `docs/graphics-waves.md`, row 11). The first app on
the Canvas path: `draws_canvas()` returns `true` and `draw_canvas` draws the calculator through the
`bifrost_render::canvas::Canvas` vocabulary. One routine serves the CPU `Surface` (shared-memory path) and the GPU
canvas (direct path). App version 1.7.0 (`springboard/apps/calculator-desktop`).

## What is implemented

### App wiring (`src/main.rs`)

- `draws_canvas()` returns `true`.
- `draw_canvas(ctx, canvas, t)` reads `canvas.size()`, calls `Calculator::draw(canvas, &ctx.ui, t)` and returns a
  full-size `Damage`. Before `configure` there is no calculator and nothing is drawn, as before.
- `draw(ctx, s, t)` (the shared-memory entry the SDK still calls on the old path) delegates to `draw_canvas` on its
  `Surface`. `Surface` implements `Canvas`, so the routine is the same.

### Drawing routine (`src/calc.rs`)

- `Calculator::draw` and every helper it calls (`label`, `draw_key`, `draw_convert`, `draw_history`, `draw_menu`,
  `draw_bar`) take `s: &mut dyn Canvas` instead of `&mut Surface`. Surface-only calls became `Canvas` calls:
  - `fill_rect` with whole-pixel bounds (`chrome::whole`, the same flooring as the old `as usize` arguments).
  - `fill_rrect` as `fill_rrect(RectF::new(x, y, w, h), radius, colour)`.
  - `blit(image, x, y, 255, 255)` as `chrome::blit`, which calls `Canvas::image` at full opacity.
- The drawing order, colours (`CARD_BG`, `SIDEBAR_BG`, `LABEL` ...), radii and type sizes (`MAC_*_PT`) are unchanged.
- The test calls (`app.draw(&mut Surface {...}, ...)`) still compile: the `Surface` coerces to
  `&mut dyn Canvas`.

### Calculator chrome on Canvas (`src/chrome.rs`, new)

- `whole(x, y, w, h)`: a rectangle floored to whole pixels and clamped at zero, as `Surface` treats `usize` bounds.
- `image_id(img)`: FNV-1a content hash over the size and pixels. It is the `ImageRef::id` the GPU side caches by, and
  it changes whenever the pixels change (the contract allows a content hash).
- `blit(c, img, x, y)`: one `Canvas::image` call at full opacity.
- `capsule`, `outline_if_borders`: the SDK capsule (shadow ring, plate, edge stroke) on `Canvas`, same numbers.
- `toolbar_band`: the opaque band (content colour and bottom hairline), the non-vibrancy branch of the SDK helper.
- `toolbar`: the SDK's `widgets::toolbar`, with the layout taken from `widgets::toolbar_layout` (the same function the
  SDK draws with, so hit-testing and drawing agree). It keeps the group capsules, the hairline between grouped
  neighbours, the selected segment plate, the pressed fill, the overflow `»` button and the accessibility records
  (`bifrost_app::controls::ax::record`).
- `glyph`: a symbol (the History toggle's `sidebar.left`, a menu chevron) is rasterised by the SDK's `glyphs::glyph`
  onto a scratch `Surface` the size of its bounds, then placed as an image. The glyph code itself still takes a
  `Surface`; see "What is left".

### SDK change (one line, `sdk/src/lib.rs`)

- `kit` re-exports the drawing vocabulary: `pub use bifrost_render::canvas;`. The app crate has no direct dependency
  on `bifrost-render` (no Cargo edits), so the app names `Canvas`, `RectF` and `ImageRef` through `kit::canvas`. This
  is the only change outside the calculator directory.

## How it maps to the contract

| Contract item | Status |
|---|---|
| `draws_canvas` / `draw_canvas` on the `App` trait (skeleton in `sdk/src/lib.rs`) | implemented by the calculator; signatures unchanged |
| One routine on CPU `Surface` and GPU canvas | the calculator's `draw` takes `&mut dyn Canvas`; the CPU path is `Surface: Canvas` |
| `Canvas` vocabulary only, no `Surface` calls in the drawing routine | done for the calculator's own drawing and its toolbar; the glyph rasteriser is the one exception (scratch surface) |
| Images by content id | `chrome::image_id` (content hash) |
| Keep the old path working | `draw(ctx, s, t)` delegates; the shell's switch (`BIFROST_WM_RENDERER`, `BIFROST_DIRECT`, `BIFROST_CHROME`, `/etc/bifrost/compositor`) is unchanged |

## Migration guide: Drawing through Canvas

For apps that want the GPU path. The lead moves this section into `docs/bifrost-ui.md`.

1. Add the override pair to your `App` impl:

   ```rust
   fn draws_canvas(&self) -> bool { true }

   fn draw_canvas(&mut self, ctx: &Ctx, canvas: &mut dyn Canvas, t: f64) -> Damage {
       let (w, h) = canvas.size();
       if let Some(view) = &mut self.view { view.draw(canvas, &ctx.ui, t); }
       let mut d = Damage::new(w as usize, h as usize);
       d.all();
       d
   }

   // Keep the shared-memory entry: the same routine on the CPU surface.
   fn draw(&mut self, ctx: &Ctx, s: &mut Surface, t: f64) -> Damage {
       self.draw_canvas(ctx, s, t)
   }
   ```

   Import the vocabulary with `use bifrost_app::kit::canvas::{Canvas, RectF, ImageRef, PathData, PathPaint, TextRun, Transform2D};`.

2. Take `&mut dyn Canvas` in every drawing helper. Do not take `&mut Surface` in a helper that the canvas path calls.

3. Translate the surface calls:

   | `Surface` (old) | `Canvas` (new) |
   |---|---|
   | `s.fill_rect(x as usize, y as usize, w as usize, h as usize, colour)` | `canvas.fill_rect(chrome::whole(x, y, w, h), colour)` (`chrome::whole` is the app's helper; use `RectF::new` for fractional bounds you want kept) |
   | `s.fill_rrect(x, y, w, h, r, colour)` | `canvas.fill_rrect(RectF::new(x, y, w, h), r, colour)` |
   | `s.stroke_rrect(x, y, w, h, r, width, colour)` | `canvas.stroke_rrect(RectF::new(x, y, w, h), r, width, colour)` |
   | `s.blit(img, x as i32, y as i32, 255, 255)` | `c.image(ImageRef { id: image_id(img), image: img }, RectF::new(x, y, img.w as f32, img.h as f32), 1.0)` |
   | `s.blit(img, x, y, tint, alpha)` with a colour tint | `c.image_tinted(ImageRef { .. }, RectF::new(..), colour)` for a coverage mask |
   | clip to a rounded rectangle, then draw | `c.push_clip(RectF, radius)` ... `c.pop_clip()` |
   | scroll or a moved group | `c.push_transform(Transform2D::translate(dx, dy))` ... `c.pop_transform()` |
   | fade a group | `c.push_opacity(a)` ... `c.pop_opacity()` |

4. Image ids: a rendered text image has no stable id from the text cache yet, so hash its content with
   `chrome::image_id` (or any id that changes when the pixels change). Ids that do not change while the pixels change
   show stale images on the GPU.

5. Coordinates: use device pixels in `Canvas` calls. The old `Surface` code converted points with `ui.pt`; keep that.
   Whole-pixel positions give the same result on both targets; fractional ones are antialiased on the GPU path.

6. Kit widgets that take a `Surface` (`widgets::toolbar`, `toolbar_band`, `source_list`, `glyphs::glyph`) cannot be
   called from `draw_canvas` yet. Copy the drawing you need onto `Canvas` (see `chrome.rs` for the toolbar and the
   glyph bridge), or keep the widget on the old `draw` path until the SDK makes it generic.

7. Test both targets: the helper's `Surface` output on the host (`cargo test` in the app crate) and a Midget screenshot
   on the direct path (below).

## What is left (todo_left)

1. **Titlebar material.** `widgets::toolbar_band` draws a translucent titlebar material when vibrancy is on and
   registers it with the shell (`FRAME_MATERIALS`, private to the SDK). The calculator's band is opaque until the SDK
   takes a canvas. On a vibrancy shell the bar loses its blur. Owner: the SDK (make `toolbar_band`, `toolbar` and
   `glyphs::glyph` generic over `Canvas`, then delete `chrome.rs`'s copies).
2. **Glyph rasteriser.** `glyphs::glyph` draws onto a `Surface` through private helpers (`line`, `disc`, `ring`,
   `polygon`, `arc`). The calculator bridges through a scratch surface, so a symbol costs one small allocation per
   frame. Owner: the SDK.
3. **Sub-pixel symbol placement.** The scratch surface sits on whole pixels, so a symbol can move by up to one pixel
   against the old placement. Same owner as 2.
4. **Hashing per frame.** `image_id` hashes every text image each frame (a few thousand pixels per label). It is
   cheap at this size, but the text cache should hand out a stable key so the GPU side skips the hash. Owner: the
   SDK `TextCache`.
5. **Tests not executed here.** The rules forbid `cargo test` for this round. The calculator crate passes
   `cargo check -p bifrost-calculator --tests` (a clean build, no warnings from the calculator files). The new
   `chrome` tests (`whole`, `image_id`, `blit` through a recording `Canvas`) have hand-worked expected values and must
   be run with `cargo test -p bifrost-calculator chrome` before merging.
6. **GPU canvas.** The direct path's canvas is `bifrost-2d`'s executor; until it executes the recorded list, the
   calculator on the direct path needs the GPU build of `bifrost-2d` (see the verification steps).

## How to verify

Host (no `cargo test` in this round; run these when the rules allow it):

```
cd springboard
CARGO_TARGET_DIR=... cargo check --offline -j 2 -p bifrost-calculator --tests
CARGO_TARGET_DIR=... cargo test --offline -p bifrost-calculator chrome
python3 tools/check-names.py
```

Midget (Intel HD 530, Mesa ANV, Vulkan 1.4):

1. Shared-memory path, the default session (labwc unless `/etc/bifrost/compositor` says `bifrost-wm`):
   `bifrost pc publish`, `bifrost pc midget ota` until `phase=accepted`, then open Calculator with
   `bifrost pc midget open-app is.olibuijr.calculator.desktop` and take `bifrost pc midget screen /tmp/calc-cpu.png`.
   Compare with the 1.6.0 screenshot: the keypad, display, history sidebar and toolbar must match.
2. Direct path: set `BIFROST_DIRECT=1` (and `BIFROST_WM_RENDERER=vulkan` with `/etc/bifrost/compositor` set to
   `bifrost-wm`), open Calculator, and take `bifrost pc midget screen /tmp/calc-gpu.png`. The GPU frame must match
   the CPU one within a pixel on whole-pixel positions. Known difference: the toolbar band is opaque on both paths
   until the titlebar material lands (todo 1).
3. Toolbar: `bifrost pc midget tap-window "Calculator" X Y` on History and Scientific. The tap must hit the same
   item as the old path (the layout is shared with `widgets::toolbar_layout`).
4. Look: run the `bifrost-hig` checklist on the two screenshots (LAY, CHR, CTL rules).
