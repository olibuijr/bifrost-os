# Wave 4, theme 14: interface, pipe and stream I/O

Theme: control, bulk, interrupt and isochronous transfers, interface and pipe I/O, and serial line settings.
Scope of this round: two pure models in `springboard/core`. Neither is wired into an SDK API, the sandbox or the
shell; both are tested on the host only (`cargo test -p` was not run in this round, per the wave rules).

## What was built

- `springboard/core/src/usb_pipe.rs`: a pipe over one endpoint descriptor from `usb_descriptors.rs`
  (`EndpointDescriptor`, `TransferType`; reused, not duplicated). It covers:
  - Packet payload size and the high-bandwidth transactions per service interval (`wMaxPacketSize` bits 0..=10
    and 11..=12), with the endpoint-type rules: extra transactions only on interrupt and isochronous endpoints,
    value 3 reserved, zero packet size rejected.
  - Splitting a control, bulk or interrupt data stage into full packets and one short tail.
  - Isochronous schedules (`IsoSchedule`, `IsoFrame`) with per-frame request and completion counts, a
    status (`Pending`, `Complete`, `Partial`) and an overrun check.
  - A request queue for bulk and interrupt I/O (`enqueue`, `complete`, `Completion::is_short`), halt and
    clear-stall (`halt`, `clear_stall`), and an idle timeout value.
  - Bulk streams: `enable_streams`, `copy_stream` (valid ids 1..=65533), `open_streams`, `disable_streams`.
    The stream handle is the stream identifier; there is no separate stream object.
- `springboard/core/src/serial_line.rs`: line settings for an asynchronous serial port (`LineSettings`): standard
  baud rates (`STANDARD_BAUD_RATES`), data bits 5..=8, parity, one or two stop bits, flow control
  (none, hardware, software), validation, bits per character, characters per second, and the time to send a
  buffer in microseconds (rounded up).

Both modules are registered with one `pub mod` line each in `springboard/core/src/lib.rs`. Each has a
`#[cfg(test)] mod tests` (nine tests in `usb_pipe.rs`, six in `serial_line.rs`) whose values were worked out by hand from the rules above
(for example 1300 bytes at 512-byte packets gives 512, 512, 276; 100 bytes of 8N1 at 115200 baud takes 8681 µs).

## Apple API names mirrored

- IOUSBHostPipe (https://developer.apple.com/documentation/iousbhost/iousbhostpipe): `enqueueIORequest`,
  `clearStall`, `enqueueIORequest(with:frameList:...)` and `sendIORequest` for isochronous, the periodic
  bandwidth and idle-timeout properties, and `enableStreams`, `copyStream(withStreamID:)`, `disableStreams`.
- IOUSBHostIsochronousFrame (https://developer.apple.com/documentation/iousbhost/iousbhostisochronousframe):
  `requestCount`, `completeCount`, `status`.
- The serial-port side mirrors no Apple API. The official reference is the IOKit serial sample and modem
  guide, which this round did not fetch; the line model is a plain serial-port concept.

REA evidence (official rendered pages, via `tools/parity/rea-page.sh`):

- `ev_af3dff22df6574b3a329229a730418f26c1b4a8bb2888f0526919194cd2c6205`: IOUSBHostPipe
- `ev_a397ac6e384b12462f7a3bc37f83bbe01b9ce924b2651d22de6c485385c968ed`: IOUSBHostIsochronousFrame

## Matrix rows closed (new status partial)

| Framework slug | Feature (as in the matrix) | Status | BifrOSt path |
|---|---|---|---|
| iousbhost | Interface, pipe and stream I/O (control, bulk, interrupt, isochronous) | partial | springboard/core/src/usb_pipe.rs |
| iousbhost | Isochronous transfer options | partial | springboard/core/src/usb_pipe.rs |
| iokit | Serial ports and modem | partial | springboard/core/src/serial_line.rs |

## Not closed, and why

- No device is claimed and no request is sent. Apps run under bwrap with no USB device nodes bound
  (`springboard/src/bare/app_policy.rs`), so the pipe model has no transport behind it yet.
- Isochronous transaction lists, transfer options (`IOUSBHostIsochronousTransaction`) and the bandwidth
  adjustment call are not modelled beyond the packet and frame arithmetic.
- Serial ports: no enumeration, no open path and no modem line control (DTR, RTS, CD). The settings model
  does not touch a device.
- Not in this theme's closed rows: Core MIDI system setup, IOBluetooth OBEX, Endpoint Security events, Matter
  controller storage, System Configuration bond and VLAN, dnssd NAT port mapping, Core Audio server plug-ins,
  and the camera and microphone permission row.
