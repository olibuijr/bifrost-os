# Core MIDI (team n, branch parity/team-n-core-midi)

Matrix data: `docs/parity-matrix/data/core-midi.json`. Status: partial, built on `parity/team-n-core-midi`, not yet
verified on Midget.

## What is built

The MIDI stack exists as pure models with host unit tests. No model opens a device.

| Area | Apple reference | BifrOSt path | Done | Needs Midget |
|---|---|---|---|---|
| MIDI 1.0 and UMP messages | MIDI 1.0 bytes, MIDI 2.0 UMP (`MIDIMessage_32`, `MIDIMessage_64`) | `springboard/audio/midi/src/ump.rs`, `events.rs` | UMP words for MIDI 1.0 channel voice, MIDI 2.0 channel voice, system, 64-bit and 128-bit data; SysEx assembly; MIDI 1.0 and UMP conversion with 7, 16 and 32-bit scaling | Yes: a device sends and receives |
| Endpoints and clients | MIDI Services (`MIDIClientCreate`, ports, `MIDIPortConnectSource`) | `springboard/audio/midi/src/endpoints.rs` | Registry of clients, devices, entities, endpoints, virtual endpoints, connections | Yes |
| Routing | MIDI Services (`MIDIReceiveBlock`) | `springboard/audio/midi/src/hub.rs` | Per-app virtual source and destination; queues stable-sorted by host time | Yes |
| Thru and transforms | MIDI Thru Connection (`MIDIThruConnectionCreate`, `MIDIValueMap`, `MIDIControlTransform`) | `springboard/audio/midi/src/thru.rs` | Value maps (identity, table, linear), control transforms, thru parameters and enable state | Yes |
| BLE MIDI packets | MIDI Bluetooth | `springboard/audio/midi/src/ble.rs`, `springboard/ui/src/midi_ble.rs` | Packet encode and decode with timestamps and MTU splitting; device pairing and classification in the system layer | Yes: a BLE MIDI keyboard on the live link |
| MIDI-CI discovery | MIDI Capability Inquiry (`MIDICIDiscoveryManager`) | `springboard/sdk/src/midi_ci.rs` | Discovery, invalidate MUID, profile inquiry, set and enabled messages | Yes |
| MIDI-CI Property Exchange | MIDI Capability Inquiry (property exchange) | `springboard/sdk/src/midi_ci_property.rs` | Capabilities inquiry and reply; Get and Set Property Data messages with a request id, header and chunk; `split` into chunks numbered from 1; `Reassembler` for in-order chunks | Yes: a device answers a Get Property Data inquiry |
| RTP-MIDI sessions | MIDI Networking | `springboard/sdk/src/midi_network.rs` | Session packets (IN, OK, NO, BY), hosts, connections, connection policy | Yes: sockets are not written yet |
| MIDI System Setup | `MIDISetupAddExternalDevice`, `MIDIExternalDeviceCreate` | `springboard/audio/midi/src/endpoints.rs` | External devices, entities and endpoints with removal | Yes: no hotplug source |

The DAW note model (`springboard/apps/daw/src/model.rs`) still stores only pitch and velocity. Note-name and frequency
conversion remains in `springboard/apps/daw/src/compose.rs` and `dsp.rs`, status parity, verified on Midget 0.7.25.

## Property Exchange details

Each message begins with the MIDI-CI common header and ends with `F7`. The sub-id 2 values are 0x30 and 0x31
(capabilities inquiry and reply), 0x34 and 0x35 (Get), 0x36 and 0x37 (Set). Get and Set messages carry a 7-bit request
id, a 14-bit header size and header, then a 14-bit chunk count, chunk index (from 1), data size and data. Every payload
byte is 7-bit. The module uses the version byte that `midi_ci.rs` already speaks.

Tests cover the byte layout of a capabilities reply, round trips for Get messages, refusal of high-bit bytes, bad
chunk numbers, framing errors, splitting, and reassembly (order, request mismatch, reset).

## Gaps

- Nothing is wired to a live device path (ALSA rawmidi and seq, BLE GATT) or to an app. The midid daemon crate
  describes that path; it has not been verified.
- No MIDI-CI subscription messages and no transport for Property Exchange over a live link.
- No RTP-MIDI socket I/O or mDNS discovery.
- No hotplug event path into the endpoint registry.
- No app-facing SDK API for MIDI beyond the protocol modules in `springboard/sdk/src/midi*.rs`.
- Nothing above is verified on Midget.
