# Theme 29: Barriers on concurrent queues

Closes the dispatch barrier and semaphore gaps in `springboard/sdk/src/dispatch.rs` (the `bifrost_app::dispatch` module,
Grand Central Dispatch parity). Pure Rust, std only, no new crates, no unsafe.

## What was built

- **Concurrent queues with barriers.** `DispatchQueue::concurrent(label, qos)` makes a private concurrent queue. Ordinary
  items on it run side by side on the shared pool. `DispatchQueue::async_barrier(f)` submits a barrier item: it waits
  until everything submitted before it has finished, runs alone, and holds back what was submitted after it until it
  returns. Scheduling lives in a `Concurrent` kind (pending FIFO with barrier flags, running count, barrier-running
  flag); jobs go to the existing pool after the queue lock is released.
- **Barrier on other queues.** `async_barrier` on a serial or global queue is a plain `async`, as the system does: a
  serial queue already runs one item at a time, and the global queues are not barrier queues.
- **Counting semaphore.** `DispatchSemaphore::new(value)`, `signal() -> bool`, `wait()`, and
  `wait_timeout(Duration) -> DispatchTimeoutResult` (`Success` or `TimedOut`). The count may go negative while waiters
  are queued. A timed-out wait gives its count back and wakes the other waiters, and a signal that lands at the deadline
  still counts as success. Clones share the semaphore.

Tests added to the existing `mod tests` in `dispatch.rs` (7 new, all behaviour tests): ordinary items overlap on a
concurrent queue, a barrier waits for earlier items and holds back later ones, a barrier never shares the queue, a
barrier on a serial queue runs as a plain async, a signal wakes a waiting thread, a timed-out wait returns its count, and
the semaphore counts available resources. No builds were run, per the task rules.

## Official API names mirrored

| Apple (official documentation) | BifrOSt |
|---|---|
| `DispatchQueue.Attributes.concurrent` (`init(label:qos:attributes:)`) | `DispatchQueue::concurrent(label, qos)` |
| `DispatchWorkItemFlags.barrier` (on `async(flags:execute:)`) | `DispatchQueue::async_barrier(f)` (queue-level flag; the work-item flag is not built yet) |
| `DispatchSemaphore.init(value:)` | `DispatchSemaphore::new(value)` |
| `DispatchSemaphore.signal() -> Int` | `DispatchSemaphore::signal() -> bool` (true when a waiter was woken, the same non-zero meaning) |
| `DispatchSemaphore.wait()` | `DispatchSemaphore::wait()` |
| `DispatchSemaphore.wait(timeout:) -> DispatchTimeoutResult` | `DispatchSemaphore::wait_timeout(Duration) -> DispatchTimeoutResult` |

Not mirrored yet: `wait(wallTimeout:)` (needs a wall-clock type), the `DispatchTime` argument (BifrOSt uses `Duration`,
as `DispatchGroup::wait` does), and the work-item barrier flag.

## REA evidence (official rendered pages, Chromium via `tools/parity/rea-page.sh`)

- `https://developer.apple.com/documentation/dispatch/dispatchworkitemflags/barrier`: `ev_2a986b16bcf91378c2e61096824ccbca77a20e498f6ff34ed7679a91ef18cf06`
- `https://developer.apple.com/documentation/dispatch/dispatchsemaphore`: `ev_965e7b9075e2f592347dea739b7d3094e9d242576825b54ffb3cc319f83dec04`
- `https://developer.apple.com/documentation/dispatch/dispatchqueue/attributes/concurrent`: `ev_d0676b2553ee782f5e3166fd6f4317612555ca3f3e3667df53ac6e908195b577`

## Matrix rows closed

| Framework slug | Feature (as in `docs/parity-matrix/data/<slug>.json`) | New status | BifrOSt path |
|---|---|---|---|
| dispatch | Barriers on concurrent queues | partial | `springboard/sdk/src/dispatch.rs` (`DispatchQueue::concurrent`, `async_barrier`) |
| dispatch | Semaphores | partial | `springboard/sdk/src/dispatch.rs` (`DispatchSemaphore`, `DispatchTimeoutResult`) |

Both rows are partial: the work-item barrier flag and the wall-clock timeout variant are still missing. The matrix itself
is updated centrally by the lead.
