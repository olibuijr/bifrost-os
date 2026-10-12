# SceneKit parity, wave 4

Three gap rows of the SceneKit framework are closed as pure Rust models in `springboard/ui/src` (bifrost-ui). None is
wired into the shell or an app, and none is tested on the VM or Midget. Nothing here draws 3D content yet.

## What was built

- **Built-in shapes** (`springboard/ui/src/mesh_primitives.rs`): `box_mesh`, `sphere_mesh` and `plane_mesh` return
  positions, normals, texture coordinates and `u32` triangle indices. Each face of a box is a segment grid. Triangles
  wind counter-clockwise from outside. Chamfered box edges, rounded plane corners and the sphere geodesic option are
  not built.
- **Morphing and skinning** (`springboard/ui/src/morph_skin.rs`): `Morpher` blends a base vertex array toward named
  targets with `Normalized` or `Additive` weights. `Skinner` moves vertices by a weighted sum of bone skin matrices
  (pose times inverse bind), after a base bind transform. Normals are not skinned, and skin weights are not
  renormalized.
- **3D data types** (`springboard/ui/src/linalg.rs`): the existing `Vector3`, `Vector4`, `Matrix4` and `Quaternion`
  types cover the vector, matrix and quaternion values. This wave adds `Vector4::ZERO`. The GLKit conversions the
  reference lists are not needed, because the two frameworks share one type here.

## Official API names mirrored

- SCNVector3, SCNVector4 (`SCNVector3Make`, `SCNVector3Zero`, `SCNVector4Zero`, `EqualToVector`)
- SCNBox (`init(width:height:length:chamferRadius:)` without the chamfer), SCNSphere (`init(radius:)`, `segmentCount`), SCNPlane (`init(width:height:)`, segment counts)
- SCNGeometrySource, SCNGeometryElement (layout only, through `mesh.rs`)
- SCNMorpher (`targets`, `weight(forTargetNamed:)`, `setWeight(_:forTargetNamed:)`, `calculationMode`)
- SCNMorpherCalculationMode (`normalized`, `additive`)
- SCNSkinner (`baseGeometryBindTransform`, `boneInverseBindTransforms`, `boneWeights`, `boneIndices`, `skeleton` as a pose)

Morph rule: the official text says the base share depends on the weight sum. This model uses base = 1 - sum(w) in
`Normalized` mode, with weights clamped to 0..1 and rescaled when the sum exceeds one. `Additive` uses weights as given.

## REA evidence (rendered official pages)

- SceneKit landing page: `ev_f5e0c5d560f5cd9314ce1e2de4652c2876fc00c342ccbfecb2914b23a1f7a576`
- SCNVector3: `ev_105eb719cd3df3bf838c9dd4e96c336bb81d4de2a020aca8ed2e9696cf951abc`
- SCNSkinner: `ev_8e17a1989a232f95b06def74a40c639644cb30e057598fa9ffc7433410d2860e`
- SCNMorpher: `ev_635154f0b96ccaeac094e63a41f2518d151827d4b2140824d50b21111432c9eb`
- SCNBox: `ev_03b2d1e8ee677f927318fdc536d94e4329d222b78f0d3bd12785ab27458316a3`
- SCNSphere: `ev_c600474138d101f2f1b5cbab29126f372d45040c6658bf53a8a8cc53031a090b`

The SceneKit landing page marks the framework deprecated in favour of its successor. It is recorded as the reference only.

## Matrix rows closed

| Framework | Feature | Status | BifrOSt path |
|---|---|---|---|
| SceneKit | Geometry and built-in shapes | partial | springboard/ui/src/mesh_primitives.rs (layout in springboard/ui/src/mesh.rs) |
| SceneKit | Skinning and morphing | partial | springboard/ui/src/morph_skin.rs |
| SceneKit | 3D data types | partial | springboard/ui/src/linalg.rs |

## Not built

Physics, particle systems, positional audio, custom shaders, scene import, views for 3D content, and constraints beyond
the existing `scene3d.rs` model. No GPU path reads these results yet.
