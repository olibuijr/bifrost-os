# Theme 34: ObservableObject

## What was built

- `springboard/sdk/src/observable_object.rs` (new): `ObservableObjectPublisher` is the aggregate change publisher of
  one model object. `send()` notifies subscribers, `sink()` receives notifications, and `watch(&Observable<T>)` sends
  one notification for every change of a property, for as long as the returned `Subscription` lives. The
  `ObservableObject` trait exposes `object_will_change()`. It is built on the existing `PassthroughSubject`
  and `Observable` in `springboard/sdk/src/observable.rs`, which are reused rather than duplicated.
- `springboard/sdk/src/dispatch.rs`: `DispatchQueue::async_and_wait(f)` submits the closure as a work item and
  returns after it finishes. The closure runs on the queue (also for the global queues, unlike `sync`). A panicking
  closure still counts as finished. The main queue refuses a wait from the main thread, as `sync` does.
- `springboard/sdk/src/lib.rs`: one `pub mod observable_object;` line.

## Official API mirrored

- Combine: `ObservableObject`, `objectWillChange`, `ObservableObjectPublisher` (`init()`, `send()`).
- Dispatch: `DispatchQueue.asyncAndWait(execute:)`.

## REA evidence

Official pages captured with `tools/parity/rea-page.sh`:

- `https://developer.apple.com/documentation/combine/observableobject`: `ev_6ff2c0ab881f72f92130bd65941030e486a72e8247eb163d316605e2ba326ee9`
- `https://developer.apple.com/documentation/combine/observableobjectpublisher`: `ev_392ce4401e47c7201a19d9c348d01cac123a21adfcab142df5c013959bdfbeac`
- `https://developer.apple.com/documentation/dispatch/dispatchqueue`: `ev_8fe8433bc2b3d7fe8dec417982cf4d16df97291e90480a4808708676c763e733` (lists `asyncAndWait(execute:)`, "Submits a work item for execution and returns only after it finishes executing")

Not built, because the official page could not be found:

- `https://developer.apple.com/documentation/foundation/nsexceptionhandler`: `ev_223bd8c50c0ff76b625b640296bb63ce3fe223d7220d918eb64db2c572dbc4ac` (page not found)
- `https://developer.apple.com/documentation/foundation/nsexceptionhandlerdelegate`: `ev_a0cca57859c2c7781e5a578df6c2fc4401f8dd37377a9f1d84e6d9ac59df640f` (page not found)

## Matrix rows closed

| Framework slug | Feature (as in data/<slug>.json) | New status | BifrOSt path |
|---|---|---|---|
| combine | ObservableObject | partial | springboard/sdk/src/observable_object.rs |
| dispatch | asyncAndWait | partial | springboard/sdk/src/dispatch.rs (`DispatchQueue::async_and_wait`) |

## Gaps left open

- ObservableObject: the official publisher emits before the change. Here `sink` runs after the value is stored, so
  subscribers see the new state, not the old one. `ObservableObjectPublisher` is single-threaded (Rc-based) like
  `Observable`.
- asyncAndWait: workloops are not covered, only queues.
- Exception Handling, "Delegate approval per exception" (`exception-handling`): not built. The official NSExceptionHandler
  and NSExceptionHandlerDelegate pages were not found on developer.apple.com, so no API could be mirrored from official
  facts. The existing panic hook in `springboard/src/bare/crashlog.rs` is unchanged.
- Observation rows are not closed here; they belong to `observable.rs`, which already covers most of them.

## Verification

- Not built or tested: this round ran no cargo, rustc or test commands, per the workflow rules. The new tests are
  written but not executed.
- `python3 tools/check-names.py` reported no hits.
