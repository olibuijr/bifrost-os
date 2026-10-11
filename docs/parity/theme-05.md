# Theme 05: central role GATT client

## What was built

`springboard/ui/src/gatt.rs` (`bifrost_ui::gatt`), a pure model of the central-role GATT client. It has no radio and no
transport. Its parts:

- `Uuid`: 16-, 32- and 128-bit UUIDs. Short forms expand on the Bluetooth base UUID. `uuid_string()` gives the short
  form for aliases and the dashed upper-case form otherwise.
- `CharProperties`: the ten characteristic property flags, with raw bits from the characteristic property field of the
  Bluetooth core specification (the official page names the flags but does not list the raw values).
- `AttrPermissions`: readable, writeable and the two encryption requirements, with `allows_read` and `allows_write`
  checks for a link's encryption state.
- `GattClient`: the remote tree (services, then characteristics, then descriptors), filled in by `on_services`,
  `on_characteristics`, `on_descriptors` and `on_value`. The tree stays "not discovered" until its level reports.
  Each operation returns an `AttRequest` for the transport to send:
  - `read` and `read_descriptor` check the read property.
  - `write` checks the property for the write kind and the length limit. Without response the limit is MTU minus 3,
    and with response it is the 512-byte attribute cap.
  - `set_notify` writes the Client Characteristic Configuration descriptor (0x2902, value 0x0001, 0x0002 or 0x0000). It
    enforces the encryption requirement and refuses a characteristic whose discovered descriptors lack the descriptor.
  - `confirm_notify` records the state after the acknowledgement.
- `WriteType`: with response and without response.

Tests live in the `tests` module of `gatt.rs`: 9 behaviour tests cover UUID parsing and round trips, malformed UUIDs,
flag and permission checks, discovery ordering, read property gating, write property and MTU checks, notify
descriptor bytes and state, encryption-gated notify, and the missing-descriptor refusal. `lib.rs` gained one line,
`pub mod gatt;`. The name guard (`python3 tools/check-names.py`) passes. No build was run, as the task required.

## Official API names mirrored

Source: DocC JSON from `python3 -I tools/parity/docc.py`:

- `corebluetooth/cbperipheral`: discovery, read, write, notify and `maximumWriteValueLength(for:)` method names
- `corebluetooth/cbcharacteristic`: `properties`, `descriptors`, `isNotifying`, `value`
- `corebluetooth/cbcharacteristicproperties`: `read`, `write`, `writeWithoutResponse`, `notify`, `indicate`, the
  encryption-required flags
- `corebluetooth/cbcharacteristicwritetype`: `withResponse`, `withoutResponse`
- `corebluetooth/cbdescriptor`: `value`
- `corebluetooth/cbattributepermissions`: `readable`, `writeable`, `readEncryptionRequired`, `writeEncryptionRequired`
- `corebluetooth/cbuuid`: `init(string:)`, `uuidString`, `data`
- `corebluetooth/cbmutablecharacteristic`: `permissions` and `subscribedCentrals` (the attribute permission model)

The mirrored names are Bluetooth-standard names, so the owner rule holds: the code uses none of the framework's
identifiers.

REA evidence (rendered official pages):

- `ev_e7a8c06d55eb95c5008d4bfbf68242bce28586b0023f2004520906d0b536e264` (landing page, `corebluetooth`)
- `ev_7c7d761259e82a9ce6a504870ffe3d018e2ae496a746ab0d123b5f055e476ef4` (`corebluetooth/cbcharacteristic`)

Two sub-paths guessed for the DocC tool (`corebluetooth/cbcharacteristic/writetype` and
`corebluetooth/cbcharacteristic/properties`) returned 404. The pages listed above all resolved, and the mapping uses
only those.

## Matrix rows closed

| framework slug | feature (as in data/<slug>.json) | new status | BifrOSt path |
|---|---|---|---|
| core-bluetooth | Central role: GATT client (services, characteristics, descriptors) | partial | springboard/ui/src/gatt.rs (`GattClient`) |
| core-bluetooth | UUIDs and attribute permissions | partial | springboard/ui/src/gatt.rs (`Uuid`, `AttrPermissions`) |

## Not closed, and why

- Peripheral role (advertising and the local GATT server) is not closed. `AttrPermissions` is the attribute-permission
  model a local server would need, but there is no server or ATT responder.
- The rest of the theme's rows (IOBluetooth hands-free, OBEX, RFCOMM, SDP, the object push and passkey UIs, Network
  service discovery, External Accessory, AVRouting, Matter and XPC) belong to other frameworks and other subsystems.
  None maps onto a GATT client.
- `springboard/src/bare/bluetooth.rs` is the existing management-socket link and key code. It is not a GATT client, and
  the model here does not touch it. An SDK API for apps and a transport would be the next step.

The parity search tool (`tools/parity/parity.py search`) fails in this worktree: its embedding file is not a zip
archive. The index search (`code_search`) was used for the code lookup instead.
