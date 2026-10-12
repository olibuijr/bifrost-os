# Parity wave 5, theme-04: Bluetooth MIDI peripheral picker and advertising

Status: partial. Pure models only. Nothing is scanned, advertised or connected yet, and nothing is wired into the
shell or an app.

## What was built

- `springboard/ui/src/midi_ble.rs` (`bifrost_ui::midi_ble`): the BLE MIDI packet wire format. It holds the MIDI
  service UUID (03B80E5A-EDE8-4B33-A751-6CE34EC4C700), `encode` (one packet with a 13-bit millisecond timestamp, a
  timestamp byte before each message, and an MTU check that subtracts the 3-byte ATT header), and `decode` (running
  status, real-time bytes, and system exclusive with timestamp bytes inside it).
- `springboard/ui/src/midi_ble_picker.rs` (`bifrost_ui::midi_ble_picker`):
  - `MidiPicker` is the peripheral list a central shows. It keeps only advertisements that carry the MIDI service
    UUID, sorts connected peripherals first and then by name, selects and forgets peripherals, and prunes idle ones
    while keeping connected ones.
  - `MidiAdvertiser` is the advertisement a local device sends. It builds flags, the MIDI service UUID and the
    name within the 31-byte legacy limit. The name is shortened on a character boundary when it does not fit, and
    the model counts connected centrals.
- `springboard/ui/src/lib.rs`: two `pub mod` lines added.

Both modules reuse existing code: `advertising` (AD structure builder and parser) and `gatt::Uuid`. Nothing
duplicates them.

## Official API names mirrored

Source: DocC pages on developer.apple.com, read with `tools/parity/docc.py`.

- CoreAudioKit, "Bluetooth Devices" group: `CABTLEMIDIWindowController` (a window controller that displays nearby
  Bluetooth-based MIDI peripherals), `CABTMIDICentralViewController` (the same list as a view controller),
  `CABTMIDILocalPeripheralViewController` (advertises the device as a Bluetooth-based MIDI peripheral). Page:
  https://developer.apple.com/documentation/coreaudiokit
- MIDI Bluetooth (BLE MIDI peripherals): the page states the framework connects to Bluetooth Low Energy MIDI
  peripherals. Its symbols are `MIDIBluetoothDriverActivateAllConnections()` and `MIDIBluetoothDriverDisconnect(_:)`.
  Page: https://developer.apple.com/documentation/coremidi/midi-bluetooth

The packet format and the MIDI service UUID follow the BLE MIDI transport specification. The Apple pages above do
not state them, so the packet-level behaviour here is not sourced from them.

## REA evidence

- `ev_fe3bb87cad5c6c66ede0d149cddbfddc7ca75f48cd572222a602976377372a63`: CoreAudioKit landing page
  (https://developer.apple.com/documentation/coreaudiokit).
- `ev_1749730ae95769f840e0070ec5aebc169aebfdcafaeb0aedf954ed33ace97a35`: MIDI Bluetooth page
  (https://developer.apple.com/documentation/coremidi/midi-bluetooth).
- A REA page for `coreaudiokit/bluetooth-devices` returned the site's "page not found" text, so it is not used.

## Tests

Each module has a `#[cfg(test)] mod tests` block. Expected values were worked by hand from the rules above: the
timestamp split (8191 gives header 0xBF and low byte 0xFF), the MTU limit (a 32-byte packet against a 20-byte limit at
MTU 23), running status, system exclusive timestamp skipping, the advertisement length (26 bytes for name "Pad", 31
for a shortened name), and the 16 to 8 byte cut on a character boundary.

## Matrix rows closed (partial)

| framework slug | feature (as in data/<slug>.json) | status | BifrOSt path |
|---|---|---|---|
| coreaudiokit | Bluetooth MIDI peripheral picker and advertising | partial | springboard/ui/src/midi_ble_picker.rs |
| core-midi | MIDI Bluetooth (BLE MIDI peripherals) | partial | springboard/ui/src/midi_ble.rs |

The core-midi row is listed as partial for the packet codec and the service UUID. It is outside theme-04 but is the
same BLE MIDI transport. The lead decides whether it is recorded.

## Not done

- No radio, no GATT client or server, no scan or connect. The Bluetooth management socket in
  `springboard/src/bare/bluetooth.rs` still has no MIDI service handling.
- No system exclusive reassembly across packets (decode reports `Truncated`).
- Not wired into the shell or any app. Not tested on the VM or the Midget.
- Other theme-04 rows (instrument analysis, MIDI-CI, network MIDI and others) are not touched.
