# Task 19: bench instrument and millisecond benchmarks

Crate `springboard/audio/bench` (package and binary `bifrost-audio-bench`, workspace member). It depends only on the
audio crates by path (`types`, `driver`, `alsa`, `units`, `midi`, and `bifrost-audiod` for the latency formulas).

## Pieces

| Module | What |
|---|---|
| `stats` | `summarize`: n, min, max, p50, p99 (nearest rank: rank = ceil(p * n / 100)); jitter = p99 - p50 |
| `onset` | `first_onset`, `onsets(samples, threshold, holdoff)`: exact frame of the first `|x| >= threshold` |
| `schedule` | `Timeline` (zero timestamp + rate + rate scalar: host ns <-> sample), `Placement` (`Later`, `Late`, `At(offset)`), `EventScheduler` (stamped events -> per-cycle offsets) |
| `instrument` | `BenchInstrument` (audio unit "Bench Instrument"), `BenchMidi` ("Bench Source" and "Bench Instrument" endpoints on a `bifrost_midi::hub::Hub`) |
| `rig` | One device on a driver plus the model of the server IO cycle; `TimeSource` (`ManualTime`, `RealTime` = `CLOCK_MONOTONIC_RAW`) |
| `midi_latency`, `roundtrip`, `sweep`, `throughput` | The four subcommands |
| `json`, `args` | The shared output object and the hand-written flag parser |

### Sample-accurate scheduling

The MIDI hub keeps events with their host timestamp. Each cycle the bench drains the destination up to the cycle's end
into the `EventScheduler`; `take_cycle(timeline, cycle_start, quantum)` gives each event its frame offset
(`floor(sample_of(host_ns) - cycle_start)`, with 1e-3 sample of slack for the rounding of host times to whole ns).
An event for a later cycle stays queued; an event already in the past plays at offset 0 and counts as `late`. The unit
renders one sample of 1.0 (every output channel) at each note-on offset and zero elsewhere.

### The IO cycle model (rig)

Cycle `k` wakes at the host time of its first frame. The mix rendered in cycle `k` is written to the device in cycle
`k + 1`; input read in cycle `k` reaches the client in cycle `k + 1`. These are the server's own quantum of buffering,
the `+ quantum` of `bifrost_audiod::latency::latency_frames`, so a measurement can be compared with the HAL
properties. The bench does not run `bifrost-audiod` itself: it drives the driver directly with this model.

## Usage

```
bifrost-audio-bench <midi-latency|audio-roundtrip|xrun-sweep|midi-throughput> [flags] [--json]
  --driver null|loopback|alsa:<card>   default null
  --rate <Hz>  --quantum <frames>  --count <n>  --seconds <s>  --json
  --stress <n>        busy threads (xrun-sweep, real-time drivers only)
  --device <uid>      exact alsa device uid (default: first full-duplex device of the card)
  --interval-ms <ms>  time between notes or impulses (default 20)
  --lead <cycles>     midi-latency: send each note this many cycles before its stamp (default 0)
  --live              against the running bifrost-audiod through the HAL (see "--live" below)
```

`null` always runs on its manual clock (instant, deterministic). `loopback` and `alsa:<card>` run in real time on
`CLOCK_MONOTONIC_RAW`; the alsa driver has not been exercised on hardware by this task (see Midget verification).

## JSON

One object per run:

```
{"bench":"midi-latency","host":"...","driver":"null","rate":48000,"quantum":64,
 "samples":100,"p50_ms":..,"p99_ms":..,"max_ms":..,"min_ms":..,"jitter_ms":<p99-p50>,"reported_ms":..|null,
 "xruns":0,"dropped":0,"extra":{...}}
```

`samples` is the number of measurements the statistics are over. `reported_ms` is what the HAL latency properties
predict (null when the bench has no prediction). `xruns` counts driver over- and underruns (and missed deadlines for
`xrun-sweep`); `dropped` counts events or impulses that never arrived.

### midi-latency

Bench Source -> hub -> Bench Instrument -> engine cycle -> device. The impulse is detected in the mix handed to the
device (the process tap). `latency = host_of(device frame of the impulse + output latency + output safety) - note host
time`. `reported_ms = (output latency + quantum + output safety) / rate`.

- With `--lead N >= 1` (and always on the null driver) the note is queued N cycles before its stamp, so it lands on its
  exact sample; the result is exactly the reported value, and any difference is a bug in the path or the model. On the
  null driver the test `null_driver_latency_is_exactly_output_plus_safety_plus_quantum` asserts this for quanta 16 to 128.
- With `--lead 0` on a real-time driver a source thread stamps each note with the host time at which it sends it, so the
  latency includes the wait for the next cycle: it is uniform between the reported value and the reported value plus one
  quantum, and `late` counts the notes that arrived after their cycle began.
- extra: `notes_sent`, `impulses` (rendered), `late`, `lead_cycles`, `interval_ms`, `async_source`, `out_total_frames`,
  `wake_late_max_ms` (how late the thread woke for a cycle).

### audio-roundtrip

An impulse is written on frame 0 of a cycle and detected in the captured stream (null: a cable from output to input;
loopback: the in-memory loopback pair; alsa: a physical cable on `--device`). The round trip is the frame distance
between the write position and the position at which the client sees the impulse. `reported_ms` is the sum of both
scopes: `(output latency + quantum + output safety) + (input latency + quantum + input safety)`.

- Loopback: measured equals reported (3 quanta at quantum 64 in the test).
- Null: the cable has no latency of its own but the null driver reports a safety offset of one quantum on output, so the
  measured value is one quantum below the reported one; that is the driver's report, not a bench error.
- extra: `impulses`, `threshold`, `capture_peak`, `measured_frames_p50`, `out_total_frames`, `in_total_frames`,
  `reported_frames`.

### xrun-sweep

Runs quantum 16, 32, 64, 128, 256 for `--seconds` each, with `--stress N` busy threads when the clock is real. An xrun is
a driver xrun or a cycle that finished after the next cycle's wake. The top-level `quantum` is the smallest quantum with
zero xruns (the largest one tried when none is clean), `samples` its cycle count, and the statistics are how late the
thread woke for each cycle (ms). `reported_ms` is that quantum's HAL output latency.

- extra: `smallest_clean_quantum` (or null), `seconds_per_quantum`, `stress_threads`, `sweep`: one object per quantum
  with `quantum`, `cycles`, `xruns`, `deadline_misses`, `wake_late_p99_ms`, `wake_late_max_ms`.

### midi-throughput

`--count` timestamped note-ons, eight per cycle at pseudo-random fractional sample positions, go through the hub, the
scheduler and the instrument on a simulated timeline. The statistics are the scheduling error, `|placed frame - exact
sample|`, in ms (so `max_ms * rate / 1000` is the worst error in samples; it is below 1 sample for on-time events, see
`sched_error_max_samples`). `dropped` is events sent minus impulses rendered.

- extra: `events_per_sec` (sent events over the wall time of the whole send, queue, schedule and render work),
  `events_sent`, `events_rejected` (the hub refused them), `impulses`, `late`, `wall_ms`, `sched_error_max_samples`,
  `events_per_cycle`.

## --live (task 28)

`--live` runs `midi-latency`, `audio-roundtrip` and `xrun-sweep` against the running `bifrost-audiod` through the
HAL client (`bifrost-audio-hal`, module `live`); the in-process model stays the default (host tests).
`midi-throughput --live` is refused. `--driver` is ignored; `--device <uid>` picks a device by its `DEVICE_UID`
(default: the default output, else the first device; a device without a uid shows as `object:<id>`). `--quantum` is
the IO procedure's buffer size. The JSON has the same shape with `driver = "live:<device uid>"` and `rate` from the
device's `NOMINAL_RATE` (else `--rate`). `reported_ms` comes from the device's `LATENCY + SAFETY_OFFSET` properties
(`LATENCY` already includes the server quantum); it is null while the server does not publish them.

The IO procedures only store into preallocated atomics (`StampLog`) and render without allocating (`render_into`).

- `audio-roundtrip --live`: an IO procedure with 1 input and 2 output channels writes 1.0 on output frame 0 every
  `interval` (after 8 warm-up calls) and records the output timestamp's sample time `S_out`; the input callback
  records the first sample over the threshold at `S_in = in_ts.sample_time + index` (hold-off half an interval). The
  HAL's input stamp is one buffer before the call and the output stamp one buffer after it, so the client-side round
  trip is `S_in - S_out + 2 * buffer` frames, the quantity `reported_ms` predicts. extra: `impulses`,
  `impulses_sent`, `threshold`, `measured_frames_p50`, `device_frames_p50` (`S_in - S_out`), `reported_frames`,
  `overloads`.
- `xrun-sweep --live`: per quantum an output-only IO procedure renders the Bench Instrument (silence) for
  `--seconds`. xruns per quantum are the cycles the server counted as missed (activation record), or the overload
  notifications when those are more. The wake-late statistics are each call's phase (`now` sample time) against
  the first call, wrapped into `[-q/2, q/2)`, minus the earliest phase. A quantum the server refuses (outside the
  device's buffer range) is listed in `extra.refused_quanta`.
- `midi-latency --live`: the Bench Instrument runs inside the bench's own IO procedure on the server. Notes go
  Bench Source -> `bifrost_midi::hub` -> the IO procedure (a std channel, `try_recv`); each call places the queued
  events with a `Timeline` anchored at the output timestamp. `latency = host_of(offset + LATENCY - quantum +
  SAFETY_OFFSET) - note stamp` on that timeline. `bifrost-midid` has no client socket yet, so the MIDI half runs
  in process: `extra.midi_path = "hub-in-process"`. `--lead` works as in-process (0: stamped when sent).

`tools/audio-bench.py midget` passes `--live` to the three device benches.

## Midget verification (not done by this task, nothing was built)

1. `bifrost-audio-bench midi-latency --driver loopback --quantum 64 --lead 2 --json`: `p50_ms` equals `reported_ms`.
2. `audio-roundtrip --driver loopback`: `p50_ms` equals `reported_ms`; then `--driver alsa:<card> --device <uid>` with a
   cable from the output to the input: compare with `reported_ms`.
3. `xrun-sweep --driver alsa:<card> --stress 4` as a normal user and with the FIFO policy.
4. Live, with `bifrost-audiod` running: `bifrost-audio-bench audio-roundtrip --live --quantum 64 --json` with
   `--device` set to a loopback (or cabled) device: `driver` is `live:<uid>`, `dropped` 0 and `p50_ms` within one
   frame of `reported_ms` (when the server publishes `LATENCY`). `xrun-sweep --live --stress 4`:
   `smallest_clean_quantum` <= 64 and `refused_quanta` lists only quanta outside the device range.
   `midi-latency --live --lead 2`: `p50_ms` equals `reported_ms`; `--lead 0`: between it and one quantum more.
5. `tools/audio-bench.py midget` runs those with `--live`.

## Left

- `midi-latency --live` routes through the hub in process: a run through `bifrost-midid` needs the daemon's client
  socket (a virtual source and destination per app), which does not exist yet (see 15-midi-server.md, Left).
- `reported_ms` for `--live` is null until the server's object tree publishes `LATENCY` and `SAFETY_OFFSET` per scope
  (the server still runs `NoObjects`); the device then also shows as `object:<id>` (no `DEVICE_UID`).
- The live xrun count trusts the server's per-procedure miss count; driver-side xruns are not visible to a client.
