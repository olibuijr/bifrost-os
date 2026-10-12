# Audio task 24: server driver wiring and restart re-anchoring

Contract: `docs/audio-architecture.md` ("Driver interface", "Clock and timestamps", "IO cycle").

## What changed

| File | Change |
|---|---|
| `springboard/audio/server/src/main.rs` | `AlsaDriver` is registered: `--drivers LIST` or `BIFROST_AUDIO_DRIVERS`, a comma list of `alsa`, `loopback`, `null` (order kept, repeats dropped, unknown names refused with exit 2). Default `alsa,loopback`: the hardware comes first, so ALSA devices are objects `1000 + index` in enumeration order and the loopback pair follows them (on a machine with no card the loopback is 1000). `null` is one stereo device `null:0` on a clock that never moves, for tests. |
| `springboard/audio/driver/src/lib.rs` | New default method `DeviceIo::take_restart(&mut self) -> Option<ZeroTimeStamp>` (None). |
| `springboard/audio/alsa/src/driver.rs` | `AlsaIo` records the zero timestamp of the restart that `recover()` does (xrun, or a resume that failed or lost timing) and hands it out once through `take_restart`; `stop` clears it. Only these lines were touched (task 25 owns the rest of the crate). |
| `springboard/audio/driver/src/null.rs` | `NullHandle::force_restart(uid)` restarts a running device's pointer at the current clock and reports it once through `take_restart`; `NullHandle::restarts(uid)`. |
| `springboard/audio/server/src/io.rs` | Cycle step 7: after `write_mix` the runner polls `take_restart`. On `Some(z)` it re-anchors (`start_host` = the host time of frame 0 of the new stream, `timeline_frames` = `z.sample_time`, so the next cycle is at the restart), bumps the zero-timestamp seed, republishes the zero timestamp to every client at once (rate scalar 1: the DLL starts over) and counts `Shared::restarts`. New `CycleReport::restarted` and `DeviceRunner::seed()`. The IO thread logs the restart. |

The loopback driver never restarts on its own, so it keeps the default (`None`).

Why it matters: before this, after an ALSA restart the hardware counter and the DLL started over at 0 but the
server's timeline kept counting. `next_cycle_ns` asked the reset DLL for the host time of a frame far ahead of the
new stream, so the IO thread slept far too long; clients also kept extrapolating from the old zero timestamp under
the old seed.

## Contract changes (additive)

- `DeviceIo::take_restart` (default `None`).
- The zero timestamp's `seed` changes whenever a device's timeline starts over (a driver restart); clients drop
  their extrapolation when it changes.
- `bifrost-audiod --drivers LIST` and `BIFROST_AUDIO_DRIVERS`.

## Tests (expected values worked out by hand; the lead runs them)

- `bifrost-audio-driver`, null: `a_forced_restart_starts_the_pointer_over_and_is_reported_once` (48000 frames
  after 1 s, 0 right after the restart, 480 after 10 ms more; reported once).
- `bifrost-audiod`, io: `a_driver_restart_republishes_the_zero_timestamp_and_reanchors_the_timeline` (restart 1 ms
  late at 11_666_666 ns: the next cycle is at the restart, the stamp is `{0, 11_666_666, 1.0, seed 2}`; the cycle
  after reads 255 frames and schedules 16_999_999 ns where the old timeline said 16_000_000).
- `bifrost-audiod`, main: driver list parsing and the test drivers' enumeration order.

Packages: `bifrost-audio-driver`, `bifrost-audiod`, `bifrost-audio-alsa`.

## Midget verification

```
bifrost pc publish && bifrost pc midget update --restart
ssh midget 'journalctl -u bifrost-audiod -b --no-pager | grep -E "drivers:|alsa: device|loopback"'
```

Expected: `drivers: [Alsa, Loopback]`, then one `alsa: device alsa:<card>:<n>` line per PCM device before the two
loopback devices. Play a sound from the shell; it comes from the speakers. Force an xrun, for example
`ssh midget 'kill -STOP $(pidof bifrost-audiod); sleep 0.2; kill -CONT $(pidof bifrost-audiod)'`: the journal shows
`stream left RUNNING ... restarting` and then `stream restarted at cycle N, timeline re-anchored (seed 2)`, and
playback continues at once (before this change it stalled after the restart). `BIFROST_AUDIO_DRIVERS=loopback` in a
unit drop-in brings back the loopback-only server.

## Left

- Hot-plug: `DeviceAdded` and `DeviceRemoved` from `AlsaDriver::poll_events` still need `ServerCore` (and the object
  tree) to add and remove device hosts; device ids are indexes fixed at start.
- An aggregate device (task 11) that wraps sub-device `DeviceIo`s should forward its clock master's `take_restart`.
- The IO thread keeps its driver-to-raw clock offset across a restart (both are `CLOCK_MONOTONIC_RAW` for ALSA).
