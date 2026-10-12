# Wire 6: Games (Tic-Tac-Toe against the strategist model, controller cursor)

Scope: `springboard/apps/games-desktop/` (version 1.6.0, package `bifrost-games`) and this note. Matrix rows are not edited here; the lead updates `docs/parity-matrix/data/`.

## What is now wired

| Matrix slug | Feature (as written in the matrix) | BifrOSt model | Where it is used in the app |
|---|---|---|---|
| gameplaykit | Strategists (turn-based AI) | `bifrost_app::controls::strategist::{GameModel, Minimax}` (`springboard/ui/src/strategist.rs`) | new `src/tictactoe.rs`: the board implements `GameModel` (players X and O, moves are cell indices, score +1 / -1 for a win or loss); the computer's O move is `Minimax::new(9).best_move` |
| gameplaykit | Entities and components | not used | (the 2048 tile motion keeps its own `TileMotion`; no ECS fit) |
| game-controller | Button, axis, switch and direction-pad elements | `system.rs` (controller service, as before) | `games.rs` `gamepad_slide` moves the Tic-Tac-Toe cursor; the South button marks the cursor square while a game is on (it starts a new game only when the game is over, as before) |

The game's phase is the shared `Flow` state machine (`src/phase.rs`, the same machine the other games use): X's line moves the game to `Won`, O's line to `Lost`. A full board with no line is a draw, which `Flow` does not track; `TicTacToe::over` and `result` read it from the board.

### How the game works

- The player is X and moves first. The computer is O and answers at once after each X move.
- Minimax is exhaustive and deterministic: the computer never misses a win, never gives one away, and always takes the first best square. From a centre opening it takes the top left corner; after a corner opening it takes the centre.
- Clicking a square plays it only when the press and the release are on the same square (a drag does not play).
- `New Game` in the toolbar starts over at any time (the same as the other games).
- The win celebration, the message row ("You won!", "The computer won", "Draw") and the Games details dump (`games_ttt_marks`, `games_ttt_phase`) follow the same rules as the other games.

## How a user reaches it

Open Games (`is.olibuijr.games.desktop`) and pick **Tic-Tac-Toe** in the sidebar. The game starts with X to move; click a square. A controller's direction pad moves the cursor square and its South button marks it.

## How to verify on the test PC (Midget)

Use Midget, not a VM.

1. Build and publish as usual: `bifrost pc publish`, then `bifrost pc midget ota` until `phase=accepted`.
2. Open the app: `bifrost pc midget open-app is.olibuijr.games.desktop`, then `bifrost pc midget state`.
3. Click the **Tic-Tac-Toe** row (sidebar), then `bifrost pc midget tap-window Games X Y` on the centre square. The screenshot (`bifrost pc midget screen PATH`) should show X in the centre and O in the top left corner.
4. Check the details dump: `games_ttt_marks=2` and `games_ttt_phase=Playing`.
5. Play the game to the end: a line ends it with "You won!" or "The computer won"; a full board ends it with "Draw". The next `New Game` clears the board.
6. Controller: press the direction pad to move the cursor, then South. The square under the cursor takes an X and the computer answers.
7. Host preview: `bifrost pc smoke games-desktop` and `--light`, to check the board and the marks against the other games.

Host unit tests (run by the lead, not in this pass): `cargo test -p bifrost-games` (`tictactoe::tests`, and `a_controller_moves_the_tic_tac_toe_cursor...`, `a_tic_tac_toe_board_is_not_restarted_by_a_stray_press` in `games.rs`).

Snapshots: adding the sidebar row changes `tests/snapshots/games-desktop-dark.png` and `games-desktop-light.png`. Those are for the lead to bless; nothing was re-blessed here.

## REA evidence

- `ev_71155ee67907fe6c0a4d727da377cc5905952026fc275612280013ea61e224c1`: rendered page of `GKMinmaxStrategist` (`tools/parity/rea-page.sh`, this pass). It says the minmax strategist exhaustively searches the game states, is deterministic, and needs a score for each state; it lists `maxLookAheadDepth`, `bestMove(for:)` and `randomMove(for:fromNumberOfBestMoves:)`.
- Official outline from `python3 -I tools/parity/docc.py gameplaykit/gkminmaxstrategist --depth 1` (the same three members as the evidence).

## Remaining gaps

- The strategist's `MonteCarlo` is not used: Tic-Tac-Toe is small enough for exact search. The shell's `random_move` (pick among the best moves with the seeded generator) is not used either, so the computer always plays the same best square for a position.
- No opponent for 2048, Minesweeper or Solitaire (they are single-player); no state machine beyond the existing `Flow` for Tic-Tac-Toe.
- The controller cursor covers only Tic-Tac-Toe. Minesweeper and Solitaire still have no cursor for a controller.
- Entities and components (`ecs.rs`): not wired; the games keep their own state structs.
- Game-controller haptics, lights and sticks other than the d-pad and left stick are not part of this change.
- `springboard/Cargo.lock` still lists `bifrost-games` at 1.5.0; it is outside the app folder, so it is not edited here.

## Matrix rows

| Slug | Feature (as written in the matrix) | BifrOSt path |
|---|---|---|
| gameplaykit | Strategists (turn-based AI) | springboard/apps/games-desktop/src/tictactoe.rs (uses springboard/ui/src/strategist.rs) |
| game-controller | Button, axis, switch and direction-pad elements | springboard/apps/games-desktop/src/games.rs (`gamepad_slide`, `gamepad_new_game`), fed by springboard/apps/games-desktop/src/system.rs |
