# SpriteKit parity, wave 3: emitter, physics and tile map models

Three pure Rust models for the gap rows of the 2D scene framework (`docs/parity-matrix/data/spritekit.json`). Each one
is a model only: there is no scene object, no update loop and no app-facing node yet, so apps cannot use them
through `bifrost_app` until a scene presenter wraps them. All three are std only, deterministic and have no
drawing or clock code.

## What was built

| Module | Mirrors | What it does |
|---|---|---|
| `springboard/ui/src/emitter.rs` | `SKEmitterNode` (particle birth rate, lifetime, speed, emission angle, accelerations, scale and rotation over time, `numParticlesToEmit`, `resetSimulation`, `advanceSimulationTime`) | Seeded xorshift generator (`Rng`). `Emitter::advance(dt)` moves live particles, drops the ones whose age reached their lifetime, then births whole particles from the carried rate. A mean and range pair samples values in `mean - range/2 .. mean + range/2`. |
| `springboard/ui/src/physics.rs` | `SKPhysicsWorld` (`gravity`, `speed`, `body(at:)`), `SKPhysicsBody` (dynamic and static, mass, restitution, gravity flag), `SKPhysicsContact` (the contacts of a step) | `World::step(dt)` applies gravity, moves dynamic circles, then resolves each overlapping pair once: positions are pushed apart by inverse mass and the closing speed is reflected with the smaller restitution. Returns the contacts. `World::body_at` finds the first circle containing a point. Circles only. |
| `springboard/ui/src/tilemap.rs` | `SKTileMapNode` (`tileSize`, `numberOfColumns`, `numberOfRows`, `mapSize`, `anchorPoint`, `tileGroup(atColumn:row:)`, `centerOfTile`, `tileColumnIndex`, `tileRowIndex`), `SKTileSet` | `TileSet` holds named groups. `TileMap` is a grid of `Option<group>` cells with set and read, cell centres, position-to-cell conversion with an anchor, and `neighbour_mask` (north 1, east 2, south 4, west 8) for auto-tiling rules. |

The modules are registered in `springboard/ui/src/lib.rs` with one `pub mod` line each. Each has a
`#[cfg(test)] mod tests` block with hand-worked expected values (for example a body falling 10 points under gravity
-10 for 0.5 s ends at y 7.5 with velocity -5; a circle with penetration 1.5 and restitution 0.5 bounces to +5).

## Official API evidence

Apple facts come from the DocC JSON (`python3 -I tools/parity/docc.py`) and the rendered pages (REA):

| Page | REA evidence id |
|---|---|
| https://developer.apple.com/documentation/spritekit/skemitternode | `ev_c7fff984968441e0a73df0fd6b2b256a5aaf8f898e6fe342bcadc606427655f8` |
| https://developer.apple.com/documentation/spritekit/skphysicsworld | `ev_cd24f6575e5af4b7a6f5fbd4c282f268b25ebdef60a6cca8292fba8b5a76483e` |
| https://developer.apple.com/documentation/spritekit/sktilemapnode | `ev_a7eaffa23bd74ed1d507a2bc5ff3ea93d721204e95faaa62d86b491f919447d7` |

The framework landing page evidence already in the matrix is `ev_9be83454be472ad9b63487454441d5b45d38296db93613aa7ecffa332b60902f`.

## Matrix rows closed

These rows move from `gap` to `partial`. The lead updates `docs/parity-matrix/` centrally.

| Framework | Feature | New status | BifrOSt path |
|---|---|---|---|
| SpriteKit | Particle emitters | partial | `springboard/ui/src/emitter.rs` |
| SpriteKit | Physics simulation | partial | `springboard/ui/src/physics.rs` |
| SpriteKit | Tile maps | partial | `springboard/ui/src/tilemap.rs` |

## Still missing

- No scene object with a per-frame update cycle, and no app-facing node that owns these models.
- Physics: no box or edge shapes, no joints (`SKPhysicsJoint*`), no fields, no contact delegate, no ray or rectangle
  queries. The solver is a single impulse pass per step, not iterative.
- Emitter: no particle textures, colours or keyframe sequences (`SKKeyframeSequence`), no render order, no z position.
- Tile map: no tile definitions or variants, no auto-tiling rule objects (`SKTileGroupRule`), no tile rendering.
