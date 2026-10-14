# Force Feedback (parity team K)

Official reference: https://developer.apple.com/documentation/forcefeedback. The framework is a legacy C API for
force-feedback devices such as wheels and joysticks. The earlier wave-5 round is in `docs/parity/w5-force-feedback.md`.

This round closes the two gap rows that have a buildable core, and leaves one gap with a stated reason. Every model
is pure std code in `springboard/core`; none is wired to an evdev node or to the shell, so the matrix status is
`partial`.

## What was built

### `springboard/core/src/ff_escape.rs` (`Escape`, `EscapeError`)

The hardware escape row. An escape is a driver command code and a payload of up to 4096 bytes. `Escape::encode`
frames it as a little-endian code, a little-endian length and the payload. `Escape::decode` refuses a short frame, a
payload longer than the limit, and bytes after the declared payload. The frame layout is BifrOSt's own; it is not a
kernel format, and no driver reads it yet.

Tests: 7 (limit, byte order, round trip, empty payload, truncation, trailing bytes, oversized header).

### `springboard/core/src/haptic_request.rs` (`Requests`, `Outcome`)

The app-facing haptic row. `Requests::request` takes a device, an app id, a `force_effect::Pattern` and the time, and
plays one pulse through `Device::pulse`. An app may get one accepted pulse per `MIN_INTERVAL_MS` (30 ms); a request
inside that gap is `Throttled`. A time that goes backwards is also throttled. A full device returns `Refused(Full)`,
and a refused pulse does not start the throttle. `Requests::forget` clears an app when it quits.

Tests: 7 (first request, throttle window, exact interval, per-app separation, backwards time, full device, forget).

## Matrix rows

| Feature | New status | BifrOSt path |
|---|---|---|
| Hardware escape commands | partial | `springboard/core/src/ff_escape.rs` |
| App-facing haptic request | partial | `springboard/core/src/haptic_request.rs`, `springboard/core/src/force_effect.rs` |
| Cooperative level and coordinate system | gap (kept) | none |

## Not done

- The shell still drops `ToShell::Haptic` (`springboard/src/shell/apphost.rs`, default arm). The next step is a
  `Haptic` arm that holds a `Requests` and a `Device`, which needs a shell edit and an evdev node to play on.
- Escape frames have no driver to write to, so no escape reaches hardware.
- Cooperative level stays a gap. The reference API marks `FFDeviceSetCooperativeLevel` as unimplemented in its
  version 1.0 reference, so there is no behaviour to match.
- Not verified on Midget and not run on the host; the lead builds and tests this branch.
