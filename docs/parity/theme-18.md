# Theme 18: device driver access (pure models)

Scope: the pure, testable half of USB and HID device access. No device is opened, no transfer is sent and no
hardware is touched. The caller reads the descriptor bytes from the kernel and sends the encoded requests and report
buffers itself, so these models can be tested on a host with no device attached.

## What was built

- `springboard/core/src/usb_descriptors.rs` (module `usb_descriptors`, core crate):
  - Standard descriptor parsers: `parse_device` (18-byte device descriptor) and `parse_configuration` (the
    configuration block with its interface and endpoint descriptors, walked by `wTotalLength`). Class-specific
    descriptors are skipped by length. Truncated, mis-typed, bad-length and orphan-endpoint input is rejected.
  - `EndpointDescriptor` (number, direction, transfer type), `InterfaceDescriptor`, `ConfigurationDescriptor`
    with `interface(number, alternate)`.
  - Control requests: `SetupPacket` with `encode` and `decode` (8-byte wire form), and the builders
    `get_descriptor`, `set_configuration`, `set_interface`. Request numbers live in `request::*`.
  - Matching: `DeviceMatch` and `InterfaceMatch` (fields left as `None` match anything) and `MatchingCriteria`
    with `InterfaceMatchOption::{MatchAll, MatchAny}`.
- `springboard/ui/src/hid_report.rs` (module `hid_report`, ui crate):
  - `ReportDescriptor::parse` reads the short items of a HID report descriptor: globals (usage page, logical and
    physical ranges, unit and exponent, report size, count and ID, push and pop), locals (usage, usage min and
    max), and the main items Input, Output, Feature, Collection and End Collection.
  - `Element` (one field): `bit_offset`, `bit_size`, logical and physical ranges, usage page and usage, flags,
    parent collection. `value` reads a signed or unsigned field from a payload. `set_value` writes a field into
    an output or feature payload and refuses constant fields and out-of-range values. `physical_value` maps the
    logical value onto the physical range and applies the unit exponent.
  - `Collection` tree with a top level at index 0; `report_ids`, `fields`, `payload_len` per report kind.
- Both modules have a `#[cfg(test)] mod tests` block (6 tests each). One `pub mod` line was added to
  `springboard/core/src/lib.rs` and one to `springboard/ui/src/lib.rs`.

Not built, and not claimed: opening a device, claiming an interface, bulk, interrupt, control or isochronous
transfers, device reset, the accessory listener and its attach and detach events, sending reports to a live device,
and any app-facing SDK wrapper. The desktop sandbox binds only sound and GPU nodes (`springboard/src/bare/app_policy.rs`),
so no app can reach a USB or HID node yet. These models are the parsing and encoding layer that a later
device-access service would call.

## Official API names mirrored

- IOUSBHost, Device Drivers: `IOUSBHostDevice` (configuration descriptor, descriptor retrieval, device requests),
  `IOUSBHostObject` (device requests and descriptors). https://developer.apple.com/documentation/iousbhost
- Accessory Access: `AAUSBAccessoryMatchingCriteria` (device criteria, interface criteria, and the
  `matchAll` and `matchAny` interface options). https://developer.apple.com/documentation/accessoryaccess
- Core HID: `HIDElement` (usage, report ID, report size, logical and physical minimum and maximum, unit exponent,
  `value` reads), `HIDElementCollection` (type, usage, child elements and collections), `HIDReportType`
  (input, output, feature). https://developer.apple.com/documentation/corehid
- HIDDriverKit: element access reads through `HIDElement` values, as above.

## REA evidence

- IOUSBHost framework page: `ev_68d2cc21fd846e5df3b52cd0daeb17d85fbd61ee7eda88009f20978adf31d53c`
- Accessory Access framework page: `ev_c4e115b1ecac4621adf40a016963ca0253fb40cba362f1b2a5c02e582c4e9ce0`
- Core HID framework page: `ev_240b60218829c246863a5fea353f4ad1ed2a709fee319e776107bb3a0c79bfc7`

DocC symbol facts were read with `tools/parity/docc.py` for `iousbhost/iousbhostdevice`,
`accessoryaccess/aausbaccessorymatchingcriteria` (and its `interfacematchingoption`), `corehid/hidelement`,
`corehid/hidelementcollection`, `corehid/hidelement/value` and `corehid/hidreporttype`.

## Matrix rows closed (for the lead to apply centrally)

| framework slug | feature (as written in the matrix) | new status | BifrOSt path |
|---|---|---|---|
| iousbhost | Device driver access (claim, configure, descriptors, device requests) | partial | springboard/core/src/usb_descriptors.rs (descriptor parsers, `SetupPacket`, request builders) |
| accessory-access | Matching criteria | partial | springboard/core/src/usb_descriptors.rs (`MatchingCriteria`, `DeviceMatch`, `InterfaceMatch`) |
| core-hid | Report descriptor elements and collections | partial | springboard/ui/src/hid_report.rs (`ReportDescriptor::parse`, `Element`, `Collection`) |
| core-hid | Sending output reports to a device | partial | springboard/ui/src/hid_report.rs (`Element::set_value` builds the report bytes; no send) |
| hiddriverkit | Raw HID report and element access for apps | partial | springboard/ui/src/hid_report.rs (`Element::value`, `Element::physical_value`; no live device) |

Rows left open in this theme: Accessory object, Accessory manager, Permission and access grant, Listener events,
Interface and pipe I/O, Isochronous transfer options, Scanners and printers, MIDI and Virtualization USB rows. Each
needs a device service or the transfer layer, which is not pure.
