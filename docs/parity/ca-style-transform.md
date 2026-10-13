# Layer style and transform parity (ca-style-transform)

Code: `springboard/ui/src/transform3d.rs` (matrix maths), `layer_style.rs` (corner/shadow geometry),
`layer_render.rs` (CPU rasteriser and compositor), `ui/src/layer_bridge.rs` (renders `Layer` trees through it, style keys on layer extra values, hit testing), SDK `springboard/sdk/src/view/style_props.rs` (`ViewBase::set_corner_radius`, `set_shadow`, `set_transform`, `set_mask`, `set_blur`, ... stored on the backing layer) drawn by `view/layer_backed.rs`.

| Row | Done | Needs Midget check |
|---|---|---|
| 3D transform make/concat/invert/rotate/scale/translate/is-identity/equal | `Transform3D` methods incl. `equals` | none (pure) |
| Affine conversion, is-affine | `from_affine`, `to_affine`, `is_affine` | none |
| sublayerTransform | `StyledLayer::sublayer_transform` (about layer centre) | visual |
| Perspective (m34) | `make_perspective`, `project`, homography sampling in `draw_transformed`, behind-eye and back-face culling | visual |
| Transform layer | `StyledLayer::transform_layer` keeps children in shared 3D, depth sorted; otherwise children flatten into the layer plane | visual |
| Corner radius, masked corners, continuous curve | `LayerStyle` + `shape_coverage` (superellipse n=5 for continuous) | compare curve with reference |
| Border | ring between outer shape and inset shape | visual |
| Shadow colour/opacity/offset/radius/path | `render_styled`, `ShadowPath` rounded rect and even-odd polygons | visual |
| Mask layer | `LayerStyle::mask` alpha | visual |
| Filters | Gaussian blur, colour matrix, background blur, background colour matrix | background blur is CPU, check speed |
| Blend modes | 14 modes incl. plus-lighter/darker, `blend_pixel` | visual |
| SDK properties | `ViewBase` setters; a layer tree using any style draws through the styled rasteriser | check in a real app view |
| Hit testing through transforms | `StyledLayer::hit_test`, `unproject`, `layer_bridge::hit_test_layer` (perspective aware) | none |

Not done: GPU path. Style applies to a view's backing layer, not to the view's own `draw` output (the canvas cannot be captured). Shell chrome (window, dock, menus) still uses the canvas shadow/rrect primitives, which are GPU-backed; swapping to the CPU rasteriser would be a regression, so they were left.
