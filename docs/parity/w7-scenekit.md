# SceneKit parity, wave 7: node actions and scene views

Framework: SceneKit (deprecated by the reference SDK in favour of RealityKit). Official doc:
https://developer.apple.com/documentation/scenekit

## What was built

`springboard/ui/src/scene_action.rs` (`bifrost_ui::scene_action`, pure model, std plus `linalg`):

- `Action`: a timed leaf (`Animate`) with an `Effect` (wait, move by or to, rotate by or to, scale by or to), a
  duration and a `Timing`; or `Sequence`, `Group` and `Repeat` (a fixed count, or forever). Constructors mirror the
  reference names (`wait`, `move_by`, `move_to`, `rotate_by`, `rotate_to`, `scale_by`, `scale_to`, `sequence`,
  `group`, `repeat`, `repeat_forever`). `duration()` gives the total time (`None` when it never ends), `reversed()`
  undoes relative actions and returns `None` for absolute ones, and `with_timing` replaces the curve.
- `ActionRunner`: `run(node, action, key)` (a key replaces a running action with the same key), `has_actions`,
  `remove_action`, `remove_all_actions`, and `step(scene, dt)`, which writes the new local pose through
  `Scene::set_pose` and returns how many actions finished. Negative or NaN time counts as zero, and a node that no
  longer exists ends its actions.

`springboard/ui/src/scene_view.rs` (`bifrost_ui::scene_view`, pure model):

- `SceneView`: the view size and its point of view (a camera node). `project_point` and `unproject_point` map between
  world points and screen points (top-left origin, depth 0 at the near plane and 1 at the far plane). `ray` gives the
  ray through a screen point, and `hit_test` returns the candidate nodes whose bounding spheres the ray crosses,
  nearest first.

Both are reachable from apps as `bifrost_app::controls::scene_action` and `bifrost_app::controls::scene_view`.

## Decisions

- Relative leaves (`MoveBy`, `RotateBy`, `ScaleBy`) add only the increment since the previous step, so two
  relative actions on one node add up. Absolute leaves interpolate from the pose the leaf first saw.
- Rotations are about the node's local axes, as the reference `rotateBy` works on the node's own axes.
- Timing curves are the common quadratic ease forms. The reference framework's exact curves are not copied, so
  intermediate frames differ slightly from Apple's.
- The view takes its aspect from its own size, so a horizontal field of view keeps its angle when the window is
  resized. Without a point of view nothing projects.
- Hit testing uses a bounding sphere per candidate, because nodes carry no geometry here. A per-triangle hit test
  (the reference `hitTest` on geometry) is not built and stays a gap.

## Limits

- Not wired into the shell or an app. Host unit tests only; not run on the VM or Midget.
- No `SCNAction` speed, pause, `customAction`, `removeFromParentNode` or `fadeOpacity` (nodes have no opacity).
- The runner has no `SCNActionPlayer` speed or timing-group control; repeat counts are exact.
- No geometry on nodes, so hit testing is approximate and no render result is produced.

## Official API names mirrored

| Reference name | BifrOSt name | Official page |
|---|---|---|
| `SCNAction` (move, rotate, scale, wait, sequence, group, repeat) | `Action`, `Effect`, `Timing` | https://developer.apple.com/documentation/scenekit/scnaction |
| `SCNAction.reversed()` | `Action::reversed` | https://developer.apple.com/documentation/scenekit/scnaction/reversed() |
| `SCNNode.runAction(_:forKey:)`, `removeAction(forKey:)`, `removeAllActions()`, `hasActions` | `ActionRunner::run`, `remove_action`, `remove_all_actions`, `has_actions` | https://developer.apple.com/documentation/scenekit/scnnode |
| `SCNSceneRenderer.projectPoint`, `unprojectPoint`, `hitTest` | `SceneView::project_point`, `unproject_point`, `hit_test` | https://developer.apple.com/documentation/scenekit/scnscenerenderer |

## Tests

Host unit tests are in the `#[cfg(test)]` modules of both files (timing curves, durations, move, sequence, group,
repeat, rotation, scale, keys, reversal; projection, unprojection, ray, hit test ordering). They were written but
not run in this wave.
