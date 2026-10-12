# Audio 21: the lowest-latency path

Owner goal: as low latency as the hardware physically allows. Every change below is small and has host tests
for its pure part (`cargo test -p bifrost-audio-alsa -p bifrost-audiod`, run by the lead).

## What was already there

mmap access, `MONOTONIC_RAW` timestamps, `avail_min = period`, `stop_threshold = boundary`, an explicit `START`
after the first period is queued (so the first cycle is not delayed), the hardware position from the mmapped
status page (no ioctl per cycle), format conversion only (no resampling in `alsa`), SCHED_FIFO 88 for the
driver thread and 80 for clients, `sched_util_min`.

## Changes

| Change | Where | Why |
|---|---|---|
| Ring of 2 periods, else 3 (`PERIOD_COUNTS`, `low_latency_buffers`); the old "largest buffer" stays when the period cannot equal the quantum or `low_latency` is off | `alsa/src/pcm.rs` `negotiate`, `AlsaOptions::low_latency` (default on) | The ring size bounds the write-ahead; a 4096 frame ring is 85 ms of queue nobody uses. |
| Quantum range of an ALSA device is half its smallest buffer when low latency is on (`quantum_range`) | `alsa/src/driver.rs` | A 2-period ring makes a device with a 64 frame minimum buffer capable of a 32 frame quantum. |
| `silence_threshold = silence_size = 0` | `pcm.rs` `set_sw_params` | The kernel never fills silence behind the driver's back; the driver silences the ring itself. |
| Converter latency: hardware FIFO (`hw_params.fifo_size`) + `PCM_DELAY` minus the queued frames, read once at `start` | `pcm.rs` `fifo_frames`, `delay_frames`, `converter_delay`; `AlsaIo::latency` | Added to the `LATENCY` figure (`Latency.output`/`input`). Never an ioctl in the cycle. |
| Cycle timer armed from the driver DLL (`DeviceRunner::next_cycle_ns` = `host_time_of`) on an absolute `timerfd` (`CycleTimer::arm_raw`), `margin` before the start, then a spin to the exact start | `server/src/io.rs` `spawn_io_thread_tuned` | Wake-up lateness no longer shifts the cycle. |
| Margin = p99 of the last 256 wake-up latenesses + 10 us, within 30 us and half a quantum (`wake_margin_ns`, `JitterTracker`) | `server/src/rt.rs` | Adapts to the machine; the spin costs at most the margin per cycle. |
| `PR_SET_TIMERSLACK` 1 ns on the IO thread | `rt::set_timer_slack_1ns` | Default slack is 50 us. |
| `mlockall(MCL_CURRENT \| MCL_FUTURE)` after setup (was before) | `main.rs`, `rt::lock_memory` | Pages setup allocated are resident too. |
| `/dev/cpu_dma_latency` held at 0 while any device runs | `rt::device_started` / `device_stopped` (refcount), called by `control.rs` | CPUs do not enter deep C-states between cycles (exit latency of 100 us or more). Power cost: the package stays in C1, a few watts more on a laptop while audio plays; released when the last device stops. |
| Driver thread pinned off the CPUs that take the sound IRQ | `rt::scan_sound_irq_threads`, `driver_cpus`, `set_thread_affinity` | They would preempt each other. Kept unchanged on a single core. |
| Sound IRQ thread (`irq/NN-snd_hda_intel`) raised to SCHED_FIFO 89 (driver 88, clients 80) and put back to 50 at the last stop | `rt.rs` | Only exists when the kernel runs threaded IRQs (`threadirqs`); otherwise the scan is empty and nothing happens. Needs only `CAP_SYS_NICE`, which the unit already has, so no privd helper. |
| Quantum 16 and 32 frames allowed (`Quantum::new_low_latency`; the server clamps at `MIN_QUANTUM` = 16) | `quantum.rs` | 0.33 and 0.67 ms at 48 kHz. |
| Sustained-xrun fallback: 8 bad cycles (device xrun or late client) in 1 s double the quantum (to at most 512), set `Shared::fallback_floor`, post `Reply::Overload` to the device's clients | `rt::FallbackPolicy`, `io.rs`, `control.rs` | A machine that cannot hold 16 frames settles at 32, 64, ... instead of crackling. The floor resets when the device stops. |

Packaging: `bifrost-audiod.service` allows `/dev/cpu_dma_latency`; `tmpfiles-bifrost-audio.conf` sets it to
`0660 root:audio`.

## Not done

- `AlsaDriver` is still not pushed into `main.rs`'s driver list (its comment says it lands there); no change to avoid
  a merge conflict with the wiring task.
- Capture-side `PCM_DELAY` (only the FIFO counts for input).
- Threaded IRQs need the `threadirqs` kernel parameter; setting it is a boot-config decision, not made here.

## Midget verification (lead)

1. Kernel: `cat /proc/cmdline` (add `threadirqs` to test the IRQ path), `pgrep -a irq/` shows `irq/NN-snd_hda_intel`.
2. Start `bifrost-audiod` as the `audio` user, run a client at 64 frames; check the journal for
   `irq N thread T raised to SCHED_FIFO 89`, `converter latency: playback X frames`, and
   `chrt -p $(pgrep -f audio-io)` (88), `grep Cpus_allowed_list /proc/$(pgrep bifrost-audiod)/task/*/status`.
3. `fuser -v /dev/cpu_dma_latency` lists the daemon while a device runs and nothing after it stops.
4. `bifrost-audio-bench xrun-sweep --driver alsa:0` with quanta 16, 32, 64, 128: record the smallest quantum with
   `xruns == 0`, and `bifrost-audio-bench audio-roundtrip --driver alsa:0` at that quantum: compare `p50_ms` to
   `reported_ms`.
5. Force overload (a client that sleeps 2 ms per cycle at quantum 32): the journal shows
   `sustained xruns at quantum 32, now 64`, the client receives an Overload notice.
