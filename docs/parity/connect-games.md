# Connect: Games app to the system services (gamepad, power, screens)

Scope: `springboard/apps/games-desktop/` only (version 1.3.0, not bumped in this round). The shell, the SDK and the
other apps are not edited. Matrix rows are not edited here; the lead promotes them after the Midget run.

The services are described in `docs/parity/wire-svc-gamepad.md`, `docs/parity/wire-svc-power.md` and
`docs/parity/wire-svc-screens.md`. The app's earlier wiring (the phase state machine and the shared random source) is in
`docs/parity/wire-app-games.md`.

## What is connected

| Service | What the app does with it | Code |
|---|---|---|
| `gamepad` | Lists the controllers at launch (`list`). Follows `connected` and `disconnected` for a notice line. Turns the direction pad and the left stick into 2048 slides, and the South button into "new game" once the game is over. | `src/system.rs` (`Link::start`, `on_reply`, `on_event`, `input`, `stick_dir`), `src/games.rs` (`gamepad_slide`, `gamepad_new_game`, `set_notice`) |
| `power` | Reads the snapshot at launch (`snapshot`: thermal state and Low Power Mode). Follows `powerStateDidChange` and `thermalStateDidChange`. In Low Power Mode, or at a serious or critical thermal state, the Minesweeper clock redraws every 5 s instead of every second. | `src/system.rs` (`reduce_effects`), `src/games.rs` (`set_reduced_effects`, `clock_step`, `tick`, `next_wake`) |
| `screens` | Reads the list at launch (`list`) and follows `changed`. Keeps the main display's visible size in points and reports it in the app's state dump (`games_display_pt`). | `src/system.rs` (`main_size`, `display`), `src/games.rs` (`set_display`, `describe`) |

Calls are made once, when the app opens. After that the shell pushes changes; the app makes no call per frame. A reply
with an error (for example `no service gamepad` on an older shell) changes nothing, and the game runs as before.

Glue flow: `main.rs` (`GamesApp`) calls `Link::start` on the first `configure`, sends each reply to `service_reply` and
each pushed event to `service_event`. `Link` returns an `Act` (`Slide`, `NewGame`, `Notice`, `State`); `GamesApp::handle`
applies it to the game and syncs the power and display state.

## How a user sees it

- **2048**: plug in a controller and the message row shows "Controller connected: <name>" (replacing the swipe hint). Press
  the direction pad or push the left stick to slide the tiles, one slide per press. When the board is lost ("No moves
  left"), South starts a new game. A won board ("You made 2048!") keeps playing; only a lost one counts as over.
- **Minesweeper**: a controller can only start a new game, and only after "Boom" or "Cleared!". There is no cursor
  yet, so the board is still used with the mouse.
- **Solitaire**: a controller can only start a new game, and only after the game is won.
- **Unplug**: "Controller disconnected: <name>". The notice clears on the next touch of the board.
- **Low Power Mode** (Settings or the battery menu): the Minesweeper clock updates every 5 s. The best time is still
  recorded from the exact moment of the winning dig, so a slower redraw never changes a record.
- **Displays**: nothing visible changes; the size is recorded in the state dump only.

## How to verify on the test PC (Midget)

Midget is the test machine (owner rule: no VM). Order: host tests (the lead builds once), then
`bifrost pc midget ota` until `phase=accepted`, then a screenshot.

1. Launch the app: `bifrost pc midget open-app is.olibuijr.games.desktop`.
2. Gamepad: plug a controller into Midget (see `wire-svc-gamepad.md`, step 1). The notice "Controller connected: <name>"
   appears in the message row. In 2048, press the direction pad: one slide per press, and a held direction does not
   repeat. Push the left stick past the middle: one slide per push. Check the directions: pushing the stick down must
   slide the tiles down; if it slides up, the stick's Y sign is inverted (see gaps).
3. Lose a 2048 board (play until "No moves left"), then press South: a new board, score 0. Before the board is lost,
   South must do nothing (the score stays).
4. Unplug the controller: "Controller disconnected: <name>" appears.
5. Power: `cat /run/bifrost/power-state` shows `low_power=0`. Turn Low Power Mode on; the file shows `low_power=1`. Open
   Minesweeper, dig one cell, and watch the clock: it moves in steps of 5 s. Turn Low Power Mode off: it moves every
   second again.
6. Displays: read the app's state dump through its control socket (`bifrost pc control list`, `methods` and `call`; see the
   `bifrost-app-control` skill). The `games_display_pt=<w>x<h>` line must equal the main display's visible size in points,
   and `games_effects` must read `reduced` in Low Power Mode and `full` otherwise. If the app exposes no describe method
   in its control methods, check the value through the screen instead: this step is then recorded as not verified.
7. Host preview: `bifrost pc smoke games-desktop` and `--light`, to check the message row with a notice.
8. Screenshot: `bifrost pc midget screen PATH` for the record.

Host unit tests (run by the lead, not in this round): `cargo test -p bifrost-games` covers `system.rs` (the stick
dead band, the direction pad diagonals, South, the power and thermal events, the screens list and its errors, the
state of a missing service) and `games.rs` (a controller slides only the 2048 board, South starts a new game only once
the game is over, the reduced clock is exact in its record, and the state dump).

## REA evidence

No new Apple documentation was needed: the mapping of the controls is the service's own wire contract, and the power and
display facts are the ones in the earlier wiring notes (`wire-svc-power.md`, `wire-svc-screens.md`, with their REA
evidence ids).

## Remaining gaps

- **Stick and direction-pad sign**: the left stick's Y axis is read as "down positive", as the Linux input layer
  reports it. The direction pad uses the hat convention stated in the shell's gamepad module. Neither is confirmed on a
  real pad yet; check on Midget (step 2).
- **One stick state for all pads**: two controllers pushing at once share one dead-band state. Fine for one player.
- **No controller cursor in Minesweeper or Solitaire**: the games have no cursor or selection model that a direction pad
  could move. Adding a cursor to Minesweeper is a small change in `games.rs`; Solitaire needs a card selection model.
- **No keyboard arrows**: `Input::Key` still reaches the app, and the Games app ignores it. The same slide mapping would
  apply once the key route is agreed. Out of this app's folder.
- **Screens: no layout use yet.** The window is already sized by the shell's `configure`, so the display size is only
  recorded (`games_display_pt`). A "fill the display" layout would need a design decision first.
- **No haptics or rumble, no battery display, no Settings page for controllers.** The power battery change is received
  and ignored: the Games app shows no battery.
- **Low Power Mode lowers the motion, it does not pause the game.** A Minesweeper clock that stops while the window is in
  the background would need a window-focus event the SDK does not deliver to apps yet.
- **Matrix rows**: `gamepad` input is `game-controller` in `docs/parity-matrix/data/game-controller.json`, and the power
  and screen rows are the ones named in the earlier wiring notes. The lead promotes them after Midget.
