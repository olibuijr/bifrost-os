# Audio 10: hog mode, quantum negotiation, latency

## Implemented

- `server/src/hog.rs`: `HogTable` (`HOG_PID` get/set, take needs the policy grant, release by owner, `check_access`
  returns `Hogged` for other pids, `fallback` picks the next unhogged device), `PidFd` (pidfd_open + poll) and
  `reap()` to release hogs of exited owners.
- `server/src/quantum.rs`: `Quantum` negotiation: smallest nonzero request, clamped to the device range with a 64
  frame floor, default 256, applied by `at_cycle_boundary()`; `cycles_per_call` for larger client buffers.
- `server/src/latency.rs`: `LATENCY`/`SAFETY_OFFSET` per scope (driver figure + quantum), total output ns vs the 4 ms
  budget, impulse round-trip helpers, `DeviceTable` of measured values, `perf_line` for `docs/perf-history.jsonl`.
- `hal/src/hog.rs`: `hog`, `release`, `hog_owner`, `must_move` over `Device::set/get(HOG_PID)`.
- One module-declaration line added to `hal/src/lib.rs` (`pub mod hog;`), besides server `lib.rs`.

All files have host unit tests with hand-computed values. Nothing is wired into the server loop (task 05 owns it).

## Left

- Wire `HogTable`/`Quantum`/latency into the object model and IO cycle (get/set of HOG_PID, BUFFER_FRAMES, LATENCY,
  SAFETY_OFFSET) once tasks 02 and 05 land; ask the policy process for the grant.
- Midget measurement (no hardware here).

## Midget verification (lead)

1. `ssh midget`, `sudo modprobe snd-aloop`.
2. Start `bifrost-audiod`; a test client hogs the ALC233 device (`bifrost pc audio set DEVICE HogMode PID`), a second
   client opening it gets `Hogged` and moves to HDMI/DP; kill the first and check `HogMode` reads -1.
3. Run a client at buffer 64 on the loopback pair, play an impulse, capture, `latency::find_impulse` gives the round
   trip; subtract nothing: record `latency::perf_line(...)` into `docs/perf-history.jsonl`. Expect <= 192 frames
   (4 ms at 48 kHz) of output latency on the ALC233.
