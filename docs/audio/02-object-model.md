# Audio 02: object model, property table and control protocol

Branch `audio/02-object-model`. Contract: `docs/audio-architecture.md` ("Object model").

## Implemented

- `springboard/audio/types/src/props.rs`: the full property table `PROPERTIES` (32 rows) with `PropDef` (selector,
  name, `ValueKind`, scope bits, carrying classes, settable), `lookup`, `check_type`, `scope_bit`. Selector
  constants are unchanged from the skeleton.
- `springboard/audio/types/src/control.rs`: the binary codec for `Request` and `Reply` (little endian, tag byte
  first, strings and byte arrays length-prefixed, scope as a byte, `PropertyValue` tagged 0 to 14). Hostile-input
  safe: counts are checked against the bytes left (no allocation from a lying count), trailing bytes, bad tags,
  scopes, bools, UTF-8 and over-`MAX_LEN` messages are errors.
- `springboard/audio/server/src/objects.rs`: `ObjectTree` with `add`, `remove` (recursive), `owned`, `define`
  (server setup, checks class, scope and type against the table), `update` (the server's own change, ignores the
  settable flag), `get`, `set` (client set: UnknownProperty, NotSettable, BadValueType), `listen`, `unlisten`,
  `drop_client`, and `flush` (one coalesced notice per client per main-loop turn, deduplicated by address).
  Declared in `server/src/lib.rs` with `pub mod objects;`.

## Not done (todo_left)

- Range checks (supported rates, buffer range) and hog ownership belong to the device layer calling `set`.
- Wiring `flush` to `Reply::Changed` and calling it once per loop turn is the control task (05).
- Logging: the module is pure; the server loop logs under `is.olibuijr.audio` / `client`.

## Midget verification (lead)

No hardware item in this task. After merging and once task 05 serves the socket:

1. `cd springboard && cargo test -p bifrost-audio-types props control` and `cargo test -p bifrost-audiod objects`
2. `python3 tools/check-names.py`
3. On Midget with the daemon running: `bifrost pc audio ls`, `bifrost pc audio get <device> nsrt`,
   `bifrost pc audio set <device> nsrt 44100`, `bifrost pc audio listen <device> fsiz` while another client sets the
   buffer size: expect exactly one notice per change burst.
