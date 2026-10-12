# Wave 4, theme 05: peripheral role (advertising and local GATT server)

## What was built

`springboard/ui/src/peripheral.rs` (bifrost-ui, pure model, std only, no radio): the peripheral side of the
Bluetooth low energy stack. It reuses the types of `springboard/ui/src/gatt.rs` (`Uuid`, `CharProperties`,
`AttrPermissions`, `Descriptor`, `WriteType`, `DEFAULT_MTU`) rather than duplicating them. The central role
(`GattClient`) was already in `gatt.rs` from wave 2 and is unchanged.

- `PeripheralManager`: power state (`ManagerState`), the local GATT database (`MutableService`,
  `MutableCharacteristic` with properties, permissions, optional stored value and descriptors), advertising
  (`AdvertisingData`, with the 31-byte legacy payload budget counted with the flags structure), connected centrals
  (`connect`, `disconnect`, `set_mtu`), notification subscriptions (`set_subscription`, `subscribed_centrals`),
  attribute requests (`on_read`, `on_write`, `pending`, `respond`, `take_replies`) and value updates
  (`update_value`, `set_ready`).
- Typed ATT error codes (`AttError`, with the ATT numeric codes) and `PeripheralError`, both with `Display`.
- A characteristic with a stored value answers reads and writes at once. One without a value queues a
  `ServerRequest` for the app, and the app answers with `respond`.
- Leaving the powered-on state stops advertising, drops connections and clears queued requests.
  Published services stay in the database.

Not wired into apps, the shell or the Bluetooth daemon. The transport that feeds this model does not exist yet.

## Official API names mirrored

Official pages, read with `tools/parity/docc.py`:

- `CBPeripheralManager`: `init(delegate:queue:)`, `add(_:)`, `remove(_:)`, `removeAllServices()`,
  `startAdvertising(_:)`, `stopAdvertising()`, `isAdvertising`, `updateValue(_:for:onSubscribedCentrals:)`,
  `respond(to:withResult:)`, the state enum `CBPeripheralManagerState`. The page states that the manager must be
  powered on before its methods are called.
- `CBATTRequest`: `central`, `characteristic`, `value`, `offset`.
- `CBMutableCharacteristic`: `init(type:properties:value:permissions:)`, `value`, `descriptors`, `properties`,
  `permissions`, `subscribedCentrals`.

## REA evidence

- `ev_1506d7519da8bb3b89a95598aef1af075ceb0941391c77a2386d95958239d0d6`: rendered page for the peripheral manager
  (services are disabled in the background unless a background mode is declared; the manager must be powered on).
- `ev_46c6817bf3ed161d49d4cab3a44c956d2cb576e9c8249d6448772e32abc6bcb9`: rendered page for the ATT request
  (central, characteristic, value, offset; answered with `respond`).

The official page for the ATT error enum was not fetched (its DocC path returned 404). The ATT error numbers
come from the Bluetooth core specification, not from Apple.

## Rules the model chooses (not Apple's documented behaviour)

- Advertising data is budgeted at 31 bytes with the flags structure. An over-long request fails with
  `AdvertisingTooLarge`. Apple's documentation does not state this limit.
- For a read, the app answers with the whole value and the model sends the bytes from the request offset.
- A write with a stored value is applied by the model, and writes without a stored value go to the app. Apple's
  documentation does not say how a stored value handles writes.
- `update_value` fails with `TransmitQueueFull` until `set_ready(true)`. Apple returns a boolean.
- Notifications are used when the notify property is set, and indications otherwise.

## Matrix rows closed

| framework slug | feature (exactly as in the matrix) | status | BifrOSt path |
|---|---|---|---|
| core-bluetooth | Peripheral role: advertising and local GATT server | partial | springboard/ui/src/peripheral.rs |

The row "ATT and Core Bluetooth errors" in `core-bluetooth` now has typed ATT error codes in the model. Its
status stays partial, so it is not listed as closed. The rows "Background execution and Live Activity privileges"
and "Usage permission and privacy declarations" have no counterpart in this change.

## Verification

Not built or run, as the task requires. The tests live in `springboard/ui/src/peripheral.rs` (`mod tests`, nine
tests). They work out expected values by hand: advertising payload sizes (13, 27, 31, 34), power gating, read
offsets and property checks, the write permission, encryption and length cap (512), notification MTU limits
(20 and 182 bytes), the transmit queue and filter, and cleanup on disconnect. `tools/check-names.py` passes.
