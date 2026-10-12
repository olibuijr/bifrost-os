# Wave 5, theme 14: scene lifecycle and game AI

Theme: "Scene and scene lifecycle" (themes.json, theme-14). This wave closes two gap rows with two pure models in
`springboard/ui/src` (bifrost-ui, std only, no unsafe). Both have host unit tests written from hand-worked values. Nothing
here is wired into an app or the shell, and nothing has run on the VM or Midget yet, so the rows are `partial`.

## What was built

- `springboard/ui/src/scene_loop.rs` (`bifrost_ui::scene_loop`): the lifecycle of a game scene (present, dismiss, resize,
  pause) and its frame cycle. A frame returns the ordered steps the caller must run (update, actions, physics,
  constraints, finish). The advance per frame is clamped (default 0.1 s), paused time does not advance the scene clock,
  and a clock that runs backwards counts as no time. The model reads no clock and runs nothing itself.
- `springboard/ui/src/strategist.rs` (`bifrost_ui::strategist`): a `GameModel` trait (players, active player, moves,
  apply, score, win, loss) and two strategists over it. `Minimax` is an exhaustive alpha-beta search to a depth, with a
  deterministic tie rule and a seeded `random_move` among the best N. `MonteCarlo` is a tree search with UCB1 selection and
  random playouts, budget-limited and driven by a seeded xorshift `Rng`. The search clones states instead of undoing moves.

## Official API names mirrored

- SpriteKit scene: `SKScene` (`present` and `dismiss` map to `didMove(to:)` and `willMove(from:)`, `resize` to
  `didChangeSize(_:)`, the frame steps to `update(_:)`, `didEvaluateActions()`, `didSimulatePhysics()`,
  `didApplyConstraints()` and `didFinishUpdate()`). Source: https://developer.apple.com/documentation/spritekit/skscene
- GameplayKit strategists: `GKStrategist`, `GKMinmaxStrategist` (`maxLookAheadDepth`, `bestMove(for:)`,
  `randomMove(for:fromNumberOfBestMoves:)`), `GKMonteCarloStrategist` (budget and exploration are the caller's knobs here),
  `GKGameModel` (`players`, `activePlayer`, `gameModelUpdates(for:)`, `score(for:)`, `isWin(for:)`, `isLoss(for:)`,
  `apply(_:)`). Source: https://developer.apple.com/documentation/gameplaykit/gkminmaxstrategist

Differences from the official APIs, stated plainly:
- `SKScene` is a node-tree object that the framework presents; `SceneLoop` is only the clock and step order. No node, view
  or renderer exists here.
- `GKGameModel` undoes moves with `unapplyGameModelUpdate(_:)`. `GameModel` has no undo; the search copies the state.
- `GKMinmaxStrategist` evaluates with the score for the player to move. Here the chosen player maximises their own score
  and every other player's turn minimises it (a zero-sum view), which matches the two-player case.
- `GKMonteCarloStrategist` scores playouts itself. Here a playout scores 1 for a win, 0 for a loss and 0.5 otherwise,
  for the root player.
- The 0.1 s step clamp and the xorshift generator are BifrOSt choices, not values taken from Apple.

## REA evidence

- `ev_5d3b54dc40089feaeb9ef97909554ea21f04e1d605583423a1d3dc12b01cf69f`: rendered page for `SKScene`.
- `ev_7516df4044a4c4782a77d5ff5c71671355822bc7a9b10652330786f1657318e3`: rendered page for `GKMinmaxStrategist`.

## Tests

`scene_loop.rs` (8 tests): present and dismiss report once, no frame outside presentation, first frame runs the full
cycle with zero delta, delta clamp (0.25 s to 0.1 s, then 0.05 s used as is), paused time not counted (0.05 s, then resume
at 2.05 gives 0.05 s and 0.10 s total), backwards clock, resize changes and negative sizes, and a new presentation restarts
the clock.

`strategist.rs` (9 tests), on a take-away game (take 1 or 2 stones, last stone wins): minimax picks 1 from 4 stones (leaves
a lost pile of 3), picks 2 from 2 stones, returns the first move from 3 stones (every move loses), returns None for the
player not to move and for depth 0, a depth-1 search scores the frontier at 0.5 and so takes the immediate win, random_move
with one candidate equals the best move, random_move among two candidates stays in the set and repeats for a seed,
MonteCarlo with 300 iterations finds the immediate win, a zero budget gives None, and the generator stays in range.

Commands run: none (no builds, per the task). `python3 tools/check-names.py` was run and reported no hits on these files.

## Matrix rows closed

| Framework slug | Feature (as written in data) | New status | BifrOSt path |
|---|---|---|---|
| spritekit | Scene and scene lifecycle | partial | springboard/ui/src/scene_loop.rs |
| gameplaykit | Strategists (turn-based AI) | partial | springboard/ui/src/strategist.rs |

Not closed here, for the lead to weigh:
- scenekit "Positional audio" maps closely to the existing `springboard/ui/src/sound_space.rs` (listener, per-source
  left and right gains, occlusion). That module predates this wave and was not written for it, so no row is claimed here.
- spritekit "Environmental nodes (audio, light)", scenekit "Physics simulation", "Particle systems", "Views for 3D content",
  "Custom shaders and render passes", "Scene asset import", glkit "Shader-based effects" and spritekit "Renderer hand-off
  and custom Metal pipeline" are not addressed: each needs a renderer, a physics world or an asset loader that this wave
  did not build.
