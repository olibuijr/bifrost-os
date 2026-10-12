# SceneKit parity, wave 3: scene tree, camera and look-at constraint

Framework: SceneKit (deprecated by the reference SDK in favour of RealityKit). Official doc:
https://developer.apple.com/documentation/scenekit

## What was built

`springboard/ui/src/scene3d.rs` (pure model, std only, `bifrost_ui::scene3d`) builds on the existing
`springboard/ui/src/linalg.rs` types (`Vector3`, `Matrix4`, `Quaternion`), which wave 1 added and which are not
duplicated here.

- `Scene`: an arena of nodes. `root()`, `add_child`, `find`, `name`, `parent`, `children`, `local_pose`, `set_pose`,
  `set_constraint`, `set_camera`, `camera`, `world_pose`, `world_position`, `world_matrix`, `camera_view`.
  Nodes are added, never removed, so a `NodeId` stays valid.
- `Pose`: position, orientation (quaternion) and per-axis scale relative to the parent. `compose` places a child pose
  in its parent's space; `matrix` gives scale, then rotation, then translation.
- `Constraint::LookAt { target, world_up }`: turns a node so its local -Z axis points at the target, keeping the
  up direction. The orientation is solved in world space and converted back into the parent's space, so a rotated
  parent is handled. A coincident target or an up vector parallel to the view leaves the node unchanged. A cycle
  of look-at constraints stops after as many hops as there are nodes.
- `Camera`: a perspective camera with a field of view measured horizontally or vertically (`ProjectionDirection`),
  aspect, near and far. `vertical_fov_radians` converts a horizontal angle, and `projection` returns the matrix from
  `Matrix4::perspective`. `Scene::camera_view` is the inverse of the camera node's world matrix.
- `quaternion_from_axes` and `look_at_orientation`: the rotation whose local X, Y and Z point along given directions,
  and the look-at rotation built from it.

Limits, stated plainly: no lights, no materials, no geometry, no billboard constraint, no inverse kinematics, no
skew, no gimbal-lock flag, no physics. Scale composes per axis, so a non-uniform scale under a rotated parent is
approximate. Nothing is wired into the shell or an app yet. Host unit tests only; not run on the VM or Midget.

## Official API names mirrored

| Reference name | BifrOSt name in `scene3d.rs` | Official page |
|---|---|---|
| `SCNNode` (position, rotation, scale, child coordinate spaces) | `Scene`, `Pose` | https://developer.apple.com/documentation/scenekit/scnnode |
| `SCNCamera` (fieldOfView, projectionDirection, zNear, zFar, camera along -Z) | `Camera`, `ProjectionDirection` | https://developer.apple.com/documentation/scenekit/scncamera |
| `SCNLookAtConstraint` (points the negative z-axis at the target, `worldUp`, `target`) | `Constraint::LookAt` | https://developer.apple.com/documentation/scenekit/scnlookatconstraint |

The reference page for the look-at constraint is marked deprecated, and so is the framework. The camera's default
projection direction is not in the fetched text, so `Camera::new` chooses horizontal explicitly and does not claim
the reference default.

## REA evidence

- `ev_264767f12439dbf1103290ef90b074174ebee91d7df825b31e986dca6396794a`: SceneKit framework landing (from the matrix row).
- `ev_b61cf2b0c5be80f057e3d70edd11c3014f18e9b3caa93e92e7f3f9b029638333`: SCNNode page.
- `ev_a4eb83f7d86b73f9c3aacc8589f6375205191119b49f9998d556822566dfd904`: SCNCamera page.
- `ev_89604f2f6fd47c39e4fb72c16820afc2a9936a1f87b96d5b2456bf2327d08dc6`: SCNLookAtConstraint page.

## Matrix rows closed

Status is `partial` for each row, because the reference rows cover more than this model does.

| Framework | Feature | Status | BifrOSt path |
|---|---|---|---|
| SceneKit | Scene graph and nodes | partial | springboard/ui/src/scene3d.rs (`Scene`, `Pose`) |
| SceneKit | Cameras, lights and materials | partial | springboard/ui/src/scene3d.rs (`Camera`, `Scene::camera_view`); lights and materials missing |
| SceneKit | Constraints | partial | springboard/ui/src/scene3d.rs (`Constraint::LookAt`); billboard, inverse kinematics missing |

The 3D data types row is not closed here: `springboard/ui/src/linalg.rs` already covers it (wave 1).

## Tests

`cargo` was not run, as the wave rules require. The tests in `scene3d.rs` (10 in total) work out their expected
values by hand: a child's world position is the parent's position plus the rotated, scaled offset (two tests); a
nested rotation composes parent last (`(1,0,0)` turned about Y is `(0,0,-1)`, unchanged by a turn about Z); the
look-at front axis points at the target, with and without a rotated parent; a coincident target is a no-op; a 90
degree horizontal field of view at aspect 2 gives a vertical focal term of 2 and a horizontal term of 1; the vertical
direction passes through; a camera 10 units up +Z views the origin at (0, 0, -10).
