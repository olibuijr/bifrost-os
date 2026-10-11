# Wave 3, theme 13: compute streams

Theme: "Compute streams" (`docs/parity-matrix/themes.json`, id `theme-13`). Three pure-Rust models close the two
rows that fit an honest shared capability. Nothing is wired into apps or the shell, and nothing was built or run
(the wave rule is no builds), so the tests are written but not executed.

## What was built

| Module | Mirrors (official API) | What it does |
|---|---|---|
| `springboard/ui/src/work_stream.rs` (`WorkStream`) | Core AI `ComputeStream`: `init()`, work serialized as needed by the values read and written, `currentWorkCompleted()` | Encodes units with read and write resource sets. A read waits for the last write to a resource. A write waits for the last write and for every read since then. Gives each unit a wave (the longest dependency chain), `ready()`, `complete(i)`, `current_work_completed()`, and `pending()`. It schedules and orders work; it does not run it. |
| `springboard/render/src/replicator.rs` (`Replicator`, `Mat4`) | `CAReplicatorLayer`: `instanceCount`, `instanceDelay`, `instanceTransform`, `instanceColor`, `instanceRed/Green/Blue/AlphaOffset` | Instance `i` is the transform applied `i` times (row-major f64 4x4 matrix). Delay is `i * instanceDelay`. Colour is the source times `instanceColor`, plus `i` times the offset, clamped to 0..1. A zero count still draws the source. `preservesDepth` and nested replicators are not modelled. |
| `springboard/render/src/emitter.rs` (`Emitter`, `Cell`, `Particle`) | `CAEmitterLayer` and `CAEmitterCell`: `birthRate`, `lifetime`, `velocity`, `emissionLongitude`, `emissionRange`, `scale`, `seed`, the multipliers for birth rate, velocity and lifetime | Per-cell fractional birth accumulation. A step advances live particles, expires those past their life, then spawns at the emitter position. Velocity follows the emission cone. A seeded LCG makes runs repeatable. Emitter shapes, emitter modes, render order, spin, scale and the cell colour and image ranges are not modelled. |

The compute and replicator and emitter models are pure. None of them uses unsafe code, a new crate or any device access.

## Apple evidence

Official pages read with `python3 -I tools/parity/docc.py` and rendered with `tools/parity/rea-page.sh`:

| Page | Facts used | REA evidence id |
|---|---|---|
| https://developer.apple.com/documentation/coreai/computestream | "A stream of work to be run asynchronously."; `currentWorkCompleted()` waits for all work encoded to the stream; encoded work is serialized as needed by the values read and written | `ev_a105589b0746d07022fcfa643e47e7947177775ec0bbc8d1d7c924796c8636fe` |
| https://developer.apple.com/documentation/quartzcore/careplicatorlayer | `instanceCount` includes the source; `instanceTransform` is applied to the previous instance; `instanceDelay` in seconds; colour offsets per instance, animatable | `ev_c6af9470d17f6fe4e4c6e4a06b373cae59274ebe765b1ad815ec90e3d4a6347e` |
| https://developer.apple.com/documentation/quartzcore/caemitterlayer | Emitter cells, `birthRate`, `lifetime`, `velocity`, `emissionRange`, `seed` and the multipliers | `ev_fef44a896dcb8d7f9dc544e867c1bae7ed8883e808b6627e69177c00da0dfa1a` |

## Tests

Each module has a `#[cfg(test)] mod tests` with hand-worked values:

- `work_stream`: six tests (reads do not serialize; read after write; write after reads; a later write forgets earlier readers; waves follow the longest chain; `complete` respects dependencies and `current_work_completed` retires the rest).
- `replicator`: six tests (zero count draws the source; instance matrices are powers of the transform, `translation(10,0,0)` gives 30 at instance 3; composed transforms, `(1,2,0)` gives `(2,4,0)` at instance 2; linear delays; green offset reaches 1.0 at instance 4 and red clamps; colour multiplier applies before the offset).
- `emitter`: seven tests (8/s at 0.25 s steps spawns 2 per step, first group expires at step 5, eight alive; fractional carry-over; movement of 10 * 0.5 = 5; direction follows the longitude; multipliers, base 4 alive versus 5 with lifetime multiplier 2, and velocity doubling to 10; seeded determinism and range bounds; zero rate and negative step spawn nothing).

They have not been compiled. The first build should run `cargo test -p` on the `ui` and `render` crates.

## Matrix rows closed

Status is `partial` for both: the models exist, but no app, shell or renderer uses them yet.

| Framework slug | Feature (as in `docs/parity-matrix/data/<slug>.json`) | New status | BifrOSt path |
|---|---|---|---|
| core-ai | Compute streams | partial | springboard/ui/src/work_stream.rs |
| core-animation | Replicator and particle layers (CAReplicatorLayer, CAEmitterLayer, CAEmitterCell) | partial | springboard/render/src/replicator.rs, springboard/render/src/emitter.rs |

Gap rows this theme did not close, with the reason:

- Core Animation "Layer tree", "Shape layer" and the "Layers" rows, the Core Graphics CGLayer row, and the other video and colour rows in the theme: they need a path or layer tree, a frame processor or a colour pipeline, and this wave has no shared capability for them.
- Core ML "Stateful models", "Custom layers" and "Loading and running a model", and Core AI "Model cache" and "Ahead-of-time compilation": they depend on the model runtime in `springboard/ui/src/model_session.rs` and `cache.rs`, which this wave did not extend.
- Compression "Multiple-step stream compression": the only inflate is in the books engine (`springboard/src/shell/books/engine/inflate.rs`) and there is no compressor. Building a streaming deflate was out of scope for one theme.
