# Parity wave 4: SpriteKit gap rows (physics joints, warp geometry, node constraints)

Three gap rows of the 2D scene framework were built as pure Rust models in `springboard/ui/src` (bifrost-ui). None is wired
into an app or the shell yet, and nothing has been built or run on the host or the Midget. The matrix itself is not
edited here: the lead updates `docs/parity-matrix/` from the table below.

## What was built

### Physics joints (extends `springboard/ui/src/physics.rs`)

The wave 3 physics world (circles, gravity, contacts) gained five joint kinds, stored in the world and solved in
`World::step`:

- Spring forces are applied as velocity changes before collisions.
- Pin, fixed, slide and limit joints are positional corrections after collisions, one pass, split by inverse mass.
  A limit joint also removes the speed that pulls its ends further apart.

Constructors on `World`: `add_pin`, `add_fixed`, `add_limit`, `add_spring`, `add_slide`, plus `joints()` and `body_mut()`.
Anchors are stored as offsets from each body's centre, so a joint follows its bodies when they move.

Official API mirrored (Apple DocC, `spritekit/...`):

| Apple API | BifrOSt |
|---|---|
| `SKPhysicsJointPin`, `joint(withBodyA:bodyB:anchor:)` | `Joint::Pin`, `World::add_pin` |
| `SKPhysicsJointFixed` | `Joint::Fixed`, `World::add_fixed` |
| `SKPhysicsJointLimit`, `maxLength` | `Joint::Limit`, `World::add_limit` |
| `SKPhysicsJointSpring`, `damping`, `frequency` | `Joint::Spring` (stiffness and damping, not frequency), `World::add_spring` |
| `SKPhysicsJointSliding`, `lowerDistanceLimit`, `upperDistanceLimit` | `Joint::Slide`, `World::add_slide` |

Deliberate differences: a spring takes `stiffness` and `damping` rather than Apple's `frequency` and `damping`, and its
rest length is the anchor distance at creation. A slide takes an axis and limits, and it has no anchor argument; its
displacement is measured from the offset the bodies had when the joint was made. Joint reaction force and torque,
motors (`rotationSpeed`), pin angle limits and friction torque are not modelled. Circles have no angle, so pin does not
rotate and there is no angle limit.

### Warp geometry (new: `springboard/ui/src/warp.rs`)

`WarpGrid` holds `columns` by `rows` cells as vertices with a source and a destination point. `map` moves a point
from the source rectangle to the destination grid by bilinear interpolation inside its cell. `interpolate` blends two
grids of the same size, which is the animation case. `with_positions`, `replacing_destination`, `replacing_source`,
`source_position` and `destination_position` cover the vertex accessors.

Official API mirrored: `SKWarpGeometryGrid` (`init(columns:rows:)`, `init(columns:rows:sourcePositions:destinationPositions:)`,
`numberOfColumns`, `numberOfRows`, `vertexCount`, `sourcePosition(at:)`, `destPosition(at:)`,
`replacingBySourcePositions(positions:)`, `replacingByDestinationPositions(positions:)`). The `SKWarpable` protocol and
the node that applies a grid are not built; a caller has to apply `map` itself.

Conventions chosen here (Apple's documentation does not state them): vertices are stored row by row, `(columns + 1)`
per row, and the source must be an axis-aligned rectangle. Only the source's corners are used for cell lookup, so the
source must be on a regular grid. Points outside the source are clamped to its edge.

### Node constraints (new: `springboard/ui/src/node_constraint.rs`)

`NodeConstraint` is an enum over one node's position and rotation: `PositionX`, `PositionY` (ranges), `Distance` (a
band around a point), `Orient` (face a point, plus an offset in radians) and `ZRotation` (a range). `apply` returns the
new (position, rotation); `apply_all` runs a list in order.

Official API mirrored: `SKConstraint` (`positionX(_:)`, `positionY(_:)`, `distance(_:to:)` for a point, `orient(to:offset:)`
for a point, `zRotation(_:)`). Distance to another node, orientation in another node's coordinate system, `referenceNode`
and `enabled` are not modelled: the caller passes the target point and decides whether to apply the list. `SKReachConstraints`
(inverse kinematics) is not built.

## Matrix rows closed

Feature text is exactly as written in `docs/parity-matrix/data/spritekit.json`. The new status is `partial`, because
Apple's features listed in the notes above are still missing.

| Framework | Feature | Status | BifrOSt path |
|---|---|---|---|
| SpriteKit | Physics joints | partial | `springboard/ui/src/physics.rs` (`Joint`, `World::add_pin`, `add_fixed`, `add_limit`, `add_spring`, `add_slide`) |
| SpriteKit | Warp geometry | partial | `springboard/ui/src/warp.rs` (`WarpGrid`, `interpolate`) |
| SpriteKit | Node constraints | partial | `springboard/ui/src/node_constraint.rs` (`NodeConstraint`, `apply_all`) |

The `Physics simulation` row is not closed by this work; it stays `partial` as the wave 3 entry.

## REA evidence

Landing page (used for the framework entry): `ev_88f873fa34397c7a3e6decb5eef5d2ee36ea910f7d088bbbc3f7debc6256220e`
(https://developer.apple.com/documentation/spritekit).

Per API mirrored (rendered official page):

| Page | evidence_id |
|---|---|
| `skphysicsjointpin` | ev_31aedf70928d824a65fa77e056f7839ab8fb83ffd407e5a07460ac346b6c265a |
| `skphysicsjointspring` | ev_52a43118b85915a3b02c610887097ebf2f11ce39a253b1846db66f8841d8d9c5 |
| `skphysicsjointsliding` | ev_394fd173869a288f7437bbcb567d01dc1c72baf188f6784e94ca229ad8d2dea0 |
| `skphysicsjointlimit` | ev_d125ac9b85ea0e0ef1ce822dfd6c5b3231713e84a0c5ea6e7bb824a4346713d2 |
| `skphysicsjointfixed` | ev_e02df1fb4dbc823111be4a118aae58611e71ece13064fb9bc29c1243b64a6a09 |
| `skwarpgeometrygrid` | ev_e2c7dea3fac62ce10f6101c2e9fb295f92cc46d12765ea573814e466b30f970a |
| `skconstraint` | ev_f563953fc047273de46c93786d526f310ae6893a480fdf2175dea51addfab5b5 |

## Tests

Each changed or new module has host tests with values worked out by hand from the rule the code implements. They were
not run (the wave rules forbid builds in this worktree).

- `physics.rs`: spring (extension 2, force 20, impulse 2 gives velocities plus and minus 2), pin (error (0, 1) split to
  y 0.5), fixed (offset (3, 0) kept after an error of (0, 1)), limit (overshoot 1 split to x 0.5 and 2.5, speed 1 reduced
  to 0.5 each), slide along the axis (limit 2 with excess 1 split), slide off the axis (error (0, 2) split to y 1).
- `warp.rs`: vertex counts and the unit square, identity mapping, a bent cell (centre maps to (0.75, 0.75)), a moved
  interior vertex (0.25, 0.25) maps to (0.35, 0.35), wrong counts rejected, the source kept on a destination change,
  halfway interpolation.
- `node_constraint.rs`: position x and y clamping, orient (pi / 2, and pi with an offset), distance pushed out to 1 and
  pulled in to (1.2, 1.6), a node inside the band unchanged, z rotation limit, list order.

## Still open in this row family

No node actions or keyframes, no warp application to a node, no reach or inverse-kinematics solver, no joint motors or
reaction forces, and no app-facing scene update loop. The remaining SpriteKit gap rows (scene lifecycle, environmental
nodes, renderer hand-off) are outside this wave.
