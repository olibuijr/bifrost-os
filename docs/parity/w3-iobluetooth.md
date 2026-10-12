# IOBluetooth parity, wave 3

Branch `parity/w3-iobluetooth`. Two gap rows of the IOBluetooth framework are built in pure Rust, std only, with no
hardware access, no socket and no network. Both are codecs: they turn bytes into values and back. The transport that
moves the bytes (an RFCOMM socket, an SDP query) is not built here.

## What was built

### RFCOMM frames: `springboard/core/src/rfcomm_frame.rs`

The RFCOMM channel layer of Bluetooth BR/EDR, per the Bluetooth Core specification (RFCOMM, TS 07.10 framing).
It encodes and decodes the address byte (DLCI, command/response bit), the control byte (SABM, UA, DM, DISC, UIH and
the poll/final bit), the one- or two-byte length field, and the frame check sequence. The FCS is the reflected CRC-8
(polynomial 0xE0, initial 0xFF, complemented). For UIH frames the FCS covers only address and control, as the
specification requires.

- `encode(&Frame) -> Result<Vec<u8>, Error>`, `decode(&[u8]) -> Result<Frame, Error>`, `check_passes(&[u8]) -> bool`.
- Errors: `BadDlci`, `TooLong`, `TooShort`, `BadAddress`, `BadControl`, `BadLength`, `BadFcs`.

Official API it mirrors: `IOBluetoothRFCOMMChannel` (the channel object: `writeSync`, `writeAsync`, `isOpen`,
`isIncoming`, `getMTU`, `close`) and `IOBluetoothRFCOMMChannelDelegate`. The framing here is the part of that channel
that is pure data; the channel object itself needs a link and is not built.

Reference: `https://developer.apple.com/documentation/iobluetooth/iobluetoothrfcommchannel`

### SDP data elements: `springboard/core/src/sdp_element.rs`

The typed values of an SDP service record. It encodes and decodes the header byte (5-bit type descriptor, 3-bit size
descriptor) for nil, unsigned and signed integers (1, 2, 4, 8 bytes, minimal size chosen on encode), 16/32/128-bit
UUIDs, text, boolean, sequence, alternative and URL elements. Variable lengths use size descriptors 5, 6 and 7.
Decoding enforces a nesting limit of 16 (`TooDeep`) and rejects trailing bytes.

- `encode(&Element) -> Vec<u8>`, `decode(&[u8]) -> Result<Element, Error>`.
- `Element` (Nil, UInt, SInt, Uuid, Text, Bool, Sequence, Alternative, Url), `Uuid` (U16, U32, U128).
- Errors: `Truncated`, `BadType`, `BadSize`, `TooDeep`, `TrailingBytes`.

Official API it mirrors: `IOBluetoothSDPDataElement` (`init(type:sizeDescriptor:size:value:)`, `getTypeDescriptor`,
`getSizeDescriptor`, `getSize`, `getValue`) and `IOBluetoothSDPUUID` (`init(uuid16:)`, `init(uuid32:)`,
`init(bytes:length:)`). The official page describes the same type and size descriptors that this module reads and
writes.

Reference: `https://developer.apple.com/documentation/iobluetooth/iobluetoothsdpdataelement`,
`https://developer.apple.com/documentation/iobluetooth/iobluetoothsdpuuid`

## Wiring

- `springboard/core/src/lib.rs`: two lines, `pub mod rfcomm_frame;` and `pub mod sdp_element;`.
- No caller in the shell yet. The shell's Bluetooth thread (`springboard/src/bare/bluetooth.rs`) still does not open an
  RFCOMM channel or query SDP. These modules are ready for that work.

## Tests

- `rfcomm_frame`: 7 tests. The SABM FCS on DLCI 0 is 0x1C (encoded as `03 3F 01 1C`); the UA FCS on DLCI 0 is 0xD7;
  a UIH round trip has 9 bytes and address 0x0F; a 200-byte payload uses length bytes 0x90 0x01; a UIH information
  byte can change without an FCS error; a damaged SABM is refused with `BadFcs`; unknown control, short input and
  DLCI 64 are refused; the whole-frame check passes for SABM and fails after a control bit flip.
- `sdp_element`: 8 tests. Minimal integer sizes (0x08 0x05, 0x09 0x12 0x34, 0x0A 00 01 00 00); a 16-bit UUID is
  0x19 0x11 0x01; text `ab` is 0x25 0x02 0x61 0x62; a sequence of UInt 5 and Bool true is 0x35 0x04 0x08 0x05 0x28
  0x01; signed -1 is 0x10 0xFF and -200 is 0x11 0xFF 0x38; a nested tree round-trips; a sequence declaring five body
  bytes with two present is `Truncated`; extra bytes are `TrailingBytes`; type 9 is `BadType(9)` and size 5 on an
  integer is `BadSize(5)`.

The expected values were worked out by hand from the CRC rule and the header layout. No build or test run was done in
this wave, as the wave brief requires.

## Matrix rows closed

| Framework | Feature (as in `docs/parity-matrix/data/iobluetooth.json`) | New status | BifrOSt path |
|---|---|---|---|
| iobluetooth | RFCOMM channels | partial | `springboard/core/src/rfcomm_frame.rs` |
| iobluetooth | SDP service records and data elements | partial | `springboard/core/src/sdp_element.rs` |

Both rows stay `partial` on purpose: the frame and element codecs exist, but no channel I/O and no SDP query or record
store exists yet.

## REA evidence

- IOBluetooth landing page: `ev_64e3d38215d8984e73469f463496b8d01f4681990d53c5b7dea31cc4eccb89be`
- IOBluetoothRFCOMMChannel: `ev_755d6c56254a42dbd7b12e406e58185bf1ed50c2e309ef1a51d5de2234a3972e`
- IOBluetoothSDPDataElement: `ev_d5412b59cfbabf11a70189ee6d72523857a8045110961d2d78b5484de3d86b18`

## Gaps left open in this framework

- RFCOMM channel I/O (open, write, close, the delegate) needs a socket and is not built.
- SDP queries and service-record publishing are not built.
- OBEX sessions, the hands-free profile, HID device ignore/count, and user notifications on device events are still
  gaps.
