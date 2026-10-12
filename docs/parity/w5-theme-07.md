# Wave 5, theme 07: data types and sources behind the C-level interfaces

Theme: "C API (engine interface, data types, constants)" (`docs/parity-matrix/themes.json`, `theme-07`). The theme is
mostly C-level engine handles (JavaScriptCore contexts, DNS-SD references, GSS objects) that have no counterpart in a
pure-Rust SDK. This round builds the pieces of it that have a clear app-facing meaning and can be modelled honestly in
std Rust: the dispatch time and timer-source types, keyed archiving, and the uniform type tag classes.

## What was built

1. `springboard/sdk/src/dispatch_time.rs` (`pub mod dispatch_time`). Pure time values for the dispatch surface.
   - `DispatchTime`: a monotonic nanosecond point. `now`, `from_uptime_nanos`, `uptime_nanos`, `advanced_by`,
     `distance_to`, `duration_from` (feeds `DispatchQueue::async_after`), `+`/`-` with an interval, and
     `DISTANT_FUTURE`. All arithmetic saturates; `Never` gives `DISTANT_FUTURE`.
   - `DispatchWallTime`: microseconds since the Unix epoch. `now`, `from_timespec` (drops sub-microsecond
     nanoseconds), `unix_micros`, `advanced_by`, `+`, `DISTANT_FUTURE`.
   - `DispatchTimeInterval`: `Seconds`, `Milliseconds`, `Microseconds`, `Nanoseconds`, `Never`, with `nanoseconds`
     (i128, no overflow) and `as_duration` (negative clamps to zero).
   - `DispatchTimeoutResult`: `Success` and `TimedOut`, with `from_finished` and `is_success`.
2. `springboard/sdk/src/timer_source.rs` (`pub mod timer_source`). The timer dispatch source as a pure model, with the
   source lifecycle.
   - `TimerSource::schedule(deadline, repeating, leeway)`, `set_event_handler`, `set_cancel_handler`.
   - Lifecycle: `activate` (once), `suspend` and `resume` (nesting counts), `cancel` (final, idempotent, runs the
     cancel handler once and drops the event handler), `is_cancelled`.
   - `advance(now)` is called by the owner's loop. It fires at the deadline and never early, with the number of
     expirations since the last delivery (coalesced). Expirations that pass while suspended arrive as one event on
     resume. A repeating interval of zero or less, or `Never`, makes a one-shot timer.
   - `next_deadline` and `latest_delivery` (deadline plus leeway).
   - Only the timer source is modelled. The file descriptor, signal, process, file system, Mach and user-data
     sources need kernel objects and are not covered. Nothing runs on a thread or a queue.
3. `springboard/sdk/src/keyed_archive.rs` (`pub mod keyed_archive`). Keyed archiving with secure coding.
   - `KeyedArchiver`: `reserve` (for cycles), `set_fields`, `encode_object`, `finish(top)`. Objects are stored once in
     a flat table, so shared objects keep their identity and cycles are allowed. `finish` checks that every reserved
     object is filled and every reference names an object.
   - `Archive`: `top`, `object(uid)`, `to_json`, `from_json` (validates the document, including references).
   - `KeyedUnarchiver::new(allowed_classes)`: `decode_top`, `decode_uid` and `decode_field(field, expected)` refuse an
     object whose concrete class is not on the allow-list, and refuse a reference that is not a kind of the expected
     class. Decoding reads by index and never recurses through the graph.
   - Limits: at most `MAX_OBJECTS` (1 << 20) objects. Field keys starting with `$` are reserved.
   - Format: JSON through `coding::JSONEncoder` and `JSONDecoder`, with `$archiver`, `$version`, `$top`, `$objects`,
     `$classes`, `$uid` and `$bytes`. This is the model's own layout, not the framework's binary property-list
     format. Archives made here are not readable by the framework, and the reverse is not true.
4. `springboard/ui/src/uttype.rs` (extended). Tag classes.
   - `TagClass` (`FilenameExtension`, `MimeType`), `tags(id, class)` (the type's own declared tags, not inherited ones)
     and `type_for_tag(class, tag)` (the reverse mapping; dotted extensions are refused).

## Official API names mirrored

| Official name (developer.apple.com) | Where | REA evidence |
|---|---|---|
| `DispatchTime` (`now`, `distantFuture`, `init(uptimeNanoseconds:)`, `advanced(by:)`, `distance(to:)`, `+`, `-`) | `dispatch_time.rs` | `ev_f6d305a9e750f01b95a5401c0873726e573f5da333fbcfbbd657b63fced3c72b` |
| `DispatchWallTime` (`now`, `init(timespec:)`, `distantFuture`) | `dispatch_time.rs` | covered by the time-constructs section of the `DispatchTime` page above |
| `DispatchTimeInterval` (`seconds`, `milliseconds`, `microseconds`, `nanoseconds`, `never`) | `dispatch_time.rs` | `ev_45743c140a936e7fd8b3babc1dca744c16e7b1bef4f49e5ffc99fe4166e419f4` |
| `DispatchTimeoutResult` (`success`, `timedOut`) | `dispatch_time.rs` | covered by the time-constructs section of the `DispatchTimeInterval` page above |
| `DispatchSourceTimer` (`schedule(deadline:repeating:leeway:)`), `DispatchSourceProtocol` (`activate`, `suspend`, `resume`, `cancel`, `isCancelled`, `setEventHandler`, `setCancelHandler`) | `timer_source.rs` | `ev_7db863f41725b8b0cb5998f0e87315cfac8264dc11f083e9e88f3c981f686cf5` |
| `UTTagClass` (`filenameExtension`, `mimeType`), `UTType.tags` | `ui/src/uttype.rs` | `ev_a1e7e83103cfe9c20acd22c45561a87a46b06e972c7129a3f8030f35d540070d` |
| `NSKeyedArchiver` (`init(requiringSecureCoding:)`, `archivedData(withRootObject:requiringSecureCoding:)`) | `keyed_archive.rs` | `ev_4d2127aa0db2d793039a9d67b4e3bd6bb23e71cdcddbe3b1e3439bf7801a0e10` |
| `NSKeyedUnarchiver` (`unarchivedObject(ofClasses:from:)`, `requiresSecureCoding`, `decodeObject(forKey:)`) | `keyed_archive.rs` | `ev_b69b313c49042eb7e44e5295acb6db225eabd8bc253c8c72161fd98942b6dd1b` |

The official pages were read with `tools/parity/docc.py` (DocC JSON) for the member lists and with
`tools/parity/rea-page.sh` for the rendered evidence. The DocC facts used: `DispatchTimeInterval` has the four
counted cases and `never`; `DispatchTimeoutResult` has `success` and `timedOut`; `DispatchWallTime` has microsecond
precision; `DispatchSourceTimer` schedules with a deadline, a repeat interval and a leeway; `DispatchSourceProtocol`
has `activate`, `suspend`, `resume` and `cancel`; `UTTagClass` has `filenameExtension` and `mimeType`.

## Matrix rows closed

| Framework slug | Feature (as in the matrix data) | New status | BifrOSt path |
|---|---|---|---|
| `dispatch` | Other dispatch sources (timer, read, write, signal, process, file system) | partial | `springboard/sdk/src/timer_source.rs` (timer source model only) |
| `foundation-files-persistence` | Archives and Serialization: keyed archiving | partial | `springboard/sdk/src/keyed_archive.rs` |
| `uniform-type-identifiers` | Generic tag classes | partial | `springboard/ui/src/uttype.rs` (`TagClass`, `tags`, `type_for_tag`) |

Also improved, status unchanged (already partial): `dispatch` / "Dispatch time types". It now has a model of
`DispatchTime`, `DispatchWallTime`, `DispatchTimeInterval` and `DispatchTimeoutResult` in
`springboard/sdk/src/dispatch_time.rs`. The matrix row can point there.

## Not done, and why

- Delayed submission (`DispatchTime` to `async_after`): `dispatch.rs` still takes a `Duration`. `duration_from` is the
  bridge; the queue itself is not changed in this round.
- File descriptor, signal, process, file system, Mach and user-data dispatch sources: need kernel objects.
- The JavaScriptCore C API rows and the GameplayKit constants rows: no honest pure-Rust counterpart was built in this
  round.
- Keyed archive binary format: the framework's binary property-list layout is not reproduced.

## Verification

- `python3 tools/check-names.py`: exit 0, no hits in the new or changed `.rs` files.
- No cargo, rustc or test run was performed in this round, per the wave rules. The unit tests are written with
  hand-worked expected values (for example: a repeating timer with period 100 and deadline 1000 that is checked at
  1350 gives 3 expirations and a next deadline of 1400; a cycle of two objects decodes back to the first object).
  They are to be run in the integration build.
