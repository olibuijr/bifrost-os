# Event descriptors (sdk-complete-44)

Mirrors the descriptor and object-specifier part of the reference event system (`AEDesc` coercion and object specifiers
in the Core Services Apple Events rows).

## What is built

- `springboard/sdk/src/event_descriptor.rs`: `Descriptor` (null, boolean, integer, real, text, list) with `coerce` to a
  `Kind`, and `from_json` for values read from a parameter record.
- `ObjectSpecifier` (class, `Form` of index, name, identifier, first or last, optional container) resolved against a tree
  of `Element`s with `resolve`.
- Tests: `springboard/sdk/tests/parity_sdk-complete-44.rs`, headless and deterministic.

## What is wired

- The module is registered in `springboard/sdk/src/lib.rs` as `bifrost_app::event_descriptor`.
- `springboard/sdk/src/app_events.rs` carries the event method and parameter record; descriptors are not yet encoded
  into the control socket.

## What needs the lab machine

- A round trip of an object specifier between two running apps over the control socket on Midget.
