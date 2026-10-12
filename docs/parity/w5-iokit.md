# Wave 5: IOKit device registry for apps

Branch `parity/w5-iokit`, on top of `parity/integration` (wave 4). Framework: IOKit (`docs/parity-matrix/data/iokit.json`).

## What was built

- `springboard/core/src/device_registry.rs` (pure model, std only): a `Device` (class, name, string properties),
  a `Matching` query (optional class, property values that must all be equal), `matching` and `find` over a
  snapshot, `changes` that diffs two snapshots by (class, name) into `Change::Arrived` and `Change::Removed`, and
  `scan_in`, which reads a sysfs-style tree (one directory per class, one child per device, one file per property).
  It reads files and writes nothing.
- `springboard/sdk/src/device_registry.rs` (app-facing): re-exports the model, `devices(class, keys)` over the
  kernel class root, and `DeviceWatch`, which keeps the last snapshot and returns the changes on each `observe`.
  Change events come from polling, not from a kernel notification. Nothing here opens a device or calls a driver.

Both modules and `pub mod device_registry;` are registered in `springboard/core/src/lib.rs` and
`springboard/sdk/src/lib.rs`.

## Official API names mirrored

These are the names of the official pages the rows cite (feature rows as written in the matrix):

- Device matching and service discovery: IOServiceGetMatchingServices, IOServiceMatching, IOIteratorNext
  (IOKit Functions, IOKitLib.h). Mirrored as `Matching`, `matching`, `find`, `devices`.
- Device registry tree and properties: IORegistryEntryGetProperty, IORegistryEntryCopyPath (IOKit Functions).
  Mirrored as `Device::property`, `scan_in`. Flat list only: no parent or child tree and no path copy.
- Device add and remove notifications: IONotificationPortCreate, IOServiceAddMatchingNotification
  (IOKit Functions). Mirrored as `changes`, `Change`, `DeviceWatch::observe`. Polled, not pushed, and no
  dispatch-queue delivery.

Official documentation used: https://developer.apple.com/documentation/iokit (landing page: "Access hardware devices
and drivers from your apps and services"; reference groups IOKitLib.h, IOKit Functions, IOKit Data Types). The
per-function pages were not fetched: the DocC paths tried (`iokit/ioregistryentry` and similar) returned 404 through
`tools/parity/docc.py`.

## REA evidence

- `ev_22503c82ab85718b26d8d932c55a8e1731e0dc60a47cbde9b3f0a864c47867d6`: rendered landing page
  `https://developer.apple.com/documentation/iokit`, title "IOKit | Apple Developer Documentation".

## Matrix rows closed

| framework slug | feature (as written in data/iokit.json) | new status | BifrOSt path |
|---|---|---|---|
| iokit | Device matching and service discovery | partial | springboard/core/src/device_registry.rs; springboard/sdk/src/device_registry.rs |
| iokit | Device registry tree and properties | partial | springboard/core/src/device_registry.rs (scan_in, Device::property) |
| iokit | Device add and remove notifications | partial | springboard/core/src/device_registry.rs (changes); springboard/sdk/src/device_registry.rs (DeviceWatch) |

## Not done, and why

- Registry tree (parent and child paths): the sysfs scan is flat, and no caller needs the tree yet.
- Real sysfs reads from the app sandbox: `CLASS_ROOT` is `/sys/class`, but whether the app sandbox binds `/sys` was not
  checked. Until it is, `devices` returns what the sandbox lets through.
- Driver calls, shared-memory queues, and HID or USB matching stay `os` rows. They need a driver-side or
  permission design, not a pure model.

## Verification

Per the task rules no build or test was run. `python3 tools/check-names.py` was run and reported no hits.
The test modules hold 14 behaviour tests (9 in core, 5 in sdk) with expected values worked out by hand.
