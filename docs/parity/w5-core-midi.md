# Wave 5: Core MIDI (gap and OS-only rows)

Branch `parity/w5-core-midi`. Pure Rust, std only. Not built or run in this wave (the wave rules forbid builds);
the unit tests were written from hand-worked values and still need a run in the next build.

## What was built

1. **BLE MIDI packet framing** (`springboard/core/src/ble_midi.rs`, app-facing name in
   `springboard/sdk/src/ble_midi.rs`). Encodes MIDI messages into BLE MIDI packets (6-bit header timestamp,
   7-bit timestamp byte per message, packets capped at an MTU, SysEx split across packets with the timestamp
   before F7). Decodes packets back to timed messages with running status, realtime bytes, and SysEx continued
   across packets. The decoder resets after any error. Framing follows the public MIDI-over-BLE specification of
   the MIDI Association. The link layer (GATT service discovery, the connection itself) is not in this module, so
   the row is partial.
2. **Device removal in the MIDI registry** (`springboard/core/src/midi_endpoints.rs`, extended). Adds
   `remove_entity` (with its endpoints and their connections), `remove_device` (all its entities), and
   `remove_external_device` (only a plugged-in device, as unplugging does; a built-in device is refused). The
   registry still is not wired into the shell; no driver calls these yet.

## Official API names mirrored

Official pages read (DocC via `tools/parity/docc.py`, rendered via REA):

- MIDI Bluetooth: `MIDIBluetoothDriverActivateAllConnections()`, `MIDIBluetoothDriverDisconnect(_:)`.
  https://developer.apple.com/documentation/coremidi/midi-bluetooth
- MIDI System Setup: `MIDISetupRemoveDevice(_:)`, `MIDISetupRemoveExternalDevice(_:)`, `MIDIDeviceRemoveEntity(_:_:)`,
  `MIDIEntityAddOrRemoveEndpoints(_:_:_:)`. https://developer.apple.com/documentation/coremidi/midi-system-setup
- Core MIDI framework landing page: https://developer.apple.com/documentation/coremidi

Mapping (behaviour, not signatures):

| Official | BifrOSt |
|---|---|
| MIDIDeviceRemoveEntity | `Registry::remove_entity` |
| MIDISetupRemoveDevice | `Registry::remove_device` |
| MIDISetupRemoveExternalDevice | `Registry::remove_external_device` |
| MIDIBluetoothDriverDisconnect | not mapped: no connection layer in this wave |
| MIDIBluetoothDriverActivateAllConnections | not mapped: no connection layer in this wave |
| MIDIEntityAddOrRemoveEndpoints | not mapped: `Registry::add_endpoint` exists, removing a single hardware endpoint does not |

## REA evidence

- `ev_13f2dd2d149d8e630038151abfb9fe6e5376dbdfb484c0bda3d00fbd2656e791` (MIDI Bluetooth page)
- `ev_64b3da8859f35a7e91d4d12871b4534430a94826f63e589dfc3a06492b20f4cb` (MIDI System Setup page)
- `ev_ff237341853cc3ad524cdb256d2a4d8916eb80c73f64929f07ea8fdc29280637` (Core MIDI landing page)

## Matrix rows closed (for the lead to update centrally)

| Framework | Feature | New status | BifrOSt path |
|---|---|---|---|
| Core MIDI | MIDI Bluetooth (BLE MIDI peripherals) | partial | springboard/core/src/ble_midi.rs, springboard/sdk/src/ble_midi.rs |
| Core MIDI | MIDI System Setup (driver and external device management) | partial | springboard/core/src/midi_endpoints.rs (remove_entity, remove_device, remove_external_device) |

## Still missing

- BLE GATT service discovery and the MIDI I/O characteristic; the link layer that calls `Decoder::decode` and writes
  `encode` output.
- Registry wiring: no driver adds or removes devices through it, and no SDK API exposes the registry.
- MIDI Capability Inquiry and MIDI Networking rows are untouched in this wave.
