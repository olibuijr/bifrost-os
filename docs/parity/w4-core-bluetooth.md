# Parity wave 4: Core Bluetooth

Official reference: https://developer.apple.com/documentation/corebluetooth

## What was built

Two pure Rust models in `springboard/ui/src` (bifrost-ui, no radio, no transport, std only). One small addition to an existing module.

- `springboard/ui/src/gatt_server.rs` (new): the peripheral role's local GATT database. Services, characteristics and descriptors get handles in the order they are added. Reads and writes go through the same permission rules an attribute server applies: the characteristic property (READ, WRITE, WRITE_WITHOUT_RESPONSE), the attribute permission bits, and encrypted-only attributes that need an encrypted link. Writes are capped at 512 bytes, and the client configuration descriptor (0x2902) must be two bytes. `is_notifying` reads that descriptor and reports whether notifications (NOTIFY) or indications (INDICATE) are enabled, when the characteristic allows them. Descriptors can only attach to the most recent characteristic.
- `springboard/ui/src/advertising.rs` (new): advertising data. The builder packs flags, the complete local name, 16-bit or 128-bit service UUID lists and manufacturer data into a payload that refuses to exceed the 31-byte legacy limit or a 254-byte field. The parser reads the structures back, stopping at a zero length or a truncated structure, and gives the flags, the local name (complete wins over shortened), the service UUIDs and the manufacturer data for one company.
- `springboard/ui/src/gatt.rs` (extended): `Uuid::from_bytes_le`, the inverse of `to_bytes_le`, used by the advertising parser.
- `springboard/ui/src/lib.rs`: one `pub mod` line each for `gatt_server` and `advertising`.

These are the models the shell and apps would need. Nothing in the shell yet advertises or serves GATT over the radio; no transport is wired in.

## Official API names mirrored

- `CBPeripheralManager`: `add(_:)`, `remove(_:)`, `removeAllServices()`, `startAdvertising(_:)`, `stopAdvertising()`, `isAdvertising`, `updateValue(_:for:onSubscribedCentrals:)`, `respond(to:withResult:)`. The model covers the GATT database and the notification state. It does not cover the radio side of advertising or respond-to-request.
- `CBMutableService`, `CBMutableCharacteristic` (`init(type:properties:value:permissions:)`, `value`, `descriptors`, `properties`, `permissions`, `subscribedCentrals`), `CBMutableDescriptor`.
- `CBAttributePermissions`: the read, write and encryption permission bits of `AttrPermissions`.
- `CBAdvertisementDataLocalNameKey` (the local name of a peripheral): the complete and shortened name structures of `advertising`.
- `CBATTError`: `AttError` mirrors the error cases the read and write paths return. Its code values are not copied.

## REA evidence

- `ev_1658a43e52276c05a4d26b7cc4a6b046e143cc0b4944a34b14272110ec9e72f6`: Core Bluetooth landing page (framework overview, topic list, background-mode note).
- `ev_02f9f53f93ebb354091000aee758bc884be40b9960984ed5a1df9d20ef75d8e9`: CBPeripheralManager (advertising, GATT database, respond-to-request methods).
- `ev_1f0555b3a5a453541f54f845e23421e5eabc20416407ed17cd60300a56491cbb`: CBMutableCharacteristic (init signature, value, descriptors, properties, permissions, subscribed centrals).

## Matrix rows closed

These rows go from their previous status to `partial` in `docs/parity-matrix/data/core-bluetooth.json`. The lead updates the matrix centrally.

| Framework slug | Feature | New status | BifrOSt path |
|---|---|---|---|
| core-bluetooth | Peripheral role: advertising and local GATT server | partial | springboard/ui/src/gatt_server.rs (GATT database and notification state); springboard/ui/src/advertising.rs (advertising payload builder) |
| core-bluetooth | Discovered peripheral and advertisement data | partial | springboard/ui/src/advertising.rs (parse, flags, local_name, service_uuids, manufacturer_data) |

## Not done

- Central manager state (`os`) and channel sounding (`gap`, needs controller hardware) are not touched.
- Background execution stays a gap: no app-level Bluetooth background policy exists.
- Scanning, connection and pairing stay `os` (the shell's mgmt socket, not an app API).
- No build or tests were run in this round, per the wave rules. The modules carry `#[cfg(test)]` tests with hand-worked expectations for the lead to run.
