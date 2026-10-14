# SceneKit parity

Official documentation: https://developer.apple.com/documentation/scenekit (macOS 10.8).

Earlier waves wrote pure models for the scene tree (`springboard/ui/src/scene3d.rs`), cameras, constraints and the asset
source. Team A5 found two rows still marked gap with no code, and both now point at real models.

## Views for 3D content (team A5)

`springboard/ui/src/scene_view.rs` models the renderer queries `SCNSceneRenderer` offers: `projectPoint`, `unprojectPoint`
and `hitTest`. A view holds the size of its area and the node whose camera is its point of view. It draws nothing. The
GPU presenter draws 2D surfaces only, so the view has no output path yet.

## Custom shaders and render passes (team A5)

`springboard/ui/src/shader_library.rs` models a shader function table: named entry points, each with a vertex, fragment or
compute stage, and checks that a vertex and fragment pair (or a kernel) fits the pipeline kind. It does not compile shader
code and no render pass API exists.

Built on parity/team-a-05; not yet verified on Midget.

## Remaining gaps

1. No 3D drawing path: `SCNView` and `SCNRenderer` have no output, and the GPU presenter is 2D.
2. No shader compilation, pipeline or technique API for apps.
3. No physics, particle or skinning models.
