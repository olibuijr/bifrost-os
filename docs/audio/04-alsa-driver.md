# Task 04: pure Rust ALSA driver (`springboard/audio/alsa`)

## Implemented

Crate `bifrost-audio-alsa`, no libasound, no new dependencies.

| File | Content |
|---|---|
| `sys.rs` | `_IOC` encoding, ioctl wrappers (EINTR retry), device open, `CLOCK_MONOTONIC_RAW` / `CLOCK_MONOTONIC` |
| `uapi.rs` | PCM hw/sw params, mmap status and control pages, `SYNC_PTR`, ioctl numbers; sizes, offsets and numbers pinned against `sound/asound.h` (checked with a C compile) |
| `ctl.rs` | control device (moved from `media/mixer.rs`): card info, PCM info, elements, dB TLV, event subscribe/read, `* Jack` booleans, ELD and monitor name |
| `convert.rs` | f32 to S32_LE / S24_LE (32-bit container) / S16_LE and back, strided so one routine serves both mmap layouts |
| `ring.rs` | pointer wrap arithmetic against `boundary`, ring layouts, pack/unpack/silence, playback and capture recovery plans |
| `pcm.rs` | `negotiate` (access, format S32>S24>S16, rate, period = quantum, largest buffer, optional `NO_PERIOD_WAKEUP`), sw params (`tstamp_mode=ENABLE`, `MONOTONIC_RAW`, `stop_threshold=boundary`), mmap of data/status/control with `SYNC_PTR` fallback, status read without a syscall, appl_ptr commit, `LINK` |
| `dll.rs` | second-order DLL, bandwidth 2 Hz decaying (tau 1 s) to 0.2 Hz; `rate_scalar`, `host_time_of`, `frames_at` |
| `timer.rs` | timerfd on `CLOCK_MONOTONIC` armed with an absolute raw-clock deadline (converted by the clock offset) |
| `driver.rs` | `AlsaDriver: Driver` (enumeration, hot-plug rescan every 1 s, jack and control events, controls) and `AlsaIo: DeviceIo` |

Mapping to the contract: `position()` reads the mapped status page (state, hw_ptr, tstamp) with a double read of the
pointer/timestamp pair and feeds the DLL when the timestamp changed; no syscall. `write_mix` converts into the ring
at `appl_ptr` and commits through the control page. A playback underrun (hardware past appl_ptr, because
`stop_threshold=boundary` never stops the stream) is detected from the pointers: the driver silences `prefill`
frames after the hardware pointer, writes the cycle there and returns `Xrun{frames_lost}`. Capture mirrors this.
Uids are `alsa:<card id>:<device>`; clock domain is card number + 1. Output latency reported is the prefill
(2 quantum), input latency one quantum; no codec delay is added.

## Decisions and deviations

- `NO_PERIOD_WAKEUP` is supported (`AlsaOptions::no_period_wakeup`) but off by default: without period interrupts the
  status page only moves on `HWSYNC`, so `position()` then issues one `HWSYNC` ioctl (the only non-futex syscall).
  With wakeups left on at the quantum-sized period, the status page is current and position is syscall free,
  which honours the real-time rule. The lead can flip the default after measuring on Midget.
- Channel count: no request in `IoConfig`, so the device maximum up to `AlsaOptions::max_channels` (default 2).

## Contract changes (additive)

`bifrost-audio-driver::DeviceIo` gained two default methods: `rate_scalar(&self) -> f64` (default 1.0) and
`host_time_of(&self, hw_frames: u64) -> Option<u64>` (default None), so the server can publish the rate scalar and
arm its cycle timer from the DLL. Task 05 should keep them when it edits that file.

## Host tests

Struct layouts and ioctl numbers; DLL convergence at +300 and -300 ppm (< 1 ppm from 2 s on, checked at every
cycle to 6 s; parameters chosen with a Python model: 0.078 ppm at 2 s); format conversion at known points; ring
wrap, under/overrun plans; negotiation against a modelled kernel device; control mapping; timer deadlines. Run by
the lead: `cargo test -p bifrost-audio-alsa`.

## Left

- Jack/ELD events do not yet re-announce a device name change on HDMI hot-plug (`DeviceAdded` only for new uids).
- Suspend/resume (`STATE_SUSPENDED`) is reported as not running; no automatic resume.
- No codec/DMA delay in `latency()`.

## Midget verification (for the lead)

1. `cd springboard && cargo test -p bifrost-audio-alsa` on the host.
2. Push the example to Midget (`tools/midget-push-shell.sh`-style copy of the built binary, or build on Midget):
   `cargo run --release --example alsa-probe -p bifrost-audio-alsa` with the shell stopped so `hw:0,0` is free.
3. Expect: the device list, jack lines, `cycles 7500 xruns 0`, and `dll rate scalar` within +-50 ppm
   (`ppm` printed).
4. Compare the jack lines with `amixer -c0 contents | grep -A2 Jack`; plug and unplug headphones and re-run.
5. Under load: `stress-ng --cpu 8` in parallel, repeat; xruns should stay 0 once the server sets SCHED_FIFO (task 10).
6. Try `AlsaOptions { no_period_wakeup: true, .. }` and compare timestamps.

## Clock contract: PCM timestamps (2026-10-12)

Every host time in the audio stack is `CLOCK_MONOTONIC_RAW` (`bifrost_audio_shm::now_ns`). The kernel stamps the PCM
status page with `CLOCK_REALTIME` unless the file declared protocol >= 2.0.12 (`SNDRV_PCM_IOCTL_USER_PVERSION`);
`sw_params.tstamp_type` alone is ignored. Measured on Midget (0.6.5): `midi-latency --live` saw 1.79e12 ms stamps
(the Unix epoch). `Pcm::open` now sends `USER_PVERSION` (2.0.15) and `TTSTAMP(MONOTONIC_RAW)` right after open, and
`sw.proto` is set. `status.tstamp` (not `audio_tstamp`) is the hardware-pointer time and follows that type, for the
mapped page and `SYNC_PTR` alike.

Defence in depth: `AlsaIo::position` accepts a stamp only within 1 s of `now_raw_ns()`; otherwise it substitutes the
read time and logs once ("device timestamps are not on the host clock"). No other kernel-sourced time reaches the
server, HAL, bench or midid: they use `now_ns()`; the timerfd uses `CLOCK_MONOTONIC` and `timer.rs` converts to RAW.
