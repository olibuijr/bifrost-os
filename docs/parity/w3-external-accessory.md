# Wave 3: External Accessory (accessory registry, notifications, sessions)

Branch `parity/w3-external-accessory`, module `springboard/sdk/src/external_accessory.rs` (`bifrost_app::external_accessory`).

## What was built

A pure model, with no device access, that an app can use to follow connected hardware accessories:

- `AccessoryManager`: registry of attached accessories. The system layer calls `attach` and `detach`. While the app is
  registered for notifications, each change is queued as an `AccessoryEvent::Connected` or `Disconnected`, and the app
  drains the queue with `take_events`. Changes made before registration are not queued.
- `Accessory`: identity (name, manufacturer, model number, serial number), declared protocol strings, connection id and
  connected state. `supports_protocol` is an exact string match.
- `accessories_for_protocol`: lists attached accessories that declare a protocol string.
- `Session`: a data session over one declared protocol, opened with `AccessoryManager::session`. It holds an input
  stream and an output stream, both built on the existing in-memory streams in `springboard/sdk/src/stream.rs`. The
  transport calls `Session::receive` with bytes from the device and `Session::take_output` to get bytes to send.
- `AccessoryError`: `NotConnected` and `ProtocolNotSupported`, with `Display` messages.

Changes to existing modules (extensions, no behaviour change for current callers):

- `springboard/sdk/src/stream.rs`: `InputStream::append` (bytes that arrive after creation; a stream at its end reads
  them next) and `OutputStream::take_written` (removes the stored bytes, freeing capacity). One test added.
- `springboard/sdk/src/lib.rs`: one `pub mod external_accessory;` line.

## Official API names mirrored

Framework External Accessory (https://developer.apple.com/documentation/externalaccessory):

- `EAAccessoryManager`: `shared()` (here `AccessoryManager::new`, the shell owns the one instance), `registerForLocalNotifications`,
  `unregisterForLocalNotifications`, `connectedAccessories`, and the connect and disconnect notifications
  (`AccessoryEvent`).
- `EAAccessory`: `name`, `manufacturer`, `modelNumber`, `serialNumber`, `protocolStrings`, `isConnected`, `connectionID`.
- `EASession`: `init(accessory:forProtocol:)` (here `AccessoryManager::session`), `accessory`, `protocolString`,
  `inputStream`, `outputStream`.

Not mirrored: `showBluetoothAccessoryPicker`, the Wi-Fi accessory configuration classes, and the
`UISupportedExternalAccessoryProtocols` manifest key. These need the shell or the permission manifest.

## REA evidence

- `ev_675edd517fbb4eb3bc84bed0657dcdc64c08ae927336f9880caf8abedd7a2143`: External Accessory landing page.
- `ev_e74a3d0a27a4c1025253176b419126c709287568227a9106a168421f857bfd71`: EAAccessoryManager page.

## Matrix rows closed (status partial)

| Framework | Feature | Status | BifrOSt path |
|---|---|---|---|
| External Accessory | Accessory manager and connect/disconnect notifications | partial | springboard/sdk/src/external_accessory.rs |
| External Accessory | Protocol matching (declared protocols) | partial | springboard/sdk/src/external_accessory.rs |
| External Accessory | Data session streams | partial | springboard/sdk/src/external_accessory.rs, springboard/sdk/src/stream.rs |

Why partial and not parity:

- Nothing calls `attach`, `detach`, `receive` or `take_output` yet, so no real device reaches an app. The shell wiring
  (for example from `springboard/src/bare/bluetooth.rs` and `springboard/src/bare/input.rs`) and the transport for
  serial or USB are still missing.
- The protocol match does not read the manifest key, and `springboard/src/bare/app_policy.rs` has no accessory grant,
  so an app cannot declare protocols or be sandboxed to them.
- The Bluetooth picker, Wi-Fi accessory configuration and the picker's permission sheet remain gaps.

## Tests

`springboard/sdk/src/external_accessory.rs` has 8 tests: queued events in order while registered, no queueing before
registration, duplicate attach and unknown detach, protocol matching, session errors and detach, bytes both ways through
the streams, closed session refusing writes, and error messages. `stream.rs` gained `append_after_end_reopens_reading`.
No build or test was run in this wave (no-build rule); the tests are to be run in the integration build.
