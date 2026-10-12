# Wave 4, theme-07: geometry and built-in shapes

Pure Rust, std only, in `springboard/ui/src` (bifrost-ui). Nothing is wired into apps or the shell yet.
Host tests are written but were not run: this round did not build (wave rule). Run them in the
integration build before any row is promoted.

## What was built

| Module | Mirrors (official names) | Content |
|---|---|---|
| `spatial.rs` | Point3D, Size3D, Rect3D, Axis3D, Ray3D, SphericalCoordinates3D | `Point3`, `Size3`, `Rect3` (box: contains, intersection, union, center), `Axis3`, `Ray3` (sphere hit, slab box hit), `Spherical3` (radius, inclination from +Z, azimuth in XY) |
| `pose.rs` | Rotation3D, RotationAxis3D, EulerAngles, Pose3D, ScaledPose3D | `Rotation3` (unit quaternion; axis-angle in and out; then, inverse), `EulerAngles` (roll about X, then pitch about Y, then yaw about Z), `Pose3` (apply, to_local, in_parent), `ScaledPose3` (uniform scale) |
| `obstacle.rs` | GKCircleObstacle, GKSphereObstacle, GKPolygonObstacle | `CircleObstacle`, `SphereObstacle` (contains, push_out), `PolygonObstacle` (even-odd contains, nearest outline point), `closest_point_on_segment` |
| `warp.rs` | SKWarpGeometryGrid | `WarpGrid`: uniform or explicit source and destination lattices, bilinear map from source to destination |
| `primitives3d.rs` | SCNBox, SCNSphere | `Geometry3` (positions, normals, u32 indices, bounds), `cuboid_geometry`, `sphere_geometry` (UV), `plane_geometry` |

Each module is registered with one `pub mod` line in `springboard/ui/src/lib.rs`. The 3D vector and
matrix types already exist in `linalg.rs` (`Vector3`, `Quaternion`, `Matrix4`) and are reused, not copied.

## Conventions this crate chose

- Vertex grids: a warp grid of `columns` by `rows` cells has `(columns + 1) * (rows + 1)` vertices. The
  official summary does not say whether its counts are cells or vertices.
- Spherical coordinates: inclination from +Z, azimuth from +X towards +Y. The summary page does not state
  the reference convention, so conversions are only as portable as this choice.
- Warp source lattice must be axis-aligned (strictly increasing columns and rows). Non-axis-aligned
  source grids are rejected. The reference allows arbitrary vertex positions.
- Built-in shapes: no winding enforcement; pole rows of the sphere are degenerate triangles kept for a
  regular index layout. No text or custom-shape geometry (text and shape geometry not built).

## REA evidence (official rendered pages)

- Spatial (landing page): `ev_9a82cba4c0a7f3763358e77009098639637c30e2257a2190d1673f9997c44e23`
- SKWarpGeometryGrid: `ev_f710f8b005a3a758b58ef097160c0fdc3baf98772bde0652de680b6854c9ba8f`
- GKObstacle: `ev_27ff80a24c123c8ac7b5cee1620540d53704ca182e15497210ae17624aaf2f5c`
- SCNBox: `ev_960561773e95f8b344719090dc89033ecab2fd662cd6bb4f16e1dc361b5300db`

DocC facts were read with `tools/parity/docc.py` for the same pages plus Point3D, SphericalCoordinates3D,
Ray3D, GKCircleObstacle, GKSphereObstacle and GKPolygonObstacle.

## Matrix rows closed (status partial; for the lead to apply centrally)

| Framework slug | Feature (as in data JSON) | New status | BifrOSt path |
|---|---|---|---|
| scenekit | Geometry and built-in shapes | partial | springboard/ui/src/primitives3d.rs |
| spatial | 3D vectors and points | partial | springboard/ui/src/spatial.rs |
| spatial | Rotations and poses | partial | springboard/ui/src/pose.rs |
| spatial | Rays and spherical coordinates | partial | springboard/ui/src/spatial.rs |
| spritekit | Warp geometry | partial | springboard/ui/src/warp.rs |
| gameplaykit | Obstacles | partial | springboard/ui/src/obstacle.rs |

Not closed: game-controller "Motion sensing (gyro and accelerometer)" (springboard/core/src/motion.rs
already holds the device-motion estimator from wave 1, so no duplicate was written); scenekit "3D data
types" (Vector3, Vector4, Matrix4 and Quaternion already exist in linalg.rs, but the row was not
re-assessed here).

## Not done

- Single-precision and double-precision variants are one `f32` type per concept.
- Angle2D, coordinate-space conversion and projective transforms are not in this theme.
- No GPU path, no app or shell use, no VM or Midget test.
