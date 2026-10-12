# Audio task 05: server core (bifrost-audiod, null and loopback drivers)

Contract: `docs/audio-architecture.md`. Task: `tasks/audio/05-server-core.md`.

## What is implemented

| File | Content |
|---|---|
| `springboard/audio/driver/src/null.rs` | `NullDriver` on a `ManualClock`. `NullHandle` (advance, drift in ppm, captured output, queued input, injected xruns, start/stop counters). Exact integer frame math (`frames_in`). |
| `springboard/audio/driver/src/loopback.rs` | `LoopbackDriver`: `loopback:out` feeds `loopback:in` in memory, one quantum of pre-roll (reported as latency). `realtime()` for the daemon, `with_clock()` for tests. |
| `springboard/audio/server/src/mixer.rs` | `VolumeRamp` (480-frame linear ramp), `mix_stream` (f32 sum, mono onto all channels), integer packing helpers (S16/S24/S32, clamped). |
| `springboard/audio/server/src/io.rs` | `ClientSlot` (memfd, eventfd, rings), `Shared` (client list handoff, quantum, counters), `DeviceRunner::cycle` (steps 1 to 6), the SCHED_FIFO 88 + uclamp IO thread on a timerfd, `set_realtime`. |
| `springboard/audio/server/src/client.rs` | `Conn`, `SO_PEERCRED`, `send_msg`/`recv_msg` with `SCM_RIGHTS`. |
| `springboard/audio/server/src/control.rs` | `ServerCore` (Hello, IO procs, device start on first running client and stop after 2 s idle, quantum = smallest running buffer, overload notices), the `ObjectApi` seam for task 02, and the `SOCK_SEQPACKET` loop. |
| `springboard/audio/server/src/main.rs` | args, `mlockall`, signalfd, drop to `audio` keeping CAP_SYS_NICE, `LoopbackDriver`, serve. |
| `springboard/audio/server/examples/sine-client.rs` | Raw test client for Midget. |
| `packaging/arch/` | `bifrost-audiod.service`, `sysusers-bifrost-audio.conf`, `tmpfiles-bifrost-audio.conf`, install lines in `package-files.sh`. |

## How the cycle maps to the contract

- Step 2: zero timestamp published into every client's memfd each cycle (`rate_scalar` = measured hw frames over nominal after 100 ms).
- Step 3/4: input is read once and written into each running client's input ring. Each running client is triggered with `shm::reset` + `shm::finish(driver_node, [client])`. The server then waits (futex, budget `budget_ns`) on the client's *done* record until its output ring holds a full quantum.
- Step 5: a client without a full quantum is skipped (silence), `overloads` and `missed_total` are bumped, and the server sends `Reply::Overload`. A client that was late and then delivered two quanta has the stale one dropped (ring at 2 quanta or more). The device always writes.
- Large buffers: a client with `buffer_frames` of N quanta is triggered every N cycles; the output ring is pre-rolled with N-1 quanta of silence.
- Step 6: `write_mix` (the driver converts to its physical format). Device xruns are counted in `Shared::xruns`.
- Quantum: `ServerCore` stores the smallest running client buffer (default 256) in `Shared::quantum`; the runner applies it at the next cycle boundary and the timeline (`next_ns`) is computed from the frame count, so it does not drift.
- Idle: with no running device the loop is `poll(-1)`: no timers, no wakeups. A running device wakes the loop every 250 ms for notices.

## Contract changes (additive)

1. **Done record.** The client memfd is `layout.total` bytes where `total` includes a 64-byte `Activation` at the end (`done_offset = total - 64`, `required = 1`). The client calls `shm::finish(&my_activation, &[&done], now)` when its IOProc finished. The server sleeps on `done.wake`. The HAL (task 06) must do this; `examples/sine-client.rs` shows it.
2. **Device object ids** are `1000 + index` inside `ServerCore` until task 02's tree assigns ids (`DEVICE_ID_BASE`, `ServerCore::device_id`). `Owned(SYSTEM_OBJECT)` falls back to this list when the object tree says unknown.
3. **`ObjectApi` trait** (control.rs): the seam through which `Get/Set/Listen/Unlisten/Owned` reach task 02's tree. `NoObjects` is the stand-in.

## Left (todo_left)

- Wire task 02's object tree into `ObjectApi` and `ServerCore::device_id`; wire `bifrost-audio-alsa`'s `AlsaDriver` into `main.rs` (one line, the comment marks it).
- The eventfd of each IO proc is created and sent but not signalled (a write is a syscall other than the futex, which the real-time rule forbids). Epoll-style clients need a design decision (a non-RT helper thread that mirrors `wake`).
- Hot-plug events are read on the 250 ms tick and on socket activity only; the ALSA driver should expose a pollable fd for them.
- `PromoteThread` checks the tid against `/proc/<peer pid>/task/` instead of the pidfd; the pidfd arrives and is closed.
- Per-stream volume/mute are `ClientSlot::set_volume` / `muted`; the object properties that set them belong to task 02.
- No heap-free guard allocator in debug builds (contract says so for client IO threads; server side the hot path is allocation-free by construction).
- Cargo note: worktrees share package hashes, so a shared `CARGO_TARGET_DIR` between agents can link another agent's `bifrost-audio-driver` (it did here: spurious "no NullDriver in null"). Checks were run with a private target dir.

## Lead: verification

Host (after merging tasks 01 to 04 so the `todo!()` bodies exist):

```
cd springboard
cargo test -p bifrost-audio-driver -p bifrost-audiod
```

Expected: mixer, null, loopback, client (fd passing), io (two clients mixed exactly, late client skipped and counted, quantum 256 to 64, ramp, input, xrun, zero stamp) and control (start/stop/idle, socket round trip) tests pass.

Midget (needs the ALSA driver wired in `main.rs`):

```
bifrost pc publish && bifrost pc midget update --restart       # installs bifrost-audiod, sysusers, tmpfiles
ssh midget 'systemctl enable --now bifrost-audiod; systemctl status bifrost-audiod --no-pager'
ssh midget 'getent passwd audio; ls -ld /run/bifrost/audio /run/bifrost/audio/control'
ssh midget 'ps -L -o tid,cls,rtprio,comm -p $(pidof bifrost-audiod)'     # IO thread: FF 88 when a stream runs
# two clients, 60 s, 256 frames
ssh midget 'sine-client 440 60 256 & sine-client 660 60 256; wait'
ssh midget 'journalctl -u bifrost-audiod --since "-2min" | grep -c xrun'   # expect 0
# idle: no streams, audiod must not wake
python3 tools/bench.py --midget                                          # idle wakeups for bifrost-audiod: 0
ssh midget 'perf stat -e context-switches -p $(pidof bifrost-audiod) sleep 10'  # ~0
```
