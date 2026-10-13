# Parity: layer tree (slug ca-layer-tree)

Row: "Layer tree (layer, layer delegate, layout managers, constraint layout)".

- API: `Layer` (shared handle), `LayerDelegate`, `LayoutManager`, `ConstraintLayoutManager`, `Constraint`, `LayerContext`, `LayerCmd`.
- Paths: `springboard/ui/src/layer_tree.rs`, `springboard/ui/src/layer_layout.rs`; SDK re-export `bifrost_app::layer_tree`;
  layer-backed views: `ViewBase::layer()` and `springboard/sdk/src/view/layer_backed.rs`.
- Done: bounds, position, anchor point, z position, frame (get/set), transform (3D matrix, 2D part used for geometry),
  contents image with gravity, scale and contents rect, opacity, hidden, masks-to-bounds, background, border; sublayer
  add/insert/above/below/remove/replace/set, superlayer, cycle guard; hit testing; point and rect conversion between
  layers; needs-display / needs-layout / layout_if_needed / display_if_needed; delegate (draw, layout, action_for) and
  an actions table; constraint layout (min/mid/max x/y, width, height; superlayer or named sibling; scale, offset).
- Key paths: `Layer::value` / `set_value` ("position.x", "opacity", "transform.rotation.z", ...). Unknown keys are kept as
  extra values, so style/subclass/animation code can store its own properties; `add_observer` reports every change.
- Rendering: a layer-backed view draws its tree after its own content, clipped to the view frame, before subviews.
- Not done: sublayer transform, mask layer, per-layer rounded corners/shadow (layer styles build on `value` keys),
  hit testing of layers through `Window` (use `Layer::hit_test`), no shell caller migrated (no prior layer code existed).
- Midget: check a layer-backed view with a sublayer, rotation and opacity draws and redraws on change.
