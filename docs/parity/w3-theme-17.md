# Parity wave 3, theme 17: distributed actors (Combine publisher gaps)

Branch `parity/w3-theme-17`. Pure Rust, std only, in `springboard/sdk/src/observable.rs` (the existing
Combine module; no new module file). The theme mixes Swift distributed actors, notifications, Network
Extension and EventKit gaps. Only the Combine rows are honest to close in pure Rust; the others need
OS services (distributed runtime, APNs, signal delivery, EventKit stores) and stay gaps.

## What was built

- `Completion<E>` (`Finished`, `Failed(E)`) and `Never`: how a publisher stopped.
- `Finite<T, E>`: replay publishers `Just`, `Empty(completeImmediately:)`, `Fail`, and `Record`. Values are
  delivered in order, then the completion, synchronously. An empty publisher without immediate completion
  never completes.
- `Future<T, E>` and `Promise<T, E>`: one later result. The first resolve wins. Subscribers before the
  result wait; later subscribers get the stored result at once. Waiting subscribers cannot be cancelled.
- `Deferred<T, E>`: a factory run once per subscriber, so nothing runs before subscription.
- `Connectable<T>` (`multicast()`, `connect()`, `autoconnect()`), `Autoconnect<T>` (`share()`): values flow
  from an upstream `PassthroughSubject` only while connected. `Autoconnect` connects on the first
  subscriber and keeps the link.
- `CombineIdentifier`: a process-unique id from an atomic counter, `raw()`, `Display` as hex. Each
  `PassthroughSubject` carries one; clones share it.
- `PassthroughSubject::subscriber_count`, `identifier`, `multicast`, `share`.

## Official API mirrored (developer.apple.com DocC)

`Future` (init(_:)), `Just` (init(_:)), `Empty` (init(completeImmediately:)), `Fail` (init(error:)),
`Record` (init(output:completion:)), `Deferred` (init(createPublisher:)), `ConnectablePublisher` (connect()),
`Subscribers.Completion`, `CombineIdentifier` (init()), `share`, `multicast`, `autoconnect`.
DocC was read with `tools/parity/docc.py`. The Combine collection page lists `Future`, `Just`, `Empty`,
`Fail`, `Deferred`, `Record`, `ConnectablePublisher` and `CombineIdentifier` in the same groups.

## REA evidence (rendered official pages, `tools/parity/rea-page.sh`)

| Page | Evidence id |
|---|---|
| combine/future | ev_423fcee815023580722ccb5bd21bae6839019488269d5da6386326b47300d340 |
| combine/just | ev_86c36e8d8ae50c2d7bb8068851ecd1a518647624ac8aa9e9aece181b5f9ef0c8 |
| combine/connectablepublisher | ev_20e6f9117cfe699a8be0fed9aa51236f3f4163f33c5562b3a1ade231ae7d3037 |
| combine/combineidentifier | ev_9023f315963a80eb8a2289cc570db00a47fd21b7ce54b79262948a5ebfd91ca2 |
| combine/subscribers/completion | ev_ddadeb209a6d2836394c809421696fe4008af3904b773b7b413f982312a1e3d4 |
| combine/deferred | ev_88db2bc41ecb0514a82ce1ee6e664a37585f1b9178e4c191155b27682ac05875 |

## Matrix rows closed (new status partial)

| Framework slug | Feature (as in data/<slug>.json) | Status | BifrOSt path |
|---|---|---|---|
| combine | Convenience publishers | partial | springboard/sdk/src/observable.rs |
| combine | Connectable publishers | partial | springboard/sdk/src/observable.rs |
| combine | Subscribers and demand | partial | springboard/sdk/src/observable.rs |
| combine | Debugging identifiers | partial | springboard/sdk/src/observable.rs |

Still open in these rows: `Record` is a constructor, not a builder; `Future` has no cancellation;
`Subscribers.Demand` and back pressure are missing (`Completion` only); `print`/`handleEvents` and
custom identifier conversion are missing; `Autoconnect` never disconnects.
Not closed: Async publishers (no async bridge), and every non-Combine row in theme 17.

## Tests

`observable.rs` tests added to the existing `mod tests`: just, empty, fail/record order, future wait and
first-result-wins, future resolved at creation with a failure, deferred per subscriber, connectable only
while connected, share with one shared link, identifier uniqueness and hex display.
Not built or run (the wave brief forbids builds).
