# Games app: parity wiring (wire/app-games)

Scope: `springboard/apps/games-desktop/` only (version 1.3.0). Matrix rows: `docs/parity-matrix/data/gameplaykit.json`
(features "State machines" and "Randomization"), `docs/parity-matrix/data/game-controller.json` (not wired, see gaps).
The matrix itself is not edited here; the lead updates the rows after the build.

## What is now wired

| Matrix slug | Feature (as in the row) | BifrOSt model | Where it is used in the app |
|---|---|---|---|
| gameplaykit | Randomization | `bifrost_core::random::{RandomSource, Xorshift64}` | `games.rs` (`Games::rand`: 2048 tile spawns, Minesweeper mine placement, Solitaire launch seed) and `solitaire.rs` (`Solitaire::deal`, the shuffle of the 52 cards) |
| gameplaykit | State machines | `bifrost_app::controls::state_machine::{State, StateMachine}` (`springboard/ui/src/state_machine.rs`) | new `src/phase.rs` (`Flow`, `Phase`): the won or lost phase of the 2048 board and of each Minesweeper board |

### Random source

The app had its own inline xorshift64 in `games.rs` and `solitaire.rs`. Both now draw from the core `Xorshift64`
model. The generator is the same algorithm with the same shifts, and the Games `rand` still returns the upper 32 bits,
so every sequence is unchanged: a seeded Solitaire deal, the 2048 spawn positions and the Minesweeper layout match the
previous build. The app keeps its own index arithmetic (`% len`), because `RandomSource::next_below` uses rejection
sampling and would change the sequences. A unit test (`the_games_rng_is_the_same_xorshift_sequence_as_before`) checks
the sequence against the old formula.

### Phase state machine

`src/phase.rs` builds one `StateMachine` per board with three named states, `playing`, `won` and `lost`. The rules:

- `playing` may go to `won` or `lost`;
- `won` may still go to `lost` (2048 can be won and then run out of moves);
- any phase may go to `playing` (a new game);
- nothing else is allowed, so a lost board cannot be won, and a finished board cannot be ended a second time.

`games.rs` calls `go(...)` where it used to assign the phase, and reads `phase()` where it used to compare. The game's
existing guards stay in place, so no move that was legal before is refused now. Unit tests in `phase.rs` cover each rule,
and `a_lost_board_does_not_move_until_restarted` covers the 2048 board.

## How a user reaches it

Open Games from the app launcher (`is.olibuijr.games.desktop`). The state machine has no separate control; it
shows in the game's status message:

- 2048: "You made 2048!" after the first 2048 tile (the game continues), "No moves left" at the end.
- Minesweeper: "Cleared!" when the last safe cell is dug, "Boom" after a mine.

The random source is behind the Solitaire deal and the 2048 and Minesweeper boards; the user sees it as a new game
each launch (the clock seed is unchanged).

## How to verify on the test PC (Midget)

Use Midget, not a VM.

1. Build and publish as usual: `bifrost pc publish`, then `bifrost pc midget ota` until `phase=accepted`.
2. Open the app: `bifrost pc midget open-app is.olibuijr.games.desktop`, then `bifrost pc midget state` to check it runs.
3. 2048: play until "No moves left" appears; the board must not move after that until New Game. Use
   `bifrost pc midget tap-window Games X Y` for a window-local point on the New Game button and check the board clears.
4. Minesweeper: dig until "Boom"; further taps on the board must do nothing until New Game. Dig every safe cell to see
   "Cleared!" and the best time.
5. Solitaire: start a new game and compare two deals with the same clock seed through the testing kit
   (`BIFROST_TEST_PIN=1`); the deal must match the previous build's for the same seed.
6. Host preview: `bifrost pc smoke games-desktop` and `--light`, to check the board and the status line.
7. Screenshot: `bifrost pc midget screen PATH` (a PNG path of your choice) for the record.

Host unit tests (run by the lead, not in this pass): `cargo test -p bifrost-games` (phase, rng and the existing game
tests) and `cargo test -p bifrost-ui state_machine`.

## REA evidence

- `ev_877ebd0689456225972e844aefe8a9459b3e7bb5c9968cb832b44851f3f8f41a`: GameplayKit (recorded in the matrix row).
- `ev_d10b4c315a8e9fdebd29e34558e41f2eed02dcfbf1b36af3a60b325938807e7a`: rendered page of GKStateMachine
  (`rea-page.sh`, this pass). The page describes the state rules: `isValidNextState` per state, `enter(_:)`,
  `didEnter(from:)`, `willExit(to:)`, `update(deltaTime:)`, and `currentState`.
- `ev_f94cc92d31aec5603e9bb08a64625d93a60238dfc2d106a0dc5c47ca473b15d6`: rendered page of GKRandomSource (this pass).
  It says a random source is deterministic and can be replicated, and that the randomization is not cryptographic.

The official outline came from `python3 -I tools/parity/docc.py gameplaykit --depth 2`.

## Remaining gaps

- Gamepad input (matrix `game-controller`): not wired. The SDK `Input` enum has no controller variant (it has `Touch`,
  `Key`, `Open`, `Command`, `Activate`, `Context`, `ContextAction`), and the shell does not forward evdev controller
  events to apps. `bifrost_ui::gamepad::ControllerInput` cannot be fed from the app. This needs an SDK input variant and
  a shell route, which are outside this app's folder.
- Keyboard arrows: `Input::Key` reaches apps, but the Games app ignores it, so 2048 is swipe or drag only. A keyboard
  mapping is a small change once the SDK input route is agreed, and the gamepad D-pad would map the same way.
- Pathfinding (`GridGraph::find_path`): none of the three games moves a unit around a map, so there is nothing to route.
- Procedural noise (`bifrost_ui::noise`): no terrain or texture in these games.
- Sprites, emitters, physics and scene nodes (`scene2d`, `emitter`, `physics`): the boards are drawn into a surface, and
  no game object needs a node model yet.
- Entities and components, strategists (AI opponents), decision trees and spatial partitioning: none of the games
  uses them.
- Solitaire's shuffle uses the index arithmetic of the old code (`x >> 16`, `% (i + 1)`) through the shared generator,
  not `core::random::shuffle`, to keep the deals unchanged. Switching to the shared shuffle would change every seeded
  deal, so it is left for a deliberate change.
- `Cargo.lock` (`springboard/Cargo.lock`, outside this folder) lists `bifrost-games` 1.2.3 without `bifrost-core`. The
  first build updates it; a `--locked` build will fail until it is regenerated.
