# audio-03: shared memory ABI (`bifrost-audio-shm`)

Implemented in `springboard/audio/shm/src/lib.rs` (all `todo!("audio-03")` replaced):

- `Activation` (64 bytes, offsets pinned by test), `ZeroStamp` seqlock (`publish` / wait-free-ish `read`),
  `RingHeader` + `Ring` (SPSC f32 planes, wrap-safe, header clamped against a hostile peer), `ring_bytes`, `layout`.
- `Region`: sealed memfd (`F_SEAL_SHRINK|GROW`), `create`/`map` (map refuses unsealed or too-short fds), `get::<T>`
  (bounds and alignment asserted), `init_ring`, `ring(offset)` (validated view), `Drop` unmaps.
- `futex::{wait, wake}` (shared, not private), `EventFd` (epoll variant), `now_ns`.
- Trigger: `finish` (saturating decrement; the target that reaches 0 is stamped TRIGGERED and woken exactly once),
  `reset`, plus `Activation::trigger` (for nodes without inputs) and `Activation::wait_triggered` (no lost wakeup:
  futex word read before the status check).
- `selftest::chain_fork` and `examples/chain.rs`: 3-node chain across fork.

Tests: layout/offset pins, layout by hand, seqlock never torn (writer thread), ring wrap/limits/hostile header/SPSC
thread test, seals, futex, eventfd, trigger semantics, 8-thread graph stress (100k cycles, per-node run counts must
equal the cycle count: no lost wakeups, no double triggers), 1 s fork chain. Only `cargo check` was run.

## Midget verification (lead)
1. `cargo test --offline -p bifrost-audio-shm --release` on the host.
2. `cargo run --offline --release --example chain -p bifrost-audio-shm` on the host and on Midget (10 s): expect
   `ok: ~10000 cycles, no misses`.
3. Optionally with `chrt -f 80` on Midget for the SCHED_FIFO case.

## Left
- 10^6-cycle loom-style run only at 10^5 cycles in the unit test; raise `CYCLES` or run in release for 10^6.
