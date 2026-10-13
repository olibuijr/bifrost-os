# Layer style and transform parity (ca-style-transform)

Code: `springboard/ui/src/transform3d.rs` (matrix maths), `layer_style.rs` (corner/shadow geometry),
`layer_render.rs` (CPU rasteriser and compositor), SDK `springboard/sdk/src/view_style.rs` (`ViewStyle` view properties).

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
| SDK properties | `ViewStyle` builder and `draw` | not yet wired into a real app view |

Not done: GPU path; hit-testing through transforms.
