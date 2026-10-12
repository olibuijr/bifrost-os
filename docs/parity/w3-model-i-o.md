# Wave 3: Model I/O (3D asset model)

Four pure Rust models for the 3D part of the Model I/O framework. Each one is a data model with no GPU, file or
network code, so it can run on the host and in tests. None of them is wired into apps or the shell yet.

## What was built

| Module | Mirrors (Apple API names) | What it does |
|---|---|---|
| `springboard/ui/src/scene_object.rs` | `MDLObject`, `MDLObjectContainer`, `MDLNamed`, `MDLTransform`, `MDLTransformOp` and the translate, rotate, scale and matrix ops | A named object arena (`SceneObjects`) with parent and child links. Each node has an ordered list of `TransformOp`s, a hidden flag, and a path such as `world/car/wheel`. It computes local and world matrices from `linalg::Matrix4`, finds nodes by path, lists descendants depth first, and reports whether a node is rendered. |
| `springboard/ui/src/view_camera.rs` | `MDLCamera`, `MDLCameraProjection`, `frameBoundingBox(_:setNearAndFar:)`, `look(at:)`, `look(at:from:)`, `projectionMatrix`, `fieldOfView`, `nearVisibilityDistance`, `farVisibilityDistance` | A position, target and up vector with perspective or orthographic projection. It builds the view, projection and view-projection matrices, and frames a bounding box by distance from the field of view. |
| `springboard/ui/src/animated_values.rs` | `MDLAnimatedValue`, `MDLAnimatedScalar`, `MDLAnimatedVector3`, `MDLAnimatedQuaternion` and their `setFloat(_:atTime:)`, `floatValue(atTime:)`, `setFloatQuaternion`, `floatQuaternionValue(atTime:)` methods | A keyframed track of scalars, vectors or rotations keyed by time in seconds. Keys stay sorted, a repeated time replaces the value there, times outside the track hold the end values, scalars and vectors blend linearly, and rotations blend with `Quaternion::slerp` on the short arc. |

All three build on `springboard/ui/src/linalg.rs` (`Vector3`, `Matrix4`, `Quaternion`). `transform3d.rs` is a
2D-style transform model, so it was not reused. The new modules add no crates, no `unsafe`, and no Apple names in
code or comments. The modules are registered in `springboard/ui/src/lib.rs` with one `pub mod` line each.

Official facts were taken from the DocC index of the collection and the object pages it links to
(`python3 -I tools/parity/docc.py modelio`). The object pages for `MDLObject`, `MDLTransform`, `MDLCamera`,
`MDLAnimatedScalar` and `MDLAnimatedQuaternion` returned HTTP 404 on the DocC endpoint when asked by slug, and only the
collection page resolved. Each behaviour is therefore taken from the collection's topic list and from the names of
the members; the exact Apple semantics of `orthographic` extent and the near-plane rule are not claimed.

## Model choices

- Transform order: the first listed `TransformOp` is the outermost one, so `[Translate, Scale]` moves a point by
  the scale and then by the translation. The local matrix is the product of the ops in list order.
- Orthographic extent: `orthographic_height` is the visible height in world units. Apple's own unit for the
  orthographic case is not confirmed here.
- Framing: the box centre becomes the target. The distance is `radius / sin(fov / 2)` for perspective, and the
  near plane is `max(distance - radius, radius * 0.01)` so it stays positive.
- Keyframes: the time type is `f64` (seconds) and values are `f32`, matching the existing `linalg` precision.

## REA evidence

- Model I/O collection page (`https://developer.apple.com/documentation/modelio`):
  `ev_7e1f6cf63a15d49447ab31542ba39010df5325091f32dafdf30f6a703cde7d99`

## Matrix rows closed

Status is `partial` for all four: the models exist and have tests, but nothing in the app, SDK or renderer uses them yet.

| Framework | Feature | New status | BifrOSt path |
|---|---|---|---|
| Model I/O | 3D asset container and object hierarchy | partial | springboard/ui/src/scene_object.rs |
| Model I/O | Transforms (local coordinate space) | partial | springboard/ui/src/scene_object.rs |
| Model I/O | Cameras | partial | springboard/ui/src/view_camera.rs |
| Model I/O | Skeletal animation and animated values | partial | springboard/ui/src/animated_values.rs |

Remaining gaps in these rows: no skeleton, joints or packed joint animation; no 3D file import or export; no
scene-to-GPU drawing; no stereoscopic camera; no lens model (distortion, aperture) in the camera.

## Tests

Each module has a `#[cfg(test)] mod tests`, with 6 to 8 behaviour tests each whose expected values were worked out
by hand: composed translations, operation order, a 90 degree rotation, path lookup, hidden inheritance, depth-first
order, perspective and orthographic matrix entries, the near and far plane mapping, framing distances, linear and
clamped keyframes, and slerp at the halfway time. The tests were not built or run, because this wave is not
allowed to run cargo.
