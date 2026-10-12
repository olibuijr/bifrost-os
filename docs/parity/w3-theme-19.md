# Theme 19: GL view and drawable (wave 3)

Pure-Rust models in `springboard/ui/src` (bifrost-ui). None of them is wired into apps or the shell, and none has been
built or run on the VM or Midget yet. Each one is a model of a rule; a surface or view has to call it before the feature
counts as working.

## What was built

| Module | Mirrors (official names) | What it models |
|---|---|---|
| `springboard/ui/src/clip.rs` | `CGContext` clip to rectangle (`clip(to:)`, `resetClip()`, save and restore of the graphics state) | A clip area that intersects with each added rectangle. It is `Unclipped`, `Bounded` or `Empty`. A save and restore stack keeps it with the drawing state. Half-open containment and a culling test (`may_paint`) let a painter skip rectangles outside the clip. |
| `springboard/ui/src/gradient.rs` (extended) | `CAGradientLayerType` (`axial`, `radial`, `conic`), `CAGradientLayer` | `GradientType` and `Gradient::layer_t`. Axial projects onto the line from start to end. Radial takes its radius from the distance to the end point and draws a circle, not an ellipse. Conic gives the clockwise fraction of a turn from the direction of the end point. `conic_t` is new. The existing `Gradient` stop ramp is unchanged. |
| `springboard/ui/src/render_loop.rs` | `GLKViewController` (`preferredFramesPerSecond`, `isPaused`, `pauseOnWillResignActive`, `resumeOnDidBecomeActive`), `MTKView` drawing modes (timed and on demand, `setNeedsDisplay`-style requests) | `RenderLoop`: a frame rate clamped to 1 to 120 fps, timed and on-demand drawing, program pause and app-activity pause kept apart, and no catch-up after a late tick. The caller passes the time in seconds, so the model reads no clock. |
| `springboard/ui/src/drawable.rs` | `GLKView` drawable colour, depth, stencil and multisample formats and size, `MTKView` `colorPixelFormat`, `depthStencilPixelFormat`, `sampleCount`, drawable size | `Drawable`: colour, depth and stencil formats, a sample count of 1, 2, 4 or 8, and a size. `validate` refuses a missing colour buffer and a bad sample count. `memory_bytes` counts the buffers, with colour and depth multiplied by the sample count. |

The model names are the bifrost-ui names. Official names appear here only in this doc, as the matrix requires.

## Official facts used

Checked from official DocC (`python3 -I tools/parity/docc.py`):

- `quartzcore/cagradientlayertype`: `axial`, `conic`, `radial` type properties. `CAGradientLayer` has `colors`, `locations`, `startPoint`, `endPoint`, `type`.
- `coregraphics/cgcontext`: `clip(to:)` sets the clipping path to the intersection with a rectangle. `resetClip()` exists.
- `glkit/glkviewcontroller`: `preferredFramesPerSecond`, `framesPerSecond`, `isPaused`, `pauseOnWillResignActive`, `resumeOnDidBecomeActive`, `framesDisplayed`.
- `glkit/glkview`: `drawableColorFormat`, `drawableDepthFormat`, `drawableStencilFormat`, `drawableMultisample`, `drawableWidth`, `drawableHeight`, `enableSetNeedsDisplay`, `display()`.
- `metalkit/mtkview`: `colorPixelFormat`, `drawableSize`, `clearColor`, `delegate`, `preferredDevice`. Its drawing modes are timed and on demand (per the rendered page, REA evidence below). The depth-stencil property name was cut off in the DocC output, so it is not cited.

Rules that are not in the DocC facts are the model's own choices, and the module docs say so:

- Radial uses the distance to the end point as the radius, so the ramp is a circle.
- Conic measures clockwise from the direction of the end point, with angle 0 at the centre.
- The sample count limit of 1, 2, 4 or 8 is the model's rule.
- Bytes per format are the model's rule. Depth24 is stored in 32 bits.

## REA evidence

The rendered official pages were captured with `tools/parity/rea-page.sh` on the headless browser.

| Page | Evidence id |
|---|---|
| `https://developer.apple.com/documentation/quartzcore/cagradientlayer/type` | `ev_7cd7cbcaf29c87b2bc10506b8bf32a0270f1252014e18762b909a69e78236fa8` |
| `https://developer.apple.com/documentation/glkit/glkview` | `ev_c34004bbd82513f3c076ba39fb3f28b6955fb4d41116a57187d2049768c771fc` |
| `https://developer.apple.com/documentation/glkit/glkviewcontroller` | `ev_01e3f464f880ed9493d4d31b298538b7b43193b23decb611d875ec1fc9457b85` |
| `https://developer.apple.com/documentation/metalkit/mtkview` | `ev_779b19a0cefcb2f8914bbfcf5e2cc896f2aa422a947a2763b7a95e97410c0397` |

Not used as evidence: the capture of `https://developer.apple.com/documentation/coregraphics/cgcontext/clip(to:)`
(`ev_8316db83ec1c50235b8d2a2450a79a5188c5450d842233130eaa459ca4a567f3`) returned a "page can't be found" page. The clip
facts come from the official `coregraphics/cgcontext` DocC page instead.

## Matrix rows closed

Statuses are the ones the lead should write. Each row moves from `gap` to `partial`. Each `bifrost` path is the module
that models the feature. None of these is `parity`, because none has been tested on Midget.

| Framework slug | Feature (exactly as in the data file) | New status | BifrOSt path |
|---|---|---|---|
| `core-graphics` | `Clipping` | partial | `springboard/ui/src/clip.rs` |
| `core-animation` | `Gradient layer (CAGradientLayer)` | partial | `springboard/ui/src/gradient.rs` (`GradientType`, `Gradient::layer_t`) |
| `glkit` | `GL view and drawable` | partial | `springboard/ui/src/drawable.rs` |
| `metalkit` | `View management (MTKView, MTKViewDelegate)` | partial | `springboard/ui/src/render_loop.rs` |

Extended but not closed: `glkit` / `Render loop controller` is already `partial`. `springboard/ui/src/render_loop.rs` now
models the frame pacing and the pause rules for it. The status stays `partial` until the loop is wired into the SDK draw
path.

## Not closed, and why

- `Clipping`: the model is not wired into `Surface`, so there is still no clip on the real surface.
- `GL view and drawable`: `drawable.rs` describes the configuration and the memory only. No GL or GPU context is created,
  and apps still have no drawable.
- `View management`: `render_loop.rs` has no delegate and no view object, so it models the drawing decision only.
- The other theme rows (accessible appearance, scrolling charts, vectorised plots, coordinate spaces and the others) are
  outside this change.

## Tests

Each module has its own `#[cfg(test)] mod tests`, with hand-worked expected values:

- `clip.rs`: 8 tests. Intersection, disjoint clip, save and restore, containment, culling and reset.
- `gradient.rs`: 5 new tests on top of the existing ones. Conic quarter turn and wrap, seam offset, centre, axial layer,
  radial layer.
- `render_loop.rs`: 6 tests. Clamping and interval, timed frames, late caller, pause, app activity, on-demand.
- `drawable.rs`: 6 tests. Memory per format, depth and stencil, multisample, sample count, missing colour, size zero.

Commands run: `python3 tools/check-names.py` (exit 0). No cargo build or test was run, by rule.
