# cg-context: the drawing context

Code: `springboard/render/src/cg/context.rs` (`DrawContext`, `Bitmap`), SDK `bifrost_app::kit::cg` and
`bifrost_app::view::with_context`.

| Area | API | BifrOSt path | Done | Needs Midget / merge |
|---|---|---|---|---|
| State stack | save/restore gstate | `DrawContext::save_gstate/restore_gstate` | whole state (CTM, clip, paints, line attrs, alpha, blend, shadow, text flags) | - |
| Transform | concat, translate, scale, rotate, get CTM, user/device point, size, rect conversion, user-to-device transform | `*_ctm`, `point_to_*`, `size_to_*`, `rect_to_*` | done | - |
| Clipping | rect, rects, path (rule), current path (nonzero, even-odd), image mask, reset, clip bounding box | `clip_to_*`, `clip`, `eo_clip`, `clip_bounding_box` | intersecting 8-bit coverage stack, restored with the state | needs `cg::raster` merged |
| Painting | fill/stroke/eofill current path, draw_path (5 modes), fill/stroke rect(s), ellipse, line segments, clear rect | `fill_path`, `stroke_path`, `draw_path`, `fill_rect`, ... | done | needs `cg::raster` |
| Path building | move, line, curves, arcs, rects, ellipse, lines, add path, copy, bounding box, current point, contains point, replace with stroked | `move_to`..`replace_path_with_stroked_path` | current path kept in device space | arc behaviour depends on `Path::arc` joining from the current point |
| Line attrs | width, cap, join, miter, dash, flatness | `set_line_*` | stored and passed to `stroke_path` | - |
| Paint | fill/stroke colour, RGB, gray, CMYK (device conversion), paint | `set_fill_*`, `set_stroke_*` | solid only until `cg::paint` merges | gradient/shading/pattern arms in `source_of` and `blit_text_image` |
| Compositing | alpha, 28 blend modes (12 separable, 4 non-separable, 12 Porter-Duff/additive) | `set_alpha`, `set_blend_mode`, `blend_pixel` | done | - |
| Shadow | offset, blur, colour; remove | `set_shadow*` | box-blur x3 Gaussian approximation, clipped, under the object | - |
| Quality flags | interpolation, antialias (+ allows), font smoothing/subpixel flags | `set_*` | interpolation + antialias act; font flags stored | glyph renderer has no smoothing switch |
| Transparency layers | begin/end, with rect | `begin_transparency_layer*` | group composited with alpha/blend/shadow of begin time | - |
| Images | draw image (nearest/bilinear, box pre-shrink at Medium/High), tiled, layer at point / in rect | `draw_image`, `draw_tiled_image`, `draw_layer_*` | done | - |
| Text | text matrix/position, font size, char spacing, show text/at point, show glyphs (positions, advances) | `show_text*`, `show_glyphs*` | fill and invisible modes through `render/src/font.rs` | stroke/clip text modes need glyph outlines (private in `font.rs`) |
| Bitmap context | create (w, h, bytes per row), from bytes, data, make image, context | `Bitmap` | done | - |
| Flush | flush, synchronize | no-ops | done | - |

Not mine (other agents): PDF drawing, gradients/shadings/patterns, tone mapping/EDR, rendering intent, colour spaces.

## Apps and shell
- `bifrost_app::view::with_context(canvas, ui, frame, |cx| ...)`: user space is the view frame in points, clipped to the
  frame; drawn offscreen and blitted, so any `Canvas` works.
- Scroll views: a dirty row repaints alone, clipped to the scroll view (`draw_dirty` in `view/mod.rs`); only a scroll view
  with its own background still repaints whole (`ScrollView::opaque_backdrop`).
- Freeform: strokes, dashed outlines and shape outlines go through `ink::stroke_polyline` (DrawContext).
