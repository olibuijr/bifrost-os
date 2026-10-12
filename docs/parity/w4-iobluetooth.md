# IOBluetooth parity, wave 4

Branch `parity/w4-iobluetooth`, on top of `parity/integration` (waves 1 to 3). Three rows of the IOBluetooth framework
are built in pure Rust, std only, with no hardware access, no socket and no network. Like wave 3, each piece is a
codec or a model: it turns bytes into values and back, or keeps per-app state. The transport that carries the bytes
(an OBEX link over RFCOMM, the radio's device events) is not built here.

## What was built

### BD addresses and classes of device: `springboard/core/src/bt_address.rs`

- `BdAddr([u8; 6])`: parses `AA:BB:CC:DD:EE:FF` or `AA-BB-...` (either case), prints upper-case colon form, and
  reports `is_zero`. `FromStr` returns `AddrError::BadFormat` (not six two-character groups) or `AddrError::BadHex`.
- `ClassOfDevice`: the 24-bit class of device. `new` refuses values above 24 bits. Accessors: `minor` (bits 2 to 7),
  `major` (bits 8 to 12, as `MajorClass`), `has_service` with the `SERVICE_*` bit constants (bits 16 to 23), and
  `kind` (`DeviceKind`). The kind reads the audio-video minor classes (headset, hands-free, microphone, loudspeaker,
  headphones) and the peripheral keyboard and pointer flags (bits 6 and 7).

Official API it mirrors: `IOBluetoothDevice` (`init(addressString:)`, `addressString`, `classOfDevice`,
`deviceClassMajor`, `deviceClassMinor`, `serviceClassMajor`), whose official page says the address string has the same
format as the framework's address-to-string conversion (the `IOBluetoothNSStringFromDeviceAddress` helper). The
string conversion and the class decoding are the parts of that API that are pure data.

Reference: `https://developer.apple.com/documentation/iobluetooth/iobluetoothdevice`

Bluetooth Assigned Numbers are the source for the major and minor class codes and the service bit positions. They are
not Apple facts.

### OBEX packets: `springboard/core/src/obex_packet.rs`

The OBEX packet layer: opcode or response code with the final bit, a 16-bit length that counts the whole packet, the
fixed part of CONNECT (version 1.0, flags, maximum packet length) and SETPATH, and the headers. A header identifier's
top two bits give its value type: text (UTF-16BE, zero-terminated, length-prefixed), byte sequence (length-prefixed),
one byte, or four bytes.

- `encode(&self) -> Result<Vec<u8>, Error>` on `Packet`; `decode(bytes, fixed) -> Result<Packet, Error>`.
  The caller passes the fixed-part length, which `fixed_len(op, response)` gives.
- `connect_request(max_len, headers)` and `connect_response(max_len, headers)`; `Packet::find(id)`.
- Constants for the opcodes, the response codes that map to the HTTP status classes, and the header identifiers used
  here (Name, Type, Length, Count, Time, Connection-ID, Target, Body, End-of-body, Who, Description).
- Errors: `TooShort`, `BadLength`, `TooLong`, `BadText`, `TypeMismatch(id)`.

Official API it mirrors: `OBEXSession` (`obexConnect`, `obexConnectResponse`, `obexPut`, `obexGet`, `obexSetPath`,
`obexDisconnect`, `obexAbort`, `getMaxPacketLength`, `getAvailableCommandPayloadLength`), and
`OBEXFileTransferServices` (`init(obexSession:)`, `sendFile`, `retrieveFolderListing`). The official page of
`OBEXSession` says that a session does nothing useful without a transport, and that the first packet a server gets is
a CONNECT request. The codec here is the packet layer under those calls. The session objects and the file-transfer
service need a transport and are not built.

Reference: `https://developer.apple.com/documentation/iobluetooth/obexsession`,
`https://developer.apple.com/documentation/iobluetooth/obexfiletransferservices`

The OBEX wire constants come from the OBEX protocol specification, not from Apple's pages.

### Device events: `springboard/sdk/src/bluetooth_events.rs`

An app-facing model of connect and disconnect notifications. `DeviceEvents::register(Filter)` returns a
`Registration` handle; a `Filter` picks the kinds (connected, disconnected) and optionally one `BdAddr`. `post` offers
an event to every matching registration and returns how many took it. `take` drains one registration's queue, oldest
first. `unregister` removes a registration, after which it gets no more events (`is_registered` reports it).
Each queue holds at most `QUEUE_CAPACITY` (64) events; when it is full, the oldest is dropped and counted (`dropped`).

Official API it mirrors: `IOBluetoothUserNotification` (`unregister()`) and the device connect and disconnect
registrations on `IOBluetoothDevice` (`register(forConnectNotifications:selector:)`,
`register(forDisconnectNotification:selector:)`). Its official page says a notification object is returned when an app
registers, and that it stops being valid after `unregister`.

Reference: `https://developer.apple.com/documentation/iobluetooth/iobluetoothusernotification`,
`https://developer.apple.com/documentation/iobluetooth/iobluetoothdevice`

The shell still handles its own connect and disconnect events (`springboard/src/bare/bluetooth.rs`, `EV_DEVICE_CONNECTED`
and `EV_DEVICE_DISCONNECTED`). Nothing feeds this queue, so the row stays partial.

## Wiring

- `springboard/core/src/lib.rs`: `pub mod bt_address;` and `pub mod obex_packet;`.
- `springboard/sdk/src/lib.rs`: `pub mod bluetooth_events;`.
- No caller yet. The shell's Bluetooth thread does not open an OBEX session or post device events to this queue.

## Tests

The expected values were worked out by hand from the rules above. No build or test run was done in this wave, as the
wave brief requires.

- `bt_address`: 7 tests. `aa:bb:0c:1D:2e:3F` and `AA-BB-0C-1D-2E-3F` are the same address, bytes `AA BB 0C 1D 2E 3F`;
  `00 1A 7D DA 71 13` prints as `00:1A:7D:DA:71:13`; five groups or one-digit groups give `BadFormat`, and `FG` or
  `+A` give `BadHex`; the all-zero address is zero and `00:...:01` is not; class `0x240404` is audio-video minor 1
  (headset) with audio and rendering bits and no telephony bit; `0x002540` is a keyboard (minor 16), `0x0025C0` a
  keyboard and pointer, and `0x002580` a pointer; a value above `0xFFFFFF` is refused.
- `obex_packet`: 8 tests. CONNECT request is `80 00 07 10 00 20 00`; PUT with name `a` is
  `82 00 0A 01 00 07 00 61 00 00`; DISCONNECT with Connection-ID 1 is `81 00 08 CB 00 00 00 01`; decode reads the
  fixed part and the name back; a round trip keeps text (with a non-ASCII letter), byte sequence, one-byte and
  four-byte headers; a declared length of 8 on a 7-byte packet is `BadLength`; a name without its zero terminator is
  `BadText`; a four-byte value under a byte-sequence identifier is `TypeMismatch(0x42)`; a 2-byte input is `TooShort`.
- `bluetooth_events`: 6 tests. A matching event is queued and drained once; an address and kind filter passes only
  connect events of one address; an unregistered handle gets nothing and cannot be removed twice; 70 posts leave 64
  queued and 6 dropped, the first queued address being the seventh posted; two registrations keep separate queues
  (2 and 1 events); posting with no registration takes nothing.

## Matrix rows closed

| Framework | Feature (as in `docs/parity-matrix/data/iobluetooth.json`) | New status | BifrOSt path |
|---|---|---|---|
| iobluetooth | OBEX sessions and file transfer | partial | `springboard/core/src/obex_packet.rs` |
| iobluetooth | Address and utility helpers | partial | `springboard/core/src/bt_address.rs` |
| iobluetooth | User notifications on device events | partial | `springboard/sdk/src/bluetooth_events.rs` |

All three stay `partial` on purpose: the codecs and the queue exist, but no OBEX session, no transport, and no radio
feed exist yet. The row "Remote device model (name, class, paired, connected)" is not closed: `bt_address` adds class
decoding, but there is still no app-facing device object.

## REA evidence

- IOBluetooth landing page: `ev_64e3d38215d8984e73469f463496b8d01f4681990d53c5b7dea31cc4eccb89be`
- OBEXSession: `ev_91bded032ea0feb6f53006801c6661ea44fa81d0e35c7eec6c85c6134940ee0f`
- IOBluetoothDevice: `ev_86d1fbe39790ae99cb5120d21b5a5f99f8be7038d99c56c682f15f911335ce6f`
- IOBluetoothUserNotification: `ev_e43cf4f738d7fb9881a2b7c145bc011d97edb93c78e181a2061a59adba571b6b`
