# GameplayKit parity: pass 1

Framework: GameplayKit (`https://developer.apple.com/documentation/gameplaykit`), macOS 10.11 and later.
Matrix data: `docs/parity-matrix/data/gameplaykit.json`. This pass takes three gaps that the code can cover in
pure Rust with the standard library: randomness, state machines and grid pathfinding.

## What was built

| Gap | Module | What it does |
|---|---|---|
| Randomization (partial) | `springboard/core/src/random.rs` | `RandomSource` trait with `Xorshift64` and `SplitMix64` sources, both seeded and deterministic. `RandomDistribution` gives uniform whole numbers over an inclusive range, with a die helper, real-valued uniform values and coin flips. `GaussianDistribution` gives normal values by Box-Muller. `ShuffledDistribution` draws from a shuffled deck, so each outcome comes once per pass and a pass never starts with the value the last one ended on. `shuffle` is a Fisher-Yates shuffle. |
| State machines (partial) | `springboard/ui/src/state_machine.rs` | `State` trait with `is_valid_next`, `did_enter`, `will_exit` and `update` hooks. `StateMachine` owns named states, enters one at a time, refuses transitions that the current state vetoes without changing anything, and runs the update of the current state only. |
| Pathfinding (partial) | `springboard/ui/src/grid_graph.rs` | `GridGraph` is a rectangle of cells with an origin that may be negative, blocked cells, and optional diagonal steps that never cut a blocked corner. `find_path` runs A* with integer costs (10 per straight step, 14 per diagonal) and an octile estimate, so the route it returns is a shortest one. |

Each module is one `pub mod` line in its crate's `lib.rs` (`springboard/core/src/lib.rs`, `springboard/ui/src/lib.rs`).
Each has a `#[cfg(test)] mod tests` block with real behaviour tests: seed reproducibility, range and bounds,
die faces and swapped bounds, Gaussian mean and spread, shuffled passes, shuffle permutation, state veto,
hook order, update routing, route detours, unreachable and blocked ends, diagonal shortcuts, corner cutting,
and negative origins.

## Official API names it mirrors

- `GKRandomSource`, `GKRandomDistribution`, `GKGaussianDistribution`, `GKShuffledDistribution`,
  `GKARC4RandomSource`, `GKLinearCongruentialRandomSource`, `GKMersenneTwisterRandomSource`
  (sources: only the two named above are built, and the names are not mirrored)
- `GKStateMachine` (`init(states:)`, `currentState`, `canEnterState(_:)`, `enter(_:)`, `update(deltaTime:)`;
  `state(forClass:)` is not mirrored), `GKState` (the Rust trait hooks are `is_valid_next`, `did_enter`,
  `will_exit` and `update`; their official selector names were not verified in this pass)
- `GKGridGraph` (`init(fromGridStartingAt:width:height:diagonalsAllowed:)`, `node(atGridPosition:)`,
  `gridOrigin`, `gridWidth`, `gridHeight`, `diagonalsAllowed`), with `findPath` from `GKGraph`

The method names in the Rust code are snake_case versions of the official ones. Where the official API takes
a class, the Rust API takes a name, and where it takes a node object, the Rust API takes a `Cell`.

## Official documentation and REA evidence

- Official API shape, read with `python3 -I tools/parity/docc.py`: `gameplaykit` (collection), `gameplaykit/gkstatemachine`,
  `gameplaykit/gkgridgraph`, `gameplaykit/gkrandomdistribution`, `gameplaykit/gkshuffleddistribution`.
- REA evidence of the rendered landing page: `ev_fea645cde0ab03571e429d39e510b36f4e9e8550935e769cff8d32198f8b9e96`
  (title "GameplayKit | Apple Developer Documentation"). The earlier matrix run recorded
  `ev_877ebd0689456225972e844aefe8a9459b3e7bb5c9968cb832b44851f3f8f41a` for the same page.

## Verification status

No build and no test run in this pass, by request. The code is written to compile against the surrounding crates
(`std` only, no `unsafe`, no new crates), and the tests are written but have not been run. The next stage must
build `springboard/core` and `springboard/ui` and run `cargo test` for the three modules.

## Remaining gaps

- Pathfinding: no obstacle graph, mesh graph, custom node classes, or route search over continuous space (`GKObstacleGraph`, `GKMeshGraph`, `GKGraphNode2D`, `GKGraphNode3D`).
- Randomization: ARC4, linear congruential and Mersenne Twister sources are not built by name. The new module is not yet exposed through `bifrost_app` (the SDK), so apps still copy an xorshift generator. `springboard/apps/music-desktop/src/music/upnext.rs` can switch to `shuffle` in a later pass.
- State machines: states are found by name, not by class, and there is no state lookup by type.
- Procedural noise, entity-component systems, rule and decision systems, spatial partitioning (quadtree, octree, R-tree), agents with goals and obstacles, and strategists (minimax, Monte Carlo): none built yet.
- SpriteKit and SceneKit integration: n/a for BifrOSt.

## Value types (parity/team-a-01)

`springboard/ui/src/gameplay_values.rs` adds the value types that the spatial, graph and strategist models use:

- `GAME_MODEL_MAX_SCORE` and `GAME_MODEL_MIN_SCORE`: the integer score bounds (`i64::MAX` and `i64::MIN`).
- `Box3` (box): an axis-aligned three-dimensional box from `min` to `max`, with `is_valid`, `contains` (faces included) and `intersects` (touching faces count).
- `Quad`: the same for two-dimensional rectangles.
- `Triangle`: three 2D points with `signed_area2` (positive for counter-clockwise winding), `center` and `contains` (edges included, either winding).
- `TriangulationMode` (vertices, centers, edge midpoints) and `SplitStrategy` (halve, linear, quadratic, reduce overlap): option sets with no raw values.

These are pure values with host unit tests. They are not yet exposed through `bifrost_app` and are not yet used by `quadtree.rs` or the graph code.
