# Audio 11: aggregates with drift compensation

Contract: `docs/audio-architecture.md` ("Hog mode, aggregates and taps", design decision 4). Task: `tasks/audio/11-aggregates.md`.

## Implemented

- `springboard/audio/toolbox/src/varispeed.rs` (moved out of `lib.rs`, same API plus `ratio()` and `reset()`): a
  Catmull-Rom resampler with a 3-frame history. The ratio is slewed linearly across each call; `input_needed` plans
  with exactly the same arithmetic as `process`; short input yields silence and fewer used frames, never a panic.
  Latency is 2 / ratio output frames. No allocation after `new`.
- `springboard/audio/server/src/aggregate.rs`:
  - `AggregateDesc` (ordered sub-device UIDs, clock source, per-sub drift flag, `Concat` or `Mirror` mode, optional
    private owner pid) with validation; `AggregateSet` registry (create, remove with owner check, `remove_owner` for
    a client that exits, `visible_to`, `is_member`). Aggregates cannot nest.
  - `Dll`: an alpha-beta tracker learning `d(sub hw frames) / d(clock hw frames)` from `HwPosition::hw_frames`.
  - `AggregateIo` (output): `begin_cycle(hw)` returns the frames each sub-device gets this cycle (`quantum * ratio`
    plus a fractional carry); the ratio is the DLL rate plus a slow correction on `written - expected`, so the loop
    transient never leaves an offset. `render_output` copies for the clock source and uncompensated devices and runs
    `Varispeed` for the rest. `max_frames()` sizes scratch buffers, `fill()` and `latency()` feed accounting.
  - `AggregateIn` (input): per-sub FIFO, DLL rate, FIFO-level correction, `render_input` always yields `quantum`
    frames at the clock rate; underruns give silence and are counted.
- Tests (hand-derived values): registry rules, DLL convergence to 1.0004, ±200/-150 ppm alignment for 1 hour of
  simulated time (relative error under 1 frame after a 20 s warm-up; clock = either device; 3 devices), exact quantum
  for the clock source, resampled counts at +1 %, bounded input FIFO at +300 ppm, varispeed unity delay, ramp, sine,
  ratio-change smoothness. Verified with a standalone `rustc --test` build (all 15 pass); `cargo check` passes.

## Left

- Wiring into the server: the IO thread (task 05/10) must call `begin_cycle` with each sub-device's `position()`
  and `write_mix` the `frames_for(i)` frames from `render_output`; `Concat` channel offsets (sum of preceding subs'
  channels) are the caller's slicing of the mix.
- Control surface: aggregate objects and properties (`FullSubDeviceList`, `ClockDevice`, `DriftCompensation`) in the
  object tree (task 02/05); creating from the Audio Setup app.
- A sub-device that disappears (`DeviceRemoved`) should drop out and the DLL be re-primed on return (`Dll::prime`).
- Reporting `AggregateIo::latency` in the aggregate's latency property.

## Midget verification (lead)

1. Host: `cd springboard && cargo test --offline -p bifrost-audiod aggregate` and `-p bifrost-audio-toolbox varispeed`.
2. After wiring, on Midget create an aggregate of the ALC233 analog output and HDMI/DP pcm=3 (clock source: analog),
   play the 1 kHz test tone for 10 minutes: `bifrost pc midget ...` start the tone, then read the profiler/xrun
   counters from `bifrost pc log` (subsystem `is.olibuijr.audio`, category `aggregate`); expect 0 xruns.
3. Record the two outputs with a loopback or scope; the drift between them must not grow beyond 1 frame over the run.
