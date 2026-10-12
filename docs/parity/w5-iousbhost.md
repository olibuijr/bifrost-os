# Wave 5: IOUSBHost (printer class over USB)

Framework: IOUSBHost, https://developer.apple.com/documentation/iousbhost (macOS 10.15, Mac Catalyst 14.0).

## What was built

The printer half of the USB gap row "Scanners and printers over USB". The earlier waves already had the descriptor
parser (`springboard/core/src/usb_descriptors.rs`), the pipe model (`usb_pipe.rs`) and the transfer planner
(`usb_transfer.rs`). This wave adds the USB printer class on top of them:

- `springboard/core/src/usb_printer.rs` (`bifrost_core::usb_printer`), pure model:
  - `is_printer_interface`, `protocol` and `Protocol` (unidirectional, bidirectional, IEEE 1284.4) from the interface
    class 7, subclass 1 and the protocol byte.
  - `bulk_endpoints`: the first bulk out and bulk in address of an interface. Interrupt endpoints are ignored.
  - `get_device_id`, `get_port_status`, `soft_reset`: the three printer class control requests as `SetupPacket`s
    (`usb_descriptors`), addressed to the interface.
  - `PortStatus`: paper empty, selected, no error and `ready` from the one-byte status reply.
  - `parse_device_id` and `DeviceId`: the IEEE 1284 device ID (two-byte big-endian length that counts itself, then
    `KEY:VALUE;` pairs). Accessors for manufacturer, model, command sets (page description languages) and `supports`.
- `springboard/sdk/src/usb_printer.rs` (`bifrost_app::usb_printer`): re-exports the model for apps, the same way
  `serial_ports` does.
- `pub mod` lines in `springboard/core/src/lib.rs` and `springboard/sdk/src/lib.rs`.

Not built: opening the device, sending the requests, and scanners. The app sandbox binds no USB device nodes
(`springboard/src/bare/app_policy.rs`), and there is no transport. Print jobs still go through `springboard/sdk/src/print.rs`
(CUPS). No scanner class is built: there is no USB scanner class in the framework's own documentation, so scanners
stay a gap.

The printer class facts (class 7, subclass 1, protocols 1 to 3, the three requests, the port status bits and the IEEE 1284
device ID layout) come from the USB printer class definition, not from Apple's pages. The Apple page only says that the
framework covers printers. Those constants were not checked against a copy of the spec in this run and should be
verified before the device path is wired.

## Official API names mirrored

- `IOUSBHostDevice` (claims devices and sends device requests): the setup packet side only.
- `IOUSBHostInterface`, `IOUSBHostPipe`: the bulk endpoint selection and request types only.
- `IOUSBHostMatchingPropertyKey`: not mirrored in this wave.

Source: `python3 -I tools/parity/docc.py iousbhost` (collection page, lists the framework's classes).

## REA evidence

- `ev_5086d7f786c12d1472290d316edabe7ca2003c75a399fd74b3c8c9be78af598f`: rendered IOUSBHost landing page
  (`tools/parity/rea-page.sh https://developer.apple.com/documentation/iousbhost`). The page says the framework covers
  cameras, audio devices, scanners, printers, keyboards, mouse devices, MIDI keyboards and USB hubs.

## Tests

`springboard/core/src/usb_printer.rs` has seven tests (not run: the wave builds nothing):

1. `printer_protocols_are_read_from_class_subclass_and_protocol`
2. `bulk_endpoints_pick_the_first_bulk_out_and_in`
3. `class_requests_use_the_interface_recipient` (request type 0xA1 for device to host, 0x21 for soft reset)
4. `port_status_bits_decide_readiness`
5. `device_id_gives_manufacturer_model_and_languages`
6. `device_id_stops_at_its_declared_length`
7. `device_id_rejects_short_past_end_and_non_text`

## Matrix rows closed

| Framework (slug) | Feature (as written in data/iousbhost.json) | Status | BifrOSt path |
|---|---|---|---|
| iousbhost | Scanners and printers over USB | partial | springboard/core/src/usb_printer.rs, springboard/sdk/src/usb_printer.rs |

The row was `gap`. It is `partial` now: the printer class model exists, the scanner side and the device transport do
not. No `os` row was changed in this wave.
