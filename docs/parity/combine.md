# Combine parity: what BifrOSt builds

Official reference: https://developer.apple.com/documentation/combine (Mac Catalyst 13.0, macOS 10.15).

REA evidence of the rendered landing page: `ev_256d776afb2e462cdacf862416e7d3b883ecb9a3c2d98116a97e0404e85c7991`.
API shape for PassthroughSubject and the operators was read from the official DocC JSON (`tools/parity/docc.py combine`).

## Built in this round

All code is pure Rust with std only, in `springboard/sdk/src/observable.rs` (`bifrost_app::observable`). It extends
the existing `Observable<T>` module and adds no new module.

| Mirrors | BifrOSt API | Notes |
|---|---|---|
| `PassthroughSubject` (Subjects) | `PassthroughSubject<T>`: `new`, `send`, `sink`, `map`, `filter` | Keeps no value. A subscriber receives only sends made after it subscribed. `send` delivers in subscription order. |
| `Publisher.filter(_:)` | `PassthroughSubject::filter(pred)` | Returns a derived subject and the link `Subscription`. |
| `Publisher.map(_:)` on a subject | `PassthroughSubject::map(f)` | Returns a derived subject and the link `Subscription`. |
| `Publishers.CombineLatest` (two sources) | `combine_latest(&a, &b) -> (Observable<(A, B)>, Cancellables)` | Both sources always hold a value, so the pair exists at once. Emits once per change of either source. The returned bag must be kept alive; clearing or dropping it detaches the pair. |

Cancellation during delivery: a subscriber cancelled from inside a `send` is skipped for the rest of that send.
The cancellation is applied when the outermost send returns. Re-entrant `send` calls are supported.

Tests (in the same module, `#[cfg(test)] mod tests`):
- `passthrough_delivers_only_later_sends_in_subscription_order`
- `passthrough_cancel_during_send_skips_the_cancelled_subscriber`
- `passthrough_filter_and_map_forward_derived_values`
- `passthrough_dropping_the_subscription_stops_delivery`
- `combine_latest_emits_once_per_change_and_detaches_on_clear`

The tests were written but not run. This stage does not build or test (see the workflow rules).

## Matrix changes

`docs/parity-matrix/data/combine.json` (copied from the parity docs branch and updated):
- "Publisher protocol and operators": still `partial`, now lists filter and combine_latest.
- "Subjects": still `partial`, now lists PassthroughSubject.
- Summary, gaps and `rea_evidence` updated. Framework status stays `partial`.

## Remaining gaps

- Operators: merge, flatMap, debounce, throttle, and publisher-typed chains (`AnyPublisher`).
- Convenience publishers: Future, Just, Empty, Fail, Deferred, Record.
- Completion and failure types, `send(completion:)`, Demand and back pressure (Subscribers and Subscriptions).
- Connectable publishers (`share`, `multicast`, `connect`).
- Schedulers: `receive(on:)`, `subscribe(on:)`, debounce timing.
- ObservableObject `objectWillChange` aggregate.
- Thread-safe publishers. `Observable` and `PassthroughSubject` are `Rc`-based and single-threaded.
- Foundation publishers (NotificationCenter, Timer, URLSession, KVO) as publishers.
- AsyncPublisher bridge and debugging identifiers.
