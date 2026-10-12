# Wave 3: Core MIDI (pure Rust models)

Framework: Core MIDI, https://developer.apple.com/documentation/coremidi (official DocC index read with
`tools/parity/docc.py`).

REA evidence of the rendered landing page: `ev_9fad8b36a5c90af601086b6d4714c8c5798cc6c400f58ac711302ea4575fb861`.
The page lists the topics this wave covers (MIDI Services, MIDI Thru Connection, MIDI Networking, MIDI Capability
Inquiry). Official API names mirrored by this wave, from the DocC pages for `coremidi/midi-thru-connection` and
`coremidi/midi-services`: `MIDIValueMap`, `MIDIControlTransform`, `MIDIThruConnectionParams`, `MIDIThruConnectionCreate`,
`MIDIThruConnectionSetParams`, `MIDIClientCreate`, `MIDIClientDispose`, `MIDIGetNumberOfDevices`,
`MIDIGetNumberOfExternalDevices`, `MIDIGetExternalDevice`, `MIDIDeviceGetEntity`, `MIDIEntityGetSource`,
`MIDIEntityGetDestination`, `MIDIPortConnectSource`, `MIDIPortDisconnectSource`, `MIDISourceCreateWithProtocol`,
`MIDIDestinationCreateWithProtocol`, `MIDIEndpointDispose`, `MIDIEndpointSetRefCons`, `MIDIEndpointGetRefCons`,
`MIDIGetNumberOfSources`, `MIDIGetSource`, `MIDIGetNumberOfDestinations`, `MIDIGetDestination`.

## What was built

### `springboard/ui/src/midi_thru.rs` (bifrost-ui)

Thru connection model for play-through of note and control events.

- `ValueMap`: a 128-entry 7-bit lookup table (`identity`, `from_table`, `linear`, `set`, `map`). Mirrors `MIDIValueMap`.
  `linear` rejects an empty input range and clamps inputs to the range ends; descending ranges are allowed.
- `ControlTransform`: renumbers one control change and maps its value. Mirrors `MIDIControlTransform`.
- `ThruEvent` (note on, note off, control change) and `ThruParams` (channel filter, note map, velocity map,
  control transforms). `ThruParams` mirrors `MIDIThruConnectionParams`.
- `ThruConnection`: `process` returns the event to forward, or `None` when disabled or filtered out.
  Mirrors the create and set-params calls as an in-memory object; no port is opened.

Not covered: the Core MIDI transform types `MIDITransform`, `MIDITransformType` and `MIDITransformControlType`
beyond the value map and control transform, persistent connections (`MIDIThruConnectionFind`), and endpoints
(`MIDIThruConnectionEndpoint`). The thru path runs on in-memory events only.

### `springboard/core/src/midi_endpoints.rs` (bifrost-core)

Object registry for MIDI services: clients, devices, entities, endpoints and client-to-source connections.

- `Registry::create_client`, `dispose_client` (removes the client's virtual endpoints and connections), `client_name`.
- `add_device` / `add_entity` / `add_endpoint` for hardware objects, and `create_virtual_endpoint` /
  `dispose_endpoint` for client-owned virtual sources and destinations (hardware endpoints cannot be disposed).
- Lookup and counts in identifier order: `device`, `entity`, `endpoint`, `number_of_devices`,
  `number_of_external_devices`, `external_device_at`, `number_of_endpoints`, `endpoint_at`.
- `set_ref_con` / `ref_con` mirror `MIDIEndpointSetRefCons` / `MIDIEndpointGetRefCons`.
- `connect_source`, `disconnect_source`, `is_connected`, `sources_for` mirror `MIDIPortConnectSource` and
  `MIDIPortDisconnectSource`.

Not covered: real client ports and the receive path (`MIDIInputPortCreateWithProtocol`, `MIDIReceiveBlock`),
event lists and packets, hardware discovery (no driver is probed), UMP protocol selection and object unique
identifiers. Lookup by unique identifier (`MIDIObjectFindByUniqueID`) is not implemented; identifiers are plain
`ObjectId` values that are unique per registry.

## Matrix rows closed (new status partial)

| Framework slug | Feature (as written in `docs/parity-matrix/data/core-midi.json`) | New status | BifrOSt path |
|---|---|---|---|
| core-midi | MIDI Thru Connection (play-through, value maps, transforms) | partial | springboard/ui/src/midi_thru.rs |
| core-midi | MIDI Services (clients, ports, device and entity lookup) | partial | springboard/core/src/midi_endpoints.rs |

Both rows stay `partial` rather than `parity`: the thru connection has no port wiring to an audio or MIDI device, and
the services registry has no receive path or hardware discovery. Neither row is tested on Midget.

## Tests

- `springboard/ui/src/midi_thru.rs`: 9 tests (identity map and masking, linear truncation, clamping, descending
  ranges and refused empty ranges, refused tables and values, control transform renumbering, note on and channel
  filter, note off and control change, disabled connection and default pass-through).
- `springboard/core/src/midi_endpoints.rs`: 6 tests (identifier allocation and no reuse, hardware endpoints in entities
  and devices, counts and indices by direction, connect rules, disposing a client, disposing endpoints and ref cons).

Expected values were worked by hand from the rules in the code (for example `64 * 64 / 127 = 32`,
`13 * 127 / 27 = 61`, `100 * 64 / 127 = 50`). No builds were run for this wave.
