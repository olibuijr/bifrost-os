# Task 14: bifrost-audio-units and bifrost-audio-engine

Branch `audio/14-units-and-engine`. Contract: `docs/audio-architecture.md` (components and AVFAudio rows of the crate
table). Task: `tasks/audio/14-units-and-engine.md`.

## What is implemented

### `bifrost-audio-units` (`springboard/audio/units`)

| Module | Content |
|---|---|
| `unit.rs` | `AudioUnit` trait (`info`, `tree`, `initialize`, `reset`, `latency_frames`, `tail_frames`, `render(&mut Block)`), `Block` (planar slices, sample time, sorted MIDI), `RenderConfig`, `UnitError`, `check_config` (effects: channels in = out; instruments and generators: no inputs). |
| `param.rs` | `ParamSpec`, `ParamHandle` (control side, atomics, `Send + Sync`), `ParameterTree` (RT side). A change carries a ramp length in frames; `fill` gives per-frame values, `advance` the block-start value. Ramps are exact and independent of how blocks are split. |
| `preset.rs` | `PresetStore`: factory presets 0..n, user presets -1, -2, ...; select applies the values through the handle. |
| `midi.rs` | `MidiEvent` (offset, status, data), packing into a `u32`, `note_to_hz`. |
| `registry.rs` | `ComponentDescription` (three four-character codes, 0 = wildcard), `ComponentRegistry` (`find_next`, `instantiate`), `with_builtins()`. Absorbs `audio_render.rs` registry. |
| `host.rs` | `UnitHost`: in-process hosting; validates frames, channels and slice lengths, resets the unit on a sample-time discontinuity, drops MIDI past the block. |
| `remote.rs` | Out-of-process hosting: `RemoteUnit<T: Transport>` (an `AudioUnit` itself), `Worker`, `LoopbackTransport` (same thread, used by tests and offline), `ShmTransport` and `ShmWorker` on `bifrost-audio-shm` rings plus activation records and the futex trigger. Control planes ride the input ring: a MIDI plane and a parameter plane (`[count, sample_time lo, hi, (id, value, ramp)...]`). |
| `builtin.rs` | `Gain` (ramped), `Biquad` (low pass, high pass, peaking; cookbook design), `SineSynth` (8 voices, MIDI). |

### `bifrost-audio-engine` (`springboard/audio/engine`)

| Module | Content |
|---|---|
| `rt.rs` | Lock-free controls: `NodeHandle` (gain, mute), `EdgeHandle` (connection gain), `PlayerHandle` (play, pause, loop, seek, position, finished), `Tap` (SPSC ring, drops whole blocks when full and counts them), `MidiHandle` (256-message queue). |
| `graph.rs` | `Graph`: nodes Output, Mixer, Player, Effect (any `AudioUnit`: effect, instrument, generator), deterministic topological order, pre-allocated buffers, `render_block` / `render_planar` / `render_interleaved` with no allocation, locks or syscalls. Node gain ramps linearly across each block. Latency and tail time are summed along the longest chain. |
| `engine.rs` | `Engine`: `attach_player/mixer/unit`, `detach`, `connect` (cycle, source and self checks), `disconnect`, `attach_tap`, `prepare`, `render_block` (manual), `render_offline` (frames, deterministic), `start_on_device` / `stop` through `bifrost-audio-hal` `Device::add_io_proc`. `render_into` fills a `BufferList` (non-interleaved or one interleaved buffer). Absorbs the model of `audio_engine.rs`. |

Logging: `eprintln!("is.olibuijr.audio/engine: ...")` on start and device errors (the crate has no dependency on the
core logger; swap when the lead wires it).

### Mapping to the done-when items

* Engine graph (player, EQ, mixer, output): `engine::tests::reference_graph` and `examples/midget_engine.rs`.
* Offline bit-identical across runs: `offline_rendering_is_bit_identical_across_runs`; also independent of the block
  size and of how the render is split (`..._does_not_depend_on_the_block_size`, `rendering_in_pieces_...`).
* Plays on Midget at 128 frames with 0 xruns: needs hardware (steps below).
* Parity rows (AVFAudio, Audio Unit): not updated here (the matrix is the lead's after Midget passes).

## What is left (todo_left)

* Midget run of `midget_engine` (needs the server, HAL and driver crates merged) and the parity-matrix rows.
* `unsafe impl Send for bifrost_audio_shm::Ring` (shm owner): `Ring` holds `&[UnsafeCell<f32>]`, so it is `!Send`; until
  then `RemoteUnit<ShmTransport>` does not implement `AudioUnit` (the loopback transport does). Units has
  `#![forbid(unsafe_code)]`, so the impl belongs in the shm crate.
* Topology changes while running on a device (the graph moves into the IO procedure; only atomics-backed controls move
  after that). A command queue or a graph swap would lift it.
* The shm worker process binary (a `bifrost-audio-unit-host` that maps the memfd, builds a `ShmWorker` and loops
  `cycle()`), and the out-of-process spawn/sandbox policy.
* SDK re-export (`bifrost_app::audio_engine`, one line, the lead) and the DAW move onto the unit trait (separate task).
* Sample-rate conversion for players whose rate differs from the engine's (toolbox converter, task 13).
* Parameter ramps use linear steps; MIDI to instruments is delivered at offset 0 of the next block.

## Verification

Host (the lead runs, from `springboard/`):

```
cargo test -p bifrost-audio-units -p bifrost-audio-engine
python3 ../tools/check-names.py
```

Midget (after the lead merged tasks 03, 05, 06, 07 and pushed with `tools/midget-push-shell.sh`/the audio package):

```
bifrost pc midget state                      # audiod running, default output device present
ssh midget 'cd /opt/bifrost && ./midget_engine 128 30'   # built from: cargo build --profile fast -p bifrost-audio-engine --example midget_engine
# the example prints one line per second: tap frames about 48000 per second, dropped=0
bifrost pc log --subsystem is.olibuijr.audio --category io --since 1m | grep -ci xrun   # expect 0
bifrost pc log --subsystem is.olibuijr.audio --category engine --since 1m               # start line, no errors
```

Repeat with `64` and `256` frames. Pass: 0 xruns at 128 over 30 s, tap `dropped=0`.
