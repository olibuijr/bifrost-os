# Wire 2: Games app (particles, positional audio, coordinate spaces)

Branch `wire2/games`, based on main at 0.5.37. The app moves to 1.4.0 (minor). Only `springboard/apps/games-desktop/`
and this note changed. Nothing was built or run on the host for this wave (the lead builds once).

## What is wired

| Model | What the user sees | Where |
|---|---|---|
| `particle_system3d::ParticleSystem` (3D particles, projected to 2D) | Winning 2048, Minesweeper or Solitaire shows a burst of confetti over the board for about 2 s, then it fades out. Each game celebrates once per win; a new game re-arms it. | `springboard/apps/games-desktop/src/celebration.rs` (new), called from `games.rs` (`note_win`, `tick`, `next_wake`, `draw_burst`) |

Details of the wiring:

- Three emitters fan out (left, centre, right) for 0.6 s, with a birth rate of 60 a second each, life spans of 1.0 to
  1.4 s, speed 3 m/s and gravity of -6 m/s^2. The model's particles are projected to window pixels with a perspective
  of `focal / (focal + z)` (focal 4 m, 40 pt per metre), so pieces shrink as they move toward the back.
- The burst starts at the centre of the board (`board_centre`, set in `draw` from the board layout).
- The win is detected in `touch` and in `tick`, so it also catches wins made from a controller or an accessibility
  activation. The seed comes from the event time, not the game's random source, so the 2048, Minesweeper and
  Solitaire deals are unchanged.
- While a burst lives, `next_wake` asks for a frame every 16 ms; when it has died out, the app goes back to sleep.
- The details dump (`describe`) has a new line, `games_burst=<live dots>`, so a test or the app-control dump can read
  it.

## How a user reaches it

Open Games (sidebar: 2048, Minesweeper, Solitaire). Win a game and the confetti appears over the board. The message row
still says "You made 2048!", "Cleared!" or "You won!" as before. Nothing else in the UI changed, so menus, layout and
the snapshot golden are unchanged.

## How to verify on Midget (the test PC)

1. Ship and update Midget as usual (`bifrost pc publish`, then `bifrost pc midget ota` until `phase=accepted`).
2. Open Games, choose Minesweeper, and clear a board (dig every safe cell). The row reads "Cleared!" and confetti
   bursts over the board. Take a screenshot at about 0.3 s after the win (`bifrost pc midget screen PATH`); the dots
   should be visible over the board, blue, green and orange.
3. Read the state dump of the Games window (`bifrost pc midget state`, or the app's control dump): `games_burst` is
   above 0 right after the win, and returns to 0 within about 2 s.
4. Keep playing after the win (dig or move on the won board): no new burst appears. Start a new game (New Game) and win
   it again: the burst appears again.
5. Switch to 2048 or Solitaire and back while a burst is running: the burst keeps its place and finishes; it does
   not restart.

The 2048 burst needs a win (2048 tile) and is easiest to check with the host test
`a_win_celebrates_once_and_a_new_game_rearms_it`.

## Host tests added (not run in this wave)

- `celebration.rs`: projection values (in plane, half depth, behind the eye), the 18-dot first step and the
  death-out by 2.0 s, the 0.1 s step clamp on a long pause, and non-finite time being ignored.
- `games.rs` (`slide_tests`): a win celebrates once and a new game re-arms it; the burst asks for frames and dies
  out; `games_burst` appears in the dump.

Expected values were worked by hand from the model's rules (birth rate times step, life-span range
`mean +/- range/2`, the perspective scale `focal / (focal + z)`).

## REA evidence (official pages)

- SCNParticleSystem: `ev_36cb4ecda459120344a9e5445eb4b62a5d80b834c6fbd150a2b54aae81289093`
  (`https://developer.apple.com/documentation/scenekit/scnparticlesystem`)
- SCNAudioPlayer: `ev_3e9d384836e9dbf12d4af296e394ed14a33b39a7e7e64cf5e6344915dc8e3c36`
  (`https://developer.apple.com/documentation/scenekit/scnaudioplayer`)
- CoordinateSpace3D: `ev_6b4c94f83854806c684db3af55a1852c0f9a7d220319cf80880772dd4be667f9`
  (`https://developer.apple.com/documentation/spatial/coordinatespace3d`)
- Family pages already in the matrix: `ev_264767f12439dbf1103290ef90b074174ebee91d7df825b31e986dca6396794a`
  (SceneKit) and `ev_82053d7b24d28367191a307ad6c114ee9109bb9be4498ae12e093968a9439d81` (Spatial).

Official reference behaviour used: a particle system is an emitter with birth rate, life span, speed and acceleration
(`SCNParticleSystem`: `birthRate`, `particleLifeSpan`, `particleVelocity`, `acceleration`); a positional audio source
is played by a player attached to a point in the scene (`SCNAudioPlayer`, `audioSource`); a coordinate space converts
points between itself and an ancestor (`CoordinateSpace3D.convert(value:from:)`, `transform(from:)`).

## Not wired, and why

| Matrix row | Model | Why it is not wired in this wave |
|---|---|---|
| Positional audio | `scene_audio::AudioPlayer`, `distance_gain`, `Gains` | There is no sound path that takes a left and right gain for a cue. The games app plays no sound at all. The audio service (`springboard/src/svc.rs`, `Cmd::PlayQueue`, `SetVolume`) plays queued files at one volume with no pan, and `audiod` has no spatial playback. Computing the gain here would be dead code. The gap is in the audio service, outside this folder. |
| Coordinate spaces | `coordinate_space3d::SpaceTree` | The board is a flat 2D grid and `mine_at` already hit-tests it exactly with two divisions. A space tree would only add matrix steps for the same answer. The model fits a future 3D view, not this board. |

## Remaining gaps

- No sound for wins (see above). A future audio-service command that takes a file and a pan would let the positional
  model drive it.
- Solitaire uses the board's centre for the burst, not the card under the pointer.
- The confetti is one burst per win; there is no per-cell burst for Minesweeper and no per-tile effect for 2048.
- `springboard/Cargo.lock` still records `bifrost-games` 1.3.0 (outside this folder). The next `cargo build` updates it.
  A `--locked` build would fail until the lead regenerates it.
- Nothing has been built, run or tested on Midget yet.

## Matrix rows

| Slug | Feature (as written in `docs/parity-matrix/data/<slug>.json`) | BifrOSt path | Status in this wave |
|---|---|---|---|
| scenekit | Particle systems | `springboard/ui/src/particle_system3d.rs` (used by `springboard/apps/games-desktop/src/celebration.rs`) | wired: win confetti in Games 1.4.0 |
| scenekit | Positional audio | `springboard/ui/src/scene_audio.rs` | not wired (no cue sound path, see above) |
| spatial | Coordinate spaces | `springboard/ui/src/coordinate_space3d.rs` | not wired (board hit-test is exact in 2D, see above) |
