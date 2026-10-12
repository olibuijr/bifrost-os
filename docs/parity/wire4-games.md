# Wire 4: Games (SpriteKit actions for 2048 tile motion)

Scope: `springboard/apps/games-desktop/` (version 1.5.0, package `bifrost-games`) and this note. Matrix row: `docs/parity-matrix/data/spritekit.json`, feature "Actions" (status stays partial until the row is rebuilt on Midget; the lead updates the matrix).

## What is now wired

| Matrix slug | Feature (as in the row) | BifrOSt model | Where it is used in the app |
|---|---|---|---|
| spritekit | Actions | `bifrost_app::controls::anim::{Animation, Curve}` (`springboard/ui/src/anim.rs`) | new `src/tile_motion.rs`: `Sequence` (animation steps played one after another, each starting when the one before ends) and `TileMotion` (one sequence per moving 2048 cell); `games.rs` plans the motion in `shift`, draws the tile scale in `draw_2048` and asks for frames in `tick` and `next_wake` |

### How the tile motion works

- A slide that merges two tiles makes the merged tile swell to 1.12 of its size in 0.08 s, then settle back to 1.0 in another 0.08 s. This is a two-step sequence (`SKAction.sequence` of two scale actions, in the reference SDK's terms).
- A new tile grows from 0.5 to 1.0 in 0.12 s.
- The merged positions come from the new `slide_merges` in `games.rs`, which reports which output positions were merged. `slide` (the old form, now test-only) returns the same values as before, so the slide tests did not change.
- The scale is drawn by shrinking the tile's cell about its centre (`tile_motion::scaled`), so the board needs no node tree.
- Reduced effects (Low Power Mode or a hot device) plan no motion at all: the board updates at once.
- The motion starts at the app clock time of the last touch, tick or draw, and frames are requested every 16 ms while a tile moves.
- Only 2048 is animated. Minesweeper and Solitaire are unchanged.

## How a user reaches it

Open Games from the app launcher (`is.olibuijr.games.desktop`), which opens on 2048. Swipe or use the arrow keys (or a controller's direction pad) to slide. A merge swells the merged tile; the new tile grows in after each slide that moves something. A new game starts without motion.

## How to verify on the test PC (Midget)

Use Midget, not a VM.

1. Build and publish as usual: `bifrost pc publish`, then `bifrost pc midget ota` until `phase=accepted`.
2. Open the app: `bifrost pc midget open-app is.olibuijr.games.desktop`, then `bifrost pc midget state` to check it runs.
3. Slide the 2048 board with `bifrost pc midget tap-window Games X Y` swipes or the key path, and take `bifrost pc midget screen PATH` a few milliseconds after a merge: the merged tile should look slightly larger than its neighbours. A screenshot taken later shows full size.
4. Check the details dump (the Games details dump): `games_tiles_moving` is above 0 just after a slide and 0 about 0.2 s later.
5. Turn on Low Power Mode (or the reduced effects setting the shell reports), slide again, and check `games_effects=reduced` with `games_tiles_moving=0`.
6. Host preview: `bifrost pc smoke games-desktop` and `--light`, to check the board and tile colours.

Host unit tests (run by the lead, not in this pass): `cargo test -p bifrost-games` (the tile motion tests in `tile_motion.rs` and `tile_tests` in `games.rs`, plus the existing slide and rng tests).

## REA evidence

- `ev_aa0a3690ef5f6ca60e2b3b788ddb84b5a2dff6ab881555a1c641993c300c51e2`: rendered page of SKAction (`rea-page.sh`, this pass). It says an action is an animation a node runs to change its position or the scene, with `duration`, `timingMode`, `timingFunction` and `speed`, and names the action initializers and the "Getting Started with Actions" article.
- The official outline came from `python3 -I tools/parity/docc.py spritekit/skaction --depth 1`.

## Remaining gaps

- Sliding tiles are not animated. The board draws cell by cell, so a tile has no path from its old cell to its new one; a real move action needs per-tile identity across a slide.
- No fades: a fill has no alpha in the SDK surface, so a tile cannot fade in.
- Board zoom (the camera): not wired. `scene2d::Scene2d` maps a scene to a view with a scale and anchor, but the board is not a scene, and there is no camera node (`SKCameraNode`); the matrix row "Node tree and camera" stays partial.
- The action model has sequences only. Group, repeat, remove and speed actions (`SKAction.group`, `repeatForever`, `removeFromParent`) are not in the app's `Sequence`.
- Cargo.lock: `springboard/Cargo.lock` still lists `bifrost-games` at 1.4.1. It is outside this app folder, so it is not edited here; the next build updates it (or the lead bumps the entry to 1.5.0).

## Matrix rows

| Slug | Feature (as written in the matrix) | BifrOSt path |
|---|---|---|
| spritekit | Actions | springboard/apps/games-desktop/src/tile_motion.rs (uses springboard/ui/src/anim.rs) |
| spritekit | Node tree and camera | not wired (see gaps); springboard/sdk/src/view/mod.rs |
