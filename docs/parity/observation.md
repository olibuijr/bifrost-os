# Observation: parity notes

Framework: Observation (`https://developer.apple.com/documentation/observation`), macOS 14.0 and later.
Status after this round: **partial**. Rows and gaps are in `docs/parity-matrix/data/observation.json`.

## What was built

All changes are in `springboard/sdk/src/observable.rs` (`bifrost_app::observable`). Std only, no new
crates, no `unsafe`, no hardware or network.

1. **Change notification, re-entrancy fixed (partial).** A `set` or `update` made from inside a subscriber
   used to be dropped for the running subscriber set. Changes are now queued and delivered after the running
   change, in order, and the value is updated at once. Cancelling another subscriber during a change is also
   durable; before, a subscriber removed mid-change could be re-added when the pass ended.
2. **Automatic dependency tracking (partial).** `with_observation_tracking(apply, on_change) -> R` runs
   `apply`, records every SDK `Observable` read by `get()` inside it, and calls `on_change` once when one of
   them changes. `sink`, `map` and `remove_duplicates` read through an internal path, so they do not record.
3. **Async change sequence, synchronous form (partial).** `Observable::observations()` returns an
   `Observations<T>` that records each later change; `try_next()` pops the oldest and `pending_len()` counts
   the waiting ones. Dropping it cancels the recording.

4. **Model declaration macro (partial).** `observable_model!` in `springboard/sdk/src/observable_object.rs` declares a
   struct whose fields are each an `Observable<T>`:
   `observable_model! { pub struct Session { pub name: String = String::from("guest"), pub count: u32 = 0 } }`.
   `Session::new()` sets the initial values, links every field to one `ObservableObjectPublisher`, and implements
   `ObservableObject`, so a write to any field notifies `object_will_change`. The subscriptions live in the struct.
   No app uses it yet. Built on parity/team-a-09; not yet verified on Midget.

Tests added in the same module (run by the next build stage, not run here): re-entrant set order, durable
cancellation during a change, tracking fires once for a read value, tracking fires for any of several reads,
tracking without reads never fires, and pull-ordered observations with cancel on drop.

## Official API names mirrored

| BifrOSt | Official name |
|---|---|
| `Observable<T>` with change delivery | `Observable` protocol, `ObservationRegistrar` |
| `with_observation_tracking(apply, on_change)` | `withObservationTracking(_:onChange:)` |
| `Observable::observations()`, `Observations<T>` | `Observations` (structure) |
| `observable_model!` | Observable macro (declarative form only) |

## Differences from the official API

- `with_observation_tracking` returns only the closure value. The official `withObservationTracking` also
  has an options overload and works with a token type; here there are no options, no event type and no token.
- The on-change callback runs after the change. The official one runs before it.
- Tracking is one-shot: after `on_change` fires, nothing is re-armed. Reads made inside `on_change` are not
  tracked.
- `Observations` is pull-based. It is not an `AsyncSequence` and does not suspend.
- Only the SDK `Observable` is tracked. The UI-model `Observable<T>` in `springboard/ui/src/observe.rs` is
  unchanged: it notifies old and new values on change and has no tracking.
- There is no `@Observable` macro. Each observable is wired by hand.

## REA evidence

- `ev_f12af59a5bcc35eb0cec29f7b3fa3568d11914f48efd308973b34ffe4851e8ef`: the rendered official page at
  `https://developer.apple.com/documentation/observation`. It lists `withObservationTracking(_:onChange:)`,
  `withObservationTracking(options:_:onChange:)`, `withContinuousObservation(options:apply:)`, `Observations`,
  `ObservationTracking` and the macros.
- `ev_59ddd532abf1677b1e3304c8d7443c1ffaef9a4e65513f0e7e8dd952321415d0`: earlier evidence recorded in the
  matrix row for the framework.
- API shape came from `tools/parity/docc.py observation` (topic listing for the framework).

## Remaining gaps

- No `@Observable` macro or per-property tracking.
- No `withContinuousObservation(options:apply:)`, no `ObservationTracking.Options`, `Event` or `Token`.
- No `AsyncSequence` form of `Observations`; no executor or suspension in the pure std model.
- Tracking covers only SDK observables, not the UI-model copy.
- `docs/bifrost-ui.md:1435` still describes the old re-entrant `set` limit. That shared doc was out of scope
  for this change and needs a later update.
- SwiftUI-style view invalidation from observed model state is not verified in code.
