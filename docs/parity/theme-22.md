# Theme 22: agents, goals and behaviors

## What was built

`springboard/ui/src/agents.rs` (pure Rust, std only, `pub mod agents;` in `springboard/ui/src/lib.rs`).
A 2D steering layer for game logic:

- `Vec2`: point and direction arithmetic (add, subtract, scale, dot, length, normalise, truncate).
- `Goal`: `Seek`, `Flee`, `ReachTargetSpeed`, `Wander` (seeded jitter), `Avoid` (circular obstacles, predicted
  within a time), `FollowPath` (returns to the path when it drifts beyond the path radius).
- `Behavior`: goals with weights; the weighted sum of their steering. Goals are addressed by index.
- `CompositeBehavior`: behaviors with weights, combined the same way.
- `Path`: polyline with radius and cyclical flag; `nearest`, `point_at_distance`, `length`.
- `Agent`: position, rotation, velocity, `max_speed`, `max_acceleration`, `radius`; `update` and
  `update_with_delegate`, with an `AgentDelegate` trait called before and after each step.
- `Rng`: xorshift64 generator, so a wander simulation is repeatable from its seed.

## Official API names mirrored

Mirrored from the GameplayKit reference: `GKAgent2D` (position, rotation, velocity, update), `GKGoal`
(seek, flee, reach target speed, wander, avoid, follow path), `GKBehavior` (weighted goals,
`setWeight`, `weight`, `remove`, `goalCount`), `GKCompositeBehavior` (weighted behaviors),
`GKPath` (radius, cyclical, point access), `GKAgentDelegate` (will and did update).

## Divergences from the reference

- No `GKAgent3D`: 2D only, as the theme's 2D agents need.
- Goals take points, not other agents. The app updates the target each frame.
- Goals are addressed by index, not by identity, so one goal can be added twice.
- The behavior is passed to `update` rather than stored on the agent, so one behavior can drive many agents.
- `GKPath` is built from points directly. Construction from graph nodes is not included, because the
  pathfinding graph family is a separate gap row.
- Obstacle avoidance covers circles only. Polygon obstacles and avoidance of other agents are not built.
- No interception goal and no mass or force model. Acceleration is limited directly.

## REA evidence

Official rendered pages captured with `tools/parity/rea-page.sh`:

| Page | REA evidence id |
|---|---|
| GKAgent2D | ev_ec3883716cf28f5e083cdd35b0666525af1767ee4f9a6599daf5292853c95b4c |
| GKGoal | ev_416dcdadf89ecc32f546417ee5042adf2a893a80fe319278c72ef380dd62e7f1 |
| GKBehavior | ev_1e8ff40be86d84f2a5fe7b0c299fb51a8b95dc79c71fa9602b9b52272d191256 |
| GKCompositeBehavior | ev_ad82d972c5168b5957427eac84f74fc59e747f2024c2726fe1339bcfb956fe66 |
| GKPath | ev_154aac1697a448b6502f5fda41eb7b84ed44324662566dbfa2fe4e557176cd1d |

GKAgentDelegate was not captured with REA (the capture run did not complete for it); its two callbacks are
taken from the DocC reference (`python3 -I tools/parity/docc.py gameplaykit/gkagentdelegate`).

## Matrix rows closed

| Framework slug | Feature (as in data/<slug>.json) | New status | BifrOSt path |
|---|---|---|---|
| gameplaykit | Agents, goals and behaviors | partial | springboard/ui/src/agents.rs |

SpriteKit and SceneKit rows in this theme are not closed: they need the scene graph, physics and
constraint runtimes, which are outside this change.

## Tests

`cargo` was not run (build rule for this round). The module has 12 behaviour tests in
`agents.rs` (`mod tests`): seek within speed limit, flee, acceleration bound on the first step,
reach target speed, behaviour weight scaling, composite weighted sum, path nearest and distance
walk, cyclical wrap, follow path on an open path, avoid steering sideways, delegate call order,
and seeded wander repeatability.
