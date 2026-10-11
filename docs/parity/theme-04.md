# Theme 04: transforms (CATransform3D, transform matrices)

## What was built

`springboard/ui/src/transform3d.rs` (`bifrost_ui::transform3d`): a pure 4x4 `Transform3D` in the layout of the
reference SDK's layer transform (`m11`..`m44`, row vectors, translation in row 4, perspective in column 4). It is
the shared model that the layer transform, 3D spatial types and transform protocols can use.

- Constructors: `identity`, `make_translation`, `make_scale`, `make_rotation` (right-handed, axis normalised, zero
  axis gives identity), `make_perspective(distance)` (sets `m34 = -1/distance`).
- Composition: `concat(a, b)` applies `a` first, then `b`; `translated`, `scaled`, `rotated` append a step.
- Queries: `invert` (`Option`, `None` when singular, Gauss-Jordan with partial pivoting), `is_affine`,
  `is_identity`, `apply(x, y, z)` (homogeneous divide, `None` when `w` is zero).
- Tests: 6 behaviour tests in `mod tests` (translation layout, concat order, quarter turn and row layout, axis
  normalisation, invert round trip and singular case, perspective divide).

It is not compiled or run in this round (no builds, per the wave rules).

## Official API mirrored

- `CATransform3D` (`m11`..`m44`, the initialiser and the Make/Concat/Translate/Scale/Rotate helpers), reference
  https://developer.apple.com/documentation/quartzcore/catransform3d
- `AffineTransform3D` and `ProjectiveTransform3D` (Spatial, "Affine and projective transforms"), reference
  https://developer.apple.com/documentation/spatial/affinetransform3d (`is_affine` is the affine test; the
  projective case is the general matrix)
- Spatial "Transform3DProtocol" translate, rotate and scale operations, reference
  https://developer.apple.com/documentation/spatial

Apple's `CATransform3DInvert` returns a value for every input; this model returns `Option` instead, which is
the honest form for a singular matrix.

## REA evidence

- `ev_1fae7d28492ea808fac5e1f287c68c4280706bbd279a9f2bf1bc60fee46979cf`: CATransform3D rendered page
- `ev_9b5c2ea521b1212be9fd0cac5b9356c09a10be324f475ea990c097682b5471e7`: AffineTransform3D rendered page

## Matrix rows closed

| Framework slug | Feature (as in data/<slug>.json) | New status | BifrOSt path |
|---|---|---|---|
| core-animation | Transforms (CATransform3D, CATransformLayer, transform matrices) | partial | springboard/ui/src/transform3d.rs |
| spatial | Affine and projective transforms | partial | springboard/ui/src/transform3d.rs |
| spatial | Protocols for translate, rotate, scale, shear | partial | springboard/ui/src/transform3d.rs |

## Still missing (why these are partial)

- `CATransformLayer` flattening and a layer tree that consumes the matrix (the renderer is not wired to it).
- Shear (`Shearable3D`, `CATransform3D` has no shear helper), protocol types, Float variants, `Rotation3D`,
  `Pose3D` and `Volumetric`.
- `ProjectiveTransform3D` as a separate type: the single `Transform3D` covers both cases.
