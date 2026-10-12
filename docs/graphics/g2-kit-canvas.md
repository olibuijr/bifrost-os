# G2 `kit-canvas`: the SDK kit draws through `Canvas`

Part of wave G2 in `docs/graphics-waves.md` (row 9) and the contract in `docs/graphics-architecture.md`. The SDK's
drawing helpers in `springboard/sdk/src/widgets.rs` and `springboard/sdk/src/glyphs.rs` now take
`&mut dyn bifrost_render::canvas::Canvas`. A `Surface` still works everywhere: `Surface` implements `Canvas`, and a
`&mut Surface` coerces to `&mut dyn Canvas` at every call site, so no app changes to compile.

## What is implemented

### `glyphs.rs`

- Every glyph draws through `Canvas`. The shell's Surface helpers (`springboard::shell::icons::{arc, disc, line,
  polygon, ring}`) are no longer imported. The glyph module has its own versions with the same geometry:
  - `line`: a path of one move and one line, stroked with width `w` (round ends, as the shell's line).
  - `disc`: `fill_rrect` of the bounding square with the radius as the corner radius.
  - `ring`: `stroke_rrect` of the bounding square, `t` wide inside the radius.
  - `arc`: a polyline on the circle (`arc_verbs`: one segment per 22.5 degrees), stroked with width `t`.
  - `polygon`: a closed path filled even-odd (`polygon_verbs`).
- `glyph`, `glyph_effect` and the numeral take `&mut dyn Canvas`.

### `widgets.rs`

- All drawing functions take `&mut dyn Canvas`: `material`, `visual_effect`, `large_title`, `section_label`,
  `outline_if_borders`, `toolbar_band`, `capsule`, `toolbar`, `source_list`, `search_field`, `folder_icon`,
  `search_button`, `path_bar`, `text_field`, `popup_button`, `menu`, `player`, `popover`, `focus_ring`, `sheet`,
  `segmented`, `stepper`, `mini_month`, and the private helpers (`label_text`, `window_material`, `sheet_button`).
- Rectangles go through `RectF` (`fill_rect`, `fill_rrect`, `stroke_rrect`). Images go through `ImageRef`:
  - `blit(c, img, x, y)`: `Canvas::image` at the pixel origin, full alpha. It replaces `Surface::blit(img, x, y, 255, 255)`.
  - `image_tinted_at(c, img, x, y, color)`: `Canvas::image_tinted` (a coverage mask in one colour).
  - `image_id(img)`: a hash of the size and pixels, the stable id the GPU cache keys on.
- `material` uses `Canvas::material` with `MaterialStyle { blur, tint, saturation: 1.0 }`, so the glass goes to the
  window server's material on the GPU path and to `glass_rrect` on the CPU path. Under Reduce Transparency it draws
  the opaque colour, as before.
- Behind-window materials (`window_material`, `visual_effect`) record the material for the shell as before, then
  fill the tint with `Canvas::fill_rrect`.
- Pixel-level helpers stay Surface-only: `blit_vibrant_px` (the exact plus-lighter / plus-darker blend), `pane`,
  `pane_px`, `pane_origin_px`. The two Canvas variants are `blit_vibrant` (below) and `pane` keeps its Surface type
  because it hands back a sub-Surface.
- `disc` and `polygon` calls in the kit go to the glyph module's Canvas versions.

### Tests

- `glyphs::canvas_tests` (9 tests): arc polyline (quarter arc, zero span), polygon outline (triangle, empty),
  line stroke, disc, ring, and a chevron drawing two segments plus an unknown name drawing nothing.
- `widgets::canvas_tests` (7 tests), against `widgets::recorder::Rec`, a `Canvas` that records calls:
  - the image id is stable for the same pixels and differs for other pixels or another shape;
  - `blit` emits one image op at `(x, y, w, h)` with alpha 1;
  - a vibrant label is a tint whose alpha is `round(strength * 255)` for each level;
  - a behind-window material with vibrancy fills its tint and declares one material to the shell;
  - without vibrancy it fills the opaque colour and declares nothing;
  - a within-window material is one `Canvas::material` op with the blur in points and the tint;
  - the toolbar band ends with a one-pixel separator at the bottom edge.
- The existing tests (widgets, glyphs) are unchanged and still pass in logic: they pass a `Surface` and read its
  pixels.

## How it maps to the contract

| Contract item | Status |
|---|---|
| `Canvas` trait and its method list | unchanged |
| `Surface` implements `Canvas` | used as is; the kit's plain-state calls reach the Surface methods |
| Kit drawing through `&mut dyn Canvas` | done for every drawing function in both files |
| Pixel helpers keep a Surface-only variant | `blit_vibrant_px`, `pane` (sub-surface) |
| Images cached by id | `image_id` (content hash) |
| Materials through `Canvas::material` | done; Reduce Transparency keeps the opaque colour |

## Contract changes

None to the contract. Additive public items: `widgets::{blit, blit_vibrant_px, image_id, image_tinted_at}`,
`glyphs::{line, disc, ring, arc, arc_verbs, polygon, polygon_verbs}` (the glyph module's own primitives, previously
private imports from the shell), and `widgets::recorder` (test only). Changed parameter types: every drawing function
in the two files now takes `&mut dyn Canvas`. Callers that pass a `Surface` compile unchanged.

## Known differences from the Surface path (todo_left)

1. **Behind-window tint: over, not replace.** The old CPU path wrote the tint over the window's pixels. Now it is a
   source-over fill. The window's own pixels under a behind-window material are transparent, so the result is the
   same there; an app that draws opaque pixels under a material now shows them through the tint.
2. **Vibrant labels are approximate.** `Canvas` has no plus-lighter or plus-darker blend, so `blit_vibrant` draws the
   text coverage in white (Dark) or black (Light) at the level's strength. The exact blend remains in
   `blit_vibrant_px`. Needed: a blend mode on `Canvas` (or a `text` op with a blend) to match it exactly.
3. **Arcs and lines use the path stroker.** It has round caps, where the shell's arc was a radial clip. Anti-aliasing
   differs by a fraction of a pixel.
4. **`image_id` hashes the pixels on every draw.** Fine for the CPU path. For the GPU path it should become an
   owner-assigned id for text runs, so text is not hashed each frame.
5. **Apps are not checked.** Only `bifrost-app` (the SDK, with tests) was checked. The 21 app crates pass `&mut
   Surface` and should compile by coercion; `cargo check` on them is the lead's step.

## Verify

Host (from `springboard/`, with the shared target dir):

    CARGO_TARGET_DIR=/home/olibuijr/Projects/bifrost-os/springboard/target-agents cargo check --offline -j 2 -p bifrost-app --tests

The lead runs the unit tests: `cargo test --offline -p bifrost-app --lib canvas_tests` (filters the new modules) and
the full `widgets::tests` and `glyphs::tests`, which must pass unchanged.

Host preview of a kit app (CPU path, compare with `main`): `bifrost pc smoke APP` and `bifrost pc smoke APP --light`.
Compare the PNG against the same preview from `main`; the expected differences are the ones listed above.

Midget (Intel HD 530, Mesa ANV, Vulkan 1.4): the session runs labwc unless `/etc/bifrost/compositor` names
`bifrost-wm`. For the kit's Canvas path on the GPU, the window server must be bifrost-wm with the Vulkan renderer:

1. `bifrost pc midget ota` to ship the build, then `bifrost pc midget update --restart` if the session changed.
2. Set the session to the window server: write `bifrost-wm` to `/etc/bifrost/compositor` on Midget (over `ssh midget`),
   then restart the session.
3. Start an SDK app with the switches on: `BIFROST_WM_RENDERER=vulkan BIFROST_DIRECT=1 <app>`.
4. Take a screenshot with `bifrost pc midget screen /tmp/kit.png` and compare with the labwc screenshot of the same
   app. Glyphs, toolbars, the source list and the menu should look the same to the eye.
5. `python3 tools/midget.py recovery` must say `phase=accepted` after any update.

If the app draws blank on the GPU path, the Canvas route is not the cause on its own: the app still draws through the
shell's shared-memory frame path until the app moves to `draw_canvas` (`app-canvas-pilot`, G2 row 11).
